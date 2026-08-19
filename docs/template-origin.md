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
| `Cargo.toml` | `workspace.package.repository` | `workspace.package.version`（`1.2.0`）は派生元の値のまま。上流に追随する |
| `.github/workflows/ci.yml` | i18n ジョブの `--filter` 参照（`rename.mjs` が書き換え漏れした箇所） | 名称の行のみ Business 固有 |
| `README.md` | 冒頭に Business 向けの案内ブロックを追加（本文は Banto の原文のまま） | 追加ブロックより下は上流差分をそのまま適用してよい |
| `CLAUDE.md` / `AGENTS.md` / `docs/plan.md` ほか Business ドキュメント | Business 固有（テンプレート由来ではない） | 差分確認の対象外 |

上記以外は**未改変**なので、上流差分をそのまま適用してよい。

### 未改変で残しているテンプレート由来の記述

`apps/admin-template/**` の Rust コメント・`e2e/visual/README.md`・`scripts/scaffold.mjs` の案内文言に `pnpm --filter admin-template ...` という旧アプリ名の例が残っている（機能に影響しない文言のみ。上流との差分を無用に増やさないため、あえて追随していない）。ディレクトリ名 `apps/admin-template` と crate 名 `admin-template` / `admin-template-core` も同じ理由で維持する（Phase 0 決定：497 箇所の参照書き換えより上流差分の取りやすさを優先）。

### 派生時に削除したもの

| 対象 | 理由 |
|---|---|
| `.github/workflows/deploy-demo.yml` | Banto 本体の GitHub Pages ライブデモ公開用。Business では不要 |
| `.github/workflows/template-acceptance.yml` | テンプレート自身（clone→rename→動く）の受け入れ検査。派生先では不要 |

**Phase 0 では `items` デモを削除しない。** `docs/recipes/add-resource.md` が「リソース追加の唯一の正式手順は `items` のルート一式をコピーして書き換えること」と定めており、Phase 2（Customer / Project）の手本になるため。**Phase 2 完了後に削除する**（対象ファイルの全量は `README.md` §2 の表）。

`packages/scan-wedge` も残す（未配線で無害。削除すると `scripts/scaffold.*` と上流差分に不要な乖離が出る）。

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
