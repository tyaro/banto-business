//! Phase 5: 消費税の計算（`docs/tax-calculation.md`）。
//!
//! **純粋関数のみ。** DB も `tauri` も `axum` も知らない（`tax-calculation.md`
//! 第5章の「この関数は純粋関数とし、DB・Tauri に依存させない」）。
//!
//! ## 原則（`CLAUDE.md` 1.7 / `tax-calculation.md` 4.1）
//!
//! **端数処理は1つの適格請求書につき、税率ごとに1回。** 明細行ごとの個別
//! 端数処理は行わない。計算順序は
//!
//! ```text
//! 1. 明細行を税率区分ごとにグループ化
//! 2. 各グループ内で対価の額を合計          ← ここまで端数処理なし
//! 3. グループごとに 合計額 × 税率 を計算
//! 4. グループごとに1回だけ端数処理          ← 端数処理はここのみ
//! 5. 各グループの税額を合算して請求書合計税額とする
//! ```
//!
//! ## 丸めの方向と桁
//!
//! 1円未満を、確定時にスナップショットした [`RoundingMode`] で処理する。
//! マイナスは**ゼロ方向**（絶対値を処理して符号を戻す）。`−3,333.5` は
//! 切捨てで `−3,333` であり、負の無限大方向の `−3,334` ではない。
//!
//! 率は basis point（1/10000）で持ち、小数を経由しない（`CLAUDE.md` 1.1）。

use serde::{Deserialize, Serialize};

/// 税率区分（`tax-calculation.md` 3）。非課税と不課税はどちらも税額 0 だが、
/// 適格請求書では区分して記載するため別の値として持つ（T-07）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TaxCategory {
    /// 標準税率 10%
    Standard10,
    /// 軽減税率 8%
    Reduced8,
    /// 非課税
    Exempt,
    /// 不課税（対象外）
    OutOfScope,
}

impl TaxCategory {
    /// 出力順。請求書の税率区分ごとの記載がこの順に並ぶ（毎回同じ並びに
    /// なるよう、入力の出現順ではなく固定の順序で出す）。
    pub const ALL: [TaxCategory; 4] = [
        TaxCategory::Standard10,
        TaxCategory::Reduced8,
        TaxCategory::Exempt,
        TaxCategory::OutOfScope,
    ];

    pub fn as_code(self) -> &'static str {
        match self {
            TaxCategory::Standard10 => "STANDARD_10",
            TaxCategory::Reduced8 => "REDUCED_8",
            TaxCategory::Exempt => "EXEMPT",
            TaxCategory::OutOfScope => "OUT_OF_SCOPE",
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        TaxCategory::ALL.into_iter().find(|c| c.as_code() == code)
    }

    /// 税率（basis point。10% = 1000、8% = 800、非課税・不課税 = 0）。
    pub fn rate_bp(self) -> i64 {
        match self {
            TaxCategory::Standard10 => 1_000,
            TaxCategory::Reduced8 => 800,
            TaxCategory::Exempt | TaxCategory::OutOfScope => 0,
        }
    }
}

/// 端数処理の方向（`tax-calculation.md` 4.3）。既定は切捨て。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RoundingMode {
    /// 切捨て（既定）
    #[default]
    Floor,
    /// 四捨五入
    Round,
    /// 切上げ
    Ceil,
}

impl RoundingMode {
    pub const ALL: [RoundingMode; 3] =
        [RoundingMode::Floor, RoundingMode::Round, RoundingMode::Ceil];

    pub fn as_code(self) -> &'static str {
        match self {
            RoundingMode::Floor => "FLOOR",
            RoundingMode::Round => "ROUND",
            RoundingMode::Ceil => "CEIL",
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        RoundingMode::ALL.into_iter().find(|m| m.as_code() == code)
    }
}

/// 税計算の入力1行。金額は税抜・円。値引き行はマイナス（B-3）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TaxLine {
    pub amount: i64,
    pub category: TaxCategory,
}

/// 税率区分ごとの集計結果。`invoice_tax_summaries` にそのまま保存する。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaxGroupResult {
    pub category: TaxCategory,
    pub rate_bp: i64,
    /// 税率区分ごとの対価合計（税抜）。**端数処理していない合計**。
    pub taxable_amount: i64,
    /// 端数処理後の消費税額。区分ごとに1回だけ処理した結果。
    pub tax_amount: i64,
}

/// 請求書1件ぶんの税集計。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaxSummary {
    /// 対価合計が 0 の税率区分は**含まない**（T-10 の決定：請求書に
    /// 「10% 対象 0円」の行を出さない）。
    pub groups: Vec<TaxGroupResult>,
    pub total_taxable: i64,
    pub total_tax: i64,
    /// 税込合計。
    pub total_amount: i64,
}

/// `value ÷ divisor` を [`RoundingMode`] で丸める。**絶対値に対して処理して
/// 符号を戻す**（`tax-calculation.md` 4.3 のマイナスの定義）。
///
/// `divisor` は正の定数（10,000）でのみ呼ぶ。
fn divide_rounded(value: i64, divisor: i64, mode: RoundingMode) -> i64 {
    debug_assert!(divisor > 0, "divisor must be positive");
    let sign = if value < 0 { -1 } else { 1 };
    let magnitude = value.abs();
    let rounded = match mode {
        RoundingMode::Floor => magnitude / divisor,
        // +0.5 相当を足してから切り捨てる。`divisor` が偶数（10,000）なので
        // ちょうど半分の値は必ず外側（絶対値が大きい方）へ寄る。
        RoundingMode::Round => (magnitude + divisor / 2) / divisor,
        // `i64::div_ceil` は unstable なので自前で切り上げる。
        RoundingMode::Ceil => (magnitude + divisor - 1) / divisor,
    };
    sign * rounded
}

/// 税率区分ごとに1回だけ端数処理して集計する（`tax-calculation.md` 4.2）。
///
/// 明細が空、または全区分の対価合計が 0 なら、グループ無し・合計 0 を返す
/// （T-09 / T-10）。
pub fn calculate_tax(lines: &[TaxLine], rounding: RoundingMode) -> TaxSummary {
    let mut groups = Vec::new();
    let mut total_taxable = 0i64;
    let mut total_tax = 0i64;

    for category in TaxCategory::ALL {
        // 手順2: グループ内で対価を合計する。ここまで端数処理はしない。
        let taxable_amount: i64 = lines
            .iter()
            .filter(|line| line.category == category)
            .map(|line| line.amount)
            .sum();
        if taxable_amount == 0 {
            // 対価合計が 0 の区分は行を作らない（T-10 の決定）。明細行が
            // 存在しない場合も、0円行しか無い場合も、値引きで相殺されて
            // 0 になった場合も同じ扱い。
            continue;
        }
        // 手順3〜4: 合計額 × 税率 を計算し、ここで1回だけ端数処理する。
        let rate_bp = category.rate_bp();
        let tax_amount = divide_rounded(taxable_amount * rate_bp, 10_000, rounding);
        total_taxable += taxable_amount;
        total_tax += tax_amount;
        groups.push(TaxGroupResult {
            category,
            rate_bp,
            taxable_amount,
            tax_amount,
        });
    }

    TaxSummary {
        groups,
        total_taxable,
        total_tax,
        total_amount: total_taxable + total_tax,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(amount: i64, category: TaxCategory) -> TaxLine {
        TaxLine { amount, category }
    }

    fn standard(amount: i64) -> TaxLine {
        line(amount, TaxCategory::Standard10)
    }

    /// T-01: 10%のみ・端数なし。
    #[test]
    fn t01_standard_rate_without_fraction() {
        let summary = calculate_tax(&[standard(100_000)], RoundingMode::Floor);
        assert_eq!(summary.total_tax, 10_000);
        assert_eq!(summary.total_amount, 110_000);
        assert_eq!(summary.groups.len(), 1);
        assert_eq!(summary.groups[0].rate_bp, 1_000);
    }

    /// T-02: 10%のみ・端数あり（既定の切捨て）。
    #[test]
    fn t02_standard_rate_with_fraction_floors() {
        let summary = calculate_tax(&[standard(33_333)], RoundingMode::Floor);
        // 3,333.3 → 3,333
        assert_eq!(summary.total_tax, 3_333);
        assert_eq!(summary.total_amount, 36_666);
    }

    /// T-03: 端数処理方向の切替（同一入力で差が出ること）。
    #[test]
    fn t03_rounding_mode_changes_the_result() {
        // 33,335 × 10% = 3,333.5
        let floor = calculate_tax(&[standard(33_335)], RoundingMode::Floor);
        let round = calculate_tax(&[standard(33_335)], RoundingMode::Round);
        let ceil = calculate_tax(&[standard(33_335)], RoundingMode::Ceil);
        assert_eq!(floor.total_tax, 3_333);
        assert_eq!(round.total_tax, 3_334);
        assert_eq!(ceil.total_tax, 3_334);
    }

    /// T-04: 8%のみ。
    #[test]
    fn t04_reduced_rate() {
        let summary = calculate_tax(&[line(33_333, TaxCategory::Reduced8)], RoundingMode::Floor);
        // 2,666.64 → 2,666
        assert_eq!(summary.total_tax, 2_666);
        assert_eq!(summary.total_amount, 35_999);
    }

    /// T-05: 10% + 8% 混在。
    #[test]
    fn t05_standard_and_reduced_mixed() {
        let summary = calculate_tax(
            &[standard(33_333), line(33_333, TaxCategory::Reduced8)],
            RoundingMode::Floor,
        );
        assert_eq!(summary.total_taxable, 66_666);
        assert_eq!(summary.total_tax, 3_333 + 2_666);
        assert_eq!(summary.total_amount, 72_665);
        assert_eq!(summary.groups.len(), 2);
    }

    /// T-06: 10% + 不課税 混在（不課税分は税額計算の対象外）。
    #[test]
    fn t06_standard_and_out_of_scope() {
        let summary = calculate_tax(
            &[standard(10_000), line(5_000, TaxCategory::OutOfScope)],
            RoundingMode::Floor,
        );
        assert_eq!(summary.total_taxable, 15_000);
        assert_eq!(summary.total_tax, 1_000);
        assert_eq!(summary.total_amount, 16_000);
    }

    /// T-07: 全税率混在。**非課税と不課税は別グループとして区別して出力する**。
    #[test]
    fn t07_all_categories_are_reported_separately() {
        let summary = calculate_tax(
            &[
                standard(10_000),
                line(10_000, TaxCategory::Reduced8),
                line(5_000, TaxCategory::Exempt),
                line(5_000, TaxCategory::OutOfScope),
            ],
            RoundingMode::Floor,
        );
        assert_eq!(summary.total_taxable, 30_000);
        assert_eq!(summary.total_tax, 1_800);
        assert_eq!(summary.total_amount, 31_800);
        assert_eq!(summary.groups.len(), 4);
        let categories: Vec<TaxCategory> = summary.groups.iter().map(|g| g.category).collect();
        assert_eq!(categories, TaxCategory::ALL.to_vec());
        // 非課税・不課税はどちらも税額 0 だが、別グループとして残る。
        assert_eq!(summary.groups[2].tax_amount, 0);
        assert_eq!(summary.groups[3].tax_amount, 0);
    }

    /// T-08: **明細行ごとに端数処理した場合と結果が異なることの検証**。
    /// 端数処理の位置を誤った実装を検出するための基準ケース。
    #[test]
    fn t08_per_group_rounding_differs_from_per_line_rounding() {
        let lines = [standard(33_335), standard(33_335), standard(33_335)];
        let summary = calculate_tax(&lines, RoundingMode::Floor);
        // 区分ごと処理: floor(100,005 × 10%) = 10,000
        assert_eq!(summary.total_tax, 10_000);
        // 行ごと処理なら floor(3,333.5) × 3 = 9,999 になるはずで、
        // **差 1 円が出ること**が本ケースの合格条件。
        let per_line: i64 = lines
            .iter()
            .map(|l| divide_rounded(l.amount * l.category.rate_bp(), 10_000, RoundingMode::Floor))
            .sum();
        assert_eq!(per_line, 9_999);
        assert_ne!(summary.total_tax, per_line);
    }

    /// T-09: 明細0件。
    #[test]
    fn t09_empty_lines() {
        let summary = calculate_tax(&[], RoundingMode::Floor);
        assert!(summary.groups.is_empty());
        assert_eq!(summary.total_taxable, 0);
        assert_eq!(summary.total_tax, 0);
        assert_eq!(summary.total_amount, 0);
    }

    /// T-10: 金額0円の明細を含む。対価合計が 0 の区分はグループを作らない。
    #[test]
    fn t10_zero_amount_lines() {
        let summary = calculate_tax(&[standard(10_000), standard(0)], RoundingMode::Floor);
        assert_eq!(summary.total_tax, 1_000);
        assert_eq!(summary.groups.len(), 1);

        // 0円行しか無い区分はグループにしない。
        let only_zero = calculate_tax(&[standard(0)], RoundingMode::Floor);
        assert!(only_zero.groups.is_empty());
        assert_eq!(only_zero.total_amount, 0);

        // 値引きで相殺されて 0 になった区分も同じ扱い。
        let cancelled_out =
            calculate_tax(&[standard(10_000), standard(-10_000)], RoundingMode::Floor);
        assert!(cancelled_out.groups.is_empty());
        assert_eq!(cancelled_out.total_tax, 0);
    }

    /// T-11a: 値引き行あり・区分合計はプラス。
    #[test]
    fn t11a_discount_line_with_positive_group_total() {
        let summary = calculate_tax(&[standard(100_000), standard(-33_335)], RoundingMode::Floor);
        assert_eq!(summary.total_taxable, 66_665);
        // 6,666.5 → 6,666（ゼロ方向切捨て）
        assert_eq!(summary.total_tax, 6_666);
    }

    /// T-11b: 値引き行あり・**区分合計がマイナス**。負方向切捨ての −3,334 に
    /// なっていたら不合格。
    #[test]
    fn t11b_discount_line_with_negative_group_total() {
        let summary = calculate_tax(&[standard(-33_335)], RoundingMode::Floor);
        assert_eq!(summary.total_taxable, -33_335);
        // −3,333.5 → −3,333（ゼロ方向）
        assert_eq!(summary.total_tax, -3_333);
        assert_eq!(summary.total_amount, -36_668);
    }

    /// マイナスの丸めは3方向とも絶対値に対して行い、符号を戻す。
    #[test]
    fn negative_rounding_is_toward_zero_in_every_mode() {
        assert_eq!(
            calculate_tax(&[standard(-33_335)], RoundingMode::Round).total_tax,
            -3_334
        );
        assert_eq!(
            calculate_tax(&[standard(-33_335)], RoundingMode::Ceil).total_tax,
            -3_334
        );
        // 切上げは絶対値が大きくなる方向。−33,331 × 10% = −3,333.1 → −3,334
        assert_eq!(
            calculate_tax(&[standard(-33_331)], RoundingMode::Ceil).total_tax,
            -3_334
        );
    }

    #[test]
    fn tax_category_codes_round_trip() {
        for category in TaxCategory::ALL {
            assert_eq!(TaxCategory::from_code(category.as_code()), Some(category));
        }
        assert_eq!(TaxCategory::from_code("NOPE"), None);
    }

    #[test]
    fn rounding_mode_codes_round_trip() {
        for mode in RoundingMode::ALL {
            assert_eq!(RoundingMode::from_code(mode.as_code()), Some(mode));
        }
        assert_eq!(RoundingMode::from_code("NOPE"), None);
        assert_eq!(RoundingMode::default(), RoundingMode::Floor);
    }
}
