//! 月カレンダー（Phase 7 準備）。conventions §2 に従い `tauri` / `axum` /
//! RBAC を知らない。
//!
//! ## 何のための画面か
//!
//! 一覧では見えないものが2つある。
//!
//! 1. **工数を付け忘れた日**。一覧は「入力した行」しか並ばないので、
//!    入力しなかった日は最初から存在しない。月グリッドに置くと、空いた
//!    平日がそのまま抜けとして見える（`docs/plan.md` Phase 7 の評価項目
//!    「工数入力継続性」）。
//! 2. **支払期限が月のどこに固まっているか**。未入金一覧（F-Y7）は
//!    一次元なので、月末に3件重なっているといった偏りが読み取れない。
//!
//! ## 保持しない
//!
//! 集計値は列として持たず、毎回 `work_logs` / `expenses` / `trips` /
//! `invoices` / `payments` から導出する。採算（`profitability.rs`）と同じ
//! 理由で、持つと元の行を直したときに再計算漏れで食い違う。
//!
//! ## 期限超過の判定はここでやらない
//!
//! `CLAUDE.md` 1.5 のとおり Overdue は状態として持たず導出する。ただし
//! この画面は「その日が支払期限の請求」を置くだけで、超過かどうかは
//! 判定しない。判定には業務日付の「今日」が要るが、カレンダーは過去も
//! 未来も同じ月グリッドに描くため、日ごとに意味が変わる値を混ぜると
//! 読み手が混乱する。超過の一覧は `payments.rs` の `outstanding` が持つ。
//!
//! ## 金額の丸め
//!
//! このモジュールは**丸めを一切しない**。経費は税込の実支出をそのまま、
//! 請求は確定済の総額と残額をそのまま合計する。カレンダーは金額の
//! 出どころを指すための画面であり、採算値（税抜換算・行ごとの丸めを伴う）
//! は `profitability.rs` の担当。同じ数字を2箇所で丸めると食い違う。

use crate::dates::{days_in_month, is_valid_date};
use banto_core::{BantoError, ListParams};
use banto_storage::Db;
use serde::{Deserialize, Serialize};

/// 月の指定に使うフィルタ名。`DataProvider.getList('calendar', params)` の
/// `filters` に `{ field: "month", op: "eq", value: "2026-08" }` として載る。
pub const MONTH_FILTER: &str = "month";

/// その日に工数が付いた案件1件ぶん。案件名まで返すのは、セルに色と名前を
/// 出すためにフロントから案件マスタを引き直させないため。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarProjectSlice {
    pub project_id: i64,
    pub project_code: String,
    pub project_name: String,
    pub minutes: i64,
}

/// カレンダー1日ぶん。
///
/// **何も無い日は行を返さない。** 月の全日を埋めた配列にはしない ——
/// グリッドの升目はフロント側が月から組み立てるものであり、サーバが
/// 空の行を 28〜31 個返しても「無い」以上の情報は増えない。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarDay {
    /// `DataProvider` の行は id を要求するので、日付をそのまま id にする。
    /// 月内で一意であり、行の同一性は日付そのもの。
    pub id: String,
    /// `YYYY-MM-DD`（ローカル日付＝業務日付。CLAUDE.md 4）。
    pub date: String,

    pub worked_minutes: i64,
    pub work_log_count: i64,
    /// 案件別の内訳。分の降順、同数なら案件コード順で安定させる。
    pub projects: Vec<CalendarProjectSlice>,

    pub expense_count: i64,
    /// 経費の合計（**税込の実支出**。税抜換算しない — モジュール冒頭の理由）。
    pub expense_amount: i64,

    /// その日にかかっている出張の数（`start_on <= date <= end_on`）。
    /// 出張は期間を持つので、1件が複数日に現れる。
    pub trip_count: i64,

    /// その日が締日の請求の数。
    pub invoice_closing_count: i64,
    /// その日が支払期限の請求の数（取消を除く）。
    pub invoice_due_count: i64,
    /// 同・残額の合計。消込済み（残額0）の請求も件数には入るが、この額には
    /// 乗らない。「期限は来るが回収は済んでいる」を区別するため。
    pub invoice_due_remaining: i64,

    pub payment_count: i64,
    pub payment_amount: i64,
}

impl CalendarDay {
    fn new(date: String) -> Self {
        Self {
            id: date.clone(),
            date,
            worked_minutes: 0,
            work_log_count: 0,
            projects: Vec::new(),
            expense_count: 0,
            expense_amount: 0,
            trip_count: 0,
            invoice_closing_count: 0,
            invoice_due_count: 0,
            invoice_due_remaining: 0,
            payment_count: 0,
            payment_amount: 0,
        }
    }
}

/// `YYYY-MM` を月初・月末の ISO 日付に開く。
///
/// 月末は `days_in_month` から求める（うるう年を含めて `dates.rs` が既に
/// 面倒を見ている）。範囲を「月初 <= x <= 月末」の閉区間で扱うのは、
/// 日付が DATE / TEXT で時刻を持たないため（CLAUDE.md 4）。時刻があると
/// 半開区間にしないと月末が落ちるが、ここではその問題が起きない。
pub fn month_range(month: &str) -> Option<(String, String)> {
    let bytes = month.as_bytes();
    if bytes.len() != 7 || bytes[4] != b'-' {
        return None;
    }
    let year: i64 = month.get(0..4)?.parse().ok()?;
    let month_number: i64 = month.get(5..7)?.parse().ok()?;
    let last_day = days_in_month(year, month_number)?;
    let first = format!("{year:04}-{month_number:02}-01");
    let last = format!("{year:04}-{month_number:02}-{last_day:02}");
    // `days_in_month` が通った時点で組み立てた日付は正しいはずだが、
    // 桁あふれ（年が5桁など）を素通しにしないために最後に検証する。
    if !is_valid_date(&first) || !is_valid_date(&last) {
        return None;
    }
    Some((first, last))
}

/// `ListParams` から対象月を取り出す。
///
/// フィルタが無い／壊れている場合は `None` を返すだけで、勝手に「今月」へ
/// 倒さない。サービス層は業務日付の「今日」を持たない（`payments.rs` の
/// `is_overdue` が today を引数で受けるのと同じ理由）ため、ここで
/// `SystemTime::now()` を呼ぶとテストが日付に依存する。
pub fn month_from_params(params: &ListParams) -> Option<String> {
    params
        .filters
        .iter()
        .find(|f| f.field == MONTH_FILTER)
        .and_then(|f| f.value.as_str())
        .map(str::to_owned)
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct WorkRow {
    worked_on: String,
    project_id: i64,
    project_code: String,
    project_name: String,
    minutes: i64,
    log_count: i64,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct AmountByDateRow {
    date: String,
    row_count: i64,
    amount: i64,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct CountByDateRow {
    date: String,
    row_count: i64,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct TripRow {
    start_on: String,
    end_on: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct InvoiceDueRow {
    due_on: String,
    total_amount: i64,
    settled_amount: i64,
}

/// `SUM` を `CAST(... AS BIGINT)` で包むのは PostgreSQL 対策
/// （`profitability.rs` と同じ理由: `SUM(bigint)` が numeric を返す）。
/// `COUNT(*)` も同様に包む —— PostgreSQL の `COUNT` は `bigint` を返すので
/// 本来は不要だが、片方だけ包むと「なぜここだけ」を毎回考えることになる。
const WORK_BY_DAY_SQL: &str = "SELECT w.worked_on, p.id AS project_id, \
     p.code AS project_code, p.name AS project_name, \
     CAST(COALESCE(SUM(w.minutes), 0) AS BIGINT) AS minutes, \
     CAST(COUNT(*) AS BIGINT) AS log_count \
     FROM work_logs w JOIN projects p ON p.id = w.project_id \
     WHERE w.worked_on >= {0} AND w.worked_on <= {1} \
     GROUP BY w.worked_on, p.id, p.code, p.name \
     ORDER BY w.worked_on, minutes DESC, p.code";

const EXPENSES_BY_DAY_SQL: &str = "SELECT spent_on AS date, \
     CAST(COUNT(*) AS BIGINT) AS row_count, \
     CAST(COALESCE(SUM(amount), 0) AS BIGINT) AS amount \
     FROM expenses WHERE spent_on >= {0} AND spent_on <= {1} \
     GROUP BY spent_on";

const PAYMENTS_BY_DAY_SQL: &str = "SELECT paid_on AS date, \
     CAST(COUNT(*) AS BIGINT) AS row_count, \
     CAST(COALESCE(SUM(amount), 0) AS BIGINT) AS amount \
     FROM payments WHERE paid_on >= {0} AND paid_on <= {1} \
     GROUP BY paid_on";

/// 締日は確定済（ISSUED）の請求だけを見る。Draft の締日は「まだそう
/// するつもり」でしかなく、カレンダーに置くと確定済と見分けが付かない。
const INVOICE_CLOSING_BY_DAY_SQL: &str = "SELECT closing_on AS date, \
     CAST(COUNT(*) AS BIGINT) AS row_count \
     FROM invoices \
     WHERE status = 'ISSUED' AND closing_on IS NOT NULL \
       AND closing_on >= {0} AND closing_on <= {1} \
     GROUP BY closing_on";

/// 支払期限は行ごとに残額を出したいので集約せずに返す。件数は最大でも
/// 月内の請求数で、束ねるほどの量にならない。
const INVOICE_DUE_SQL: &str = "SELECT i.due_on, i.total_amount, \
     CAST(COALESCE((SELECT SUM(a.allocated_amount + a.difference_amount) \
         FROM payment_allocations a WHERE a.invoice_id = i.id), 0) AS BIGINT) \
         AS settled_amount \
     FROM invoices i \
     WHERE i.status = 'ISSUED' AND i.due_on IS NOT NULL \
       AND i.due_on >= {0} AND i.due_on <= {1}";

/// 出張は期間を持つので、月に**かかっている**ものを取る（月内に始まる
/// ものだけではない —— 月をまたぐ出張の途中の日が抜ける）。
///
/// 条件は「月末以前に始まり、月初以降に終わる」。**プレースホルダは
/// SQL 内での出現順に `{0}`（月初）→ `{1}`（月末）でなければならない** ——
/// SQLite の `?` は出現順に束縛されるので、`{1}` を先に書くと月初と月末が
/// 入れ替わる（PostgreSQL の `$1`/`$2` は番号で解決するので気付けない）。
/// そのため `end_on >= 月初 AND start_on <= 月末` の順で書く。
const TRIPS_OVERLAPPING_SQL: &str = "SELECT start_on, end_on FROM trips \
     WHERE end_on >= {0} AND start_on <= {1}";

/// カレンダーのサービス層（conventions §2）。読み取り専用 —— 集計値を
/// 保持しない（モジュール冒頭）ので、書き込みの入口もイベント通知も持たない。
#[derive(Clone)]
pub struct CalendarService {
    db: Db,
}

impl CalendarService {
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    /// 2つの `?` / `$1,$2` を方言に合わせて埋める。
    fn range_sql(&self, template: &str) -> String {
        let dialect = self.db.dialect();
        template
            .replace("{0}", &dialect.placeholder(1))
            .replace("{1}", &dialect.placeholder(2))
    }

    /// 対象月の日別集計。`month` は `YYYY-MM`。
    ///
    /// 月として解釈できなければ `Validation` を返す。空の結果を返すのでは
    /// なく弾くのは、`2026-13` のような指定が「その月にデータが無い」と
    /// 区別できなくなるため。
    pub async fn month(&self, month: &str) -> Result<Vec<CalendarDay>, BantoError> {
        let Some((first, last)) = month_range(month) else {
            return Err(BantoError::Validation {
                field_errors: vec![banto_core::FieldError {
                    field: MONTH_FILTER.to_string(),
                    message: format!("month must be YYYY-MM, got {month:?}"),
                }],
            });
        };

        // 日付をキーにした挿入順非依存の集約。`BTreeMap` にすると
        // キー（ISO 日付）の辞書順＝日付順になるので、最後に並べ直さなくてよい。
        let mut days: std::collections::BTreeMap<String, CalendarDay> =
            std::collections::BTreeMap::new();

        for row in self.work_rows(&first, &last).await? {
            let day = days
                .entry(row.worked_on.clone())
                .or_insert_with(|| CalendarDay::new(row.worked_on.clone()));
            day.worked_minutes += row.minutes;
            day.work_log_count += row.log_count;
            day.projects.push(CalendarProjectSlice {
                project_id: row.project_id,
                project_code: row.project_code,
                project_name: row.project_name,
                minutes: row.minutes,
            });
        }

        for row in self.amount_rows(EXPENSES_BY_DAY_SQL, &first, &last).await? {
            let day = days
                .entry(row.date.clone())
                .or_insert_with(|| CalendarDay::new(row.date.clone()));
            day.expense_count += row.row_count;
            day.expense_amount += row.amount;
        }

        for row in self.amount_rows(PAYMENTS_BY_DAY_SQL, &first, &last).await? {
            let day = days
                .entry(row.date.clone())
                .or_insert_with(|| CalendarDay::new(row.date.clone()));
            day.payment_count += row.row_count;
            day.payment_amount += row.amount;
        }

        for row in self.closing_rows(&first, &last).await? {
            let day = days
                .entry(row.date.clone())
                .or_insert_with(|| CalendarDay::new(row.date.clone()));
            day.invoice_closing_count += row.row_count;
        }

        for row in self.due_rows(&first, &last).await? {
            let day = days
                .entry(row.due_on.clone())
                .or_insert_with(|| CalendarDay::new(row.due_on.clone()));
            day.invoice_due_count += 1;
            // 残額の定義は `payments.rs` に一本化する（0 未満にしない、
            // 決定 C-11）。ここで `total - settled` を書き直すと、
            // 過入金の扱いが2箇所に分かれる。
            day.invoice_due_remaining +=
                crate::payments::remaining_amount(row.total_amount, row.settled_amount);
        }

        // 出張だけは行が期間なので、日へ展開してから数える。
        for trip in self.trip_rows(&first, &last).await? {
            for date in span_dates(&trip.start_on, &trip.end_on, &first, &last) {
                let day = days
                    .entry(date.clone())
                    .or_insert_with(|| CalendarDay::new(date));
                day.trip_count += 1;
            }
        }

        Ok(days.into_values().collect())
    }

    async fn work_rows(&self, first: &str, last: &str) -> Result<Vec<WorkRow>, BantoError> {
        let sql = self.range_sql(WORK_BY_DAY_SQL);
        match &self.db {
            Db::Sqlite(pool) => {
                sqlx::query_as::<_, WorkRow>(&sql)
                    .bind(first)
                    .bind(last)
                    .fetch_all(pool)
                    .await
            }
            #[cfg(feature = "postgres")]
            Db::Postgres(pool) => {
                sqlx::query_as::<_, WorkRow>(&sql)
                    .bind(first)
                    .bind(last)
                    .fetch_all(pool)
                    .await
            }
        }
        .map_err(banto_storage::storage_error)
    }

    async fn amount_rows(
        &self,
        template: &str,
        first: &str,
        last: &str,
    ) -> Result<Vec<AmountByDateRow>, BantoError> {
        let sql = self.range_sql(template);
        match &self.db {
            Db::Sqlite(pool) => {
                sqlx::query_as::<_, AmountByDateRow>(&sql)
                    .bind(first)
                    .bind(last)
                    .fetch_all(pool)
                    .await
            }
            #[cfg(feature = "postgres")]
            Db::Postgres(pool) => {
                sqlx::query_as::<_, AmountByDateRow>(&sql)
                    .bind(first)
                    .bind(last)
                    .fetch_all(pool)
                    .await
            }
        }
        .map_err(banto_storage::storage_error)
    }

    async fn closing_rows(
        &self,
        first: &str,
        last: &str,
    ) -> Result<Vec<CountByDateRow>, BantoError> {
        let sql = self.range_sql(INVOICE_CLOSING_BY_DAY_SQL);
        match &self.db {
            Db::Sqlite(pool) => {
                sqlx::query_as::<_, CountByDateRow>(&sql)
                    .bind(first)
                    .bind(last)
                    .fetch_all(pool)
                    .await
            }
            #[cfg(feature = "postgres")]
            Db::Postgres(pool) => {
                sqlx::query_as::<_, CountByDateRow>(&sql)
                    .bind(first)
                    .bind(last)
                    .fetch_all(pool)
                    .await
            }
        }
        .map_err(banto_storage::storage_error)
    }

    async fn due_rows(&self, first: &str, last: &str) -> Result<Vec<InvoiceDueRow>, BantoError> {
        let sql = self.range_sql(INVOICE_DUE_SQL);
        match &self.db {
            Db::Sqlite(pool) => {
                sqlx::query_as::<_, InvoiceDueRow>(&sql)
                    .bind(first)
                    .bind(last)
                    .fetch_all(pool)
                    .await
            }
            #[cfg(feature = "postgres")]
            Db::Postgres(pool) => {
                sqlx::query_as::<_, InvoiceDueRow>(&sql)
                    .bind(first)
                    .bind(last)
                    .fetch_all(pool)
                    .await
            }
        }
        .map_err(banto_storage::storage_error)
    }

    async fn trip_rows(&self, first: &str, last: &str) -> Result<Vec<TripRow>, BantoError> {
        let sql = self.range_sql(TRIPS_OVERLAPPING_SQL);
        match &self.db {
            Db::Sqlite(pool) => {
                sqlx::query_as::<_, TripRow>(&sql)
                    .bind(first)
                    .bind(last)
                    .fetch_all(pool)
                    .await
            }
            #[cfg(feature = "postgres")]
            Db::Postgres(pool) => {
                sqlx::query_as::<_, TripRow>(&sql)
                    .bind(first)
                    .bind(last)
                    .fetch_all(pool)
                    .await
            }
        }
        .map_err(banto_storage::storage_error)
    }
}

/// 出張の期間を、表示中の月に収まる範囲で日付へ展開する。
///
/// 月をまたぐ出張は前後がはみ出すので、`window_first`/`window_last` で
/// 切り詰める。切り詰めないと隣の月の升目に入れようとして落ちる。
fn span_dates(start: &str, end: &str, window_first: &str, window_last: &str) -> Vec<String> {
    let from = if start < window_first {
        window_first
    } else {
        start
    };
    let to = if end > window_last { window_last } else { end };
    let (Some(from_days), Some(to_days)) = (
        crate::dates::days_since_epoch(from),
        crate::dates::days_since_epoch(to),
    ) else {
        // 日付として読めない行は無視する。書き込み時に検証済みなので
        // 通常は起きないが、ここで落とすとカレンダー全体が出なくなる。
        return Vec::new();
    };
    if to_days < from_days {
        return Vec::new();
    }
    (from_days..=to_days)
        .map(crate::dates::to_iso_date)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::customers::{CustomerInput, CustomersService, DAY_END_OF_MONTH};
    use crate::db::migrate_memory;
    use crate::expenses::{ExpenseInput, ExpensesService};
    use crate::invoices::{InvoiceInput, InvoiceLineInput, InvoicesService};
    use crate::payments::{PaymentAllocationInput, PaymentInput, PaymentsService};
    use crate::projects::{ProjectInput, ProjectsService};
    use crate::trips::{TripInput, TripsService};
    use crate::work_logs::{WorkLogInput, WorkLogsService};
    use banto_core::{FilterOp, FilterState};

    // --- 純粋関数（DB を要らないもの） ---

    #[test]
    fn month_range_opens_a_month_into_its_first_and_last_day() {
        assert_eq!(
            month_range("2026-08"),
            Some(("2026-08-01".to_string(), "2026-08-31".to_string()))
        );
        assert_eq!(
            month_range("2026-09"),
            Some(("2026-09-01".to_string(), "2026-09-30".to_string()))
        );
    }

    /// うるう年の2月。`days_in_month` に委ねているが、カレンダーの升目が
    /// 1日ずれるのは目に見える不具合なので入口の側でも固定しておく。
    #[test]
    fn month_range_handles_february_in_leap_and_common_years() {
        for (month, last) in [
            ("2028-02", "2028-02-29"),
            ("2026-02", "2026-02-28"),
            // 100年ルール（平年）と 400年ルール（うるう年）
            ("2100-02", "2100-02-28"),
            ("2000-02", "2000-02-29"),
        ] {
            assert_eq!(
                month_range(month).map(|(_, l)| l),
                Some(last.to_string()),
                "month {month}"
            );
        }
    }

    #[test]
    fn month_range_rejects_malformed_input() {
        for bad in [
            "2026-13",
            "2026-00",
            "2026-8",
            "202608",
            "2026-08-01",
            "",
            "abcd-ef",
        ] {
            assert_eq!(month_range(bad), None, "should reject {bad:?}");
        }
    }

    /// フィルタが無ければ `None`。勝手に「今月」へ倒さない
    /// （サービス層は業務日付を持たない）。
    #[test]
    fn month_from_params_reads_the_month_filter_and_defaults_to_none() {
        let mut params = ListParams::default();
        assert_eq!(month_from_params(&params), None);

        params.filters.push(FilterState {
            field: MONTH_FILTER.to_string(),
            op: FilterOp::Eq,
            value: serde_json::json!("2026-08"),
        });
        assert_eq!(month_from_params(&params), Some("2026-08".to_string()));
    }

    #[test]
    fn span_dates_expands_a_trip_and_clips_it_to_the_visible_month() {
        assert_eq!(
            span_dates("2026-08-03", "2026-08-05", "2026-08-01", "2026-08-31"),
            vec!["2026-08-03", "2026-08-04", "2026-08-05"]
        );
        // 前月から続いている出張は月初から
        assert_eq!(
            span_dates("2026-07-30", "2026-08-02", "2026-08-01", "2026-08-31"),
            vec!["2026-08-01", "2026-08-02"]
        );
        // 翌月へ続く出張は月末まで
        assert_eq!(
            span_dates("2026-08-30", "2026-09-02", "2026-08-01", "2026-08-31"),
            vec!["2026-08-30", "2026-08-31"]
        );
        // 日帰り
        assert_eq!(
            span_dates("2026-08-10", "2026-08-10", "2026-08-01", "2026-08-31"),
            vec!["2026-08-10"]
        );
        // 逆転している行は無視する（カレンダー全体を落とさない）
        assert!(span_dates("2026-08-10", "2026-08-09", "2026-08-01", "2026-08-31").is_empty());
    }

    // --- DB を伴う集計 ---

    struct Fixture {
        db: Db,
        calendar: CalendarService,
        customer_id: i64,
        project_id: i64,
        other_project_id: i64,
    }

    async fn fixture() -> Fixture {
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
                closing_day: DAY_END_OF_MONTH,
                payment_month_offset: 1,
                payment_day: DAY_END_OF_MONTH,
                note: None,
            })
            .await
            .expect("customer");
        let projects = ProjectsService::new(db.clone());
        let mut ids = Vec::new();
        for (code, name) in [("P001", "架空案件A"), ("P002", "架空案件B")] {
            ids.push(
                projects
                    .create(ProjectInput {
                        code: code.to_string(),
                        customer_id: customer.id,
                        name: name.to_string(),
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
                    .expect("project")
                    .id,
            );
        }
        Fixture {
            calendar: CalendarService::new(db.clone()),
            db,
            customer_id: customer.id,
            project_id: ids[0],
            other_project_id: ids[1],
        }
    }

    impl Fixture {
        async fn add_work_log(&self, project_id: i64, worked_on: &str, minutes: i64) {
            WorkLogsService::new(self.db.clone())
                .create(WorkLogInput {
                    project_id,
                    trip_id: None,
                    worked_on: worked_on.to_string(),
                    work_category_code: "DESIGN".to_string(),
                    minutes,
                    // 原価レートは記録時点の単価を行に焼き付ける（CLAUDE.md 1.2）。
                    // 既定の作業分類にはレートが入っていないので明示する。
                    applied_rate: Some(5_000),
                    description: None,
                    invoiced: false,
                })
                .await
                .expect("work log");
        }

        async fn add_expense(&self, spent_on: &str, amount: i64) {
            ExpensesService::new(self.db.clone())
                .create(ExpenseInput {
                    project_id: self.project_id,
                    trip_id: None,
                    spent_on: spent_on.to_string(),
                    expense_category_code: "TRANSPORT".to_string(),
                    payee: None,
                    amount,
                    tax_category: Some("STANDARD_10".to_string()),
                    description: None,
                    billable: false,
                    invoiced: false,
                })
                .await
                .expect("expense");
        }

        /// 税抜 `taxable` 円・10% の請求書を1件確定して返す
        /// （`payments.rs` の同名ヘルパと同じ組み立て）。
        async fn issue_invoice(&self, taxable: i64) -> crate::invoices::InvoiceDetail {
            let invoices = InvoicesService::new(self.db.clone());
            let draft = invoices
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
            invoices.issue(draft.invoice.id).await.expect("issue")
        }

        async fn day(&self, month: &str, date: &str) -> Option<CalendarDay> {
            self.calendar
                .month(month)
                .await
                .expect("month")
                .into_iter()
                .find(|d| d.date == date)
        }
    }

    #[tokio::test]
    async fn an_empty_month_returns_no_rows() {
        let f = fixture().await;
        assert!(f.calendar.month("2026-08").await.unwrap().is_empty());
    }

    /// 壊れた月指定は空ではなくエラー。空を返すと「その月にデータが無い」と
    /// 区別が付かない。
    #[tokio::test]
    async fn a_malformed_month_is_a_validation_error_not_an_empty_month() {
        let f = fixture().await;
        let err = f.calendar.month("2026-13").await.unwrap_err();
        assert!(
            matches!(err, BantoError::Validation { .. }),
            "expected Validation, got {err:?}"
        );
    }

    #[tokio::test]
    async fn work_logs_are_summed_per_day_and_broken_down_by_project() {
        let f = fixture().await;
        f.add_work_log(f.project_id, "2026-08-03", 120).await;
        f.add_work_log(f.other_project_id, "2026-08-03", 60).await;
        f.add_work_log(f.project_id, "2026-08-03", 30).await;

        let rows = f.calendar.month("2026-08").await.unwrap();
        assert_eq!(rows.len(), 1);
        let day = &rows[0];
        assert_eq!(day.date, "2026-08-03");
        // 行の id は日付そのもの（DataProvider が id を要求するため）
        assert_eq!(day.id, "2026-08-03");
        assert_eq!(day.worked_minutes, 210);
        assert_eq!(day.work_log_count, 3);
        assert_eq!(day.projects.len(), 2);
        // 分の降順
        assert_eq!(day.projects[0].project_id, f.project_id);
        assert_eq!(day.projects[0].minutes, 150);
        assert_eq!(day.projects[0].project_code, "P001");
        assert_eq!(day.projects[1].project_id, f.other_project_id);
        assert_eq!(day.projects[1].minutes, 60);
    }

    /// 何も無い日は行を返さない（月の全日を埋めない）。
    #[tokio::test]
    async fn days_without_anything_are_absent_from_the_result() {
        let f = fixture().await;
        f.add_work_log(f.project_id, "2026-08-03", 60).await;
        f.add_work_log(f.project_id, "2026-08-07", 60).await;

        let dates: Vec<String> = f
            .calendar
            .month("2026-08")
            .await
            .unwrap()
            .into_iter()
            .map(|d| d.date)
            .collect();
        assert_eq!(dates, vec!["2026-08-03", "2026-08-07"]);
    }

    /// 月の境界。月初・月末は入り、隣の月の隣接日は入らない。
    #[tokio::test]
    async fn rows_outside_the_month_are_excluded_at_both_boundaries() {
        let f = fixture().await;
        for date in ["2026-07-31", "2026-08-01", "2026-08-31", "2026-09-01"] {
            f.add_work_log(f.project_id, date, 60).await;
        }

        let dates: Vec<String> = f
            .calendar
            .month("2026-08")
            .await
            .unwrap()
            .into_iter()
            .map(|d| d.date)
            .collect();
        assert_eq!(dates, vec!["2026-08-01", "2026-08-31"]);
    }

    /// 経費は**税込の実支出**をそのまま合計する（税抜換算しない）。
    /// 換算するのは採算（`profitability.rs`）の役目。
    #[tokio::test]
    async fn expenses_are_summed_with_the_tax_inclusive_amount() {
        let f = fixture().await;
        f.add_expense("2026-08-05", 1_100).await;
        f.add_expense("2026-08-05", 2_200).await;

        let day = f.day("2026-08", "2026-08-05").await.expect("the day");
        assert_eq!(day.expense_count, 2);
        assert_eq!(day.expense_amount, 3_300);
        // 経費だけの日は工数ゼロ
        assert_eq!(day.worked_minutes, 0);
        assert!(day.projects.is_empty());
    }

    #[tokio::test]
    async fn a_trip_appears_on_every_day_it_covers() {
        let f = fixture().await;
        TripsService::new(f.db.clone())
            .create(TripInput {
                project_id: f.project_id,
                destination: "架空市".to_string(),
                start_on: "2026-08-10".to_string(),
                end_on: "2026-08-12".to_string(),
                onsite_days: 3,
                nights: 2,
                note: None,
                generate: None,
            })
            .await
            .expect("trip");

        let trip_days: Vec<String> = f
            .calendar
            .month("2026-08")
            .await
            .unwrap()
            .into_iter()
            .filter(|d| d.trip_count > 0)
            .map(|d| d.date)
            .collect();
        assert_eq!(trip_days, vec!["2026-08-10", "2026-08-11", "2026-08-12"]);
    }

    /// 支払期限の残額は `payments.rs::remaining_amount` の定義に従う
    /// （充当額 + 差額を引き、0 未満にしない）。一部入金なら残る。
    #[tokio::test]
    async fn an_invoice_due_date_carries_the_remaining_amount() {
        let f = fixture().await;
        let issued = f.issue_invoice(100_000).await;
        let due_on = issued.invoice.due_on.clone().expect("due_on");
        let total = issued.invoice.total_amount;

        PaymentsService::new(f.db.clone())
            .create(PaymentInput {
                customer_id: f.customer_id,
                paid_on: due_on.clone(),
                amount: 40_000,
                method: Some("振込".to_string()),
                note: None,
                allocations: vec![PaymentAllocationInput {
                    invoice_id: issued.invoice.id,
                    allocated_amount: 40_000,
                    difference_reason: None,
                    difference_amount: 0,
                    note: None,
                }],
            })
            .await
            .expect("payment");

        let month = &due_on[0..7];
        let day = f.day(month, &due_on).await.expect("the due date");
        assert_eq!(day.invoice_due_count, 1);
        assert_eq!(day.invoice_due_remaining, total - 40_000);
        // 同じ日に入金も立っている
        assert_eq!(day.payment_count, 1);
        assert_eq!(day.payment_amount, 40_000);
    }

    /// 満額消し込んだ請求は、期限の件数には残るが残額には乗らない
    /// （「期限は来るが回収済み」を区別する）。
    #[tokio::test]
    async fn a_fully_settled_invoice_still_counts_but_carries_no_remaining() {
        let f = fixture().await;
        let issued = f.issue_invoice(100_000).await;
        let due_on = issued.invoice.due_on.clone().expect("due_on");
        let total = issued.invoice.total_amount;

        PaymentsService::new(f.db.clone())
            .create(PaymentInput {
                customer_id: f.customer_id,
                paid_on: "2026-09-15".to_string(),
                amount: total,
                method: None,
                note: None,
                allocations: vec![PaymentAllocationInput {
                    invoice_id: issued.invoice.id,
                    allocated_amount: total,
                    difference_reason: None,
                    difference_amount: 0,
                    note: None,
                }],
            })
            .await
            .expect("payment");

        let day = f
            .day(&due_on[0..7], &due_on)
            .await
            .expect("the due date should still be present");
        assert_eq!(day.invoice_due_count, 1);
        assert_eq!(day.invoice_due_remaining, 0);
    }

    /// 未確定（Draft）の請求は締日も支払期限もカレンダーに出さない。
    #[tokio::test]
    async fn a_draft_invoice_is_not_placed_on_the_calendar() {
        let f = fixture().await;
        InvoicesService::new(f.db.clone())
            .create(InvoiceInput {
                customer_id: f.customer_id,
                closing_on: Some("2026-08-31".to_string()),
                due_on: Some("2026-08-31".to_string()),
                corrected_invoice_id: None,
                note: None,
                lines: vec![InvoiceLineInput {
                    project_id: f.project_id,
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
            .expect("draft");

        assert!(f.calendar.month("2026-08").await.unwrap().is_empty());
    }
}
