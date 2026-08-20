//! Phase 6: 入金と消込（`docs/domain/schema.md` §5）。conventions §2 に従い
//! `tauri` / `axum` / RBAC を知らない。
//!
//! ## 構造（`CLAUDE.md` 1.4）
//!
//! `Payment` は `customer_id` を持ち、請求書との対応は `PaymentAllocation` が
//! 持つ N:M。まとめ入金（1入金 → 複数請求書）と分割入金（1請求書 ← 複数入金）の
//! 両方が起きるため、`Payment.invoice_id` は作らない。
//!
//! ## 保持しない（要件 F-Y5 / F-Y6、`CLAUDE.md` 1.5）
//!
//! 入金状態（一部入金 / 入金済）も期限超過も**列として持たない**。
//!
//! ```text
//! 消込額   := Σ(充当額 + 差額)
//! 残額     := max(0, 請求額 − 消込額)          ← 0 未満にしない（F-Y4）
//! Overdue  := 支払期限 < 今日 AND 残額 > 0 AND 状態 ≠ CANCELLED
//! ```
//!
//! 状態として持つと日次バッチが必要になり、アプリを毎日起動しない構成では
//! 実態とずれる。
//!
//! ## 差額（決定 C-19）
//!
//! `difference_amount` は「入金額には含まれないが請求書を閉じる額」。振込手数料は
//! **原則として先方負担**なので差し引かれるのは例外だが、差し引かれたとき・
//! 値引きしたとき・過入金のときに理由コード付きで記録し、請求書を閉じられる
//! ようにする。差額を消込に効かせないと、手数料ぶんの端数が請求書に永久に
//! 残ってしまう。

use crate::dates::is_valid_date;
use crate::invoices::{STATUS_CANCELLED, STATUS_ISSUED};
use banto_core::{BantoError, FieldError, ListParams, ListResult};
use banto_server::ServerEvent;
use banto_storage::{ColumnMap, Db, Dialect};
use serde::{Deserialize, Serialize};
use sqlx::{QueryBuilder, Sqlite};
use tokio::sync::broadcast;

/// 差額理由コード（要件 F-Y3 / `schema.md` §5.2）。
pub const DIFFERENCE_REASONS: [&str; 5] = [
    "TRANSFER_FEE",
    "WITHHOLDING",
    "DISCOUNT",
    "OVERPAYMENT",
    "OTHER",
];
const REASON_OTHER: &str = "OTHER";

const MAX_AMOUNT: i64 = 9_999_999_999;
const MAX_TEXT_LEN: usize = 120;
const MAX_NOTE_LEN: usize = 500;
const MAX_ALLOCATIONS: usize = 100;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Payment {
    pub id: i64,
    #[sqlx(rename = "customer_id")]
    pub customer_id: i64,
    #[sqlx(rename = "paid_on")]
    pub paid_on: String,
    /// 入金額（円）。実際に着金した額。
    pub amount: i64,
    pub method: Option<String>,
    pub note: Option<String>,
    #[sqlx(rename = "created_at")]
    pub created_at: String,
    #[sqlx(rename = "updated_at")]
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct PaymentAllocation {
    pub id: i64,
    #[sqlx(rename = "payment_id")]
    pub payment_id: i64,
    #[sqlx(rename = "invoice_id")]
    pub invoice_id: i64,
    #[sqlx(rename = "allocated_amount")]
    pub allocated_amount: i64,
    #[sqlx(rename = "difference_reason")]
    pub difference_reason: Option<String>,
    /// 入金額に含まれないが請求書を閉じる額（決定 C-19）。
    #[sqlx(rename = "difference_amount")]
    pub difference_amount: i64,
    pub note: Option<String>,
    #[sqlx(rename = "created_at")]
    pub created_at: String,
}

/// 入金1件と充当先。`getOne` の戻り値。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaymentDetail {
    #[serde(flatten)]
    pub payment: Payment,
    pub customer_name: String,
    pub allocations: Vec<PaymentAllocation>,
    /// 入金額のうちまだどの請求書にも充てていない額。0 でないまま放置すると
    /// 「どこへ入ったか分からない入金」になるので、画面で見えるようにする。
    pub unallocated_amount: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaymentAllocationInput {
    pub invoice_id: i64,
    pub allocated_amount: i64,
    pub difference_reason: Option<String>,
    #[serde(default)]
    pub difference_amount: i64,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaymentInput {
    pub customer_id: i64,
    pub paid_on: String,
    pub amount: i64,
    pub method: Option<String>,
    pub note: Option<String>,
    #[serde(default)]
    pub allocations: Vec<PaymentAllocationInput>,
}

/// 請求書1件の入金状況（要件 F-Y4〜F-Y6）。**すべて導出値**で、列としては
/// 持たない。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InvoiceSettlement {
    pub invoice_id: i64,
    pub invoice_number: Option<String>,
    pub customer_id: i64,
    pub customer_name: String,
    pub status: String,
    pub issued_on: Option<String>,
    pub due_on: Option<String>,
    pub total_amount: i64,
    /// 充当額の合計（実際に入ってきた額のうちこの請求書に充てた分）。
    pub allocated_amount: i64,
    /// 差額の合計（手数料の先方差引・値引き・過入金の超過分）。
    pub difference_amount: i64,
    /// 消込額 = 充当額 + 差額。
    pub settled_amount: i64,
    /// 残額。**0 未満にしない**（F-Y4）。
    pub remaining_amount: i64,
    /// 表示上の入金状態（決定 C-15 の導出）。
    /// `DRAFT` / `CANCELLED` / `PAID` / `PARTIALLY_PAID` / `ISSUED`。
    pub settlement_status: String,
    /// 期限超過か（F-Y6）。判定日は呼び出し時点の業務日付。
    pub overdue: bool,
}

/// 表示上の入金状態（決定 C-15）。`invoices.status` には持たない。
pub fn settlement_status(status: &str, settled: i64, remaining: i64) -> &'static str {
    match status {
        "DRAFT" => "DRAFT",
        s if s == STATUS_CANCELLED => "CANCELLED",
        _ if remaining == 0 && settled > 0 => "PAID",
        _ if settled > 0 => "PARTIALLY_PAID",
        _ => "ISSUED",
    }
}

/// 期限超過の判定（`CLAUDE.md` 1.5 / 要件 F-Y6）。
///
/// **期限当日は超過しない**（`due < today` であって `<=` ではない）。支払期限が
/// 未設定なら判定しない — 期限が無いものを遅れているとは言えない。
pub fn is_overdue(due_on: Option<&str>, remaining: i64, status: &str, today: &str) -> bool {
    let Some(due) = due_on else {
        return false;
    };
    // 業務日付は `YYYY-MM-DD` の固定長なので、文字列比較がそのまま日付比較に
    // なる（ゼロ埋めされているため）。
    due < today && remaining > 0 && status != STATUS_CANCELLED
}

/// 残額（F-Y4）。**0 未満にしない** — 過入金は超過分を差額に記録するだけで、
/// 前受金として繰り越さない（決定 C-11）。
pub fn remaining_amount(total_amount: i64, settled_amount: i64) -> i64 {
    (total_amount - settled_amount).max(0)
}

struct NormalizedAllocation {
    invoice_id: i64,
    allocated_amount: i64,
    difference_reason: Option<String>,
    difference_amount: i64,
    note: Option<String>,
}

struct NormalizedPayment {
    customer_id: i64,
    paid_on: String,
    amount: i64,
    method: Option<String>,
    note: Option<String>,
    allocations: Vec<NormalizedAllocation>,
}

fn optional_text(
    errors: &mut Vec<FieldError>,
    field: &str,
    value: &Option<String>,
    max: usize,
) -> Option<String> {
    let trimmed = value.as_deref().map(str::trim).unwrap_or("");
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.chars().count() > max {
        errors.push(FieldError {
            field: field.to_string(),
            message: format!("{max}文字以内で入力してください"),
        });
        return None;
    }
    Some(trimmed.to_string())
}

fn validate(input: &PaymentInput) -> Result<NormalizedPayment, BantoError> {
    let mut errors: Vec<FieldError> = Vec::new();

    if input.customer_id <= 0 {
        errors.push(FieldError {
            field: "customerId".to_string(),
            message: "顧客を選択してください".to_string(),
        });
    }
    let paid_on = input.paid_on.trim().to_string();
    if !is_valid_date(&paid_on) {
        errors.push(FieldError {
            field: "paidOn".to_string(),
            message: "日付は YYYY-MM-DD で入力してください".to_string(),
        });
    }
    if input.amount <= 0 || input.amount > MAX_AMOUNT {
        errors.push(FieldError {
            field: "amount".to_string(),
            message: "入金額は1円以上で入力してください".to_string(),
        });
    }
    let method = optional_text(&mut errors, "method", &input.method, MAX_TEXT_LEN);
    let note = optional_text(&mut errors, "note", &input.note, MAX_NOTE_LEN);

    if input.allocations.len() > MAX_ALLOCATIONS {
        errors.push(FieldError {
            field: "allocations".to_string(),
            message: format!("充当は{MAX_ALLOCATIONS}件までです"),
        });
    }

    let mut allocations = Vec::with_capacity(input.allocations.len());
    let mut allocated_total = 0i64;
    for (index, allocation) in input.allocations.iter().enumerate() {
        let field = |name: &str| format!("allocations.{index}.{name}");
        if allocation.invoice_id <= 0 {
            errors.push(FieldError {
                field: field("invoiceId"),
                message: "請求書を選択してください".to_string(),
            });
        }
        if allocation.allocated_amount < 0 || allocation.allocated_amount > MAX_AMOUNT {
            errors.push(FieldError {
                field: field("allocatedAmount"),
                message: "充当額は0円以上で入力してください".to_string(),
            });
        }
        if allocation.difference_amount < 0 || allocation.difference_amount > MAX_AMOUNT {
            errors.push(FieldError {
                field: field("differenceAmount"),
                message: "差額は0円以上で入力してください".to_string(),
            });
        }
        if allocation.allocated_amount == 0 && allocation.difference_amount == 0 {
            errors.push(FieldError {
                field: field("allocatedAmount"),
                message: "充当額か差額のどちらかを入力してください".to_string(),
            });
        }
        let reason = allocation
            .difference_reason
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        if let Some(code) = &reason {
            if !DIFFERENCE_REASONS.contains(&code.as_str()) {
                errors.push(FieldError {
                    field: field("differenceReason"),
                    message: "差額理由が不正です".to_string(),
                });
            }
        }
        // 差額があるのに理由が無いと、後から見て「なぜ閉じたのか」が分からない。
        if allocation.difference_amount > 0 && reason.is_none() {
            errors.push(FieldError {
                field: field("differenceReason"),
                message: "差額を入力した場合は理由を選択してください".to_string(),
            });
        }
        let allocation_note =
            optional_text(&mut errors, &field("note"), &allocation.note, MAX_NOTE_LEN);
        // `OTHER` は理由コードだけでは何も伝えないので備考を必須にする
        // （`schema.md` §5.2）。
        if reason.as_deref() == Some(REASON_OTHER) && allocation_note.is_none() {
            errors.push(FieldError {
                field: field("note"),
                message: "その他を選んだ場合は備考を入力してください".to_string(),
            });
        }
        allocated_total += allocation.allocated_amount.max(0);
        allocations.push(NormalizedAllocation {
            invoice_id: allocation.invoice_id,
            allocated_amount: allocation.allocated_amount,
            difference_reason: reason,
            difference_amount: allocation.difference_amount,
            note: allocation_note,
        });
    }

    // 入ってきた額より多くは充てられない（差額は入金額に含まれないので
    // この判定には入れない）。
    if allocated_total > input.amount {
        errors.push(FieldError {
            field: "allocations".to_string(),
            message: "充当額の合計が入金額を超えています".to_string(),
        });
    }

    if errors.is_empty() {
        Ok(NormalizedPayment {
            customer_id: input.customer_id,
            paid_on,
            amount: input.amount,
            method,
            note,
            allocations,
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
        .column("customerId", "customer_id")
        .column("paidOn", "paid_on")
        .column("amount", "amount")
        .column("method", "method")
        .column("note", "note")
        .column("createdAt", "created_at")
        .column("updatedAt", "updated_at")
}

const RESOURCE: &str = "payments";
const COLUMNS: &str = "id, customer_id, paid_on, amount, method, note, created_at, updated_at";
const ALLOCATION_COLUMNS: &str = "id, payment_id, invoice_id, allocated_amount, \
     difference_reason, difference_amount, note, created_at";

fn today_expr(dialect: Dialect) -> &'static str {
    match dialect {
        Dialect::Sqlite => "date('now')",
        Dialect::Postgres => "CURRENT_DATE::text",
    }
}

/// 入金と充当の保存を1トランザクションで行う。充当だけ入れ替わって入金額と
/// 食い違う状態を残さないため（`invoices.rs` の明細と同じ理由）。
macro_rules! save_impl {
    ($fn_name:ident, $backend:ty, $dialect:expr) => {
        async fn $fn_name(
            pool: &sqlx::Pool<$backend>,
            id: Option<i64>,
            value: &NormalizedPayment,
        ) -> Result<Payment, BantoError> {
            let dialect = $dialect;
            let today = today_expr(dialect);
            let mut tx = pool.begin().await.map_err(banto_storage::storage_error)?;

            let payment: Payment = match id {
                None => {
                    let sql = format!(
                        "INSERT INTO payments (customer_id, paid_on, amount, method, note, \
                         created_at, updated_at) \
                         VALUES ({}, {}, {}, {}, {}, {today}, {today}) RETURNING {COLUMNS}",
                        dialect.placeholder(1),
                        dialect.placeholder(2),
                        dialect.placeholder(3),
                        dialect.placeholder(4),
                        dialect.placeholder(5),
                    );
                    sqlx::query_as(&sql)
                        .bind(value.customer_id)
                        .bind(value.paid_on.as_str())
                        .bind(value.amount)
                        .bind(value.method.as_deref())
                        .bind(value.note.as_deref())
                        .fetch_one(&mut *tx)
                        .await
                        .map_err(banto_storage::storage_error)?
                }
                Some(id) => {
                    let sql = format!(
                        "UPDATE payments SET customer_id = {}, paid_on = {}, amount = {}, \
                         method = {}, note = {}, updated_at = {today} \
                         WHERE id = {} RETURNING {COLUMNS}",
                        dialect.placeholder(1),
                        dialect.placeholder(2),
                        dialect.placeholder(3),
                        dialect.placeholder(4),
                        dialect.placeholder(5),
                        dialect.placeholder(6),
                    );
                    sqlx::query_as(&sql)
                        .bind(value.customer_id)
                        .bind(value.paid_on.as_str())
                        .bind(value.amount)
                        .bind(value.method.as_deref())
                        .bind(value.note.as_deref())
                        .bind(id)
                        .fetch_one(&mut *tx)
                        .await
                        .map_err(|err| banto_storage::not_found(err, RESOURCE, id.to_string()))?
                }
            };

            let delete_sql = format!(
                "DELETE FROM payment_allocations WHERE payment_id = {}",
                dialect.placeholder(1)
            );
            sqlx::query(&delete_sql)
                .bind(payment.id)
                .execute(&mut *tx)
                .await
                .map_err(banto_storage::storage_error)?;

            let allocation_sql = format!(
                "INSERT INTO payment_allocations (payment_id, invoice_id, allocated_amount, \
                 difference_reason, difference_amount, note, created_at) \
                 VALUES ({}, {}, {}, {}, {}, {}, {today})",
                dialect.placeholder(1),
                dialect.placeholder(2),
                dialect.placeholder(3),
                dialect.placeholder(4),
                dialect.placeholder(5),
                dialect.placeholder(6),
            );
            for allocation in &value.allocations {
                sqlx::query(&allocation_sql)
                    .bind(payment.id)
                    .bind(allocation.invoice_id)
                    .bind(allocation.allocated_amount)
                    .bind(allocation.difference_reason.as_deref())
                    .bind(allocation.difference_amount)
                    .bind(allocation.note.as_deref())
                    .execute(&mut *tx)
                    .await
                    .map_err(banto_storage::storage_error)?;
            }

            tx.commit().await.map_err(banto_storage::storage_error)?;
            Ok(payment)
        }
    };
}

save_impl!(save_sqlite, sqlx::Sqlite, Dialect::Sqlite);
#[cfg(feature = "postgres")]
save_impl!(save_postgres, sqlx::Postgres, Dialect::Postgres);

/// 請求書1件ぶんの消込集計（SQL で合計するだけ。丸めは無い）。
#[derive(Debug, Clone, sqlx::FromRow)]
struct SettlementRow {
    #[sqlx(rename = "invoice_id")]
    invoice_id: i64,
    #[sqlx(rename = "invoice_number")]
    invoice_number: Option<String>,
    #[sqlx(rename = "customer_id")]
    customer_id: i64,
    #[sqlx(rename = "customer_name")]
    customer_name: String,
    status: String,
    #[sqlx(rename = "issued_on")]
    issued_on: Option<String>,
    #[sqlx(rename = "due_on")]
    due_on: Option<String>,
    #[sqlx(rename = "total_amount")]
    total_amount: i64,
    #[sqlx(rename = "allocated_amount")]
    allocated_amount: i64,
    #[sqlx(rename = "difference_amount")]
    difference_amount: i64,
}

impl SettlementRow {
    fn into_settlement(self, today: &str) -> InvoiceSettlement {
        let settled_amount = self.allocated_amount + self.difference_amount;
        let remaining_amount = remaining_amount(self.total_amount, settled_amount);
        let overdue = is_overdue(
            self.due_on.as_deref(),
            remaining_amount,
            &self.status,
            today,
        );
        InvoiceSettlement {
            settlement_status: settlement_status(&self.status, settled_amount, remaining_amount)
                .to_string(),
            invoice_id: self.invoice_id,
            invoice_number: self.invoice_number,
            customer_id: self.customer_id,
            customer_name: self.customer_name,
            status: self.status,
            issued_on: self.issued_on,
            due_on: self.due_on,
            total_amount: self.total_amount,
            allocated_amount: self.allocated_amount,
            difference_amount: self.difference_amount,
            settled_amount,
            remaining_amount,
            overdue,
        }
    }
}

/// `CAST(... AS BIGINT)` は PostgreSQL の `SUM(bigint)` が numeric を返すため
/// （`profitability.rs` と同じ理由）。相関サブクエリにしているのは、充当が
/// 1件も無い請求書も 0 として出すため（LEFT JOIN + GROUP BY より読みやすい）。
const SETTLEMENT_SELECT: &str = "SELECT i.id AS invoice_id, i.invoice_number, \
     i.customer_id, c.name AS customer_name, i.status, i.issued_on, i.due_on, \
     i.total_amount, \
     CAST(COALESCE((SELECT SUM(a.allocated_amount) FROM payment_allocations a \
         WHERE a.invoice_id = i.id), 0) AS BIGINT) AS allocated_amount, \
     CAST(COALESCE((SELECT SUM(a.difference_amount) FROM payment_allocations a \
         WHERE a.invoice_id = i.id), 0) AS BIGINT) AS difference_amount \
     FROM invoices i JOIN customers c ON c.id = i.customer_id";

/// 入金のサービス層（conventions §2）。
#[derive(Clone)]
pub struct PaymentsService {
    db: Db,
    events: Option<broadcast::Sender<ServerEvent>>,
}

impl PaymentsService {
    pub fn new(db: Db) -> Self {
        Self { db, events: None }
    }

    pub fn with_events(mut self, events: broadcast::Sender<ServerEvent>) -> Self {
        self.events = Some(events);
        self
    }

    fn notify_changed(&self) {
        if let Some(tx) = &self.events {
            // 消込は請求書の見え方（残額・入金状態）も変えるので、両方に流す。
            for resource in ["payments", "invoices"] {
                let _ = tx.send(ServerEvent::ResourceChanged {
                    resource: resource.to_string(),
                });
            }
        }
    }

    async fn today(&self) -> Result<String, BantoError> {
        let dialect = self.db.dialect();
        let sql = format!("SELECT {}", today_expr(dialect));
        match &self.db {
            Db::Sqlite(pool) => sqlx::query_scalar::<_, String>(&sql).fetch_one(pool).await,
            #[cfg(feature = "postgres")]
            Db::Postgres(pool) => sqlx::query_scalar::<_, String>(&sql).fetch_one(pool).await,
        }
        .map_err(banto_storage::storage_error)
    }

    pub async fn list(&self, params: ListParams) -> Result<ListResult<Payment>, BantoError> {
        let columns = column_map();
        let select_rows = format!("SELECT {COLUMNS} FROM payments");
        let select_count = "SELECT COUNT(*) FROM payments".to_string();
        match &self.db {
            Db::Sqlite(pool) => {
                let mut rows_builder: QueryBuilder<'_, Sqlite> = QueryBuilder::new(&select_rows);
                banto_storage::list_query::sqlite::apply_list_params(
                    &mut rows_builder,
                    &columns,
                    &params,
                )?;
                let rows: Vec<Payment> = rows_builder
                    .build_query_as::<Payment>()
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
                let rows: Vec<Payment> = rows_builder
                    .build_query_as::<Payment>()
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

    async fn payment_row(&self, id: i64) -> Result<Payment, BantoError> {
        let dialect = self.db.dialect();
        let sql = format!(
            "SELECT {COLUMNS} FROM payments WHERE id = {}",
            dialect.placeholder(1)
        );
        match &self.db {
            Db::Sqlite(pool) => {
                sqlx::query_as::<_, Payment>(&sql)
                    .bind(id)
                    .fetch_one(pool)
                    .await
            }
            #[cfg(feature = "postgres")]
            Db::Postgres(pool) => {
                sqlx::query_as::<_, Payment>(&sql)
                    .bind(id)
                    .fetch_one(pool)
                    .await
            }
        }
        .map_err(|err| banto_storage::not_found(err, RESOURCE, id.to_string()))
    }

    async fn allocations_of(&self, payment_id: i64) -> Result<Vec<PaymentAllocation>, BantoError> {
        let dialect = self.db.dialect();
        let sql = format!(
            "SELECT {ALLOCATION_COLUMNS} FROM payment_allocations WHERE payment_id = {} \
             ORDER BY id",
            dialect.placeholder(1)
        );
        match &self.db {
            Db::Sqlite(pool) => {
                sqlx::query_as::<_, PaymentAllocation>(&sql)
                    .bind(payment_id)
                    .fetch_all(pool)
                    .await
            }
            #[cfg(feature = "postgres")]
            Db::Postgres(pool) => {
                sqlx::query_as::<_, PaymentAllocation>(&sql)
                    .bind(payment_id)
                    .fetch_all(pool)
                    .await
            }
        }
        .map_err(banto_storage::storage_error)
    }

    async fn customer_name(&self, customer_id: i64) -> Result<String, BantoError> {
        let dialect = self.db.dialect();
        let sql = format!(
            "SELECT name FROM customers WHERE id = {}",
            dialect.placeholder(1)
        );
        let found: Option<String> = match &self.db {
            Db::Sqlite(pool) => {
                sqlx::query_scalar(&sql)
                    .bind(customer_id)
                    .fetch_optional(pool)
                    .await
            }
            #[cfg(feature = "postgres")]
            Db::Postgres(pool) => {
                sqlx::query_scalar(&sql)
                    .bind(customer_id)
                    .fetch_optional(pool)
                    .await
            }
        }
        .map_err(banto_storage::storage_error)?;
        found.ok_or_else(|| BantoError::Validation {
            field_errors: vec![FieldError {
                field: "customerId".to_string(),
                message: "顧客が見つかりません".to_string(),
            }],
        })
    }

    /// 充当先が実在し、**この顧客の確定済み請求書である**ことを確かめる。
    /// Draft は請求していないので充当できず、取消済みも同様。
    async fn ensure_invoices_are_billable(
        &self,
        customer_id: i64,
        allocations: &[NormalizedAllocation],
    ) -> Result<(), BantoError> {
        let dialect = self.db.dialect();
        let sql = format!(
            "SELECT status FROM invoices WHERE id = {} AND customer_id = {}",
            dialect.placeholder(1),
            dialect.placeholder(2)
        );
        for (index, allocation) in allocations.iter().enumerate() {
            let status: Option<String> = match &self.db {
                Db::Sqlite(pool) => {
                    sqlx::query_scalar(&sql)
                        .bind(allocation.invoice_id)
                        .bind(customer_id)
                        .fetch_optional(pool)
                        .await
                }
                #[cfg(feature = "postgres")]
                Db::Postgres(pool) => {
                    sqlx::query_scalar(&sql)
                        .bind(allocation.invoice_id)
                        .bind(customer_id)
                        .fetch_optional(pool)
                        .await
                }
            }
            .map_err(banto_storage::storage_error)?;
            let message = match status.as_deref() {
                None => "この顧客の請求書を選択してください",
                Some(s) if s == STATUS_ISSUED => continue,
                Some(_) => "確定済みの請求書にのみ充当できます",
            };
            return Err(BantoError::Validation {
                field_errors: vec![FieldError {
                    field: format!("allocations.{index}.invoiceId"),
                    message: message.to_string(),
                }],
            });
        }
        Ok(())
    }

    async fn detail(&self, payment: Payment) -> Result<PaymentDetail, BantoError> {
        let customer_name = self.customer_name(payment.customer_id).await?;
        let allocations = self.allocations_of(payment.id).await?;
        let allocated: i64 = allocations.iter().map(|a| a.allocated_amount).sum();
        Ok(PaymentDetail {
            unallocated_amount: payment.amount - allocated,
            customer_name,
            payment,
            allocations,
        })
    }

    pub async fn get(&self, id: i64) -> Result<PaymentDetail, BantoError> {
        let payment = self.payment_row(id).await?;
        self.detail(payment).await
    }

    pub async fn create(&self, input: PaymentInput) -> Result<PaymentDetail, BantoError> {
        let value = validate(&input)?;
        self.customer_name(value.customer_id).await?;
        self.ensure_invoices_are_billable(value.customer_id, &value.allocations)
            .await?;
        let payment = match &self.db {
            Db::Sqlite(pool) => save_sqlite(pool, None, &value).await?,
            #[cfg(feature = "postgres")]
            Db::Postgres(pool) => save_postgres(pool, None, &value).await?,
        };
        self.notify_changed();
        self.detail(payment).await
    }

    pub async fn update(&self, id: i64, input: PaymentInput) -> Result<PaymentDetail, BantoError> {
        // 入金は確定という概念を持たない（請求書と違い番号を採番しないため）。
        // 誤入力は直せるほうが実態に合う。
        self.payment_row(id).await?;
        let value = validate(&input)?;
        self.customer_name(value.customer_id).await?;
        self.ensure_invoices_are_billable(value.customer_id, &value.allocations)
            .await?;
        let payment = match &self.db {
            Db::Sqlite(pool) => save_sqlite(pool, Some(id), &value).await?,
            #[cfg(feature = "postgres")]
            Db::Postgres(pool) => save_postgres(pool, Some(id), &value).await?,
        };
        self.notify_changed();
        self.detail(payment).await
    }

    pub async fn delete(&self, id: i64) -> Result<(), BantoError> {
        let dialect = self.db.dialect();
        // payment_allocations は ON DELETE CASCADE。
        let sql = format!("DELETE FROM payments WHERE id = {}", dialect.placeholder(1));
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

    async fn settlements_where(
        &self,
        where_clause: &str,
        bind_id: Option<i64>,
    ) -> Result<Vec<InvoiceSettlement>, BantoError> {
        let today = self.today().await?;
        let sql = format!("{SETTLEMENT_SELECT} {where_clause}");
        let rows: Vec<SettlementRow> = match &self.db {
            Db::Sqlite(pool) => {
                let query = sqlx::query_as::<_, SettlementRow>(&sql);
                match bind_id {
                    Some(id) => query.bind(id).fetch_all(pool).await,
                    None => query.fetch_all(pool).await,
                }
            }
            #[cfg(feature = "postgres")]
            Db::Postgres(pool) => {
                let query = sqlx::query_as::<_, SettlementRow>(&sql);
                match bind_id {
                    Some(id) => query.bind(id).fetch_all(pool).await,
                    None => query.fetch_all(pool).await,
                }
            }
        }
        .map_err(banto_storage::storage_error)?;
        Ok(rows
            .into_iter()
            .map(|row| row.into_settlement(&today))
            .collect())
    }

    /// 請求書1件の入金状況。請求書が無ければ `NotFound`。
    pub async fn settlement(&self, invoice_id: i64) -> Result<InvoiceSettlement, BantoError> {
        let dialect = self.db.dialect();
        let where_clause = format!("WHERE i.id = {}", dialect.placeholder(1));
        let mut rows = self
            .settlements_where(&where_clause, Some(invoice_id))
            .await?;
        if rows.is_empty() {
            return Err(BantoError::NotFound {
                resource: "invoices".to_string(),
                id: invoice_id.to_string(),
            });
        }
        Ok(rows.remove(0))
    }

    /// 未入金・期限超過の一覧（要件 F-Y7）。**残額が残っている確定済み請求書**を
    /// 期限の近い順に返す。Draft と取消済みは対象外（請求していない／取り消した
    /// ものは回収対象ではない）。
    ///
    /// 期限未設定の請求書も残額があれば含める（回収対象ではあるため）。並びは
    /// 期限が早い順で、未設定は末尾。
    pub async fn outstanding(&self) -> Result<Vec<InvoiceSettlement>, BantoError> {
        let where_clause = format!("WHERE i.status = '{STATUS_ISSUED}' ORDER BY i.due_on");
        let rows = self.settlements_where(&where_clause, None).await?;
        Ok(rows
            .into_iter()
            .filter(|settlement| settlement.remaining_amount > 0)
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::customers::{CustomerInput, CustomersService, DAY_END_OF_MONTH};
    use crate::db::migrate_memory;
    use crate::invoices::{InvoiceInput, InvoiceLineInput, InvoicesService};
    use crate::projects::{ProjectInput, ProjectsService};

    struct Fixture {
        payments: PaymentsService,
        invoices: InvoicesService,
        customer_id: i64,
        project_id: i64,
    }

    async fn fixture() -> Fixture {
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
                contract_amount: Some(1_000_000),
                billing_hourly_rate: Some(10_000),
                scope: None,
                note: None,
            })
            .await
            .expect("project");
        Fixture {
            payments: PaymentsService::new(pool.clone()),
            invoices: InvoicesService::new(pool.clone()),
            customer_id: customer.id,
            project_id: project.id,
        }
    }

    impl Fixture {
        /// 税抜 `taxable` 円・10% の請求書を1件確定して返す（税込は 1.1 倍）。
        async fn issue_invoice(&self, taxable: i64) -> i64 {
            let draft = self
                .invoices
                .create(InvoiceInput {
                    customer_id: self.customer_id,
                    closing_on: None,
                    due_on: None,
                    corrected_invoice_id: None,
                    note: None,
                    lines: vec![InvoiceLineInput {
                        project_id: self.project_id,
                        item_name: "設計".to_string(),
                        quantity: 1,
                        unit_price: taxable,
                        tax_category: "STANDARD_10".to_string(),
                        source_type: None,
                        source_id: None,
                        note: None,
                    }],
                })
                .await
                .expect("draft");
            self.invoices
                .issue(draft.invoice.id)
                .await
                .expect("issue")
                .invoice
                .id
        }
    }

    fn payment(
        customer_id: i64,
        amount: i64,
        allocations: Vec<PaymentAllocationInput>,
    ) -> PaymentInput {
        PaymentInput {
            customer_id,
            paid_on: "2026-09-30".to_string(),
            amount,
            method: Some("振込".to_string()),
            note: None,
            allocations,
        }
    }

    fn allocation(invoice_id: i64, allocated: i64) -> PaymentAllocationInput {
        PaymentAllocationInput {
            invoice_id,
            allocated_amount: allocated,
            difference_reason: None,
            difference_amount: 0,
            note: None,
        }
    }

    // --- V-4: Overdue 導出の境界（純粋関数） ---

    #[test]
    fn overdue_is_false_on_the_due_date_itself() {
        // 期限当日は超過しない（`due < today` であって `<=` ではない）。
        assert!(!is_overdue(
            Some("2026-09-30"),
            1_000,
            "ISSUED",
            "2026-09-30"
        ));
        assert!(is_overdue(
            Some("2026-09-30"),
            1_000,
            "ISSUED",
            "2026-10-01"
        ));
    }

    #[test]
    fn overdue_is_false_when_nothing_remains() {
        assert!(!is_overdue(Some("2026-09-30"), 0, "ISSUED", "2026-10-31"));
    }

    #[test]
    fn overdue_is_false_for_cancelled_invoices() {
        assert!(!is_overdue(
            Some("2026-09-30"),
            1_000,
            "CANCELLED",
            "2026-10-31"
        ));
    }

    #[test]
    fn overdue_is_false_without_a_due_date() {
        assert!(!is_overdue(None, 1_000, "ISSUED", "2026-10-31"));
    }

    #[test]
    fn remaining_never_goes_below_zero() {
        assert_eq!(remaining_amount(110_000, 100_000), 10_000);
        assert_eq!(remaining_amount(110_000, 110_000), 0);
        // 過入金でもマイナスにしない（決定 C-11）。
        assert_eq!(remaining_amount(110_000, 120_000), 0);
    }

    #[test]
    fn settlement_status_is_derived_from_amounts() {
        assert_eq!(settlement_status("DRAFT", 0, 0), "DRAFT");
        assert_eq!(settlement_status("CANCELLED", 0, 0), "CANCELLED");
        assert_eq!(settlement_status("ISSUED", 0, 110_000), "ISSUED");
        assert_eq!(
            settlement_status("ISSUED", 50_000, 60_000),
            "PARTIALLY_PAID"
        );
        assert_eq!(settlement_status("ISSUED", 110_000, 0), "PAID");
    }

    // --- V-3: 消込の4パターン ---

    /// 一部入金（分割入金。要件 F-Y2）。
    #[tokio::test]
    async fn partial_payments_accumulate_until_the_invoice_is_settled() {
        let f = fixture().await;
        // 税抜 100,000 → 税込 110,000
        let invoice_id = f.issue_invoice(100_000).await;

        f.payments
            .create(payment(
                f.customer_id,
                50_000,
                vec![allocation(invoice_id, 50_000)],
            ))
            .await
            .expect("first payment");
        let after_first = f.payments.settlement(invoice_id).await.expect("settlement");
        assert_eq!(after_first.total_amount, 110_000);
        assert_eq!(after_first.settled_amount, 50_000);
        assert_eq!(after_first.remaining_amount, 60_000);
        assert_eq!(after_first.settlement_status, "PARTIALLY_PAID");

        f.payments
            .create(payment(
                f.customer_id,
                60_000,
                vec![allocation(invoice_id, 60_000)],
            ))
            .await
            .expect("second payment");
        let after_second = f.payments.settlement(invoice_id).await.expect("settlement");
        assert_eq!(after_second.remaining_amount, 0);
        assert_eq!(after_second.settlement_status, "PAID");
    }

    /// まとめ入金（1入金 → 複数請求書。要件 F-Y1）。
    #[tokio::test]
    async fn one_payment_settles_several_invoices() {
        let f = fixture().await;
        let first = f.issue_invoice(100_000).await; // 税込 110,000
        let second = f.issue_invoice(50_000).await; // 税込  55,000

        f.payments
            .create(payment(
                f.customer_id,
                165_000,
                vec![allocation(first, 110_000), allocation(second, 55_000)],
            ))
            .await
            .expect("bundled payment");

        for invoice_id in [first, second] {
            let settlement = f.payments.settlement(invoice_id).await.expect("settlement");
            assert_eq!(settlement.remaining_amount, 0, "invoice {invoice_id}");
            assert_eq!(settlement.settlement_status, "PAID");
        }
    }

    /// 手数料の先方差引。差額を記録して請求書を閉じる（決定 C-19）。
    #[tokio::test]
    async fn a_deducted_transfer_fee_closes_the_invoice_through_the_difference() {
        let f = fixture().await;
        let invoice_id = f.issue_invoice(100_000).await; // 税込 110,000

        // 先方が手数料 660 円を差し引いて 109,340 円入金した場合。
        f.payments
            .create(payment(
                f.customer_id,
                109_340,
                vec![PaymentAllocationInput {
                    invoice_id,
                    allocated_amount: 109_340,
                    difference_reason: Some("TRANSFER_FEE".to_string()),
                    difference_amount: 660,
                    note: None,
                }],
            ))
            .await
            .expect("payment with fee");

        let settlement = f.payments.settlement(invoice_id).await.expect("settlement");
        assert_eq!(settlement.allocated_amount, 109_340);
        assert_eq!(settlement.difference_amount, 660);
        assert_eq!(settlement.settled_amount, 110_000);
        assert_eq!(settlement.remaining_amount, 0);
        assert_eq!(settlement.settlement_status, "PAID");
    }

    /// 過入金。残額はマイナスにせず 0 で止める（決定 C-11）。
    #[tokio::test]
    async fn overpayment_is_recorded_but_never_makes_the_remainder_negative() {
        let f = fixture().await;
        let invoice_id = f.issue_invoice(100_000).await; // 税込 110,000

        f.payments
            .create(payment(
                f.customer_id,
                120_000,
                vec![PaymentAllocationInput {
                    invoice_id,
                    allocated_amount: 120_000,
                    difference_reason: Some("OVERPAYMENT".to_string()),
                    difference_amount: 10_000,
                    note: None,
                }],
            ))
            .await
            .expect("overpayment");

        let settlement = f.payments.settlement(invoice_id).await.expect("settlement");
        assert_eq!(settlement.settled_amount, 130_000);
        assert_eq!(settlement.remaining_amount, 0);
        assert_eq!(settlement.settlement_status, "PAID");
    }

    // --- 入力の検証 ---

    #[tokio::test]
    async fn allocations_cannot_exceed_the_payment_amount() {
        let f = fixture().await;
        let invoice_id = f.issue_invoice(100_000).await;
        let err = f
            .payments
            .create(payment(
                f.customer_id,
                50_000,
                vec![allocation(invoice_id, 60_000)],
            ))
            .await
            .unwrap_err();
        match err {
            BantoError::Validation { field_errors } => {
                assert_eq!(field_errors[0].field, "allocations");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[tokio::test]
    async fn drafts_and_cancelled_invoices_cannot_receive_payments() {
        let f = fixture().await;
        let draft = f
            .invoices
            .create(InvoiceInput {
                customer_id: f.customer_id,
                closing_on: None,
                due_on: None,
                corrected_invoice_id: None,
                note: None,
                lines: vec![InvoiceLineInput {
                    project_id: f.project_id,
                    item_name: "設計".to_string(),
                    quantity: 1,
                    unit_price: 10_000,
                    tax_category: "STANDARD_10".to_string(),
                    source_type: None,
                    source_id: None,
                    note: None,
                }],
            })
            .await
            .expect("draft");
        let err = f
            .payments
            .create(payment(
                f.customer_id,
                10_000,
                vec![allocation(draft.invoice.id, 10_000)],
            ))
            .await
            .unwrap_err();
        assert!(matches!(err, BantoError::Validation { .. }), "{err:?}");

        let issued = f.issue_invoice(10_000).await;
        f.invoices.cancel(issued).await.expect("cancel");
        let err = f
            .payments
            .create(payment(
                f.customer_id,
                11_000,
                vec![allocation(issued, 11_000)],
            ))
            .await
            .unwrap_err();
        assert!(matches!(err, BantoError::Validation { .. }), "{err:?}");
    }

    #[tokio::test]
    async fn a_difference_needs_a_reason_and_other_needs_a_note() {
        let f = fixture().await;
        let invoice_id = f.issue_invoice(100_000).await;

        let mut no_reason = allocation(invoice_id, 109_340);
        no_reason.difference_amount = 660;
        let err = f
            .payments
            .create(payment(f.customer_id, 109_340, vec![no_reason]))
            .await
            .unwrap_err();
        match err {
            BantoError::Validation { field_errors } => {
                assert_eq!(field_errors[0].field, "allocations.0.differenceReason");
            }
            other => panic!("unexpected: {other:?}"),
        }

        let other_without_note = PaymentAllocationInput {
            invoice_id,
            allocated_amount: 109_340,
            difference_reason: Some("OTHER".to_string()),
            difference_amount: 660,
            note: None,
        };
        let err = f
            .payments
            .create(payment(f.customer_id, 109_340, vec![other_without_note]))
            .await
            .unwrap_err();
        match err {
            BantoError::Validation { field_errors } => {
                assert_eq!(field_errors[0].field, "allocations.0.note");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[tokio::test]
    async fn unallocated_amount_is_reported() {
        let f = fixture().await;
        let invoice_id = f.issue_invoice(100_000).await;
        let created = f
            .payments
            .create(payment(
                f.customer_id,
                110_000,
                vec![allocation(invoice_id, 100_000)],
            ))
            .await
            .expect("payment");
        assert_eq!(created.unallocated_amount, 10_000);
    }

    #[tokio::test]
    async fn payments_can_be_edited_and_deleted() {
        let f = fixture().await;
        let invoice_id = f.issue_invoice(100_000).await;
        let created = f
            .payments
            .create(payment(
                f.customer_id,
                50_000,
                vec![allocation(invoice_id, 50_000)],
            ))
            .await
            .expect("payment");

        // 充当を直すと請求書の残額も追随する（消込は導出値だから）。
        f.payments
            .update(
                created.payment.id,
                payment(
                    f.customer_id,
                    110_000,
                    vec![allocation(invoice_id, 110_000)],
                ),
            )
            .await
            .expect("update");
        let settled = f.payments.settlement(invoice_id).await.expect("settlement");
        assert_eq!(settled.remaining_amount, 0);

        f.payments.delete(created.payment.id).await.expect("delete");
        let after_delete = f.payments.settlement(invoice_id).await.expect("settlement");
        assert_eq!(after_delete.remaining_amount, 110_000);
        assert_eq!(after_delete.settlement_status, "ISSUED");
    }

    /// 未入金一覧（要件 F-Y7）は残額のある確定済み請求書だけを返す。
    #[tokio::test]
    async fn outstanding_lists_only_invoices_with_a_remainder() {
        let f = fixture().await;
        let unpaid = f.issue_invoice(100_000).await;
        let paid = f.issue_invoice(50_000).await;
        f.payments
            .create(payment(
                f.customer_id,
                55_000,
                vec![allocation(paid, 55_000)],
            ))
            .await
            .expect("payment");

        let outstanding = f.payments.outstanding().await.expect("outstanding");
        let ids: Vec<i64> = outstanding.iter().map(|s| s.invoice_id).collect();
        assert_eq!(ids, vec![unpaid]);
        assert_eq!(outstanding[0].remaining_amount, 110_000);
        assert_eq!(outstanding[0].customer_name, "架空商事");
    }

    #[tokio::test]
    async fn settlement_of_an_unknown_invoice_is_not_found() {
        let f = fixture().await;
        let err = f.payments.settlement(9_999).await.unwrap_err();
        assert!(matches!(err, BantoError::NotFound { .. }), "{err:?}");
    }
}
