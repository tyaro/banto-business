-- Phase 2 基本マスター: 顧客（docs/domain/schema.md §2.1）。
--
-- 締日・支払条件は文字列ではなくコード値で持つ（Phase 1 決定 C-8、
-- docs/domain/open-questions.md）。`closing_day` / `payment_day` は
-- 1..28 または 99（= 月末）。29〜31 を許さないのは、2月に存在しない日を
-- 業務日付として保持しないため。支払期限は Invoice 側に確定値として
-- 保存し、このマスタの変更が過去の請求書へ波及しないようにする
-- （CLAUDE.md 1.2 と同じ考え方）。
CREATE TABLE customers (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    code TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    contact_person TEXT,
    address TEXT,
    phone TEXT,
    email TEXT,
    billing_name TEXT,
    closing_day INTEGER NOT NULL,
    payment_month_offset INTEGER NOT NULL,
    payment_day INTEGER NOT NULL,
    note TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
