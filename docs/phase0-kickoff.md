# Phase 0 着手用プロンプト

Claude Code に貼り付けて使う。**このファイル自体はリポジトリに残さなくてよい。**

---

## 事前準備（人間側）

Claude Code を起動する前に：

1. `tyaro/banto-business` リポジトリを作成
2. Banto テンプレートから派生させ、ローカルにクローン
3. 以下のファイルをリポジトリ直下に配置

```
CLAUDE.md
AGENTS.md
docs/plan.md
docs/tax-calculation.md
docs/banto-feedback.md
docs/template-origin.md
```

4. `banto` リポジトリをローカルの隣接ディレクトリにクローン（差分確認用）
5. リポジトリのルートで `claude` を起動

---

## Phase 0 プロンプト

```
Banto Business の Phase 0 を実施します。

まず CLAUDE.md、AGENTS.md、docs/plan.md を読んでください。
特に docs/plan.md の第18章 Phase 0 と第4章（バージョン管理）が対象範囲です。

作業前に、現在のリポジトリ状態を確認して以下を報告してください。

1. テンプレート由来のファイル構成
2. Banto への依存の指定方法（現在タグ固定になっているか、main参照か）
3. 派生元と思われる Banto のバージョン
4. アプリ名・package名・crate名で変更が必要な箇所の一覧
5. 削除対象と思われるデモリソース

報告を受けてから作業指示を出します。**確認前に変更を加えないでください。**
```

### 報告確認後の実施指示

```
確認しました。Phase 0 の作業を実施してください。

1. アプリ名を Banto Business に変更
2. package名 / crate名を変更
3. Tauri 設定を更新
4. デモリソースを削除
5. Banto 依存を git タグ参照に変更（タグ: vX.Y.Z）
6. docs/template-origin.md の「現在の派生状態」と
   「テンプレート由来ファイル」を実態に合わせて記入
7. AGENTS.md 第2章の Banto バージョンと採用パッケージを記入

完了条件は「空の Banto Business アプリが起動すること」です。
ビルドと起動を確認して報告してください。

CLAUDE.md 第6章の禁止事項に該当する作業が必要になった場合は、
実施せずに確認を取ってください。
```

---

## Phase 1 プロンプト

Phase 0 完了後に使う。

```
Phase 1（要件・ドメイン設計）を開始します。

docs/plan.md 第7〜14章と docs/tax-calculation.md を読んでください。

Phase 1 の目的は実装ではなく設計の確定です。コードは書かないでください。

まず docs/tax-calculation.md の「7. 未決事項」と、各所の TODO を
一覧にして提示してください。私が判断すべき事項を整理したいです。

そのうえで、設計上あなたが判断に迷う点、
計画書に書かれていない曖昧な点を挙げてください。

特に以下は推測で埋めないこと（CLAUDE.md 第7章）:
- 税計算
- 採算計算
- 消込ロジック
```

### Phase 1 成果物の作成指示

未決事項を確定させた後：

```
未決事項が確定しました。docs/domain/ に以下を作成してください。

- requirements.md   要件定義
- er-diagram.md     ER図（Mermaid）
- schema.md         DB設計（テーブル定義・型・制約）
- state-machine.md  状態遷移（Invoice / Payment / Project）
- glossary.md       用語集

制約は CLAUDE.md 第1章の絶対規約に従うこと。特に:
- 金額は INTEGER（円）
- 原価レートは WorkLog 行にスナップショット
- Invoice は Customer + InvoiceLine の 1:N
- Payment は PaymentAllocation 経由の N:M
- Overdue はカラムにしない

作成後、docs/tax-calculation.md の TODO を確定内容で更新してください。
```

---

## 各 Phase 共通の締め指示

Phase 完了時に毎回使う。

```
Phase N を完了します。以下を実施してください。

1. docs/plan.md 第18章の Phase N 完了条件と照合し、
   満たしているか項目ごとに報告
2. docs/banto-feedback.md に、この Phase で
   Banto の標準手順から外れた点・回避策を記録
   （なければ「なし」と明記）
3. AGENTS.md 第7章の Phase 進行テーブルを更新
```

---

## 運用上の注意

- **Phase を飛ばさない。** Phase 1 が確定するまで Phase 2 以降のテーブルを作らせない
- **金額ロジックはテストとセットで受け取る。** テストなしの実装は差し戻す
- **`docs/banto-feedback.md` の記録を後回しにしない。** Phase 完了時ではなく、気づいた時点で書かせる
- 長い作業では CLAUDE.md の規約が薄れることがある。金額・請求・入金に関わる実装の前に
  「CLAUDE.md 第9章のチェックリストを確認してください」と挟むと効く
