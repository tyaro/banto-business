-- PostgreSQL port of migrations-sqlite/0009_work_categories.sql（conventions §11）。
--   INTEGER（真偽値・並び順） -> BIGINT（Rust 側は i64 で読む）
--   シード行は SQLite 版と同一（コード・表示名・移動フラグ・並び順）
CREATE TABLE work_categories (
    code TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    excluded_from_effective_rate BIGINT NOT NULL DEFAULT 0,
    sort_order BIGINT NOT NULL,
    active BIGINT NOT NULL DEFAULT 1
);

INSERT INTO work_categories (code, name, excluded_from_effective_rate, sort_order) VALUES
    ('DESIGN', '設計', 0, 10),
    ('PLC', 'PLC開発', 0, 20),
    ('SCADA', 'SCADA開発', 0, 30),
    ('PC_APP', 'PCアプリ開発', 0, 40),
    ('TEST', 'テスト', 0, 50),
    ('INTERNAL', '社内調整', 0, 60),
    ('ONSITE', '現地作業', 0, 70),
    ('TRAVEL', '移動', 1, 80),
    ('MEETING', '打合せ', 0, 90),
    ('OTHER', 'その他', 0, 100);
