//! Database bootstrap for the admin-template app (spec §12): connect and
//! run embedded migrations.
//!
//! デモデータの投入は行わない。`items` デモリソースを削除した時点で
//! シード対象が無くなったため（`docs/template-origin.md` の派生差分を参照）。
//! 業務データは利用者が入力するものであり、起動時に架空の行を作らない。

use banto_core::BantoError;
use banto_storage::Db;

/// Connect to the **SQLite** database at `path` and run migrations. Used by
/// the `src-tauri` adapter with a
/// path under the app's data directory - desktop is always local SQLite, so
/// this entry point stays SQLite-fixed (V2 "PostgreSQL アプリ全体対応": the
/// Postgres path is opt-in via [`init_db_from_target`], never here).
///
/// Returns a backend-agnostic [`Db`] handle: the service layer takes `Db`, not
/// a concrete pool. The migration helper dispatches on the handle's
/// backend, so this function simply builds the SQLite handle and hands it off.
pub async fn init_db(path: impl AsRef<std::path::Path>) -> Result<Db, BantoError> {
    let db = Db::connect_sqlite(path).await?;
    run_migrations(&db).await?;
    Ok(db)
}

/// Connect using a backend selected by the `target` connection string and run
/// the matching migrations (V2 "PostgreSQL アプリ全体対応", PR3).
///
/// Backend selection is by URL scheme: a `postgres://` / `postgresql://`
/// `target` opens a **PostgreSQL** backend (feature `postgres`); anything else
/// is treated as a **SQLite** filesystem path (the default, byte-for-byte
/// identical to [`init_db`]). This is what `bin/banto-serve.rs` calls with its
/// `BANTO_DB` env value, so pointing `BANTO_DB` at a `postgres://` URL is all
/// it takes to run the whole app on Postgres; a plain path keeps the existing
/// SQLite behavior untouched.
pub async fn init_db_from_target(target: &str) -> Result<Db, BantoError> {
    if is_postgres_url(target) {
        #[cfg(feature = "postgres")]
        {
            let db = Db::connect_postgres(target).await?;
            run_migrations(&db).await?;
            Ok(db)
        }
        #[cfg(not(feature = "postgres"))]
        {
            Err(BantoError::Storage(format!(
                "connection target {target:?} is a PostgreSQL URL but this build was compiled without the `postgres` feature"
            )))
        }
    } else {
        init_db(target).await
    }
}

/// Does `target` name a PostgreSQL server (vs. a SQLite filesystem path)?
/// Matches the two canonical libpq URL schemes.
///
/// `pub` so the single backend-selection rule lives here and is reused rather
/// than duplicated: `bin/banto-serve.rs` calls this to decide whether to skip
/// the SQLite-only startup restore (`BackupService::apply_pending_restore_at_startup`)
/// for a `BANTO_DB` that points at Postgres, keeping that guard in lock-step
/// with [`init_db_from_target`]'s own scheme check below.
pub fn is_postgres_url(target: &str) -> bool {
    target.starts_with("postgres://") || target.starts_with("postgresql://")
}

/// Same as [`init_db`] but against a private in-memory SQLite database. Used by
/// tests so each test gets an isolated, migrated database.
pub async fn init_db_memory() -> Result<Db, BantoError> {
    let db = Db::connect_sqlite_memory().await?;
    run_migrations(&db).await?;
    Ok(db)
}

/// [`init_db_memory`] の crate 内向けの別名。各サービスのテストが
/// 「マイグレーション済み・空の DB」を指す名前として使っている。
#[cfg(test)]
pub(crate) async fn migrate_memory() -> Result<Db, BantoError> {
    init_db_memory().await
}

/// Run the embedded migrations matching the handle's backend. The two migration
/// sets are byte-distinct SQL (`migrations-sqlite/` is the historical DDL kept
/// unchanged for backward compatibility; `migrations-postgres/` is the
/// strict-typed Postgres port), so each backend embeds and runs its own.
async fn run_migrations(db: &Db) -> Result<(), BantoError> {
    match db {
        Db::Sqlite(pool) => sqlx::migrate!("./migrations-sqlite")
            .run(pool)
            .await
            .map_err(|err| BantoError::Storage(err.to_string())),
        #[cfg(feature = "postgres")]
        Db::Postgres(pool) => sqlx::migrate!("./migrations-postgres")
            .run(pool)
            .await
            .map_err(|err| BantoError::Storage(err.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// マイグレーションが通り、業務テーブルが**空で**出来上がること。
    /// デモシードを廃止したので、初回起動直後に行があってはいけない。
    #[tokio::test]
    async fn init_db_memory_migrates_to_an_empty_database() {
        let db = init_db_memory()
            .await
            .expect("init_db_memory should succeed");
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM customers")
            .fetch_one(db.as_sqlite().expect("sqlite handle"))
            .await
            .unwrap();
        assert_eq!(count, 0);
    }

    /// 二度目の初期化でも壊れない（同じ DB へ再度マイグレーションを流す）。
    #[tokio::test]
    async fn migrating_twice_on_the_same_db_is_a_no_op() {
        let db = Db::connect_sqlite_memory().await.unwrap();
        run_migrations(&db).await.unwrap();
        run_migrations(&db).await.unwrap();
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM customers")
            .fetch_one(db.as_sqlite().expect("sqlite handle"))
            .await
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn postgres_url_detection_selects_backend_by_scheme() {
        assert!(is_postgres_url("postgres://user:pass@localhost/db"));
        assert!(is_postgres_url("postgresql://localhost/db"));
        assert!(!is_postgres_url("./banto-dev.sqlite3"));
        assert!(!is_postgres_url("/var/lib/banto/app.sqlite3"));
        assert!(!is_postgres_url("C:\\data\\app.sqlite3"));
    }
}
