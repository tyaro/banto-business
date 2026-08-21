-- Phase 8: 未解決の衝突（`docs/domain/sync.md` 11.7）。
--
-- ## なぜ保存が要るのか
--
-- 衝突した行は取り込まれずに応答へ返るだけで、**PC は保留状態を持たない**
-- （2026-08-21 決定）。一方で進捗（`sync_state`）は衝突があっても進むので、
-- 差し戻された行が次の同期でもう一度送られてくることはない。
--
-- つまり受け取った側がここへ**書き留めてから進捗を進めない限り、その編集は
-- 二度と現れない**。画面を閉じたら消える持ち方にすると、片方の端末で入れた
-- 工数が黙って消える。解決させる画面より先に、入れ物を用意する。
--
-- ## 行の中身を JSON で持つ理由
--
-- 衝突の両側は8テーブルぶんの異なる列構成を取る（`sync/rows.rs` の目録）。
-- 型付きの列で持つと目録が増えるたびにここも変える羽目になるので、
-- `SyncRow` をそのまま JSON 文字列で持つ。**業務データの複製ではなく、
-- 利用者に選ばせるための一時的な控え**なので、検索も集計もしない。
--
-- 外部キーを張らないのは、片側が墓石（削除済み）でも保持する必要があるため
-- （CLAUDE.md 第5章のスナップショット系と同じ理由）。
CREATE TABLE sync_conflicts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    peer_device_id INTEGER NOT NULL,
    table_name TEXT NOT NULL,
    row_key TEXT NOT NULL,
    -- BOTH_CHANGED / INVOICED_FROZEN（`ConflictReason`）。
    reason TEXT NOT NULL,
    -- どちらも `SyncRow` の JSON。
    local_row TEXT NOT NULL,
    incoming_row TEXT NOT NULL,
    detected_at TEXT NOT NULL,
    -- 解決済みは残す（同じ行で繰り返し揉めていることが分かる）。
    resolved_at TEXT
);

-- 「未解決を新しい順に並べる」と「同じ行の未解決を差し替える」の両方で使う。
CREATE INDEX idx_sync_conflicts_open ON sync_conflicts (resolved_at, table_name, row_key);
