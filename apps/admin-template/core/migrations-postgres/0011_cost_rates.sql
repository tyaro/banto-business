-- PostgreSQL port of migrations-sqlite/0011_cost_rates.sql（conventions §11）。
--   金額の INTEGER -> BIGINT（Rust 側は i64。CLAUDE.md 1.1 で金額は整数円）
--   updated_at は TEXT のまま（業務日付を YYYY-MM-DD 文字列で保持する規約）
CREATE TABLE cost_rates (
    work_category_code TEXT PRIMARY KEY,
    hourly_rate BIGINT NOT NULL,
    updated_at TEXT NOT NULL
);
