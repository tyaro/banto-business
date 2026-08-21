# テンプレート派生元とマージ判断の記録

---

## なぜこのファイルが必要か

Banto Business は Banto を**同梱（vendoring）**して派生している（下記「依存方式」）。パッケージもボイラープレート（Tauri設定 / CI定義 / ディレクトリ構成 / ビルドスクリプト / lint設定等）も、派生した瞬間から独立したコピーになり、Banto 側の改善が自動では反映されない。

放置すると `banto-industrial` と `banto-business` の2つの派生先で構成が徐々に乖離し、「Banto の標準構成」が実質的に存在しなくなる。

**全面同期は行わない。** Banto 側テンプレート更新時に差分を確認し、選択的に取り込み、判断をここに記録する。

---

## 現在の派生状態

| 項目 | 値 |
|---|---|
| 派生元リポジトリ | `tyaro/banto` |
| 派生元コミット | `f471ff145a0242109818f2bcd064dd6be27855de`（main、PR #160 マージ、2026-08-15 12:01 JST） |
| 派生元タグ | **なし**（`v1.2.0` から 35 コミット先行。下記「タグではなくコミットで固定する理由」参照） |
| 派生時のバージョン表記 | `1.2.0`（`workspace.package.version` / `@banto/*`。派生元の値をそのまま維持） |
| 派生日 | 2026-08-19 |
| 最終マージ確認日 | 2026-08-19 |
| 最終マージ確認時の Banto HEAD | `f471ff1`（差分なし） |
| 派生後の動作確認 | 2026-08-19 Windows 実機で `tauri dev` 起動確認済み。CI は ubuntu / windows 双方 green |

派生時の `Initial commit` は `tyaro/banto` の `f471ff1` と **tree ハッシュが完全一致**する（`80eb3a2fc2108eafb22aef6cdcc14208397459c7`）。以降の Banto 側差分は `git diff f471ff1..<新しいコミット>` で機械的に取れる。

### 依存方式：同梱（vendoring）＋コミット固定

Banto の `crates/*` と `packages/*` は **このリポジトリ内に同梱**し、`path` 依存 / `workspace:*` 参照で使う。git タグ参照の薄い構成には**しない**。

理由：

- Banto 側の `docs/publishing.md` は、タグ参照（`github:tyaro/banto#vX.Y.Z&path:packages/...` / `Cargo.toml` の `git`+`tag`）の対象を `@banto/*` パッケージと `banto-*` クレートに限定し、**「`admin_template_core`/`src-tauri` はアプリ固有のためタグ参照の対象外（`admin-template` は banto リポジトリそのものをクローンして使う前提）」**と明記している。アプリ本体は結局コピーになるため、薄い構成にしても二重管理が残る
- テンプレートの公式導線（`scripts/rename.mjs` / `scripts/scaffold.mjs`）がモノレポ一式のコピーを前提に作られている

### タグではなくコミットで固定する理由

派生元 `f471ff1` にはタグが付いていない（直近タグ `v1.2.0` は 35 コミット前）。Banto のタグ運用規約（`docs/publishing.md`）は「マイルストーンマージ毎にタグを打たない。消費側が固定参照する必要がある破壊的変更時のみ更新する」であり、タグが常に最新 main を指すとは限らない。

`v1.2.0` へ巻き戻すと、派生アプリに直接効く修正（issue #150 / ADR-0007 の「派生アプリ dev で `.svelte.ts` が 500 になる」回避、REST/Tauri 両経路の監査対称是正、`rename.mjs` の keyring 識別子追随など）を失う。したがって **固定単位はタグではなくコミット SHA** とする。

> この方針は `CLAUDE.md` 第3章に反映済み（2026-08-19、Phase 0 で決定）。

---

## テンプレート由来ファイル

派生時にテンプレートから持ってきたファイル。**このリストにあるものは、Banto 側更新時に差分確認の対象とする。**

同梱方式のため、**Business 固有の追加物を除くツリー全体がテンプレート由来**である。したがって「由来リスト」ではなく **Business 側で改変・追加したものの一覧**を保守する方が実用的なので、この節はその形で維持する（差分確認は `git diff f471ff1..<新しいコミット>` を全体に対して取り、下表の改変済みパスだけ手動で突き合わせる）。

### Business 側で改変したテンプレート由来ファイル（Phase 0 時点）

| ファイル / ディレクトリ | 改変内容 | 上流差分適用時の注意 |
|---|---|---|
| `package.json` | `name` → `banto-business` / `description` / `--filter` 参照 | 名称関連の行は上書きしない |
| `apps/admin-template/package.json` | `name` → `banto-business-app` | 同上 |
| `apps/admin-template/src-tauri/tauri.conf.json` | `productName` / `identifier`（`dev.banto.business`）/ ウィンドウタイトル | 同上。CSP は上流に追随する（`verify-architecture` rule 12 が `security_headers.rs` と照合） |
| `apps/admin-template/src-tauri/src/keyring_store.rs` | keyring `SERVICE_NAME` → `dev.banto.business` | 同上 |
| `apps/admin-template/src/app.html` | `<title>` / `apple-mobile-web-app-title` | 同上 |
| `apps/admin-template/static/manifest.webmanifest` | PWA `name` / `short_name` | 同上 |
| `apps/admin-template/src/lib/components/Sidebar.svelte` | ブランド表示 | 同上 |
| `apps/admin-template/src/routes/login/+page.svelte` | ブランド表示・見出し | 同上 |
| `apps/admin-template/src/routes/panel/[id]/+page.svelte` | `<title>` 接尾辞 | 同上 |
| `e2e/tests/smoke.spec.ts` / `e2e/visual/visual.spec.ts` / `e2e/visual/a11y.spec.ts` | ログイン見出しアサーション | 同上 |
| `e2e/playwright.config.ts` | `--filter` 参照 | 同上 |
| `Cargo.toml` | `workspace.package.repository` / `workspace.dependencies` に `reqwest` を追加（Phase 8 同期のクライアント側） | `workspace.package.version`（`1.2.0`）は派生元の値のまま。上流に追随する。`reqwest` は `tauri` が既に引いている版を名指ししただけで、依存グラフにクレートは増えない（`docs/domain/sync.md` 9節） |
| `.github/workflows/ci.yml` | i18n ジョブの `--filter` 参照（`rename.mjs` が書き換え漏れした箇所） | 名称の行のみ Business 固有 |
| `README.md` | 冒頭に Business 向けの案内ブロックを追加（本文は Banto の原文のまま） | 追加ブロックより下は上流差分をそのまま適用してよい |
| `.gitignore` | 末尾に業務データ（`*.sqlite3` / `backups/` / `attachments/`）の除外を追加 | 公開リポジトリのため必須。上流差分適用時に消さない |
| `.claude/launch.json` | 作業ディレクトリを `D:\develop\banto` → `D:\develop\banto-business` に修正 | 上流の設定変更は取り込んでよいが、パスは Business のものを維持 |
| `CLAUDE.md` / `AGENTS.md` / `docs/plan.md` ほか Business ドキュメント | Business 固有（テンプレート由来ではない） | 差分確認の対象外 |

上記以外は**未改変**なので、上流差分をそのまま適用してよい。

### 未改変で残しているテンプレート由来の記述

`apps/admin-template/**` の Rust コメント・`e2e/visual/README.md` の案内文言に `pnpm --filter admin-template ...` という旧アプリ名の例が残っている（機能に影響しない文言のみ。上流との差分を無用に増やさないため、あえて追随していない）。ディレクトリ名 `apps/admin-template` と crate 名 `admin-template` / `admin-template-core` も同じ理由で維持する（Phase 0 決定：497 箇所の参照書き換えより上流差分の取りやすさを優先）。

### 派生時に削除したもの

| 対象 | 理由 |
|---|---|
| `.github/workflows/deploy-demo.yml` | Banto 本体の GitHub Pages ライブデモ公開用。Business では不要 |
| `.github/workflows/template-acceptance.yml` | テンプレート自身（clone→rename→動く）の受け入れ検査。派生先では不要 |

`packages/scan-wedge` は残す（未配線で無害）。

### [2026-08-20] Phase 7 直前 — `items` デモ一式の削除

Phase 0 では「`docs/recipes/add-resource.md` がリソース追加の正式手順を『`items` のルート一式をコピーして書き換えること』と定めており、Phase 2（Customer / Project）の手本になる」という理由で残していた。Phase 6 までで Business 側のリソースが9本揃い、手本としての役目が終わったため、実運用（Phase 7）に入る前に削除した。

削除した範囲:

| 層 | 対象 |
|---|---|
| Rust サービス層 | `core/src/items.rs`・`core/src/rest/items.rs` |
| Rust 配線 | `lib.rs` の `pub mod items`・`rest/mod.rs` の `Services.items` とルータ・`bin/banto-serve.rs`・`src-tauri` の `items_*` コマンド7本と `AppState.items` |
| DB | `items` テーブルの `DROP`（`0021_drop_items_demo.sql`、両方言）。`0001_items.sql` は適用済みなので編集していない（CLAUDE.md 第5章） |
| デモシード | `core/src/db.rs` の 1,000 行シード一式（`items` が唯一の投入先だった）。初回起動時の DB は**空**になる |
| フロント | `routes/(app)/items/`・`resources/items.ts`・`itemsAdmin.ts`・ナビ項目・`items.*` / `panels.*` の i18n キー |
| ダッシュボード | `items` から集計していたチャート群・ドック・パネルのポップアウト一式（`dashboard.ts` / `panels.ts` / `popout.ts` / `DashboardPanel.svelte` / `routes/panel/[id]` / `sampleData.ts`）。ダッシュボードは未入金・期限超過のパネルだけになった |
| CSV エクスポート | `items_export_csv_to_folder`（Tauri）と `AppState.exports_dir`。呼び出し元がグリッドの `items` 一覧しか無かったため |
| E2E | items の CRUD / CSV / 日報シナリオ。RBAC・監査・添付のシナリオは `customers` / `expenses` へ振り替えた |
| Banto ツール | `scripts/scaffold.mjs` と `scripts/scaffold.test.mjs`（後述） |

**同梱 Banto に1箇所だけ手を入れた**（CLAUDE.md 第3章の例外。利用者に確認済み）:

- `crates/banto-admin-services/src/backup.rs` の `REQUIRED_TABLES` から `items` を外し、`["settings", "users", "audit_log"]` にした。この検査の意図は「Banto の DB かどうかの粗い判定」であり、判定対象は Banto が必ず作るテーブルに限るのが筋。`items` を残したままだと、Banto 自身が削除を促しているデモテーブルを消した瞬間に**バックアップ復元が全滅する**（作成側は成功するので、気付くのは復元しようとした時）。`docs/banto-feedback.md` に Banto 共通の問題として記録済み。上流差分を取り込むときは、この1行が上書きされていないか確認すること。

**`scripts/scaffold.mjs` / `scripts/scaffold.test.mjs` を削除した。** 新しいテンプレートのコピーから不要な資産を「引く」ためのツールで、`items` のルート一式・ダッシュボードのドック配線を行単位で直書きしている。Phase 0 を通過した派生リポジトリでは二度と実行しないうえ、上の削除でパターンが全て失われて実行すれば失敗する。CI からも npm script からも呼ばれていない（`scaffold.test.mjs` はどの `pnpm test` にも配線されていなかった）。`README.md` 本文（原文のまま保持）と `docs/scaffold-presets-plan.md` には `pnpm scaffold` の記述が残るが、冒頭の案内ブロックのとおり Banto 本体向けの記述として読むこと。

`docs/recipes/add-resource.md` の手本も `items` を前提に書かれているが、これは Banto の文書なので原文のまま残す。このリポジトリで手本にするなら `customers`（最小構成）または `expenses`（添付あり）を読むこと。

---

## 更新手順

Banto 側テンプレートが更新された場合：

```
1. Banto の差分を確認（固定単位はタグではなくコミット）
   git -C ../banto fetch origin main
   git -C ../banto diff <前回の派生元コミット>..<新しいコミット>

2. 変更を3分類する
   - 取り込む       … バグ修正・セキュリティ・共通基盤の改善
   - 見送る         … Industrial固有 / Businessに不要 / 改変済みで競合
   - 保留           … 判断に時間が必要

3. 取り込む変更を手動適用

4. ビルド・テスト・アプリ起動を確認

5. 下の「マージ判断ログ」に記録

6. 「現在の派生状態」の派生元コミット・最終マージ確認日を更新
```

**「とりあえず全部取り込む」をしない。** 改変済みファイルへの機械的な適用は、アプリ名変更等の派生時カスタマイズを壊す。

---

## マージ判断ログ

<!--
新しい記録を下に追記する。

### [YYYY-MM-DD] Banto vA.B.C → vX.Y.Z

| 変更 | 判断 | 理由 |
|---|---|---|
|  | 取り込む / 見送る / 保留 |  |

確認：ビルド [ ] / テスト [ ] / 起動 [ ]
-->

### [記録例・削除可] YYYY-MM-DD Banto v0.1.0 → v0.2.0

| 変更 | 判断 | 理由 |
|---|---|---|
| CI に Windows ビルドを追加 | 取り込む | 共通基盤の改善。Business でも必要 |
| `tauri.conf.json` のデフォルトウィンドウサイズ変更 | 見送る | 派生時に Business 向けに調整済み。上書きすると劣化 |
| Industrial向けデモリソースの追加 | 見送る | Business に不要 |

確認：ビルド [ ] / テスト [ ] / 起動 [ ]

---

## 乖離の許容範囲

以下は Industrial との乖離を**許容する**（Business 固有の要件によるもの）。

| 項目 | 乖離内容 | 理由 |
|---|---|---|
| アプリ名・識別子・ブランド表示 | `banto-business` / `Banto Business` / `dev.banto.business` | 派生アプリとして当然の乖離 |
| CI ワークフロー構成 | `deploy-demo` / `template-acceptance` を削除 | テンプレート本体固有のジョブ |
| ディレクトリ名・crate 名 | `apps/admin-template` / `admin-template` / `admin-template-core` を**改名せず維持** | 上流差分をパス単位でそのまま取るため（Phase 0 決定） |

以下は乖離を**許容しない**（Banto 標準構成として揃えるべきもの）。

| 項目 | 理由 |
|---|---|
| Rust / Node のバージョン | ビルド再現性 |
| lint / format 設定 | コードスタイルの一貫性 |
| マイグレーションの管理方式 | Banto 標準手順の維持 |
| CI の基本構成 | 品質基準の統一 |
