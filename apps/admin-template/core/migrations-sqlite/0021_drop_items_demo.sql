-- テンプレート由来のデモリソース `items` を削除する（Phase 7 実運用前の片付け。
-- 経緯は docs/template-origin.md「[2026-08-20] Phase 7 直前 — `items` デモ一式の削除」）。
--
-- `0001_items.sql` は適用済みなので編集しない（CLAUDE.md 第5章「マイグレーション
-- は前方向のみ」）。破壊的変更は必ず新しいマイグレーションで行う。
--
-- 業務データは持たないテーブル（架空のデモ商品のみ）なので、退避せずに落とす。
-- 添付は `attachments` 側に `resource = 'items'` として残りうる。参照先が消える
-- 前に先に消しておかないと、どのレコードにも辿り着けない行が残る。
DELETE FROM attachments WHERE resource = 'items';
DROP TABLE IF EXISTS items;
