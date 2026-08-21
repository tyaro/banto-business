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
//! | [`SyncService::push`] | スマホ → PC | スマホ側で変わった行の取り込み |
//!
//! ## 衝突は PC で抱え込まない
//!
//! 衝突した行は**取り込まずに応答へ返す**だけで、PC は保留状態を持たない
//! （2026-08-21 決定）。選ぶのは操作している側 —— スマホ —— で、PC 側にも
//! 未解決の山を作ると同じものを二重に管理することになる。PC は
//! 「これは両方で変わっている」と判定して差し戻すところまでを担う。
//!
//! 差し戻された行は次の同期でもう一度送られるわけではない（進捗は進む、
//! 下記）。**受け取った側が応答の衝突を保存してから進捗を進めること**が
//! 呼び出し側の責任になる。
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
use crate::sync::rows::{
    apply_row, read_row, read_rows, table_spec, validate_row, values_equal, SyncRow, SyncValue,
    SYNCED_TABLES,
};
use crate::sync::{
    last_change_seq, outbox_head, outbox_since, peer_state, record_peer_progress, stored_device_id,
    OUTBOX_PAGE_SIZE,
};

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

/// 取り込みを求める1行。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PushRow {
    #[serde(flatten)]
    pub row: SyncRow,
    /// 衝突を承知のうえで上書きする（利用者が「こちらを採る」と選んだ）。
    ///
    /// **請求済みの凍結は破れない**（[`ConflictReason::InvoicedFrozen`]）。
    /// 既に取引先へ渡っている可能性のある請求の裏付けを、同期の一存で
    /// 書き換えないため。
    #[serde(default)]
    pub force: bool,
}

/// [`SyncService::push`] の要求。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PushRequest {
    pub peer_device_id: i64,
    /// 相手が取り込み終えた、**こちら（PC）の** outbox の最後の `seq`。
    ///
    /// 衝突判定の基準線。この seq より後にこちらでも同じ行が変わっていれば、
    /// 相手はこちらの版を見ないまま直したことになる（＝両方で変わった）。
    pub pulled_through_seq: i64,
    /// このページに含まれる、**相手の** outbox の最後の `seq`。
    pub through_seq: i64,
    pub rows: Vec<PushRow>,
}

/// 取り込めなかった行。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Conflict {
    pub table: String,
    pub key: String,
    pub reason: ConflictReason,
    /// こちら（PC）が持っている版。
    pub local: SyncRow,
    /// 送られてきた版。
    pub incoming: SyncRow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ConflictReason {
    /// 両端末が独立に直した（`docs/domain/sync.md` 6節）。`force` で解決できる。
    BothChanged,
    /// 請求済みの行（決定 C-20 / 6.1）。**`force` でも破れない。**
    /// 請求書を取消してから直す（F-I8）。
    InvoicedFrozen,
}

/// [`SyncService::push`] の応答。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PushResult {
    pub device_id: i64,
    /// 書き込んだ行数（新規 + 更新）。
    pub applied: usize,
    /// 同値だったので書かなかった行数。
    pub unchanged: usize,
    /// 取り込めなかった行。**送り主が保存すること**（PC は保留しない）。
    pub conflicts: Vec<Conflict>,
    /// 取り込み終えた、相手の outbox の `seq`。
    pub received_through_seq: i64,
}

impl PushResult {
    pub fn has_conflicts(&self) -> bool {
        !self.conflicts.is_empty()
    }
}

/// 請求済みかどうかを持つ列。持たないテーブルでは凍結判定をしない。
const INVOICED: &str = "invoiced";

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
            return Err(same_device_error(device_id));
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
    /// 相手から送られてきた行を取り込む。
    ///
    /// ## 1行ごとの判断
    ///
    /// | こちらの状態 | 送られてきた版 | 結果 |
    /// |---|---|---|
    /// | 無い | — | **INSERT** |
    /// | 有る | 全列が同値 | **何もしない**（収束条件） |
    /// | 有る・請求済み | 差がある | 衝突 `INVOICED_FROZEN`（`force` でも破れない） |
    /// | 有る・基準線より後にこちらも変更 | 差がある | 衝突 `BOTH_CHANGED`（`force` で上書き可） |
    /// | 有る・こちらは触っていない | 差がある | **UPDATE** |
    ///
    /// 「全列が同値なら書かない」は単なる最適化ではない。取り込んだ行は
    /// トリガで outbox に載り、次の同期で送り主へ返る。そこで書き込んで
    /// しまうと両端末が永久に同じ行を送り合う。
    ///
    /// ## 進捗は衝突があっても進める
    ///
    /// 衝突した行のぶんだけ進捗を止めると、**未解決の1行がその後ろの
    /// 全部を堰き止める**。衝突は応答で返しているので、送り主が保存すれば
    /// 失われない。逆に言えば、**保存してから進捗を進めるのは送り主の責任**。
    ///
    /// ## 並べ替え
    ///
    /// 依存順（[`SYNCED_TABLES`] の並び）に直してから流す。pull はこの順で
    /// 返すので通常は既に整っているが、順序を送り手の善意に頼ると
    /// 外部キー違反という分かりにくい形で失敗する。
    pub async fn push(&self, request: PushRequest) -> Result<PushResult, BantoError> {
        let device_id = stored_device_id(&self.settings).await?;
        if request.peer_device_id == device_id {
            return Err(same_device_error(device_id));
        }

        // 依存順に並べ替える。目録に無いテーブルはここで弾く。
        let mut ordered: Vec<(&'static crate::sync::rows::TableSpec, &PushRow)> = Vec::new();
        for spec in &SYNCED_TABLES {
            for row in &request.rows {
                if row.row.table == spec.name {
                    ordered.push((spec, row));
                }
            }
        }
        if ordered.len() != request.rows.len() {
            let unknown: Vec<&str> = request
                .rows
                .iter()
                .map(|row| row.row.table.as_str())
                .filter(|table| table_spec(table).is_none())
                .collect();
            return Err(BantoError::Validation {
                field_errors: vec![FieldError {
                    field: "table".to_string(),
                    message: format!("同期対象にないテーブルが送られてきた: {unknown:?}"),
                }],
            });
        }

        let mut applied = 0usize;
        let mut unchanged = 0usize;
        let mut conflicts = Vec::new();

        for (spec, push_row) in ordered {
            let incoming = &push_row.row;
            validate_row(spec, incoming)?;

            let Some(local) = read_row(&self.db, spec, &incoming.key).await? else {
                apply_row(&self.db, spec, incoming, false).await?;
                applied += 1;
                continue;
            };

            if values_equal(spec, &local, incoming) {
                unchanged += 1;
                continue;
            }

            // 請求済みは両端末とも編集できない（決定 C-20）。`force` でも破れない。
            if is_invoiced(&local) {
                conflicts.push(Conflict {
                    table: spec.name.to_string(),
                    key: incoming.key.clone(),
                    reason: ConflictReason::InvoicedFrozen,
                    local,
                    incoming: incoming.clone(),
                });
                continue;
            }

            let changed_here = last_change_seq(&self.db, spec.name, &incoming.key).await?
                > request.pulled_through_seq;
            if changed_here && !push_row.force {
                conflicts.push(Conflict {
                    table: spec.name.to_string(),
                    key: incoming.key.clone(),
                    reason: ConflictReason::BothChanged,
                    local,
                    incoming: incoming.clone(),
                });
                continue;
            }

            apply_row(&self.db, spec, incoming, true).await?;
            applied += 1;
        }

        record_peer_progress(
            &self.db,
            request.peer_device_id,
            request.through_seq,
            request.pulled_through_seq,
        )
        .await?;

        Ok(PushResult {
            device_id,
            applied,
            unchanged,
            conflicts,
            received_through_seq: peer_state(&self.db, request.peer_device_id)
                .await?
                .received_through_seq,
        })
    }
}

/// `invoiced` を持つ表で、その行が請求済みか。列を持たない表では常に偽。
fn is_invoiced(row: &SyncRow) -> bool {
    matches!(row.get(INVOICED), Some(SyncValue::Int(flag)) if *flag != 0)
}

fn same_device_error(device_id: i64) -> BantoError {
    BantoError::Validation {
        field_errors: vec![FieldError {
            field: "peerDeviceId".to_string(),
            message: format!(
                "相手と同じデバイス番号（{device_id}）です。\
                 id の採番レンジが分かれていないため同期できません。\
                 どちらかの設定を変更してください"
            ),
        }],
    }
}

#[cfg(test)]
mod tests_support {
    use super::*;
    use crate::customers::{CustomerInput, DAY_END_OF_MONTH};
    use crate::projects::ProjectInput;

    pub fn customer_input(code: &str, name: &str) -> CustomerInput {
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

    pub fn project_input(code: &str, customer_id: i64) -> ProjectInput {
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

    pub fn service(db: &Db) -> SyncService {
        SyncService::new(db.clone(), SettingsService::new(db.clone()))
    }

    /// 請求済みの工数を1件作る。
    ///
    /// 請求書を組み立てず入力の `invoiced` を立てるのは、ここで確かめたいのが
    /// 「凍結された行が同期でどう扱われるか」だけだから。請求書の側の
    /// 振る舞いは `invoices` のテストが見ている。
    pub async fn seed_invoiced_work_log(db: &Db) -> i64 {
        use crate::masters::{CostRateInput, MastersService};
        use crate::projects::ProjectsService;
        use crate::work_logs::{WorkLogInput, WorkLogsService};

        let customer = crate::customers::CustomersService::new(db.clone())
            .create(customer_input("C001", "架空商事"))
            .await
            .expect("customer");
        let project = ProjectsService::new(db.clone())
            .create(project_input("2026-001", customer.id))
            .await
            .expect("project");
        MastersService::new(db.clone())
            .set_cost_rate(CostRateInput {
                work_category_code: "DESIGN".to_string(),
                hourly_rate: 5000,
            })
            .await
            .expect("cost rate");
        WorkLogsService::new(db.clone())
            .create(WorkLogInput {
                project_id: project.id,
                trip_id: None,
                worked_on: "2026-08-01".to_string(),
                work_category_code: "DESIGN".to_string(),
                minutes: 60,
                applied_rate: None,
                description: None,
                invoiced: true,
            })
            .await
            .expect("work log")
            .id
    }

    /// 1往復ぶんの結果。
    pub struct Exchange {
        pub pc_applied: usize,
        pub phone_applied: usize,
        pub pc_conflicts: usize,
        pub phone_conflicts: usize,
        pub pc_through_seq: i64,
        pub phone_through_seq: i64,
    }

    /// PC ⇄ スマホを1往復させる。
    ///
    /// **どちらも取り込む前に両方から引く。** 引いてから取り込むと、取り込みが
    /// トリガで積んだぶんが同じ往復のうちに相手へ回り、何周目の変化を見て
    /// いるのか分からなくなる。実際の手順（引く → 取り込む → 送る）とも合う。
    pub async fn exchange(
        pc: &SyncService,
        phone: &SyncService,
        pc_after_seq: i64,
        phone_after_seq: i64,
    ) -> Exchange {
        let from_pc = pc
            .pull(PullRequest {
                after_seq: pc_after_seq,
            })
            .await
            .expect("pull from pc");
        let from_phone = phone
            .pull(PullRequest {
                after_seq: phone_after_seq,
            })
            .await
            .expect("pull from phone");

        let phone_result = phone
            .push(PushRequest {
                peer_device_id: 0,
                pulled_through_seq: phone_after_seq,
                through_seq: from_pc.through_seq,
                rows: from_pc
                    .rows
                    .into_iter()
                    .map(|row| PushRow { row, force: false })
                    .collect(),
            })
            .await
            .expect("phone applies");
        let pc_result = pc
            .push(PushRequest {
                peer_device_id: 1,
                pulled_through_seq: from_pc.through_seq,
                through_seq: from_phone.through_seq,
                rows: from_phone
                    .rows
                    .into_iter()
                    .map(|row| PushRow { row, force: false })
                    .collect(),
            })
            .await
            .expect("pc applies");

        Exchange {
            pc_applied: pc_result.applied,
            phone_applied: phone_result.applied,
            pc_conflicts: pc_result.conflicts.len(),
            phone_conflicts: phone_result.conflicts.len(),
            pc_through_seq: from_pc.through_seq,
            phone_through_seq: from_phone.through_seq,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::tests_support::*;
    use super::*;
    use crate::customers::CustomersService;
    use crate::db::migrate_memory;
    use crate::projects::ProjectsService;
    use crate::sync::rows::SyncValue;
    use crate::sync::{set_device_id, DEFAULT_DEVICE_ID};

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

#[cfg(test)]
mod push_tests {
    use super::tests_support::*;
    use super::*;
    use crate::customers::CustomersService;
    use crate::db::migrate_memory;
    use crate::sync::rows::{read_row, table_spec, SyncValue};
    use crate::sync::{ensure_id_range, outbox_head, set_device_id};

    /// 相手側（スマホ想定）の DB。デバイス番号 1 でレンジを分けておく。
    async fn peer_db() -> (Db, SyncService) {
        let db = migrate_memory().await.unwrap();
        let settings = SettingsService::new(db.clone());
        set_device_id(&settings, 1).await.expect("set device id");
        ensure_id_range(&db, 1).await.expect("range");
        let sync = SyncService::new(db.clone(), settings);
        (db, sync)
    }

    fn push(rows: Vec<SyncRow>) -> PushRequest {
        PushRequest {
            peer_device_id: 1,
            pulled_through_seq: 0,
            through_seq: 1,
            rows: rows
                .into_iter()
                .map(|row| PushRow { row, force: false })
                .collect(),
        }
    }

    async fn customers_row(db: &Db, id: i64) -> SyncRow {
        read_row(db, table_spec("customers").unwrap(), &id.to_string())
            .await
            .expect("read")
            .expect("row")
    }

    fn with(row: &SyncRow, column: &str, value: SyncValue) -> SyncRow {
        let mut next = row.clone();
        next.values.insert(column.to_string(), value);
        next
    }

    #[tokio::test]
    async fn an_unknown_row_is_inserted_with_the_senders_own_id() {
        let (peer, _) = peer_db().await;
        let created = CustomersService::new(peer.clone())
            .create(customer_input("C001", "架空商事"))
            .await
            .expect("create");
        // 相手のレンジ（10億〜）で採番されていること。
        assert!(created.id >= 1_000_000_000);
        let row = customers_row(&peer, created.id).await;

        let db = migrate_memory().await.unwrap();
        let result = service(&db)
            .push(push(vec![row.clone()]))
            .await
            .expect("push");
        assert_eq!(result.applied, 1);
        assert_eq!(result.unchanged, 0);
        assert!(!result.has_conflicts());

        // id も日時も送り主の値のまま入ること。
        let landed = customers_row(&db, created.id).await;
        assert_eq!(landed, row);
    }

    /// **収束条件。** 同値の行を書き込むと outbox に載り、次の同期で送り主へ
    /// 返り、また書き込まれ……と両端末が永久に送り合う。
    #[tokio::test]
    async fn an_identical_row_is_not_written_at_all() {
        let (peer, _) = peer_db().await;
        let created = CustomersService::new(peer.clone())
            .create(customer_input("C001", "架空商事"))
            .await
            .expect("create");
        let row = customers_row(&peer, created.id).await;

        let db = migrate_memory().await.unwrap();
        let sync = service(&db);
        sync.push(push(vec![row.clone()]))
            .await
            .expect("first push");
        let head_after_first = outbox_head(&db).await.expect("head");

        let again = sync.push(push(vec![row])).await.expect("second push");
        assert_eq!(again.applied, 0);
        assert_eq!(again.unchanged, 1);
        assert_eq!(
            outbox_head(&db).await.expect("head"),
            head_after_first,
            "同値なら outbox が伸びないこと（伸びると送り合いが終わらない）"
        );
    }

    #[tokio::test]
    async fn a_row_we_never_touched_is_updated() {
        let (peer, _) = peer_db().await;
        let customers = CustomersService::new(peer.clone());
        let created = customers
            .create(customer_input("C001", "架空商事"))
            .await
            .expect("create");
        let row = customers_row(&peer, created.id).await;

        let db = migrate_memory().await.unwrap();
        let sync = service(&db);
        sync.push(push(vec![row.clone()])).await.expect("push");

        // 相手側だけが直した。
        customers
            .update(created.id, customer_input("C001", "架空商事（改称）"))
            .await
            .expect("update");
        let edited = customers_row(&peer, created.id).await;

        let result = sync
            .push(PushRequest {
                // こちらの outbox は取り込みぶんで伸びているが、相手はそれを
                // 取り込み済みという前提（基準線を現在値に置く）。
                pulled_through_seq: outbox_head(&db).await.expect("head"),
                ..push(vec![edited.clone()])
            })
            .await
            .expect("push");
        assert_eq!(result.applied, 1);
        assert!(!result.has_conflicts());
        assert_eq!(customers_row(&db, created.id).await, edited);
    }

    /// 両方が独立に直したら、取り込まずに差し戻す。
    #[tokio::test]
    async fn a_row_changed_on_both_sides_is_returned_as_a_conflict() {
        let (peer, _) = peer_db().await;
        let created = CustomersService::new(peer.clone())
            .create(customer_input("C001", "架空商事"))
            .await
            .expect("create");
        let row = customers_row(&peer, created.id).await;

        let db = migrate_memory().await.unwrap();
        let sync = service(&db);
        sync.push(push(vec![row.clone()])).await.expect("push");

        // こちらでも直す（基準線 0 のまま = 相手はこの変更を見ていない）。
        CustomersService::new(db.clone())
            .update(created.id, customer_input("C001", "PC 側で改称"))
            .await
            .expect("update");

        let incoming = with(&row, "name", SyncValue::Text("スマホ側で改称".to_string()));
        let result = sync.push(push(vec![incoming.clone()])).await.expect("push");

        assert_eq!(result.applied, 0);
        assert_eq!(result.conflicts.len(), 1);
        assert_eq!(result.conflicts[0].reason, ConflictReason::BothChanged);
        assert_eq!(result.conflicts[0].table, "customers");
        assert_eq!(
            result.conflicts[0].local.get("name"),
            Some(&SyncValue::Text("PC 側で改称".to_string()))
        );
        assert_eq!(
            result.conflicts[0].incoming.get("name"),
            Some(&SyncValue::Text("スマホ側で改称".to_string()))
        );
        // 取り込んでいないこと。
        assert_eq!(
            customers_row(&db, created.id).await.get("name"),
            Some(&SyncValue::Text("PC 側で改称".to_string()))
        );
    }

    /// 利用者が「こちらを採る」と選んだら上書きできる。
    #[tokio::test]
    async fn force_resolves_a_both_changed_conflict() {
        let (peer, _) = peer_db().await;
        let created = CustomersService::new(peer.clone())
            .create(customer_input("C001", "架空商事"))
            .await
            .expect("create");
        let row = customers_row(&peer, created.id).await;

        let db = migrate_memory().await.unwrap();
        let sync = service(&db);
        sync.push(push(vec![row.clone()])).await.expect("push");
        CustomersService::new(db.clone())
            .update(created.id, customer_input("C001", "PC 側で改称"))
            .await
            .expect("update");

        let incoming = with(&row, "name", SyncValue::Text("スマホ側で改称".to_string()));
        let result = sync
            .push(PushRequest {
                rows: vec![PushRow {
                    row: incoming.clone(),
                    force: true,
                }],
                ..push(vec![])
            })
            .await
            .expect("push");
        assert_eq!(result.applied, 1);
        assert!(!result.has_conflicts());
        assert_eq!(customers_row(&db, created.id).await, incoming);
    }

    /// **請求済みの凍結は `force` でも破れない**（決定 C-20 / sync.md 6.1）。
    /// 請求書は既に取引先へ渡っている可能性があり、その裏付けを同期の一存で
    /// 書き換えない。取消（赤伝）→ 直す → 再発行の手順へ進んでもらう。
    #[tokio::test]
    async fn an_invoiced_row_is_frozen_even_with_force() {
        let db = migrate_memory().await.unwrap();
        let seeded = seed_invoiced_work_log(&db).await;
        let spec = table_spec("work_logs").expect("spec");
        let local = read_row(&db, spec, &seeded.to_string())
            .await
            .expect("read")
            .expect("row");
        assert_eq!(local.get("invoiced"), Some(&SyncValue::Int(1)));

        let incoming = with(&local, "minutes", SyncValue::Int(999));
        for force in [false, true] {
            let result = service(&db)
                .push(PushRequest {
                    rows: vec![PushRow {
                        row: incoming.clone(),
                        force,
                    }],
                    ..push(vec![])
                })
                .await
                .expect("push");
            assert_eq!(result.applied, 0, "force={force}");
            assert_eq!(result.conflicts.len(), 1, "force={force}");
            assert_eq!(
                result.conflicts[0].reason,
                ConflictReason::InvoicedFrozen,
                "force={force}"
            );
        }
        // 元の値のままであること。
        let after = read_row(&db, spec, &seeded.to_string())
            .await
            .expect("read")
            .expect("row");
        assert_eq!(after, local);
    }

    /// 未解決の1行が、その後ろの全部を堰き止めないこと。
    #[tokio::test]
    async fn the_watermark_advances_even_when_a_row_conflicts() {
        let (peer, _) = peer_db().await;
        let created = CustomersService::new(peer.clone())
            .create(customer_input("C001", "架空商事"))
            .await
            .expect("create");
        let row = customers_row(&peer, created.id).await;

        let db = migrate_memory().await.unwrap();
        let sync = service(&db);
        sync.push(push(vec![row.clone()])).await.expect("push");
        CustomersService::new(db.clone())
            .update(created.id, customer_input("C001", "PC 側で改称"))
            .await
            .expect("update");

        let incoming = with(&row, "name", SyncValue::Text("スマホ側で改称".to_string()));
        let result = sync
            .push(PushRequest {
                through_seq: 42,
                ..push(vec![incoming])
            })
            .await
            .expect("push");
        assert!(result.has_conflicts());
        assert_eq!(result.received_through_seq, 42);
    }

    /// 依存順に並べ替えてから流すこと（送り手の順序に頼らない）。
    #[tokio::test]
    async fn rows_are_reordered_so_parents_land_first() {
        let (peer, _) = peer_db().await;
        let customers = CustomersService::new(peer.clone());
        let customer = customers
            .create(customer_input("C001", "架空商事"))
            .await
            .expect("create");
        let project = crate::projects::ProjectsService::new(peer.clone())
            .create(project_input("2026-001", customer.id))
            .await
            .expect("create");

        let customer_row = customers_row(&peer, customer.id).await;
        let project_row = read_row(
            &peer,
            table_spec("projects").expect("spec"),
            &project.id.to_string(),
        )
        .await
        .expect("read")
        .expect("row");

        // わざと子を先に並べる。
        let db = migrate_memory().await.unwrap();
        let result = service(&db)
            .push(push(vec![project_row, customer_row]))
            .await
            .expect("push");
        assert_eq!(result.applied, 2);
    }

    #[tokio::test]
    async fn an_unknown_table_is_refused() {
        let db = migrate_memory().await.unwrap();
        let row = SyncRow {
            table: "invoices".to_string(),
            key: "1".to_string(),
            values: Default::default(),
        };
        assert!(service(&db).push(push(vec![row])).await.is_err());
    }

    /// **足りない列を既定値で埋めない。** 埋めると、送り手の不具合で列が
    /// 落ちたときに「0 円」が正しい値として入り、同期は成功したように見える。
    #[tokio::test]
    async fn a_row_missing_a_column_is_refused() {
        let (peer, _) = peer_db().await;
        let created = CustomersService::new(peer.clone())
            .create(customer_input("C001", "架空商事"))
            .await
            .expect("create");
        let mut row = customers_row(&peer, created.id).await;
        row.values.remove("closing_day");

        let db = migrate_memory().await.unwrap();
        assert!(service(&db).push(push(vec![row])).await.is_err());
    }

    #[tokio::test]
    async fn a_peer_claiming_our_own_device_number_cannot_push() {
        let db = migrate_memory().await.unwrap();
        let result = service(&db)
            .push(PushRequest {
                peer_device_id: 0,
                ..push(vec![])
            })
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn a_push_row_round_trips_through_json() {
        let row = SyncRow {
            table: "customers".to_string(),
            key: "7".to_string(),
            values: [
                ("id".to_string(), SyncValue::Int(7)),
                ("note".to_string(), SyncValue::Null),
                ("name".to_string(), SyncValue::Text("架空商事".to_string())),
            ]
            .into_iter()
            .collect(),
        };
        let push_row = PushRow {
            row: row.clone(),
            force: true,
        };
        let json = serde_json::to_value(&push_row).expect("serialise");
        assert_eq!(json["table"], "customers");
        assert_eq!(json["force"], true);
        assert_eq!(json["values"]["note"], serde_json::Value::Null);
        assert_eq!(
            serde_json::from_value::<PushRow>(json).expect("deserialise"),
            push_row
        );

        // `force` は省略できる（既定は false）。
        let bare: PushRow = serde_json::from_value(serde_json::json!({
            "table": "customers", "key": "7", "values": { "id": 7 }
        }))
        .expect("deserialise without force");
        assert!(!bare.force);
    }

    /// **2台を通しで回し、2周目が完全な無変更になること。**
    ///
    /// 収束のいちばん素直な確認。片方の取り込みがもう片方へ返り、そこでまた
    /// 書き込まれると、この検査が「2周目も applied > 0」で落ちる。
    #[tokio::test]
    async fn two_devices_converge_and_then_go_quiet() {
        let pc = migrate_memory().await.unwrap();
        let pc_sync = service(&pc);
        let (phone, phone_sync) = peer_db().await;

        CustomersService::new(pc.clone())
            .create(customer_input("C001", "PC で作った顧客"))
            .await
            .expect("create on pc");
        CustomersService::new(phone.clone())
            .create(customer_input("C002", "スマホで作った顧客"))
            .await
            .expect("create on phone");

        // 1周目。
        let first = exchange(&pc_sync, &phone_sync, 0, 0).await;
        assert_eq!(first.pc_applied, 1, "PC はスマホの1件を取り込む");
        assert_eq!(first.phone_applied, 1, "スマホは PC の1件を取り込む");
        assert_eq!(first.pc_conflicts + first.phone_conflicts, 0);

        // 2周目。取り込みぶんが outbox に載っているので行は流れるが、
        // 中身は同値なので1件も書かれないこと。
        let second = exchange(
            &pc_sync,
            &phone_sync,
            first.pc_through_seq,
            first.phone_through_seq,
        )
        .await;
        assert_eq!(second.pc_applied, 0, "2周目に書き込みが起きないこと");
        assert_eq!(second.phone_applied, 0, "2周目に書き込みが起きないこと");
        assert_eq!(second.pc_conflicts + second.phone_conflicts, 0);

        // 両方に2件ずつ在ること。
        for db in [&pc, &phone] {
            let listed = CustomersService::new(db.clone())
                .list(banto_core::ListParams::default())
                .await
                .expect("list");
            assert_eq!(listed.total_count, 2);
        }
    }
}
