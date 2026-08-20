-- PostgreSQL port of migrations-sqlite/0023_tombstones.sql（conventions §11）。
--   deleted_at は TEXT のまま（SQLite と同じ ISO 文字列）
--
-- 論理削除（トゥームストーン。`docs/domain/sync.md` 5節）。
--
-- 行が消えているとき、「削除された」のか「まだ同期で届いていない」のかは
-- 区別できない。物理削除をやめ、`deleted_at`（NULL なら生存）を立てる。
--
-- **この 0023 は列を足すだけで、既存の挙動は変えない。** 物理削除を論理削除へ
-- 置き換え、全ての参照へ `deleted_at IS NULL` を入れるのは次の段（サービス層）
-- で行う。列の追加と参照の総書き換えを 1 回に混ぜると、レビューで
-- 「どの変更がどちらの都合か」が読めなくなる。
--
-- 対象は双方向同期の 8 テーブルのみ。請求・入金は同期しない（PC 専用）ので
-- 付けない — 付けると「同期のために要る列」と「そうでない列」の区別が
-- 曖昧になる。
--
-- トゥームストーンは消さない。消すと、相手がまだ知らない削除が
-- 「知らない行」に戻り、次の同期で復活する。個人事業の規模なら件数は問題に
-- ならない。
ALTER TABLE customers ADD COLUMN deleted_at TEXT;
ALTER TABLE projects ADD COLUMN deleted_at TEXT;
ALTER TABLE work_categories ADD COLUMN deleted_at TEXT;
ALTER TABLE expense_categories ADD COLUMN deleted_at TEXT;
ALTER TABLE cost_rates ADD COLUMN deleted_at TEXT;
ALTER TABLE trips ADD COLUMN deleted_at TEXT;
ALTER TABLE work_logs ADD COLUMN deleted_at TEXT;
ALTER TABLE expenses ADD COLUMN deleted_at TEXT;
