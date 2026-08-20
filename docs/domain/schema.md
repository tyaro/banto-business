# DB設計

Phase 1 の確定内容（`open-questions.md`）に基づくテーブル定義。**実装は Phase 2 以降。ここでは定義のみ。**

## 全体規約（`CLAUDE.md` 1.1 / 第5章）

| 項目               | 規約                                                                                                                                            |
| ------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------- |
| 金額               | **`INTEGER NOT NULL`（円）**。`REAL` は使用禁止                                                                                                 |
| 時間               | 分単位の `INTEGER`（`minutes`）。小数時間は持たない                                                                                             |
| 率                 | basis point（`rate_bp`。10% = 1000）または分子分母                                                                                              |
| 日付               | `TEXT NOT NULL`（ISO 8601 `YYYY-MM-DD`、JST のローカル業務日付）。時刻は持たない                                                                |
| 真偽値             | `INTEGER NOT NULL`（0 / 1）— SQLite / PostgreSQL 双方で素直に扱えるため                                                                         |
| ID                 | `TEXT`（UUID v4）。Banto の既存リソースに合わせる                                                                                               |
| マイグレーション   | `apps/admin-template/core/migrations-sqlite/NNNN_*.sql` と `migrations-postgres/` に**同名・同連番で対**にする（`verify-architecture` rule 11） |
| スナップショット列 | 外部キー制約を張らない（マスタ変更で過去が壊れるため。`CLAUDE.md` 第5章）                                                                       |

---

## 1. マスタ

### 1.1 `work_categories`（作業分類）

| 列                             | 型                           | 説明                                                                                                    |
| ------------------------------ | ---------------------------- | ------------------------------------------------------------------------------------------------------- |
| `code`                         | `TEXT PRIMARY KEY`           | `DESIGN` / `PLC` / `SCADA` / `PC_APP` / `TEST` / `INTERNAL` / `ONSITE` / `TRAVEL` / `MEETING` / `OTHER` |
| `name`                         | `TEXT NOT NULL`              | 表示名（設計 / PLC開発 / …）                                                                            |
| `excluded_from_effective_rate` | `INTEGER NOT NULL DEFAULT 0` | **実質時間単価（移動除く）の分母から外すか。** `TRAVEL` のみ 1                                          |
| `sort_order`                   | `INTEGER NOT NULL`           | 表示順                                                                                                  |
| `active`                       | `INTEGER NOT NULL DEFAULT 1` | 廃止した分類は 0（過去データは残る）                                                                    |

> 「移動」の判定に**コード文字列の比較を使わない**（`AGENTS.md` 3.2）。`excluded_from_effective_rate` フラグで判定する。分類を増やしても計算ロジックを触らずに済む。

### 1.2 `cost_rates`（内部原価レート）

| 列                   | 型                 | 説明                                                              |
| -------------------- | ------------------ | ----------------------------------------------------------------- |
| `work_category_code` | `TEXT PRIMARY KEY` | `work_categories.code` を参照                                     |
| `hourly_rate`        | `INTEGER NOT NULL` | 時間単価（円）。**新規 WorkLog 入力時の既定値の供給源にのみ使う** |
| `updated_at`         | `TEXT NOT NULL`    |                                                                   |

> このテーブルは**採算計算では参照しない**（`CLAUDE.md` 1.2）。参照するのは `work_logs.applied_rate`。

### 1.3 `expense_categories`（経費分類）

| 列                     | 型                           | 説明                                                                                   |
| ---------------------- | ---------------------------- | -------------------------------------------------------------------------------------- |
| `code`                 | `TEXT PRIMARY KEY`           | `TRANSPORT` / `LODGING` / `SHIPPING` / `MATERIAL` / `SUPPLIES` / `OUTSOURCE` / `OTHER` |
| `name`                 | `TEXT NOT NULL`              |                                                                                        |
| `default_tax_category` | `TEXT NOT NULL`              | 既定の仕入側税区分。全て `STANDARD_10`                                                 |
| `sort_order`           | `INTEGER NOT NULL`           |                                                                                        |
| `active`               | `INTEGER NOT NULL DEFAULT 1` |                                                                                        |

### 1.4 発行者情報（settings）

登録番号・氏名／屋号・住所・振込先口座は **Banto の `settings`（`banto-admin-services`）に key-value で保持**する。専用テーブルを作らない（Banto が提供済みの機能を再実装しない。`CLAUDE.md` 第2章）。

**実値はリポジトリに置かない**（`CLAUDE.md` 第8章）。シード・テストフィクスチャはサンプル値を使う。

---

## 2. 顧客・案件

### 2.1 `customers`

| 列                          | 型                     | 説明                                       |
| --------------------------- | ---------------------- | ------------------------------------------ |
| `id`                        | `TEXT PRIMARY KEY`     |                                            |
| `code`                      | `TEXT NOT NULL UNIQUE` | 顧客コード（手入力）                       |
| `name`                      | `TEXT NOT NULL`        |                                            |
| `contact_person`            | `TEXT`                 | 担当者                                     |
| `address`                   | `TEXT`                 |                                            |
| `phone` / `email`           | `TEXT`                 |                                            |
| `billing_name`              | `TEXT`                 | 請求先名（顧客名と異なる場合）             |
| `closing_day`               | `INTEGER NOT NULL`     | 締日。1〜28 または **99（月末）**          |
| `payment_month_offset`      | `INTEGER NOT NULL`     | 締日から何ヶ月後に支払われるか（1 = 翌月） |
| `payment_day`               | `INTEGER NOT NULL`     | 支払日。1〜28 または **99（月末）**        |
| `note`                      | `TEXT`                 |                                            |
| `created_at` / `updated_at` | `TEXT NOT NULL`        |                                            |

> 「月末締め・翌月末払い」＝ `closing_day=99, payment_month_offset=1, payment_day=99`。**土日祝の調整はしない**（C-8）。

### 2.2 `projects`

| 列                          | 型                                       | 説明                                                                                              |
| --------------------------- | ---------------------------------------- | ------------------------------------------------------------------------------------------------- |
| `id`                        | `TEXT PRIMARY KEY`                       |                                                                                                   |
| `code`                      | `TEXT NOT NULL UNIQUE`                   | 案件番号 `YYYY-NNN`（自動採番・手修正可）                                                         |
| `customer_id`               | `TEXT NOT NULL REFERENCES customers(id)` |                                                                                                   |
| `name`                      | `TEXT NOT NULL`                          |                                                                                                   |
| `status`                    | `TEXT NOT NULL`                          | `PROSPECT` / `ORDERED` / `IN_PROGRESS` / `AWAITING_ACCEPTANCE` / `COMPLETED` / `LOST` / `ON_HOLD` |
| `started_on`                | `TEXT`                                   | 開始日                                                                                            |
| `due_on`                    | `TEXT`                                   | 終了予定日                                                                                        |
| `estimate_amount`           | `INTEGER`                                | 見積額（税抜・円）                                                                                |
| `contract_amount`           | `INTEGER`                                | 契約額（税抜・円）。**粗利計算には使わず、請求進捗の分母に使う**                                  |
| `scope`                     | `TEXT`                                   | 担当範囲                                                                                          |
| `note`                      | `TEXT`                                   |                                                                                                   |
| `created_at` / `updated_at` | `TEXT NOT NULL`                          |                                                                                                   |

---

## 3. 工数・出張・経費

### 3.1 `trips`

| 列                          | 型                                      | 説明         |
| --------------------------- | --------------------------------------- | ------------ |
| `id`                        | `TEXT PRIMARY KEY`                      |              |
| `project_id`                | `TEXT NOT NULL REFERENCES projects(id)` |              |
| `destination`               | `TEXT NOT NULL`                         | 出張先       |
| `start_on` / `end_on`       | `TEXT NOT NULL`                         |              |
| `onsite_days`               | `INTEGER NOT NULL`                      | 現地作業日数 |
| `nights`                    | `INTEGER NOT NULL`                      | 宿泊数       |
| `note`                      | `TEXT`                                  |              |
| `created_at` / `updated_at` | `TEXT NOT NULL`                         |              |

> Trip 削除時は生成物を削除せず、`work_logs.trip_id` / `expenses.trip_id` を **NULL 化**する（C-6）。したがって `trip_id` の外部キーは `ON DELETE SET NULL`。

### 3.2 `work_logs`

| 列                          | 型                                             | 説明                                                                                                             |
| --------------------------- | ---------------------------------------------- | ---------------------------------------------------------------------------------------------------------------- |
| `id`                        | `TEXT PRIMARY KEY`                             |                                                                                                                  |
| `project_id`                | `TEXT NOT NULL REFERENCES projects(id)`        |                                                                                                                  |
| `trip_id`                   | `TEXT REFERENCES trips(id) ON DELETE SET NULL` | 出張から生成された場合                                                                                           |
| `worked_on`                 | `TEXT NOT NULL`                                | 作業日                                                                                                           |
| `work_category_code`        | `TEXT NOT NULL`                                | 作業分類                                                                                                         |
| `minutes`                   | `INTEGER NOT NULL`                             | 作業時間（分）                                                                                                   |
| **`applied_rate`**          | **`INTEGER NOT NULL`**                         | **記録時点の時間単価（円）を焼き付けた値。`cost_rates` を参照しない（`CLAUDE.md` 1.2）。外部キー制約を張らない** |
| **`internal_cost`**         | **`INTEGER NOT NULL`**                         | **`floor(minutes × applied_rate ÷ 60)` を保存する**                                                              |
| `description`               | `TEXT`                                         | 作業内容                                                                                                         |
| `invoiced`                  | `INTEGER NOT NULL DEFAULT 0`                   | 請求書に載せたか                                                                                                 |
| `created_at` / `updated_at` | `TEXT NOT NULL`                                |                                                                                                                  |

> `internal_cost` を保存するのは、**丸め済みの行原価を合計する**という決定（C-1）を DB レベルで保証するため。都度計算にすると集計側で「合計してから丸める」実装が紛れ込みうる。

### 3.3 `expenses`

| 列                          | 型                                             | 説明                                                                      |
| --------------------------- | ---------------------------------------------- | ------------------------------------------------------------------------- |
| `id`                        | `TEXT PRIMARY KEY`                             |                                                                           |
| `project_id`                | `TEXT NOT NULL REFERENCES projects(id)`        |                                                                           |
| `trip_id`                   | `TEXT REFERENCES trips(id) ON DELETE SET NULL` |                                                                           |
| `spent_on`                  | `TEXT NOT NULL`                                | 支出日                                                                    |
| `expense_category_code`     | `TEXT NOT NULL`                                |                                                                           |
| `payee`                     | `TEXT`                                         | 支払先                                                                    |
| `amount`                    | `INTEGER NOT NULL`                             | 金額（税込の支払額。**仕入側の実支出**）                                  |
| `tax_category`              | `TEXT NOT NULL`                                | 仕入側の税区分（`STANDARD_10` / `REDUCED_8` / `EXEMPT` / `OUT_OF_SCOPE`） |
| `description`               | `TEXT`                                         |                                                                           |
| `billable`                  | `INTEGER NOT NULL DEFAULT 0`                   | 顧客請求対象か                                                            |
| `invoiced`                  | `INTEGER NOT NULL DEFAULT 0`                   | 請求書に載せたか。**`billable` とは別フラグ**                             |
| `created_at` / `updated_at` | `TEXT NOT NULL`                                |                                                                           |

> 領収書は Banto の Attachments に紐づける（`CLAUDE.md` 1.6。正本は会計ソフト側）。

---

## 4. 請求

### 4.1 `invoices`

| 列                           | 型                                       | 説明                                                                                                         |
| ---------------------------- | ---------------------------------------- | ------------------------------------------------------------------------------------------------------------ |
| `id`                         | `TEXT PRIMARY KEY`                       |                                                                                                              |
| `invoice_number`             | `TEXT UNIQUE`                            | `INV-YYYY-NNNN`。**Draft では NULL、確定時に採番**（欠番を作らないため）                                     |
| `customer_id`                | `TEXT NOT NULL REFERENCES customers(id)` | **`project_id` は持たない**（`CLAUDE.md` 1.3）                                                               |
| `status`                     | `TEXT NOT NULL`                          | `DRAFT` / `ISSUED` / `CANCELLED` のみ。**`PARTIALLY_PAID` / `PAID` / `OVERDUE` は保持しない**（導出値）      |
| `issued_on`                  | `TEXT`                                   | 発行日（取引年月日）。確定時に確定                                                                           |
| `closing_on`                 | `TEXT`                                   | 締日                                                                                                         |
| `due_on`                     | `TEXT`                                   | **支払期限。確定時に顧客マスタから算出して保存**（マスタ変更が過去に波及しないこと）                         |
| `corrected_invoice_id`       | `TEXT`                                   | 赤伝で差し替えた元請求書（C-10）。**外部キー制約は張らない**（取消済み請求書の削除可否に引きずられないため） |
| `total_taxable`              | `INTEGER NOT NULL DEFAULT 0`             | 税抜合計（確定時スナップショット）                                                                           |
| `total_tax`                  | `INTEGER NOT NULL DEFAULT 0`             | 消費税合計（同上）                                                                                           |
| `total_amount`               | `INTEGER NOT NULL DEFAULT 0`             | 税込合計（同上）                                                                                             |
| `rounding_mode`              | `TEXT NOT NULL`                          | `FLOOR` / `ROUND` / `CEIL`。**確定時の設定値をスナップショット**（`CLAUDE.md` 1.7）                          |
| `issuer_name`                | `TEXT`                                   | 確定時の発行者名（settings のスナップショット）                                                              |
| `issuer_registration_number` | `TEXT`                                   | 確定時の登録番号（同上）                                                                                     |
| `issuer_address`             | `TEXT`                                   |                                                                                                              |
| `note`                       | `TEXT`                                   |                                                                                                              |
| `created_at` / `updated_at`  | `TEXT NOT NULL`                          |                                                                                                              |

### 4.2 `invoice_lines`

| 列             | 型                                                        | 説明                                                                                                  |
| -------------- | --------------------------------------------------------- | ----------------------------------------------------------------------------------------------------- |
| `id`           | `TEXT PRIMARY KEY`                                        |                                                                                                       |
| `invoice_id`   | `TEXT NOT NULL REFERENCES invoices(id) ON DELETE CASCADE` |                                                                                                       |
| `project_id`   | `TEXT NOT NULL`                                           | **どの案件の売上か**（`CLAUDE.md` 1.3）。確定後もマスタ変更に引きずられないよう外部キー制約は張らない |
| `line_no`      | `INTEGER NOT NULL`                                        | 表示順                                                                                                |
| `item_name`    | `TEXT NOT NULL`                                           | 品目                                                                                                  |
| `quantity`     | `INTEGER NOT NULL DEFAULT 1`                              | 数量                                                                                                  |
| `unit_price`   | `INTEGER NOT NULL`                                        | **単価（整数円のみ）**（B-2）                                                                         |
| `amount`       | `INTEGER NOT NULL`                                        | 行金額（税抜）。**マイナス可**（値引き行、B-3）                                                       |
| `tax_category` | `TEXT NOT NULL`                                           | 請求側の税区分。立替経費の再請求も既定は `STANDARD_10`（B-5）                                         |
| `source_type`  | `TEXT`                                                    | `WORK_LOG` / `EXPENSE` / `MANUAL`（どこから起こした明細か）                                           |
| `note`         | `TEXT`                                                    |                                                                                                       |

> 工数から起こす明細は「時間単価 × 時間」を**行金額として整数で確定**させてから載せる（B-2）。

### 4.3 `invoice_tax_summaries`（税率区分ごとの集計）

確定時のスナップショット。**適格請求書の「税率ごとに区分して合計した対価の額・消費税額・適用税率」の記載根拠**。

| 列               | 型                                                        | 説明                                                    |
| ---------------- | --------------------------------------------------------- | ------------------------------------------------------- |
| `id`             | `TEXT PRIMARY KEY`                                        |                                                         |
| `invoice_id`     | `TEXT NOT NULL REFERENCES invoices(id) ON DELETE CASCADE` |                                                         |
| `tax_category`   | `TEXT NOT NULL`                                           |                                                         |
| `rate_bp`        | `INTEGER NOT NULL`                                        | basis point（10% = 1000、8% = 800、非課税・不課税 = 0） |
| `taxable_amount` | `INTEGER NOT NULL`                                        | 税率区分ごとの対価合計（税抜）                          |
| `tax_amount`     | `INTEGER NOT NULL`                                        | **端数処理後**の消費税額。区分ごとに1回だけ処理した結果 |

> **対価合計が 0 の区分は行を作らない**（T-10）。

---

## 5. 入金

### 5.1 `payments`

| 列                          | 型                                       | 説明                                           |
| --------------------------- | ---------------------------------------- | ---------------------------------------------- |
| `id`                        | `TEXT PRIMARY KEY`                       |                                                |
| `customer_id`               | `TEXT NOT NULL REFERENCES customers(id)` | **`invoice_id` は持たない**（`CLAUDE.md` 1.4） |
| `paid_on`                   | `TEXT NOT NULL`                          | 入金日                                         |
| `amount`                    | `INTEGER NOT NULL`                       | 入金額（円）                                   |
| `method`                    | `TEXT`                                   | 振込 / 現金 等                                 |
| `note`                      | `TEXT`                                   |                                                |
| `created_at` / `updated_at` | `TEXT NOT NULL`                          |                                                |

### 5.2 `payment_allocations`（消込）

| 列                  | 型                                                        | 説明                                                                  |
| ------------------- | --------------------------------------------------------- | --------------------------------------------------------------------- |
| `id`                | `TEXT PRIMARY KEY`                                        |                                                                       |
| `payment_id`        | `TEXT NOT NULL REFERENCES payments(id) ON DELETE CASCADE` |                                                                       |
| `invoice_id`        | `TEXT NOT NULL REFERENCES invoices(id)`                   |                                                                       |
| `allocated_amount`  | `INTEGER NOT NULL`                                        | 充当額（円）                                                          |
| `difference_reason` | `TEXT`                                                    | `TRANSFER_FEE` / `WITHHOLDING` / `DISCOUNT` / `OVERPAYMENT` / `OTHER` |
| `difference_amount` | `INTEGER NOT NULL DEFAULT 0`                              | 差額（手数料・源泉等）                                                |
| `note`              | `TEXT`                                                    | `OTHER` の場合は必須                                                  |
| `created_at`        | `TEXT NOT NULL`                                           |                                                                       |

---

## 6. 保持しないもの（意図的な不在）

| 保持しない                                  | 代わりに                                                   | 根拠                 |
| ------------------------------------------- | ---------------------------------------------------------- | -------------------- |
| `invoices.status = OVERDUE`                 | `due_on < 今日 AND 残額 > 0 AND status ≠ CANCELLED` の導出 | `CLAUDE.md` 1.5      |
| `invoices.status = PARTIALLY_PAID` / `PAID` | 充当額合計からの導出                                       | `state-machine.md` 1 |
| `invoices.project_id`                       | `invoice_lines.project_id`                                 | `CLAUDE.md` 1.3      |
| `payments.invoice_id`                       | `payment_allocations`                                      | `CLAUDE.md` 1.4      |
| 案件採算の集計テーブル                      | 都度集計（WorkLog / Expense / InvoiceLine から導出）       | 二重管理を避ける     |
| 通貨コード                                  | 円のみ                                                     | C-13                 |
| 残業・休日単価                              | 作業分類別レートのみ                                       | C-14                 |

---

## 7. マイグレーション計画（Phase 2 以降）

| 連番                                                                               | 内容             | Phase |
| ---------------------------------------------------------------------------------- | ---------------- | ----- |
| `0007_work_categories.sql` / `0008_expense_categories.sql` / `0009_cost_rates.sql` | マスタ3種        | 2     |
| `0010_customers.sql`                                                               | 顧客             | 2     |
| `0011_projects.sql`                                                                | 案件             | 2     |
| `0012_trips.sql` / `0013_work_logs.sql` / `0014_expenses.sql`                      | 工数・出張・経費 | 3     |
| `0015_invoices.sql` / `0016_invoice_lines.sql` / `0017_invoice_tax_summaries.sql`  | 請求             | 5     |
| `0018_payments.sql` / `0019_payment_allocations.sql`                               | 入金             | 6     |

- 連番は既存の Banto テンプレート分（`0001`〜`0006`）に続ける
- **SQLite と PostgreSQL の2方言を対で追加する**（`verify-architecture` rule 11 が機械検査）
