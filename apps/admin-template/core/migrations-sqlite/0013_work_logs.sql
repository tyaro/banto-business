-- Phase 3: 工数（docs/domain/schema.md §3.2）。
--
-- applied_rate: 記録時点の時間単価を**行に焼き付ける**（CLAUDE.md 1.2）。
--   cost_rates への外部キーは張らない — マスタを消しても過去の採算が
--   壊れてはならないため（CLAUDE.md 第5章のスナップショット列の扱い）。
-- internal_cost: floor(minutes × applied_rate ÷ 60) を保存する。
--   都度計算にすると集計側で「合計してから丸める」実装が紛れ込みうるが、
--   Phase 1 決定 C-1 は「行ごとに丸めた原価の合計」と定めている。列として
--   持つことでその定義を DB レベルで固定する。
CREATE TABLE work_logs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER NOT NULL REFERENCES projects(id),
    trip_id INTEGER REFERENCES trips(id) ON DELETE SET NULL,
    worked_on TEXT NOT NULL,
    work_category_code TEXT NOT NULL,
    minutes INTEGER NOT NULL,
    applied_rate INTEGER NOT NULL,
    internal_cost INTEGER NOT NULL,
    description TEXT,
    invoiced INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX idx_work_logs_project ON work_logs(project_id, worked_on);
CREATE INDEX idx_work_logs_trip ON work_logs(trip_id);
