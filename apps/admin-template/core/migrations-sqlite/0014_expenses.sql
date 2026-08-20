-- Phase 3: 経費（docs/domain/schema.md §3.3）。
--
-- billable（顧客請求対象か）と invoiced（請求書に載せたか）は**別フラグ**
-- （AGENTS.md 3.4）。請求対象でも未請求の期間があり、両者を1つにすると
-- 「請求し忘れ」を検出できなくなる。
--
-- tax_category は**仕入側**の区分（国際線は不課税など）。顧客へ再請求する
-- ときの区分は一律 10%（Phase 1 決定 B-5）で、請求側は InvoiceLine が持つ。
CREATE TABLE expenses (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER NOT NULL REFERENCES projects(id),
    trip_id INTEGER REFERENCES trips(id) ON DELETE SET NULL,
    spent_on TEXT NOT NULL,
    expense_category_code TEXT NOT NULL,
    payee TEXT,
    amount INTEGER NOT NULL,
    tax_category TEXT NOT NULL,
    description TEXT,
    billable INTEGER NOT NULL DEFAULT 0,
    invoiced INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX idx_expenses_project ON expenses(project_id, spent_on);
CREATE INDEX idx_expenses_trip ON expenses(trip_id);
