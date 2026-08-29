-- no-transaction
-- ↑ sqlx への指令。banto-storage は SQLite 接続で PRAGMA foreign_keys=ON に
--   しており（crates/banto-storage/src/sqlite.rs）、FK 有効のまま下の
--   DROP TABLE customers を行うと、projects 等の子テーブルに参照行がある
--   DB（=実運用DB）では FOREIGN KEY constraint failed で失敗する。
--   PRAGMA foreign_keys はトランザクション内では no-op のため、sqlx の
--   トランザクションから外し、このファイル内で明示的に
--   OFF → BEGIN…COMMIT → ON する（SQLite 公式のテーブル再作成手順）。
--
-- 【履歴】初版はこの点を見落とし「FK は有効化していない」と誤認したまま
-- リリースされ、v0.1.0-alpha.2 がデータ入りDBで起動時に即クラッシュした。
-- 適用成功した実DBが存在しない段階だったため、CLAUDE.md 第5章の例外として
-- 2026-08-29 に本修正版へ書き換えた（ユーザー承認済み）。空DBで初版が
-- 適用済みの開発用DBはチェックサム不一致になるので作り直すこと。
--
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
--
-- 途中で kill された場合の再実行も冪等: customers_new は RENAME 済みで
-- 存在せず、トリガーは DROP TABLE で customers と一緒に削除されているため、
-- このファイルを最初から再実行しても壊れない。

PRAGMA foreign_keys = OFF;

BEGIN;

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

COMMIT;

PRAGMA foreign_keys = ON;
