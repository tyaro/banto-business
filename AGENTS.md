# AGENTS.md

Banto Business の開発手順書。AIエージェント・開発者共通。

**常時適用の規約は `CLAUDE.md` を参照すること。このファイルは手順と構造を扱う。**

---

## 1. リポジトリ構成

```
banto-business/
├── CLAUDE.md              # 常時適用規約（最優先）
├── AGENTS.md              # このファイル
├── docs/
│   ├── plan.md            # 開発計画書
│   ├── tax-calculation.md # 税計算仕様（Phase 1で確定）
│   ├── banto-feedback.md  # Bantoへのフィードバックログ（Phase 2から記録）
│   ├── template-origin.md # テンプレート派生元とマージ判断の記録
│   ├── domain/            # Phase 1 成果物（ER図・状態遷移・用語集）
│   └── adr/               # 設計判断記録
├── src/                   # SvelteKit フロントエンド
│   ├── lib/
│   │   ├── domain/        # ドメイン型定義
│   │   ├── features/      # 機能単位（customer / project / worklog / ...）
│   │   └── components/    # Business固有UI（汎用UIはBantoから）
│   └── routes/
├── src-tauri/             # Rust バックエンド
│   ├── src/
│   │   ├── domain/        # ドメインロジック（採算・税計算・消込）
│   │   ├── repository/    # 永続化
│   │   └── commands/      # Tauri コマンド
│   └── migrations/
└── tests/
```

### ディレクトリ責務

| ディレクトリ | 置くもの | 置かないもの |
|---|---|---|
| `src-tauri/src/domain/` | 採算計算・税計算・消込ロジック（純粋関数中心） | DB / Tauri 依存 |
| `src-tauri/src/repository/` | SQLクエリ・永続化 | 業務ロジック |
| `src-tauri/src/commands/` | Tauriコマンド定義（薄く保つ） | 業務ロジック |
| `src/lib/features/` | 画面単位のロジック | 汎用UIコンポーネント |
| `src/lib/components/` | Business固有UI | Bantoが提供済みのUI |

**金額計算は必ず `src-tauri/src/domain/` に置く。フロントエンドで金額計算をしない。**
（表示フォーマットのみフロント側で行う）

---

## 2. 使用 Banto バージョン

| 項目 | 値 |
|---|---|
| Banto タグ | `vX.Y.Z` ← Phase 0 で記入 |
| 派生元テンプレートタグ | `vX.Y.Z` ← Phase 0 で記入 |
| 最終確認日 | YYYY-MM-DD |

更新手順は `docs/template-origin.md` を参照。

### 利用中の Banto パッケージ

Phase 0 以降、実際に採用したものだけを記載する。

**フロントエンド**

- [ ] `@banto/admin-core`
- [ ] `@banto/grid-svelte`
- [ ] `@banto/forms`
- [ ] `@banto/charts`
- [ ] `@banto/dock-svelte`
- [ ] `@banto/report`
- [ ] `@banto/attachments`

**Rust**

- [ ] `banto-core`
- [ ] `banto-storage`
- [ ] `banto-server`
- [ ] `banto-admin-services`
- [ ] `banto-attachments`

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

| 指標 | 分母 |
|---|---|
| 移動込み | 全WorkLog時間 |
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

- ファイル名：`migrations/NNNN_snake_case_description.sql`
- **適用済みマイグレーションは編集しない。** 変更は新規ファイルで
- 前方向のみ。down migration は用意しない（Backup/Restore で対応）
- 金額カラム：`INTEGER NOT NULL`
- 日付カラム：`TEXT`（ISO 8601 / JST の業務日付）
- スナップショット系カラムに外部キー制約を張らない
- マイグレーション追加時は必ず Backup/Restore の動作を確認する

---

## 5. テスト要件

### 必須

| 対象 | 理由 |
|---|---|
| 案件採算計算 | 意思決定に直結 |
| 税計算（税率区分別集計・端数処理） | 対外文書。誤りが取引先に届く |
| Payment 消込 | 4種の差額パターン全て |
| Overdue 導出の境界条件 | 期限当日 / 残額0 / Cancelled |
| Trip 一括生成 | 生成件数と内訳 |

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

現在の Phase：**Phase 0**

| Phase | 内容 | 状態 |
|---|---|---|
| 0 | リポジトリ作成・テンプレート派生 | 未着手 |
| 1 | 要件・ドメイン設計 | 未着手 |
| 2 | 基本マスター（Customer / Project） | 未着手 |
| 3 | 工数・経費（WorkLog / Trip / Expense） | 未着手 |
| 4 | 採算管理 | 未着手 |
| 5 | 請求（Invoice / PDF） | 未着手 |
| 6 | 入金管理（Payment） | 未着手 |
| 7 | 実運用評価 | 未着手 |

**Phase 1 が確定するまで Phase 2 以降のテーブルを先行実装しない。**

各 Phase 完了時に：

1. `docs/banto-feedback.md` を更新（Phase 2 以降）
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
