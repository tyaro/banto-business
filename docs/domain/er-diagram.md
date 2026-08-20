# ER図

`docs/plan.md` 第7章の関連を、Phase 1 の決定（`open-questions.md`）を反映して確定させたもの。

---

## 全体

```mermaid
erDiagram
    CUSTOMER ||--o{ PROJECT : "受注する"
    CUSTOMER ||--o{ INVOICE : "請求先"
    CUSTOMER ||--o{ PAYMENT : "入金元"

    PROJECT ||--o{ WORK_LOG : "工数"
    PROJECT ||--o{ TRIP : "出張"
    PROJECT ||--o{ EXPENSE : "経費"
    PROJECT ||--o{ INVOICE_LINE : "売上の計上先"

    TRIP ||--o{ WORK_LOG : "一括生成（trip_id）"
    TRIP ||--o{ EXPENSE : "一括生成（trip_id）"

    INVOICE ||--|{ INVOICE_LINE : "明細"
    INVOICE ||--o{ INVOICE_TAX_SUMMARY : "税率区分ごとの集計（確定時スナップショット）"
    INVOICE ||--o{ PAYMENT_ALLOCATION : "充当先"
    PAYMENT ||--o{ PAYMENT_ALLOCATION : "充当元"
    INVOICE ||--o| INVOICE : "corrected_invoice_id（赤伝の対応）"

    COST_RATE }o--|| WORK_CATEGORY : "作業分類ごとの単価"
    WORK_LOG }o--|| WORK_CATEGORY : "分類"
    EXPENSE }o--|| EXPENSE_CATEGORY : "分類"

    SETTINGS ||--o{ INVOICE : "発行者情報をスナップショット"
```

### この図が表している設計判断

| 判断                               | 図での現れ方                                                               | 根拠                  |
| ---------------------------------- | -------------------------------------------------------------------------- | --------------------- |
| Invoice は Project と 1:1 にしない | `INVOICE` は `CUSTOMER` に属し、`PROJECT` へは `INVOICE_LINE` 経由で繋がる | `CLAUDE.md` 1.3       |
| Payment は Invoice と 1:1 にしない | `PAYMENT_ALLOCATION` を挟んだ N:M                                          | `CLAUDE.md` 1.4       |
| 原価レートは参照しない             | `WORK_LOG` は `COST_RATE` を参照しない（`applied_rate` を行に持つ）        | `CLAUDE.md` 1.2       |
| Trip は生成のまとまり              | `TRIP` から `WORK_LOG` / `EXPENSE` へ 1:N。削除時は `trip_id` を NULL 化   | C-6                   |
| 税集計は確定時スナップショット     | `INVOICE_TAX_SUMMARY` を独立テーブルで保持                                 | `CLAUDE.md` 1.2 / 1.7 |

---

## 採算計算の依存関係

案件採算がどのデータから導出されるかを示す。**Overdue と同じく、採算値はテーブルに保持しない導出値**。

```mermaid
flowchart TD
    WL["WorkLog<br/>minutes / applied_rate"] -->|"floor(分 × 単価 ÷ 60) を行ごとに確定し合計"| COST["工数原価"]
    EX["Expense<br/>amount"] --> DIRECT["直接経費"]
    IL["InvoiceLine（確定済）<br/>amount（税抜）"] --> SALES["案件売上"]

    SALES --> PROFIT["案件粗利"]
    COST --> PROFIT
    DIRECT --> PROFIT

    PROFIT --> RATE1["実質時間単価（移動込み）<br/>÷ 全 WorkLog 時間"]
    PROFIT --> RATE2["実質時間単価（移動除く）<br/>÷ 移動を除く WorkLog 時間"]
    PROFIT --> MARGIN["粗利率<br/>÷ 案件売上"]

    PJ["Project.contract_amount"] --> PROG["請求進捗<br/>案件売上 ÷ 契約額"]
    SALES --> PROG
```

- 実質時間単価は**2種を必ず併記**する（`AGENTS.md` 3.5。片方のみを返す API を作らない）
- 顧客請求対象の経費（`billable = true`）は**原価にも売上にも計上する**（C-4 両建て）

---

## 入金消込の関係

```mermaid
flowchart LR
    INV1["Invoice A<br/>請求額 100,000"]
    INV2["Invoice B<br/>請求額 50,000"]
    PAY["Payment<br/>入金 149,340"]

    PAY -->|"充当 99,780<br/>TRANSFER_FEE 220"| INV1
    PAY -->|"充当 49,560<br/>TRANSFER_FEE 440"| INV2

    INV1 --> R1["残額 0 → Paid"]
    INV2 --> R2["残額 0 → Paid"]
```

- 1件の入金が複数請求書に分かれる（まとめ入金）／1つの請求書に複数の入金が付く（分割入金）の両方を `PAYMENT_ALLOCATION` で表現する
- 振込手数料の先方差引・源泉徴収は**差額理由コード付きの充当**として記録し、請求額そのものは変更しない
