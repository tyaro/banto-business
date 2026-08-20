//! Phase 8: デバイス間同期の土台（`docs/domain/sync.md`）。conventions §2 に
//! 従い `tauri` / `axum` / RBAC を知らない。
//!
//! この段では **id の採番レンジとデバイス番号だけ**を扱う。outbox への記録と
//! 論理削除への移行は次の段（サービス層）で行う。列と土台を足す回と、既存の
//! 全参照を書き換える回を混ぜると、レビューで「どの変更がどちらの都合か」が
//! 読めなくなる。
//!
//! ## なぜレンジで分けるのか
//!
//! 双方向同期の対象8テーブルのうち5つ（`customers` / `projects` / `trips` /
//! `work_logs` / `expenses`）は `INTEGER PRIMARY KEY AUTOINCREMENT` で、
//! 2台が独立に採番すると必ず衝突する。残り3つ（`work_categories` /
//! `expense_categories` / `cost_rates`）は利用者が決めるコードが主キーなので
//! 衝突しない —— 同じコードは同じ行を意味する（`docs/domain/sync.md` 3.0）。
//!
//! 端末ごとに重ならないレンジを与えると、**id が全端末で同じ行を指す**。
//! 外部キーの値もそのまま通るので、同期のたびに id を translate する必要が
//! 無い。translate は1箇所間違えると「工数が別の案件に付く」という金額バグに
//! なるので、そもそも要らない設計にする。

use banto_core::{BantoError, FieldError};
use banto_storage::Db;

/// デバイス番号を保持する設定キー。
pub const DEVICE_ID_KEY: &str = "sync.device.id";

/// 1端末あたりの id レンジ幅。
///
/// 10億にしているのは、`i64` の上限（約 9.2×10^18）に対して 90 億台ぶんの
/// 余裕がありつつ、1端末で 10 億行に到達することが個人事業では起こり得ない
/// ため。桁を見れば何番の端末が作った行かが目視で分かるのも利点。
pub const DEVICE_ID_RANGE: i64 = 1_000_000_000;

/// 既定のデバイス番号。**0 は PC（母艦）**。
///
/// 既定を 0 にしているのは、既存の DB がそのまま 1〜 のレンジに収まるため。
/// Phase 7 で実データを入れた後に Phase 8 へ進んでも、id の振り直しが要らない。
pub const DEFAULT_DEVICE_ID: i64 = 0;

/// レンジを持つ（＝整数 PK の）テーブル。
///
/// コード PK の3テーブルはここに入れない（採番しないので分ける対象が無い）。
pub const RANGED_TABLES: [&str; 5] = ["customers", "projects", "trips", "work_logs", "expenses"];

/// デバイス番号 → そのレンジの先頭 id。
///
/// デバイス 0 は 1 から（0 を id に使わないため）。デバイス 1 は
/// 1,000,000,000 から。
pub fn range_start(device_id: i64) -> i64 {
    if device_id == 0 {
        1
    } else {
        device_id * DEVICE_ID_RANGE
    }
}

/// デバイス番号 → そのレンジの最後の id（この値を含む）。
pub fn range_end(device_id: i64) -> i64 {
    (device_id + 1) * DEVICE_ID_RANGE - 1
}

/// id → それを採番したデバイス番号。
///
/// 同期で受け取った行が「相手が作ったもの」か「自分が作って返ってきたもの」
/// かを、id だけで判定できる。
pub fn owning_device(id: i64) -> i64 {
    id / DEVICE_ID_RANGE
}

/// デバイス番号として受け入れられる値か。
///
/// 上限は `i64` を溢れさせない範囲。負数を弾くのは、レンジが負に回ると
/// [`owning_device`] の除算が意図しない向きに丸まるため。
pub fn is_valid_device_id(device_id: i64) -> bool {
    (0..=9_000_000_000).contains(&device_id)
}

fn invalid_device_id(device_id: i64) -> BantoError {
    BantoError::Validation {
        field_errors: vec![FieldError {
            field: DEVICE_ID_KEY.to_string(),
            message: format!("device id must be between 0 and 9000000000, got {device_id}"),
        }],
    }
}

/// このデバイスの番号と、それに対応する採番レンジ。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceRange {
    pub device_id: i64,
    pub first_id: i64,
    pub last_id: i64,
}

impl DeviceRange {
    pub fn new(device_id: i64) -> Result<Self, BantoError> {
        if !is_valid_device_id(device_id) {
            return Err(invalid_device_id(device_id));
        }
        Ok(Self {
            device_id,
            first_id: range_start(device_id),
            last_id: range_end(device_id),
        })
    }

    pub fn contains(&self, id: i64) -> bool {
        (self.first_id..=self.last_id).contains(&id)
    }
}

/// 保存済みのデバイス番号を読む。未設定なら [`DEFAULT_DEVICE_ID`]。
///
/// 壊れた値（数値でない・範囲外）は既定へ倒さずエラーにする。既定へ倒すと、
/// **設定を書き損じた端末が PC のレンジで採番を始めて衝突する** —— 同期して
/// 初めて気付くうえ、その時点では両方に行が入っている。
pub async fn stored_device_id(
    settings: &crate::settings::SettingsService,
) -> Result<i64, BantoError> {
    let Some(raw) = settings.get(DEVICE_ID_KEY).await? else {
        return Ok(DEFAULT_DEVICE_ID);
    };
    let parsed: i64 = raw.trim().parse().map_err(|_| BantoError::Validation {
        field_errors: vec![FieldError {
            field: DEVICE_ID_KEY.to_string(),
            message: format!("device id must be an integer, got {raw:?}"),
        }],
    })?;
    if !is_valid_device_id(parsed) {
        return Err(invalid_device_id(parsed));
    }
    Ok(parsed)
}

/// デバイス番号を保存する。
pub async fn set_device_id(
    settings: &crate::settings::SettingsService,
    device_id: i64,
) -> Result<(), BantoError> {
    if !is_valid_device_id(device_id) {
        return Err(invalid_device_id(device_id));
    }
    settings.set(DEVICE_ID_KEY, &device_id.to_string()).await
}

/// 墓石に入れる時刻の SQL 式（**両方言とも TEXT に落とす**）。
///
/// `deleted_at` は TEXT 列なので、PostgreSQL では明示的に `::text` へ落とす
/// 必要がある。`Dialect::now_expr()` は PostgreSQL で `NOW()`（timestamptz）を
/// 返すため、そのまま TEXT 列へ入れると型エラーになる —— 各サービスが
/// `today_expr` を自前で持ち `CURRENT_DATE::text` と書いているのと同じ理由。
///
/// 業務日付（`worked_on` 等）と違い、こちらは**日付ではなく日時**にする。
/// 墓石は同期の順序を追うための記録で、同じ日に消して作り直した行を
/// 区別できないと困る。
pub fn deleted_at_expr(dialect: banto_storage::Dialect) -> &'static str {
    match dialect {
        banto_storage::Dialect::Sqlite => "datetime('now')",
        banto_storage::Dialect::Postgres => "NOW()::text",
    }
}

/// 論理削除された行を除いた**派生表**を組み立てる（`docs/domain/sync.md` 5節）。
///
/// ```text
/// SELECT id, ... FROM (SELECT * FROM work_logs WHERE deleted_at IS NULL) AS work_logs
/// ```
///
/// 基底の `SELECT ... FROM work_logs` に直接 `WHERE deleted_at IS NULL` を
/// 足せないのは、一覧が `banto_storage::list_query` の `append_where` で
/// **後から `WHERE` 句を継ぎ足す**ため。絞り込み付きの一覧で
/// `... WHERE deleted_at IS NULL WHERE code = ?` になってしまう。
///
/// `append_where` 側に「基底の述語」を渡せるようにするのが本筋だが、
/// あれは同梱している Banto のコードで、Business 都合では書き換えない
/// （`CLAUDE.md` 第3章）。`docs/banto-feedback.md` に記録した。
///
/// 派生表で包むと、外側にどんな絞り込みが付いても**削除済みの行が見えることは
/// 原理的に無い**。フィルタとして差し込む方式（`deletedAt is_null` を
/// `params.filters` へ push する）も考えられるが、`deletedAt` を絞り込み可能な
/// 列として公開することになり、呼び出し側の書き方次第で墓石が見えてしまう。
///
/// 別名を元の表名に揃えているのは、外側の `ORDER BY` や絞り込みが列名を
/// そのまま使えるようにするため。PostgreSQL は派生表に別名を要求する。
pub fn live(table: &str) -> String {
    format!("(SELECT * FROM {table} WHERE deleted_at IS NULL) AS {table}")
}

/// 採番カウンタをこの端末のレンジ先頭まで進める。
///
/// SQLite の `AUTOINCREMENT` は `sqlite_sequence.seq` の次の値を採番するので、
/// **レンジ先頭 − 1** を書き込めば次の INSERT がレンジ先頭になる。
///
/// **下げることは絶対にしない**（`MAX` を取る）。既に採番が進んでいる端末で
/// 巻き戻すと、既存行と同じ id を再発行して主キー衝突か、最悪は別の行の
/// 上書きになる。
///
/// デバイス 0（PC）は既定レンジが 1〜 で、何もしなくても正しく採番される
/// ため触らない。既存 DB の `sqlite_sequence` を無用に書き換えない。
pub async fn ensure_id_range(db: &Db, device_id: i64) -> Result<(), BantoError> {
    if !is_valid_device_id(device_id) {
        return Err(invalid_device_id(device_id));
    }
    if device_id == DEFAULT_DEVICE_ID {
        return Ok(());
    }
    let floor = range_start(device_id) - 1;

    match db {
        Db::Sqlite(pool) => {
            for table in RANGED_TABLES {
                // `sqlite_sequence` は SQLite の内部表で、`name` に UNIQUE 制約が
                // 無い（`ON CONFLICT(name)` は使えない）。まず UPDATE を撃ち、
                // 対象行が無ければ INSERT する。`max(seq, floor)` は 2 引数の
                // スカラ関数で、**引き上げだけ**を行う。
                let update = format!(
                    "UPDATE sqlite_sequence SET seq = max(seq, {floor}) WHERE name = '{table}'"
                );
                let affected = sqlx::query(&update)
                    .execute(pool)
                    .await
                    .map_err(banto_storage::storage_error)?
                    .rows_affected();
                if affected == 0 {
                    // まだ一度も INSERT していない表には行が無い。
                    let insert = format!(
                        "INSERT INTO sqlite_sequence (name, seq) VALUES ('{table}', {floor})"
                    );
                    sqlx::query(&insert)
                        .execute(pool)
                        .await
                        .map_err(banto_storage::storage_error)?;
                }
            }
            Ok(())
        }
        #[cfg(feature = "postgres")]
        Db::Postgres(pool) => {
            for table in RANGED_TABLES {
                // IDENTITY 列のシーケンス名は `pg_get_serial_sequence` から引く
                // （命名規則を直書きしない）。`setval(..., false)` は「次に
                // 発行する値がこれ」の意味。
                let sql = format!(
                    "SELECT setval(pg_get_serial_sequence('{table}', 'id'), \
                     GREATEST({}, COALESCE((SELECT MAX(id) FROM {table}), 0) + 1), false)",
                    floor + 1
                );
                sqlx::query(&sql)
                    .execute(pool)
                    .await
                    .map_err(banto_storage::storage_error)?;
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::customers::{CustomerInput, CustomersService, DAY_END_OF_MONTH};
    use crate::db::migrate_memory;
    use crate::settings::SettingsService;

    // --- 純粋関数 ---

    #[test]
    fn device_zero_starts_at_one_so_an_existing_database_needs_no_renumbering() {
        assert_eq!(range_start(0), 1);
        assert_eq!(range_end(0), 999_999_999);
    }

    #[test]
    fn ranges_do_not_overlap() {
        for device_id in 0..5 {
            assert!(range_start(device_id) <= range_end(device_id));
            assert!(
                range_end(device_id) < range_start(device_id + 1),
                "device {device_id} と {} のレンジが重なっている",
                device_id + 1
            );
        }
    }

    #[test]
    fn owning_device_recovers_the_device_from_an_id() {
        assert_eq!(owning_device(1), 0);
        assert_eq!(owning_device(999_999_999), 0);
        assert_eq!(owning_device(1_000_000_000), 1);
        assert_eq!(owning_device(1_999_999_999), 1);
        assert_eq!(owning_device(2_000_000_000), 2);
    }

    #[test]
    fn a_range_contains_exactly_its_own_ids() {
        let pixel = DeviceRange::new(1).expect("device 1");
        assert!(pixel.contains(1_000_000_000));
        assert!(pixel.contains(1_999_999_999));
        assert!(!pixel.contains(999_999_999));
        assert!(!pixel.contains(2_000_000_000));
    }

    #[test]
    fn an_out_of_range_device_id_is_rejected() {
        assert!(!is_valid_device_id(-1));
        assert!(!is_valid_device_id(9_000_000_001));
        assert!(is_valid_device_id(0));
        assert!(is_valid_device_id(9_000_000_000));
        assert!(DeviceRange::new(-1).is_err());
    }

    // --- 設定 ---

    #[tokio::test]
    async fn an_unset_device_id_defaults_to_the_pc() {
        let db = migrate_memory().await.unwrap();
        let settings = SettingsService::new(db);
        assert_eq!(
            stored_device_id(&settings).await.unwrap(),
            DEFAULT_DEVICE_ID
        );
    }

    #[tokio::test]
    async fn a_device_id_round_trips_through_settings() {
        let db = migrate_memory().await.unwrap();
        let settings = SettingsService::new(db);
        set_device_id(&settings, 1).await.expect("set");
        assert_eq!(stored_device_id(&settings).await.unwrap(), 1);
    }

    /// 壊れた値は既定へ倒さずエラーにする。倒すと、設定を書き損じた端末が
    /// PC のレンジで採番を始め、同期して初めて衝突に気付くことになる。
    #[tokio::test]
    async fn a_corrupt_device_id_is_an_error_not_a_silent_default() {
        let db = migrate_memory().await.unwrap();
        let settings = SettingsService::new(db);
        settings.set(DEVICE_ID_KEY, "ぬるぽ").await.expect("set");
        assert!(stored_device_id(&settings).await.is_err());

        settings.set(DEVICE_ID_KEY, "-5").await.expect("set");
        assert!(stored_device_id(&settings).await.is_err());
    }

    // --- 採番レンジ ---

    async fn customer_ids(db: &Db, count: usize) -> Vec<i64> {
        let customers = CustomersService::new(db.clone());
        let mut ids = Vec::new();
        for i in 0..count {
            ids.push(
                customers
                    .create(CustomerInput {
                        code: format!("C{i:03}"),
                        name: format!("架空商事 {i}"),
                        contact_person: None,
                        address: None,
                        phone: None,
                        email: None,
                        billing_name: None,
                        closing_day: DAY_END_OF_MONTH,
                        payment_month_offset: 1,
                        payment_day: DAY_END_OF_MONTH,
                        note: None,
                    })
                    .await
                    .expect("customer")
                    .id,
            );
        }
        ids
    }

    #[tokio::test]
    async fn device_zero_keeps_numbering_from_one() {
        let db = migrate_memory().await.unwrap();
        ensure_id_range(&db, 0).await.expect("ensure");
        assert_eq!(customer_ids(&db, 2).await, vec![1, 2]);
    }

    #[tokio::test]
    async fn device_one_numbers_inside_its_own_range() {
        let db = migrate_memory().await.unwrap();
        ensure_id_range(&db, 1).await.expect("ensure");
        let ids = customer_ids(&db, 2).await;
        assert_eq!(ids, vec![1_000_000_000, 1_000_000_001]);
        for id in ids {
            assert_eq!(owning_device(id), 1);
        }
    }

    /// **採番カウンタは下げない。** 既に進んでいる端末で巻き戻すと、既存行と
    /// 同じ id を再発行して主キー衝突になる。
    #[tokio::test]
    async fn ensure_id_range_never_rewinds_an_advanced_counter() {
        let db = migrate_memory().await.unwrap();
        ensure_id_range(&db, 2).await.expect("device 2");
        let first = customer_ids(&db, 1).await[0];
        assert_eq!(owning_device(first), 2);

        // 何度呼んでも巻き戻らない（同期のたびに呼ばれても安全）。
        ensure_id_range(&db, 2).await.expect("再実行");
        let customers = CustomersService::new(db.clone());
        let next = customers
            .create(CustomerInput {
                code: "C999".to_string(),
                name: "架空商事 999".to_string(),
                contact_person: None,
                address: None,
                phone: None,
                email: None,
                billing_name: None,
                closing_day: DAY_END_OF_MONTH,
                payment_month_offset: 1,
                payment_day: DAY_END_OF_MONTH,
                note: None,
            })
            .await
            .expect("customer")
            .id;
        assert_eq!(next, first + 1);
    }

    /// **V-11（要件）。** 2台が採番した id が衝突しないこと。
    #[tokio::test]
    async fn ids_from_two_devices_never_collide() {
        let pc = migrate_memory().await.unwrap();
        ensure_id_range(&pc, 0).await.expect("pc");
        let pc_ids = customer_ids(&pc, 5).await;

        let pixel = migrate_memory().await.unwrap();
        ensure_id_range(&pixel, 1).await.expect("pixel");
        let pixel_ids = customer_ids(&pixel, 5).await;

        for id in &pc_ids {
            assert!(
                !pixel_ids.contains(id),
                "id {id} が両端末で採番された: {pc_ids:?} / {pixel_ids:?}"
            );
        }
        assert!(pc_ids.iter().all(|id| owning_device(*id) == 0));
        assert!(pixel_ids.iter().all(|id| owning_device(*id) == 1));
    }

    /// レンジ分けの対象は整数 PK の5テーブルのみ。コード PK の3テーブルは
    /// 採番しないので含めない（`docs/domain/sync.md` 3.0）。
    #[test]
    fn only_the_integer_keyed_tables_are_ranged() {
        assert_eq!(RANGED_TABLES.len(), 5);
        for code_keyed in ["work_categories", "expense_categories", "cost_rates"] {
            assert!(
                !RANGED_TABLES.contains(&code_keyed),
                "{code_keyed} はコード PK なので採番レンジの対象外"
            );
        }
    }
}
