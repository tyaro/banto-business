-- PostgreSQL port of migrations-sqlite/0018_projects_billing_rate.sql（conventions §11）。
--   金額の INTEGER -> BIGINT
ALTER TABLE projects ADD COLUMN billing_hourly_rate BIGINT;
