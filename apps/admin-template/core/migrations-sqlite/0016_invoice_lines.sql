-- Phase 5: 請求明細（docs/domain/schema.md §4.2）。
--
-- project_id で「どの案件の売上か」を持つ（CLAUDE.md 1.3）。**外部キー制約は
-- 張らない** — 確定済みの明細は案件マスタの変更・削除に引きずられてはならず、
-- 案件採算の売上（要件 F-P4）はこの列を根拠に集計するため。
--
-- amount は税抜の行金額で**マイナス可**（値引き行。決定 B-3）。工数から
-- 起こす明細は「時間単価 × 時間」を整数で確定させてから載せる（決定 B-2）。
-- 行ごとの税額は持たない — 端数処理は税率区分ごとに1回だけで、行ごとには
-- 行わない（CLAUDE.md 1.7）。
--
-- source_type は WORK_LOG / EXPENSE / MANUAL。どこから起こした明細かを
-- 残すのは、確定時に元の工数・経費へ invoiced を立てるため。
CREATE TABLE invoice_lines (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    invoice_id INTEGER NOT NULL REFERENCES invoices(id) ON DELETE CASCADE,
    project_id INTEGER NOT NULL,
    line_no INTEGER NOT NULL,
    item_name TEXT NOT NULL,
    quantity INTEGER NOT NULL DEFAULT 1,
    unit_price INTEGER NOT NULL,
    amount INTEGER NOT NULL,
    tax_category TEXT NOT NULL,
    source_type TEXT,
    -- 元の工数・経費の id（source_type に対応）。マスタ扱いの参照ではなく
    -- 「この行はどれを請求したか」の記録なので、外部キー制約は張らない。
    source_id INTEGER,
    note TEXT
);

CREATE INDEX idx_invoice_lines_invoice ON invoice_lines(invoice_id, line_no);
CREATE INDEX idx_invoice_lines_project ON invoice_lines(project_id);
