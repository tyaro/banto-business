-- Phase 3: 作業分類マスタ（docs/domain/schema.md §1.1）。
--
-- `excluded_from_effective_rate` は「実質時間単価（移動除く）の分母から
-- 外すか」のフラグ。移動の判定にコード文字列の比較を使わないための列で
-- （AGENTS.md 3.2）、分類を増減しても採算計算のロジックを触らずに済む。
CREATE TABLE work_categories (
    code TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    excluded_from_effective_rate INTEGER NOT NULL DEFAULT 0,
    sort_order INTEGER NOT NULL,
    active INTEGER NOT NULL DEFAULT 1
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
