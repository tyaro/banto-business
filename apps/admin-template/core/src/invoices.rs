//! Phase 5: 請求（`docs/domain/schema.md` §4 / `docs/tax-calculation.md`）。
//! conventions §2 に従い `tauri` / `axum` / RBAC を知らない。
//!
//! ## 構造（`CLAUDE.md` 1.3）
//!
//! `Invoice` は `customer_id` を持ち、案件は `InvoiceLine.project_id` が持つ。
//! 1請求書に複数案件（締日まとめ請求）と、1案件に複数請求書（着手金／中間金／
//! 検収）の両方が実務で起きるため、`Invoice.project_id` は作らない。
//!
//! ## 状態（`CLAUDE.md` 1.5 / 決定 C-15）
//!
//! `status` は `DRAFT` / `ISSUED` / `CANCELLED` の3値のみ。`PARTIALLY_PAID` /
//! `PAID` / `Overdue` は保持せず、消込の残額と支払期限から都度導出する。
//!
//! ## 確定時のスナップショット（要件 F-I7）
//!
//! 確定（`issue`）で、請求書番号・発行日・支払期限・端数処理方向・発行者情報・
//! 税率区分ごとの集計を**その時点の値で焼き付ける**。以後、設定や顧客マスタを
//! 変えても発行済みの請求書は動かない（`CLAUDE.md` 1.2 と同じ考え方）。
//! 確定後は明細を編集できない（F-I8）— 訂正は取消（赤伝）＋新規発行で行う。

use crate::dates::{add_months, day_of_month, is_valid_date, parse_iso_date};
use crate::issuer::{IssuerService, IssuerSettings};
use crate::profitability::taxable_amount;
use crate::tax::{calculate_tax, RoundingMode, TaxCategory, TaxLine, TaxSummary};
use banto_admin_services::settings::SettingsService;
use banto_core::{BantoError, FieldError, ListParams, ListResult};
use banto_server::ServerEvent;
use banto_storage::{ColumnMap, Db, Dialect};
use serde::{Deserialize, Serialize};
use sqlx::{QueryBuilder, Sqlite};
use tokio::sync::broadcast;

/// 請求書の状態（決定 C-15）。
pub const INVOICE_STATUSES: [&str; 3] = ["DRAFT", "ISSUED", "CANCELLED"];
pub const STATUS_DRAFT: &str = "DRAFT";
pub const STATUS_ISSUED: &str = "ISSUED";
pub const STATUS_CANCELLED: &str = "CANCELLED";

/// 明細の出どころ（`schema.md` §4.2）。確定時に元の工数・経費へ `invoiced` を
/// 立て、取消時に戻すために使う。
pub const SOURCE_WORK_LOG: &str = "WORK_LOG";
pub const SOURCE_EXPENSE: &str = "EXPENSE";
pub const SOURCE_MANUAL: &str = "MANUAL";

const MAX_AMOUNT: i64 = 9_999_999_999;
const MAX_QUANTITY: i64 = 100_000;
const MAX_TEXT_LEN: usize = 120;
const MAX_NOTE_LEN: usize = 500;
const MAX_LINES: usize = 200;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct Invoice {
    pub id: i64,
    /// `INV-YYYY-NNNN`。**Draft では `None`**、確定時に採番する（決定 C-9。
    /// 適格請求書の連続性のため欠番を作らない）。
    #[sqlx(rename = "invoice_number")]
    pub invoice_number: Option<String>,
    #[sqlx(rename = "customer_id")]
    pub customer_id: i64,
    pub status: String,
    #[sqlx(rename = "issued_on")]
    pub issued_on: Option<String>,
    #[sqlx(rename = "closing_on")]
    pub closing_on: Option<String>,
    /// 支払期限。確定時に顧客マスタの締日・支払条件から算出して保存する
    /// （決定 C-8）。手で入れてあればそれを優先する。
    #[sqlx(rename = "due_on")]
    pub due_on: Option<String>,
    /// 赤伝で差し替えた元請求書（決定 C-10）。
    #[sqlx(rename = "corrected_invoice_id")]
    pub corrected_invoice_id: Option<i64>,
    #[sqlx(rename = "total_taxable")]
    pub total_taxable: i64,
    #[sqlx(rename = "total_tax")]
    pub total_tax: i64,
    #[sqlx(rename = "total_amount")]
    pub total_amount: i64,
    #[sqlx(rename = "rounding_mode")]
    pub rounding_mode: String,
    #[sqlx(rename = "issuer_name")]
    pub issuer_name: Option<String>,
    #[sqlx(rename = "issuer_registration_number")]
    pub issuer_registration_number: Option<String>,
    #[sqlx(rename = "issuer_address")]
    pub issuer_address: Option<String>,
    pub note: Option<String>,
    #[sqlx(rename = "created_at")]
    pub created_at: String,
    #[sqlx(rename = "updated_at")]
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct InvoiceLine {
    pub id: i64,
    #[sqlx(rename = "invoice_id")]
    pub invoice_id: i64,
    #[sqlx(rename = "project_id")]
    pub project_id: i64,
    #[sqlx(rename = "line_no")]
    pub line_no: i64,
    #[sqlx(rename = "item_name")]
    pub item_name: String,
    pub quantity: i64,
    #[sqlx(rename = "unit_price")]
    pub unit_price: i64,
    /// 行金額（税抜）＝ `quantity × unit_price`。**マイナス可**（値引き行、B-3）。
    /// 行ごとの税額は持たない — 端数処理は税率区分ごとに1回だけ（`CLAUDE.md` 1.7）。
    pub amount: i64,
    #[sqlx(rename = "tax_category")]
    pub tax_category: String,
    #[sqlx(rename = "source_type")]
    pub source_type: Option<String>,
    #[sqlx(rename = "source_id")]
    pub source_id: Option<i64>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct InvoiceTaxSummary {
    pub id: i64,
    #[sqlx(rename = "invoice_id")]
    pub invoice_id: i64,
    #[sqlx(rename = "tax_category")]
    pub tax_category: String,
    #[sqlx(rename = "rate_bp")]
    pub rate_bp: i64,
    #[sqlx(rename = "taxable_amount")]
    pub taxable_amount: i64,
    #[sqlx(rename = "tax_amount")]
    pub tax_amount: i64,
}

/// 1件の請求書とその明細・税集計。`getOne` の戻り値。
///
/// 顧客名は表示・PDF 用に join して載せる（`invoices` にスナップショットは
/// 持たない — `schema.md` §4.1 の設計に従う）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InvoiceDetail {
    #[serde(flatten)]
    pub invoice: Invoice,
    pub customer_name: String,
    pub customer_billing_name: Option<String>,
    pub lines: Vec<InvoiceLine>,
    pub tax_summaries: Vec<InvoiceTaxSummary>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InvoiceLineInput {
    pub project_id: i64,
    pub item_name: String,
    #[serde(default = "default_quantity")]
    pub quantity: i64,
    pub unit_price: i64,
    pub tax_category: String,
    pub source_type: Option<String>,
    pub source_id: Option<i64>,
    pub note: Option<String>,
}

fn default_quantity() -> i64 {
    1
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InvoiceInput {
    pub customer_id: i64,
    pub closing_on: Option<String>,
    /// 空なら確定時に顧客マスタから算出する（決定 C-8）。
    pub due_on: Option<String>,
    /// 赤伝の差し替え先（決定 C-10）。
    pub corrected_invoice_id: Option<i64>,
    pub note: Option<String>,
    #[serde(default)]
    pub lines: Vec<InvoiceLineInput>,
}

/// 未請求の工数・経費から起こす明細候補（要件 F-I1）。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CandidateQuery {
    pub customer_id: i64,
    pub from: String,
    pub to: String,
}

/// 候補1件。**1件の工数・経費につき1行**にする（`invoice_lines.source_id` が
/// 単一 id なので、まとめると確定時にどれへ `invoiced` を立てるか辿れない）。
/// まとめたい場合は Draft の明細を手で編集する。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CandidateLine {
    pub project_id: i64,
    pub project_code: String,
    pub project_name: String,
    pub source_type: String,
    pub source_id: i64,
    pub item_name: String,
    pub quantity: i64,
    pub unit_price: i64,
    pub amount: i64,
    pub tax_category: String,
    pub note: String,
    /// 工数候補のみ。請求単価が未設定なら `None` で金額 0（画面で単価の入力を
    /// 促す）。
    pub billing_hourly_rate: Option<i64>,
    pub minutes: Option<i64>,
}

/// 工数から起こす行金額（決定 B-2）。`floor(分 × 時間単価 ÷ 60)` で、
/// 内部原価（`work_logs::internal_cost`）と同じ丸め方を使う。
pub fn work_line_amount(minutes: i64, billing_hourly_rate: i64) -> i64 {
    minutes * billing_hourly_rate / 60
}

/// 分を「1h30m」の形にする（候補の備考に内訳を残すため）。
fn format_minutes(minutes: i64) -> String {
    let hours = minutes / 60;
    let rest = minutes % 60;
    if hours > 0 && rest > 0 {
        format!("{hours}h{rest}m")
    } else if hours > 0 {
        format!("{hours}h")
    } else {
        format!("{rest}m")
    }
}

/// 支払期限を締日と支払条件から算出する（決定 C-8）。
///
/// 締日の年月に `payment_month_offset` ヶ月を足し、`payment_day`（`99` は月末）
/// の日を返す。**土日祝の調整はしない**（祝日マスタを持たないため、推測で
/// 前後させると実態とずれる）。
pub fn derive_due_date(
    closing_on: &str,
    payment_month_offset: i64,
    payment_day: i64,
) -> Option<String> {
    let (year, month, _) = parse_iso_date(closing_on)?;
    let (due_year, due_month) = add_months(year, month, payment_month_offset)?;
    day_of_month(
        due_year,
        due_month,
        payment_day,
        payment_day == crate::customers::DAY_END_OF_MONTH,
    )
}

/// 締日を発行日の年月と顧客の締日コードから決める（`closing_on` 未入力時）。
pub fn derive_closing_date(issued_on: &str, closing_day: i64) -> Option<String> {
    let (year, month, _) = parse_iso_date(issued_on)?;
    day_of_month(
        year,
        month,
        closing_day,
        closing_day == crate::customers::DAY_END_OF_MONTH,
    )
}

struct NormalizedLine {
    project_id: i64,
    line_no: i64,
    item_name: String,
    quantity: i64,
    unit_price: i64,
    amount: i64,
    tax_category: String,
    source_type: Option<String>,
    source_id: Option<i64>,
    note: Option<String>,
}

struct NormalizedInvoice {
    customer_id: i64,
    closing_on: Option<String>,
    due_on: Option<String>,
    corrected_invoice_id: Option<i64>,
    note: Option<String>,
    lines: Vec<NormalizedLine>,
}

fn optional_date(
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
            message: "日付は YYYY-MM-DD で入力してください".to_string(),
        });
        return None;
    }
    Some(trimmed.to_string())
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

fn validate(input: &InvoiceInput) -> Result<NormalizedInvoice, BantoError> {
    let mut errors: Vec<FieldError> = Vec::new();

    if input.customer_id <= 0 {
        errors.push(FieldError {
            field: "customerId".to_string(),
            message: "顧客を選択してください".to_string(),
        });
    }
    let closing_on = optional_date(&mut errors, "closingOn", &input.closing_on);
    let due_on = optional_date(&mut errors, "dueOn", &input.due_on);
    let note = optional_text(&mut errors, "note", &input.note, MAX_NOTE_LEN);

    if input.lines.len() > MAX_LINES {
        errors.push(FieldError {
            field: "lines".to_string(),
            message: format!("明細は{MAX_LINES}行までです"),
        });
    }

    let mut lines = Vec::with_capacity(input.lines.len());
    for (index, line) in input.lines.iter().enumerate() {
        let field = |name: &str| format!("lines.{index}.{name}");
        if line.project_id <= 0 {
            errors.push(FieldError {
                field: field("projectId"),
                message: "案件を選択してください".to_string(),
            });
        }
        let item_name = line.item_name.trim();
        if item_name.is_empty() {
            errors.push(FieldError {
                field: field("itemName"),
                message: "必須項目です".to_string(),
            });
        } else if item_name.chars().count() > MAX_TEXT_LEN {
            errors.push(FieldError {
                field: field("itemName"),
                message: format!("{MAX_TEXT_LEN}文字以内で入力してください"),
            });
        }
        if !(1..=MAX_QUANTITY).contains(&line.quantity) {
            errors.push(FieldError {
                field: field("quantity"),
                message: format!("数量は1〜{MAX_QUANTITY}で入力してください"),
            });
        }
        // 単価はマイナスを許す（値引き行。決定 B-3）。桁の打ち間違いは
        // 絶対値の上限で捕まえる。
        if line.unit_price.abs() > MAX_AMOUNT {
            errors.push(FieldError {
                field: field("unitPrice"),
                message: "金額が大きすぎます".to_string(),
            });
        }
        if TaxCategory::from_code(line.tax_category.trim()).is_none() {
            errors.push(FieldError {
                field: field("taxCategory"),
                message: "税区分が不正です".to_string(),
            });
        }
        let source_type = line
            .source_type
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        if let Some(source) = &source_type {
            if ![SOURCE_WORK_LOG, SOURCE_EXPENSE, SOURCE_MANUAL].contains(&source.as_str()) {
                errors.push(FieldError {
                    field: field("sourceType"),
                    message: "明細の種別が不正です".to_string(),
                });
            }
        }
        let line_note = optional_text(&mut errors, &field("note"), &line.note, MAX_NOTE_LEN);

        lines.push(NormalizedLine {
            project_id: line.project_id,
            line_no: index as i64 + 1,
            item_name: item_name.to_string(),
            quantity: line.quantity,
            unit_price: line.unit_price,
            // 行金額はサーバ側で確定させる（フロントで金額計算をしない。
            // AGENTS.md 第1章）。工数から起こす行は候補生成の時点で
            // `work_line_amount` により単価へ丸め込み済み（決定 B-2）。
            amount: line.quantity * line.unit_price,
            tax_category: line.tax_category.trim().to_string(),
            source_type,
            source_id: line.source_id,
            note: line_note,
        });
    }

    if errors.is_empty() {
        Ok(NormalizedInvoice {
            customer_id: input.customer_id,
            closing_on,
            due_on,
            corrected_invoice_id: input.corrected_invoice_id,
            note,
            lines,
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
        .column("invoiceNumber", "invoice_number")
        .column("customerId", "customer_id")
        .column("status", "status")
        .column("issuedOn", "issued_on")
        .column("closingOn", "closing_on")
        .column("dueOn", "due_on")
        .column("totalTaxable", "total_taxable")
        .column("totalTax", "total_tax")
        .column("totalAmount", "total_amount")
        .column("createdAt", "created_at")
        .column("updatedAt", "updated_at")
}

const RESOURCE: &str = "invoices";
const COLUMNS: &str = "id, invoice_number, customer_id, status, issued_on, closing_on, due_on, \
     corrected_invoice_id, total_taxable, total_tax, total_amount, rounding_mode, \
     issuer_name, issuer_registration_number, issuer_address, note, created_at, updated_at";
const LINE_COLUMNS: &str = "id, invoice_id, project_id, line_no, item_name, quantity, \
     unit_price, amount, tax_category, source_type, source_id, note";
const SUMMARY_COLUMNS: &str = "id, invoice_id, tax_category, rate_bp, taxable_amount, tax_amount";

fn today_expr(dialect: Dialect) -> &'static str {
    match dialect {
        Dialect::Sqlite => "date('now')",
        Dialect::Postgres => "CURRENT_DATE::text",
    }
}

fn not_draft_error() -> BantoError {
    BantoError::Validation {
        field_errors: vec![FieldError {
            field: "status".to_string(),
            message: "確定済み・取消済みの請求書は編集できません（訂正は取消と再発行で行います）"
                .to_string(),
        }],
    }
}

/// 明細の保存（作成・更新）を1トランザクションで行う。請求書行と明細行が
/// ばらばらに書かれると、途中で失敗したときに「明細だけ入れ替わった請求書」が
/// 残るため（`trips.rs` の一括生成と同じ理由）。
macro_rules! save_impl {
    ($fn_name:ident, $backend:ty, $dialect:expr) => {
        async fn $fn_name(
            pool: &sqlx::Pool<$backend>,
            id: Option<i64>,
            value: &NormalizedInvoice,
        ) -> Result<Invoice, BantoError> {
            let dialect = $dialect;
            let today = today_expr(dialect);
            let mut tx = pool.begin().await.map_err(banto_storage::storage_error)?;

            let invoice: Invoice = match id {
                None => {
                    let sql = format!(
                        "INSERT INTO invoices (customer_id, status, closing_on, due_on, \
                         corrected_invoice_id, rounding_mode, note, created_at, updated_at) \
                         VALUES ({}, '{STATUS_DRAFT}', {}, {}, {}, '{}', {}, {today}, {today}) \
                         RETURNING {COLUMNS}",
                        dialect.placeholder(1),
                        dialect.placeholder(2),
                        dialect.placeholder(3),
                        dialect.placeholder(4),
                        // Draft の時点では設定値をそのまま入れておくが、確定時に
                        // 改めてスナップショットし直す（F-I7）。
                        RoundingMode::default().as_code(),
                        dialect.placeholder(5),
                    );
                    sqlx::query_as(&sql)
                        .bind(value.customer_id)
                        .bind(value.closing_on.as_deref())
                        .bind(value.due_on.as_deref())
                        .bind(value.corrected_invoice_id)
                        .bind(value.note.as_deref())
                        .fetch_one(&mut *tx)
                        .await
                        .map_err(banto_storage::storage_error)?
                }
                Some(id) => {
                    let sql = format!(
                        "UPDATE invoices SET customer_id = {}, closing_on = {}, due_on = {}, \
                         corrected_invoice_id = {}, note = {}, updated_at = {today} \
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
                        .bind(value.closing_on.as_deref())
                        .bind(value.due_on.as_deref())
                        .bind(value.corrected_invoice_id)
                        .bind(value.note.as_deref())
                        .bind(id)
                        .fetch_one(&mut *tx)
                        .await
                        .map_err(|err| banto_storage::not_found(err, RESOURCE, id.to_string()))?
                }
            };

            // 明細は毎回まるごと入れ替える。行の差分更新にすると line_no の
            // 振り直しと削除の取りこぼしが絡んで壊れやすい（Draft の明細は
            // 高々 200 行なので入れ替えで足りる）。
            let delete_sql = format!(
                "DELETE FROM invoice_lines WHERE invoice_id = {}",
                dialect.placeholder(1)
            );
            sqlx::query(&delete_sql)
                .bind(invoice.id)
                .execute(&mut *tx)
                .await
                .map_err(banto_storage::storage_error)?;

            let line_sql = format!(
                "INSERT INTO invoice_lines (invoice_id, project_id, line_no, item_name, \
                 quantity, unit_price, amount, tax_category, source_type, source_id, note) \
                 VALUES ({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
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
            for line in &value.lines {
                sqlx::query(&line_sql)
                    .bind(invoice.id)
                    .bind(line.project_id)
                    .bind(line.line_no)
                    .bind(line.item_name.as_str())
                    .bind(line.quantity)
                    .bind(line.unit_price)
                    .bind(line.amount)
                    .bind(line.tax_category.as_str())
                    .bind(line.source_type.as_deref())
                    .bind(line.source_id)
                    .bind(line.note.as_deref())
                    .execute(&mut *tx)
                    .await
                    .map_err(banto_storage::storage_error)?;
            }

            tx.commit().await.map_err(banto_storage::storage_error)?;
            Ok(invoice)
        }
    };
}

save_impl!(save_sqlite, sqlx::Sqlite, Dialect::Sqlite);
#[cfg(feature = "postgres")]
save_impl!(save_postgres, sqlx::Postgres, Dialect::Postgres);

/// 確定（`issue`）を1トランザクションで行う。請求書番号の採番・税集計の保存・
/// 元データへの `invoiced` 反映が途中で切れると、番号だけ進んで明細が無い、
/// といった状態が残るため。
macro_rules! issue_impl {
    ($fn_name:ident, $backend:ty, $dialect:expr) => {
        #[allow(clippy::too_many_arguments)]
        async fn $fn_name(
            pool: &sqlx::Pool<$backend>,
            id: i64,
            invoice_number: &str,
            issued_on: &str,
            closing_on: Option<&str>,
            due_on: Option<&str>,
            issuer: &IssuerSettings,
            summary: &TaxSummary,
            sources: &[(String, i64)],
        ) -> Result<Invoice, BantoError> {
            let dialect = $dialect;
            let today = today_expr(dialect);
            let mut tx = pool.begin().await.map_err(banto_storage::storage_error)?;

            let sql = format!(
                "UPDATE invoices SET invoice_number = {}, status = '{STATUS_ISSUED}', \
                 issued_on = {}, closing_on = {}, due_on = {}, total_taxable = {}, \
                 total_tax = {}, total_amount = {}, rounding_mode = {}, issuer_name = {}, \
                 issuer_registration_number = {}, issuer_address = {}, updated_at = {today} \
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
                dialect.placeholder(12),
            );
            let invoice: Invoice = sqlx::query_as(&sql)
                .bind(invoice_number)
                .bind(issued_on)
                .bind(closing_on)
                .bind(due_on)
                .bind(summary.total_taxable)
                .bind(summary.total_tax)
                .bind(summary.total_amount)
                .bind(issuer.rounding_mode.as_code())
                .bind(issuer.name.as_deref())
                .bind(issuer.registration_number.as_deref())
                .bind(issuer.address.as_deref())
                .bind(id)
                .fetch_one(&mut *tx)
                .await
                .map_err(|err| banto_storage::not_found(err, RESOURCE, id.to_string()))?;

            let clear_sql = format!(
                "DELETE FROM invoice_tax_summaries WHERE invoice_id = {}",
                dialect.placeholder(1)
            );
            sqlx::query(&clear_sql)
                .bind(id)
                .execute(&mut *tx)
                .await
                .map_err(banto_storage::storage_error)?;

            let summary_sql = format!(
                "INSERT INTO invoice_tax_summaries (invoice_id, tax_category, rate_bp, \
                 taxable_amount, tax_amount) VALUES ({}, {}, {}, {}, {})",
                dialect.placeholder(1),
                dialect.placeholder(2),
                dialect.placeholder(3),
                dialect.placeholder(4),
                dialect.placeholder(5),
            );
            for group in &summary.groups {
                sqlx::query(&summary_sql)
                    .bind(id)
                    .bind(group.category.as_code())
                    .bind(group.rate_bp)
                    .bind(group.taxable_amount)
                    .bind(group.tax_amount)
                    .execute(&mut *tx)
                    .await
                    .map_err(banto_storage::storage_error)?;
            }

            for (source_type, source_id) in sources {
                let table = if source_type == SOURCE_WORK_LOG {
                    "work_logs"
                } else {
                    "expenses"
                };
                let mark_sql = format!(
                    "UPDATE {table} SET invoiced = 1 WHERE id = {}",
                    dialect.placeholder(1)
                );
                sqlx::query(&mark_sql)
                    .bind(source_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(banto_storage::storage_error)?;
            }

            tx.commit().await.map_err(banto_storage::storage_error)?;
            Ok(invoice)
        }
    };
}

issue_impl!(issue_sqlite, sqlx::Sqlite, Dialect::Sqlite);
#[cfg(feature = "postgres")]
issue_impl!(issue_postgres, sqlx::Postgres, Dialect::Postgres);

/// 取消（赤伝）を1トランザクションで行う。状態変更と `invoiced` の巻き戻しが
/// 割れると、請求されていないのに請求済みの工数・経費が残る。
macro_rules! cancel_impl {
    ($fn_name:ident, $backend:ty, $dialect:expr) => {
        async fn $fn_name(
            pool: &sqlx::Pool<$backend>,
            id: i64,
            sources: &[(String, i64)],
        ) -> Result<Invoice, BantoError> {
            let dialect = $dialect;
            let today = today_expr(dialect);
            let mut tx = pool.begin().await.map_err(banto_storage::storage_error)?;

            let sql = format!(
                "UPDATE invoices SET status = '{STATUS_CANCELLED}', updated_at = {today} \
                 WHERE id = {} RETURNING {COLUMNS}",
                dialect.placeholder(1)
            );
            let invoice: Invoice = sqlx::query_as(&sql)
                .bind(id)
                .fetch_one(&mut *tx)
                .await
                .map_err(|err| banto_storage::not_found(err, RESOURCE, id.to_string()))?;

            // 取消した請求書に載っていた工数・経費は「未請求」に戻す。戻さないと
            // 差し替えの請求書を候補から起こせず、請求し忘れになる（F-E2 の
            // `invoiced` は「請求書に載せたか」であり、取消はその取り消し）。
            for (source_type, source_id) in sources {
                let table = if source_type == SOURCE_WORK_LOG {
                    "work_logs"
                } else {
                    "expenses"
                };
                let unmark_sql = format!(
                    "UPDATE {table} SET invoiced = 0 WHERE id = {}",
                    dialect.placeholder(1)
                );
                sqlx::query(&unmark_sql)
                    .bind(source_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(banto_storage::storage_error)?;
            }

            tx.commit().await.map_err(banto_storage::storage_error)?;
            Ok(invoice)
        }
    };
}

cancel_impl!(cancel_sqlite, sqlx::Sqlite, Dialect::Sqlite);
#[cfg(feature = "postgres")]
cancel_impl!(cancel_postgres, sqlx::Postgres, Dialect::Postgres);

/// 候補生成で使う顧客・案件の情報。
#[derive(Debug, Clone, sqlx::FromRow)]
struct CustomerTerms {
    #[sqlx(rename = "closing_day")]
    closing_day: i64,
    #[sqlx(rename = "payment_month_offset")]
    payment_month_offset: i64,
    #[sqlx(rename = "payment_day")]
    payment_day: i64,
    name: String,
    #[sqlx(rename = "billing_name")]
    billing_name: Option<String>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct WorkCandidateRow {
    id: i64,
    #[sqlx(rename = "project_id")]
    project_id: i64,
    #[sqlx(rename = "project_code")]
    project_code: String,
    #[sqlx(rename = "project_name")]
    project_name: String,
    #[sqlx(rename = "billing_hourly_rate")]
    billing_hourly_rate: Option<i64>,
    #[sqlx(rename = "worked_on")]
    worked_on: String,
    #[sqlx(rename = "category_name")]
    category_name: Option<String>,
    minutes: i64,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct ExpenseCandidateRow {
    id: i64,
    #[sqlx(rename = "project_id")]
    project_id: i64,
    #[sqlx(rename = "project_code")]
    project_code: String,
    #[sqlx(rename = "project_name")]
    project_name: String,
    #[sqlx(rename = "spent_on")]
    spent_on: String,
    #[sqlx(rename = "category_name")]
    category_name: Option<String>,
    amount: i64,
    #[sqlx(rename = "tax_category")]
    tax_category: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct LineSource {
    #[sqlx(rename = "source_type")]
    source_type: String,
    #[sqlx(rename = "source_id")]
    source_id: i64,
}

/// 請求のサービス層（conventions §2）。
#[derive(Clone)]
pub struct InvoicesService {
    db: Db,
    issuer: IssuerService,
    events: Option<broadcast::Sender<ServerEvent>>,
}

impl InvoicesService {
    pub fn new(db: Db) -> Self {
        let issuer = IssuerService::new(SettingsService::new(db.clone()));
        Self {
            db,
            issuer,
            events: None,
        }
    }

    pub fn with_events(mut self, events: broadcast::Sender<ServerEvent>) -> Self {
        self.events = Some(events);
        self
    }

    fn notify_changed(&self) {
        if let Some(tx) = &self.events {
            for resource in ["invoices", "work_logs", "expenses"] {
                let _ = tx.send(ServerEvent::ResourceChanged {
                    resource: resource.to_string(),
                });
            }
        }
    }

    pub async fn list(&self, params: ListParams) -> Result<ListResult<Invoice>, BantoError> {
        let columns = column_map();
        let select_rows = format!("SELECT {COLUMNS} FROM invoices");
        let select_count = "SELECT COUNT(*) FROM invoices".to_string();
        match &self.db {
            Db::Sqlite(pool) => {
                let mut rows_builder: QueryBuilder<'_, Sqlite> = QueryBuilder::new(&select_rows);
                banto_storage::list_query::sqlite::apply_list_params(
                    &mut rows_builder,
                    &columns,
                    &params,
                )?;
                let rows: Vec<Invoice> = rows_builder
                    .build_query_as::<Invoice>()
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
                let rows: Vec<Invoice> = rows_builder
                    .build_query_as::<Invoice>()
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

    async fn invoice_row(&self, id: i64) -> Result<Invoice, BantoError> {
        let dialect = self.db.dialect();
        let sql = format!(
            "SELECT {COLUMNS} FROM invoices WHERE id = {}",
            dialect.placeholder(1)
        );
        match &self.db {
            Db::Sqlite(pool) => {
                sqlx::query_as::<_, Invoice>(&sql)
                    .bind(id)
                    .fetch_one(pool)
                    .await
            }
            #[cfg(feature = "postgres")]
            Db::Postgres(pool) => {
                sqlx::query_as::<_, Invoice>(&sql)
                    .bind(id)
                    .fetch_one(pool)
                    .await
            }
        }
        .map_err(|err| banto_storage::not_found(err, RESOURCE, id.to_string()))
    }

    async fn lines_of(&self, invoice_id: i64) -> Result<Vec<InvoiceLine>, BantoError> {
        let dialect = self.db.dialect();
        let sql = format!(
            "SELECT {LINE_COLUMNS} FROM invoice_lines WHERE invoice_id = {} ORDER BY line_no",
            dialect.placeholder(1)
        );
        match &self.db {
            Db::Sqlite(pool) => {
                sqlx::query_as::<_, InvoiceLine>(&sql)
                    .bind(invoice_id)
                    .fetch_all(pool)
                    .await
            }
            #[cfg(feature = "postgres")]
            Db::Postgres(pool) => {
                sqlx::query_as::<_, InvoiceLine>(&sql)
                    .bind(invoice_id)
                    .fetch_all(pool)
                    .await
            }
        }
        .map_err(banto_storage::storage_error)
    }

    async fn summaries_of(&self, invoice_id: i64) -> Result<Vec<InvoiceTaxSummary>, BantoError> {
        let dialect = self.db.dialect();
        let sql = format!(
            "SELECT {SUMMARY_COLUMNS} FROM invoice_tax_summaries WHERE invoice_id = {} \
             ORDER BY id",
            dialect.placeholder(1)
        );
        match &self.db {
            Db::Sqlite(pool) => {
                sqlx::query_as::<_, InvoiceTaxSummary>(&sql)
                    .bind(invoice_id)
                    .fetch_all(pool)
                    .await
            }
            #[cfg(feature = "postgres")]
            Db::Postgres(pool) => {
                sqlx::query_as::<_, InvoiceTaxSummary>(&sql)
                    .bind(invoice_id)
                    .fetch_all(pool)
                    .await
            }
        }
        .map_err(banto_storage::storage_error)
    }

    async fn customer_terms(&self, customer_id: i64) -> Result<CustomerTerms, BantoError> {
        let dialect = self.db.dialect();
        let sql = format!(
            "SELECT closing_day, payment_month_offset, payment_day, name, billing_name \
             FROM customers WHERE id = {}",
            dialect.placeholder(1)
        );
        let found = match &self.db {
            Db::Sqlite(pool) => {
                sqlx::query_as::<_, CustomerTerms>(&sql)
                    .bind(customer_id)
                    .fetch_optional(pool)
                    .await
            }
            #[cfg(feature = "postgres")]
            Db::Postgres(pool) => {
                sqlx::query_as::<_, CustomerTerms>(&sql)
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

    /// 明細の案件が実在し、**この請求書の顧客の案件である**ことを確かめる。
    /// 別の顧客の案件を1枚に混ぜると、案件採算の売上が別顧客へ付く。
    async fn ensure_projects_belong_to_customer(
        &self,
        customer_id: i64,
        lines: &[NormalizedLine],
    ) -> Result<(), BantoError> {
        let dialect = self.db.dialect();
        let sql = format!(
            "SELECT COUNT(*) FROM projects WHERE id = {} AND customer_id = {}",
            dialect.placeholder(1),
            dialect.placeholder(2)
        );
        for (index, line) in lines.iter().enumerate() {
            let count: i64 = match &self.db {
                Db::Sqlite(pool) => {
                    sqlx::query_scalar(&sql)
                        .bind(line.project_id)
                        .bind(customer_id)
                        .fetch_one(pool)
                        .await
                }
                #[cfg(feature = "postgres")]
                Db::Postgres(pool) => {
                    sqlx::query_scalar(&sql)
                        .bind(line.project_id)
                        .bind(customer_id)
                        .fetch_one(pool)
                        .await
                }
            }
            .map_err(banto_storage::storage_error)?;
            if count == 0 {
                return Err(BantoError::Validation {
                    field_errors: vec![FieldError {
                        field: format!("lines.{index}.projectId"),
                        message: "この顧客の案件を選択してください".to_string(),
                    }],
                });
            }
        }
        Ok(())
    }

    async fn detail(&self, invoice: Invoice) -> Result<InvoiceDetail, BantoError> {
        let terms = self.customer_terms(invoice.customer_id).await?;
        let lines = self.lines_of(invoice.id).await?;
        let tax_summaries = self.summaries_of(invoice.id).await?;
        Ok(InvoiceDetail {
            customer_name: terms.name,
            customer_billing_name: terms.billing_name,
            invoice,
            lines,
            tax_summaries,
        })
    }

    pub async fn get(&self, id: i64) -> Result<InvoiceDetail, BantoError> {
        let invoice = self.invoice_row(id).await?;
        self.detail(invoice).await
    }

    pub async fn create(&self, input: InvoiceInput) -> Result<InvoiceDetail, BantoError> {
        let value = validate(&input)?;
        self.customer_terms(value.customer_id).await?;
        self.ensure_projects_belong_to_customer(value.customer_id, &value.lines)
            .await?;
        let invoice = match &self.db {
            Db::Sqlite(pool) => save_sqlite(pool, None, &value).await?,
            #[cfg(feature = "postgres")]
            Db::Postgres(pool) => save_postgres(pool, None, &value).await?,
        };
        self.notify_changed();
        self.detail(invoice).await
    }

    pub async fn update(&self, id: i64, input: InvoiceInput) -> Result<InvoiceDetail, BantoError> {
        let current = self.invoice_row(id).await?;
        if current.status != STATUS_DRAFT {
            return Err(not_draft_error());
        }
        let value = validate(&input)?;
        self.customer_terms(value.customer_id).await?;
        self.ensure_projects_belong_to_customer(value.customer_id, &value.lines)
            .await?;
        let invoice = match &self.db {
            Db::Sqlite(pool) => save_sqlite(pool, Some(id), &value).await?,
            #[cfg(feature = "postgres")]
            Db::Postgres(pool) => save_postgres(pool, Some(id), &value).await?,
        };
        self.notify_changed();
        self.detail(invoice).await
    }

    /// Draft のみ削除できる。確定済みは削除せず取消（赤伝）で扱う — 適格請求書の
    /// 連続性のため、採番済みの番号を消してはならない（決定 C-9）。
    pub async fn delete(&self, id: i64) -> Result<(), BantoError> {
        let current = self.invoice_row(id).await?;
        if current.status != STATUS_DRAFT {
            return Err(not_draft_error());
        }
        let dialect = self.db.dialect();
        // invoice_lines / invoice_tax_summaries は ON DELETE CASCADE。
        let sql = format!("DELETE FROM invoices WHERE id = {}", dialect.placeholder(1));
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

    /// `INV-YYYY-NNNN` の次番号（決定 C-9）。年内の最大値 + 1 で、欠番を作らない
    /// ため Draft では採番しない。
    async fn next_invoice_number(&self, year: i64) -> Result<String, BantoError> {
        let prefix = format!("INV-{year:04}-");
        let dialect = self.db.dialect();
        let sql = format!(
            "SELECT invoice_number FROM invoices WHERE invoice_number LIKE {} \
             ORDER BY invoice_number DESC LIMIT 1",
            dialect.placeholder(1)
        );
        let pattern = format!("{prefix}%");
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
        .map_err(banto_storage::storage_error)?
        .flatten();
        let next = latest
            .as_deref()
            .and_then(|number| number.rsplit('-').next())
            .and_then(|serial| serial.parse::<i64>().ok())
            .unwrap_or(0)
            + 1;
        Ok(format!("{prefix}{next:04}"))
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

    async fn line_sources(&self, invoice_id: i64) -> Result<Vec<(String, i64)>, BantoError> {
        let dialect = self.db.dialect();
        let sql = format!(
            "SELECT source_type, source_id FROM invoice_lines \
             WHERE invoice_id = {} AND source_id IS NOT NULL AND source_type IS NOT NULL",
            dialect.placeholder(1)
        );
        let rows: Vec<LineSource> = match &self.db {
            Db::Sqlite(pool) => sqlx::query_as(&sql).bind(invoice_id).fetch_all(pool).await,
            #[cfg(feature = "postgres")]
            Db::Postgres(pool) => sqlx::query_as(&sql).bind(invoice_id).fetch_all(pool).await,
        }
        .map_err(banto_storage::storage_error)?;
        Ok(rows
            .into_iter()
            .filter(|row| row.source_type == SOURCE_WORK_LOG || row.source_type == SOURCE_EXPENSE)
            .map(|row| (row.source_type, row.source_id))
            .collect())
    }

    /// 確定（要件 F-I7）。番号・発行日・支払期限・端数処理方向・発行者情報・
    /// 税率区分ごとの集計をスナップショットし、元の工数・経費を請求済みにする。
    pub async fn issue(&self, id: i64) -> Result<InvoiceDetail, BantoError> {
        let current = self.invoice_row(id).await?;
        if current.status != STATUS_DRAFT {
            return Err(BantoError::Validation {
                field_errors: vec![FieldError {
                    field: "status".to_string(),
                    message: "この請求書は既に確定または取消されています".to_string(),
                }],
            });
        }
        let lines = self.lines_of(id).await?;
        if lines.is_empty() {
            return Err(BantoError::Validation {
                field_errors: vec![FieldError {
                    field: "lines".to_string(),
                    message: "明細が1行もありません".to_string(),
                }],
            });
        }

        let issuer = self.issuer.get().await?;
        let tax_lines: Vec<TaxLine> = lines
            .iter()
            .filter_map(|line| {
                TaxCategory::from_code(&line.tax_category).map(|category| TaxLine {
                    amount: line.amount,
                    category,
                })
            })
            .collect();
        let summary = calculate_tax(&tax_lines, issuer.rounding_mode);

        let issued_on = self.today().await?;
        let (year, _, _) = parse_iso_date(&issued_on).ok_or_else(|| BantoError::Validation {
            field_errors: vec![FieldError {
                field: "issuedOn".to_string(),
                message: "発行日を決定できませんでした".to_string(),
            }],
        })?;
        let invoice_number = self.next_invoice_number(year).await?;

        let terms = self.customer_terms(current.customer_id).await?;
        // 締日が未入力なら発行月の締日、支払期限が未入力なら締日から算出する
        // （決定 C-8）。手で入れてあればそちらを優先する。
        let closing_on = current
            .closing_on
            .clone()
            .or_else(|| derive_closing_date(&issued_on, terms.closing_day));
        let due_on = current.due_on.clone().or_else(|| {
            closing_on.as_deref().and_then(|closing| {
                derive_due_date(closing, terms.payment_month_offset, terms.payment_day)
            })
        });

        let sources = self.line_sources(id).await?;
        let invoice = match &self.db {
            Db::Sqlite(pool) => {
                issue_sqlite(
                    pool,
                    id,
                    &invoice_number,
                    &issued_on,
                    closing_on.as_deref(),
                    due_on.as_deref(),
                    &issuer,
                    &summary,
                    &sources,
                )
                .await?
            }
            #[cfg(feature = "postgres")]
            Db::Postgres(pool) => {
                issue_postgres(
                    pool,
                    id,
                    &invoice_number,
                    &issued_on,
                    closing_on.as_deref(),
                    due_on.as_deref(),
                    &issuer,
                    &summary,
                    &sources,
                )
                .await?
            }
        };
        self.notify_changed();
        self.detail(invoice).await
    }

    /// 取消（赤伝。決定 C-10）。確定済みのみ。差し替えの請求書は
    /// `corrected_invoice_id` にこの請求書の id を入れて新規に作る。
    pub async fn cancel(&self, id: i64) -> Result<InvoiceDetail, BantoError> {
        let current = self.invoice_row(id).await?;
        if current.status != STATUS_ISSUED {
            return Err(BantoError::Validation {
                field_errors: vec![FieldError {
                    field: "status".to_string(),
                    message: "確定済みの請求書のみ取消できます".to_string(),
                }],
            });
        }
        let sources = self.line_sources(id).await?;
        let invoice = match &self.db {
            Db::Sqlite(pool) => cancel_sqlite(pool, id, &sources).await?,
            #[cfg(feature = "postgres")]
            Db::Postgres(pool) => cancel_postgres(pool, id, &sources).await?,
        };
        self.notify_changed();
        self.detail(invoice).await
    }

    /// 未請求の工数・経費から明細候補を作る（要件 F-I1）。
    ///
    /// 期間は業務日付（工数は `worked_on`、経費は `spent_on`）で絞る。経費は
    /// **顧客請求対象（`billable`）のみ**が候補になる（F-E2）。
    pub async fn candidates(
        &self,
        query: CandidateQuery,
    ) -> Result<Vec<CandidateLine>, BantoError> {
        let mut errors = Vec::new();
        let from = optional_date(&mut errors, "from", &Some(query.from.clone()));
        let to = optional_date(&mut errors, "to", &Some(query.to.clone()));
        let (Some(from), Some(to)) = (from, to) else {
            return Err(BantoError::Validation {
                field_errors: if errors.is_empty() {
                    vec![FieldError {
                        field: "from".to_string(),
                        message: "期間を入力してください".to_string(),
                    }]
                } else {
                    errors
                },
            });
        };
        if to < from {
            return Err(BantoError::Validation {
                field_errors: vec![FieldError {
                    field: "to".to_string(),
                    message: "期間の終わりは開始以降にしてください".to_string(),
                }],
            });
        }
        self.customer_terms(query.customer_id).await?;

        let dialect = self.db.dialect();
        let work_sql = format!(
            "SELECT w.id, w.project_id, p.code AS project_code, p.name AS project_name, \
             p.billing_hourly_rate, w.worked_on, c.name AS category_name, w.minutes \
             FROM work_logs w \
             JOIN projects p ON p.id = w.project_id \
             LEFT JOIN work_categories c ON c.code = w.work_category_code \
             WHERE p.customer_id = {} AND w.deleted_at IS NULL AND w.invoiced = 0 \
             AND w.worked_on >= {} \
             AND w.worked_on <= {} ORDER BY w.project_id, w.worked_on, w.id",
            dialect.placeholder(1),
            dialect.placeholder(2),
            dialect.placeholder(3),
        );
        let expense_sql = format!(
            "SELECT e.id, e.project_id, p.code AS project_code, p.name AS project_name, \
             e.spent_on, c.name AS category_name, e.amount, e.tax_category \
             FROM expenses e \
             JOIN projects p ON p.id = e.project_id \
             LEFT JOIN expense_categories c ON c.code = e.expense_category_code \
             WHERE p.customer_id = {} AND e.deleted_at IS NULL AND e.billable = 1 \
             AND e.invoiced = 0 \
             AND e.spent_on >= {} AND e.spent_on <= {} \
             ORDER BY e.project_id, e.spent_on, e.id",
            dialect.placeholder(1),
            dialect.placeholder(2),
            dialect.placeholder(3),
        );

        let (work_rows, expense_rows) = match &self.db {
            Db::Sqlite(pool) => {
                let work: Vec<WorkCandidateRow> = sqlx::query_as(&work_sql)
                    .bind(query.customer_id)
                    .bind(&from)
                    .bind(&to)
                    .fetch_all(pool)
                    .await
                    .map_err(banto_storage::storage_error)?;
                let expenses: Vec<ExpenseCandidateRow> = sqlx::query_as(&expense_sql)
                    .bind(query.customer_id)
                    .bind(&from)
                    .bind(&to)
                    .fetch_all(pool)
                    .await
                    .map_err(banto_storage::storage_error)?;
                (work, expenses)
            }
            #[cfg(feature = "postgres")]
            Db::Postgres(pool) => {
                let work: Vec<WorkCandidateRow> = sqlx::query_as(&work_sql)
                    .bind(query.customer_id)
                    .bind(&from)
                    .bind(&to)
                    .fetch_all(pool)
                    .await
                    .map_err(banto_storage::storage_error)?;
                let expenses: Vec<ExpenseCandidateRow> = sqlx::query_as(&expense_sql)
                    .bind(query.customer_id)
                    .bind(&from)
                    .bind(&to)
                    .fetch_all(pool)
                    .await
                    .map_err(banto_storage::storage_error)?;
                (work, expenses)
            }
        };

        let mut candidates = Vec::with_capacity(work_rows.len() + expense_rows.len());
        for row in work_rows {
            // 請求単価が未設定なら金額 0 で出す（画面で単価の入力を促す）。
            // 黙って内部原価レートを使わない — 原価と売上が同じ数字になり
            // 粗利が常にゼロになる（CLAUDE.md 1.2）。
            let amount = row
                .billing_hourly_rate
                .map(|rate| work_line_amount(row.minutes, rate))
                .unwrap_or(0);
            let rate_note = match row.billing_hourly_rate {
                Some(rate) => format!(
                    "{} {} × {}円/時",
                    row.worked_on,
                    format_minutes(row.minutes),
                    rate
                ),
                None => format!(
                    "{} {}（請求単価が未設定です）",
                    row.worked_on,
                    format_minutes(row.minutes)
                ),
            };
            candidates.push(CandidateLine {
                project_id: row.project_id,
                project_code: row.project_code,
                project_name: row.project_name,
                source_type: SOURCE_WORK_LOG.to_string(),
                source_id: row.id,
                item_name: row.category_name.unwrap_or_else(|| "作業".to_string()),
                quantity: 1,
                unit_price: amount,
                amount,
                tax_category: TaxCategory::Standard10.as_code().to_string(),
                note: rate_note,
                billing_hourly_rate: row.billing_hourly_rate,
                minutes: Some(row.minutes),
            });
        }
        for row in expense_rows {
            // 立替経費の再請求は**税抜換算額**で起こす。原価も税抜で計上して
            // いる（要件 F-P8）ので、実費請求なら粗利影響がゼロになる
            // （決定 C-4）。請求側の税区分は一律 10%（決定 B-5）。
            let amount = taxable_amount(row.amount, &row.tax_category);
            candidates.push(CandidateLine {
                project_id: row.project_id,
                project_code: row.project_code,
                project_name: row.project_name,
                source_type: SOURCE_EXPENSE.to_string(),
                source_id: row.id,
                item_name: row.category_name.unwrap_or_else(|| "経費".to_string()),
                quantity: 1,
                unit_price: amount,
                amount,
                tax_category: TaxCategory::Standard10.as_code().to_string(),
                note: format!("{} 立替（税込 {}円）", row.spent_on, row.amount),
                billing_hourly_rate: None,
                minutes: None,
            });
        }
        Ok(candidates)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::customers::{CustomerInput, CustomersService, DAY_END_OF_MONTH};
    use crate::db::migrate_memory;
    use crate::expenses::{ExpenseInput, ExpensesService};
    use crate::issuer::IssuerInput;
    use crate::masters::{CostRateInput, MastersService};
    use crate::projects::{ProjectInput, ProjectsService};
    use crate::work_logs::{WorkLogInput, WorkLogsService};

    struct Fixture {
        invoices: InvoicesService,
        issuer: IssuerService,
        work_logs: WorkLogsService,
        expenses: ExpensesService,
        customer_id: i64,
        project_id: i64,
    }

    /// 月末締め・翌月末払いの顧客1件と、請求時間単価 10,000円/時 の案件1件。
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
                billing_name: Some("架空商事株式会社".to_string()),
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
        let masters = MastersService::new(pool.clone());
        masters
            .set_cost_rate(CostRateInput {
                work_category_code: "DESIGN".to_string(),
                hourly_rate: 6_000,
            })
            .await
            .expect("cost rate");
        Fixture {
            invoices: InvoicesService::new(pool.clone()),
            issuer: IssuerService::new(SettingsService::new(pool.clone())),
            work_logs: WorkLogsService::new(pool.clone()),
            expenses: ExpensesService::new(pool.clone()),
            customer_id: customer.id,
            project_id: project.id,
        }
    }

    fn line(project_id: i64, unit_price: i64) -> InvoiceLineInput {
        InvoiceLineInput {
            project_id,
            item_name: "設計".to_string(),
            quantity: 1,
            unit_price,
            tax_category: "STANDARD_10".to_string(),
            source_type: None,
            source_id: None,
            note: None,
        }
    }

    fn draft(customer_id: i64, lines: Vec<InvoiceLineInput>) -> InvoiceInput {
        InvoiceInput {
            customer_id,
            closing_on: None,
            due_on: None,
            corrected_invoice_id: None,
            note: None,
            lines,
        }
    }

    #[test]
    fn derives_due_dates_from_closing_and_payment_terms() {
        // 月末締め・翌月末払い
        assert_eq!(
            derive_due_date("2026-08-31", 1, DAY_END_OF_MONTH),
            Some("2026-09-30".to_string())
        );
        // 20日締め・翌々月10日払い
        assert_eq!(
            derive_due_date("2026-08-20", 2, 10),
            Some("2026-10-10".to_string())
        );
        // 年をまたぐ
        assert_eq!(
            derive_due_date("2026-12-31", 1, DAY_END_OF_MONTH),
            Some("2027-01-31".to_string())
        );
        // 月末払いが2月に落ちる（閏年でない年）
        assert_eq!(
            derive_due_date("2027-01-31", 1, DAY_END_OF_MONTH),
            Some("2027-02-28".to_string())
        );
    }

    #[test]
    fn derives_the_closing_date_from_the_issue_month() {
        assert_eq!(
            derive_closing_date("2026-08-20", DAY_END_OF_MONTH),
            Some("2026-08-31".to_string())
        );
        assert_eq!(
            derive_closing_date("2026-08-20", 20),
            Some("2026-08-20".to_string())
        );
    }

    /// 工数から起こす行金額は `floor(分 × 時間単価 ÷ 60)`（決定 B-2）。
    #[test]
    fn work_line_amount_floors_like_internal_cost() {
        // 7分 × 10,000円/時 = 1,166.66… → 1,166
        assert_eq!(work_line_amount(7, 10_000), 1_166);
        assert_eq!(work_line_amount(60, 10_000), 10_000);
        assert_eq!(work_line_amount(90, 10_000), 15_000);
    }

    #[tokio::test]
    async fn a_new_invoice_is_a_draft_without_a_number() {
        let f = fixture().await;
        let created = f
            .invoices
            .create(draft(f.customer_id, vec![line(f.project_id, 100_000)]))
            .await
            .expect("create");
        assert_eq!(created.invoice.status, STATUS_DRAFT);
        // 欠番を作らないため Draft では採番しない（決定 C-9）。
        assert_eq!(created.invoice.invoice_number, None);
        assert_eq!(created.lines.len(), 1);
        assert_eq!(created.lines[0].amount, 100_000);
        assert_eq!(created.customer_name, "架空商事");
        // 税集計は確定時に作る。
        assert!(created.tax_summaries.is_empty());
    }

    /// 行金額はサーバ側で `数量 × 単価` から確定させる（フロントで計算しない）。
    #[tokio::test]
    async fn line_amount_is_computed_from_quantity_and_unit_price() {
        let f = fixture().await;
        let mut input = line(f.project_id, 33_333);
        input.quantity = 3;
        let created = f
            .invoices
            .create(draft(f.customer_id, vec![input]))
            .await
            .expect("create");
        assert_eq!(created.lines[0].amount, 99_999);
    }

    #[tokio::test]
    async fn issuing_snapshots_number_totals_and_tax_summaries() {
        let f = fixture().await;
        f.issuer
            .set(IssuerInput {
                name: Some("架空設計事務所".to_string()),
                registration_number: Some("T1234567890123".to_string()),
                address: Some("架空県架空市1-2-3".to_string()),
                bank_account: None,
                rounding_mode: "FLOOR".to_string(),
            })
            .await
            .expect("issuer");

        let created = f
            .invoices
            .create(draft(
                f.customer_id,
                vec![line(f.project_id, 33_333), line(f.project_id, 33_333)],
            ))
            .await
            .expect("create");
        let issued = f.invoices.issue(created.invoice.id).await.expect("issue");

        assert_eq!(issued.invoice.status, STATUS_ISSUED);
        let number = issued.invoice.invoice_number.expect("number");
        assert!(number.ends_with("-0001"), "{number}");
        assert!(number.starts_with("INV-"), "{number}");
        // 税率区分ごとに1回だけ端数処理する（CLAUDE.md 1.7）。
        // 66,666 × 10% = 6,666.6 → 6,666。行ごとなら 3,333 × 2 = 6,666 で
        // 同じだが、合計・区分の保存が正しいことをここで固定する。
        assert_eq!(issued.invoice.total_taxable, 66_666);
        assert_eq!(issued.invoice.total_tax, 6_666);
        assert_eq!(issued.invoice.total_amount, 73_332);
        assert_eq!(issued.tax_summaries.len(), 1);
        assert_eq!(issued.tax_summaries[0].rate_bp, 1_000);
        assert_eq!(issued.tax_summaries[0].taxable_amount, 66_666);
        // 発行者情報のスナップショット（要件 F-I7）。
        assert_eq!(
            issued.invoice.issuer_name.as_deref(),
            Some("架空設計事務所")
        );
        assert_eq!(
            issued.invoice.issuer_registration_number.as_deref(),
            Some("T1234567890123")
        );
        assert_eq!(issued.invoice.rounding_mode, "FLOOR");
        // 締日・支払期限は顧客マスタから算出して保存する（決定 C-8）。
        let closing = issued.invoice.closing_on.expect("closing");
        let due = issued.invoice.due_on.expect("due");
        assert_eq!(
            derive_due_date(&closing, 1, DAY_END_OF_MONTH).as_deref(),
            Some(due.as_str())
        );
    }

    /// 明細行ごとに端数処理した場合と結果が変わるケース（T-08 と同じ趣旨）を
    /// 請求書の確定経由でも固定する。
    #[tokio::test]
    async fn issuing_rounds_once_per_tax_category() {
        let f = fixture().await;
        let created = f
            .invoices
            .create(draft(
                f.customer_id,
                vec![
                    line(f.project_id, 33_335),
                    line(f.project_id, 33_335),
                    line(f.project_id, 33_335),
                ],
            ))
            .await
            .expect("create");
        let issued = f.invoices.issue(created.invoice.id).await.expect("issue");
        // 区分ごと: floor(100,005 × 10%) = 10,000（行ごとなら 9,999）
        assert_eq!(issued.invoice.total_tax, 10_000);
    }

    #[tokio::test]
    async fn invoice_numbers_increment_within_the_year() {
        let f = fixture().await;
        let mut numbers = Vec::new();
        for _ in 0..2 {
            let created = f
                .invoices
                .create(draft(f.customer_id, vec![line(f.project_id, 10_000)]))
                .await
                .expect("create");
            let issued = f.invoices.issue(created.invoice.id).await.expect("issue");
            numbers.push(issued.invoice.invoice_number.expect("number"));
        }
        assert!(numbers[0].ends_with("-0001"), "{numbers:?}");
        assert!(numbers[1].ends_with("-0002"), "{numbers:?}");
    }

    /// T-12: 端数処理方向は確定時にスナップショットされ、設定を変えても
    /// 既発行の請求書の税額は変わらない。
    #[tokio::test]
    async fn changing_the_rounding_setting_does_not_move_issued_invoices() {
        let f = fixture().await;
        let created = f
            .invoices
            .create(draft(f.customer_id, vec![line(f.project_id, 33_335)]))
            .await
            .expect("create");
        let issued = f.invoices.issue(created.invoice.id).await.expect("issue");
        assert_eq!(issued.invoice.total_tax, 3_333);

        f.issuer
            .set(IssuerInput {
                name: None,
                registration_number: None,
                address: None,
                bank_account: None,
                rounding_mode: "ROUND".to_string(),
            })
            .await
            .expect("issuer");

        let reloaded = f.invoices.get(issued.invoice.id).await.expect("get");
        assert_eq!(reloaded.invoice.total_tax, 3_333);
        assert_eq!(reloaded.invoice.rounding_mode, "FLOOR");

        // 新しい請求書には新しい設定が効く。
        let next = f
            .invoices
            .create(draft(f.customer_id, vec![line(f.project_id, 33_335)]))
            .await
            .expect("create");
        let next_issued = f.invoices.issue(next.invoice.id).await.expect("issue");
        assert_eq!(next_issued.invoice.total_tax, 3_334);
        assert_eq!(next_issued.invoice.rounding_mode, "ROUND");
    }

    /// F-I8: 確定後は明細を編集できない。
    #[tokio::test]
    async fn issued_invoices_cannot_be_edited_or_deleted() {
        let f = fixture().await;
        let created = f
            .invoices
            .create(draft(f.customer_id, vec![line(f.project_id, 10_000)]))
            .await
            .expect("create");
        let id = created.invoice.id;
        f.invoices.issue(id).await.expect("issue");

        let err = f
            .invoices
            .update(id, draft(f.customer_id, vec![line(f.project_id, 20_000)]))
            .await
            .unwrap_err();
        assert!(matches!(err, BantoError::Validation { .. }), "{err:?}");
        let err = f.invoices.delete(id).await.unwrap_err();
        assert!(matches!(err, BantoError::Validation { .. }), "{err:?}");
        // 二重確定もできない。
        let err = f.invoices.issue(id).await.unwrap_err();
        assert!(matches!(err, BantoError::Validation { .. }), "{err:?}");
    }

    #[tokio::test]
    async fn an_empty_invoice_cannot_be_issued() {
        let f = fixture().await;
        let created = f
            .invoices
            .create(draft(f.customer_id, vec![]))
            .await
            .expect("create");
        let err = f.invoices.issue(created.invoice.id).await.unwrap_err();
        assert!(matches!(err, BantoError::Validation { .. }), "{err:?}");
    }

    #[tokio::test]
    async fn drafts_can_be_edited_and_deleted() {
        let f = fixture().await;
        let created = f
            .invoices
            .create(draft(f.customer_id, vec![line(f.project_id, 10_000)]))
            .await
            .expect("create");
        let updated = f
            .invoices
            .update(
                created.invoice.id,
                draft(
                    f.customer_id,
                    vec![line(f.project_id, 20_000), line(f.project_id, 5_000)],
                ),
            )
            .await
            .expect("update");
        assert_eq!(updated.lines.len(), 2);
        assert_eq!(updated.lines[0].line_no, 1);
        assert_eq!(updated.lines[1].line_no, 2);
        f.invoices.delete(created.invoice.id).await.expect("delete");
        assert!(f.invoices.get(created.invoice.id).await.is_err());
    }

    /// 別の顧客の案件を明細に混ぜられない（案件採算の売上が別顧客へ付くため）。
    #[tokio::test]
    async fn lines_must_reference_a_project_of_the_same_customer() {
        let f = fixture().await;
        let err = f
            .invoices
            .create(draft(f.customer_id, vec![line(f.project_id + 999, 10_000)]))
            .await
            .unwrap_err();
        match err {
            BantoError::Validation { field_errors } => {
                assert_eq!(field_errors[0].field, "lines.0.projectId");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    /// **論理削除が請求候補へ波及しないこと**（`docs/domain/sync.md` 5節）。
    ///
    /// ここが漏れると、**消したはずの工数・経費が取引先への請求書に載る**。
    /// 候補の段階で気付ければよいが、まとめて選ぶ操作なので見落としやすい。
    #[tokio::test]
    async fn deleted_rows_are_not_offered_as_invoice_candidates() {
        let f = fixture().await;
        let doomed_work = f
            .work_logs
            .create(WorkLogInput {
                project_id: f.project_id,
                trip_id: None,
                worked_on: "2026-08-20".to_string(),
                work_category_code: "DESIGN".to_string(),
                minutes: 90,
                applied_rate: None,
                description: None,
                invoiced: false,
            })
            .await
            .expect("work log");
        let doomed_expense = f
            .expenses
            .create(ExpenseInput {
                project_id: f.project_id,
                trip_id: None,
                spent_on: "2026-08-21".to_string(),
                expense_category_code: "TRANSPORT".to_string(),
                payee: None,
                amount: 11_000,
                tax_category: Some("STANDARD_10".to_string()),
                description: None,
                billable: true,
                invoiced: false,
            })
            .await
            .expect("billable expense");

        let query = CandidateQuery {
            customer_id: f.customer_id,
            from: "2026-08-01".to_string(),
            to: "2026-08-31".to_string(),
        };
        assert_eq!(
            f.invoices
                .candidates(query.clone())
                .await
                .expect("before")
                .len(),
            2
        );

        f.work_logs
            .delete(doomed_work.id)
            .await
            .expect("delete 工数");
        f.expenses
            .delete(doomed_expense.id)
            .await
            .expect("delete 経費");

        assert!(
            f.invoices
                .candidates(query)
                .await
                .expect("after")
                .is_empty(),
            "削除した行が請求候補に残っている"
        );
    }

    #[tokio::test]
    async fn candidates_list_uninvoiced_work_and_billable_expenses() {
        let f = fixture().await;
        f.work_logs
            .create(WorkLogInput {
                project_id: f.project_id,
                trip_id: None,
                worked_on: "2026-08-20".to_string(),
                work_category_code: "DESIGN".to_string(),
                minutes: 90,
                applied_rate: None,
                description: None,
                invoiced: false,
            })
            .await
            .expect("work log");
        // 請求対象の経費（税込 11,000 → 税抜 10,000）
        f.expenses
            .create(ExpenseInput {
                project_id: f.project_id,
                trip_id: None,
                spent_on: "2026-08-21".to_string(),
                expense_category_code: "TRANSPORT".to_string(),
                payee: None,
                amount: 11_000,
                tax_category: Some("STANDARD_10".to_string()),
                description: None,
                billable: true,
                invoiced: false,
            })
            .await
            .expect("billable expense");
        // 請求対象でない経費は候補に出ない。
        f.expenses
            .create(ExpenseInput {
                project_id: f.project_id,
                trip_id: None,
                spent_on: "2026-08-21".to_string(),
                expense_category_code: "TRANSPORT".to_string(),
                payee: None,
                amount: 3_300,
                tax_category: Some("STANDARD_10".to_string()),
                description: None,
                billable: false,
                invoiced: false,
            })
            .await
            .expect("internal expense");

        let candidates = f
            .invoices
            .candidates(CandidateQuery {
                customer_id: f.customer_id,
                from: "2026-08-01".to_string(),
                to: "2026-08-31".to_string(),
            })
            .await
            .expect("candidates");
        assert_eq!(candidates.len(), 2);

        let work = candidates
            .iter()
            .find(|c| c.source_type == SOURCE_WORK_LOG)
            .expect("work candidate");
        // 90分 × 10,000円/時 = 15,000（内部原価 6,000円/時 は使わない）
        assert_eq!(work.amount, 15_000);
        assert_eq!(work.billing_hourly_rate, Some(10_000));
        assert_eq!(work.minutes, Some(90));

        let expense = candidates
            .iter()
            .find(|c| c.source_type == SOURCE_EXPENSE)
            .expect("expense candidate");
        // 税抜換算（要件 F-P8 と同じ額）。請求側の税区分は一律 10%（決定 B-5）。
        assert_eq!(expense.amount, 10_000);
        assert_eq!(expense.tax_category, "STANDARD_10");

        // 期間外は出ない。
        let outside = f
            .invoices
            .candidates(CandidateQuery {
                customer_id: f.customer_id,
                from: "2026-09-01".to_string(),
                to: "2026-09-30".to_string(),
            })
            .await
            .expect("candidates");
        assert!(outside.is_empty());
    }

    /// 確定で元の工数・経費が請求済みになり、取消で未請求へ戻る。
    #[tokio::test]
    async fn issuing_and_cancelling_move_the_invoiced_flag() {
        let f = fixture().await;
        let work_log = f
            .work_logs
            .create(WorkLogInput {
                project_id: f.project_id,
                trip_id: None,
                worked_on: "2026-08-20".to_string(),
                work_category_code: "DESIGN".to_string(),
                minutes: 60,
                applied_rate: None,
                description: None,
                invoiced: false,
            })
            .await
            .expect("work log");

        let created = f
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
                    source_type: Some(SOURCE_WORK_LOG.to_string()),
                    source_id: Some(work_log.id),
                    note: None,
                }],
            })
            .await
            .expect("create");

        // Draft の間は未請求のまま（確定して初めて請求済みになる）。
        assert_eq!(f.work_logs.get(work_log.id).await.expect("get").invoiced, 0);

        let issued = f.invoices.issue(created.invoice.id).await.expect("issue");
        assert_eq!(f.work_logs.get(work_log.id).await.expect("get").invoiced, 1);
        // 請求済みになったので候補には出ない。
        let candidates = f
            .invoices
            .candidates(CandidateQuery {
                customer_id: f.customer_id,
                from: "2026-08-01".to_string(),
                to: "2026-08-31".to_string(),
            })
            .await
            .expect("candidates");
        assert!(candidates.is_empty());

        let cancelled = f.invoices.cancel(issued.invoice.id).await.expect("cancel");
        assert_eq!(cancelled.invoice.status, STATUS_CANCELLED);
        // 取消で未請求へ戻り、差し替えの請求書を候補から起こせる。
        assert_eq!(f.work_logs.get(work_log.id).await.expect("get").invoiced, 0);
        let after_cancel = f
            .invoices
            .candidates(CandidateQuery {
                customer_id: f.customer_id,
                from: "2026-08-01".to_string(),
                to: "2026-08-31".to_string(),
            })
            .await
            .expect("candidates");
        assert_eq!(after_cancel.len(), 1);
    }

    /// 赤伝は元請求書を指す（決定 C-10）。
    #[tokio::test]
    async fn a_replacement_invoice_points_at_the_cancelled_one() {
        let f = fixture().await;
        let created = f
            .invoices
            .create(draft(f.customer_id, vec![line(f.project_id, 10_000)]))
            .await
            .expect("create");
        let issued = f.invoices.issue(created.invoice.id).await.expect("issue");
        f.invoices.cancel(issued.invoice.id).await.expect("cancel");

        let mut replacement = draft(f.customer_id, vec![line(f.project_id, 12_000)]);
        replacement.corrected_invoice_id = Some(issued.invoice.id);
        let created = f.invoices.create(replacement).await.expect("create");
        assert_eq!(
            created.invoice.corrected_invoice_id,
            Some(issued.invoice.id)
        );
    }

    #[tokio::test]
    async fn only_issued_invoices_can_be_cancelled() {
        let f = fixture().await;
        let created = f
            .invoices
            .create(draft(f.customer_id, vec![line(f.project_id, 10_000)]))
            .await
            .expect("create");
        let err = f.invoices.cancel(created.invoice.id).await.unwrap_err();
        assert!(matches!(err, BantoError::Validation { .. }), "{err:?}");
    }

    #[tokio::test]
    async fn unknown_invoice_is_not_found() {
        let f = fixture().await;
        let err = f.invoices.get(999).await.unwrap_err();
        assert!(matches!(err, BantoError::NotFound { .. }), "{err:?}");
    }

    #[tokio::test]
    async fn rejects_invalid_lines() {
        let f = fixture().await;
        let mut bad = line(f.project_id, 10_000);
        bad.tax_category = "NOPE".to_string();
        let err = f
            .invoices
            .create(draft(f.customer_id, vec![bad]))
            .await
            .unwrap_err();
        match err {
            BantoError::Validation { field_errors } => {
                assert_eq!(field_errors[0].field, "lines.0.taxCategory");
            }
            other => panic!("unexpected: {other:?}"),
        }

        let mut blank = line(f.project_id, 10_000);
        blank.item_name = "  ".to_string();
        assert!(f
            .invoices
            .create(draft(f.customer_id, vec![blank]))
            .await
            .is_err());
    }

    /// 値引き行（マイナス単価）を許す（決定 B-3 / 要件 F-I4）。
    #[tokio::test]
    async fn discount_lines_are_accepted() {
        let f = fixture().await;
        let mut discount = line(f.project_id, -33_335);
        discount.item_name = "値引き".to_string();
        let created = f
            .invoices
            .create(draft(
                f.customer_id,
                vec![line(f.project_id, 100_000), discount],
            ))
            .await
            .expect("create");
        let issued = f.invoices.issue(created.invoice.id).await.expect("issue");
        assert_eq!(issued.invoice.total_taxable, 66_665);
        // 6,666.5 → 6,666（ゼロ方向切捨て）
        assert_eq!(issued.invoice.total_tax, 6_666);
    }
}
