-- Phase 8: 変更を outbox へ記録するトリガ（`docs/domain/sync.md` 4節）。
--
-- ## なぜサービス層ではなくトリガか
--
-- 当初の設計は「書き込み系のサービスが行の変更と同じトランザクション内で
-- outbox へ1行足す」だったが、実装に入って**書き込みの入口が19箇所**あると
-- 分かった: 各サービスの create/update/delete が15、`invoices.rs` の確定・取消が
-- `work_logs` / `expenses` の `invoiced` を直接 UPDATE する経路が4、そして
-- `trips.rs` の一括生成が `work_logs` / `expenses` を直接 INSERT する。
--
-- 全部に手で足すと、**1箇所忘れただけでその変更は永久に同期されない**。
-- しかも忘れたことは同期して初めて分かる（相手に届かない、という形で）。
-- 新しい書き込み経路を足したときも同じ穴が空く。
--
-- トリガなら:
--   - 行の変更と**同一トランザクション**（トリガは呼び出し元の文の一部）
--   - **忘れようがない**。どの経路から書いても記録される
--   - 経路が増えても追従不要
--
-- 代償は「ロジックが DB 側にある」こと。方言ごとに2セット書く必要があり、
-- `conventions.md` §11 の2方言対応と同じ保守コストがかかる。それでも
-- 「取りこぼしが構造的に起きない」という outbox 採用の理由そのものを
-- 守れるほうを採った。
--
-- ## op の決め方
--
-- 物理削除はしないので DELETE トリガは要らない。論理削除は UPDATE なので、
-- `deleted_at` が NULL から非 NULL へ変わった UPDATE を `DELETE` として記録する。
-- 墓石をさらに更新した場合（既に非 NULL）は `UPDATE` のまま。
--
-- ## 記録しないもの
--
-- マイグレーションのシード（作業分類・経費分類の初期行）はこのトリガより前に
-- 流れるので記録されない。**両端末が同じマイグレーションを流すので、初期行は
-- 最初から双方に同じものが在る** —— 同期する必要が無い。

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

CREATE TRIGGER sync_outbox_projects_insert AFTER INSERT ON projects
BEGIN
    INSERT INTO sync_outbox (table_name, row_key, op, changed_at)
    VALUES ('projects', CAST(NEW.id AS TEXT), 'INSERT', datetime('now'));
END;

CREATE TRIGGER sync_outbox_projects_update AFTER UPDATE ON projects
BEGIN
    INSERT INTO sync_outbox (table_name, row_key, op, changed_at)
    VALUES (
        'projects',
        CAST(NEW.id AS TEXT),
        CASE
            WHEN OLD.deleted_at IS NULL AND NEW.deleted_at IS NOT NULL THEN 'DELETE'
            ELSE 'UPDATE'
        END,
        datetime('now')
    );
END;

CREATE TRIGGER sync_outbox_trips_insert AFTER INSERT ON trips
BEGIN
    INSERT INTO sync_outbox (table_name, row_key, op, changed_at)
    VALUES ('trips', CAST(NEW.id AS TEXT), 'INSERT', datetime('now'));
END;

CREATE TRIGGER sync_outbox_trips_update AFTER UPDATE ON trips
BEGIN
    INSERT INTO sync_outbox (table_name, row_key, op, changed_at)
    VALUES (
        'trips',
        CAST(NEW.id AS TEXT),
        CASE
            WHEN OLD.deleted_at IS NULL AND NEW.deleted_at IS NOT NULL THEN 'DELETE'
            ELSE 'UPDATE'
        END,
        datetime('now')
    );
END;

CREATE TRIGGER sync_outbox_work_logs_insert AFTER INSERT ON work_logs
BEGIN
    INSERT INTO sync_outbox (table_name, row_key, op, changed_at)
    VALUES ('work_logs', CAST(NEW.id AS TEXT), 'INSERT', datetime('now'));
END;

CREATE TRIGGER sync_outbox_work_logs_update AFTER UPDATE ON work_logs
BEGIN
    INSERT INTO sync_outbox (table_name, row_key, op, changed_at)
    VALUES (
        'work_logs',
        CAST(NEW.id AS TEXT),
        CASE
            WHEN OLD.deleted_at IS NULL AND NEW.deleted_at IS NOT NULL THEN 'DELETE'
            ELSE 'UPDATE'
        END,
        datetime('now')
    );
END;

CREATE TRIGGER sync_outbox_expenses_insert AFTER INSERT ON expenses
BEGIN
    INSERT INTO sync_outbox (table_name, row_key, op, changed_at)
    VALUES ('expenses', CAST(NEW.id AS TEXT), 'INSERT', datetime('now'));
END;

CREATE TRIGGER sync_outbox_expenses_update AFTER UPDATE ON expenses
BEGIN
    INSERT INTO sync_outbox (table_name, row_key, op, changed_at)
    VALUES (
        'expenses',
        CAST(NEW.id AS TEXT),
        CASE
            WHEN OLD.deleted_at IS NULL AND NEW.deleted_at IS NOT NULL THEN 'DELETE'
            ELSE 'UPDATE'
        END,
        datetime('now')
    );
END;

CREATE TRIGGER sync_outbox_work_categories_insert AFTER INSERT ON work_categories
BEGIN
    INSERT INTO sync_outbox (table_name, row_key, op, changed_at)
    VALUES ('work_categories', NEW.code, 'INSERT', datetime('now'));
END;

CREATE TRIGGER sync_outbox_work_categories_update AFTER UPDATE ON work_categories
BEGIN
    INSERT INTO sync_outbox (table_name, row_key, op, changed_at)
    VALUES (
        'work_categories',
        NEW.code,
        CASE
            WHEN OLD.deleted_at IS NULL AND NEW.deleted_at IS NOT NULL THEN 'DELETE'
            ELSE 'UPDATE'
        END,
        datetime('now')
    );
END;

CREATE TRIGGER sync_outbox_expense_categories_insert AFTER INSERT ON expense_categories
BEGIN
    INSERT INTO sync_outbox (table_name, row_key, op, changed_at)
    VALUES ('expense_categories', NEW.code, 'INSERT', datetime('now'));
END;

CREATE TRIGGER sync_outbox_expense_categories_update AFTER UPDATE ON expense_categories
BEGIN
    INSERT INTO sync_outbox (table_name, row_key, op, changed_at)
    VALUES (
        'expense_categories',
        NEW.code,
        CASE
            WHEN OLD.deleted_at IS NULL AND NEW.deleted_at IS NOT NULL THEN 'DELETE'
            ELSE 'UPDATE'
        END,
        datetime('now')
    );
END;

CREATE TRIGGER sync_outbox_cost_rates_insert AFTER INSERT ON cost_rates
BEGIN
    INSERT INTO sync_outbox (table_name, row_key, op, changed_at)
    VALUES ('cost_rates', NEW.work_category_code, 'INSERT', datetime('now'));
END;

CREATE TRIGGER sync_outbox_cost_rates_update AFTER UPDATE ON cost_rates
BEGIN
    INSERT INTO sync_outbox (table_name, row_key, op, changed_at)
    VALUES (
        'cost_rates',
        NEW.work_category_code,
        CASE
            WHEN OLD.deleted_at IS NULL AND NEW.deleted_at IS NOT NULL THEN 'DELETE'
            ELSE 'UPDATE'
        END,
        datetime('now')
    );
END;
