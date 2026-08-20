//! Phase 3: 工数（docs/domain/schema.md §3.2）。conventions §2 に従い
//! `tauri` / `axum` / RBAC を知らない。
//!
//! ここが Business の中心的な金額ロジックの入口になる。守るべき規約:
//!
//! - **時間は分単位の整数**（Phase 1 決定 C-1）。小数時間を持たない
//!   （CLAUDE.md 1.1 の浮動小数点禁止）
//! - **適用単価は行に焼き付ける**（CLAUDE.md 1.2）。`cost_rates` は新規
//!   入力時の既定値の供給源でしかなく、後から変えても過去は動かない
//! - **内部原価は行ごとに1回だけ丸める**。`floor(分 × 単価 ÷ 60)` を
//!   整数演算で計算し、列に保存する（Phase 1 決定 C-1）

use crate::dates::is_valid_date;
use banto_core::{BantoError, FieldError, ListParams, ListResult};
use banto_server::ServerEvent;
use banto_storage::{ColumnMap, Db, Dialect};
use serde::{Deserialize, Serialize};
use sqlx::{QueryBuilder, Sqlite};
use tokio::sync::broadcast;

/// 1行の工数。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct WorkLog {
    pub id: i64,
    #[sqlx(rename = "project_id")]
    pub project_id: i64,
    #[sqlx(rename = "trip_id")]
    pub trip_id: Option<i64>,
    #[sqlx(rename = "worked_on")]
    pub worked_on: String,
    #[sqlx(rename = "work_category_code")]
    pub work_category_code: String,
    /// 作業時間（分）。
    pub minutes: i64,
    /// 記録時点の時間単価（円/時）を焼き付けた値（CLAUDE.md 1.2）。
    #[sqlx(rename = "applied_rate")]
    pub applied_rate: i64,
    /// `floor(minutes × applied_rate ÷ 60)`（円）。行ごとに確定した原価。
    #[sqlx(rename = "internal_cost")]
    pub internal_cost: i64,
    pub description: Option<String>,
    pub invoiced: i64,
    #[sqlx(rename = "created_at")]
    pub created_at: String,
    #[sqlx(rename = "updated_at")]
    pub updated_at: String,
}

/// 作成・更新のペイロード。`appliedRate` を省略すると、作業分類の既定
/// レート（`cost_rates`）を使う（要件 F-W2）。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkLogInput {
    pub project_id: i64,
    pub trip_id: Option<i64>,
    pub worked_on: String,
    pub work_category_code: String,
    pub minutes: i64,
    pub applied_rate: Option<i64>,
    pub description: Option<String>,
    #[serde(default)]
    pub invoiced: bool,
}

/// 1日の上限（分）。24時間を超える工数は入力ミスとして弾く。
const MAX_MINUTES_PER_ENTRY: i64 = 24 * 60;
const MAX_RATE: i64 = 1_000_000;
const MAX_DESCRIPTION_LEN: usize = 500;

/// **内部原価の計算（Phase 1 決定 C-1）。**
///
/// `floor(分 × 時間単価 ÷ 60)` を i64 の整数演算で求める。浮動小数点を
/// 経由しないので、丸め方向は「1円未満の切捨て」で確定する。
///
/// 分単位で入力する以上、`90分 × 6,500円/時 = 9,750円` のように割り切れる
/// ケースばかりではない（例: `50分 × 6,500円/時 = 5,416.66…` → 5,416円）。
/// **行ごとに1回だけ**丸め、案件原価はその合計とする — 合計してから丸めると
/// 明細と合計が一致せず、採算画面で説明できなくなる。
pub fn internal_cost(minutes: i64, hourly_rate: i64) -> i64 {
    // i64 の乗算オーバーフローは入力上限（24時間 × 100万円/時）で起こり得ない。
    minutes * hourly_rate / 60
}

fn column_map() -> ColumnMap {
    ColumnMap::new()
        .column("id", "id")
        .column("projectId", "project_id")
        .column("tripId", "trip_id")
        .column("workedOn", "worked_on")
        .column("workCategoryCode", "work_category_code")
        .column("minutes", "minutes")
        .column("appliedRate", "applied_rate")
        .column("internalCost", "internal_cost")
        .column("description", "description")
        .column("invoiced", "invoiced")
        .column("createdAt", "created_at")
        .column("updatedAt", "updated_at")
}

const RESOURCE: &str = "work-logs";
const COLUMNS: &str = "id, project_id, trip_id, worked_on, work_category_code, minutes, \
     applied_rate, internal_cost, description, invoiced, created_at, updated_at";

fn today_expr(dialect: Dialect) -> &'static str {
    match dialect {
        Dialect::Sqlite => "date('now')",
        Dialect::Postgres => "CURRENT_DATE::text",
    }
}

/// 検証済みの入力。単価と原価はここで確定する。
pub(crate) struct NormalizedWorkLog {
    pub project_id: i64,
    pub trip_id: Option<i64>,
    pub worked_on: String,
    pub work_category_code: String,
    pub minutes: i64,
    pub applied_rate: i64,
    pub internal_cost: i64,
    pub description: Option<String>,
    pub invoiced: i64,
}

/// 工数のサービス層（conventions §2）。
#[derive(Clone)]
pub struct WorkLogsService {
    db: Db,
    events: Option<broadcast::Sender<ServerEvent>>,
}

impl WorkLogsService {
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

    /// 入力を検証し、単価・原価を確定する。`applied_rate` 未指定なら
    /// 作業分類の既定レートを引く（要件 F-W2）。既定も未設定なら
    /// 「単価を入力してください」で弾く — 単価0で黙って記録すると、
    /// 原価0の工数が採算に紛れ込む。
    async fn normalize(&self, input: &WorkLogInput) -> Result<NormalizedWorkLog, BantoError> {
        let mut errors: Vec<FieldError> = Vec::new();

        let worked_on = input.worked_on.trim().to_string();
        if worked_on.is_empty() {
            errors.push(FieldError {
                field: "workedOn".to_string(),
                message: "必須項目です".to_string(),
            });
        } else if !is_valid_date(&worked_on) {
            errors.push(FieldError {
                field: "workedOn".to_string(),
                message: "YYYY-MM-DD の形式で入力してください".to_string(),
            });
        }

        if input.minutes <= 0 {
            errors.push(FieldError {
                field: "minutes".to_string(),
                message: "1分以上で入力してください".to_string(),
            });
        } else if input.minutes > MAX_MINUTES_PER_ENTRY {
            errors.push(FieldError {
                field: "minutes".to_string(),
                message: format!("{MAX_MINUTES_PER_ENTRY}分（24時間）以内で入力してください"),
            });
        }

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

        let category_code = input.work_category_code.trim().to_string();
        let masters = crate::masters::MastersService::new(self.db.clone());
        let categories = masters.list_work_categories().await?;
        let category = categories.iter().find(|c| c.code == category_code);
        if category.is_none() {
            errors.push(FieldError {
                field: "workCategoryCode".to_string(),
                message: "作業分類を選択してください".to_string(),
            });
        }

        let applied_rate = match input.applied_rate {
            Some(rate) => Some(rate),
            None => category.and_then(|c| c.hourly_rate),
        };
        let applied_rate = match applied_rate {
            Some(rate) if rate < 0 => {
                errors.push(FieldError {
                    field: "appliedRate".to_string(),
                    message: "0以上で入力してください".to_string(),
                });
                0
            }
            Some(rate) if rate > MAX_RATE => {
                errors.push(FieldError {
                    field: "appliedRate".to_string(),
                    message: format!("{MAX_RATE}以下で入力してください"),
                });
                0
            }
            Some(rate) => rate,
            None => {
                errors.push(FieldError {
                    field: "appliedRate".to_string(),
                    message: "この作業分類の内部原価レートが未設定です。単価を入力するか、設定画面でレートを登録してください".to_string(),
                });
                0
            }
        };

        self.ensure_project_exists(input.project_id, &mut errors)
            .await?;

        if !errors.is_empty() {
            return Err(BantoError::Validation {
                field_errors: errors,
            });
        }

        Ok(NormalizedWorkLog {
            project_id: input.project_id,
            trip_id: input.trip_id,
            worked_on,
            work_category_code: category_code,
            minutes: input.minutes,
            applied_rate,
            internal_cost: internal_cost(input.minutes, applied_rate),
            description,
            invoiced: i64::from(input.invoiced),
        })
    }

    async fn ensure_project_exists(
        &self,
        project_id: i64,
        errors: &mut Vec<FieldError>,
    ) -> Result<(), BantoError> {
        let dialect = self.db.dialect();
        let sql = format!(
            "SELECT COUNT(*) FROM projects WHERE id = {}",
            dialect.placeholder(1)
        );
        let count: i64 = match &self.db {
            Db::Sqlite(pool) => {
                sqlx::query_scalar(&sql)
                    .bind(project_id)
                    .fetch_one(pool)
                    .await
            }
            #[cfg(feature = "postgres")]
            Db::Postgres(pool) => {
                sqlx::query_scalar(&sql)
                    .bind(project_id)
                    .fetch_one(pool)
                    .await
            }
        }
        .map_err(banto_storage::storage_error)?;
        if count == 0 {
            errors.push(FieldError {
                field: "projectId".to_string(),
                message: "案件を選択してください".to_string(),
            });
        }
        Ok(())
    }

    pub async fn list(&self, params: ListParams) -> Result<ListResult<WorkLog>, BantoError> {
        let columns = column_map();
        let select_rows = format!("SELECT {COLUMNS} FROM work_logs");
        const SELECT_COUNT: &str = "SELECT COUNT(*) FROM work_logs";

        match &self.db {
            Db::Sqlite(pool) => {
                let mut rows_builder: QueryBuilder<'_, Sqlite> = QueryBuilder::new(&select_rows);
                banto_storage::list_query::sqlite::apply_list_params(
                    &mut rows_builder,
                    &columns,
                    &params,
                )?;
                let rows: Vec<WorkLog> = rows_builder
                    .build_query_as::<WorkLog>()
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
                let rows: Vec<WorkLog> = rows_builder
                    .build_query_as::<WorkLog>()
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

    pub async fn get(&self, id: i64) -> Result<WorkLog, BantoError> {
        let dialect = self.db.dialect();
        let sql = format!(
            "SELECT {COLUMNS} FROM work_logs WHERE id = {}",
            dialect.placeholder(1)
        );
        match &self.db {
            Db::Sqlite(pool) => {
                sqlx::query_as::<_, WorkLog>(&sql)
                    .bind(id)
                    .fetch_one(pool)
                    .await
            }
            #[cfg(feature = "postgres")]
            Db::Postgres(pool) => {
                sqlx::query_as::<_, WorkLog>(&sql)
                    .bind(id)
                    .fetch_one(pool)
                    .await
            }
        }
        .map_err(|err| banto_storage::not_found(err, RESOURCE, id.to_string()))
    }

    pub async fn create(&self, input: WorkLogInput) -> Result<WorkLog, BantoError> {
        let value = self.normalize(&input).await?;
        let dialect = self.db.dialect();
        let today = today_expr(dialect);
        let sql = format!(
            "INSERT INTO work_logs (project_id, trip_id, worked_on, work_category_code, minutes, \
             applied_rate, internal_cost, description, invoiced, created_at, updated_at) \
             VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {today}, {today}) RETURNING {COLUMNS}",
            dialect.placeholder(1),
            dialect.placeholder(2),
            dialect.placeholder(3),
            dialect.placeholder(4),
            dialect.placeholder(5),
            dialect.placeholder(6),
            dialect.placeholder(7),
            dialect.placeholder(8),
            dialect.placeholder(9),
        );
        let row = match &self.db {
            Db::Sqlite(pool) => {
                bind_input(sqlx::query_as::<_, WorkLog>(&sql), &value)
                    .fetch_one(pool)
                    .await
            }
            #[cfg(feature = "postgres")]
            Db::Postgres(pool) => {
                bind_input(sqlx::query_as::<_, WorkLog>(&sql), &value)
                    .fetch_one(pool)
                    .await
            }
        }
        .map_err(banto_storage::storage_error)?;
        self.notify_changed();
        Ok(row)
    }

    /// 更新。**適用単価と内部原価は再計算する**（分や単価を直したら原価も
    /// 追随しないと行内で辻褄が合わなくなる）。ただし単価の既定値は更新時も
    /// 「その時点のレートマスタ」ではなく**入力値**を優先するため、
    /// `appliedRate` を省略した更新は元の行の単価を引き継ぐ。
    pub async fn update(&self, id: i64, input: WorkLogInput) -> Result<WorkLog, BantoError> {
        let mut input = input;
        if input.applied_rate.is_none() {
            // 既存行の単価を引き継ぐ（マスタが変わっても過去の行は動かない、
            // という CLAUDE.md 1.2 の考え方を更新でも守る）。
            input.applied_rate = Some(self.get(id).await?.applied_rate);
        }
        let value = self.normalize(&input).await?;
        let dialect = self.db.dialect();
        let sql = format!(
            "UPDATE work_logs SET project_id = {}, trip_id = {}, worked_on = {}, \
             work_category_code = {}, minutes = {}, applied_rate = {}, internal_cost = {}, \
             description = {}, invoiced = {}, updated_at = {} WHERE id = {} RETURNING {COLUMNS}",
            dialect.placeholder(1),
            dialect.placeholder(2),
            dialect.placeholder(3),
            dialect.placeholder(4),
            dialect.placeholder(5),
            dialect.placeholder(6),
            dialect.placeholder(7),
            dialect.placeholder(8),
            dialect.placeholder(9),
            today_expr(dialect),
            dialect.placeholder(10),
        );
        let row = match &self.db {
            Db::Sqlite(pool) => {
                bind_input(sqlx::query_as::<_, WorkLog>(&sql), &value)
                    .bind(id)
                    .fetch_one(pool)
                    .await
            }
            #[cfg(feature = "postgres")]
            Db::Postgres(pool) => {
                bind_input(sqlx::query_as::<_, WorkLog>(&sql), &value)
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
        let sql = format!(
            "DELETE FROM work_logs WHERE id = {}",
            dialect.placeholder(1)
        );
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
    query: sqlx::query::QueryAs<'q, DB, WorkLog, <DB as sqlx::Database>::Arguments<'q>>,
    value: &'q NormalizedWorkLog,
) -> sqlx::query::QueryAs<'q, DB, WorkLog, <DB as sqlx::Database>::Arguments<'q>>
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
        .bind(value.worked_on.as_str())
        .bind(value.work_category_code.as_str())
        .bind(value.minutes)
        .bind(value.applied_rate)
        .bind(value.internal_cost)
        .bind(value.description.as_deref())
        .bind(value.invoiced)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::customers::{CustomerInput, CustomersService, DAY_END_OF_MONTH};
    use crate::db::migrate_memory;
    use crate::masters::{CostRateInput, MastersService};
    use crate::projects::{ProjectInput, ProjectsService};

    /// 顧客1件・案件1件・DESIGN のレート 6,000円/時 を用意した状態。
    async fn fixture() -> (WorkLogsService, MastersService, i64) {
        let pool = migrate_memory().await.expect("migrate_memory");
        let customers = CustomersService::new(pool.clone());
        let customer = customers
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
        let projects = ProjectsService::new(pool.clone());
        let project = projects
            .create(ProjectInput {
                code: String::new(),
                customer_id: customer.id,
                name: "架空案件".to_string(),
                status: "IN_PROGRESS".to_string(),
                started_on: None,
                due_on: None,
                estimate_amount: None,
                contract_amount: None,
                scope: None,
                note: None,
            })
            .await
            .expect("project");
        let masters = MastersService::new(pool.clone());
        masters
            .set_cost_rate(CostRateInput {
                work_category_code: "DESIGN".to_string(),
                hourly_rate: 6000,
            })
            .await
            .expect("rate");
        (WorkLogsService::new(pool), masters, project.id)
    }

    fn input(project_id: i64, minutes: i64) -> WorkLogInput {
        WorkLogInput {
            project_id,
            trip_id: None,
            worked_on: "2026-08-20".to_string(),
            work_category_code: "DESIGN".to_string(),
            minutes,
            applied_rate: None,
            description: None,
            invoiced: false,
        }
    }

    fn field_errors(err: &BantoError) -> Vec<(String, String)> {
        match err {
            BantoError::Validation { field_errors } => field_errors
                .iter()
                .map(|e| (e.field.clone(), e.message.clone()))
                .collect(),
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    /// **CLAUDE.md 第6章の必須テスト（工数原価）。** 丸めは1円未満の切捨てで、
    /// 行ごとに1回だけ行う。
    #[test]
    fn internal_cost_floors_to_the_yen_per_row() {
        // 割り切れるケース
        assert_eq!(internal_cost(60, 6000), 6000);
        assert_eq!(internal_cost(90, 6000), 9000);
        assert_eq!(internal_cost(30, 6000), 3000);
        // 割り切れないケース: 50分 × 6,500円/時 = 5,416.66… → 5,416
        assert_eq!(internal_cost(50, 6500), 5416);
        // 1分 × 6,500円/時 = 108.33… → 108
        assert_eq!(internal_cost(1, 6500), 108);
        // 単価0（社内調整などで原価を載せない運用）は0円
        assert_eq!(internal_cost(120, 0), 0);
    }

    /// 「行ごとに丸めてから合計」と「合計してから丸め」で結果が変わることを
    /// 固定する。Phase 1 決定 C-1 は前者。税計算の T-08 と同じ趣旨のケース。
    #[test]
    fn per_row_rounding_differs_from_rounding_the_sum() {
        let rows = [(50_i64, 6500_i64), (50, 6500), (50, 6500)];
        let per_row: i64 = rows.iter().map(|(m, r)| internal_cost(*m, *r)).sum();
        let total_minutes: i64 = rows.iter().map(|(m, _)| *m).sum();
        let rounded_sum = internal_cost(total_minutes, 6500);
        assert_eq!(per_row, 5416 * 3); // 16,248
        assert_eq!(rounded_sum, 16_250);
        assert_ne!(
            per_row, rounded_sum,
            "行ごと丸めと合計後丸めは 2 円ずれる（この差が出ることが本ケースの目的）"
        );
    }

    #[tokio::test]
    async fn create_snapshots_the_master_rate_and_stores_the_cost() {
        let (svc, masters, project_id) = fixture().await;
        let created = svc.create(input(project_id, 90)).await.expect("create");
        assert_eq!(created.applied_rate, 6000);
        assert_eq!(created.internal_cost, 9000);

        // レートを変えても既存行は動かない（CLAUDE.md 1.2）。
        masters
            .set_cost_rate(CostRateInput {
                work_category_code: "DESIGN".to_string(),
                hourly_rate: 9000,
            })
            .await
            .expect("rate change");
        let fetched = svc.get(created.id).await.expect("get");
        assert_eq!(fetched.applied_rate, 6000, "過去の行は据え置き");
        assert_eq!(fetched.internal_cost, 9000);

        // 新規行は新しいレートを引く。
        let after = svc.create(input(project_id, 60)).await.expect("create");
        assert_eq!(after.applied_rate, 9000);
        assert_eq!(after.internal_cost, 9000);
    }

    #[tokio::test]
    async fn explicit_rate_overrides_the_master_default() {
        let (svc, _, project_id) = fixture().await;
        let mut values = input(project_id, 60);
        values.applied_rate = Some(4500);
        let created = svc.create(values).await.expect("create");
        assert_eq!(created.applied_rate, 4500);
        assert_eq!(created.internal_cost, 4500);
    }

    /// レート未設定の分類は、単価0で黙って記録せずエラーにする。
    #[tokio::test]
    async fn unset_rate_is_an_error_not_a_silent_zero() {
        let (svc, _, project_id) = fixture().await;
        let mut values = input(project_id, 60);
        values.work_category_code = "PLC".to_string(); // レート未設定
        let err = svc.create(values).await.expect_err("no rate");
        assert_eq!(field_errors(&err)[0].0, "appliedRate");
    }

    #[tokio::test]
    async fn minutes_bounds_are_enforced() {
        let (svc, _, project_id) = fixture().await;
        for minutes in [0_i64, -30, 24 * 60 + 1] {
            let err = svc
                .create(input(project_id, minutes))
                .await
                .expect_err("invalid minutes");
            assert_eq!(field_errors(&err)[0].0, "minutes");
        }
        svc.create(input(project_id, 24 * 60))
            .await
            .expect("24時間ちょうどは許す");
    }

    #[tokio::test]
    async fn unknown_project_or_category_is_a_field_error() {
        let (svc, _, project_id) = fixture().await;
        let mut bad_project = input(project_id, 60);
        bad_project.project_id = 9999;
        let err = svc.create(bad_project).await.expect_err("no project");
        assert!(field_errors(&err).iter().any(|(f, _)| f == "projectId"));

        let mut bad_category = input(project_id, 60);
        bad_category.work_category_code = "NOPE".to_string();
        let err = svc.create(bad_category).await.expect_err("no category");
        assert!(field_errors(&err)
            .iter()
            .any(|(f, _)| f == "workCategoryCode"));
    }

    #[tokio::test]
    async fn update_recomputes_cost_and_keeps_the_original_rate_when_omitted() {
        let (svc, masters, project_id) = fixture().await;
        let created = svc.create(input(project_id, 60)).await.expect("create");
        masters
            .set_cost_rate(CostRateInput {
                work_category_code: "DESIGN".to_string(),
                hourly_rate: 9000,
            })
            .await
            .expect("rate change");

        // appliedRate 省略の更新は「元の行の単価」を引き継ぐ（マスタの
        // 新レートを勝手に適用しない）。
        let updated = svc
            .update(created.id, input(project_id, 120))
            .await
            .expect("update");
        assert_eq!(updated.applied_rate, 6000);
        assert_eq!(updated.internal_cost, 12_000, "分の変更に原価が追随する");
    }

    #[tokio::test]
    async fn delete_removes_the_row() {
        let (svc, _, project_id) = fixture().await;
        let created = svc.create(input(project_id, 60)).await.expect("create");
        svc.delete(created.id).await.expect("delete");
        assert!(matches!(
            svc.get(created.id).await.expect_err("gone"),
            BantoError::NotFound { .. }
        ));
    }
}
