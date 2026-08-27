-- アルファ実使用からのフィードバック（2026-08-27）: 顧客の締日・支払サイト・
-- 支払日を任意化する。導入時点で全部揃っていなくても顧客を登録でき、後から
-- 埋められるようにする（0007_customers.sql の NOT NULL を撤去）。
--
-- 値の意味・許容範囲（1..28 または 99=月末、オフセットは 0..6）は不変
-- （customers.rs の検証。Some のときだけ従来ルールを適用、None は合法）。
--
-- SQLite は列の NOT NULL を後から外せないため、テーブル再作成で行う
-- （conventions §11 は「マイグレーションの流儀」を定めるが本リポジトリに
-- 再作成の前例が無いため、ここで新たに確立する）。
--
-- 再作成で温存する制約・オブジェクト（0007/0023/0024 由来。漏らすと
-- 静かに壊れる）:
--   - id: INTEGER PRIMARY KEY AUTOINCREMENT
--   - code: NOT NULL UNIQUE
--   - name / created_at / updated_at: NOT NULL
--   - deleted_at: 0023_tombstones.sql で追加された論理削除列
--   - トリガー sync_outbox_customers_insert / _update（0024）: テーブルを
--     DROP すると SQLite が自動的に道連れで削除するので、リネーム後に
--     同一定義で再作成する
--   - customers 自体には INDEX は無い（子テーブル側の FK 参照
--     customer_id は projects/invoices/payments にあるが、これらは
--     `REFERENCES customers(id)` の宣言のみで SQLite の
--     `PRAGMA foreign_keys` は本リポジトリで有効化していないため
--     （customers.rs delete() のコメント参照）、customers 側の
--     DROP/RENAME に対して外部キー起因のエラーは発生しない
CREATE TABLE customers_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    code TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    contact_person TEXT,
    address TEXT,
    phone TEXT,
    email TEXT,
    billing_name TEXT,
    closing_day INTEGER,
    payment_month_offset INTEGER,
    payment_day INTEGER,
    note TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT
);

INSERT INTO customers_new (
    id, code, name, contact_person, address, phone, email, billing_name,
    closing_day, payment_month_offset, payment_day, note, created_at,
    updated_at, deleted_at
)
SELECT
    id, code, name, contact_person, address, phone, email, billing_name,
    closing_day, payment_month_offset, payment_day, note, created_at,
    updated_at, deleted_at
FROM customers;

DROP TABLE customers;
ALTER TABLE customers_new RENAME TO customers;

-- 0024_sync_outbox_triggers.sql と同一定義（DROP TABLE で失われたトリガーを
-- 再作成する）。
CREATE TRIGGER sync_outbox_customers_insert AFTER INSERT ON customers
BEGIN
    INSERT INTO sync_outbox (table_name, row_key, op, changed_at)
    VALUES ('customers', CAST(NEW.id AS TEXT), 'INSERT', datetime('now'));
END;

CREATE TRIGGER sync_outbox_customers_update AFTER UPDATE ON customers
BEGIN
    INSERT INTO sync_outbox (table_name, row_key, op, changed_at)
    VALUES (
        'customers',
        CAST(NEW.id AS TEXT),
        CASE
            WHEN OLD.deleted_at IS NULL AND NEW.deleted_at IS NOT NULL THEN 'DELETE'
            ELSE 'UPDATE'
        END,
        datetime('now')
    );
END;
