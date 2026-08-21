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
use serde::{Deserialize, Serialize};

pub mod protocol;
pub mod rows;

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
    now_text_expr(dialect)
}

/// 現在日時を **TEXT として**返す SQL 式（両方言）。
///
/// `Dialect::now_expr()` は PostgreSQL で `NOW()`（timestamptz）を返すので、
/// TEXT 列にそのまま入れると型エラーになる。SQLite に日時型が無く、この
/// アプリの日時列がすべて TEXT であるための差。
pub fn now_text_expr(dialect: banto_storage::Dialect) -> &'static str {
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

/// outbox の1件（`docs/domain/sync.md` 4節）。
///
/// 書き込みは **DB トリガ**が行う（`migrations-*/0024_sync_outbox_triggers.sql`）。
/// サービス層から積む形にしなかった理由はそのマイグレーションの冒頭に書いた ——
/// 要点は、書き込みの入口が19箇所あり、1箇所忘れるとその変更が永久に同期
/// されないため。ここにあるのは**読み取り側**だけ。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct OutboxEntry {
    /// 端末内で単調増加する連番。同期の watermark。
    pub seq: i64,
    #[sqlx(rename = "table_name")]
    pub table_name: String,
    /// 整数 PK は10進文字列、コード PK はコードそのもの
    /// （`docs/domain/sync.md` 3.0）。
    #[sqlx(rename = "row_key")]
    pub row_key: String,
    /// `INSERT` / `UPDATE` / `DELETE`。`DELETE` は論理削除
    /// （`deleted_at` が NULL から非 NULL になった UPDATE）を指す。
    pub op: String,
    #[sqlx(rename = "changed_at")]
    pub changed_at: String,
}

/// 同期で1回に送る最大件数。
///
/// 上限を置くのは、初回同期で全件が1つの応答に載るのを避けるため。
/// 続きは「最後に受け取った `seq`」から次を要求する。
pub const OUTBOX_PAGE_SIZE: i64 = 500;

const OUTBOX_COLUMNS: &str = "seq, table_name, row_key, op, changed_at";

/// `after_seq` より後の変更を古い順に返す（最大 [`OUTBOX_PAGE_SIZE`] 件）。
///
/// **時計に依存しない。** `seq` は端末内で単調増加するので、相手が持っている
/// 最後の `seq` を渡せば続きが漏れなく取れる。`updated_at` を watermark に
/// 使わない理由は `docs/domain/sync.md` 4節。
pub async fn outbox_since(db: &Db, after_seq: i64) -> Result<Vec<OutboxEntry>, BantoError> {
    let dialect = db.dialect();
    let sql = format!(
        "SELECT {OUTBOX_COLUMNS} FROM sync_outbox WHERE seq > {} \
         ORDER BY seq LIMIT {OUTBOX_PAGE_SIZE}",
        dialect.placeholder(1)
    );
    match db {
        Db::Sqlite(pool) => {
            sqlx::query_as::<_, OutboxEntry>(&sql)
                .bind(after_seq)
                .fetch_all(pool)
                .await
        }
        #[cfg(feature = "postgres")]
        Db::Postgres(pool) => {
            sqlx::query_as::<_, OutboxEntry>(&sql)
                .bind(after_seq)
                .fetch_all(pool)
                .await
        }
    }
    .map_err(banto_storage::storage_error)
}

/// この端末の outbox の最後の `seq`（空なら 0）。
pub async fn outbox_head(db: &Db) -> Result<i64, BantoError> {
    const SQL: &str = "SELECT CAST(COALESCE(MAX(seq), 0) AS BIGINT) FROM sync_outbox";
    match db {
        Db::Sqlite(pool) => sqlx::query_scalar::<_, i64>(SQL).fetch_one(pool).await,
        #[cfg(feature = "postgres")]
        Db::Postgres(pool) => sqlx::query_scalar::<_, i64>(SQL).fetch_one(pool).await,
    }
    .map_err(banto_storage::storage_error)
}

/// 相手端末ごとの同期の進捗（`sync_state`、`docs/domain/sync.md` 4節）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct PeerState {
    #[sqlx(rename = "peer_device_id")]
    pub peer_device_id: i64,
    /// 自分の outbox のうち、相手へ送り終えたと記録している最後の seq。
    ///
    /// **同期の判断には使わない。** どこまで届いたかを正しく知っているのは
    /// 受け取った側だけなので、送る量はいつも相手が指定する `after_seq` で
    /// 決める（`protocol` モジュールの冒頭を参照）。ここにあるのは
    /// 「前回どこまで送ったか」の控えで、用途は診断のみ。
    #[sqlx(rename = "sent_through_seq")]
    pub sent_through_seq: i64,
    /// 相手の outbox のうち、こちらが取り込み終えた最後の seq。
    #[sqlx(rename = "received_through_seq")]
    pub received_through_seq: i64,
    #[sqlx(rename = "last_synced_at")]
    pub last_synced_at: Option<String>,
}

impl PeerState {
    /// まだ一度も同期していない相手。
    pub fn unsynced(peer_device_id: i64) -> Self {
        Self {
            peer_device_id,
            sent_through_seq: 0,
            received_through_seq: 0,
            last_synced_at: None,
        }
    }
}

/// 相手端末の進捗を読む。行が無ければ [`PeerState::unsynced`]。
///
/// 行が無いことを**エラーにしない**のは、初回の同期がまさにその状態だから。
/// 0 から引き直すのは正しい振る舞いで、多く送るだけで壊れない。
pub async fn peer_state(db: &Db, peer_device_id: i64) -> Result<PeerState, BantoError> {
    if !is_valid_device_id(peer_device_id) {
        return Err(invalid_device_id(peer_device_id));
    }
    let dialect = db.dialect();
    let sql = format!(
        "SELECT peer_device_id, sent_through_seq, received_through_seq, last_synced_at \
         FROM sync_state WHERE peer_device_id = {}",
        dialect.placeholder(1)
    );
    let found = match db {
        Db::Sqlite(pool) => {
            sqlx::query_as::<_, PeerState>(&sql)
                .bind(peer_device_id)
                .fetch_optional(pool)
                .await
        }
        #[cfg(feature = "postgres")]
        Db::Postgres(pool) => {
            sqlx::query_as::<_, PeerState>(&sql)
                .bind(peer_device_id)
                .fetch_optional(pool)
                .await
        }
    }
    .map_err(banto_storage::storage_error)?;

    Ok(found.unwrap_or_else(|| PeerState::unsynced(peer_device_id)))
}

/// この端末の outbox に記録された、その行の最後の `seq`（無ければ 0）。
///
/// 取り込み時の衝突判定に使う。相手が取り込み終えた seq より後にこちら側でも
/// 変わっていれば、**両方が独立に直した**ということになる
/// （`docs/domain/sync.md` 6節）。
pub async fn last_change_seq(db: &Db, table: &str, row_key: &str) -> Result<i64, BantoError> {
    let dialect = db.dialect();
    let sql = format!(
        "SELECT CAST(COALESCE(MAX(seq), 0) AS BIGINT) FROM sync_outbox \
         WHERE table_name = {} AND row_key = {}",
        dialect.placeholder(1),
        dialect.placeholder(2)
    );
    match db {
        Db::Sqlite(pool) => {
            sqlx::query_scalar::<_, i64>(&sql)
                .bind(table)
                .bind(row_key)
                .fetch_one(pool)
                .await
        }
        #[cfg(feature = "postgres")]
        Db::Postgres(pool) => {
            sqlx::query_scalar::<_, i64>(&sql)
                .bind(table)
                .bind(row_key)
                .fetch_one(pool)
                .await
        }
    }
    .map_err(banto_storage::storage_error)
}

/// 相手端末との進捗を刻む。
///
/// **下げない**（`max` / `GREATEST` を取る）。応答が前後したり、古い要求が
/// 遅れて届いたりしたときに巻き戻すと、取り込み済みの範囲をもう一度
/// 取り込みに行くことになる。多く取り込むこと自体は同値判定で吸収されるが、
/// 巻き戻しは衝突の再提示を招くので避ける。
pub async fn record_peer_progress(
    db: &Db,
    peer_device_id: i64,
    received_through_seq: i64,
    sent_through_seq: i64,
) -> Result<(), BantoError> {
    if !is_valid_device_id(peer_device_id) {
        return Err(invalid_device_id(peer_device_id));
    }
    let dialect = db.dialect();
    // 引き上げだけを行う関数名が方言で違う（SQLite は 2 引数の `max`）。
    let greatest = match dialect {
        banto_storage::Dialect::Sqlite => "max",
        banto_storage::Dialect::Postgres => "GREATEST",
    };
    let sql = format!(
        "INSERT INTO sync_state \
         (peer_device_id, sent_through_seq, received_through_seq, last_synced_at) \
         VALUES ({}, {}, {}, {now}) \
         ON CONFLICT (peer_device_id) DO UPDATE SET \
         sent_through_seq = {greatest}(sync_state.sent_through_seq, excluded.sent_through_seq), \
         received_through_seq = \
         {greatest}(sync_state.received_through_seq, excluded.received_through_seq), \
         last_synced_at = excluded.last_synced_at",
        dialect.placeholder(1),
        dialect.placeholder(2),
        dialect.placeholder(3),
        now = now_text_expr(dialect)
    );
    match db {
        Db::Sqlite(pool) => sqlx::query(&sql)
            .bind(peer_device_id)
            .bind(sent_through_seq)
            .bind(received_through_seq)
            .execute(pool)
            .await
            .map(|_| ()),
        #[cfg(feature = "postgres")]
        Db::Postgres(pool) => sqlx::query(&sql)
            .bind(peer_device_id)
            .bind(sent_through_seq)
            .bind(received_through_seq)
            .execute(pool)
            .await
            .map(|_| ()),
    }
    .map_err(banto_storage::storage_error)
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

    // --- outbox（記録は DB トリガ） ---

    /// 素の作成・更新・論理削除が INSERT / UPDATE / DELETE として並ぶこと。
    #[tokio::test]
    async fn the_trigger_records_insert_update_and_logical_delete() {
        let db = migrate_memory().await.unwrap();
        let customers = CustomersService::new(db.clone());

        let created = customers
            .create(customer_input("C001", "架空商事"))
            .await
            .expect("create");
        customers
            .update(created.id, customer_input("C001", "架空商事（改称）"))
            .await
            .expect("update");
        customers.delete(created.id).await.expect("delete");

        let entries = outbox_since(&db, 0).await.expect("outbox");
        let ops: Vec<&str> = entries.iter().map(|e| e.op.as_str()).collect();
        assert_eq!(ops, vec!["INSERT", "UPDATE", "DELETE"]);
        assert!(entries.iter().all(|e| e.table_name == "customers"));
        assert!(
            entries.iter().all(|e| e.row_key == created.id.to_string()),
            "row_key は10進文字列の id: {entries:?}"
        );
        // seq は単調増加。
        assert!(entries.windows(2).all(|w| w[0].seq < w[1].seq));
    }

    /// **サービス層を通らない書き込みも記録されること。**
    ///
    /// トリガにした一番の理由。`trips` の一括生成は `work_logs` / `expenses` を
    /// 直接 INSERT し、`invoices` の確定・取消は `invoiced` を直接 UPDATE する。
    /// サービス層に記録を足す設計だと、この4経路を忘れた時点で
    /// **その変更が永久に同期されない**。
    #[tokio::test]
    async fn writes_that_bypass_the_service_layer_are_recorded_too() {
        let db = migrate_memory().await.unwrap();
        let customers = CustomersService::new(db.clone());
        let customer = customers
            .create(customer_input("C001", "架空商事"))
            .await
            .expect("customer");
        let projects = crate::projects::ProjectsService::new(db.clone());
        let project = projects
            .create(crate::projects::ProjectInput {
                code: "P001".to_string(),
                customer_id: customer.id,
                name: "架空案件".to_string(),
                status: "IN_PROGRESS".to_string(),
                started_on: None,
                due_on: None,
                estimate_amount: None,
                contract_amount: None,
                billing_hourly_rate: Some(10_000),
                scope: None,
                note: None,
            })
            .await
            .expect("project");

        // 一括生成はレートマスタを引くので、先に登録しておく。
        let masters = crate::masters::MastersService::new(db.clone());
        for code in ["TRAVEL", "ONSITE"] {
            masters
                .set_cost_rate(crate::masters::CostRateInput {
                    work_category_code: code.to_string(),
                    hourly_rate: 6_000,
                })
                .await
                .expect("cost rate");
        }

        let before = outbox_head(&db).await.expect("head");

        // 出張の一括生成 —— trips.rs が work_logs / expenses を直接 INSERT する。
        let generated = crate::trips::TripsService::new(db.clone())
            .create(crate::trips::TripInput {
                project_id: project.id,
                destination: "架空市".to_string(),
                start_on: "2026-09-01".to_string(),
                end_on: "2026-09-03".to_string(),
                onsite_days: 3,
                nights: 2,
                note: None,
                generate: Some(crate::trips::TripGenerationInput {
                    travel_minutes_one_way: 180,
                    onsite_minutes_per_day: 480,
                    transport_amount: 24_000,
                    lodging_amount_per_night: 9_500,
                    billable: true,
                }),
            })
            .await
            .expect("trip generation");

        let entries = outbox_since(&db, before).await.expect("outbox");
        let count = |table: &str| entries.iter().filter(|e| e.table_name == table).count();
        assert_eq!(count("trips"), 1, "出張そのもの");
        assert_eq!(
            count("work_logs"),
            generated.travel_work_logs + generated.onsite_work_logs,
            "生成した工数が記録されていない: {entries:?}"
        );
        assert_eq!(
            count("expenses"),
            generated.expenses,
            "生成した経費が記録されていない"
        );
        assert!(
            entries.iter().all(|e| e.op == "INSERT"),
            "生成は全て INSERT のはず: {entries:?}"
        );
    }

    /// 請求の確定・取消による `invoiced` の書き換えも記録される
    /// （`invoices.rs` がサービス層を通さず UPDATE する経路）。
    #[tokio::test]
    async fn issuing_an_invoice_records_the_source_rows_it_flips() {
        let db = migrate_memory().await.unwrap();
        let customers = CustomersService::new(db.clone());
        let customer = customers
            .create(customer_input("C001", "架空商事"))
            .await
            .expect("customer");
        let projects = crate::projects::ProjectsService::new(db.clone());
        let project = projects
            .create(crate::projects::ProjectInput {
                code: "P001".to_string(),
                customer_id: customer.id,
                name: "架空案件".to_string(),
                status: "IN_PROGRESS".to_string(),
                started_on: None,
                due_on: None,
                estimate_amount: None,
                contract_amount: None,
                billing_hourly_rate: Some(10_000),
                scope: None,
                note: None,
            })
            .await
            .expect("project");
        let work_logs = crate::work_logs::WorkLogsService::new(db.clone());
        let work_log = work_logs
            .create(crate::work_logs::WorkLogInput {
                project_id: project.id,
                trip_id: None,
                worked_on: "2026-08-20".to_string(),
                work_category_code: "DESIGN".to_string(),
                minutes: 60,
                applied_rate: Some(6_000),
                description: None,
                invoiced: false,
            })
            .await
            .expect("work log");

        let invoices = crate::invoices::InvoicesService::new(db.clone());
        let draft = invoices
            .create(crate::invoices::InvoiceInput {
                customer_id: customer.id,
                closing_on: None,
                due_on: None,
                corrected_invoice_id: None,
                note: None,
                lines: vec![crate::invoices::InvoiceLineInput {
                    project_id: project.id,
                    item_name: "設計".to_string(),
                    quantity: 1,
                    unit_price: 10_000,
                    tax_category: "STANDARD_10".to_string(),
                    source_type: Some("WORK_LOG".to_string()),
                    source_id: Some(work_log.id),
                    note: None,
                }],
            })
            .await
            .expect("draft");

        let before_issue = outbox_head(&db).await.expect("head");
        invoices.issue(draft.invoice.id).await.expect("issue");

        let entries = outbox_since(&db, before_issue).await.expect("outbox");
        let flipped = entries
            .iter()
            .find(|e| e.table_name == "work_logs" && e.row_key == work_log.id.to_string())
            .unwrap_or_else(|| panic!("確定が立てた invoiced が記録されていない: {entries:?}"));
        assert_eq!(flipped.op, "UPDATE");

        // 取消も同じく記録される。
        let before_cancel = outbox_head(&db).await.expect("head");
        invoices.cancel(draft.invoice.id).await.expect("cancel");
        let entries = outbox_since(&db, before_cancel).await.expect("outbox");
        assert!(
            entries
                .iter()
                .any(|e| e.table_name == "work_logs" && e.op == "UPDATE"),
            "取消が戻した invoiced が記録されていない: {entries:?}"
        );
    }

    /// `after_seq` より後だけを返し、`seq` の昇順で並ぶこと（同期の watermark）。
    #[tokio::test]
    async fn outbox_since_returns_only_what_follows_the_watermark() {
        let db = migrate_memory().await.unwrap();
        let customers = CustomersService::new(db.clone());
        customers
            .create(customer_input("C001", "架空商事"))
            .await
            .expect("1件目");
        let watermark = outbox_head(&db).await.expect("head");

        customers
            .create(customer_input("C002", "架空工業"))
            .await
            .expect("2件目");

        let entries = outbox_since(&db, watermark).await.expect("outbox");
        assert_eq!(
            entries.len(),
            1,
            "watermark 以前が混ざっている: {entries:?}"
        );
        assert!(entries[0].seq > watermark);

        // 空の DB では head が 0 で、そこから全件が取れる。
        let fresh = migrate_memory().await.unwrap();
        assert_eq!(outbox_head(&fresh).await.unwrap(), 0);
        assert!(outbox_since(&fresh, 0).await.unwrap().is_empty());
    }

    /// コード PK の表は `row_key` にコードそのものが入る（10進文字列ではない）。
    #[tokio::test]
    async fn a_code_keyed_table_records_the_code_as_its_row_key() {
        let db = migrate_memory().await.unwrap();
        let before = outbox_head(&db).await.expect("head");

        crate::masters::MastersService::new(db.clone())
            .set_cost_rate(crate::masters::CostRateInput {
                work_category_code: "DESIGN".to_string(),
                hourly_rate: 6_500,
            })
            .await
            .expect("set_cost_rate");

        let entries = outbox_since(&db, before).await.expect("outbox");
        let rate = entries
            .iter()
            .find(|e| e.table_name == "cost_rates")
            .unwrap_or_else(|| panic!("レートの変更が記録されていない: {entries:?}"));
        assert_eq!(rate.row_key, "DESIGN");
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
