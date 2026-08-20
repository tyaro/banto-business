-- PostgreSQL port of migrations-sqlite/0024_sync_outbox_triggers.sql（conventions §11）。
--   SQLite は表ごとに INSERT/UPDATE の2トリガを直書きする。PostgreSQL は
--   トリガ関数を1つ置き、主キー列名を引数で渡して8表から共有する。
--   `NOW()::text` は SQLite の `datetime('now')` に合わせた TEXT 化（SQLite に
--   日時型が無いので、この列は両方言とも TEXT）。
--
-- 設計の背景（なぜサービス層ではなくトリガか）は SQLite 側の冒頭コメントを参照。

CREATE FUNCTION sync_record_change() RETURNS trigger AS $$
DECLARE
    key_value text;
BEGIN
    -- 主キーの列名は表ごとに違う（id / code / work_category_code）ので、
    -- トリガ引数で受け取って動的に取り出す。
    EXECUTE format('SELECT ($1).%I::text', TG_ARGV[0]) INTO key_value USING NEW;

    INSERT INTO sync_outbox (table_name, row_key, op, changed_at)
    VALUES (
        TG_TABLE_NAME,
        key_value,
        CASE
            WHEN TG_OP = 'INSERT' THEN 'INSERT'
            WHEN OLD.deleted_at IS NULL AND NEW.deleted_at IS NOT NULL THEN 'DELETE'
            ELSE 'UPDATE'
        END,
        NOW()::text
    );
    -- AFTER トリガの戻り値は使われない。
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER sync_outbox_customers
AFTER INSERT OR UPDATE ON customers
FOR EACH ROW EXECUTE FUNCTION sync_record_change('id');

CREATE TRIGGER sync_outbox_projects
AFTER INSERT OR UPDATE ON projects
FOR EACH ROW EXECUTE FUNCTION sync_record_change('id');

CREATE TRIGGER sync_outbox_trips
AFTER INSERT OR UPDATE ON trips
FOR EACH ROW EXECUTE FUNCTION sync_record_change('id');

CREATE TRIGGER sync_outbox_work_logs
AFTER INSERT OR UPDATE ON work_logs
FOR EACH ROW EXECUTE FUNCTION sync_record_change('id');

CREATE TRIGGER sync_outbox_expenses
AFTER INSERT OR UPDATE ON expenses
FOR EACH ROW EXECUTE FUNCTION sync_record_change('id');

CREATE TRIGGER sync_outbox_work_categories
AFTER INSERT OR UPDATE ON work_categories
FOR EACH ROW EXECUTE FUNCTION sync_record_change('code');

CREATE TRIGGER sync_outbox_expense_categories
AFTER INSERT OR UPDATE ON expense_categories
FOR EACH ROW EXECUTE FUNCTION sync_record_change('code');

CREATE TRIGGER sync_outbox_cost_rates
AFTER INSERT OR UPDATE ON cost_rates
FOR EACH ROW EXECUTE FUNCTION sync_record_change('work_category_code');
