//! Phase 8: 同期を実行する側（スマホ）。`docs/domain/sync.md` 11節。
//! conventions §2 に従い `tauri` / `axum` / RBAC を知らない。
//!
//! ## 「相手」は3つの入口しか持たない
//!
//! 話しかけるのは常にスマホ側で、PC は受け身の handshake / pull / push だけを
//! 持つ（11.1、[`crate::sync::protocol`]）。ここはその3つを**どの順で何回
//! 呼ぶか**を決めるだけで、行の当て方そのものは知らない。
//!
//! ## 取り込みは「自分の push を呼ぶ」
//!
//! 相手から引いた行を自分の DB へ当てるのは、[`crate::sync::protocol::SyncService::push`]
//! そのもの —— 衝突判定も同値判定も進捗の記録も、受け口と同じ実装で済む。
//! 引いた行を当てる専用の経路を別に書くと、**衝突の判定規則が2箇所になる**。
//!
//! ## 手順
//!
//! ```text
//!   handshake              互いの番号と、相手がどこまで取り込んだか
//!      │
//!      ├─ 引く（相手 → 自分）   相手の pull を自分の push へ流す
//!      │
//!      └─ 送る（自分 → 相手）   自分の pull を相手の push へ流す
//! ```
//!
//! **引くのが先。** 相手の版を見てから送ると、こちらの編集が「相手の版を
//! 見ないまま直した」に見えなくなる行が減る（＝衝突が減る）。逆順にすると、
//! 送った直後に引いた行が衝突として差し戻される。
//!
//! ## 送る量を先に締め切る
//!
//! 引いた行を当てるとトリガが自分の outbox へ積む。そのまま送ると、
//! **今引いたばかりの行をそのまま送り返す**ことになる。相手は同値と判定して
//! 書かないので壊れはしないが、毎回の同期が無駄に往復する。
//!
//! そこで**引く前に自分の outbox の頭を控え**、送るのはそこまでに限る。
//! 控えより後に積まれたものは次回の同期で送られる（それが自分の編集なら
//! 送るべきだし、今取り込んだ行の写しなら相手が既に持っている）。
//!
//! ## 衝突は進捗を進める前に書き留める
//!
//! 差し戻された行は次の同期でもう一度送られてこない（11.7）。
//! [`crate::sync::conflicts`] へ保存してから進捗が進むよう、取り込みの直後に
//! 記録する。

use banto_core::{BantoError, FieldError};
use banto_storage::Db;
use serde::{Deserialize, Serialize};
use std::future::Future;

use crate::sync::conflicts::record_conflicts;
use crate::sync::protocol::{
    Conflict, Handshake, HandshakeRequest, Pull, PullRequest, PushRequest, PushResult, PushRow,
    SyncService,
};
use crate::sync::{outbox_head, peer_state, stored_device_id};

/// 相手端末の3つの入口。
///
/// トレイトにするのは**試験のため**。実物は HTTP だが、テストでは同じ
/// プロセスの [`SyncService`] を直に呼ぶ実装を挿して、ネットワークを
/// 立てずに往復を確かめる（`docs/domain/sync.md` 11節の「PC 2台で先に
/// 試す」と同じ考え方をプロセス内へ持ち込んだもの）。
///
/// `async fn` ではなく `impl Future + Send` を返すのは、この future が
/// Tauri コマンドの中で `await` されるため —— 既定の `async fn` in trait は
/// `Send` を約束しない。
pub trait SyncPeer {
    fn handshake(
        &self,
        request: HandshakeRequest,
    ) -> impl Future<Output = Result<Handshake, BantoError>> + Send;

    fn pull(&self, request: PullRequest) -> impl Future<Output = Result<Pull, BantoError>> + Send;

    fn push(
        &self,
        request: PushRequest,
    ) -> impl Future<Output = Result<PushResult, BantoError>> + Send;
}

/// 1回の同期の結果。画面へそのまま出す。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncOutcome {
    /// こちらの番号と相手の番号。取り違えに気付けるよう両方返す。
    pub device_id: i64,
    pub peer_device_id: i64,
    /// 相手から引いて**書いた**行数。
    pub pulled_applied: usize,
    /// 相手から引いたが同値だったので書かなかった行数。
    pub pulled_unchanged: usize,
    /// 相手へ送って**相手が書いた**行数。
    pub pushed_applied: usize,
    /// 相手へ送ったが相手側で同値だった行数。
    pub pushed_unchanged: usize,
    /// この同期で新たに差し戻された衝突の件数。
    pub conflicts_detected: usize,
    /// 保存されている未解決の衝突の総数（過去のぶんを含む）。
    pub open_conflicts: i64,
}

impl SyncOutcome {
    /// 何も動かなかったか。画面で「変更はありません」を出すのに使う。
    pub fn is_quiet(&self) -> bool {
        self.pulled_applied == 0 && self.pushed_applied == 0 && self.conflicts_detected == 0
    }
}

/// 同期を実行する。
///
/// `local` は自分の [`SyncService`]、`peer` は相手。手順の全体は
/// モジュール冒頭を参照。
pub async fn run_sync<P: SyncPeer>(
    local: &SyncService,
    peer: &P,
) -> Result<SyncOutcome, BantoError> {
    let db = local.db();
    let device_id = stored_device_id(local.settings()).await?;

    // 送る量の締め切り（モジュール冒頭）。引く前に控える。
    let send_ceiling = outbox_head(db).await?;

    let shaken = peer
        .handshake(HandshakeRequest {
            peer_device_id: device_id,
        })
        .await?;
    if shaken.device_id == device_id {
        return Err(same_device_error(device_id));
    }
    ensure_known_tables(&shaken)?;

    let peer_device_id = shaken.device_id;
    let mut outcome = SyncOutcome {
        device_id,
        peer_device_id,
        pulled_applied: 0,
        pulled_unchanged: 0,
        pushed_applied: 0,
        pushed_unchanged: 0,
        conflicts_detected: 0,
        open_conflicts: 0,
    };

    // --- 引く（相手 → 自分）---
    //
    // `after_seq` はこちらの控え。相手が「送った」と記録した値は使わない
    // （進捗の持ち主は受け取る側、11.4）。
    let mut taken_through = peer_state(db, peer_device_id).await?.received_through_seq;
    loop {
        let pulled = peer
            .pull(PullRequest {
                after_seq: taken_through,
            })
            .await?;
        let has_more = pulled.has_more();
        let through_seq = pulled.through_seq;
        let applied = local
            .push(PushRequest {
                peer_device_id,
                // 相手がこちらの outbox をどこまで取り込んだか。ここより後に
                // こちらでも同じ行が変わっていれば「両方で直した」。
                pulled_through_seq: shaken.received_through_seq,
                through_seq,
                rows: into_push_rows(pulled.rows),
            })
            .await?;

        // 進捗は `push` の中で刻まれる。保存はその後でも、次に引く範囲は
        // `taken_through` で決まるので取りこぼさない —— ただし衝突は
        // 差し戻されたきりなので、ここで必ず書き留める。
        save_conflicts(db, peer_device_id, &applied.conflicts, &mut outcome).await?;
        outcome.pulled_applied += applied.applied;
        outcome.pulled_unchanged += applied.unchanged;

        taken_through = through_seq;
        if !has_more {
            break;
        }
    }

    // --- 送る（自分 → 相手）---
    let mut sent_through = shaken.received_through_seq;
    while sent_through < send_ceiling {
        let mine = local
            .pull(PullRequest {
                after_seq: sent_through,
            })
            .await?;
        if mine.rows.is_empty() && mine.through_seq == sent_through {
            // 相手が知らない変更が残っていない。`send_ceiling` は控えた
            // 時点の値なので、その後に墓石だけが積まれた等でここへ来うる。
            break;
        }
        let accepted = peer
            .push(PushRequest {
                peer_device_id: device_id,
                // こちらが相手の outbox をどこまで取り込んだか（引く段の結果）。
                pulled_through_seq: taken_through,
                through_seq: mine.through_seq,
                rows: into_push_rows(mine.rows),
            })
            .await?;

        save_conflicts(db, peer_device_id, &accepted.conflicts, &mut outcome).await?;
        outcome.pushed_applied += accepted.applied;
        outcome.pushed_unchanged += accepted.unchanged;

        sent_through = mine.through_seq;
    }

    outcome.open_conflicts = crate::sync::conflicts::open_conflict_count(db).await?;
    Ok(outcome)
}

/// 差し戻された衝突を書き留め、件数を数える。
async fn save_conflicts(
    db: &Db,
    peer_device_id: i64,
    conflicts: &[Conflict],
    outcome: &mut SyncOutcome,
) -> Result<(), BantoError> {
    if conflicts.is_empty() {
        return Ok(());
    }
    record_conflicts(db, peer_device_id, conflicts).await?;
    outcome.conflicts_detected += conflicts.len();
    Ok(())
}

fn into_push_rows(rows: Vec<crate::sync::rows::SyncRow>) -> Vec<PushRow> {
    rows.into_iter()
        .map(|row| PushRow { row, force: false })
        .collect()
}

/// 相手が知らないテーブルを持っていないか。
///
/// 片方だけ更新した端末どうしを繋ぐと、新しいテーブルの行が黙って落ちる。
/// 落としたことは outbox にも残らないので、**触る前に**止める。
fn ensure_known_tables(shaken: &Handshake) -> Result<(), BantoError> {
    let mine: Vec<&str> = crate::sync::rows::SYNCED_TABLES
        .iter()
        .map(|spec| spec.name)
        .collect();
    let missing: Vec<&str> = mine
        .iter()
        .copied()
        .filter(|name| !shaken.tables.iter().any(|table| table == name))
        .collect();
    let extra: Vec<&str> = shaken
        .tables
        .iter()
        .filter(|table| !mine.iter().any(|name| name == table))
        .map(|table| table.as_str())
        .collect();
    if missing.is_empty() && extra.is_empty() {
        return Ok(());
    }
    Err(BantoError::Validation {
        field_errors: vec![FieldError {
            field: "tables".to_string(),
            message: format!(
                "同期対象のテーブルが一致しない（相手に無い: {missing:?} / 相手だけにある: {extra:?}）。\
                 どちらかのアプリを更新すること"
            ),
        }],
    })
}

fn same_device_error(device_id: i64) -> BantoError {
    BantoError::Validation {
        field_errors: vec![FieldError {
            field: crate::sync::DEVICE_ID_KEY.to_string(),
            message: format!(
                "相手も同じデバイス番号 {device_id} を名乗っている。\
                 id レンジが分かれていないので同期できない"
            ),
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::customers::{CustomerInput, CustomersService, DAY_END_OF_MONTH};
    use crate::db::migrate_memory;
    use crate::settings::SettingsService;
    use crate::sync::conflicts::open_conflicts;
    use crate::sync::{ensure_id_range, set_device_id};

    /// 同じプロセスの [`SyncService`] を相手に見立てる。
    ///
    /// HTTP を挟まないので、確かめているのは**手順**（何をどの順で何回
    /// 呼ぶか）だけ。ヘッダやトークンの取り回しは `http_peer` の担当で、
    /// そちらは実サーバを立てるテストが見る。
    struct InProcessPeer {
        service: SyncService,
    }

    impl SyncPeer for InProcessPeer {
        async fn handshake(&self, request: HandshakeRequest) -> Result<Handshake, BantoError> {
            self.service.handshake(request).await
        }

        async fn pull(&self, request: PullRequest) -> Result<Pull, BantoError> {
            self.service.pull(request).await
        }

        async fn push(&self, request: PushRequest) -> Result<PushResult, BantoError> {
            self.service.push(request).await
        }
    }

    /// PC（番号 0）とスマホ（番号 1）を1台ずつ用意する。
    async fn two_devices() -> (Device, Device) {
        let pc = device(0).await;
        let phone = device(1).await;
        (pc, phone)
    }

    struct Device {
        db: Db,
        service: SyncService,
    }

    async fn device(device_id: i64) -> Device {
        let db = migrate_memory().await.expect("migrate");
        let settings = SettingsService::new(db.clone());
        set_device_id(&settings, device_id)
            .await
            .expect("device id");
        ensure_id_range(&db, device_id).await.expect("range");
        let service = SyncService::new(db.clone(), settings);
        Device { db, service }
    }

    /// スマホ側から1回同期する。
    async fn sync(phone: &Device, pc: &Device) -> SyncOutcome {
        let peer = InProcessPeer {
            service: pc.service.clone(),
        };
        run_sync(&phone.service, &peer).await.expect("sync")
    }

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

    async fn add_customer(device: &Device, code: &str, name: &str) -> i64 {
        CustomersService::new(device.db.clone())
            .create(customer_input(code, name))
            .await
            .expect("customer")
            .id
    }

    async fn customer_names(device: &Device) -> Vec<String> {
        let rows = CustomersService::new(device.db.clone())
            .list(Default::default())
            .await
            .expect("list");
        rows.rows.into_iter().map(|row| row.name).collect()
    }

    /// 両端末の行が、1回の同期で互いへ渡る。
    #[tokio::test]
    async fn one_sync_carries_rows_both_ways() {
        let (pc, phone) = two_devices().await;
        add_customer(&pc, "C001", "架空商事").await;
        add_customer(&phone, "C900", "架空製作所").await;

        let outcome = sync(&phone, &pc).await;

        assert_eq!(outcome.device_id, 1);
        assert_eq!(outcome.peer_device_id, 0);
        assert_eq!(outcome.pulled_applied, 1, "PC の行を1件取り込む");
        assert_eq!(outcome.pushed_applied, 1, "スマホの行を1件送る");
        assert_eq!(outcome.conflicts_detected, 0);

        let mut pc_names = customer_names(&pc).await;
        let mut phone_names = customer_names(&phone).await;
        pc_names.sort();
        phone_names.sort();
        assert_eq!(pc_names, vec!["架空商事", "架空製作所"]);
        assert_eq!(phone_names, pc_names);
    }

    /// **2回目は何も動かない。** 取り込んだ行をそのまま送り返して往復し
    /// 続けると、同期のたびに「変更あり」と出て実態と合わなくなる。
    #[tokio::test]
    async fn a_second_sync_is_quiet() {
        let (pc, phone) = two_devices().await;
        add_customer(&pc, "C001", "架空商事").await;
        add_customer(&phone, "C900", "架空製作所").await;
        sync(&phone, &pc).await;

        let again = sync(&phone, &pc).await;

        assert!(again.is_quiet(), "2回目で動きが残っている: {again:?}");
        assert_eq!(again.pulled_applied, 0);
        assert_eq!(again.pushed_applied, 0);
    }

    /// 何も無い状態で同期しても落ちない（初回起動でボタンを押した場合）。
    #[tokio::test]
    async fn syncing_two_empty_devices_does_nothing() {
        let (pc, phone) = two_devices().await;
        let outcome = sync(&phone, &pc).await;
        assert!(outcome.is_quiet());
        assert_eq!(outcome.open_conflicts, 0);
    }

    /// 同じ行を両方で直すと衝突になり、**保存される**。
    ///
    /// 保存を確かめるのがこのテストの主眼 —— 進捗は衝突があっても進むので、
    /// 書き留め損ねるとその編集は二度と現れない（11.7）。
    #[tokio::test]
    async fn a_row_changed_on_both_devices_is_recorded_as_a_conflict() {
        let (pc, phone) = two_devices().await;
        let id = add_customer(&pc, "C001", "架空商事").await;
        sync(&phone, &pc).await;

        // 同期後に、同じ顧客を双方で別の名前へ直す。
        for (device, name) in [
            (&pc, "架空商事（PC で改名）"),
            (&phone, "架空商事（スマホで改名）"),
        ] {
            CustomersService::new(device.db.clone())
                .update(id, customer_input("C001", name))
                .await
                .expect("update");
        }

        let outcome = sync(&phone, &pc).await;

        assert_eq!(outcome.conflicts_detected, 1);
        assert_eq!(outcome.open_conflicts, 1);

        let stored = open_conflicts(&phone.db).await.expect("conflicts");
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].table, "customers");
        assert_eq!(stored[0].key, id.to_string());
    }

    /// 同じ行が繰り返し揉めても、未解決は1件のまま。
    #[tokio::test]
    async fn the_same_row_conflicting_twice_stays_one_open_entry() {
        let (pc, phone) = two_devices().await;
        let id = add_customer(&pc, "C001", "架空商事").await;
        sync(&phone, &pc).await;

        for round in 1..=2 {
            for (device, side) in [(&pc, "PC"), (&phone, "スマホ")] {
                CustomersService::new(device.db.clone())
                    .update(
                        id,
                        customer_input("C001", &format!("架空商事 {side}{round}")),
                    )
                    .await
                    .expect("update");
            }
            sync(&phone, &pc).await;
        }

        let stored = open_conflicts(&phone.db).await.expect("conflicts");
        assert_eq!(stored.len(), 1, "未解決が積み上がっている: {stored:?}");
    }

    /// 相手が自分と同じ番号を名乗ったら、**触る前に**止める。
    #[tokio::test]
    async fn syncing_with_a_device_that_shares_our_number_is_refused() {
        let pc = device(0).await;
        let twin = device(0).await;
        let peer = InProcessPeer {
            service: pc.service.clone(),
        };

        let error = run_sync(&twin.service, &peer)
            .await
            .expect_err("同じ番号なら拒否する");

        assert!(matches!(error, BantoError::Validation { .. }), "{error:?}");
    }
}
