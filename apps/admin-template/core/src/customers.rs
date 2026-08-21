//! Customers resource service (Phase 2 基本マスター、`docs/domain/schema.md`
//! §2.1): 顧客マスタのドメインロジック。conventions §2 に従い `tauri` /
//! `axum` / RBAC / HTTP を知らない — 認可・監査・イベント通知は呼び出し側
//! （REST / Tauri の wiring 層）が付ける。
//!
//! [`CustomerInput`] のフィールド名はフロントのフォームスキーマ
//! （`apps/admin-template/src/lib/banto/resources/customers.ts`）と1対1に
//! 対応させ、`BantoError::Validation` のフィールドエラーがそのまま該当の
//! 入力欄へ戻るようにする（`items.rs` と同じ流儀）。
//!
//! 締日・支払条件をコード値で持つ理由と 99（月末）の意味は Phase 1 の決定
//! C-8（`docs/domain/open-questions.md`）にある。

use banto_core::{BantoError, FieldError, ListParams, ListResult};
use banto_server::ServerEvent;
use banto_storage::{ColumnMap, Db, Dialect};
use serde::{Deserialize, Serialize};
use sqlx::{QueryBuilder, Sqlite};
use tokio::sync::broadcast;

/// `customers` の1行。ワイヤ形状（camelCase）はフロントのリソース定義に
/// 合わせる。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Customer {
    pub id: i64,
    pub code: String,
    pub name: String,
    #[sqlx(rename = "contact_person")]
    pub contact_person: Option<String>,
    pub address: Option<String>,
    pub phone: Option<String>,
    pub email: Option<String>,
    #[sqlx(rename = "billing_name")]
    pub billing_name: Option<String>,
    /// 締日。1..=28 または 99（月末）。
    #[sqlx(rename = "closing_day")]
    pub closing_day: i64,
    /// 締日から何ヶ月後に支払われるか（0 = 当月、1 = 翌月）。
    #[sqlx(rename = "payment_month_offset")]
    pub payment_month_offset: i64,
    /// 支払日。1..=28 または 99（月末）。
    #[sqlx(rename = "payment_day")]
    pub payment_day: i64,
    pub note: Option<String>,
    #[sqlx(rename = "created_at")]
    pub created_at: String,
    #[sqlx(rename = "updated_at")]
    pub updated_at: String,
}

/// 作成・更新のペイロード。フィールド名はフロントのフォームスキーマと一致
/// させる（`items.rs::ItemInput` と同じ理由）。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomerInput {
    pub code: String,
    pub name: String,
    pub contact_person: Option<String>,
    pub address: Option<String>,
    pub phone: Option<String>,
    pub email: Option<String>,
    pub billing_name: Option<String>,
    pub closing_day: i64,
    pub payment_month_offset: i64,
    pub payment_day: i64,
    pub note: Option<String>,
}

const MAX_CODE_LEN: usize = 20;
/// 自動採番の接頭辞と、その走査用の LIKE パターン。
const CODE_PREFIX: &str = "C";
const CODE_PREFIX_PATTERN: &str = "C%";
const MAX_NAME_LEN: usize = 60;
const MAX_TEXT_LEN: usize = 120;
const MAX_NOTE_LEN: usize = 500;

/// 月末を表す番兵値。1..=28 と 99 のみを許し、29〜31 を許さないのは
/// 「2月に存在しない日」を業務日付の元データとして保持しないため
/// （Phase 1 決定 C-8）。
pub const DAY_END_OF_MONTH: i64 = 99;
const MAX_DAY_OF_MONTH: i64 = 28;
/// 支払条件の月オフセットの上限（0 = 当月払い、6 = 半年後）。実務上これ以上
/// の条件は無く、入力ミス（12 と 120 の取り違え等）を弾く上限として置く。
const MAX_PAYMENT_MONTH_OFFSET: i64 = 6;

fn required_message() -> String {
    "必須項目です".to_string()
}

fn max_length_message(max: usize) -> String {
    format!("{max}文字以内で入力してください")
}

fn day_message() -> String {
    format!("1〜{MAX_DAY_OF_MONTH}の日、または月末（{DAY_END_OF_MONTH}）で入力してください")
}

/// 必須のテキスト項目を検証して trim 済みの値を返す。
fn check_required_text(
    errors: &mut Vec<FieldError>,
    field: &str,
    value: &str,
    max_len: usize,
) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        errors.push(FieldError {
            field: field.to_string(),
            message: required_message(),
        });
    } else if trimmed.chars().count() > max_len {
        errors.push(FieldError {
            field: field.to_string(),
            message: max_length_message(max_len),
        });
    }
    trimmed.to_string()
}

/// 任意のテキスト項目を検証する。空文字は `None` に正規化して、DB 上で
/// 「空文字」と「未入力」が混在しないようにする。
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

/// 締日・支払日の妥当性（1..=28 または 99）。
fn check_day(errors: &mut Vec<FieldError>, field: &str, value: i64) {
    if !((1..=MAX_DAY_OF_MONTH).contains(&value) || value == DAY_END_OF_MONTH) {
        errors.push(FieldError {
            field: field.to_string(),
            message: day_message(),
        });
    }
}

/// 検証済みの入力値。SQL へ渡す前に trim / 空文字→NULL の正規化を済ませた形。
struct NormalizedCustomer {
    code: String,
    name: String,
    contact_person: Option<String>,
    address: Option<String>,
    phone: Option<String>,
    email: Option<String>,
    billing_name: Option<String>,
    closing_day: i64,
    payment_month_offset: i64,
    payment_day: i64,
    note: Option<String>,
}

/// [`CustomerInput`] をフロントのスキーマと同じ規則で検証する。最初の1件で
/// 止めず全違反を返すのは `items.rs::validate_item_input` と同じ
/// （`@banto/forms` の `validateAll` に合わせる）。
fn validate(input: &CustomerInput) -> Result<NormalizedCustomer, BantoError> {
    let mut errors: Vec<FieldError> = Vec::new();

    // **空欄を許す。** 空なら `create` が採番する（`next_code`）。個人事業では
    // 顧客コードを自分で決める意味が薄く、`C001` を毎回考えるのは手間でしかない。
    // 案件番号が既に同じ扱い（要件 F-M3）なので、そちらに揃えた。
    //
    // 会計ソフト側の得意先コードに合わせたい場合のために、**入力もできる**
    // ままにしてある（必須にしないだけで、廃止はしない）。
    let code = input.code.trim().to_string();
    if code.chars().count() > MAX_CODE_LEN {
        errors.push(FieldError {
            field: "code".to_string(),
            message: max_length_message(MAX_CODE_LEN),
        });
    }
    let name = check_required_text(&mut errors, "name", &input.name, MAX_NAME_LEN);
    let contact_person = check_optional_text(
        &mut errors,
        "contactPerson",
        &input.contact_person,
        MAX_TEXT_LEN,
    );
    let address = check_optional_text(&mut errors, "address", &input.address, MAX_TEXT_LEN);
    let phone = check_optional_text(&mut errors, "phone", &input.phone, MAX_TEXT_LEN);
    let email = check_optional_text(&mut errors, "email", &input.email, MAX_TEXT_LEN);
    let billing_name = check_optional_text(
        &mut errors,
        "billingName",
        &input.billing_name,
        MAX_NAME_LEN,
    );
    let note = check_optional_text(&mut errors, "note", &input.note, MAX_NOTE_LEN);

    check_day(&mut errors, "closingDay", input.closing_day);
    check_day(&mut errors, "paymentDay", input.payment_day);
    if !(0..=MAX_PAYMENT_MONTH_OFFSET).contains(&input.payment_month_offset) {
        errors.push(FieldError {
            field: "paymentMonthOffset".to_string(),
            message: format!("0〜{MAX_PAYMENT_MONTH_OFFSET}で入力してください"),
        });
    }

    if errors.is_empty() {
        Ok(NormalizedCustomer {
            code,
            name,
            contact_person,
            address,
            phone,
            email,
            billing_name,
            closing_day: input.closing_day,
            payment_month_offset: input.payment_month_offset,
            payment_day: input.payment_day,
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
        .column("name", "name")
        .column("contactPerson", "contact_person")
        .column("address", "address")
        .column("phone", "phone")
        .column("email", "email")
        .column("billingName", "billing_name")
        .column("closingDay", "closing_day")
        .column("paymentMonthOffset", "payment_month_offset")
        .column("paymentDay", "payment_day")
        .column("note", "note")
        .column("createdAt", "created_at")
        .column("updatedAt", "updated_at")
}

const RESOURCE: &str = "customers";
const COLUMNS: &str = "id, code, name, contact_person, address, phone, email, billing_name, \
     closing_day, payment_month_offset, payment_day, note, created_at, updated_at";

/// 「今日」の日付式。`items.rs::today_expr` と同じ理由でここに置く
/// （`Dialect::now_expr()` は datetime を返すが、業務日付は `YYYY-MM-DD` の
/// 日付のみで保持する規約 — CLAUDE.md 第4章）。
fn today_expr(dialect: Dialect) -> &'static str {
    match dialect {
        Dialect::Sqlite => "date('now')",
        Dialect::Postgres => "CURRENT_DATE::text",
    }
}

/// UNIQUE 制約違反（顧客コード重複）を、フォームの `code` 欄に戻る
/// `Validation` エラーへ変換する。素の DB エラーを見せると利用者には何が
/// 起きたか分からず、`items` の流儀（フィールドエラーは該当入力欄へ戻す）
/// とも揃わないため。
fn map_unique_violation(err: sqlx::Error, code: &str) -> BantoError {
    let is_unique_violation = err
        .as_database_error()
        .map(|db_err| {
            // SQLite は "UNIQUE constraint failed"、Postgres は SQLSTATE 23505。
            db_err.code().as_deref() == Some("23505")
                || db_err.message().contains("UNIQUE constraint failed")
        })
        .unwrap_or(false);
    if is_unique_violation {
        BantoError::Validation {
            field_errors: vec![FieldError {
                field: "code".to_string(),
                message: format!("顧客コード「{code}」は既に使われています"),
            }],
        }
    } else {
        banto_storage::storage_error(err)
    }
}

/// 顧客マスタのサービス層（conventions §2）。`Clone` が安いのは `Db` と
/// `broadcast::Sender` がいずれも Arc-backed なため（`ItemsService` と同型）。
#[derive(Clone)]
pub struct CustomersService {
    db: Db,
    events: Option<broadcast::Sender<ServerEvent>>,
}

impl CustomersService {
    pub fn new(db: Db) -> Self {
        Self { db, events: None }
    }

    /// イベント送信器を付ける。`create`/`update`/`delete` が成功後に
    /// `ServerEvent::ResourceChanged` を送る（`ItemsService` と同じ）。
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

    pub async fn list(&self, params: ListParams) -> Result<ListResult<Customer>, BantoError> {
        let columns = column_map();
        let select_rows = format!("SELECT {COLUMNS} FROM {}", crate::sync::live("customers"));
        let select_count = format!("SELECT COUNT(*) FROM {}", crate::sync::live("customers"));

        match &self.db {
            Db::Sqlite(pool) => {
                let mut rows_builder: QueryBuilder<'_, Sqlite> = QueryBuilder::new(&select_rows);
                banto_storage::list_query::sqlite::apply_list_params(
                    &mut rows_builder,
                    &columns,
                    &params,
                )?;
                let rows: Vec<Customer> = rows_builder
                    .build_query_as::<Customer>()
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
                let rows: Vec<Customer> = rows_builder
                    .build_query_as::<Customer>()
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

    pub async fn get(&self, id: i64) -> Result<Customer, BantoError> {
        let dialect = self.db.dialect();
        let sql = format!(
            "SELECT {COLUMNS} FROM customers WHERE id = {} AND deleted_at IS NULL",
            dialect.placeholder(1)
        );
        match &self.db {
            Db::Sqlite(pool) => {
                sqlx::query_as::<_, Customer>(&sql)
                    .bind(id)
                    .fetch_one(pool)
                    .await
            }
            #[cfg(feature = "postgres")]
            Db::Postgres(pool) => {
                sqlx::query_as::<_, Customer>(&sql)
                    .bind(id)
                    .fetch_one(pool)
                    .await
            }
        }
        .map_err(|err| banto_storage::not_found(err, RESOURCE, id.to_string()))
    }

    /// 次の顧客コード `C001` を返す。既存の `C` + 数字のうち最大の連番 + 1。
    ///
    /// 欠番は詰めない（詰めると、削除した顧客の番号を別の顧客が引き継ぐ）。
    /// 墓石も走査対象に含める —— 論理削除した行もコードを保持し続けるので
    /// （`docs/domain/sync.md` 5.1）、詰めると UNIQUE 制約に当たる。
    pub async fn next_code(&self) -> Result<String, BantoError> {
        let dialect = self.db.dialect();
        let sql = format!(
            "SELECT code FROM customers WHERE code LIKE {} ORDER BY code DESC LIMIT 1",
            dialect.placeholder(1)
        );
        let latest: Option<String> = match &self.db {
            Db::Sqlite(pool) => {
                sqlx::query_scalar(&sql)
                    .bind(CODE_PREFIX_PATTERN)
                    .fetch_optional(pool)
                    .await
            }
            #[cfg(feature = "postgres")]
            Db::Postgres(pool) => {
                sqlx::query_scalar(&sql)
                    .bind(CODE_PREFIX_PATTERN)
                    .fetch_optional(pool)
                    .await
            }
        }
        .map_err(banto_storage::storage_error)?;

        let next = latest
            .as_deref()
            .and_then(|code| code.strip_prefix(CODE_PREFIX))
            .and_then(|seq| seq.parse::<u32>().ok())
            .unwrap_or(0)
            + 1;
        Ok(format!("{CODE_PREFIX}{next:03}"))
    }

    pub async fn create(&self, input: CustomerInput) -> Result<Customer, BantoError> {
        let mut value = validate(&input)?;
        // 空欄なら採番する（案件番号と同じ扱い、要件 F-M3）。
        if value.code.is_empty() {
            value.code = self.next_code().await?;
        }
        let dialect = self.db.dialect();
        let today = today_expr(dialect);
        let sql = format!(
            "INSERT INTO customers (code, name, contact_person, address, phone, email, \
             billing_name, closing_day, payment_month_offset, payment_day, note, created_at, \
             updated_at) VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {today}, {today}) \
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
        let customer = match &self.db {
            Db::Sqlite(pool) => {
                bind_input(sqlx::query_as::<_, Customer>(&sql), &value)
                    .fetch_one(pool)
                    .await
            }
            #[cfg(feature = "postgres")]
            Db::Postgres(pool) => {
                bind_input(sqlx::query_as::<_, Customer>(&sql), &value)
                    .fetch_one(pool)
                    .await
            }
        }
        .map_err(|err| map_unique_violation(err, &value.code))?;
        self.notify_changed();
        Ok(customer)
    }

    pub async fn update(&self, id: i64, input: CustomerInput) -> Result<Customer, BantoError> {
        let value = validate(&input)?;
        let dialect = self.db.dialect();
        let sql = format!(
            "UPDATE customers SET code = {}, name = {}, contact_person = {}, address = {}, \
             phone = {}, email = {}, billing_name = {}, closing_day = {}, \
             payment_month_offset = {}, payment_day = {}, note = {}, updated_at = {} \
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
        let customer = match &self.db {
            Db::Sqlite(pool) => {
                bind_input(sqlx::query_as::<_, Customer>(&sql), &value)
                    .bind(id)
                    .fetch_one(pool)
                    .await
            }
            #[cfg(feature = "postgres")]
            Db::Postgres(pool) => {
                bind_input(sqlx::query_as::<_, Customer>(&sql), &value)
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
        Ok(customer)
    }

    /// 顧客を削除する（**論理削除**。`docs/domain/sync.md` 5節）。
    ///
    /// 案件が1件でも紐づいている場合は削除せず `Validation` エラーを返す —
    /// 外部キーに任せて素の DB エラーを見せるより、「どの案件が残っているか」を
    /// 利用者に伝えられるため（SQLite は既定で外部キー制約を強制しない構成も
    /// あり、方言差に依存しない判定にする狙いもある）。
    ///
    /// **数えるのは生きている案件だけ。** 墓石を数えると、実際には何も
    /// 残っていないのに「案件が2件あるため削除できません」と出て、
    /// **永久に消せない顧客**ができる。
    ///
    /// 案件に加えて請求書・入金も数える。物理削除の頃はこの2つを見ていなくても
    /// 外部キーが弾いていたが、論理削除では行が消えないので外部キーは何も
    /// 言わない。放置すると、確定済みの請求書がぶら下がったまま顧客だけ
    /// 見えなくなる。
    pub async fn delete(&self, id: i64) -> Result<(), BantoError> {
        let dialect = self.db.dialect();
        // (表示名, 表, 論理削除の対象か)。請求・入金は同期しないので
        // `deleted_at` を持たない（`docs/domain/sync.md` 1節）。
        let checks = [
            ("案件", "projects", true),
            ("請求書", "invoices", false),
            ("入金", "payments", false),
        ];
        let mut blockers: Vec<String> = Vec::new();
        for (label, table, soft_deletable) in checks {
            let alive = if soft_deletable {
                " AND deleted_at IS NULL"
            } else {
                ""
            };
            let count_sql = format!(
                "SELECT COUNT(*) FROM {table} WHERE customer_id = {}{alive}",
                dialect.placeholder(1)
            );
            let count: i64 = match &self.db {
                Db::Sqlite(pool) => {
                    sqlx::query_scalar(&count_sql)
                        .bind(id)
                        .fetch_one(pool)
                        .await
                }
                #[cfg(feature = "postgres")]
                Db::Postgres(pool) => {
                    sqlx::query_scalar(&count_sql)
                        .bind(id)
                        .fetch_one(pool)
                        .await
                }
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
                        "この顧客には{}が紐づいているため削除できません。先に削除するか別の顧客へ付け替えてください",
                        blockers.join("・")
                    ),
                }],
            });
        }

        let today = today_expr(dialect);
        let sql = format!(
            "UPDATE customers SET deleted_at = {}, updated_at = {today} WHERE id = {} \
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

/// INSERT / UPDATE の bind 順を1箇所に閉じる。SQLite / Postgres の両アームで
/// 同じ順序を2回書くと、片方だけ列を足したときに静かにずれるため。
fn bind_input<'q, DB>(
    query: sqlx::query::QueryAs<'q, DB, Customer, <DB as sqlx::Database>::Arguments<'q>>,
    value: &'q NormalizedCustomer,
) -> sqlx::query::QueryAs<'q, DB, Customer, <DB as sqlx::Database>::Arguments<'q>>
where
    DB: sqlx::Database,
    String: sqlx::Type<DB> + sqlx::Encode<'q, DB>,
    &'q str: sqlx::Type<DB> + sqlx::Encode<'q, DB>,
    i64: sqlx::Type<DB> + sqlx::Encode<'q, DB>,
    Option<&'q str>: sqlx::Type<DB> + sqlx::Encode<'q, DB>,
{
    query
        .bind(value.code.as_str())
        .bind(value.name.as_str())
        .bind(value.contact_person.as_deref())
        .bind(value.address.as_deref())
        .bind(value.phone.as_deref())
        .bind(value.email.as_deref())
        .bind(value.billing_name.as_deref())
        .bind(value.closing_day)
        .bind(value.payment_month_offset)
        .bind(value.payment_day)
        .bind(value.note.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrate_memory;
    use crate::projects::{ProjectInput, ProjectsService};

    /// マイグレーション済み・シード無しの DB（`items.rs` のテストと同じ流儀）。
    async fn service() -> CustomersService {
        let pool = migrate_memory().await.expect("migrate_memory");
        CustomersService::new(pool)
    }

    /// 顧客コードは空欄で保存できる（案件番号と同じ扱い、要件 F-M3）。
    /// 個人事業では `C001` を毎回考えるのが手間でしかない。
    #[tokio::test]
    async fn a_blank_code_is_auto_numbered() {
        let svc = service().await;
        let first = svc.create(valid_input("")).await.expect("first");
        let second = svc.create(valid_input("")).await.expect("second");

        assert_eq!(first.code, "C001");
        assert_eq!(second.code, "C002");
    }

    /// 会計ソフト側の得意先コードに合わせたい場合のために、入力もできる。
    #[tokio::test]
    async fn an_explicit_code_is_kept_as_given() {
        let svc = service().await;
        let created = svc
            .create(valid_input("TOKUISAKI-9"))
            .await
            .expect("create");
        assert_eq!(created.code, "TOKUISAKI-9");
    }

    /// 自分で付けたコードが混ざっていても、採番は `C` 付きの最大値を見る。
    #[tokio::test]
    async fn auto_numbering_ignores_codes_that_are_not_its_own() {
        let svc = service().await;
        svc.create(valid_input("ZZZ-999")).await.expect("manual");
        let auto = svc.create(valid_input("")).await.expect("auto");
        assert_eq!(auto.code, "C001");
    }

    /// **墓石のコードも走査対象。** 論理削除した行はコードを保持し続けるので
    /// （`docs/domain/sync.md` 5.1）、詰めると UNIQUE 制約に当たる。
    #[tokio::test]
    async fn auto_numbering_does_not_reuse_a_deleted_code() {
        let svc = service().await;
        let first = svc.create(valid_input("")).await.expect("first");
        assert_eq!(first.code, "C001");
        svc.delete(first.id).await.expect("delete");

        let next = svc.create(valid_input("")).await.expect("next");
        assert_eq!(next.code, "C002", "消した番号を使い回さないこと");
    }

    fn valid_input(code: &str) -> CustomerInput {
        CustomerInput {
            code: code.to_string(),
            name: "架空商事".to_string(),
            contact_person: Some("担当 太郎".to_string()),
            address: None,
            phone: None,
            email: None,
            billing_name: None,
            // 月末締め・翌月末払い
            closing_day: DAY_END_OF_MONTH,
            payment_month_offset: 1,
            payment_day: DAY_END_OF_MONTH,
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
    async fn deleting_a_customer_is_a_soft_delete() {
        let svc = service().await;
        svc.create(valid_input("C001")).await.expect("残す");
        let doomed = svc.create(valid_input("C002")).await.expect("消す").id;

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

    /// **請求書がぶら下がっている顧客は消せない。**
    ///
    /// 物理削除の頃は外部キーが弾いていたが、論理削除では行が消えないので
    /// 外部キーは何も言わない。放置すると、確定済みの請求書がぶら下がったまま
    /// 顧客だけ見えなくなる。請求・入金は同期しないので墓石を持たない。
    #[tokio::test]
    async fn a_customer_with_invoices_cannot_be_deleted() {
        let db = migrate_memory().await.expect("migrate_memory");
        let customers = CustomersService::new(db.clone());
        let customer = customers
            .create(valid_input("C001"))
            .await
            .expect("customer");
        let projects = ProjectsService::new(db.clone());
        let project = projects
            .create(ProjectInput {
                code: "P001".to_string(),
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

        let invoices = crate::invoices::InvoicesService::new(db.clone());
        invoices
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
                    unit_price: 100_000,
                    tax_category: "STANDARD_10".to_string(),
                    source_type: None,
                    source_id: None,
                    note: None,
                }],
            })
            .await
            .expect("invoice");

        // 案件のほうは請求明細に参照されているので、そもそも消せない。
        let project_err = projects
            .delete(project.id)
            .await
            .expect_err("請求明細が参照している案件を消せている");
        assert!(
            field_errors(&project_err)[0].1.contains("請求明細1件"),
            "{:?}",
            field_errors(&project_err)
        );

        // 顧客は案件と請求書の両方が理由として挙がる。
        let err = customers
            .delete(customer.id)
            .await
            .expect_err("請求書が残っているのに顧客を消せている");
        let (_, message) = field_errors(&err).into_iter().next().expect("field error");
        assert!(message.contains("案件1件"), "{message}");
        assert!(message.contains("請求書1件"), "{message}");
    }

    /// **墓石の案件が顧客の削除を永久にブロックしないこと。**
    ///
    /// 削除ガードが墓石まで数えると、実際には何も残っていないのに
    /// 「案件が1件あるため削除できません」と出て、**二度と消せない顧客**が
    /// できる。案件を論理削除にした時点でこの経路が踏めるようになった。
    #[tokio::test]
    async fn a_tombstoned_project_does_not_block_deleting_its_customer() {
        let db = migrate_memory().await.expect("migrate_memory");
        let customers = CustomersService::new(db.clone());
        let customer = customers
            .create(valid_input("C001"))
            .await
            .expect("customer");

        let projects = ProjectsService::new(db.clone());
        let project = projects
            .create(ProjectInput {
                code: "P001".to_string(),
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

        // 案件が生きているうちは拒否される（既存の挙動）。
        assert!(
            customers.delete(customer.id).await.is_err(),
            "案件が残っているのに顧客を消せている"
        );

        // 案件を論理削除すると、顧客も消せるようになる。
        projects.delete(project.id).await.expect("delete 案件");
        customers
            .delete(customer.id)
            .await
            .expect("墓石の案件が顧客の削除をブロックしている");
    }

    #[tokio::test]
    async fn create_then_get_round_trips() {
        let svc = service().await;
        let created = svc.create(valid_input("C001")).await.expect("create");
        assert_eq!(created.code, "C001");
        assert_eq!(created.closing_day, DAY_END_OF_MONTH);
        assert_eq!(created.payment_month_offset, 1);
        assert!(!created.created_at.is_empty());

        let fetched = svc.get(created.id).await.expect("get");
        assert_eq!(fetched, created);
    }

    #[tokio::test]
    async fn optional_blank_text_is_stored_as_null() {
        let svc = service().await;
        let mut input = valid_input("C002");
        // 空白のみの入力は「未入力」として扱い、空文字を DB に残さない。
        input.address = Some("   ".to_string());
        let created = svc.create(input).await.expect("create");
        assert_eq!(created.address, None);
    }

    #[tokio::test]
    async fn validation_reports_every_violation_at_once() {
        let svc = service().await;
        let err = svc
            .create(CustomerInput {
                // 空欄は**もう違反ではない**（採番される）。全件返すことを
                // 確かめるテストなので、長さ違反に置き換えて code を残す。
                code: "X".repeat(MAX_CODE_LEN + 1),
                name: String::new(),
                contact_person: None,
                address: None,
                phone: None,
                email: None,
                billing_name: None,
                closing_day: 31,
                payment_month_offset: 99,
                payment_day: 0,
                note: None,
            })
            .await
            .expect_err("should fail validation");
        let fields: Vec<String> = field_errors(&err).into_iter().map(|(f, _)| f).collect();
        // 最初の1件で止めず全件返す（@banto/forms の validateAll に合わせる）。
        assert!(fields.contains(&"code".to_string()));
        assert!(fields.contains(&"name".to_string()));
        assert!(fields.contains(&"closingDay".to_string()));
        assert!(fields.contains(&"paymentDay".to_string()));
        assert!(fields.contains(&"paymentMonthOffset".to_string()));
    }

    /// 29〜31 を弾くのは「2月に存在しない日」を締日として持たないため
    /// （Phase 1 決定 C-8）。月末は 99 で表す。
    #[tokio::test]
    async fn closing_day_accepts_1_to_28_and_end_of_month_only() {
        let svc = service().await;
        for (i, day) in [1_i64, 28, DAY_END_OF_MONTH].into_iter().enumerate() {
            let mut input = valid_input(&format!("OK{i}"));
            input.closing_day = day;
            svc.create(input).await.expect("valid closing day");
        }
        for (i, day) in [0_i64, 29, 31, 100].into_iter().enumerate() {
            let mut input = valid_input(&format!("NG{i}"));
            input.closing_day = day;
            let err = svc.create(input).await.expect_err("invalid closing day");
            assert_eq!(field_errors(&err)[0].0, "closingDay");
        }
    }

    #[tokio::test]
    async fn duplicate_code_maps_to_a_field_error_not_a_raw_db_error() {
        let svc = service().await;
        svc.create(valid_input("DUP")).await.expect("first create");
        let err = svc
            .create(valid_input("DUP"))
            .await
            .expect_err("second create should fail");
        let errors = field_errors(&err);
        assert_eq!(errors[0].0, "code");
        assert!(
            errors[0].1.contains("DUP"),
            "message names the code: {:?}",
            errors[0].1
        );
    }

    #[tokio::test]
    async fn update_changes_fields_and_get_reflects_it() {
        let svc = service().await;
        let created = svc.create(valid_input("C010")).await.expect("create");
        let mut input = valid_input("C010");
        input.name = "架空商事（改称後）".to_string();
        input.payment_month_offset = 2;
        let updated = svc.update(created.id, input).await.expect("update");
        assert_eq!(updated.name, "架空商事（改称後）");
        assert_eq!(updated.payment_month_offset, 2);
        assert_eq!(updated.id, created.id);
    }

    #[tokio::test]
    async fn update_missing_id_is_not_found() {
        let svc = service().await;
        let err = svc
            .update(999, valid_input("C999"))
            .await
            .expect_err("missing id");
        assert!(matches!(err, BantoError::NotFound { .. }), "got {err:?}");
    }

    #[tokio::test]
    async fn delete_removes_the_row() {
        let svc = service().await;
        let created = svc.create(valid_input("C020")).await.expect("create");
        svc.delete(created.id).await.expect("delete");
        let err = svc.get(created.id).await.expect_err("gone");
        assert!(matches!(err, BantoError::NotFound { .. }), "got {err:?}");
    }

    #[tokio::test]
    async fn delete_missing_id_is_not_found() {
        let svc = service().await;
        let err = svc.delete(4242).await.expect_err("missing id");
        assert!(matches!(err, BantoError::NotFound { .. }), "got {err:?}");
    }

    /// 案件が紐づく顧客は削除できない。外部キー任せの生 DB エラーではなく、
    /// 残件数を伝える `Validation` を返す。
    #[tokio::test]
    async fn delete_is_refused_while_projects_reference_the_customer() {
        let pool = migrate_memory().await.expect("migrate_memory");
        let customers = CustomersService::new(pool.clone());
        let projects = ProjectsService::new(pool);
        let customer = customers.create(valid_input("C030")).await.expect("create");
        projects
            .create(ProjectInput {
                code: String::new(),
                customer_id: customer.id,
                name: "架空案件".to_string(),
                status: "ORDERED".to_string(),
                started_on: None,
                due_on: None,
                estimate_amount: None,
                contract_amount: None,
                billing_hourly_rate: None,
                scope: None,
                note: None,
            })
            .await
            .expect("project create");

        let err = customers
            .delete(customer.id)
            .await
            .expect_err("delete should be refused");
        let errors = field_errors(&err);
        assert!(
            errors[0].1.contains("1件"),
            "message states the count: {:?}",
            errors[0].1
        );
        // 顧客は残っている。
        customers.get(customer.id).await.expect("still there");
    }
}
