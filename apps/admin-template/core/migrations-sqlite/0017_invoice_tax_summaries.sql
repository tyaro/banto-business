-- Phase 5: 税率区分ごとの集計（docs/domain/schema.md §4.3）。
--
-- **適格請求書の「税率ごとに区分して合計した対価の額・消費税額・適用税率」の
-- 記載根拠**。確定時のスナップショットで、明細から都度再計算しない — 税率や
-- 端数処理設定を後から変えても発行済みの請求書が動かないようにするため
-- （CLAUDE.md 1.2 と同じ考え方）。
--
-- 対価合計が 0 の区分は行を作らない（docs/tax-calculation.md T-10 の決定）。
-- rate_bp は basis point（10% = 1000、8% = 800、非課税・不課税 = 0）。率を
-- 小数で持たない（CLAUDE.md 1.1）。
CREATE TABLE invoice_tax_summaries (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    invoice_id INTEGER NOT NULL REFERENCES invoices(id) ON DELETE CASCADE,
    tax_category TEXT NOT NULL,
    rate_bp INTEGER NOT NULL,
    taxable_amount INTEGER NOT NULL,
    tax_amount INTEGER NOT NULL
);

CREATE INDEX idx_invoice_tax_summaries_invoice ON invoice_tax_summaries(invoice_id);
