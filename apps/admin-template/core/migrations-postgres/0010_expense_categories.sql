-- PostgreSQL port of migrations-sqlite/0010_expense_categories.sql（conventions §11）。
-- 経費分類マスタ（docs/domain/schema.md §1.3）。整数列は BIGINT。
--
-- `default_tax_category` は仕入側の既定税区分。全て STANDARD_10 で、
-- 国際線（不課税）などは行ごとに変更する（Phase 1 決定 B-8 / tax-calculation.md 3）。
CREATE TABLE expense_categories (
    code TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    default_tax_category TEXT NOT NULL,
    sort_order BIGINT NOT NULL,
    active BIGINT NOT NULL DEFAULT 1
);

INSERT INTO expense_categories (code, name, default_tax_category, sort_order) VALUES
    ('TRANSPORT', '交通費', 'STANDARD_10', 10),
    ('LODGING', '宿泊費', 'STANDARD_10', 20),
    ('SHIPPING', '送料', 'STANDARD_10', 30),
    ('MATERIAL', '部材費', 'STANDARD_10', 40),
    ('SUPPLIES', '消耗品', 'STANDARD_10', 50),
    ('OUTSOURCE', '外注費', 'STANDARD_10', 60),
    ('OTHER', 'その他', 'STANDARD_10', 70);
