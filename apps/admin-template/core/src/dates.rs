//! 業務日付（JST のローカル日付、`YYYY-MM-DD`）の最小限の計算。
//!
//! 日付ライブラリを足さない（conventions §3 / ADR-0002）。Banto 本体も
//! 同じ方針で `db.rs` と `banto-admin-services/src/backup.rs` が
//! Howard Hinnant の civil-date アルゴリズムを自前で持っている。ここは
//! Business が必要とする**逆方向（日付 → 通算日）と日数加算**を足したもので、
//! 出張の一括生成が現地作業を日ごとに配置するために使う。
//!
//! 扱うのは業務日付だけ。タイムゾーン変換・時刻は持たない（CLAUDE.md 第4章）。

/// `YYYY-MM-DD` から 1970-01-01 起点の通算日へ。書式が不正なら `None`。
///
/// 存在しない日（2026-02-30 等）は `None` を返す — 往復変換して元の文字列に
/// 戻るかで判定するので、月ごとの日数表も閏年判定も持たずに済む。
pub fn days_since_epoch(iso_date: &str) -> Option<i64> {
    let bytes = iso_date.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return None;
    }
    if !bytes
        .iter()
        .enumerate()
        .all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit())
    {
        return None;
    }
    let year: i64 = iso_date[0..4].parse().ok()?;
    let month: i64 = iso_date[5..7].parse().ok()?;
    let day: i64 = iso_date[8..10].parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }

    // days_from_civil（Hinnant）。`iso_date_from_days_since_epoch` の逆。
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let mp = if month > 2 { month - 3 } else { month + 9 }; // [0, 11]
    let doy = (153 * mp + 2) / 5 + day - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    let days = era * 146097 + doe - 719468;

    // 2026-02-30 のような「形式は正しいが存在しない日」を弾く。
    if to_iso_date(days) == iso_date {
        Some(days)
    } else {
        None
    }
}

/// 1970-01-01 起点の通算日から `YYYY-MM-DD` へ（civil_from_days、Hinnant）。
pub fn to_iso_date(days: i64) -> String {
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

/// `iso_date` の `days` 日後（負なら前）。書式が不正なら `None`。
pub fn add_days(iso_date: &str, days: i64) -> Option<String> {
    days_since_epoch(iso_date).map(|base| to_iso_date(base + days))
}

/// 日付が `YYYY-MM-DD` として妥当か（存在する日か）。
pub fn is_valid_date(iso_date: &str) -> bool {
    days_since_epoch(iso_date).is_some()
}

/// `start` から `end` までの日数（両端を含む）。順序が逆なら `None`。
pub fn inclusive_day_span(start: &str, end: &str) -> Option<i64> {
    let start = days_since_epoch(start)?;
    let end = days_since_epoch(end)?;
    if end < start {
        return None;
    }
    Some(end - start + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_known_dates() {
        for (date, days) in [
            ("1970-01-01", 0),
            ("1970-01-02", 1),
            ("1969-12-31", -1),
            ("2000-02-29", 11016), // 400年周期の閏日
            ("2026-08-20", 20685),
        ] {
            assert_eq!(days_since_epoch(date), Some(days), "{date}");
            assert_eq!(to_iso_date(days), date);
        }
    }

    #[test]
    fn rejects_malformed_and_nonexistent_dates() {
        for bad in [
            "2026-8-20",  // ゼロ埋めなし
            "2026/08/20", // 区切りが違う
            "2026-13-01", // 月が範囲外
            "2026-02-30", // 2月に存在しない日
            "2026-04-31", // 4月に存在しない日
            "2027-02-29", // 平年の閏日
            "",
        ] {
            assert_eq!(days_since_epoch(bad), None, "{bad} は不正");
            assert!(!is_valid_date(bad));
        }
        // 閏年の 2/29 は妥当。
        assert!(is_valid_date("2028-02-29"));
    }

    #[test]
    fn adds_days_across_month_and_year_boundaries() {
        assert_eq!(add_days("2026-08-30", 2).as_deref(), Some("2026-09-01"));
        assert_eq!(add_days("2026-12-31", 1).as_deref(), Some("2027-01-01"));
        assert_eq!(add_days("2028-02-28", 1).as_deref(), Some("2028-02-29"));
        assert_eq!(add_days("2026-09-01", -1).as_deref(), Some("2026-08-31"));
        assert_eq!(add_days("bad", 1), None);
    }

    #[test]
    fn counts_inclusive_spans() {
        assert_eq!(inclusive_day_span("2026-09-01", "2026-09-03"), Some(3));
        assert_eq!(inclusive_day_span("2026-09-01", "2026-09-01"), Some(1));
        assert_eq!(inclusive_day_span("2026-09-03", "2026-09-01"), None);
    }
}
