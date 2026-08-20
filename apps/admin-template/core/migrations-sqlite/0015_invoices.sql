-- Phase 5: 請求書（docs/domain/schema.md §4.1）。
--
-- CLAUDE.md 1.3: **project_id を持たない。** 1請求書に複数案件（締日まとめ
-- 請求）と、1案件に複数請求書（着手金／中間金／検収）の両方が実務で発生する
-- ため、案件は invoice_lines 側が持つ。
--
-- CLAUDE.md 1.5: status に Overdue を作らない。PARTIALLY_PAID / PAID も
-- 保持しない（決定 C-15）。入金状態は消込の残額から都度導出する。
--
-- 確定（ISSUED）時にスナップショットする列（CLAUDE.md 1.2 と同じ考え方 —
-- マスタや設定を後から変えても、発行済みの請求書は動かない）:
--   invoice_number / issued_on / due_on / rounding_mode / issuer_* / total_*
--
-- invoice_number は Draft では NULL。確定時に採番して欠番を作らない
-- （決定 C-9。適格請求書の連続性）。UNIQUE は NULL を重複扱いしないため、
-- 未採番の Draft が何件あっても衝突しない。
CREATE TABLE invoices (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    invoice_number TEXT UNIQUE,
    customer_id INTEGER NOT NULL REFERENCES customers(id),
    status TEXT NOT NULL,
    issued_on TEXT,
    closing_on TEXT,
    due_on TEXT,
    -- 赤伝で差し替えた元請求書（決定 C-10）。外部キー制約は張らない
    -- （取消済み請求書の削除可否に引きずられないため）。
    corrected_invoice_id INTEGER,
    total_taxable INTEGER NOT NULL DEFAULT 0,
    total_tax INTEGER NOT NULL DEFAULT 0,
    total_amount INTEGER NOT NULL DEFAULT 0,
    rounding_mode TEXT NOT NULL,
    issuer_name TEXT,
    issuer_registration_number TEXT,
    issuer_address TEXT,
    note TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX idx_invoices_customer ON invoices(customer_id, issued_on);
CREATE INDEX idx_invoices_status ON invoices(status);
