-- Phase 3: 出張（docs/domain/schema.md §3.1）。
--
-- Trip は「出張というまとまり」であり、工数・経費の実体ではない。削除時は
-- 生成物（work_logs / expenses）を消さず trip_id を NULL 化する
-- （Phase 1 決定 C-6：工数実績が消えると案件採算が壊れるため）。
-- そのため参照側の外部キーは ON DELETE SET NULL。
CREATE TABLE trips (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id INTEGER NOT NULL REFERENCES projects(id),
    destination TEXT NOT NULL,
    start_on TEXT NOT NULL,
    end_on TEXT NOT NULL,
    onsite_days INTEGER NOT NULL,
    nights INTEGER NOT NULL,
    note TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX idx_trips_project ON trips(project_id);
