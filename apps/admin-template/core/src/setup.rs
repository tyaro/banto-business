//! Phase 7 準備: 初期セットアップの道しるべ（`docs/mobile-ui-plan.md` P2-1）。
//!
//! まっさらな DB で「発行者情報 → 内部原価レート → 顧客 → 案件 → 工数」の
//! 依存順を、dashboard が自分で判定して見せるための読み取り専用サービス。
//! conventions §2 に従い `tauri` / `axum` / RBAC を知らない。
//!
//! 各項目の「済」判定（2026-08-25 承認済み。変更する場合は要確認）:
//!
//! 1. 発行者: [`IssuerService`] 経由で登録番号が保存済み（空でない）。
//!    SQL を重複させず、既存の読み出しをそのまま再利用する。
//! 2. レート: **有効（`active`）な作業分類が1件以上あり、かつその全てに
//!    既定レートが設定されている。** レート未設定の分類で工数を保存すると
//!    エラーになる仕様（`work_logs.rs`）なので、「1件でも」ではなく「全部」
//!    を基準にする（`masters.rs` の一覧クエリと同じ JOIN 条件）。
//! 3. 顧客・4. 案件・5. 工数: 生きている行（`deleted_at IS NULL`）が1件以上。
//!
//! 5項目すべて `true` になったら `all_done` も `true` になり、画面側は
//! チェックリスト自体を消す（このモジュールは真偽だけを返し、表示判断は
//! フロントに任せる）。

use crate::issuer::IssuerService;
use banto_core::BantoError;
use banto_storage::Db;
use serde::{Deserialize, Serialize};

/// 初期セットアップの進捗。5項目すべてが揃うと `all_done` が `true`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupStatus {
    pub issuer_done: bool,
    pub rates_done: bool,
    pub customers_done: bool,
    pub projects_done: bool,
    pub work_logs_done: bool,
    pub all_done: bool,
}

/// テーブルの生きている行（`deleted_at IS NULL`）が1件でもあるか。
///
/// `CAST(COUNT(*) AS BIGINT)` は PostgreSQL 方言の罠対策（`COUNT` が
/// `numeric` ではなく `bigint` を返すことを明示する。`calendar.rs` /
/// `sync.rs::has_ranged_rows` と同じ書き方）。
async fn has_alive_rows(db: &Db, table: &str) -> Result<bool, BantoError> {
    let sql = format!("SELECT CAST(COUNT(*) AS BIGINT) FROM {table} WHERE deleted_at IS NULL");
    let count: i64 = match db {
        Db::Sqlite(pool) => sqlx::query_scalar(&sql).fetch_one(pool).await,
        #[cfg(feature = "postgres")]
        Db::Postgres(pool) => sqlx::query_scalar(&sql).fetch_one(pool).await,
    }
    .map_err(banto_storage::storage_error)?;
    Ok(count > 0)
}

/// 有効な作業分類の件数と、そのうちレート未設定の件数。
///
/// `masters.rs::list_work_categories` と同じ JOIN 条件（`cost_rates` の
/// 生きている行だけを見る）を使うが、一覧を丸ごと読んで Rust 側で集計する
/// のではなく、SQL 側で件数だけを返す（判定にしか使わないため）。
async fn active_work_category_counts(db: &Db) -> Result<(i64, i64), BantoError> {
    const ACTIVE_SQL: &str =
        "SELECT CAST(COUNT(*) AS BIGINT) FROM work_categories WHERE deleted_at IS NULL AND active = 1";
    const MISSING_SQL: &str = "SELECT CAST(COUNT(*) AS BIGINT) FROM work_categories c \
         LEFT JOIN cost_rates r \
           ON r.work_category_code = c.code AND r.deleted_at IS NULL \
         WHERE c.deleted_at IS NULL AND c.active = 1 AND r.hourly_rate IS NULL";

    let active: i64 = match db {
        Db::Sqlite(pool) => sqlx::query_scalar(ACTIVE_SQL).fetch_one(pool).await,
        #[cfg(feature = "postgres")]
        Db::Postgres(pool) => sqlx::query_scalar(ACTIVE_SQL).fetch_one(pool).await,
    }
    .map_err(banto_storage::storage_error)?;
    let missing: i64 = match db {
        Db::Sqlite(pool) => sqlx::query_scalar(MISSING_SQL).fetch_one(pool).await,
        #[cfg(feature = "postgres")]
        Db::Postgres(pool) => sqlx::query_scalar(MISSING_SQL).fetch_one(pool).await,
    }
    .map_err(banto_storage::storage_error)?;
    Ok((active, missing))
}

/// 初期セットアップの進捗を判定する。
///
/// 発行者情報は [`IssuerService`] の既存の読み出しをそのまま使う（SQL を
/// 重複させない）。それ以外は `db` に直接問い合わせる — 4項目とも Business
/// ドメインの生テーブル数件を数えるだけで、専用サービスを新設するほどの
/// ロジックを持たないため。
pub async fn setup_status(db: &Db, issuer: &IssuerService) -> Result<SetupStatus, BantoError> {
    let issuer_done = issuer.get().await?.registration_number.is_some();

    let (active_categories, missing_rates) = active_work_category_counts(db).await?;
    let rates_done = active_categories > 0 && missing_rates == 0;

    let customers_done = has_alive_rows(db, "customers").await?;
    let projects_done = has_alive_rows(db, "projects").await?;
    let work_logs_done = has_alive_rows(db, "work_logs").await?;

    let all_done = issuer_done && rates_done && customers_done && projects_done && work_logs_done;

    Ok(SetupStatus {
        issuer_done,
        rates_done,
        customers_done,
        projects_done,
        work_logs_done,
        all_done,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::customers::{CustomersService, DAY_END_OF_MONTH};
    use crate::db::migrate_memory;
    use crate::issuer::IssuerInput;
    use crate::masters::{CostRateInput, MastersService};
    use crate::projects::{ProjectInput, ProjectsService};
    use crate::settings::SettingsService;
    use crate::work_logs::{WorkLogInput, WorkLogsService};
    use banto_core::ListParams;

    /// 架空の発行者入力（`CLAUDE.md` 第8章: 実値はリポジトリに書かない）。
    fn issuer_input() -> IssuerInput {
        IssuerInput {
            name: Some("架空設計事務所".to_string()),
            registration_number: Some("T1234567890123".to_string()),
            address: None,
            bank_account: None,
            rounding_mode: "FLOOR".to_string(),
        }
    }

    fn customer_input(code: &str) -> crate::customers::CustomerInput {
        crate::customers::CustomerInput {
            code: code.to_string(),
            name: "架空商事".to_string(),
            contact_person: None,
            address: None,
            phone: None,
            email: None,
            billing_name: None,
            closing_day: Some(DAY_END_OF_MONTH),
            payment_month_offset: Some(1),
            payment_day: Some(DAY_END_OF_MONTH),
            note: None,
        }
    }

    fn project_input(customer_id: i64) -> ProjectInput {
        ProjectInput {
            code: String::new(),
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

    #[tokio::test]
    async fn nothing_done_on_a_fresh_database() {
        let db = migrate_memory().await.expect("migrate_memory");
        let issuer = IssuerService::new(SettingsService::new(db.clone()));
        let status = setup_status(&db, &issuer).await.expect("setup_status");
        assert_eq!(
            status,
            SetupStatus {
                issuer_done: false,
                rates_done: false,
                customers_done: false,
                projects_done: false,
                work_logs_done: false,
                all_done: false,
            }
        );
    }

    /// 各項目を1つずつ満たすと、該当フラグだけが true になる。
    #[tokio::test]
    async fn each_item_flips_its_own_flag_independently() {
        let db = migrate_memory().await.expect("migrate_memory");
        let issuer = IssuerService::new(SettingsService::new(db.clone()));

        issuer.set(issuer_input()).await.expect("issuer set");
        let status = setup_status(&db, &issuer).await.expect("setup_status");
        assert!(status.issuer_done);
        assert!(!status.rates_done);
        assert!(!status.customers_done);
        assert!(!status.projects_done);
        assert!(!status.work_logs_done);
        assert!(!status.all_done);

        let customers = CustomersService::new(db.clone());
        let customer = customers
            .create(customer_input("C001"))
            .await
            .expect("customer");
        let status = setup_status(&db, &issuer).await.expect("setup_status");
        assert!(status.customers_done);
        assert!(!status.projects_done);

        let projects = ProjectsService::new(db.clone());
        let project = projects
            .create(project_input(customer.id))
            .await
            .expect("project");
        let status = setup_status(&db, &issuer).await.expect("setup_status");
        assert!(status.projects_done);
        assert!(!status.work_logs_done);
        assert!(!status.rates_done);

        // レートを設定しないまま工数を記録しようとするとエラーになる仕様
        // （`work_logs.rs`）なので、work_logs_done を満たすにはまずレートを
        // 埋める。
        let masters = MastersService::new(db.clone());
        masters
            .set_cost_rate(CostRateInput {
                work_category_code: "DESIGN".to_string(),
                hourly_rate: 6_000,
            })
            .await
            .expect("cost rate");

        let work_logs = WorkLogsService::new(db.clone());
        work_logs
            .create(WorkLogInput {
                project_id: project.id,
                trip_id: None,
                worked_on: "2026-08-24".to_string(),
                work_category_code: "DESIGN".to_string(),
                minutes: 60,
                applied_rate: None,
                description: None,
                invoiced: false,
            })
            .await
            .expect("work log");

        let status = setup_status(&db, &issuer).await.expect("setup_status");
        assert!(status.work_logs_done);
        // rates_done はまだ false — DESIGN 以外の active 分類が未設定のまま
        // 残っているはず（シードは10分類、レート設定は1件のみ）。
        assert!(!status.rates_done);
        assert!(!status.all_done);
    }

    /// レート判定は「active 分類のうち1つでも未設定なら false」
    /// 「全 active に設定で true」「active 分類ゼロなら false」の3ケース。
    #[tokio::test]
    async fn rates_done_requires_every_active_category_to_have_a_rate() {
        let db = migrate_memory().await.expect("migrate_memory");
        let issuer = IssuerService::new(SettingsService::new(db.clone()));
        let masters = MastersService::new(db.clone());

        // シード直後: active 分類はあるが、どれもレート未設定。
        let status = setup_status(&db, &issuer).await.expect("setup_status");
        assert!(!status.rates_done);

        let categories = masters.list_work_categories().await.expect("list");
        let active_codes: Vec<String> = categories
            .iter()
            .filter(|c| c.active == 1)
            .map(|c| c.code.clone())
            .collect();
        assert!(
            !active_codes.is_empty(),
            "シードに active 分類が無い前提が崩れた"
        );

        // 1つを残して全部埋める → まだ false。
        for code in active_codes.iter().skip(1) {
            masters
                .set_cost_rate(CostRateInput {
                    work_category_code: code.clone(),
                    hourly_rate: 5_000,
                })
                .await
                .expect("cost rate");
        }
        let status = setup_status(&db, &issuer).await.expect("setup_status");
        assert!(!status.rates_done, "1分類だけ未設定なのに true になった");

        // 残り1つも埋める → true。
        masters
            .set_cost_rate(CostRateInput {
                work_category_code: active_codes[0].clone(),
                hourly_rate: 5_000,
            })
            .await
            .expect("cost rate");
        let status = setup_status(&db, &issuer).await.expect("setup_status");
        assert!(status.rates_done);

        // active 分類がゼロなら（全部無効化）false に戻る。
        for code in &active_codes {
            sqlx::query("UPDATE work_categories SET active = 0 WHERE code = ?")
                .bind(code)
                .execute(db.as_sqlite().expect("sqlite pool"))
                .await
                .expect("deactivate");
        }
        let status = setup_status(&db, &issuer).await.expect("setup_status");
        assert!(
            !status.rates_done,
            "active 分類がゼロなのに rates_done が true になった"
        );
    }

    /// 削除済み行はカウントしない（顧客・案件・工数の3資源）。
    #[tokio::test]
    async fn soft_deleted_rows_do_not_count_toward_done() {
        let db = migrate_memory().await.expect("migrate_memory");
        let issuer = IssuerService::new(SettingsService::new(db.clone()));

        let customers = CustomersService::new(db.clone());
        let customer = customers
            .create(customer_input("C001"))
            .await
            .expect("customer");
        let projects = ProjectsService::new(db.clone());
        let project = projects
            .create(project_input(customer.id))
            .await
            .expect("project");
        let masters = MastersService::new(db.clone());
        masters
            .set_cost_rate(CostRateInput {
                work_category_code: "DESIGN".to_string(),
                hourly_rate: 6_000,
            })
            .await
            .expect("cost rate");
        let work_logs = WorkLogsService::new(db.clone());
        let log = work_logs
            .create(WorkLogInput {
                project_id: project.id,
                trip_id: None,
                worked_on: "2026-08-24".to_string(),
                work_category_code: "DESIGN".to_string(),
                minutes: 60,
                applied_rate: None,
                description: None,
                invoiced: false,
            })
            .await
            .expect("work log");

        // 工数を削除すると、たとえ他の判定を満たしていても false に戻る。
        work_logs.delete(log.id).await.expect("delete work log");
        let status = setup_status(&db, &issuer).await.expect("setup_status");
        assert!(!status.work_logs_done);

        // `list` を通した生存確認とも一致する（同じ deleted_at 条件）。
        let logs_after_delete = work_logs
            .list(ListParams::default())
            .await
            .expect("list work logs");
        assert!(logs_after_delete.rows.is_empty());
    }
}
