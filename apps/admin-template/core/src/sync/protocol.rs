//! Phase 8: 同期プロトコルの PC 側（`docs/domain/sync.md` 11節）。
//! conventions §2 に従い `tauri` / `axum` / RBAC を知らない。
//!
//! ## なぜ片側だけがサーバなのか
//!
//! 対等な2台に見えるが、**話しかけるのは常にスマホ側**。PC は自宅 LAN に
//! 据え置きで、スマホの IP は Wi-Fi に乗るたび変わりうるし、外出中は
//! そもそも到達できない。「戻ってきた側が話しかける」のが自然な向きになる。
//!
//! したがってこのモジュールは**受け身**の3つだけを持つ。
//!
//! | 入口 | 向き | 中身 |
//! |---|---|---|
//! | [`SyncService::handshake`] | 確認 | 互いのデバイス番号と、どこまで進んでいるか |
//! | [`SyncService::pull`] | PC → スマホ | PC 側で変わった行 |
//! | （次段）push | スマホ → PC | スマホ側で変わった行の取り込み |
//!
//! この段では **PC を一切書き換えない読み取りだけ**を入れる。取り込み
//! （push）と衝突提示は次段。読む側だけ先に固めておくと、取り込みの
//! テストで「送られてきた形」を実物で作れる。
//!
//! ## 進捗の持ち主
//!
//! `after_seq` は**要求する側が持つ**。`sync_state` にサーバが覚えている値を
//! 正としない —— 実際に行を適用したのは要求した側で、応答が途中で切れた
//! ことを知っているのも要求した側だけ。サーバが「送った」と記録した時点で
//! 進めてしまうと、届かなかった変更が二度と送られない。
//!
//! `sync_state` は次段の push で「PC がどこまで取り込んだか」を刻む。
//! [`SyncService::handshake`] はそれを読んで返すだけで、書かない。

use banto_core::{BantoError, FieldError};
use banto_storage::Db;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::settings::SettingsService;
use crate::sync::rows::{read_rows, table_spec, SyncRow, SYNCED_TABLES};
use crate::sync::{outbox_head, outbox_since, peer_state, stored_device_id, OUTBOX_PAGE_SIZE};

/// 同期の入口（読み取りのみ）。
#[derive(Clone)]
pub struct SyncService {
    db: Db,
    settings: SettingsService,
}

/// [`SyncService::handshake`] の要求。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HandshakeRequest {
    /// 話しかけてきた端末のデバイス番号（`docs/domain/sync.md` 3.4）。
    pub peer_device_id: i64,
}

/// [`SyncService::handshake`] の応答。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Handshake {
    /// この端末（PC）のデバイス番号。
    pub device_id: i64,
    /// この端末の outbox の最後の `seq`。相手はこれと自分の控えを比べて、
    /// 引くものが残っているかを判断する。
    pub outbox_head: i64,
    /// この端末が相手から取り込み終えた最後の `seq`（次段の push が刻む）。
    pub received_through_seq: i64,
    /// 同期対象のテーブル名（依存順）。相手が知らないテーブルが増えていたら
    /// 気付けるようにする。
    pub tables: Vec<String>,
    /// 1回の [`SyncService::pull`] が返す最大件数。
    pub page_size: i64,
}

/// [`SyncService::pull`] の要求。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequest {
    /// 相手が既に持っている最後の `seq`。初回は 0。
    pub after_seq: i64,
}

/// [`SyncService::pull`] の応答。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pull {
    pub device_id: i64,
    /// このページに含まれる最後の `seq`。**適用し終えてから**次の要求の
    /// `after_seq` に使う。
    pub through_seq: i64,
    /// この端末の outbox の最後の `seq`。`through_seq < head_seq` なら続きが
    /// ある。
    pub head_seq: i64,
    /// 変わった行の**現在の姿**。[`SYNCED_TABLES`] の依存順に並ぶので、
    /// 受け側はこの順に流し込めば外部キーで弾かれない。
    pub rows: Vec<SyncRow>,
}

impl Pull {
    /// 続きがあるか。
    pub fn has_more(&self) -> bool {
        self.through_seq < self.head_seq
    }
}

impl SyncService {
    pub fn new(db: Db, settings: SettingsService) -> Self {
        Self { db, settings }
    }

    /// 互いの立ち位置を確認する。
    ///
    /// **両端末が同じデバイス番号を名乗ったら拒否する。** これは id レンジが
    /// 分かれていないということで、そのまま同期すると別々の行が同じ id を
    /// 持ったまま混ざる（`docs/domain/sync.md` 3節）。混ざってから気付くと
    /// どちらが正かを機械的に判定できないので、**触る前に**止める。
    pub async fn handshake(&self, request: HandshakeRequest) -> Result<Handshake, BantoError> {
        let device_id = stored_device_id(&self.settings).await?;
        if request.peer_device_id == device_id {
            return Err(BantoError::Validation {
                field_errors: vec![FieldError {
                    field: "peerDeviceId".to_string(),
                    message: format!(
                        "相手と同じデバイス番号（{device_id}）です。\
                         id の採番レンジが分かれていないため同期できません。\
                         どちらかの設定を変更してください"
                    ),
                }],
            });
        }

        let peer = peer_state(&self.db, request.peer_device_id).await?;
        Ok(Handshake {
            device_id,
            outbox_head: outbox_head(&self.db).await?,
            received_through_seq: peer.received_through_seq,
            tables: SYNCED_TABLES
                .iter()
                .map(|spec| spec.name.to_string())
                .collect(),
            page_size: OUTBOX_PAGE_SIZE,
        })
    }

    /// `after_seq` より後に変わった行を返す。
    ///
    /// ## 「変更履歴」ではなく「今の姿」を送る
    ///
    /// outbox は `(テーブル, 主キー, 操作)` しか持たず、変更前後の値を持たない。
    /// ここでは outbox を**変わった行の目次**として使い、値は現物の表から
    /// 読み直す。同じ行が1ページ内で3回変わっていても、送るのは最後の姿1件。
    ///
    /// ページ境界の後にさらに変わった行は、この呼び出しでは「先の姿」で
    /// 送られ、次のページで同じ行がもう一度送られる。**同じ行を二度送るのは
    /// 無害**（受け側は同値なら何もしない、`docs/domain/sync.md` 6.2）で、
    /// 取りこぼしよりずっと安い。
    ///
    /// ## 墓石も送る
    ///
    /// [`crate::sync::live`] は使わない。削除を伝えないと、相手側に残った行が
    /// 次の同期でこちらへ復活してくる。
    pub async fn pull(&self, request: PullRequest) -> Result<Pull, BantoError> {
        let device_id = stored_device_id(&self.settings).await?;
        let head_seq = outbox_head(&self.db).await?;
        let entries = outbox_since(&self.db, request.after_seq).await?;

        let through_seq = entries
            .last()
            .map(|entry| entry.seq)
            .unwrap_or(request.after_seq);

        // テーブルごとに主キーを集める。同じ行の複数回の変更はここで1つに
        // 畳まれる（`BTreeMap` のキーなので重複しない）。
        let mut keys_by_table: BTreeMap<&'static str, Vec<String>> = BTreeMap::new();
        for entry in &entries {
            let spec = table_spec(&entry.table_name).ok_or_else(|| {
                // 目録に無いテーブルがトリガから積まれている。黙って落とすと
                // その変更が相手へ永久に届かないので、同期ごと止める。
                BantoError::Validation {
                    field_errors: vec![FieldError {
                        field: "tableName".to_string(),
                        message: format!(
                            "同期対象にないテーブルが outbox にある: {}",
                            entry.table_name
                        ),
                    }],
                }
            })?;
            let keys = keys_by_table.entry(spec.name).or_default();
            if !keys.contains(&entry.row_key) {
                keys.push(entry.row_key.clone());
            }
        }

        // 依存順に読む（`SYNCED_TABLES` の並び）。`BTreeMap` の辞書順ではなく
        // こちらを正とするのは、受け側が並び順のまま流し込めるようにするため。
        let mut rows = Vec::new();
        for spec in &SYNCED_TABLES {
            let Some(keys) = keys_by_table.get(spec.name) else {
                continue;
            };
            rows.extend(read_rows(&self.db, spec, keys).await?);
        }

        Ok(Pull {
            device_id,
            through_seq,
            head_seq,
            rows,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::customers::{CustomerInput, CustomersService, DAY_END_OF_MONTH};
    use crate::db::migrate_memory;
    use crate::projects::{ProjectInput, ProjectsService};
    use crate::sync::rows::SyncValue;
    use crate::sync::{set_device_id, DEFAULT_DEVICE_ID};

    fn customer_input(code: &str, name: &str) -> CustomerInput {
        CustomerInput {
            code: code.to_string(),
            name: name.to_string(),
            contact_person: None,
            address: None,
            phone: None,
            email: None,
            billing_name: None,
            closing_day: DAY_END_OF_MONTH,
            payment_month_offset: 1,
            payment_day: DAY_END_OF_MONTH,
            note: None,
        }
    }

    fn project_input(code: &str, customer_id: i64) -> ProjectInput {
        ProjectInput {
            code: code.to_string(),
            customer_id,
            name: "架空案件".to_string(),
            status: "IN_PROGRESS".to_string(),
            started_on: None,
            due_on: None,
            estimate_amount: None,
            contract_amount: None,
            billing_hourly_rate: None,
            scope: None,
            note: None,
        }
    }

    fn service(db: &Db) -> SyncService {
        SyncService::new(db.clone(), SettingsService::new(db.clone()))
    }

    #[tokio::test]
    async fn a_fresh_pc_reports_device_zero_and_an_empty_outbox() {
        let db = migrate_memory().await.unwrap();
        let shaken = service(&db)
            .handshake(HandshakeRequest { peer_device_id: 1 })
            .await
            .expect("handshake");

        assert_eq!(shaken.device_id, DEFAULT_DEVICE_ID);
        assert_eq!(shaken.outbox_head, 0);
        assert_eq!(shaken.received_through_seq, 0);
        assert_eq!(shaken.page_size, OUTBOX_PAGE_SIZE);
        assert_eq!(shaken.tables.len(), 8);
        assert_eq!(shaken.tables[0], "work_categories");
    }

    /// **同じデバイス番号を名乗る相手は触る前に断る。**
    ///
    /// 番号が同じということは id の採番レンジが分かれておらず、別々の行が
    /// 同じ id を持ったまま混ざる。混ざってから気付くと機械的に直せない。
    #[tokio::test]
    async fn a_peer_claiming_our_own_device_number_is_refused() {
        let db = migrate_memory().await.unwrap();
        let error = service(&db)
            .handshake(HandshakeRequest { peer_device_id: 0 })
            .await
            .expect_err("同番は拒否");
        assert!(matches!(error, BantoError::Validation { .. }));
    }

    #[tokio::test]
    async fn the_check_follows_the_configured_device_number() {
        let db = migrate_memory().await.unwrap();
        let settings = SettingsService::new(db.clone());
        set_device_id(&settings, 1).await.expect("set");
        let sync = SyncService::new(db.clone(), settings);

        // 自分が 1 になったので、今度は 1 が拒否され 0 が通る。
        assert!(sync
            .handshake(HandshakeRequest { peer_device_id: 1 })
            .await
            .is_err());
        assert_eq!(
            sync.handshake(HandshakeRequest { peer_device_id: 0 })
                .await
                .expect("handshake")
                .device_id,
            1
        );
    }

    #[tokio::test]
    async fn an_empty_outbox_pulls_nothing() {
        let db = migrate_memory().await.unwrap();
        let pulled = service(&db)
            .pull(PullRequest { after_seq: 0 })
            .await
            .expect("pull");
        assert!(pulled.rows.is_empty());
        assert_eq!(pulled.through_seq, 0);
        assert_eq!(pulled.head_seq, 0);
        assert!(!pulled.has_more());
    }

    #[tokio::test]
    async fn a_created_row_is_pulled_with_its_current_values() {
        let db = migrate_memory().await.unwrap();
        let customers = CustomersService::new(db.clone());
        let created = customers
            .create(customer_input("C001", "架空商事"))
            .await
            .expect("create");

        let pulled = service(&db)
            .pull(PullRequest { after_seq: 0 })
            .await
            .expect("pull");
        assert_eq!(pulled.rows.len(), 1);
        assert_eq!(pulled.rows[0].table, "customers");
        assert_eq!(pulled.rows[0].key, created.id.to_string());
        assert_eq!(
            pulled.rows[0].get("name"),
            Some(&SyncValue::Text("架空商事".to_string()))
        );
        assert_eq!(pulled.through_seq, pulled.head_seq);
        assert!(!pulled.has_more());
    }

    /// outbox は「変わった行の目次」で、送るのは**最後の姿1件**。
    /// 3回直しても3件送らない。
    #[tokio::test]
    async fn repeated_edits_collapse_into_one_row() {
        let db = migrate_memory().await.unwrap();
        let customers = CustomersService::new(db.clone());
        let created = customers
            .create(customer_input("C001", "架空商事"))
            .await
            .expect("create");
        for name in ["二代目", "三代目", "四代目"] {
            customers
                .update(created.id, customer_input("C001", name))
                .await
                .expect("update");
        }

        let pulled = service(&db)
            .pull(PullRequest { after_seq: 0 })
            .await
            .expect("pull");
        assert_eq!(pulled.rows.len(), 1, "畳まれること");
        assert_eq!(
            pulled.rows[0].get("name"),
            Some(&SyncValue::Text("四代目".to_string()))
        );
    }

    /// 削除は墓石として届く。届かないと相手側に残った行が次の同期で
    /// こちらへ復活してくる。
    #[tokio::test]
    async fn a_deleted_row_is_pulled_as_a_tombstone() {
        let db = migrate_memory().await.unwrap();
        let customers = CustomersService::new(db.clone());
        let created = customers
            .create(customer_input("C001", "架空商事"))
            .await
            .expect("create");
        customers.delete(created.id).await.expect("delete");

        let pulled = service(&db)
            .pull(PullRequest { after_seq: 0 })
            .await
            .expect("pull");
        assert_eq!(pulled.rows.len(), 1);
        assert!(pulled.rows[0].is_deleted());
    }

    /// 並びは依存順。受け側がこの順に流し込めば外部キーで弾かれない。
    #[tokio::test]
    async fn rows_arrive_parents_first() {
        let db = migrate_memory().await.unwrap();
        let customers = CustomersService::new(db.clone());
        let projects = ProjectsService::new(db.clone());
        let customer = customers
            .create(customer_input("C001", "架空商事"))
            .await
            .expect("create");
        projects
            .create(project_input("2026-001", customer.id))
            .await
            .expect("create");

        let pulled = service(&db)
            .pull(PullRequest { after_seq: 0 })
            .await
            .expect("pull");
        let tables: Vec<&str> = pulled.rows.iter().map(|r| r.table.as_str()).collect();
        assert_eq!(tables, vec!["customers", "projects"]);
    }

    /// 適用済みぶんは二度と送られない。
    #[tokio::test]
    async fn the_watermark_advances_past_what_was_already_sent() {
        let db = migrate_memory().await.unwrap();
        let customers = CustomersService::new(db.clone());
        customers
            .create(customer_input("C001", "架空商事"))
            .await
            .expect("create");

        let sync = service(&db);
        let first = sync.pull(PullRequest { after_seq: 0 }).await.expect("pull");
        assert_eq!(first.rows.len(), 1);

        let second = sync
            .pull(PullRequest {
                after_seq: first.through_seq,
            })
            .await
            .expect("pull");
        assert!(second.rows.is_empty());
        assert_eq!(second.through_seq, first.through_seq);
        assert!(!second.has_more());

        // 追加の変更は次の要求で拾える。
        customers
            .create(customer_input("C002", "架空製作所"))
            .await
            .expect("create");
        let third = sync
            .pull(PullRequest {
                after_seq: second.through_seq,
            })
            .await
            .expect("pull");
        assert_eq!(third.rows.len(), 1);
        assert_eq!(
            third.rows[0].get("code"),
            Some(&SyncValue::Text("C002".to_string()))
        );
    }

    /// 初回同期で全件が1つの応答に載らないこと。続きは `through_seq` から
    /// 引き直せること。
    #[tokio::test]
    async fn a_large_outbox_is_paged_and_resumable() {
        let db = migrate_memory().await.unwrap();
        let customers = CustomersService::new(db.clone());
        let total = OUTBOX_PAGE_SIZE + 3;
        for n in 0..total {
            customers
                .create(customer_input(&format!("C{n:04}"), "架空商事"))
                .await
                .expect("create");
        }

        let sync = service(&db);
        let first = sync.pull(PullRequest { after_seq: 0 }).await.expect("pull");
        assert_eq!(first.rows.len() as i64, OUTBOX_PAGE_SIZE);
        assert!(first.has_more(), "続きがあると分かること");

        let second = sync
            .pull(PullRequest {
                after_seq: first.through_seq,
            })
            .await
            .expect("pull");
        assert_eq!(second.rows.len() as i64, total - OUTBOX_PAGE_SIZE);
        assert!(!second.has_more());
    }
}
