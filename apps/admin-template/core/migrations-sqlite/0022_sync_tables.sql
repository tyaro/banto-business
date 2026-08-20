-- Phase 8: デバイス間同期の土台（`docs/domain/sync.md` 4節）。
--
-- ## なぜ `updated_at` を使わないか
--
-- 全テーブルが `updated_at` を持つが、中身は `datetime('now')` の **UTC 秒精度**
-- （`banto_storage::Dialect::now_expr`）。「前回同期以降に変わった行」を
-- `updated_at > 最終同期時刻` で拾うと、同一秒内の更新を取りこぼす。端末間の
-- 時計ずれの影響も受ける。金額データでこれは踏みたくない。
--
-- 代わりに端末内で単調増加する `seq` を持ち、「相手が持っている最後の seq より
-- 後」を送る。時計に一切依存しない。
--
-- ## `row_key` が TEXT である理由
--
-- 双方向同期の対象8テーブルのうち3つ（`work_categories` /
-- `expense_categories` / `cost_rates`）は **TEXT の主キー**（利用者が決める
-- コード）。整数 PK の5テーブルと1つの表で扱えるよう、整数 PK は10進文字列に
-- して入れる。
CREATE TABLE sync_outbox (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    table_name TEXT NOT NULL,
    row_key TEXT NOT NULL,
    -- INSERT / UPDATE / DELETE。DELETE は論理削除（`deleted_at` を立てた）
    -- ことを指す（0023）。物理削除は行わない。
    op TEXT NOT NULL,
    changed_at TEXT NOT NULL
);

-- 同期は常に「seq > N」で読むので、主キーの昇順走査で足りる。
-- `(table_name, row_key)` の索引は衝突判定（同じ行が双方で変わったか）で使う。
CREATE INDEX idx_sync_outbox_row ON sync_outbox (table_name, row_key);

-- 相手端末ごとの進捗。
--
-- `peer_device_id` は相手のデバイス番号（`docs/domain/sync.md` 3.4）。
-- `sent_through_seq`   … 自分の outbox のうち、相手へ送り終えた最後の seq
-- `received_through_seq` … 相手の outbox のうち、取り込み終えた最後の seq
--
-- 相手が複数になっても行が増えるだけで済むよう、端末ごとに1行持つ。
CREATE TABLE sync_state (
    peer_device_id INTEGER PRIMARY KEY,
    sent_through_seq INTEGER NOT NULL DEFAULT 0,
    received_through_seq INTEGER NOT NULL DEFAULT 0,
    last_synced_at TEXT
);
