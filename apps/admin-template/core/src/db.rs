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
        // SQLite は `sqlx::migrate!().run()` に直接渡さず、
        // `run_sqlite_migrations` を経由する（理由はそちらのコメント参照:
        // sqlx-sqlite は `-- no-transaction` を解釈しない）。
        Db::Sqlite(pool) => run_sqlite_migrations(pool).await,
        #[cfg(feature = "postgres")]
        Db::Postgres(pool) => sqlx::migrate!("./migrations-postgres")
            .run(pool)
            .await
            .map_err(|err| BantoError::Storage(err.to_string())),
    }
}

/// SQLite 向けの埋め込みマイグレーション適用。`sqlx::Migrator::run()` を
/// そのまま使わず、その中身（`run_direct`, `sqlx-core::migrate::migrator`）を
/// 1点だけ変えて手で再実装している: 各マイグレーションを適用する際、
/// `migration.no_tx` なら**トランザクションに包まず**素の
/// `sqlx::raw_sql(...).execute(&mut *conn)` で流し、そうでなければ従来通り
/// `Migrate::apply`（`self.begin()` で1トランザクションに包む）を使う。
///
/// # なぜ標準の `Migrator::run()` をそのまま使えないか
///
/// `sqlx-sqlite`（本リポジトリが使う 0.8.6 時点）は埋め込みマイグレーションの
/// `-- no-transaction` 指令を**解釈しない**。`Migration::no_tx` フィールド
/// 自体はバックエンド非依存のパーサ（`sqlx-core::migrate::source`、
/// マイグレーションSQLの先頭行が `-- no-transaction` かどうかを見るだけ）が
/// 設定するが、それを実際に読んでトランザクション包みを回避するのは
/// `sqlx-postgres` の `Migrate::apply` 実装だけで、`sqlx-sqlite` の同名実装
/// （`sqlx-sqlite-0.8.6/src/migrate.rs`）は `migration.no_tx` を一切参照せず、
/// 常に `self.begin()` でマイグレーション全体を1トランザクションに包む。
///
/// このため 0026（`PRAGMA foreign_keys` を OFF にしてから customers
/// テーブルを作り直すマイグレーション）を素の `Migrator::run()` にそのまま
/// 渡すと、
/// - `PRAGMA foreign_keys` はトランザクション内では no-op のため無効化されず
/// - マイグレーションSQL内の独自の `BEGIN` が sqlx が既に開いている
///   トランザクションとぶつかり `cannot start a transaction within a
///   transaction` で失敗する
///
/// ことを実測で確認した（`db::tests::migration_0026_survives_a_populated_database`
/// はこの関数を経由しないと同じ理由で失敗する）。
///
/// `PRAGMA defer_foreign_keys=ON`（トランザクション内でも変更できる）で
/// 回避できないかも検証したが、`DROP TABLE` で発生する遅延違反カウンタは
/// 「別テーブルへ `INSERT` してから `RENAME`」では解消されず
/// （`PRAGMA foreign_key_check` は 0 件を返すのに `COMMIT` が
/// `FOREIGN KEY constraint failed` になる。SQLite の内部カウンタは
/// テーブル名ではなく実際に発行された行操作単位で追跡されるため）、この
/// 再作成パターンには使えないと確認した。
///
/// # なぜ「no_tx だけ先に適用してから標準 Migrator に渡す」では駄目か
///
/// 最初の実装はそうしていたが、`no_tx` マイグレーション（0026）は
/// 順序上あとの版であり、まだ `customers` テーブルすら存在しない
/// （version 1〜25 未適用の）空DBに対して先出しで流すと
/// `no such table: customers` で失敗する。マイグレーションは版番号順に
/// 適用しないといけないため、この関数は `Migrator::run_direct` と同じ
/// ループ構造をそのまま踏襲し、`no_tx` かどうかで1マイグレーションだけ
/// 適用方法を変える。
///
/// ロック・チェックサム検証・dirty検出は `sqlx-core` の `Migrate` トレイト
/// 経由でそのまま利用しており、独自実装はしていない。
async fn run_sqlite_migrations(pool: &sqlx::SqlitePool) -> Result<(), BantoError> {
    use sqlx::migrate::Migrate;

    let migrator = sqlx::migrate!("./migrations-sqlite");
    let mut conn = pool
        .acquire()
        .await
        .map_err(|err| BantoError::Storage(err.to_string()))?;

    conn.lock()
        .await
        .map_err(|err| BantoError::Storage(err.to_string()))?;
    conn.ensure_migrations_table()
        .await
        .map_err(|err| BantoError::Storage(err.to_string()))?;

    if let Some(dirty_version) = conn
        .dirty_version()
        .await
        .map_err(|err| BantoError::Storage(err.to_string()))?
    {
        return Err(BantoError::Storage(format!(
            "migration {dirty_version} is partially applied; manual repair needed"
        )));
    }

    let applied: std::collections::HashMap<i64, Vec<u8>> = conn
        .list_applied_migrations()
        .await
        .map_err(|err| BantoError::Storage(err.to_string()))?
        .into_iter()
        .map(|m| (m.version, m.checksum.into_owned()))
        .collect();

    // sqlx 標準の `Migrator::run` と同じ防御: 適用済みとして記録されている
    // バージョンが埋め込みソースに存在しなければ（=マイグレーションファイル
    // の削除）、黙って続行せずエラーにする。
    let source_versions: std::collections::HashSet<i64> =
        migrator.migrations.iter().map(|m| m.version).collect();
    for version in applied.keys() {
        if !source_versions.contains(version) {
            return Err(BantoError::Storage(format!(
                "migration {version} was previously applied but no longer exists \
                 in the embedded migrations"
            )));
        }
    }

    let mut migrations: Vec<_> = migrator.migrations.iter().collect();
    migrations.sort_by_key(|m| m.version);

    for migration in migrations {
        if migration.migration_type.is_down_migration() {
            continue;
        }

        match applied.get(&migration.version) {
            Some(applied_checksum) => {
                if applied_checksum.as_slice() != migration.checksum.as_ref() {
                    return Err(BantoError::Storage(format!(
                        "migration {}'s checksum does not match the one already applied \
                         (an applied migration file must never be edited)",
                        migration.version
                    )));
                }
            }
            None if migration.no_tx => {
                // トランザクションの外側で素の SQL として流す（このマイグ
                // レーション自身が内部で BEGIN/COMMIT を管理する前提）。
                sqlx::raw_sql(migration.sql.as_ref())
                    .execute(&mut *conn)
                    .await
                    .map_err(|err| {
                        BantoError::Storage(format!(
                            "no-transaction migration {} failed: {err}",
                            migration.version
                        ))
                    })?;

                // `Migrate::apply` (sqlx-sqlite) が書く行と同一形式。
                sqlx::query(
                    "INSERT INTO _sqlx_migrations \
                        (version, description, success, checksum, execution_time) \
                     VALUES (?, ?, TRUE, ?, -1)",
                )
                .bind(migration.version)
                .bind(migration.description.as_ref())
                .bind(migration.checksum.as_ref())
                .execute(&mut *conn)
                .await
                .map_err(|err| BantoError::Storage(err.to_string()))?;
            }
            None => {
                // 通常どおり: sqlx 標準の `Migrate::apply` が1トランザクションに
                // 包んで適用する。
                conn.apply(migration)
                    .await
                    .map_err(|err| BantoError::Storage(err.to_string()))?;
            }
        }
    }

    conn.unlock()
        .await
        .map_err(|err| BantoError::Storage(err.to_string()))?;

    Ok(())
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

    /// 再発防止テスト: v0.1.0-alpha.2 は実機（データ入りDB）で起動即
    /// クラッシュした。banto-storage は SQLite 接続で
    /// `PRAGMA foreign_keys=ON` にしており
    /// (`crates/banto-storage/src/sqlite.rs`)、0026 の初版は「FK は
    /// 本リポジトリで有効化していない」という誤った前提のまま
    /// `DROP TABLE customers` でテーブル再作成しており、customers を
    /// 参照する行（projects 等）が1件でもあると
    /// `FOREIGN KEY constraint failed` で失敗していた。起動時マイグレー
    /// ションは `.expect` なので、この失敗＝アプリのクラッシュだった。
    ///
    /// 0025 相当のスキーマに架空データ（顧客1件・それを参照する案件1件）を
    /// 入れた状態から、**実際に起動時が呼ぶ `run_migrations()`** を通して
    /// 0026 を適用し、成功すること・データが温存されることを確認する。
    ///
    /// `run_migrations()` を直接呼ぶのが重要: 0026 のSQL内容そのものが
    /// 正しくても、`run_sqlite_migrations` が無ければ
    /// `sqlx::migrate!().run()` が `-- no-transaction` を無視してマイグレー
    /// ション全体をトランザクションに包んでしまい、
    /// `cannot start a transaction within a transaction` で失敗する
    /// （`run_sqlite_migrations` のコメント参照）。SQL文だけを
    /// 直接実行して検証するテストだとこの回帰を検知できない。
    #[tokio::test]
    async fn migration_0026_survives_a_populated_database() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("regression-0026.sqlite3");
        // banto-storage 経由で接続する: 本番と同じく PRAGMA foreign_keys=ON
        // が効いた接続を再現する。
        let db = Db::connect_sqlite(&db_path).await.expect("connect_sqlite");
        let pool = db.as_sqlite().expect("sqlite handle");

        // 0025 相当まで適用する: 埋め込み Migrator から version < 26 だけを
        // 抜き出した「部分 Migrator」を、sqlx 標準の適用機構
        // （各マイグレーションを個別トランザクションで包む本物の
        // `Migrate::apply`）でそのまま流す。
        let full_migrator = sqlx::migrate!("./migrations-sqlite");
        let up_to_25: Vec<_> = full_migrator
            .migrations
            .iter()
            .filter(|m| m.version < 26)
            .cloned()
            .collect();
        let partial_migrator = sqlx::migrate::Migrator {
            migrations: std::borrow::Cow::Owned(up_to_25),
            ..sqlx::migrate::Migrator::DEFAULT
        };
        partial_migrator
            .run(pool)
            .await
            .expect("migrating up to version 25 should succeed");

        // 架空データ: 顧客1件 + それを参照する案件1件
        // （CLAUDE.md 8章: テスト・サンプルデータは架空のものを使う）。
        sqlx::raw_sql(
            "INSERT INTO customers \
                (code, name, closing_day, payment_month_offset, payment_day, \
                 created_at, updated_at) \
             VALUES \
                ('C-TEST-001', '架空商事株式会社', 20, 1, 10, \
                 '2026-08-01', '2026-08-01');",
        )
        .execute(pool)
        .await
        .expect("insert fixture customer");

        sqlx::raw_sql(
            "INSERT INTO projects \
                (code, customer_id, name, status, created_at, updated_at) \
             SELECT 'P-TEST-001', id, '架空案件', 'IN_PROGRESS', \
                    '2026-08-01', '2026-08-01' \
             FROM customers WHERE code = 'C-TEST-001';",
        )
        .execute(pool)
        .await
        .expect("insert fixture project referencing the customer");

        // 本題: 顧客を参照する行がある状態で、実運用と同じ run_migrations()
        // が成功すること。旧版ではここで FOREIGN KEY constraint failed に
        // なっていた。
        run_migrations(&db)
            .await
            .expect("run_migrations should succeed even with referencing rows");

        // 顧客データが温存されている。
        let (code, name): (String, String) =
            sqlx::query_as("SELECT code, name FROM customers WHERE code = 'C-TEST-001'")
                .fetch_one(pool)
                .await
                .expect("fixture customer should survive the table rebuild");
        assert_eq!(code, "C-TEST-001");
        assert_eq!(name, "架空商事株式会社");

        // closing_day / payment_month_offset / payment_day が NULL で
        // INSERT できる（任意化の本旨）。
        sqlx::raw_sql(
            "INSERT INTO customers (code, name, created_at, updated_at) \
             VALUES ('C-TEST-002', '架空テスト商事', '2026-08-01', '2026-08-01');",
        )
        .execute(pool)
        .await
        .expect("closing_day etc. should now be nullable");

        // sync_outbox トリガーが再作成されている。
        let trigger_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'trigger' \
             AND name IN ('sync_outbox_customers_insert', 'sync_outbox_customers_update')",
        )
        .fetch_one(pool)
        .await
        .expect("query trigger count");
        assert_eq!(trigger_count, 2);

        // PRAGMA foreign_keys が ON に戻っている。
        let foreign_keys: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
            .fetch_one(pool)
            .await
            .expect("query foreign_keys pragma");
        assert_eq!(foreign_keys, 1);
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
