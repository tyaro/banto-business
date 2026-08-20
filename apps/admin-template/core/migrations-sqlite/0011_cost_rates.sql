-- Phase 3: 内部原価レート（docs/domain/schema.md §1.2）。
--
-- **このテーブルは採算計算では参照しない**（CLAUDE.md 1.2）。参照するのは
-- work_logs.applied_rate に焼き付けた値。ここは「新規入力時の既定値の
-- 供給源」でしかなく、後から単価を変えても過去の採算は動かない。
--
-- 行が無い分類は「レート未設定」を意味する。0 円のダミー行を撒くと
-- 「単価0で記録された工数」と区別できなくなるため、あえて空で始める。
CREATE TABLE cost_rates (
    work_category_code TEXT PRIMARY KEY,
    hourly_rate INTEGER NOT NULL,
    updated_at TEXT NOT NULL
);
