//! Phase 3: 出張と一括生成ウィザード（docs/domain/schema.md §3.1、
//! docs/plan.md 9.1、要件 F-T1〜F-T3）。conventions §2 に従い `tauri` /
//! `axum` / RBAC を知らない。
//!
//! ## なぜ一括生成が要るか
//!
//! 出張1回につき「移動工数 × 2、現地工数 × 日数、交通費、宿泊費」を個別
//! 画面で入力する運用は続かない（plan.md 9.1）。ここは入力負荷の要件
//! （requirements.md U-1: 出張1回の入力が1画面で完結する）を満たす中核。
//!
//! ## 生成物は通常のレコード
//!
//! 生成された WorkLog / Expense は普通の行で、`trip_id` で出張に紐づく
//! だけ。生成後に個別編集・削除できる（要件 F-T2）。Trip を削除しても
//! 生成物は残り `trip_id` が NULL になる（要件 F-T3 / Phase 1 決定 C-6）—
//! 工数実績が消えると案件採算が壊れるため。
//!
//! ## 生成は1トランザクション
//!
//! 途中で失敗したら Trip ごと巻き戻す。「出張は登録されたが工数だけ入って
//! いない」中途半端な状態を残さない。

use banto_core::{BantoError, FieldError, ListParams, ListResult};
use banto_server::ServerEvent;
use banto_storage::{ColumnMap, Db, Dialect};
use serde::{Deserialize, Serialize};
use sqlx::{QueryBuilder, Sqlite};
use tokio::sync::broadcast;

use crate::dates::{add_days, inclusive_day_span, is_valid_date};
use crate::work_logs::internal_cost;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Trip {
    pub id: i64,
    #[sqlx(rename = "project_id")]
    pub project_id: i64,
    pub destination: String,
    #[sqlx(rename = "start_on")]
    pub start_on: String,
    #[sqlx(rename = "end_on")]
    pub end_on: String,
    /// 現地作業日数。
    #[sqlx(rename = "onsite_days")]
    pub onsite_days: i64,
    /// 宿泊数。
    pub nights: i64,
    pub note: Option<String>,
    #[sqlx(rename = "created_at")]
    pub created_at: String,
    #[sqlx(rename = "updated_at")]
    pub updated_at: String,
}

/// 出張の登録内容。生成の指示（`generate`）を伴うと WorkLog / Expense を
/// 一括生成する（要件 F-T1）。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TripInput {
    pub project_id: i64,
    pub destination: String,
    pub start_on: String,
    pub end_on: String,
    pub onsite_days: i64,
    pub nights: i64,
    pub note: Option<String>,
    /// 省略すると Trip だけを登録する（生成なし）。
    pub generate: Option<TripGenerationInput>,
}

/// 一括生成の入力（Phase 1 決定 C-7）。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TripGenerationInput {
    /// 片道の移動時間（分）。往復2件の WorkLog を作る。
    pub travel_minutes_one_way: i64,
    /// 現地作業の1日あたり時間（分）。`onsite_days` 日分の WorkLog を作る。
    pub onsite_minutes_per_day: i64,
    /// 交通費（往復合計・円）。0 なら経費を作らない。
    pub transport_amount: i64,
    /// 1泊あたりの宿泊費（円）。0 なら経費を作らない。
    pub lodging_amount_per_night: i64,
    /// 生成する経費を顧客請求対象にするか（既定 true）。
    #[serde(default = "default_true")]
    pub billable: bool,
}

fn default_true() -> bool {
    true
}

/// 一括生成の結果（要件 F-T1 の「生成結果を一覧で確認」に渡す内訳）。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TripGenerationResult {
    pub trip: Trip,
    /// 生成した移動工数の件数（往復2件。移動時間が0なら0件）。
    pub travel_work_logs: usize,
    /// 生成した現地作業工数の件数（現地作業日数分）。
    pub onsite_work_logs: usize,
    /// 生成した経費の件数（交通費・宿泊費で最大2件）。
    pub expenses: usize,
    /// 生成した工数の内部原価合計（円）。行ごとに丸めた値の合計。
    pub total_internal_cost: i64,
    /// 生成した経費の金額合計（円）。
    pub total_expense_amount: i64,
}

const MAX_ONSITE_DAYS: i64 = 60;
const MAX_NIGHTS: i64 = 60;
const MAX_MINUTES: i64 = 24 * 60;
const MAX_AMOUNT: i64 = 9_999_999_999;
const MAX_TEXT_LEN: usize = 120;
const MAX_NOTE_LEN: usize = 500;

const TRAVEL_CATEGORY: &str = "TRAVEL";
const ONSITE_CATEGORY: &str = "ONSITE";
const TRANSPORT_CATEGORY: &str = "TRANSPORT";
const LODGING_CATEGORY: &str = "LODGING";

fn column_map() -> ColumnMap {
    ColumnMap::new()
        .column("id", "id")
        .column("projectId", "project_id")
        .column("destination", "destination")
        .column("startOn", "start_on")
        .column("endOn", "end_on")
        .column("onsiteDays", "onsite_days")
        .column("nights", "nights")
        .column("note", "note")
        .column("createdAt", "created_at")
        .column("updatedAt", "updated_at")
}

const RESOURCE: &str = "trips";
const COLUMNS: &str = "id, project_id, destination, start_on, end_on, onsite_days, nights, note, \
     created_at, updated_at";

fn today_expr(dialect: Dialect) -> &'static str {
    match dialect {
        Dialect::Sqlite => "date('now')",
        Dialect::Postgres => "CURRENT_DATE::text",
    }
}

/// 出張のサービス層（conventions §2）。
#[derive(Clone)]
pub struct TripsService {
    db: Db,
    events: Option<broadcast::Sender<ServerEvent>>,
}

impl TripsService {
    pub fn new(db: Db) -> Self {
        Self { db, events: None }
    }

    pub fn with_events(mut self, events: broadcast::Sender<ServerEvent>) -> Self {
        self.events = Some(events);
        self
    }

    fn notify_changed(&self) {
        if let Some(tx) = &self.events {
            // 生成物が増えるため、工数・経費の一覧も再取得させる。
            for resource in [RESOURCE, "work-logs", "expenses"] {
                let _ = tx.send(ServerEvent::ResourceChanged {
                    resource: resource.to_string(),
                });
            }
        }
    }

    async fn validate(&self, input: &TripInput) -> Result<(), BantoError> {
        let mut errors: Vec<FieldError> = Vec::new();

        let destination = input.destination.trim();
        if destination.is_empty() {
            errors.push(FieldError {
                field: "destination".to_string(),
                message: "必須項目です".to_string(),
            });
        } else if destination.chars().count() > MAX_TEXT_LEN {
            errors.push(FieldError {
                field: "destination".to_string(),
                message: format!("{MAX_TEXT_LEN}文字以内で入力してください"),
            });
        }

        for (field, value) in [("startOn", &input.start_on), ("endOn", &input.end_on)] {
            if !is_valid_date(value.trim()) {
                errors.push(FieldError {
                    field: field.to_string(),
                    message: "YYYY-MM-DD の形式で入力してください".to_string(),
                });
            }
        }
        // 期間の前後関係と、期間に収まらない日数・泊数を弾く。
        let span = inclusive_day_span(input.start_on.trim(), input.end_on.trim());
        match span {
            None if is_valid_date(input.start_on.trim()) && is_valid_date(input.end_on.trim()) => {
                errors.push(FieldError {
                    field: "endOn".to_string(),
                    message: "終了日は開始日以降にしてください".to_string(),
                });
            }
            Some(days) => {
                // 現地作業日数が出張期間を超えるのは入力ミス（生成した工数の
                // 日付が出張期間の外へはみ出す）。
                if input.onsite_days > days {
                    errors.push(FieldError {
                        field: "onsiteDays".to_string(),
                        message: format!("出張期間は{days}日です。それ以内で入力してください"),
                    });
                }
                // 3日間（2泊3日）なら泊数は最大2。
                if input.nights > days - 1 {
                    errors.push(FieldError {
                        field: "nights".to_string(),
                        message: format!("出張期間は{days}日です。宿泊は{}泊以内です", days - 1),
                    });
                }
            }
            None => {}
        }

        if !(0..=MAX_ONSITE_DAYS).contains(&input.onsite_days) {
            errors.push(FieldError {
                field: "onsiteDays".to_string(),
                message: format!("0〜{MAX_ONSITE_DAYS}で入力してください"),
            });
        }
        if !(0..=MAX_NIGHTS).contains(&input.nights) {
            errors.push(FieldError {
                field: "nights".to_string(),
                message: format!("0〜{MAX_NIGHTS}で入力してください"),
            });
        }
        if let Some(note) = input.note.as_deref() {
            if note.trim().chars().count() > MAX_NOTE_LEN {
                errors.push(FieldError {
                    field: "note".to_string(),
                    message: format!("{MAX_NOTE_LEN}文字以内で入力してください"),
                });
            }
        }

        if let Some(gen) = &input.generate {
            if !(0..=MAX_MINUTES).contains(&gen.travel_minutes_one_way) {
                errors.push(FieldError {
                    field: "generate.travelMinutesOneWay".to_string(),
                    message: format!("0〜{MAX_MINUTES}分で入力してください"),
                });
            }
            if !(0..=MAX_MINUTES).contains(&gen.onsite_minutes_per_day) {
                errors.push(FieldError {
                    field: "generate.onsiteMinutesPerDay".to_string(),
                    message: format!("0〜{MAX_MINUTES}分で入力してください"),
                });
            }
            for (field, amount) in [
                ("generate.transportAmount", gen.transport_amount),
                (
                    "generate.lodgingAmountPerNight",
                    gen.lodging_amount_per_night,
                ),
            ] {
                if !(0..=MAX_AMOUNT).contains(&amount) {
                    errors.push(FieldError {
                        field: field.to_string(),
                        message: format!("0〜{MAX_AMOUNT}で入力してください"),
                    });
                }
            }
        }

        let dialect = self.db.dialect();
        let sql = format!(
            "SELECT COUNT(*) FROM projects WHERE id = {} AND deleted_at IS NULL",
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

        if errors.is_empty() {
            Ok(())
        } else {
            Err(BantoError::Validation {
                field_errors: errors,
            })
        }
    }

    /// 生成に使う単価。移動・現地の作業分類のレートが未設定なら、単価0で
    /// 黙って生成せずエラーにする（`work_logs.rs` の方針と揃える）。
    async fn generation_rates(&self, gen: &TripGenerationInput) -> Result<(i64, i64), BantoError> {
        let masters = crate::masters::MastersService::new(self.db.clone());
        let categories = masters.list_work_categories().await?;
        let mut errors: Vec<FieldError> = Vec::new();
        let mut rate_for = |code: &str, needed: bool, field: &str| -> i64 {
            let rate = categories
                .iter()
                .find(|c| c.code == code)
                .and_then(|c| c.hourly_rate);
            match rate {
                Some(rate) => rate,
                None => {
                    if needed {
                        errors.push(FieldError {
                            field: field.to_string(),
                            message: format!(
                                "作業分類「{code}」の内部原価レートが未設定です。設定画面で登録してください"
                            ),
                        });
                    }
                    0
                }
            }
        };
        let travel_rate = rate_for(
            TRAVEL_CATEGORY,
            gen.travel_minutes_one_way > 0,
            "generate.travelMinutesOneWay",
        );
        let onsite_rate = rate_for(
            ONSITE_CATEGORY,
            gen.onsite_minutes_per_day > 0,
            "generate.onsiteMinutesPerDay",
        );
        if errors.is_empty() {
            Ok((travel_rate, onsite_rate))
        } else {
            Err(BantoError::Validation {
                field_errors: errors,
            })
        }
    }

    pub async fn list(&self, params: ListParams) -> Result<ListResult<Trip>, BantoError> {
        let columns = column_map();
        let select_rows = format!("SELECT {COLUMNS} FROM {}", crate::sync::live("trips"));
        let select_count = format!("SELECT COUNT(*) FROM {}", crate::sync::live("trips"));

        match &self.db {
            Db::Sqlite(pool) => {
                let mut rows_builder: QueryBuilder<'_, Sqlite> = QueryBuilder::new(&select_rows);
                banto_storage::list_query::sqlite::apply_list_params(
                    &mut rows_builder,
                    &columns,
                    &params,
                )?;
                let rows: Vec<Trip> = rows_builder
                    .build_query_as::<Trip>()
                    .fetch_all(pool)
                    .await
                    .map_err(banto_storage::storage_error)?;
                let mut count_builder: QueryBuilder<'_, Sqlite> = QueryBuilder::new(&select_count);
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
                let rows: Vec<Trip> = rows_builder
                    .build_query_as::<Trip>()
                    .fetch_all(pool)
                    .await
                    .map_err(banto_storage::storage_error)?;
                let mut count_builder: QueryBuilder<'_, sqlx::Postgres> =
                    QueryBuilder::new(&select_count);
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

    pub async fn get(&self, id: i64) -> Result<Trip, BantoError> {
        let dialect = self.db.dialect();
        let sql = format!(
            "SELECT {COLUMNS} FROM trips WHERE id = {} AND deleted_at IS NULL",
            dialect.placeholder(1)
        );
        match &self.db {
            Db::Sqlite(pool) => {
                sqlx::query_as::<_, Trip>(&sql)
                    .bind(id)
                    .fetch_one(pool)
                    .await
            }
            #[cfg(feature = "postgres")]
            Db::Postgres(pool) => {
                sqlx::query_as::<_, Trip>(&sql)
                    .bind(id)
                    .fetch_one(pool)
                    .await
            }
        }
        .map_err(|err| banto_storage::not_found(err, RESOURCE, id.to_string()))
    }

    /// 出張を登録し、`generate` があれば工数・経費を一括生成する
    /// （要件 F-T1）。**全体を1トランザクションで実行**し、途中で失敗したら
    /// Trip ごと巻き戻す。
    pub async fn create(&self, input: TripInput) -> Result<TripGenerationResult, BantoError> {
        self.validate(&input).await?;
        let rates = match &input.generate {
            Some(gen) => Some(self.generation_rates(gen).await?),
            None => None,
        };

        let result = match &self.db {
            Db::Sqlite(pool) => create_sqlite(pool, &input, rates).await,
            #[cfg(feature = "postgres")]
            Db::Postgres(pool) => create_postgres(pool, &input, rates).await,
        }?;
        self.notify_changed();
        Ok(result)
    }

    pub async fn update(&self, id: i64, input: TripInput) -> Result<Trip, BantoError> {
        // 更新では再生成しない（生成は「登録時に1回」の操作。既存の生成物を
        // 黙って作り直すと、手で直した工数が失われる）。
        self.validate(&input).await?;
        let dialect = self.db.dialect();
        let sql = format!(
            "UPDATE trips SET project_id = {}, destination = {}, start_on = {}, end_on = {}, \
             onsite_days = {}, nights = {}, note = {}, updated_at = {} \
             WHERE id = {} RETURNING {COLUMNS}",
            dialect.placeholder(1),
            dialect.placeholder(2),
            dialect.placeholder(3),
            dialect.placeholder(4),
            dialect.placeholder(5),
            dialect.placeholder(6),
            dialect.placeholder(7),
            today_expr(dialect),
            dialect.placeholder(8),
        );
        let note = input
            .note
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let row = match &self.db {
            Db::Sqlite(pool) => {
                sqlx::query_as::<_, Trip>(&sql)
                    .bind(input.project_id)
                    .bind(input.destination.trim())
                    .bind(input.start_on.trim())
                    .bind(input.end_on.trim())
                    .bind(input.onsite_days)
                    .bind(input.nights)
                    .bind(note)
                    .bind(id)
                    .fetch_one(pool)
                    .await
            }
            #[cfg(feature = "postgres")]
            Db::Postgres(pool) => {
                sqlx::query_as::<_, Trip>(&sql)
                    .bind(input.project_id)
                    .bind(input.destination.trim())
                    .bind(input.start_on.trim())
                    .bind(input.end_on.trim())
                    .bind(input.onsite_days)
                    .bind(input.nights)
                    .bind(note)
                    .bind(id)
                    .fetch_one(pool)
                    .await
            }
        }
        .map_err(|err| banto_storage::not_found(err, RESOURCE, id.to_string()))?;
        self.notify_changed();
        Ok(row)
    }

    /// この出張に紐づく生成物の件数（削除前の確認表示用。要件 F-T3）。
    pub async fn linked_record_counts(&self, id: i64) -> Result<(i64, i64), BantoError> {
        let dialect = self.db.dialect();
        let work_sql = format!(
            "SELECT COUNT(*) FROM work_logs WHERE trip_id = {} AND deleted_at IS NULL",
            dialect.placeholder(1)
        );
        let expense_sql = format!(
            "SELECT COUNT(*) FROM expenses WHERE trip_id = {} AND deleted_at IS NULL",
            dialect.placeholder(1)
        );
        let counts = match &self.db {
            Db::Sqlite(pool) => {
                let work: i64 = sqlx::query_scalar(&work_sql)
                    .bind(id)
                    .fetch_one(pool)
                    .await
                    .map_err(banto_storage::storage_error)?;
                let expense: i64 = sqlx::query_scalar(&expense_sql)
                    .bind(id)
                    .fetch_one(pool)
                    .await
                    .map_err(banto_storage::storage_error)?;
                (work, expense)
            }
            #[cfg(feature = "postgres")]
            Db::Postgres(pool) => {
                let work: i64 = sqlx::query_scalar(&work_sql)
                    .bind(id)
                    .fetch_one(pool)
                    .await
                    .map_err(banto_storage::storage_error)?;
                let expense: i64 = sqlx::query_scalar(&expense_sql)
                    .bind(id)
                    .fetch_one(pool)
                    .await
                    .map_err(banto_storage::storage_error)?;
                (work, expense)
            }
        };
        Ok(counts)
    }

    /// 出張を削除する。**生成物は消さず `trip_id` を NULL 化する**
    /// （要件 F-T3 / Phase 1 決定 C-6）。SQLite は既定で外部キーを強制
    /// しない構成もあるため、`ON DELETE SET NULL` に任せず明示的に更新して
    /// 方言差に依存しないようにする。
    pub async fn delete(&self, id: i64) -> Result<(), BantoError> {
        let dialect = self.db.dialect();
        let today = today_expr(dialect);
        // 生成物の切り離しも生きている行だけに当てる。墓石まで触ると、
        // 次の段（outbox）で「削除済みの行が変更された」という記録が立つ。
        let detach_work = format!(
            "UPDATE work_logs SET trip_id = NULL WHERE trip_id = {} AND deleted_at IS NULL",
            dialect.placeholder(1)
        );
        let detach_expense = format!(
            "UPDATE expenses SET trip_id = NULL WHERE trip_id = {} AND deleted_at IS NULL",
            dialect.placeholder(1)
        );
        // 論理削除（`docs/domain/sync.md` 5節）。物理削除だと、同期の相手が
        // 「削除された」のか「まだ届いていない」のか区別できない。
        let delete_sql = format!(
            "UPDATE trips SET deleted_at = {}, updated_at = {today} WHERE id = {} \
             AND deleted_at IS NULL",
            crate::sync::deleted_at_expr(dialect),
            dialect.placeholder(1)
        );

        let rows_affected = match &self.db {
            Db::Sqlite(pool) => {
                sqlx::query(&detach_work)
                    .bind(id)
                    .execute(pool)
                    .await
                    .map_err(banto_storage::storage_error)?;
                sqlx::query(&detach_expense)
                    .bind(id)
                    .execute(pool)
                    .await
                    .map_err(banto_storage::storage_error)?;
                sqlx::query(&delete_sql)
                    .bind(id)
                    .execute(pool)
                    .await
                    .map(|r| r.rows_affected())
            }
            #[cfg(feature = "postgres")]
            Db::Postgres(pool) => {
                sqlx::query(&detach_work)
                    .bind(id)
                    .execute(pool)
                    .await
                    .map_err(banto_storage::storage_error)?;
                sqlx::query(&detach_expense)
                    .bind(id)
                    .execute(pool)
                    .await
                    .map_err(banto_storage::storage_error)?;
                sqlx::query(&delete_sql)
                    .bind(id)
                    .execute(pool)
                    .await
                    .map(|r| r.rows_affected())
            }
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

/// 生成する行の設計図。SQL を書く前に「何件・どの日付・いくら」を確定させ、
/// SQLite / Postgres の両アームが同じ内容を書き込むようにする。
struct GenerationPlan {
    /// (作業日, 作業分類, 分, 単価, 原価, 説明)
    work_logs: Vec<(String, &'static str, i64, i64, i64, String)>,
    /// (支出日, 経費分類, 金額, 説明)
    expenses: Vec<(String, &'static str, i64, String)>,
    travel_count: usize,
    onsite_count: usize,
}

/// 一括生成の内訳を決める純粋関数（テストしやすいよう SQL から分離）。
///
/// - 移動: 往路は出発日、復路は最終日に置く（要件 F-T1 の「往復2件」）
/// - 現地作業: 開始日から1日ずつ日付を進めて `onsite_days` 件（`dates` の
///   日数加算を使う。出張期間を超える日数は検証で弾いてあるので、生成した
///   日付が期間外へはみ出すことはない）
/// - 交通費・宿泊費は出発日に1件ずつ。宿泊費は「1泊単価 × 泊数」
/// - 現地作業: 開始日から `onsite_days` 日分。日付は開始日 + n 日ではなく
///   **開始日から順に日付を進める**必要があるが、日付ライブラリを足さない
///   方針（conventions §3）のため、ここでは全件を開始日に置き、日付の調整は
///   利用者が生成結果の一覧で行う（plan.md 9.1 の「生成結果を一覧で確認・
///   個別修正」）。日付計算を自前実装して閏年を間違えるより、明示的に
///   「初日にまとめる」ほうが誤りが少ない
fn plan_generation(input: &TripInput, travel_rate: i64, onsite_rate: i64) -> GenerationPlan {
    let mut work_logs = Vec::new();
    let mut expenses = Vec::new();
    let start = input.start_on.trim().to_string();
    let end = input.end_on.trim().to_string();
    let destination = input.destination.trim();
    let gen = input
        .generate
        .as_ref()
        .expect("plan_generation is only called with a generation request");

    let mut travel_count = 0;
    if gen.travel_minutes_one_way > 0 {
        for (day, label) in [(start.clone(), "往路"), (end.clone(), "復路")] {
            work_logs.push((
                day,
                TRAVEL_CATEGORY,
                gen.travel_minutes_one_way,
                travel_rate,
                internal_cost(gen.travel_minutes_one_way, travel_rate),
                format!("{destination} 出張 {label}"),
            ));
            travel_count += 1;
        }
    }

    let mut onsite_count = 0;
    if gen.onsite_minutes_per_day > 0 {
        for day_index in 0..input.onsite_days {
            // 開始日から1日ずつ進める。書式は検証済みなので add_days は
            // 必ず Some を返すが、万一の不正入力でも初日に落として続行する。
            let worked_on = add_days(&start, day_index).unwrap_or_else(|| start.clone());
            work_logs.push((
                worked_on,
                ONSITE_CATEGORY,
                gen.onsite_minutes_per_day,
                onsite_rate,
                internal_cost(gen.onsite_minutes_per_day, onsite_rate),
                format!("{destination} 現地作業 {}日目", day_index + 1),
            ));
            onsite_count += 1;
        }
    }

    if gen.transport_amount > 0 {
        expenses.push((
            start.clone(),
            TRANSPORT_CATEGORY,
            gen.transport_amount,
            format!("{destination} 出張 交通費（往復）"),
        ));
    }
    if gen.lodging_amount_per_night > 0 && input.nights > 0 {
        expenses.push((
            start.clone(),
            LODGING_CATEGORY,
            gen.lodging_amount_per_night * input.nights,
            format!("{destination} 出張 宿泊費（{}泊）", input.nights),
        ));
    }

    GenerationPlan {
        work_logs,
        expenses,
        travel_count,
        onsite_count,
    }
}

macro_rules! create_impl {
    ($fn_name:ident, $backend:ty, $dialect:expr) => {
        /// Trip 登録 + 一括生成を1トランザクションで実行する。
        async fn $fn_name(
            pool: &sqlx::Pool<$backend>,
            input: &TripInput,
            rates: Option<(i64, i64)>,
        ) -> Result<TripGenerationResult, BantoError> {
            let dialect = $dialect;
            let today = today_expr(dialect);
            let mut tx = pool.begin().await.map_err(banto_storage::storage_error)?;

            let trip_sql = format!(
                "INSERT INTO trips (project_id, destination, start_on, end_on, onsite_days, \
                 nights, note, created_at, updated_at) \
                 VALUES ({}, {}, {}, {}, {}, {}, {}, {today}, {today}) RETURNING {COLUMNS}",
                dialect.placeholder(1),
                dialect.placeholder(2),
                dialect.placeholder(3),
                dialect.placeholder(4),
                dialect.placeholder(5),
                dialect.placeholder(6),
                dialect.placeholder(7),
            );
            let note = input
                .note
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let trip: Trip = sqlx::query_as(&trip_sql)
                .bind(input.project_id)
                .bind(input.destination.trim())
                .bind(input.start_on.trim())
                .bind(input.end_on.trim())
                .bind(input.onsite_days)
                .bind(input.nights)
                .bind(note)
                .fetch_one(&mut *tx)
                .await
                .map_err(banto_storage::storage_error)?;

            let (travel_count, onsite_count, expense_count, total_cost, total_amount) =
                match (&input.generate, rates) {
                    (Some(gen), Some((travel_rate, onsite_rate))) => {
                        let plan = plan_generation(input, travel_rate, onsite_rate);
                        let work_sql = format!(
                            "INSERT INTO work_logs (project_id, trip_id, worked_on, \
                             work_category_code, minutes, applied_rate, internal_cost, \
                             description, invoiced, created_at, updated_at) \
                             VALUES ({}, {}, {}, {}, {}, {}, {}, {}, 0, {today}, {today})",
                            dialect.placeholder(1),
                            dialect.placeholder(2),
                            dialect.placeholder(3),
                            dialect.placeholder(4),
                            dialect.placeholder(5),
                            dialect.placeholder(6),
                            dialect.placeholder(7),
                            dialect.placeholder(8),
                        );
                        let mut total_cost = 0i64;
                        for (worked_on, category, minutes, rate, cost, description) in
                            &plan.work_logs
                        {
                            sqlx::query(&work_sql)
                                .bind(input.project_id)
                                .bind(trip.id)
                                .bind(worked_on.as_str())
                                .bind(*category)
                                .bind(*minutes)
                                .bind(*rate)
                                .bind(*cost)
                                .bind(description.as_str())
                                .execute(&mut *tx)
                                .await
                                .map_err(banto_storage::storage_error)?;
                            total_cost += cost;
                        }

                        let expense_sql = format!(
                            "INSERT INTO expenses (project_id, trip_id, spent_on, \
                             expense_category_code, payee, amount, tax_category, description, \
                             billable, invoiced, created_at, updated_at) \
                             VALUES ({}, {}, {}, {}, NULL, {}, 'STANDARD_10', {}, {}, 0, \
                             {today}, {today})",
                            dialect.placeholder(1),
                            dialect.placeholder(2),
                            dialect.placeholder(3),
                            dialect.placeholder(4),
                            dialect.placeholder(5),
                            dialect.placeholder(6),
                            dialect.placeholder(7),
                        );
                        let mut total_amount = 0i64;
                        for (spent_on, category, amount, description) in &plan.expenses {
                            sqlx::query(&expense_sql)
                                .bind(input.project_id)
                                .bind(trip.id)
                                .bind(spent_on.as_str())
                                .bind(*category)
                                .bind(*amount)
                                .bind(description.as_str())
                                .bind(i64::from(gen.billable))
                                .execute(&mut *tx)
                                .await
                                .map_err(banto_storage::storage_error)?;
                            total_amount += amount;
                        }

                        (
                            plan.travel_count,
                            plan.onsite_count,
                            plan.expenses.len(),
                            total_cost,
                            total_amount,
                        )
                    }
                    _ => (0, 0, 0, 0, 0),
                };

            tx.commit().await.map_err(banto_storage::storage_error)?;

            Ok(TripGenerationResult {
                trip,
                travel_work_logs: travel_count,
                onsite_work_logs: onsite_count,
                expenses: expense_count,
                total_internal_cost: total_cost,
                total_expense_amount: total_amount,
            })
        }
    };
}

create_impl!(create_sqlite, sqlx::Sqlite, Dialect::Sqlite);
#[cfg(feature = "postgres")]
create_impl!(create_postgres, sqlx::Postgres, Dialect::Postgres);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::customers::{CustomerInput, CustomersService, DAY_END_OF_MONTH};
    use crate::db::migrate_memory;
    use crate::expenses::ExpensesService;
    use crate::masters::{CostRateInput, MastersService};
    use crate::projects::{ProjectInput, ProjectsService};
    use crate::work_logs::WorkLogsService;
    use banto_core::ListParams;

    /// 案件1件 + 移動 3,000円/時・現地 6,000円/時 を設定した状態。
    async fn fixture() -> (TripsService, WorkLogsService, ExpensesService, i64) {
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
                closing_day: Some(DAY_END_OF_MONTH),
                payment_month_offset: Some(1),
                payment_day: Some(DAY_END_OF_MONTH),
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
        let masters = MastersService::new(pool.clone());
        for (code, rate) in [("TRAVEL", 3000), ("ONSITE", 6000)] {
            masters
                .set_cost_rate(CostRateInput {
                    work_category_code: code.to_string(),
                    hourly_rate: rate,
                })
                .await
                .expect("rate");
        }
        (
            TripsService::new(pool.clone()),
            WorkLogsService::new(pool.clone()),
            ExpensesService::new(pool),
            project.id,
        )
    }

    fn trip_input(project_id: i64, generate: Option<TripGenerationInput>) -> TripInput {
        TripInput {
            project_id,
            destination: "架空工業 本社工場".to_string(),
            start_on: "2026-09-01".to_string(),
            end_on: "2026-09-03".to_string(),
            onsite_days: 3,
            nights: 2,
            note: None,
            generate,
        }
    }

    /// **論理削除の基本形と、紐づき件数が墓石を数えないこと**
    /// （`docs/domain/sync.md` 5節）。
    ///
    /// 紐づき件数は「この出張を消すと何件が孤児になるか」を利用者に見せる値。
    /// 墓石を数えると、実際には何も残っていないのに「工数2件・経費1件が
    /// 紐づいています」と出て、消してよいかの判断を誤らせる。
    #[tokio::test]
    async fn deleting_a_trip_is_a_soft_delete_and_counts_ignore_tombstones() {
        let (trips, work_logs, expenses, project_id) = fixture().await;
        trips
            .create(trip_input(project_id, None))
            .await
            .expect("残す出張");
        let generated = trips
            .create(trip_input(project_id, Some(generation())))
            .await
            .expect("消す出張");
        let doomed = generated.trip.id;

        let (work_count, expense_count) = trips.linked_record_counts(doomed).await.expect("counts");
        assert!(work_count > 0 && expense_count > 0);

        // 紐づいた工数を1件だけ論理削除すると、件数がその分だけ減る。
        let linked_work = work_logs
            .list(ListParams::default())
            .await
            .unwrap()
            .rows
            .into_iter()
            .find(|row| row.trip_id == Some(doomed))
            .expect("紐づいた工数");
        work_logs.delete(linked_work.id).await.expect("delete 工数");
        let (after_work, _) = trips.linked_record_counts(doomed).await.expect("counts");
        assert_eq!(after_work, work_count - 1, "墓石を紐づき件数に数えている");

        // 出張そのものの論理削除。
        assert_eq!(
            trips.list(ListParams::default()).await.unwrap().total_count,
            2
        );
        trips.delete(doomed).await.expect("delete 出張");

        assert!(matches!(
            trips.get(doomed).await.expect_err("墓石は get で返さない"),
            BantoError::NotFound { .. }
        ));
        let listed = trips.list(ListParams::default()).await.unwrap();
        assert_eq!(listed.total_count, 1, "削除した出張が件数に残っている");
        assert_ne!(listed.rows[0].id, doomed);

        // 生成物は残り、trip_id だけ外れている（既存の挙動を変えていない）。
        let surviving = expenses.list(ListParams::default()).await.unwrap();
        assert_eq!(
            surviving.rows.len(),
            generated.expenses,
            "生成した経費が道連れで消えている"
        );
        assert!(
            surviving.rows.iter().all(|row| row.trip_id.is_none()),
            "削除した出張への紐づきが残っている"
        );

        assert!(
            trips.delete(doomed).await.is_err(),
            "二重削除が成功している"
        );
    }

    fn generation() -> TripGenerationInput {
        TripGenerationInput {
            travel_minutes_one_way: 180,
            onsite_minutes_per_day: 480,
            transport_amount: 24_000,
            lodging_amount_per_night: 9_500,
            billable: true,
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

    /// **CLAUDE.md 第6章の必須テスト（Trip 一括生成：生成件数と内訳）。**
    #[tokio::test]
    async fn generates_two_travel_logs_onsite_logs_and_two_expenses() {
        let (trips, work_logs, expenses, project_id) = fixture().await;
        let result = trips
            .create(trip_input(project_id, Some(generation())))
            .await
            .expect("create with generation");

        // 内訳: 移動 往復2件 / 現地 3日分 / 経費 2件（交通費・宿泊費）
        assert_eq!(result.travel_work_logs, 2);
        assert_eq!(result.onsite_work_logs, 3);
        assert_eq!(result.expenses, 2);

        // 金額: 移動 180分 × 3,000円/時 = 9,000円 × 2 = 18,000円
        //       現地 480分 × 6,000円/時 = 48,000円 × 3 = 144,000円
        assert_eq!(result.total_internal_cost, 18_000 + 144_000);
        // 交通費 24,000 + 宿泊費 9,500 × 2泊 = 43,000円
        assert_eq!(result.total_expense_amount, 24_000 + 19_000);

        // 実際に DB に入っている件数も確認する（返り値の自己申告だけを
        // 信じない）。
        let logs = work_logs
            .list(ListParams::default())
            .await
            .expect("work log list");
        assert_eq!(logs.total_count, 5);
        assert!(logs
            .rows
            .iter()
            .all(|row| row.trip_id == Some(result.trip.id)));
        assert_eq!(
            logs.rows
                .iter()
                .filter(|row| row.work_category_code == "TRAVEL")
                .count(),
            2
        );

        let costs = expenses
            .list(ListParams::default())
            .await
            .expect("expense list");
        assert_eq!(costs.total_count, 2);
        // 生成した経費は既定で顧客請求対象（Phase 1 決定 C-7）。
        assert!(costs.rows.iter().all(|row| row.billable == 1));
        let lodging = costs
            .rows
            .iter()
            .find(|row| row.expense_category_code == "LODGING")
            .expect("lodging expense");
        assert_eq!(lodging.amount, 19_000, "1泊単価 × 泊数");

        // 現地作業は開始日から1日ずつ進む（初日にまとめない）。
        let mut onsite_days: Vec<&str> = logs
            .rows
            .iter()
            .filter(|row| row.work_category_code == "ONSITE")
            .map(|row| row.worked_on.as_str())
            .collect();
        onsite_days.sort_unstable();
        assert_eq!(onsite_days, ["2026-09-01", "2026-09-02", "2026-09-03"]);

        // 移動は往路が出発日・復路が最終日。
        let mut travel_days: Vec<&str> = logs
            .rows
            .iter()
            .filter(|row| row.work_category_code == "TRAVEL")
            .map(|row| row.worked_on.as_str())
            .collect();
        travel_days.sort_unstable();
        assert_eq!(travel_days, ["2026-09-01", "2026-09-03"]);
    }

    /// 移動時間0・宿泊0 のような「一部だけ生成しない」指定を許す
    /// （日帰り出張で交通費だけ、など）。
    #[tokio::test]
    async fn zero_values_skip_the_corresponding_rows() {
        let (trips, work_logs, expenses, project_id) = fixture().await;
        let mut input = trip_input(project_id, Some(generation()));
        input.nights = 0;
        if let Some(gen) = input.generate.as_mut() {
            gen.travel_minutes_one_way = 0;
            gen.lodging_amount_per_night = 0;
        }
        let result = trips.create(input).await.expect("create");
        assert_eq!(result.travel_work_logs, 0);
        assert_eq!(result.onsite_work_logs, 3);
        assert_eq!(result.expenses, 1, "交通費のみ");

        assert_eq!(
            work_logs
                .list(ListParams::default())
                .await
                .expect("list")
                .total_count,
            3
        );
        assert_eq!(
            expenses
                .list(ListParams::default())
                .await
                .expect("list")
                .total_count,
            1
        );
    }

    /// 生成を指示しなければ Trip だけが登録される。
    #[tokio::test]
    async fn without_generation_only_the_trip_is_created() {
        let (trips, work_logs, _, project_id) = fixture().await;
        let result = trips
            .create(trip_input(project_id, None))
            .await
            .expect("create");
        assert_eq!(result.travel_work_logs + result.onsite_work_logs, 0);
        assert_eq!(
            work_logs
                .list(ListParams::default())
                .await
                .expect("list")
                .total_count,
            0
        );
    }

    /// レート未設定の分類で生成しようとしたら、単価0で作らずエラーにする。
    /// **Trip 自体も作られない**（トランザクションで巻き戻る前に弾く）。
    #[tokio::test]
    async fn missing_rate_aborts_the_whole_creation() {
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
                closing_day: Some(DAY_END_OF_MONTH),
                payment_month_offset: Some(1),
                payment_day: Some(DAY_END_OF_MONTH),
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
        let trips = TripsService::new(pool.clone());

        let err = trips
            .create(trip_input(project.id, Some(generation())))
            .await
            .expect_err("rates are unset");
        let fields = field_errors(&err);
        assert!(
            fields.iter().any(|f| f.starts_with("generate.")),
            "{fields:?}"
        );

        assert_eq!(
            trips
                .list(ListParams::default())
                .await
                .expect("list")
                .total_count,
            0,
            "Trip も作られていない"
        );
    }

    /// **要件 F-T3 / Phase 1 決定 C-6。** 出張を削除しても生成物は残り、
    /// `trip_id` だけが NULL になる。工数実績が消えると案件採算が壊れるため。
    #[tokio::test]
    async fn deleting_a_trip_detaches_but_keeps_the_generated_rows() {
        let (trips, work_logs, expenses, project_id) = fixture().await;
        let result = trips
            .create(trip_input(project_id, Some(generation())))
            .await
            .expect("create");

        // 削除前に影響件数を提示できる（要件 F-T3 の確認表示）。
        let (work_count, expense_count) = trips
            .linked_record_counts(result.trip.id)
            .await
            .expect("counts");
        assert_eq!((work_count, expense_count), (5, 2));

        trips.delete(result.trip.id).await.expect("delete");

        let logs = work_logs.list(ListParams::default()).await.expect("list");
        assert_eq!(logs.total_count, 5, "工数は残る");
        assert!(
            logs.rows.iter().all(|row| row.trip_id.is_none()),
            "trip_id は NULL 化"
        );

        let costs = expenses.list(ListParams::default()).await.expect("list");
        assert_eq!(costs.total_count, 2, "経費も残る");
        assert!(costs.rows.iter().all(|row| row.trip_id.is_none()));

        assert!(matches!(
            trips.get(result.trip.id).await.expect_err("gone"),
            BantoError::NotFound { .. }
        ));
    }

    #[tokio::test]
    async fn invalid_period_and_counts_are_rejected() {
        let (trips, _, _, project_id) = fixture().await;
        let mut reversed = trip_input(project_id, None);
        reversed.start_on = "2026-09-05".to_string();
        reversed.end_on = "2026-09-01".to_string();
        assert!(
            field_errors(&trips.create(reversed).await.expect_err("reversed"))
                .contains(&"endOn".to_string())
        );

        let mut too_many_days = trip_input(project_id, None);
        too_many_days.onsite_days = 61;
        assert!(
            field_errors(&trips.create(too_many_days).await.expect_err("too many"))
                .contains(&"onsiteDays".to_string())
        );
    }

    /// 出張期間に収まらない現地作業日数・泊数は入力ミスとして弾く
    /// （生成した工数の日付が期間外へはみ出すため）。
    #[tokio::test]
    async fn counts_must_fit_inside_the_trip_period() {
        let (trips, _, _, project_id) = fixture().await;
        // 2026-09-01〜09-03 は3日間・最大2泊。
        let mut too_many_onsite = trip_input(project_id, None);
        too_many_onsite.onsite_days = 4;
        assert!(field_errors(
            &trips
                .create(too_many_onsite)
                .await
                .expect_err("4日は入らない")
        )
        .contains(&"onsiteDays".to_string()));

        let mut too_many_nights = trip_input(project_id, None);
        too_many_nights.nights = 3;
        assert!(field_errors(
            &trips
                .create(too_many_nights)
                .await
                .expect_err("3泊は入らない")
        )
        .contains(&"nights".to_string()));

        // 境界（3日・2泊）はそのまま通る。
        trips
            .create(trip_input(project_id, None))
            .await
            .expect("3日2泊は妥当");
    }

    /// 更新では再生成しない（手で直した生成物を黙って作り直さない）。
    #[tokio::test]
    async fn update_does_not_regenerate() {
        let (trips, work_logs, _, project_id) = fixture().await;
        let created = trips
            .create(trip_input(project_id, Some(generation())))
            .await
            .expect("create");

        let mut input = trip_input(project_id, Some(generation()));
        input.destination = "架空工業 第二工場".to_string();
        let updated = trips.update(created.trip.id, input).await.expect("update");
        assert_eq!(updated.destination, "架空工業 第二工場");
        assert_eq!(
            work_logs
                .list(ListParams::default())
                .await
                .expect("list")
                .total_count,
            5,
            "生成物は増えない"
        );
    }
}
