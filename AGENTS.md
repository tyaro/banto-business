# AGENTS.md

Banto Business の開発手順書。AIエージェント・開発者共通。

**常時適用の規約は `CLAUDE.md` を参照すること。このファイルは手順と構造を扱う。**

---

## 1. リポジトリ構成

Banto を**同梱（vendoring）**したモノレポ構成。`crates/*` と `packages/*` は Banto 由来のため **Business 都合で書き換えない**（CLAUDE.md 第3章）。

```
banto-business/
├── CLAUDE.md                 # 常時適用規約（最優先）
├── AGENTS.md                 # このファイル
├── docs/
│   ├── plan.md               # 開発計画書
│   ├── tax-calculation.md    # 税計算仕様（Phase 1で確定）
│   ├── banto-feedback.md     # Bantoへのフィードバックログ
│   ├── template-origin.md    # 派生元コミットとマージ判断の記録
│   ├── domain/               # Phase 1 成果物（ER図・状態遷移・用語集）
│   ├── adr/                  # 設計判断記録
│   └── （その他は Banto 由来のテンプレート文書）
├── crates/                   # ★Banto 由来（改変しない）
│   └── banto-{core,storage,server,attachments,admin-services}
├── packages/                 # ★Banto 由来（改変しない）
│   └── @banto/{admin-core,grid-svelte,forms,charts,report,theme,...}
├── apps/admin-template/      # アプリ本体（ディレクトリ名は Phase 0 決定で維持）
│   ├── core/                 # Rust ドメイン/サービス層（tauri/axum 非依存）
│   │   ├── src/
│   │   │   ├── domain/       # ★Business：採算・税計算・消込（純粋関数）
│   │   │   ├── <resource>.rs # ★Business：サービス層（customer.rs 等）
│   │   │   └── rest/         # ★Business：REST ルート
│   │   ├── migrations-sqlite/
│   │   └── migrations-postgres/
│   ├── src-tauri/src/lib.rs  # Tauri コマンド（薄く保つ）
│   └── src/                  # SvelteKit フロントエンド
│       ├── lib/banto/resources/  # ★Business：リソース定義（スキーマ）
│       ├── lib/components/       # ★Business固有UI（汎用UIは @banto/* から）
│       └── routes/(app)/<resource>/
└── e2e/
```

**新しい CRUD リソースの追加は `docs/recipes/add-resource.md` の手順に従う**（`items` のルート一式をコピーして書き換えるのが Banto の正式方式。動的ルート生成は不採用）。

### ディレクトリ責務

| ディレクトリ                                   | 置くもの                                                                              | 置かないもの        |
| ---------------------------------------------- | ------------------------------------------------------------------------------------- | ------------------- |
| `apps/admin-template/core/src/domain/`         | 採算計算・税計算・消込ロジック（純粋関数中心）                                        | DB / Tauri 依存     |
| `apps/admin-template/core/src/<resource>.rs`   | SQLクエリ・永続化・CRUD（sqlx。`column_map()` でソート/フィルタ列をホワイトリスト化） | 金額の業務ロジック  |
| `apps/admin-template/core/src/rest/`           | REST ルーティング（`RoleGuard` + `record_write`）                                     | 業務ロジック        |
| `apps/admin-template/src-tauri/src/lib.rs`     | Tauriコマンド定義（薄く保つ。REST と**同一の**認可・監査）                            | 業務ロジック        |
| `apps/admin-template/src/lib/banto/resources/` | リソース定義・スキーマ                                                                | 金額計算            |
| `apps/admin-template/src/lib/components/`      | Business固有UI                                                                        | Bantoが提供済みのUI |
| `crates/` `packages/`                          | （Banto 由来。触らない）                                                              | Business のコード   |

**金額計算は必ず `apps/admin-template/core/src/domain/` に置く。フロントエンドで金額計算をしない。**
（表示フォーマットのみフロント側で行う）

---

## 2. 使用 Banto バージョン

| 項目           | 値                                                              |
| -------------- | --------------------------------------------------------------- |
| 依存方式       | **同梱（vendoring）＋派生元コミット固定**（CLAUDE.md 第3章）    |
| 派生元コミット | `f471ff1`（`tyaro/banto` main、2026-08-15）                     |
| 派生元タグ     | なし（`v1.2.0` + 35 コミット。バージョン表記は `1.2.0` のまま） |
| 最終確認日     | 2026-08-19                                                      |

更新手順は `docs/template-origin.md` を参照。

### 利用中の Banto パッケージ

同梱しているため全て利用可能。下表の「配線」は**テンプレート既定でアプリに配線済みか**（`apps/admin-template/src` からの import 実績）、「Business での用途」は本アプリで担わせる役割。

**フロントエンド**

| パッケージ           | 配線             | Business での用途                                                |
| -------------------- | ---------------- | ---------------------------------------------------------------- |
| `@banto/admin-core`  | 済（33ファイル） | アプリシェル・CRUD 基盤・リソース定義                            |
| `@banto/grid-svelte` | 済（8）          | 一覧（案件 / 工数 / 請求 / 入金）                                |
| `@banto/forms`       | 済（5）          | 入力フォーム全般                                                 |
| `@banto/theme`       | 済（3）          | テーマ                                                           |
| `@banto/dock-svelte` | 済（3）          | ダッシュボードパネル（Phase 4 以降）                             |
| `@banto/charts`      | 済（3）          | Phase 4：案件採算のグラフ                                        |
| `@banto/report`      | 済（2）          | Phase 5：適格請求書 PDF                                          |
| `@banto/attachments` | 済（3）          | Phase 3：領収書の参照コピー（正本は会計ソフト側／CLAUDE.md 1.6） |
| `@banto/tree-svelte` | 済（3）          | 現時点で用途なし（テンプレート由来のまま）                       |
| `@banto/scan-wedge`  | 未配線           | 用途なし（同梱のまま保持）                                       |

**Rust**

| クレート               | 用途                              |
| ---------------------- | --------------------------------- |
| `banto-core`           | 共通型・`BantoError`              |
| `banto-storage`        | 永続化（SQLite。`list_query` 等） |
| `banto-server`         | LAN 向け REST サーバ              |
| `banto-admin-services` | settings / audit / users（RBAC）  |
| `banto-attachments`    | Phase 3：領収書添付               |

---

## 3. ドメインルール

### 3.1 エンティティ関連

```
Customer
   ↓
Project
 ├─ WorkLog ─┐
 ├─ Trip ────┤ (Trip は WorkLog / Expense を生成)
 └─ Expense ─┘

Customer ─→ Invoice ─→ InvoiceLine ─→ Project
                ↕
        PaymentAllocation
                ↕
             Payment
```

### 3.2 WorkLog

作業分類：設計 / PLC開発 / SCADA開発 / PCアプリ開発 / テスト / 社内調整 / 現地作業 / **移動** / 打合せ / その他

- `applied_rate` を記録時に焼き付ける（`CLAUDE.md` 1.2）
- 「移動」分類は実質時間単価の計算で分母から除外する系統を持つため、**分類の識別子を文字列比較でハードコードしない**（enum / コード値で扱う）

### 3.3 Trip

Trip 登録から以下を一括生成する：

```
Trip
 ├─ WorkLog（移動）× 往復2件
 ├─ WorkLog（現地作業）× 現地作業日数
 ├─ Expense（交通費）
 └─ Expense（宿泊費）
```

- 生成物は通常の WorkLog / Expense レコード。生成後は個別に編集可能
- `trip_id` で紐づけを保持する
- **Trip 削除時の挙動は Phase 1 で決定する**（現時点で実装しない。既定案：生成物は残し `trip_id` を NULL 化）

### 3.4 Expense

分類：交通費 / 宿泊費 / 送料 / 部材費 / 消耗品 / 外注費 / その他

- 税区分（10% / 8%軽減 / 不課税 / 非課税）を必ず保持
- `billable`（顧客請求対象か）と `invoiced`（請求済か）は別フラグ

### 3.5 案件採算

```
案件売上 - 工数原価 - 直接経費 = 案件粗利
```

実質時間単価は **2種を必ず併記**：

| 指標     | 分母                            |
| -------- | ------------------------------- |
| 移動込み | 全WorkLog時間                   |
| 移動除く | 分類「移動」を除いたWorkLog時間 |

片方のみを返すAPIを作らない。

### 3.6 Invoice 状態遷移

```
Draft → Issued → Partially Paid ⇄ Paid
                      ↓
                  Cancelled
```

- `Waiting Payment` は使わない（`Issued` に統合）
- `Overdue` は状態ではなく導出値（`CLAUDE.md` 1.5）
- `Issued` 以降、明細は編集不可。訂正は `Cancelled` + 新規発行（赤伝）

### 3.7 Payment 消込

`PaymentAllocation` の差額理由コード：

- `TRANSFER_FEE` — 振込手数料
- `WITHHOLDING` — 源泉徴収
- `DISCOUNT` — 値引き
- `OVERPAYMENT` — 過入金
- `OTHER` — その他（備考必須）

---

## 4. DB Migration ルール

- 配置：`apps/admin-template/core/migrations-sqlite/NNNN_snake_case_description.sql`
- **SQLite と Postgres の2方言を必ず対で追加する**（`migrations-postgres/` に同名・同連番。`pnpm verify:architecture` の rule 11 が機械検査する）
- **適用済みマイグレーションは編集しない。** 変更は新規ファイルで
- 前方向のみ。down migration は用意しない（Backup/Restore で対応）
- 金額カラム：`INTEGER NOT NULL`
- 日付カラム：`TEXT`（ISO 8601 / JST の業務日付）
- スナップショット系カラムに外部キー制約を張らない
- マイグレーション追加時は必ず Backup/Restore の動作を確認する

---

## 4.1 開発コンテナでの `src-tauri` のビルド

Claude Code の作業コンテナは既定で Tauri のシステム依存を持たないため `cargo check -p admin-template` が失敗する。**「Tauri 側は検証できない」で済ませない。** 以下を一度入れれば `src-tauri` のコンパイル・テスト・clippy まで手元で通せる。

```sh
apt-get update -qq && apt-get install -y \
  libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev libsoup-3.0-dev
```

（GUI の起動確認は依然として実機が必要。ここで通せるのはコンパイルとテストまで。）

`cargo check -p admin-template` を通していないコミットを push すると、CI の Tauri check（ubuntu / windows）で初めて落ちる。**サービスを追加したときは `rest::Services` の初期化箇所が `src-tauri` 側にもある**（`start_embedded_server`）ことに注意する。

---

## 5. テスト要件

### 必須

| 対象                               | 理由                         |
| ---------------------------------- | ---------------------------- |
| 案件採算計算                       | 意思決定に直結               |
| 税計算（税率区分別集計・端数処理） | 対外文書。誤りが取引先に届く |
| Payment 消込                       | 4種の差額パターン全て        |
| Overdue 導出の境界条件             | 期限当日 / 残額0 / Cancelled |
| Trip 一括生成                      | 生成件数と内訳               |

### 任意

- UI スナップショット
- E2E

### 原則

**金額が絡むロジックは例外なくテストを書く。** テストなしでの金額ロジックのマージは不可。

税計算テストは `docs/tax-calculation.md` のケース表を網羅すること。

---

## 6. 禁止事項

- Banto が提供する機能の再実装
- Banto 本体の設計規約の複製・改変
- Business固有機能の Banto への逆流
- Banto `main` ブランチへの追従
- 金額の浮動小数点保持
- レート・単価の参照方式（スナップショットしない実装）
- `Invoice.project_id` の追加
- `Invoice ↔ Payment` の 1:1 実装
- `status` への `Overdue` 追加
- フロントエンドでの金額計算
- 適用済みマイグレーションの編集
- 税計算・採算計算・消込ロジックの推測実装（不明点は必ず質問）
- 会計機能（仕訳・決算・確定申告）の実装

---

## 7. Phase 進行

現在の Phase：**Phase 5（請求）着手可**

| Phase | 内容                                   | 状態   |
| ----- | -------------------------------------- | ------ |
| 0     | リポジトリ作成・テンプレート派生       | 完了   |
| 1     | 要件・ドメイン設計                     | 完了   |
| 2     | 基本マスター（Customer / Project）     | 完了   |
| 3     | 工数・経費（WorkLog / Trip / Expense） | 完了   |
| 4     | 採算管理                               | 完了   |
| 5     | 請求（Invoice / PDF）                  | 着手可 |
| 6     | 入金管理（Payment）                    | 未着手 |
| 7     | 実運用評価                             | 未着手 |

Phase 1 の成果物は `docs/domain/`（requirements / er-diagram / schema / state-machine / glossary / open-questions）にあり、未決事項はゼロ。`docs/tax-calculation.md` も確定済み。

Phase 4 着手時に1件（C-16：経費を採算に計上するときの税抜換算）が新たに判明し、
確認のうえ `docs/domain/open-questions.md` に決定として記録した。**`expenses.amount`
は税込の実支出、案件採算は税抜**なので、行ごとに仕入側の税区分で税抜へ換算する
（1円未満切捨て、行ごとに1回）。

**案件売上は Phase 5 の `invoice_lines` が入るまで構造的に 0 になる**
（F-P4：未請求の作業・経費は売上に立たない）。`ProfitabilityService::revenue_for`
がその唯一の差し替え地点。

**リソース識別子は Rust の識別子として妥当な綴りにする**（`work_logs` / `cost_rates`）。`@banto/admin-core` の DataProvider が Tauri コマンドを `${resource}_list` の規約で呼ぶため、ハイフンを含む名前はコマンドを定義できない（`docs/banto-feedback.md` に記録）。画面の URL は `/work-logs` のようにケバブケースで構わない。

Phase 4 までで `docs/banto-feedback.md` に11件記録済み。**`items` デモの削除は未実施**（Phase 4 以降の任意のタイミングで行う。もう手本としては使わない）。

**Phase 1 が確定するまで Phase 2 以降のテーブルを先行実装しない。**

各 Phase 完了時に：

1. `docs/banto-feedback.md` を更新（Phase 2 以降。それ以前も気づいた時点で記録する）
2. このテーブルの状態を更新
3. 完了条件を満たしているか `docs/plan.md` 第18章と照合

---

## 8. コミット規約

```
<type>(<scope>): <subject>

type:  feat / fix / refactor / docs / test / chore / deps
scope: customer / project / worklog / trip / expense / invoice / payment
       / profitability / banto-deps / migration
```

Banto 依存タグを更新する場合は `deps(banto-deps):` を使い、**更新理由と動作確認内容を本文に記載する。**
