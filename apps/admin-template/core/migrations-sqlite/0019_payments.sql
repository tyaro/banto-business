-- Phase 6: 入金（docs/domain/schema.md §5.1）。
--
-- CLAUDE.md 1.4: **invoice_id を持たない。** まとめ入金（1入金を複数請求書へ）と
-- 分割入金（1請求書へ複数入金）の両方が起きるため、対応付けは
-- payment_allocations が持つ N:M。
--
-- 入金状態（一部入金 / 入金済）も期限超過も**保持しない**（要件 F-Y5 / F-Y6）。
-- 充当額の合計と支払期限から都度導出する — 状態として持つと日次バッチが必要に
-- なり、アプリを毎日起動しないローカルファースト構成で実態とずれる
-- （CLAUDE.md 1.5）。
CREATE TABLE payments (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    customer_id INTEGER NOT NULL REFERENCES customers(id),
    paid_on TEXT NOT NULL,
    amount INTEGER NOT NULL,
    method TEXT,
    note TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX idx_payments_customer ON payments(customer_id, paid_on);
