-- Phase 2 基本マスター: 案件（docs/domain/schema.md §2.2）。
--
-- `contract_amount` / `estimate_amount` は INTEGER（円・税抜）。金額を
-- REAL で持たない（CLAUDE.md 1.1）。契約額は粗利計算には使わず、
-- 請求進捗（案件売上 ÷ 契約額）の分母として使う（Phase 1 決定 C-3）。
--
-- `status` は 7 値のコード（PROSPECT/ORDERED/IN_PROGRESS/
-- AWAITING_ACCEPTANCE/COMPLETED/LOST/ON_HOLD）。採算集計は LOST を除く
-- 全状態が対象（Phase 1 決定 C-12）。
CREATE TABLE projects (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    code TEXT NOT NULL UNIQUE,
    customer_id INTEGER NOT NULL REFERENCES customers(id),
    name TEXT NOT NULL,
    status TEXT NOT NULL,
    started_on TEXT,
    due_on TEXT,
    estimate_amount INTEGER,
    contract_amount INTEGER,
    scope TEXT,
    note TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX idx_projects_customer ON projects(customer_id);
CREATE INDEX idx_projects_status ON projects(status);
