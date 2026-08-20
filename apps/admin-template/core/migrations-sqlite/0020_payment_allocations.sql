-- Phase 6: 消込（docs/domain/schema.md §5.2）。
--
-- 1入金を複数請求書へ、1請求書へ複数入金を充当できる（要件 F-Y1 / F-Y2）。
--
-- difference_amount は「入金額に含まれないが請求書を閉じる額」。振込手数料の
-- 先方差引・値引き・過入金の超過分をここに記録する（要件 F-Y3、決定 C-19）。
-- 残額は `請求額 − Σ(充当額 + 差額)` を 0 で下げ止めして導出する（F-Y4）。
--
-- 請求書側は ON DELETE を持たない（確定済みの請求書は消さない。取消で扱う）。
-- 入金を消せば充当も消える（ON DELETE CASCADE）— 入金の取り消しは消込ごと
-- 無かったことにするのが実態に合う。
CREATE TABLE payment_allocations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    payment_id INTEGER NOT NULL REFERENCES payments(id) ON DELETE CASCADE,
    invoice_id INTEGER NOT NULL REFERENCES invoices(id),
    allocated_amount INTEGER NOT NULL,
    difference_reason TEXT,
    difference_amount INTEGER NOT NULL DEFAULT 0,
    note TEXT,
    created_at TEXT NOT NULL
);

CREATE INDEX idx_payment_allocations_payment ON payment_allocations(payment_id);
CREATE INDEX idx_payment_allocations_invoice ON payment_allocations(invoice_id);
