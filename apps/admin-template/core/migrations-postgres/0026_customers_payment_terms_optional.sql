-- PostgreSQL port of migrations-sqlite/0026_customers_payment_terms_optional.sql
-- （conventions §11）。Postgres は列を後から NULL 許容にできるので
-- ALTER COLUMN で足りる（SQLite側のようなテーブル再作成は不要）。
ALTER TABLE customers ALTER COLUMN closing_day DROP NOT NULL;
ALTER TABLE customers ALTER COLUMN payment_month_offset DROP NOT NULL;
ALTER TABLE customers ALTER COLUMN payment_day DROP NOT NULL;
