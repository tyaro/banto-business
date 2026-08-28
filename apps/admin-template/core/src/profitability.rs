//! Phase 4: 案件採算（要件 F-P1〜F-P7）。conventions §2 に従い `tauri` /
//! `axum` / RBAC を知らない。
//!
//! ## 保持しない
//!
//! 採算値は列として持たず、常に `work_logs` / `expenses` / 確定済
//! `invoice_lines` から導出する（F-P7）。持つと、行を1件直したときに
//! 再計算漏れで数値が食い違う。
//!
//! ## 丸めの方向と桁（CLAUDE.md 1.1）
//!
//! | 値 | 式 | 丸め |
//! |---|---|---|
//! | 工数原価 | `work_logs.internal_cost` の合計 | 行ごとに確定済（丸めない） |
//! | 経費原価 | 行ごとに税抜換算した額の合計 | **行ごとに1円未満を切捨て** |
//! | 実質時間単価 | `粗利 × 60 ÷ 分` | 1円未満を切捨て（マイナスはゼロ方向） |
//! | 粗利率・請求進捗 | `分子 × 10000 ÷ 分母` | basis point 未満を切捨て（同上） |
//!
//! 「行ごとに丸めた額を合計する」であり「合計してから丸める」ではない
//! （Phase 1 決定 C-1 と同じ方針）。i64 の整数演算のみで計算し、浮動小数点を
//! 経由しない。
//!
//! ## 経費の税抜換算（決定 2026-08-20）
//!
//! `expenses.amount` は**税込の実支出**だが、案件採算は税抜で集計する
//! （Phase 1 決定 A-3：消費税は預り金／仮払いであり採算に含めない）。
//! そのため行ごとに仕入側の税区分で税抜へ換算する。

use crate::expenses::TAX_CATEGORIES;
use banto_core::BantoError;
use banto_storage::Db;
use serde::{Deserialize, Serialize};

const RESOURCE: &str = "profitability";

/// 案件1件の採算。
///
/// **実質時間単価は必ず2種を同時に返す**（F-P2）。移動を分母に入れるか
/// どうかで数値が大きく変わり、片方だけを見ると受注可否の判断を誤るため、
/// 片方だけを返す入口を作らない。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectProfitability {
    /// DataProvider の `getOne` が id を要求するため、案件 id をそのまま
    /// このリソースの id として返す（案件と 1:1）。
    pub id: i64,
    pub project_id: i64,
    pub project_code: String,
    pub project_name: String,
    pub status: String,
    /// 採算集計の対象か（`ProjectStatus::counts_toward_profitability`）。
    /// 失注案件でも数値は出すが、集計対象でないことを画面に伝える。
    pub counts_toward_profitability: bool,

    /// 契約額（税抜・円）。**粗利計算には使わない**（Phase 1 決定 C-3）。
    /// 請求進捗の分母としてのみ使う。
    pub contract_amount: Option<i64>,
    /// 案件売上 = 確定済 InvoiceLine の税抜合計（F-P4）。
    pub revenue: i64,

    /// 工数原価 = `internal_cost` の合計。
    pub work_cost: i64,
    /// 直接経費 = 税抜換算した経費の合計。
    pub expense_cost: i64,
    /// うち顧客請求対象（F-P6 の両建て分。請求されれば売上にも立つ）。
    pub billable_expense_cost: i64,
    /// うち顧客請求対象かつ未請求。請求し忘れの検出用（F-E2 の2フラグ）。
    pub uninvoiced_billable_expense_cost: i64,
    pub total_cost: i64,

    /// 粗利 = 売上 − 原価。売上は確定済の請求のみなので、未請求の案件では
    /// 原価のぶんマイナスになる。誤読を防ぐため請求進捗を併記する（F-P5）。
    pub gross_profit: i64,
    /// 粗利率（basis point = 1/10000）。売上が 0 なら `None`。
    pub gross_margin_bp: Option<i64>,
    /// 請求進捗（案件売上 ÷ 契約額、basis point）。契約額が未設定なら `None`。
    pub invoice_progress_bp: Option<i64>,

    /// 総投入時間（分）。
    pub total_minutes: i64,
    /// うち実質時間単価（移動除く）の分母から外す分。判定は作業分類の
    /// `excluded_from_effective_rate` フラグで行い、コード文字列を比較しない
    /// （F-P3）。
    pub excluded_minutes: i64,
    /// 実質時間単価（移動込み、円/時）。総投入時間が 0 なら `None`。
    pub effective_rate_including_travel: Option<i64>,
    /// 実質時間単価（移動除く、円/時）。分母が 0 なら `None`。
    pub effective_rate_excluding_travel: Option<i64>,
}

/// 税抜への換算（1円未満切捨て、マイナスはゼロ方向）。
///
/// 仕入側の税区分（`docs/tax-calculation.md` 3）ごとに分母が変わる。
/// 非課税（`EXEMPT`）・不課税（`OUT_OF_SCOPE`）は消費税を含まないので
/// 支払額がそのまま税抜額になる。未知のコードも換算しない — 書き込み時に
/// [`TAX_CATEGORIES`] で検証済みであり、ここで勝手な分母を当てるより
/// 換算しないほうが安全（実支出を下回らない）。
pub fn taxable_amount(amount: i64, tax_category: &str) -> i64 {
    debug_assert!(
        TAX_CATEGORIES.contains(&tax_category),
        "unknown tax category: {tax_category}"
    );
    match tax_category {
        "STANDARD_10" => amount * 100 / 110,
        "REDUCED_8" => amount * 100 / 108,
        _ => amount,
    }
}

/// 実質時間単価（円/時）。分母が 0 以下なら `None`。
///
/// 「粗利 ÷ 総投入時間」を分のまま計算する（`粗利 × 60 ÷ 分`）。時間へ
/// 変換してから割ると、変換の時点で丸めが1回増える。
pub fn effective_hourly_rate(gross_profit: i64, minutes: i64) -> Option<i64> {
    if minutes <= 0 {
        return None;
    }
    Some(gross_profit * 60 / minutes)
}

/// 比率を basis point（1/10000）で返す。分母が 0 以下なら `None`。
/// 率を小数で持たない（CLAUDE.md 1.1）。
pub fn ratio_bp(numerator: i64, denominator: i64) -> Option<i64> {
    if denominator <= 0 {
        return None;
    }
    Some(numerator * 10_000 / denominator)
}

/// 工数側の集計結果。金額の丸めは `internal_cost` の時点で済んでいるので、
/// ここは純粋な合計であり SQL に任せてよい（経費と違い丸めの判断が無い）。
#[derive(Debug, Clone, sqlx::FromRow)]
struct WorkTotals {
    #[sqlx(rename = "work_cost")]
    work_cost: i64,
    #[sqlx(rename = "total_minutes")]
    total_minutes: i64,
    #[sqlx(rename = "excluded_minutes")]
    excluded_minutes: i64,
}

/// 経費1行のうち採算に必要な列。税抜換算は行ごとの丸めを伴う金額ロジック
/// なので、SQL ではなく Rust 側で行う（[`taxable_amount`] を単体でテスト
/// できるようにするため）。
#[derive(Debug, Clone, sqlx::FromRow)]
struct ExpenseRow {
    amount: i64,
    #[sqlx(rename = "tax_category")]
    tax_category: String,
    billable: i64,
    invoiced: i64,
}

/// 案件の識別情報。採算のためだけに `Project` 全体を読む必要は無い。
#[derive(Debug, Clone, sqlx::FromRow)]
struct ProjectHeader {
    id: i64,
    code: String,
    name: String,
    status: String,
    #[sqlx(rename = "contract_amount")]
    contract_amount: Option<i64>,
}

/// `SUM` の結果を `CAST(... AS BIGINT)` で包むのは PostgreSQL 対策。
/// PostgreSQL の `SUM(bigint)` は `numeric` を返すため、そのままでは i64 に
/// デコードできない。SQLite では BIGINT が INTEGER 親和性を持つので同じ
/// SQL がそのまま通る（conventions §11 の2方言対応）。
const WORK_TOTALS_SQL: &str = "SELECT \
     CAST(COALESCE(SUM(w.internal_cost), 0) AS BIGINT) AS work_cost, \
     CAST(COALESCE(SUM(w.minutes), 0) AS BIGINT) AS total_minutes, \
     CAST(COALESCE(SUM(CASE WHEN c.excluded_from_effective_rate = 1 \
         THEN w.minutes ELSE 0 END), 0) AS BIGINT) AS excluded_minutes \
     FROM work_logs w \
     LEFT JOIN work_categories c ON c.code = w.work_category_code \
     WHERE w.deleted_at IS NULL AND w.project_id = ";

/// 採算のサービス層（conventions §2）。読み取り専用 — 採算値を保持しない
/// （F-P7）ので、書き込みの入口もイベント通知も持たない。
#[derive(Clone)]
pub struct ProfitabilityService {
    db: Db,
}

impl ProfitabilityService {
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    /// 案件売上 = **確定済（ISSUED）** InvoiceLine の税抜合計（F-P4）。
    ///
    /// Draft は売上に立てない（まだ請求していない）。取消（CANCELLED）も
    /// 除く — 赤伝で差し替えた分が二重に計上されないようにするため。
    /// 未請求の作業・経費も売上に立たない（F-P4 の決定）ので、請求前の案件は
    /// 売上 0 のまま原価だけが立つ。誤読を防ぐのが請求進捗の併記（F-P5）。
    async fn revenue_for(&self, project_id: i64) -> Result<i64, BantoError> {
        let dialect = self.db.dialect();
        // `CAST(... AS BIGINT)` は PostgreSQL の `SUM(bigint)` が numeric を
        // 返すため（`WORK_TOTALS_SQL` と同じ理由）。
        let sql = format!(
            "SELECT CAST(COALESCE(SUM(l.amount), 0) AS BIGINT) \
             FROM invoice_lines l JOIN invoices i ON i.id = l.invoice_id \
             WHERE l.project_id = {} AND i.status = 'ISSUED'",
            dialect.placeholder(1)
        );
        match &self.db {
            Db::Sqlite(pool) => {
                sqlx::query_scalar::<_, i64>(&sql)
                    .bind(project_id)
                    .fetch_one(pool)
                    .await
            }
            #[cfg(feature = "postgres")]
            Db::Postgres(pool) => {
                sqlx::query_scalar::<_, i64>(&sql)
                    .bind(project_id)
                    .fetch_one(pool)
                    .await
            }
        }
        .map_err(banto_storage::storage_error)
    }

    async fn project_header(&self, project_id: i64) -> Result<ProjectHeader, BantoError> {
        let dialect = self.db.dialect();
        let sql = format!(
            "SELECT id, code, name, status, contract_amount FROM projects \
             WHERE id = {} AND deleted_at IS NULL",
            dialect.placeholder(1)
        );
        match &self.db {
            Db::Sqlite(pool) => {
                sqlx::query_as::<_, ProjectHeader>(&sql)
                    .bind(project_id)
                    .fetch_one(pool)
                    .await
            }
            #[cfg(feature = "postgres")]
            Db::Postgres(pool) => {
                sqlx::query_as::<_, ProjectHeader>(&sql)
                    .bind(project_id)
                    .fetch_one(pool)
                    .await
            }
        }
        .map_err(|err| banto_storage::not_found(err, RESOURCE, project_id.to_string()))
    }

    async fn work_totals(&self, project_id: i64) -> Result<WorkTotals, BantoError> {
        let dialect = self.db.dialect();
        let sql = format!("{WORK_TOTALS_SQL}{}", dialect.placeholder(1));
        match &self.db {
            Db::Sqlite(pool) => {
                sqlx::query_as::<_, WorkTotals>(&sql)
                    .bind(project_id)
                    .fetch_one(pool)
                    .await
            }
            #[cfg(feature = "postgres")]
            Db::Postgres(pool) => {
                sqlx::query_as::<_, WorkTotals>(&sql)
                    .bind(project_id)
                    .fetch_one(pool)
                    .await
            }
        }
        .map_err(banto_storage::storage_error)
    }

    async fn expense_rows(&self, project_id: i64) -> Result<Vec<ExpenseRow>, BantoError> {
        let dialect = self.db.dialect();
        let sql = format!(
            "SELECT amount, tax_category, billable, invoiced FROM expenses \
             WHERE project_id = {} AND deleted_at IS NULL",
            dialect.placeholder(1)
        );
        match &self.db {
            Db::Sqlite(pool) => {
                sqlx::query_as::<_, ExpenseRow>(&sql)
                    .bind(project_id)
                    .fetch_all(pool)
                    .await
            }
            #[cfg(feature = "postgres")]
            Db::Postgres(pool) => {
                sqlx::query_as::<_, ExpenseRow>(&sql)
                    .bind(project_id)
                    .fetch_all(pool)
                    .await
            }
        }
        .map_err(banto_storage::storage_error)
    }

    /// 案件1件の採算。案件が無ければ `NotFound`。
    pub async fn get(&self, project_id: i64) -> Result<ProjectProfitability, BantoError> {
        let header = self.project_header(project_id).await?;
        let work = self.work_totals(project_id).await?;
        let expenses = self.expense_rows(project_id).await?;
        let revenue = self.revenue_for(project_id).await?;

        // 税抜換算は行ごとに1回だけ丸める（合計してから丸めない）。
        let mut expense_cost = 0i64;
        let mut billable_expense_cost = 0i64;
        let mut uninvoiced_billable_expense_cost = 0i64;
        for row in &expenses {
            let taxable = taxable_amount(row.amount, &row.tax_category);
            expense_cost += taxable;
            if row.billable != 0 {
                billable_expense_cost += taxable;
                if row.invoiced == 0 {
                    uninvoiced_billable_expense_cost += taxable;
                }
            }
        }

        let total_cost = work.work_cost + expense_cost;
        let gross_profit = revenue - total_cost;
        let effective_minutes = work.total_minutes - work.excluded_minutes;
        let status = header.status.clone();

        Ok(ProjectProfitability {
            id: header.id,
            project_id: header.id,
            project_code: header.code,
            project_name: header.name,
            counts_toward_profitability: crate::projects::ProjectStatus::from_code(&status)
                .map(|s| s.counts_toward_profitability())
                // 未知のコードは採算集計の対象外として扱う（状態を増やす
                // マイグレーションと実装のズレを、数値に混ぜない）。
                .unwrap_or(false),
            status,
            contract_amount: header.contract_amount,
            revenue,
            work_cost: work.work_cost,
            expense_cost,
            billable_expense_cost,
            uninvoiced_billable_expense_cost,
            total_cost,
            gross_profit,
            gross_margin_bp: ratio_bp(gross_profit, revenue),
            invoice_progress_bp: header
                .contract_amount
                .and_then(|contract| ratio_bp(revenue, contract)),
            total_minutes: work.total_minutes,
            excluded_minutes: work.excluded_minutes,
            effective_rate_including_travel: effective_hourly_rate(
                gross_profit,
                work.total_minutes,
            ),
            effective_rate_excluding_travel: effective_hourly_rate(gross_profit, effective_minutes),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::customers::{CustomerInput, CustomersService, DAY_END_OF_MONTH};
    use crate::db::migrate_memory;
    use crate::expenses::{ExpenseInput, ExpensesService};
    use crate::masters::{CostRateInput, MastersService};
    use crate::projects::{ProjectInput, ProjectsService};
    use crate::work_logs::{WorkLogInput, WorkLogsService};

    struct Fixture {
        profitability: ProfitabilityService,
        work_logs: WorkLogsService,
        expenses: ExpensesService,
        projects: ProjectsService,
        pool: Db,
        customer_id: i64,
        project_id: i64,
    }

    /// 顧客1件・案件1件（契約額 1,000,000円）と、DESIGN 6,000円/時 ・
    /// TRAVEL 3,000円/時 のレートを用意した状態。
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
                closing_day: Some(DAY_END_OF_MONTH),
                payment_month_offset: Some(1),
                payment_day: Some(DAY_END_OF_MONTH),
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
                billing_hourly_rate: None,
                scope: None,
                note: None,
            })
            .await
            .expect("project");
        let masters = MastersService::new(pool.clone());
        for (code, rate) in [("DESIGN", 6_000), ("TRAVEL", 3_000)] {
            masters
                .set_cost_rate(CostRateInput {
                    work_category_code: code.to_string(),
                    hourly_rate: rate,
                })
                .await
                .expect("rate");
        }
        Fixture {
            profitability: ProfitabilityService::new(pool.clone()),
            work_logs: WorkLogsService::new(pool.clone()),
            expenses: ExpensesService::new(pool.clone()),
            projects,
            pool,
            customer_id: customer.id,
            project_id: project.id,
        }
    }

    fn work_log(project_id: i64, code: &str, minutes: i64) -> WorkLogInput {
        WorkLogInput {
            project_id,
            trip_id: None,
            worked_on: "2026-08-20".to_string(),
            work_category_code: code.to_string(),
            minutes,
            applied_rate: None,
            description: None,
            invoiced: false,
        }
    }

    fn expense(project_id: i64, amount: i64, tax_category: &str) -> ExpenseInput {
        ExpenseInput {
            project_id,
            trip_id: None,
            spent_on: "2026-08-20".to_string(),
            expense_category_code: "TRANSPORT".to_string(),
            payee: None,
            amount,
            tax_category: Some(tax_category.to_string()),
            description: None,
            billable: false,
            invoiced: false,
        }
    }

    /// **論理削除が採算へ波及しないこと**（`docs/domain/sync.md` 5節）。
    ///
    /// 採算は `work_logs` / `expenses` を直接集計するので、`deleted_at IS NULL`
    /// の入れ忘れがそのまま「消したはずの行が原価に乗る」になる。しかも画面上は
    /// 何も起きないので気付けない。ここで固定する。
    #[tokio::test]
    async fn deleted_rows_drop_out_of_the_profitability_totals() {
        let f = fixture().await;
        f.work_logs
            .create(work_log(f.project_id, "DESIGN", 60))
            .await
            .expect("残す工数");
        let doomed_work = f
            .work_logs
            .create(work_log(f.project_id, "DESIGN", 120))
            .await
            .expect("消す工数");
        f.expenses
            .create(expense(f.project_id, 1_100, "STANDARD_10"))
            .await
            .expect("残す経費");
        let doomed_expense = f
            .expenses
            .create(expense(f.project_id, 5_500, "STANDARD_10"))
            .await
            .expect("消す経費");

        let before = f.profitability.get(f.project_id).await.expect("before");
        assert_eq!(before.total_minutes, 180);

        f.work_logs
            .delete(doomed_work.id)
            .await
            .expect("delete 工数");
        f.expenses
            .delete(doomed_expense.id)
            .await
            .expect("delete 経費");

        let after = f.profitability.get(f.project_id).await.expect("after");
        assert_eq!(after.total_minutes, 60, "削除した工数が分数に残っている");
        assert!(
            after.work_cost < before.work_cost,
            "削除した工数が原価に残っている: {} → {}",
            before.work_cost,
            after.work_cost
        );
        // 経費は税抜換算されるので、残るのは 1,100 円ぶんだけ。
        assert_eq!(
            after.expense_cost,
            taxable_amount(1_100, "STANDARD_10"),
            "削除した経費が原価に残っている"
        );
    }

    #[test]
    fn taxable_amount_backs_out_tax_by_category() {
        // 10%: 11,000 円の支払 → 税抜 10,000 円
        assert_eq!(taxable_amount(11_000, "STANDARD_10"), 10_000);
        // 切捨て: 1,000 × 100 ÷ 110 = 909.09... → 909
        assert_eq!(taxable_amount(1_000, "STANDARD_10"), 909);
        // 8%: 1,080 → 1,000 ちょうど
        assert_eq!(taxable_amount(1_080, "REDUCED_8"), 1_000);
        assert_eq!(taxable_amount(1_000, "REDUCED_8"), 925);
        // 非課税・不課税は換算しない
        assert_eq!(taxable_amount(1_000, "EXEMPT"), 1_000);
        assert_eq!(taxable_amount(1_000, "OUT_OF_SCOPE"), 1_000);
    }

    /// 行ごとに丸めた額の合計は、合計してから丸めた額と一致しないことがある
    /// （要件 V-1 と同じ趣旨の固定）。丸めの位置を誤った実装が入っても
    /// 数値が変わらなければ、このテストは意味を持たない — 差が出る金額を
    /// 選んである。
    #[tokio::test]
    async fn expense_cost_rounds_per_row_not_after_summing() {
        let f = fixture().await;
        for _ in 0..3 {
            f.expenses
                .create(expense(f.project_id, 1_002, "STANDARD_10"))
                .await
                .expect("expense");
        }
        let result = f.profitability.get(f.project_id).await.expect("get");
        // 行ごと: floor(1,002 × 100 ÷ 110) = 910 → 910 × 3 = 2,730
        assert_eq!(result.expense_cost, 2_730);
        // 合計後に丸めると 3,006 × 100 ÷ 110 = 2,732.7… → 2,732 で 2円ずれる。
        assert_eq!(3_006 * 100 / 110, 2_732);
        assert_ne!(result.expense_cost, 3_006 * 100 / 110);
    }

    /// 実質時間単価は移動込み・移動除くの2種が同時に出る（F-P2）。
    /// 移動の判定は作業分類のフラグで行う（F-P3）。
    #[tokio::test]
    async fn effective_rates_are_reported_both_ways() {
        let f = fixture().await;
        // 設計 600分（6,000円/時 → 60,000円）+ 移動 300分（3,000円/時 → 15,000円）
        f.work_logs
            .create(work_log(f.project_id, "DESIGN", 600))
            .await
            .expect("design");
        f.work_logs
            .create(work_log(f.project_id, "TRAVEL", 300))
            .await
            .expect("travel");

        let result = f.profitability.get(f.project_id).await.expect("get");
        assert_eq!(result.work_cost, 60_000 + 15_000);
        assert_eq!(result.total_minutes, 900);
        assert_eq!(result.excluded_minutes, 300);
        // 売上 0 なので粗利は −75,000 円。
        assert_eq!(result.gross_profit, -75_000);
        // 移動込み: −75,000 × 60 ÷ 900 = −5,000
        assert_eq!(result.effective_rate_including_travel, Some(-5_000));
        // 移動除く: −75,000 × 60 ÷ 600 = −7,500
        assert_eq!(result.effective_rate_excluding_travel, Some(-7_500));
    }

    /// 工数が無ければ実質時間単価は 0 除算にならず `None`。
    #[tokio::test]
    async fn effective_rates_are_none_without_work_logs() {
        let f = fixture().await;
        let result = f.profitability.get(f.project_id).await.expect("get");
        assert_eq!(result.total_minutes, 0);
        assert_eq!(result.effective_rate_including_travel, None);
        assert_eq!(result.effective_rate_excluding_travel, None);
    }

    /// 移動しかない案件では「移動除く」の分母が 0 になる。
    #[tokio::test]
    async fn excluding_travel_is_none_when_only_travel_was_logged() {
        let f = fixture().await;
        f.work_logs
            .create(work_log(f.project_id, "TRAVEL", 120))
            .await
            .expect("travel");
        let result = f.profitability.get(f.project_id).await.expect("get");
        assert!(result.effective_rate_including_travel.is_some());
        assert_eq!(result.effective_rate_excluding_travel, None);
    }

    /// 工数原価は行ごとに丸めた `internal_cost` の合計（Phase 1 決定 C-1）。
    #[tokio::test]
    async fn work_cost_sums_row_level_rounded_costs() {
        let f = fixture().await;
        // 6,000円/時 で 7分 → floor(7 × 6000 ÷ 60) = 700
        for _ in 0..3 {
            f.work_logs
                .create(work_log(f.project_id, "DESIGN", 7))
                .await
                .expect("work log");
        }
        let result = f.profitability.get(f.project_id).await.expect("get");
        assert_eq!(result.work_cost, 2_100);
    }

    /// 顧客請求対象の経費は「うち請求対象」「うち未請求」に分けて出す。
    #[tokio::test]
    async fn billable_expenses_are_reported_separately() {
        let f = fixture().await;
        let mut billable = expense(f.project_id, 11_000, "STANDARD_10");
        billable.billable = true;
        f.expenses.create(billable).await.expect("billable");

        let mut invoiced = expense(f.project_id, 5_500, "STANDARD_10");
        invoiced.billable = true;
        invoiced.invoiced = true;
        f.expenses.create(invoiced).await.expect("invoiced");

        f.expenses
            .create(expense(f.project_id, 2_200, "STANDARD_10"))
            .await
            .expect("internal");

        let result = f.profitability.get(f.project_id).await.expect("get");
        assert_eq!(result.expense_cost, 10_000 + 5_000 + 2_000);
        assert_eq!(result.billable_expense_cost, 15_000);
        assert_eq!(result.uninvoiced_billable_expense_cost, 10_000);
    }

    /// 売上が 0 の間は粗利率を出さない（0 除算にしない）。請求進捗は
    /// 契約額があれば 0% として出す — 「まだ請求していない」ことを
    /// 見せるのが F-P5 の目的なので、こちらは `None` にしない。
    #[tokio::test]
    async fn margin_is_none_and_progress_is_zero_before_invoicing() {
        let f = fixture().await;
        f.work_logs
            .create(work_log(f.project_id, "DESIGN", 60))
            .await
            .expect("work log");
        let result = f.profitability.get(f.project_id).await.expect("get");
        assert_eq!(result.revenue, 0);
        assert_eq!(result.gross_margin_bp, None);
        assert_eq!(result.invoice_progress_bp, Some(0));
    }

    /// 契約額が未設定なら請求進捗は出せない。
    #[tokio::test]
    async fn progress_is_none_without_contract_amount() {
        let f = fixture().await;
        let project = f.projects.get(f.project_id).await.expect("project");
        f.projects
            .update(
                f.project_id,
                ProjectInput {
                    code: project.code.clone(),
                    customer_id: project.customer_id,
                    name: project.name.clone(),
                    status: project.status.clone(),
                    started_on: None,
                    due_on: None,
                    estimate_amount: None,
                    contract_amount: None,
                    billing_hourly_rate: None,
                    scope: None,
                    note: None,
                },
            )
            .await
            .expect("update");
        let result = f.profitability.get(f.project_id).await.expect("get");
        assert_eq!(result.contract_amount, None);
        assert_eq!(result.invoice_progress_bp, None);
    }

    #[tokio::test]
    async fn unknown_project_is_not_found() {
        let f = fixture().await;
        let err = f.profitability.get(f.project_id + 999).await.unwrap_err();
        assert!(matches!(err, BantoError::NotFound { .. }), "{err:?}");
    }

    /// 失注案件でも数値は返すが、採算集計の対象外であることを添える。
    #[tokio::test]
    async fn lost_projects_are_flagged_as_out_of_scope() {
        let f = fixture().await;
        let project = f.projects.get(f.project_id).await.expect("project");
        f.projects
            .update(
                f.project_id,
                ProjectInput {
                    code: project.code.clone(),
                    customer_id: project.customer_id,
                    name: project.name.clone(),
                    status: "LOST".to_string(),
                    started_on: None,
                    due_on: None,
                    estimate_amount: None,
                    contract_amount: project.contract_amount,
                    billing_hourly_rate: project.billing_hourly_rate,
                    scope: None,
                    note: None,
                },
            )
            .await
            .expect("update");
        let result = f.profitability.get(f.project_id).await.expect("get");
        assert_eq!(result.status, "LOST");
        assert!(!result.counts_toward_profitability);
    }

    /// 確定した請求書の明細だけが売上に立つ（F-P4）。Draft と取消は立たない。
    #[tokio::test]
    async fn revenue_comes_from_issued_invoice_lines_only() {
        use crate::invoices::{InvoiceInput, InvoiceLineInput, InvoicesService};

        let f = fixture().await;
        let invoices = InvoicesService::new(f.pool.clone());
        let input = |unit_price: i64| InvoiceInput {
            customer_id: f.customer_id,
            closing_on: None,
            due_on: None,
            corrected_invoice_id: None,
            note: None,
            lines: vec![InvoiceLineInput {
                project_id: f.project_id,
                item_name: "設計".to_string(),
                quantity: 1,
                unit_price,
                tax_category: "STANDARD_10".to_string(),
                source_type: None,
                source_id: None,
                note: None,
            }],
        };

        // Draft のうちは売上に立たない。
        let draft = invoices.create(input(200_000)).await.expect("draft");
        let before = f.profitability.get(f.project_id).await.expect("get");
        assert_eq!(before.revenue, 0);
        assert_eq!(before.gross_margin_bp, None);

        let issued = invoices.issue(draft.invoice.id).await.expect("issue");
        let after = f.profitability.get(f.project_id).await.expect("get");
        // 税抜の明細合計がそのまま売上（消費税は採算に含めない。決定 A-3）。
        assert_eq!(after.revenue, 200_000);
        assert_eq!(after.gross_profit, 200_000);
        // 契約額 1,000,000 に対する請求進捗 20%
        assert_eq!(after.invoice_progress_bp, Some(2_000));
        assert_eq!(after.gross_margin_bp, Some(10_000));

        // 取消した請求書は売上から外れる（赤伝の二重計上を防ぐ）。
        invoices.cancel(issued.invoice.id).await.expect("cancel");
        let cancelled = f.profitability.get(f.project_id).await.expect("get");
        assert_eq!(cancelled.revenue, 0);
    }

    #[test]
    fn ratio_bp_guards_zero_and_negative_denominators() {
        assert_eq!(ratio_bp(1, 0), None);
        assert_eq!(ratio_bp(1, -1), None);
        // 30% = 3000 bp
        assert_eq!(ratio_bp(300_000, 1_000_000), Some(3_000));
        // 切捨て（ゼロ方向）: −1 ÷ 3 × 10000 = −3333.3... → −3333
        assert_eq!(ratio_bp(-1, 3), Some(-3_333));
    }
}
