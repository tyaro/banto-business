//! Phase 3: 経費（docs/domain/schema.md §3.3）。conventions §2 に従い
//! `tauri` / `axum` / RBAC を知らない。
//!
//! - 金額は INTEGER（円。CLAUDE.md 1.1）。`amount` は**仕入側の実支払額**
//! - `tax_category` も**仕入側**の区分。顧客への再請求は一律 10%
//!   （Phase 1 決定 B-5）で、そちらは Phase 5 の InvoiceLine が持つ
//! - `billable`（顧客請求対象か）と `invoiced`（請求書に載せたか）は
//!   別フラグ（AGENTS.md 3.4）。1つにすると「請求し忘れ」を検出できない

use crate::dates::is_valid_date;
use banto_core::{BantoError, FieldError, ListParams, ListResult};
use banto_server::ServerEvent;
use banto_storage::{ColumnMap, Db, Dialect};
use serde::{Deserialize, Serialize};
use sqlx::{QueryBuilder, Sqlite};
use tokio::sync::broadcast;

/// 仕入側の税区分（docs/tax-calculation.md 3）。非課税と不課税はどちらも
/// 税額0だが、仕入税額控除の集計で区別が要るためコードを分けて保持する。
pub const TAX_CATEGORIES: [&str; 4] = ["STANDARD_10", "REDUCED_8", "EXEMPT", "OUT_OF_SCOPE"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Expense {
    pub id: i64,
    #[sqlx(rename = "project_id")]
    pub project_id: i64,
    #[sqlx(rename = "trip_id")]
    pub trip_id: Option<i64>,
    #[sqlx(rename = "spent_on")]
    pub spent_on: String,
    #[sqlx(rename = "expense_category_code")]
    pub expense_category_code: String,
    pub payee: Option<String>,
    /// 支払額（円）。
    pub amount: i64,
    #[sqlx(rename = "tax_category")]
    pub tax_category: String,
    pub description: Option<String>,
    pub billable: i64,
    pub invoiced: i64,
    #[sqlx(rename = "created_at")]
    pub created_at: String,
    #[sqlx(rename = "updated_at")]
    pub updated_at: String,
}

/// 作成・更新のペイロード。`taxCategory` を省略すると経費分類の既定区分
/// （`expense_categories.default_tax_category`）を使う。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExpenseInput {
    pub project_id: i64,
    pub trip_id: Option<i64>,
    pub spent_on: String,
    pub expense_category_code: String,
    pub payee: Option<String>,
    pub amount: i64,
    pub tax_category: Option<String>,
    pub description: Option<String>,
    #[serde(default)]
    pub billable: bool,
    #[serde(default)]
    pub invoiced: bool,
}

const MAX_AMOUNT: i64 = 9_999_999_999;
const MAX_TEXT_LEN: usize = 120;
const MAX_DESCRIPTION_LEN: usize = 500;

fn column_map() -> ColumnMap {
    ColumnMap::new()
        .column("id", "id")
        .column("projectId", "project_id")
        .column("tripId", "trip_id")
        .column("spentOn", "spent_on")
        .column("expenseCategoryCode", "expense_category_code")
        .column("payee", "payee")
        .column("amount", "amount")
        .column("taxCategory", "tax_category")
        .column("description", "description")
        .column("billable", "billable")
        .column("invoiced", "invoiced")
        .column("createdAt", "created_at")
        .column("updatedAt", "updated_at")
}

const RESOURCE: &str = "expenses";
const COLUMNS: &str = "id, project_id, trip_id, spent_on, expense_category_code, payee, amount, \
     tax_category, description, billable, invoiced, created_at, updated_at";

fn today_expr(dialect: Dialect) -> &'static str {
    match dialect {
        Dialect::Sqlite => "date('now')",
        Dialect::Postgres => "CURRENT_DATE::text",
    }
}

pub(crate) struct NormalizedExpense {
    pub project_id: i64,
    pub trip_id: Option<i64>,
    pub spent_on: String,
    pub expense_category_code: String,
    pub payee: Option<String>,
    pub amount: i64,
    pub tax_category: String,
    pub description: Option<String>,
    pub billable: i64,
    pub invoiced: i64,
}

/// 経費のサービス層（conventions §2）。
#[derive(Clone)]
pub struct ExpensesService {
    db: Db,
    events: Option<broadcast::Sender<ServerEvent>>,
}

impl ExpensesService {
    pub fn new(db: Db) -> Self {
        Self { db, events: None }
    }

    pub fn with_events(mut self, events: broadcast::Sender<ServerEvent>) -> Self {
        self.events = Some(events);
        self
    }

    fn notify_changed(&self) {
        if let Some(tx) = &self.events {
            let _ = tx.send(ServerEvent::ResourceChanged {
                resource: RESOURCE.to_string(),
            });
        }
    }

    async fn normalize(&self, input: &ExpenseInput) -> Result<NormalizedExpense, BantoError> {
        let mut errors: Vec<FieldError> = Vec::new();

        let spent_on = input.spent_on.trim().to_string();
        if spent_on.is_empty() {
            errors.push(FieldError {
                field: "spentOn".to_string(),
                message: "必須項目です".to_string(),
            });
        } else if !is_valid_date(&spent_on) {
            errors.push(FieldError {
                field: "spentOn".to_string(),
                message: "YYYY-MM-DD の形式で入力してください".to_string(),
            });
        }

        // 金額0を許すのは、無償対応の記録（0円で行だけ残す）が実務で
        // 起こりうるため。マイナスは返金の意味になり集計の解釈が割れるので
        // 弾く（返金は別途プラス金額の調整行で表現する）。
        if input.amount < 0 {
            errors.push(FieldError {
                field: "amount".to_string(),
                message: "0以上で入力してください".to_string(),
            });
        } else if input.amount > MAX_AMOUNT {
            errors.push(FieldError {
                field: "amount".to_string(),
                message: format!("{MAX_AMOUNT}以下で入力してください"),
            });
        }

        let payee = match input.payee.as_deref().map(str::trim) {
            None | Some("") => None,
            Some(text) => {
                if text.chars().count() > MAX_TEXT_LEN {
                    errors.push(FieldError {
                        field: "payee".to_string(),
                        message: format!("{MAX_TEXT_LEN}文字以内で入力してください"),
                    });
                }
                Some(text.to_string())
            }
        };
        let description = match input.description.as_deref().map(str::trim) {
            None | Some("") => None,
            Some(text) => {
                if text.chars().count() > MAX_DESCRIPTION_LEN {
                    errors.push(FieldError {
                        field: "description".to_string(),
                        message: format!("{MAX_DESCRIPTION_LEN}文字以内で入力してください"),
                    });
                }
                Some(text.to_string())
            }
        };

        let category_code = input.expense_category_code.trim().to_string();
        let masters = crate::masters::MastersService::new(self.db.clone());
        let categories = masters.list_expense_categories().await?;
        let category = categories.iter().find(|c| c.code == category_code);
        if category.is_none() {
            errors.push(FieldError {
                field: "expenseCategoryCode".to_string(),
                message: "経費分類を選択してください".to_string(),
            });
        }

        let tax_category = match input.tax_category.as_deref().map(str::trim) {
            None | Some("") => category
                .map(|c| c.default_tax_category.clone())
                .unwrap_or_else(|| "STANDARD_10".to_string()),
            Some(code) => {
                if !TAX_CATEGORIES.contains(&code) {
                    errors.push(FieldError {
                        field: "taxCategory".to_string(),
                        message: "税区分の値が不正です".to_string(),
                    });
                }
                code.to_string()
            }
        };

        let dialect = self.db.dialect();
        let sql = format!(
            "SELECT COUNT(*) FROM projects WHERE id = {}",
            dialect.placeholder(1)
        );
        let project_count: i64 = match &self.db {
            Db::Sqlite(pool) => {
                sqlx::query_scalar(&sql)
                    .bind(input.project_id)
                    .fetch_one(pool)
                    .await
            }
            #[cfg(feature = "postgres")]
            Db::Postgres(pool) => {
                sqlx::query_scalar(&sql)
                    .bind(input.project_id)
                    .fetch_one(pool)
                    .await
            }
        }
        .map_err(banto_storage::storage_error)?;
        if project_count == 0 {
            errors.push(FieldError {
                field: "projectId".to_string(),
                message: "案件を選択してください".to_string(),
            });
        }

        if !errors.is_empty() {
            return Err(BantoError::Validation {
                field_errors: errors,
            });
        }

        Ok(NormalizedExpense {
            project_id: input.project_id,
            trip_id: input.trip_id,
            spent_on,
            expense_category_code: category_code,
            payee,
            amount: input.amount,
            tax_category,
            description,
            billable: i64::from(input.billable),
            invoiced: i64::from(input.invoiced),
        })
    }

    pub async fn list(&self, params: ListParams) -> Result<ListResult<Expense>, BantoError> {
        let columns = column_map();
        let select_rows = format!("SELECT {COLUMNS} FROM expenses");
        const SELECT_COUNT: &str = "SELECT COUNT(*) FROM expenses";

        match &self.db {
            Db::Sqlite(pool) => {
                let mut rows_builder: QueryBuilder<'_, Sqlite> = QueryBuilder::new(&select_rows);
                banto_storage::list_query::sqlite::apply_list_params(
                    &mut rows_builder,
                    &columns,
                    &params,
                )?;
                let rows: Vec<Expense> = rows_builder
                    .build_query_as::<Expense>()
                    .fetch_all(pool)
                    .await
                    .map_err(banto_storage::storage_error)?;
                let mut count_builder: QueryBuilder<'_, Sqlite> = QueryBuilder::new(SELECT_COUNT);
                banto_storage::list_query::sqlite::append_where(
                    &mut count_builder,
                    &columns,
                    &params.filters,
                )?;
                let total_count: i64 = count_builder
                    .build_query_scalar()
                    .fetch_one(pool)
                    .await
                    .map_err(banto_storage::storage_error)?;
                Ok(ListResult {
                    rows,
                    total_count: total_count as u64,
                })
            }
            #[cfg(feature = "postgres")]
            Db::Postgres(pool) => {
                let mut rows_builder: QueryBuilder<'_, sqlx::Postgres> =
                    QueryBuilder::new(&select_rows);
                banto_storage::list_query::postgres::apply_list_params(
                    &mut rows_builder,
                    &columns,
                    &params,
                )?;
                let rows: Vec<Expense> = rows_builder
                    .build_query_as::<Expense>()
                    .fetch_all(pool)
                    .await
                    .map_err(banto_storage::storage_error)?;
                let mut count_builder: QueryBuilder<'_, sqlx::Postgres> =
                    QueryBuilder::new(SELECT_COUNT);
                banto_storage::list_query::postgres::append_where(
                    &mut count_builder,
                    &columns,
                    &params.filters,
                )?;
                let total_count: i64 = count_builder
                    .build_query_scalar()
                    .fetch_one(pool)
                    .await
                    .map_err(banto_storage::storage_error)?;
                Ok(ListResult {
                    rows,
                    total_count: total_count as u64,
                })
            }
        }
    }

    pub async fn get(&self, id: i64) -> Result<Expense, BantoError> {
        let dialect = self.db.dialect();
        let sql = format!(
            "SELECT {COLUMNS} FROM expenses WHERE id = {}",
            dialect.placeholder(1)
        );
        match &self.db {
            Db::Sqlite(pool) => {
                sqlx::query_as::<_, Expense>(&sql)
                    .bind(id)
                    .fetch_one(pool)
                    .await
            }
            #[cfg(feature = "postgres")]
            Db::Postgres(pool) => {
                sqlx::query_as::<_, Expense>(&sql)
                    .bind(id)
                    .fetch_one(pool)
                    .await
            }
        }
        .map_err(|err| banto_storage::not_found(err, RESOURCE, id.to_string()))
    }

    pub async fn create(&self, input: ExpenseInput) -> Result<Expense, BantoError> {
        let value = self.normalize(&input).await?;
        let dialect = self.db.dialect();
        let today = today_expr(dialect);
        let sql = format!(
            "INSERT INTO expenses (project_id, trip_id, spent_on, expense_category_code, payee, \
             amount, tax_category, description, billable, invoiced, created_at, updated_at) \
             VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {today}, {today}) \
             RETURNING {COLUMNS}",
            dialect.placeholder(1),
            dialect.placeholder(2),
            dialect.placeholder(3),
            dialect.placeholder(4),
            dialect.placeholder(5),
            dialect.placeholder(6),
            dialect.placeholder(7),
            dialect.placeholder(8),
            dialect.placeholder(9),
            dialect.placeholder(10),
        );
        let row = match &self.db {
            Db::Sqlite(pool) => {
                bind_input(sqlx::query_as::<_, Expense>(&sql), &value)
                    .fetch_one(pool)
                    .await
            }
            #[cfg(feature = "postgres")]
            Db::Postgres(pool) => {
                bind_input(sqlx::query_as::<_, Expense>(&sql), &value)
                    .fetch_one(pool)
                    .await
            }
        }
        .map_err(banto_storage::storage_error)?;
        self.notify_changed();
        Ok(row)
    }

    pub async fn update(&self, id: i64, input: ExpenseInput) -> Result<Expense, BantoError> {
        let value = self.normalize(&input).await?;
        let dialect = self.db.dialect();
        let sql = format!(
            "UPDATE expenses SET project_id = {}, trip_id = {}, spent_on = {}, \
             expense_category_code = {}, payee = {}, amount = {}, tax_category = {}, \
             description = {}, billable = {}, invoiced = {}, updated_at = {} \
             WHERE id = {} RETURNING {COLUMNS}",
            dialect.placeholder(1),
            dialect.placeholder(2),
            dialect.placeholder(3),
            dialect.placeholder(4),
            dialect.placeholder(5),
            dialect.placeholder(6),
            dialect.placeholder(7),
            dialect.placeholder(8),
            dialect.placeholder(9),
            dialect.placeholder(10),
            today_expr(dialect),
            dialect.placeholder(11),
        );
        let row = match &self.db {
            Db::Sqlite(pool) => {
                bind_input(sqlx::query_as::<_, Expense>(&sql), &value)
                    .bind(id)
                    .fetch_one(pool)
                    .await
            }
            #[cfg(feature = "postgres")]
            Db::Postgres(pool) => {
                bind_input(sqlx::query_as::<_, Expense>(&sql), &value)
                    .bind(id)
                    .fetch_one(pool)
                    .await
            }
        }
        .map_err(|err| banto_storage::not_found(err, RESOURCE, id.to_string()))?;
        self.notify_changed();
        Ok(row)
    }

    pub async fn delete(&self, id: i64) -> Result<(), BantoError> {
        let dialect = self.db.dialect();
        let sql = format!("DELETE FROM expenses WHERE id = {}", dialect.placeholder(1));
        let rows_affected = match &self.db {
            Db::Sqlite(pool) => sqlx::query(&sql)
                .bind(id)
                .execute(pool)
                .await
                .map(|r| r.rows_affected()),
            #[cfg(feature = "postgres")]
            Db::Postgres(pool) => sqlx::query(&sql)
                .bind(id)
                .execute(pool)
                .await
                .map(|r| r.rows_affected()),
        }
        .map_err(banto_storage::storage_error)?;
        if rows_affected == 0 {
            return Err(BantoError::NotFound {
                resource: RESOURCE.to_string(),
                id: id.to_string(),
            });
        }
        self.notify_changed();
        Ok(())
    }
}

fn bind_input<'q, DB>(
    query: sqlx::query::QueryAs<'q, DB, Expense, <DB as sqlx::Database>::Arguments<'q>>,
    value: &'q NormalizedExpense,
) -> sqlx::query::QueryAs<'q, DB, Expense, <DB as sqlx::Database>::Arguments<'q>>
where
    DB: sqlx::Database,
    &'q str: sqlx::Type<DB> + sqlx::Encode<'q, DB>,
    i64: sqlx::Type<DB> + sqlx::Encode<'q, DB>,
    Option<i64>: sqlx::Type<DB> + sqlx::Encode<'q, DB>,
    Option<&'q str>: sqlx::Type<DB> + sqlx::Encode<'q, DB>,
{
    query
        .bind(value.project_id)
        .bind(value.trip_id)
        .bind(value.spent_on.as_str())
        .bind(value.expense_category_code.as_str())
        .bind(value.payee.as_deref())
        .bind(value.amount)
        .bind(value.tax_category.as_str())
        .bind(value.description.as_deref())
        .bind(value.billable)
        .bind(value.invoiced)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::customers::{CustomerInput, CustomersService, DAY_END_OF_MONTH};
    use crate::db::migrate_memory;
    use crate::projects::{ProjectInput, ProjectsService};

    async fn fixture() -> (ExpensesService, i64) {
        let pool = migrate_memory().await.expect("migrate_memory");
        let customer = CustomersService::new(pool.clone())
            .create(CustomerInput {
                code: "C001".to_string(),
                name: "架空商事".to_string(),
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
            .expect("customer");
        let project = ProjectsService::new(pool.clone())
            .create(ProjectInput {
                code: String::new(),
                customer_id: customer.id,
                name: "架空案件".to_string(),
                status: "IN_PROGRESS".to_string(),
                started_on: None,
                due_on: None,
                estimate_amount: None,
                contract_amount: None,
                billing_hourly_rate: None,
                scope: None,
                note: None,
            })
            .await
            .expect("project");
        (ExpensesService::new(pool), project.id)
    }

    fn input(project_id: i64) -> ExpenseInput {
        ExpenseInput {
            project_id,
            trip_id: None,
            spent_on: "2026-08-20".to_string(),
            expense_category_code: "TRANSPORT".to_string(),
            payee: Some("架空鉄道".to_string()),
            amount: 12_800,
            tax_category: None,
            description: None,
            billable: true,
            invoiced: false,
        }
    }

    fn field_errors(err: &BantoError) -> Vec<String> {
        match err {
            BantoError::Validation { field_errors } => {
                field_errors.iter().map(|e| e.field.clone()).collect()
            }
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    /// 税区分を省略すると経費分類の既定（全て STANDARD_10）が入る。
    #[tokio::test]
    async fn tax_category_defaults_from_the_category_master() {
        let (svc, project_id) = fixture().await;
        let created = svc.create(input(project_id)).await.expect("create");
        assert_eq!(created.tax_category, "STANDARD_10");
        assert_eq!(created.amount, 12_800);
        assert_eq!(created.billable, 1);
        assert_eq!(created.invoiced, 0, "billable と invoiced は別フラグ");
    }

    /// 国際線のような不課税の仕入は行ごとに区分を変えられる
    /// （docs/tax-calculation.md 3 の「既定値であり、行ごとに変更可能」）。
    #[tokio::test]
    async fn tax_category_can_be_overridden_per_row() {
        let (svc, project_id) = fixture().await;
        let mut values = input(project_id);
        values.tax_category = Some("OUT_OF_SCOPE".to_string());
        let created = svc.create(values).await.expect("create");
        assert_eq!(created.tax_category, "OUT_OF_SCOPE");
    }

    #[tokio::test]
    async fn unknown_tax_category_is_rejected() {
        let (svc, project_id) = fixture().await;
        let mut values = input(project_id);
        values.tax_category = Some("STANDARD_8".to_string());
        let err = svc.create(values).await.expect_err("bad tax code");
        assert!(field_errors(&err).contains(&"taxCategory".to_string()));
    }

    /// 金額0は許す（無償対応の記録）。マイナスは返金の意味になり集計の
    /// 解釈が割れるため弾く。
    #[tokio::test]
    async fn zero_is_allowed_but_negative_is_not() {
        let (svc, project_id) = fixture().await;
        let mut zero = input(project_id);
        zero.amount = 0;
        svc.create(zero).await.expect("0円は許す");

        let mut negative = input(project_id);
        negative.amount = -1;
        let err = svc.create(negative).await.expect_err("negative");
        assert!(field_errors(&err).contains(&"amount".to_string()));
    }

    #[tokio::test]
    async fn invalid_date_and_unknown_refs_are_field_errors() {
        let (svc, project_id) = fixture().await;
        let mut bad_date = input(project_id);
        bad_date.spent_on = "2026-02-30".to_string(); // 存在しない日
        assert!(
            field_errors(&svc.create(bad_date).await.expect_err("bad date"))
                .contains(&"spentOn".to_string())
        );

        let mut bad_category = input(project_id);
        bad_category.expense_category_code = "NOPE".to_string();
        assert!(
            field_errors(&svc.create(bad_category).await.expect_err("bad category"))
                .contains(&"expenseCategoryCode".to_string())
        );

        let mut bad_project = input(project_id);
        bad_project.project_id = 9999;
        assert!(
            field_errors(&svc.create(bad_project).await.expect_err("bad project"))
                .contains(&"projectId".to_string())
        );
    }

    #[tokio::test]
    async fn update_and_delete_round_trip() {
        let (svc, project_id) = fixture().await;
        let created = svc.create(input(project_id)).await.expect("create");
        let mut values = input(project_id);
        values.amount = 15_000;
        values.invoiced = true;
        let updated = svc.update(created.id, values).await.expect("update");
        assert_eq!(updated.amount, 15_000);
        assert_eq!(updated.invoiced, 1);

        svc.delete(created.id).await.expect("delete");
        assert!(matches!(
            svc.get(created.id).await.expect_err("gone"),
            BantoError::NotFound { .. }
        ));
    }
}
