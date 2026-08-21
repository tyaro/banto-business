//! 未解決の衝突の入れ物（`docs/domain/sync.md` 11.7、マイグレーション 0025）。
//! conventions §2 に従い `tauri` / `axum` / RBAC を知らない。
//!
//! ## なぜ保存するのか
//!
//! 衝突した行は取り込まれずに応答へ返るだけで、PC は保留状態を持たない。
//! 一方で進捗は衝突があっても進むので、**差し戻された行が次の同期でもう一度
//! 送られてくることはない**。受け取った側がここへ書き留めてから進捗を
//! 進めない限り、その編集は二度と現れない。
//!
//! ## 同じ行が再び揉めたら差し替える
//!
//! 未解決のまま何度も同期すると同じ行が積み上がる。利用者から見れば
//! 「選ぶべきものが1つ」なので、`(table, key)` の未解決は常に1件に保つ。
//! 解決済みは残す —— 同じ行で繰り返し揉めていることが後から分かる。

use banto_core::BantoError;
use banto_storage::Db;
use serde::{Deserialize, Serialize};

use crate::sync::now_text_expr;
use crate::sync::protocol::{Conflict, ConflictReason};
use crate::sync::rows::SyncRow;

/// 保存された衝突1件。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredConflict {
    pub id: i64,
    pub peer_device_id: i64,
    pub table: String,
    pub key: String,
    pub reason: ConflictReason,
    /// こちらが持っている版。
    pub local: SyncRow,
    /// 相手から送られてきた版。
    pub incoming: SyncRow,
    pub detected_at: String,
}

/// 未解決の衝突を数える。同期の結果表示で使う。
pub async fn open_conflict_count(db: &Db) -> Result<i64, BantoError> {
    let sql = "SELECT CAST(COUNT(*) AS BIGINT) FROM sync_conflicts WHERE resolved_at IS NULL";
    match db {
        Db::Sqlite(pool) => sqlx::query_scalar::<_, i64>(sql).fetch_one(pool).await,
        #[cfg(feature = "postgres")]
        Db::Postgres(pool) => sqlx::query_scalar::<_, i64>(sql).fetch_one(pool).await,
    }
    .map_err(banto_storage::storage_error)
}

/// 未解決の衝突を新しい順に読む。
pub async fn open_conflicts(db: &Db) -> Result<Vec<StoredConflict>, BantoError> {
    let sql = "SELECT id, peer_device_id, table_name, row_key, reason, \
               local_row, incoming_row, detected_at \
               FROM sync_conflicts WHERE resolved_at IS NULL ORDER BY id DESC";
    let raw: Vec<StoredRow> = match db {
        Db::Sqlite(pool) => sqlx::query_as::<_, StoredRow>(sql).fetch_all(pool).await,
        #[cfg(feature = "postgres")]
        Db::Postgres(pool) => sqlx::query_as::<_, StoredRow>(sql).fetch_all(pool).await,
    }
    .map_err(banto_storage::storage_error)?;

    raw.into_iter().map(StoredRow::into_conflict).collect()
}

/// 衝突を書き留める。**進捗を刻む前に呼ぶこと**（モジュール冒頭）。
///
/// 同じ行の未解決は先に消してから入れ直す。1件ずつ実行するのは、
/// `sync_conflicts` が同期対象ではなく（トリガも無い）、途中で落ちても
/// 次の同期で同じ衝突が再び差し戻されるだけで壊れないため。
pub async fn record_conflicts(
    db: &Db,
    peer_device_id: i64,
    conflicts: &[Conflict],
) -> Result<(), BantoError> {
    for conflict in conflicts {
        clear_open(db, &conflict.table, &conflict.key).await?;
        insert(db, peer_device_id, conflict).await?;
    }
    Ok(())
}

async fn clear_open(db: &Db, table: &str, key: &str) -> Result<(), BantoError> {
    let dialect = db.dialect();
    let sql = format!(
        "DELETE FROM sync_conflicts \
         WHERE resolved_at IS NULL AND table_name = {} AND row_key = {}",
        dialect.placeholder(1),
        dialect.placeholder(2)
    );
    match db {
        Db::Sqlite(pool) => sqlx::query(&sql)
            .bind(table)
            .bind(key)
            .execute(pool)
            .await
            .map(|_| ()),
        #[cfg(feature = "postgres")]
        Db::Postgres(pool) => sqlx::query(&sql)
            .bind(table)
            .bind(key)
            .execute(pool)
            .await
            .map(|_| ()),
    }
    .map_err(banto_storage::storage_error)
}

async fn insert(db: &Db, peer_device_id: i64, conflict: &Conflict) -> Result<(), BantoError> {
    let dialect = db.dialect();
    let sql = format!(
        "INSERT INTO sync_conflicts \
         (peer_device_id, table_name, row_key, reason, local_row, incoming_row, detected_at) \
         VALUES ({}, {}, {}, {}, {}, {}, {now})",
        dialect.placeholder(1),
        dialect.placeholder(2),
        dialect.placeholder(3),
        dialect.placeholder(4),
        dialect.placeholder(5),
        dialect.placeholder(6),
        now = now_text_expr(dialect)
    );
    let reason = reason_code(conflict.reason);
    let local = serde_json::to_string(&conflict.local).map_err(serialize_error)?;
    let incoming = serde_json::to_string(&conflict.incoming).map_err(serialize_error)?;

    match db {
        Db::Sqlite(pool) => sqlx::query(&sql)
            .bind(peer_device_id)
            .bind(&conflict.table)
            .bind(&conflict.key)
            .bind(reason)
            .bind(&local)
            .bind(&incoming)
            .execute(pool)
            .await
            .map(|_| ()),
        #[cfg(feature = "postgres")]
        Db::Postgres(pool) => sqlx::query(&sql)
            .bind(peer_device_id)
            .bind(&conflict.table)
            .bind(&conflict.key)
            .bind(reason)
            .bind(&local)
            .bind(&incoming)
            .execute(pool)
            .await
            .map(|_| ()),
    }
    .map_err(banto_storage::storage_error)
}

/// DB に入れる理由コード。`ConflictReason` の serde 表現と同じ綴りにして、
/// 画面へ返すときにそのまま往復できるようにする。
fn reason_code(reason: ConflictReason) -> &'static str {
    match reason {
        ConflictReason::BothChanged => "BOTH_CHANGED",
        ConflictReason::InvoicedFrozen => "INVOICED_FROZEN",
    }
}

fn parse_reason(raw: &str) -> Result<ConflictReason, BantoError> {
    match raw {
        "BOTH_CHANGED" => Ok(ConflictReason::BothChanged),
        "INVOICED_FROZEN" => Ok(ConflictReason::InvoicedFrozen),
        other => Err(BantoError::Other(format!(
            "unknown sync conflict reason {other:?}"
        ))),
    }
}

fn serialize_error(error: serde_json::Error) -> BantoError {
    BantoError::Other(format!("failed to serialize a sync row: {error}"))
}

/// `sync_conflicts` の生の行。JSON 2列を解いてから [`StoredConflict`] にする。
#[derive(sqlx::FromRow)]
struct StoredRow {
    id: i64,
    peer_device_id: i64,
    table_name: String,
    row_key: String,
    reason: String,
    local_row: String,
    incoming_row: String,
    detected_at: String,
}

impl StoredRow {
    fn into_conflict(self) -> Result<StoredConflict, BantoError> {
        Ok(StoredConflict {
            id: self.id,
            peer_device_id: self.peer_device_id,
            table: self.table_name,
            key: self.row_key,
            reason: parse_reason(&self.reason)?,
            local: serde_json::from_str(&self.local_row).map_err(deserialize_error)?,
            incoming: serde_json::from_str(&self.incoming_row).map_err(deserialize_error)?,
            detected_at: self.detected_at,
        })
    }
}

fn deserialize_error(error: serde_json::Error) -> BantoError {
    BantoError::Other(format!("failed to read a stored sync row: {error}"))
}
