//! Projects resource service (Phase 2 基本マスター、`docs/domain/schema.md`
//! §2.2): 案件マスタのドメインロジック。conventions §2 に従い `tauri` /
//! `axum` / RBAC / HTTP を知らない。
//!
//! 金額（見積額・契約額）は INTEGER（円・税抜）で保持する（CLAUDE.md 1.1）。
//! 契約額は粗利計算には使わず、請求進捗（案件売上 ÷ 契約額）の分母として
//! 使う（Phase 1 決定 C-3、`docs/domain/open-questions.md`）。
//!
//! 状態は7値のコード（Phase 1 決定 C-12）。採算集計の対象は `LOST` を除く
//! 全状態で、この判定は Phase 4 の採算サービス側が [`ProjectStatus::
//! counts_toward_profitability`] を使って行う — 画面やクエリで
//! `status != "LOST"` を直書きしない。

use crate::dates::is_valid_date;
use banto_core::{BantoError, FieldError, ListParams, ListResult};
use banto_server::ServerEvent;
use banto_storage::{ColumnMap, Db, Dialect};
use serde::{Deserialize, Serialize};
use sqlx::{QueryBuilder, Sqlite};
use tokio::sync::broadcast;

/// 案件の状態（Phase 1 決定 C-12）。DB には文字列コードで保持する。
///
/// 「採算集計の対象か」をコード文字列の比較ではなくこの列挙で判定するのは、
/// `AGENTS.md` 3.2 が作業分類について定めているのと同じ理由 — 状態を増減
/// したときに集計側のロジックを直さずに済ませるため。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectStatus {
    /// 見込
    Prospect,
    /// 受注
    Ordered,
    /// 進行中
    InProgress,
    /// 検収待ち
    AwaitingAcceptance,
    /// 完了
    Completed,
    /// 失注
    Lost,
    /// 保留
    OnHold,
}

impl ProjectStatus {
    pub const ALL: [ProjectStatus; 7] = [
        ProjectStatus::Prospect,
        ProjectStatus::Ordered,
        ProjectStatus::InProgress,
        ProjectStatus::AwaitingAcceptance,
        ProjectStatus::Completed,
        ProjectStatus::Lost,
        ProjectStatus::OnHold,
    ];

    pub fn as_code(self) -> &'static str {
        match self {
            ProjectStatus::Prospect => "PROSPECT",
            ProjectStatus::Ordered => "ORDERED",
            ProjectStatus::InProgress => "IN_PROGRESS",
            ProjectStatus::AwaitingAcceptance => "AWAITING_ACCEPTANCE",
            ProjectStatus::Completed => "COMPLETED",
            ProjectStatus::Lost => "LOST",
            ProjectStatus::OnHold => "ON_HOLD",
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        ProjectStatus::ALL.into_iter().find(|s| s.as_code() == code)
    }

    /// 案件採算の集計対象か（Phase 1 決定 C-12: 失注のみ対象外）。
    pub fn counts_toward_profitability(self) -> bool {
        self != ProjectStatus::Lost
    }

    /// ダッシュボードの「進行中案件」に含めるか（Phase 1 決定 C-12）。
    pub fn is_active(self) -> bool {
        matches!(
            self,
            ProjectStatus::Ordered | ProjectStatus::InProgress | ProjectStatus::AwaitingAcceptance
        )
    }
}

/// `projects` の1行。ワイヤ形状（camelCase）はフロントのリソース定義に
/// 合わせる。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: i64,
    pub code: String,
    #[sqlx(rename = "customer_id")]
    pub customer_id: i64,
    pub name: String,
    pub status: String,
    #[sqlx(rename = "started_on")]
    pub started_on: Option<String>,
    #[sqlx(rename = "due_on")]
    pub due_on: Option<String>,
    /// 見積額（円・税抜）。
    #[sqlx(rename = "estimate_amount")]
    pub estimate_amount: Option<i64>,
    /// 契約額（円・税抜）。粗利計算には使わない（請求進捗の分母）。
    #[sqlx(rename = "contract_amount")]
    pub contract_amount: Option<i64>,
    /// 請求時間単価（円/時・税抜。決定 C-17）。工数から請求明細を起こすときの
    /// 単価で、**内部原価の `cost_rates` とは別物**（CLAUDE.md 1.2）。未設定なら
    /// `None` で、請求書の候補生成時に単価の入力を求める。
    #[sqlx(rename = "billing_hourly_rate")]
    pub billing_hourly_rate: Option<i64>,
    pub scope: Option<String>,
    pub note: Option<String>,
    #[sqlx(rename = "created_at")]
    pub created_at: String,
    #[sqlx(rename = "updated_at")]
    pub updated_at: String,
}

/// 作成・更新のペイロード。`code` を空で送ると `YYYY-NNN` を自動採番する
/// （要件 F-M3。手修正できるよう、採番後も通常の編集対象）。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectInput {
    #[serde(default)]
    pub code: String,
    pub customer_id: i64,
    pub name: String,
    pub status: String,
    pub started_on: Option<String>,
    pub due_on: Option<String>,
    pub estimate_amount: Option<i64>,
    pub contract_amount: Option<i64>,
    pub billing_hourly_rate: Option<i64>,
    pub scope: Option<String>,
    pub note: Option<String>,
}

const MAX_CODE_LEN: usize = 20;
const MAX_NAME_LEN: usize = 80;
const MAX_SCOPE_LEN: usize = 200;
const MAX_NOTE_LEN: usize = 500;
/// 金額の上限（円）。i64 の範囲より遥かに手前で弾くのは、桁の打ち間違い
/// （100万を1億と打つ等）を入力時点で捕まえるため。
const MAX_AMOUNT: i64 = 9_999_999_999;

fn required_message() -> String {
    "必須項目です".to_string()
}

fn max_length_message(max: usize) -> String {
    format!("{max}文字以内で入力してください")
}

struct NormalizedProject {
    code: String,
    customer_id: i64,
    name: String,
    status: String,
    started_on: Option<String>,
    due_on: Option<String>,
    estimate_amount: Option<i64>,
    contract_amount: Option<i64>,
    billing_hourly_rate: Option<i64>,
    scope: Option<String>,
    note: Option<String>,
}

fn check_optional_text(
    errors: &mut Vec<FieldError>,
    field: &str,
    value: &Option<String>,
    max_len: usize,
) -> Option<String> {
    let trimmed = value.as_deref().map(str::trim).unwrap_or("");
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.chars().count() > max_len {
        errors.push(FieldError {
            field: field.to_string(),
            message: max_length_message(max_len),
        });
    }
    Some(trimmed.to_string())
}

fn check_optional_date(
    errors: &mut Vec<FieldError>,
    field: &str,
    value: &Option<String>,
) -> Option<String> {
    let trimmed = value.as_deref().map(str::trim).unwrap_or("");
    if trimmed.is_empty() {
        return None;
    }
    if !is_valid_date(trimmed) {
        errors.push(FieldError {
            field: field.to_string(),
            message: "YYYY-MM-DD の形式で入力してください".to_string(),
        });
    }
    Some(trimmed.to_string())
}

fn check_optional_amount(errors: &mut Vec<FieldError>, field: &str, value: Option<i64>) {
    if let Some(amount) = value {
        if amount < 0 {
            errors.push(FieldError {
                field: field.to_string(),
                message: "0以上で入力してください".to_string(),
            });
        } else if amount > MAX_AMOUNT {
            errors.push(FieldError {
                field: field.to_string(),
                message: format!("{MAX_AMOUNT}以下で入力してください"),
            });
        }
    }
}

/// [`ProjectInput`] を検証する。`code` は空を許す（自動採番のため、採番は
/// [`ProjectsService::create`] 側で行う）。
fn validate(input: &ProjectInput) -> Result<NormalizedProject, BantoError> {
    let mut errors: Vec<FieldError> = Vec::new();

    let code = input.code.trim().to_string();
    if code.chars().count() > MAX_CODE_LEN {
        errors.push(FieldError {
            field: "code".to_string(),
            message: max_length_message(MAX_CODE_LEN),
        });
    }

    let name = input.name.trim().to_string();
    if name.is_empty() {
        errors.push(FieldError {
            field: "name".to_string(),
            message: required_message(),
        });
    } else if name.chars().count() > MAX_NAME_LEN {
        errors.push(FieldError {
            field: "name".to_string(),
            message: max_length_message(MAX_NAME_LEN),
        });
    }

    if ProjectStatus::from_code(input.status.trim()).is_none() {
        errors.push(FieldError {
            field: "status".to_string(),
            message: "状態の値が不正です".to_string(),
        });
    }

    let started_on = check_optional_date(&mut errors, "startedOn", &input.started_on);
    let due_on = check_optional_date(&mut errors, "dueOn", &input.due_on);
    // 開始日 > 終了予定日 を弾く。文字列比較で足りるのは ISO 8601 の
    // `YYYY-MM-DD` が辞書順 = 時系列順になるため（`CLAUDE.md` 第4章の
    // 日付表現を前提にした意図的な比較）。
    if let (Some(start), Some(due)) = (&started_on, &due_on) {
        if is_valid_date(start) && is_valid_date(due) && start > due {
            errors.push(FieldError {
                field: "dueOn".to_string(),
                message: "終了予定日は開始日以降にしてください".to_string(),
            });
        }
    }

    check_optional_amount(&mut errors, "estimateAmount", input.estimate_amount);
    check_optional_amount(&mut errors, "contractAmount", input.contract_amount);
    check_optional_amount(&mut errors, "billingHourlyRate", input.billing_hourly_rate);

    let scope = check_optional_text(&mut errors, "scope", &input.scope, MAX_SCOPE_LEN);
    let note = check_optional_text(&mut errors, "note", &input.note, MAX_NOTE_LEN);

    if errors.is_empty() {
        Ok(NormalizedProject {
            code,
            customer_id: input.customer_id,
            name,
            status: input.status.trim().to_string(),
            started_on,
            due_on,
            estimate_amount: input.estimate_amount,
            contract_amount: input.contract_amount,
            billing_hourly_rate: input.billing_hourly_rate,
            scope,
            note,
        })
    } else {
        Err(BantoError::Validation {
            field_errors: errors,
        })
    }
}

fn column_map() -> ColumnMap {
    ColumnMap::new()
        .column("id", "id")
        .column("code", "code")
        .column("customerId", "customer_id")
        .column("name", "name")
        .column("status", "status")
        .column("startedOn", "started_on")
        .column("dueOn", "due_on")
        .column("estimateAmount", "estimate_amount")
        .column("contractAmount", "contract_amount")
        .column("billingHourlyRate", "billing_hourly_rate")
        .column("scope", "scope")
        .column("note", "note")
        .column("createdAt", "created_at")
        .column("updatedAt", "updated_at")
}

const RESOURCE: &str = "projects";
const COLUMNS: &str = "id, code, customer_id, name, status, started_on, due_on, \
     estimate_amount, contract_amount, billing_hourly_rate, scope, note, \
     created_at, updated_at";

fn today_expr(dialect: Dialect) -> &'static str {
    match dialect {
        Dialect::Sqlite => "date('now')",
        Dialect::Postgres => "CURRENT_DATE::text",
    }
}

fn map_unique_violation(err: sqlx::Error, code: &str) -> BantoError {
    let is_unique_violation = err
        .as_database_error()
        .map(|db_err| {
            db_err.code().as_deref() == Some("23505")
                || db_err.message().contains("UNIQUE constraint failed")
        })
        .unwrap_or(false);
    if is_unique_violation {
        BantoError::Validation {
            field_errors: vec![FieldError {
                field: "code".to_string(),
                message: format!("案件番号「{code}」は既に使われています"),
            }],
        }
    } else {
        banto_storage::storage_error(err)
    }
}

/// 案件マスタのサービス層（conventions §2）。
#[derive(Clone)]
pub struct ProjectsService {
    db: Db,
    events: Option<broadcast::Sender<ServerEvent>>,
}

impl ProjectsService {
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

    /// DB 側の「今日」を `YYYY-MM-DD` で取る。日付ライブラリを足さない文化
    /// （conventions §3）に従い、ホスト時計ではなく SQL の日付式を使う
    /// （方言差は [`today_expr`] が吸収する）。
    async fn today(&self) -> Result<String, BantoError> {
        let sql = format!("SELECT {}", today_expr(self.db.dialect()));
        match &self.db {
            Db::Sqlite(pool) => sqlx::query_scalar::<_, String>(&sql).fetch_one(pool).await,
            #[cfg(feature = "postgres")]
            Db::Postgres(pool) => sqlx::query_scalar::<_, String>(&sql).fetch_one(pool).await,
        }
        .map_err(banto_storage::storage_error)
    }

    /// 次の案件番号 `YYYY-NNN` を返す（要件 F-M3）。同じ年の既存コードのうち
    /// 最大の連番 + 1。欠番は詰めない（請求書番号と違い、案件番号に連続性の
    /// 要件は無いが、採番のたびに過去を走査して詰めると既存の番号と衝突する）。
    pub async fn next_code(&self) -> Result<String, BantoError> {
        let today = self.today().await?;
        let year = &today[..4];
        let pattern = format!("{year}-%");
        let dialect = self.db.dialect();
        let sql = format!(
            "SELECT code FROM projects WHERE code LIKE {} ORDER BY code DESC LIMIT 1",
            dialect.placeholder(1)
        );
        let latest: Option<String> = match &self.db {
            Db::Sqlite(pool) => {
                sqlx::query_scalar(&sql)
                    .bind(&pattern)
                    .fetch_optional(pool)
                    .await
            }
            #[cfg(feature = "postgres")]
            Db::Postgres(pool) => {
                sqlx::query_scalar(&sql)
                    .bind(&pattern)
                    .fetch_optional(pool)
                    .await
            }
        }
        .map_err(banto_storage::storage_error)?;

        let next = latest
            .as_deref()
            .and_then(|code| code.split_once('-'))
            .and_then(|(_, seq)| seq.parse::<u32>().ok())
            .unwrap_or(0)
            + 1;
        Ok(format!("{year}-{next:03}"))
    }

    /// 顧客が存在するかを確認する。存在しない顧客 ID を外部キー任せにすると
    /// 素の DB エラーになり、フォームのどの欄が悪いのか利用者に伝わらない。
    async fn ensure_customer_exists(&self, customer_id: i64) -> Result<(), BantoError> {
        let dialect = self.db.dialect();
        let sql = format!(
            "SELECT COUNT(*) FROM customers WHERE id = {} AND deleted_at IS NULL",
            dialect.placeholder(1)
        );
        let count: i64 = match &self.db {
            Db::Sqlite(pool) => {
                sqlx::query_scalar(&sql)
                    .bind(customer_id)
                    .fetch_one(pool)
                    .await
            }
            #[cfg(feature = "postgres")]
            Db::Postgres(pool) => {
                sqlx::query_scalar(&sql)
                    .bind(customer_id)
                    .fetch_one(pool)
                    .await
            }
        }
        .map_err(banto_storage::storage_error)?;
        if count == 0 {
            return Err(BantoError::Validation {
                field_errors: vec![FieldError {
                    field: "customerId".to_string(),
                    message: "顧客を選択してください".to_string(),
                }],
            });
        }
        Ok(())
    }

    pub async fn list(&self, params: ListParams) -> Result<ListResult<Project>, BantoError> {
        let columns = column_map();
        let select_rows = format!("SELECT {COLUMNS} FROM {}", crate::sync::live("projects"));
        let select_count = format!("SELECT COUNT(*) FROM {}", crate::sync::live("projects"));

        match &self.db {
            Db::Sqlite(pool) => {
                let mut rows_builder: QueryBuilder<'_, Sqlite> = QueryBuilder::new(&select_rows);
                banto_storage::list_query::sqlite::apply_list_params(
                    &mut rows_builder,
                    &columns,
                    &params,
                )?;
                let rows: Vec<Project> = rows_builder
                    .build_query_as::<Project>()
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
                let rows: Vec<Project> = rows_builder
                    .build_query_as::<Project>()
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

    pub async fn get(&self, id: i64) -> Result<Project, BantoError> {
        let dialect = self.db.dialect();
        let sql = format!(
            "SELECT {COLUMNS} FROM projects WHERE id = {} AND deleted_at IS NULL",
            dialect.placeholder(1)
        );
        match &self.db {
            Db::Sqlite(pool) => {
                sqlx::query_as::<_, Project>(&sql)
                    .bind(id)
                    .fetch_one(pool)
                    .await
            }
            #[cfg(feature = "postgres")]
            Db::Postgres(pool) => {
                sqlx::query_as::<_, Project>(&sql)
                    .bind(id)
                    .fetch_one(pool)
                    .await
            }
        }
        .map_err(|err| banto_storage::not_found(err, RESOURCE, id.to_string()))
    }

    pub async fn create(&self, input: ProjectInput) -> Result<Project, BantoError> {
        let mut value = validate(&input)?;
        self.ensure_customer_exists(value.customer_id).await?;
        if value.code.is_empty() {
            value.code = self.next_code().await?;
        }
        let dialect = self.db.dialect();
        let today = today_expr(dialect);
        let sql = format!(
            "INSERT INTO projects (code, customer_id, name, status, started_on, due_on, \
             estimate_amount, contract_amount, billing_hourly_rate, scope, note, \
             created_at, updated_at) \
             VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {today}, {today}) \
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
            dialect.placeholder(11),
        );
        let project = match &self.db {
            Db::Sqlite(pool) => {
                bind_input(sqlx::query_as::<_, Project>(&sql), &value)
                    .fetch_one(pool)
                    .await
            }
            #[cfg(feature = "postgres")]
            Db::Postgres(pool) => {
                bind_input(sqlx::query_as::<_, Project>(&sql), &value)
                    .fetch_one(pool)
                    .await
            }
        }
        .map_err(|err| map_unique_violation(err, &value.code))?;
        self.notify_changed();
        Ok(project)
    }

    pub async fn update(&self, id: i64, input: ProjectInput) -> Result<Project, BantoError> {
        let value = validate(&input)?;
        if value.code.is_empty() {
            return Err(BantoError::Validation {
                field_errors: vec![FieldError {
                    field: "code".to_string(),
                    message: required_message(),
                }],
            });
        }
        self.ensure_customer_exists(value.customer_id).await?;
        let dialect = self.db.dialect();
        let sql = format!(
            "UPDATE projects SET code = {}, customer_id = {}, name = {}, status = {}, \
             started_on = {}, due_on = {}, estimate_amount = {}, contract_amount = {}, \
             billing_hourly_rate = {}, scope = {}, note = {}, updated_at = {} \
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
            dialect.placeholder(11),
            today_expr(dialect),
            dialect.placeholder(12),
        );
        let project = match &self.db {
            Db::Sqlite(pool) => {
                bind_input(sqlx::query_as::<_, Project>(&sql), &value)
                    .bind(id)
                    .fetch_one(pool)
                    .await
            }
            #[cfg(feature = "postgres")]
            Db::Postgres(pool) => {
                bind_input(sqlx::query_as::<_, Project>(&sql), &value)
                    .bind(id)
                    .fetch_one(pool)
                    .await
            }
        }
        .map_err(|err| {
            if matches!(err, sqlx::Error::RowNotFound) {
                BantoError::NotFound {
                    resource: RESOURCE.to_string(),
                    id: id.to_string(),
                }
            } else {
                map_unique_violation(err, &value.code)
            }
        })?;
        self.notify_changed();
        Ok(project)
    }

    /// 案件にぶら下がっている**生きている**子の件数を数え、1件でもあれば
    /// `Validation` を返す。
    ///
    /// 対象は `work_logs` / `expenses` / `trips` / `invoice_lines`。
    /// `invoice_lines` を含めるのは、確定済みの請求書が参照している案件を
    /// 消せてしまうと、請求書の明細から案件名を辿れなくなるため
    /// （`invoice_lines` は論理削除の対象外なので `deleted_at` を見ない）。
    async fn ensure_no_children(&self, id: i64) -> Result<(), BantoError> {
        let dialect = self.db.dialect();
        // (表示名, SQL) の順に数える。最初に見つかった1件で止めず全部数えるのは、
        // 「工数を消したら次は経費で怒られた」を繰り返させないため。
        let checks = [
            ("工数", "work_logs", true),
            ("経費", "expenses", true),
            ("出張", "trips", true),
            ("請求明細", "invoice_lines", false),
        ];
        let mut blockers: Vec<String> = Vec::new();
        for (label, table, soft_deletable) in checks {
            let alive = if soft_deletable {
                " AND deleted_at IS NULL"
            } else {
                ""
            };
            let sql = format!(
                "SELECT COUNT(*) FROM {table} WHERE project_id = {}{alive}",
                dialect.placeholder(1)
            );
            let count: i64 = match &self.db {
                Db::Sqlite(pool) => sqlx::query_scalar(&sql).bind(id).fetch_one(pool).await,
                #[cfg(feature = "postgres")]
                Db::Postgres(pool) => sqlx::query_scalar(&sql).bind(id).fetch_one(pool).await,
            }
            .map_err(banto_storage::storage_error)?;
            if count > 0 {
                blockers.push(format!("{label}{count}件"));
            }
        }
        if !blockers.is_empty() {
            return Err(BantoError::Validation {
                field_errors: vec![FieldError {
                    field: "id".to_string(),
                    message: format!(
                        "この案件には{}が紐づいているため削除できません。先に削除するか別の案件へ付け替えてください",
                        blockers.join("・")
                    ),
                }],
            });
        }
        Ok(())
    }

    /// 案件を削除する（**論理削除**。`docs/domain/sync.md` 5節）。
    ///
    /// **子が残っていれば拒否する。** 物理削除だった頃は外部キーが弾いていたが
    /// （`work_logs` / `expenses` / `trips` / `invoice_lines` が `project_id` を
    /// 参照する）、論理削除では行が消えないので外部キーは何も言わない。
    /// ガードを置かないと、工数がぶら下がったまま案件だけが見えなくなり、
    /// **採算からも一覧からも辿れない工数**が残る。
    ///
    /// 数えるのは生きている子だけ。墓石を数えると、実際には何も残っていない
    /// 案件が永久に消せなくなる（顧客側の同じ判断と揃える）。
    pub async fn delete(&self, id: i64) -> Result<(), BantoError> {
        self.ensure_no_children(id).await?;
        let dialect = self.db.dialect();
        let today = today_expr(dialect);
        let sql = format!(
            "UPDATE projects SET deleted_at = {}, updated_at = {today} WHERE id = {} \
             AND deleted_at IS NULL",
            crate::sync::deleted_at_expr(dialect),
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

/// INSERT / UPDATE の bind 順を1箇所に閉じる（`customers.rs` と同じ理由）。
fn bind_input<'q, DB>(
    query: sqlx::query::QueryAs<'q, DB, Project, <DB as sqlx::Database>::Arguments<'q>>,
    value: &'q NormalizedProject,
) -> sqlx::query::QueryAs<'q, DB, Project, <DB as sqlx::Database>::Arguments<'q>>
where
    DB: sqlx::Database,
    &'q str: sqlx::Type<DB> + sqlx::Encode<'q, DB>,
    i64: sqlx::Type<DB> + sqlx::Encode<'q, DB>,
    Option<i64>: sqlx::Type<DB> + sqlx::Encode<'q, DB>,
    Option<&'q str>: sqlx::Type<DB> + sqlx::Encode<'q, DB>,
{
    query
        .bind(value.code.as_str())
        .bind(value.customer_id)
        .bind(value.name.as_str())
        .bind(value.status.as_str())
        .bind(value.started_on.as_deref())
        .bind(value.due_on.as_deref())
        .bind(value.estimate_amount)
        .bind(value.contract_amount)
        .bind(value.billing_hourly_rate)
        .bind(value.scope.as_deref())
        .bind(value.note.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::customers::{CustomerInput, CustomersService, DAY_END_OF_MONTH};
    use crate::db::migrate_memory;

    /// 顧客を1件だけ作った状態のサービス（案件は顧客に属するため）。
    async fn service_with_customer() -> (ProjectsService, i64) {
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
                closing_day: Some(DAY_END_OF_MONTH),
                payment_month_offset: Some(1),
                payment_day: Some(DAY_END_OF_MONTH),
                note: None,
            })
            .await
            .expect("customer create");
        (ProjectsService::new(pool), customer.id)
    }

    fn valid_input(customer_id: i64) -> ProjectInput {
        ProjectInput {
            code: String::new(),
            customer_id,
            name: "架空ライン制御盤更新".to_string(),
            status: "ORDERED".to_string(),
            started_on: None,
            due_on: None,
            estimate_amount: None,
            contract_amount: None,
            billing_hourly_rate: None,
            scope: None,
            note: None,
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

    /// **論理削除の基本形**（`docs/domain/sync.md` 5節）。
    #[tokio::test]
    async fn deleting_a_project_is_a_soft_delete() {
        let (svc, customer_id) = service_with_customer().await;
        let mut keep = valid_input(customer_id);
        keep.code = "P001".to_string();
        svc.create(keep).await.expect("残す");
        let mut doomed_input = valid_input(customer_id);
        doomed_input.code = "P002".to_string();
        let doomed = svc.create(doomed_input).await.expect("消す").id;

        svc.delete(doomed).await.expect("delete");

        assert!(matches!(
            svc.get(doomed).await.expect_err("墓石は get で返さない"),
            BantoError::NotFound { .. }
        ));
        let listed = svc.list(ListParams::default()).await.unwrap();
        assert_eq!(listed.total_count, 1);
        assert_ne!(listed.rows[0].id, doomed);
        assert!(svc.delete(doomed).await.is_err(), "二重削除が成功している");
    }

    /// **消した顧客の下に案件を作れないこと。**
    ///
    /// 顧客の存在確認が墓石まで見ると、削除済みの顧客にぶら下がる案件を
    /// 作れてしまい、一覧から辿れない案件ができる。
    #[tokio::test]
    async fn a_tombstoned_customer_cannot_receive_new_projects() {
        let db = migrate_memory().await.expect("migrate_memory");
        let customers = CustomersService::new(db.clone());
        let customer = customers
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
        customers.delete(customer.id).await.expect("delete 顧客");

        let projects = ProjectsService::new(db);
        let err = projects
            .create(valid_input(customer.id))
            .await
            .expect_err("消した顧客に案件を作れている");
        assert_eq!(
            field_errors(&err)
                .into_iter()
                .map(|(f, _)| f)
                .collect::<Vec<_>>(),
            vec!["customerId"]
        );
    }

    /// **子が生きている案件は消せない。**
    ///
    /// 物理削除の頃は外部キーが弾いていたが、論理削除では行が消えないので
    /// 外部キーは何も言わない。ガードが無いと、工数がぶら下がったまま案件だけ
    /// 見えなくなり、採算からも一覧からも辿れない工数が残る。
    #[tokio::test]
    async fn a_project_with_live_children_cannot_be_deleted() {
        let db = migrate_memory().await.expect("migrate_memory");
        let customers = CustomersService::new(db.clone());
        let customer = customers
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
        let projects = ProjectsService::new(db.clone());
        let project = projects
            .create(valid_input(customer.id))
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

        let err = projects
            .delete(project.id)
            .await
            .expect_err("工数が残っているのに案件を消せている");
        let (field, message) = field_errors(&err).into_iter().next().expect("field error");
        assert_eq!(field, "id");
        assert!(message.contains("工数1件"), "{message}");

        // 工数を論理削除すると、案件も消せるようになる（墓石は数えない）。
        work_logs.delete(work_log.id).await.expect("delete 工数");
        projects
            .delete(project.id)
            .await
            .expect("墓石の工数が案件の削除をブロックしている");
    }

    #[tokio::test]
    async fn create_then_get_round_trips() {
        let (svc, customer_id) = service_with_customer().await;
        let mut input = valid_input(customer_id);
        input.contract_amount = Some(1_200_000);
        let created = svc.create(input).await.expect("create");
        assert_eq!(created.contract_amount, Some(1_200_000));
        assert_eq!(svc.get(created.id).await.expect("get"), created);
    }

    /// 空の `code` は `YYYY-NNN` で自動採番する（要件 F-M3）。
    #[tokio::test]
    async fn blank_code_is_auto_numbered_per_year() {
        let (svc, customer_id) = service_with_customer().await;
        let first = svc.create(valid_input(customer_id)).await.expect("first");
        let second = svc.create(valid_input(customer_id)).await.expect("second");

        let year = &first.code[..4];
        assert_eq!(first.code, format!("{year}-001"));
        assert_eq!(second.code, format!("{year}-002"));
        assert!(
            year.chars().all(|c| c.is_ascii_digit()),
            "year prefix: {year}"
        );
    }

    #[tokio::test]
    async fn explicit_code_is_kept_as_given() {
        let (svc, customer_id) = service_with_customer().await;
        let mut input = valid_input(customer_id);
        input.code = "SPECIAL-1".to_string();
        let created = svc.create(input).await.expect("create");
        assert_eq!(created.code, "SPECIAL-1");
    }

    #[tokio::test]
    async fn duplicate_code_maps_to_a_field_error() {
        let (svc, customer_id) = service_with_customer().await;
        let mut input = valid_input(customer_id);
        input.code = "DUP".to_string();
        svc.create(input.clone()).await.expect("first");
        let err = svc.create(input).await.expect_err("second");
        assert_eq!(field_errors(&err)[0].0, "code");
    }

    #[tokio::test]
    async fn unknown_customer_is_a_field_error_not_a_foreign_key_error() {
        let (svc, _) = service_with_customer().await;
        let err = svc
            .create(valid_input(9999))
            .await
            .expect_err("unknown customer");
        assert_eq!(field_errors(&err)[0].0, "customerId");
    }

    #[tokio::test]
    async fn invalid_status_is_rejected() {
        let (svc, customer_id) = service_with_customer().await;
        let mut input = valid_input(customer_id);
        input.status = "SOMETHING_ELSE".to_string();
        let err = svc.create(input).await.expect_err("bad status");
        assert_eq!(field_errors(&err)[0].0, "status");
    }

    #[tokio::test]
    async fn all_seven_statuses_are_accepted() {
        let (svc, customer_id) = service_with_customer().await;
        for status in ProjectStatus::ALL {
            let mut input = valid_input(customer_id);
            input.status = status.as_code().to_string();
            svc.create(input).await.expect("status should be accepted");
        }
    }

    /// 採算集計の対象は失注を除く全状態（Phase 1 決定 C-12）。
    #[tokio::test]
    async fn only_lost_is_excluded_from_profitability() {
        for status in ProjectStatus::ALL {
            assert_eq!(
                status.counts_toward_profitability(),
                status != ProjectStatus::Lost,
                "{:?}",
                status
            );
        }
        assert!(ProjectStatus::Ordered.is_active());
        assert!(ProjectStatus::InProgress.is_active());
        assert!(ProjectStatus::AwaitingAcceptance.is_active());
        assert!(!ProjectStatus::Completed.is_active());
        assert!(!ProjectStatus::Prospect.is_active());
    }

    #[tokio::test]
    async fn dates_must_be_iso_and_ordered() {
        let (svc, customer_id) = service_with_customer().await;

        let mut bad_format = valid_input(customer_id);
        bad_format.started_on = Some("2026/08/20".to_string());
        let err = svc.create(bad_format).await.expect_err("bad format");
        assert_eq!(field_errors(&err)[0].0, "startedOn");

        let mut reversed = valid_input(customer_id);
        reversed.started_on = Some("2026-09-01".to_string());
        reversed.due_on = Some("2026-08-31".to_string());
        let err = svc.create(reversed).await.expect_err("reversed range");
        assert_eq!(field_errors(&err)[0].0, "dueOn");

        let mut same_day = valid_input(customer_id);
        same_day.started_on = Some("2026-08-31".to_string());
        same_day.due_on = Some("2026-08-31".to_string());
        svc.create(same_day).await.expect("same day is allowed");
    }

    #[tokio::test]
    async fn negative_amount_is_rejected() {
        let (svc, customer_id) = service_with_customer().await;
        let mut input = valid_input(customer_id);
        input.contract_amount = Some(-1);
        let err = svc.create(input).await.expect_err("negative amount");
        assert_eq!(field_errors(&err)[0].0, "contractAmount");
    }

    #[tokio::test]
    async fn update_requires_an_explicit_code() {
        let (svc, customer_id) = service_with_customer().await;
        let created = svc.create(valid_input(customer_id)).await.expect("create");
        // 更新で code を空にするのは「消してよい」意思表示ではないので弾く
        // （作成時の自動採番と違い、既存の番号を黙って振り直さない）。
        let err = svc
            .update(created.id, valid_input(customer_id))
            .await
            .expect_err("blank code on update");
        assert_eq!(field_errors(&err)[0].0, "code");
    }

    #[tokio::test]
    async fn update_and_delete_round_trip() {
        let (svc, customer_id) = service_with_customer().await;
        let created = svc.create(valid_input(customer_id)).await.expect("create");
        let mut input = valid_input(customer_id);
        input.code = created.code.clone();
        input.status = "COMPLETED".to_string();
        let updated = svc.update(created.id, input).await.expect("update");
        assert_eq!(updated.status, "COMPLETED");

        svc.delete(created.id).await.expect("delete");
        assert!(matches!(
            svc.get(created.id).await.expect_err("gone"),
            BantoError::NotFound { .. }
        ));
    }
}
