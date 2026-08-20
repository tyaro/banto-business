# Banto フィードバックログ

Banto Business の開発中に見つかった Banto 本体への課題を記録する。

**記録開始：Phase 2**（ただし Phase 0/1 でも Banto 由来の摩擦に気づいた時点で記録する。「後でまとめない」原則を優先）

---

## なぜ逐次記録するか

計画書 第22章の第二の成功条件は「Banto が汎用的な業務アプリケーション基盤として成立していることの実証」である。この検証根拠がこのファイルになる。

最も価値のある摩擦は開発初期に発生するが、開発が進むと慣れによって「そういうもの」として受け入れられ、後から思い出せなくなる。**開発完了後にまとめて振り返らない。気づいた時点で記録する。**

---

## 記録ルール

1. **その場で書く。** 後でまとめようとしない
2. 回避策を実装したら必ず記録する（回避策の存在自体が Banto の課題の証拠）
3. 分類が判断できない場合は `未分類` のままでよい。分類より記録が優先
4. Banto 側へ還元するのは `Banto共通` のみ。`Business固有` は還元しない

---

## 分類基準

| 分類 | 判断基準 |
|---|---|
| `Banto共通` | Industrial でも他ドメインでも同様に発生しうる。フレームワークの汎用性の問題 |
| `Business固有` | 事業管理ドメイン特有。Banto に持ち込むと汎用性を損なう |
| `未分類` | 判断保留 |

**迷ったら `未分類` で記録し、Phase 完了時にまとめて判断する。**

---

## ログ

<!--
新しい記録を下に追記する。フォーマット：

### [YYYY-MM-DD] Phase N — 一行要約

| 項目 | 内容 |
|---|---|
| 事象 | Bantoに足りなかった / 想定と違った点 |
| 影響 | 開発への影響（時間・実装の歪み） |
| 回避策 | 実際にどう対処したか |
| 分類 | Banto共通 / Business固有 / 未分類 |
| Banto Issue | #NNN または 未起票 |
| 状態 | 未対応 / 起票済 / Banto側対応済 / 対応不要 |

-->

### [2026-08-19] Phase 0 — `rename.mjs` が CI ワークフロー内の `--filter <旧アプリ名>` を書き換えない

| 項目 | 内容 |
|---|---|
| 事象 | `scripts/rename.mjs` はルート `package.json` と `e2e/playwright.config.ts` の `--filter admin-template` は追随させるが、`.github/workflows/ci.yml` の i18n ジョブにある `pnpm --filter admin-template paraglide:compile` を書き換えない。リネーム直後の派生リポジトリは CI がその1ジョブで落ちる |
| 影響 | Phase 0 の最初の CI が失敗する。原因がリネーム漏れだと気づくまで時間を取られる（スクリプトは「✔ 書き換えました」と成功報告するため、漏れが見えない） |
| 回避策 | `.github/workflows/ci.yml:110` を手動で `--filter banto-business-app` に修正。`docs/template-origin.md` の改変ファイル表に記録 |
| 分類 | Banto共通（Industrial でも同じ導線を通れば同様に発生する） |
| Banto Issue | 未起票 |
| 状態 | 未対応 |

> 補足：機能に影響しない箇所（`apps/admin-template/**` の Rust コメント、`e2e/visual/README.md`、`scripts/scaffold.mjs` の案内文言）にも旧アプリ名の例が残るが、こちらは上流差分を増やさないため意図的に未修正。

---

### [2026-08-19] Phase 0 — 派生元 main HEAD にタグが無く「タグ固定」が成立しない

| 項目 | 内容 |
|---|---|
| 事象 | Banto のタグ運用規約は「破壊的変更時のみタグ更新」であり、派生時点の main HEAD（`f471ff1`）にタグが無い。直近タグ `v1.2.0` は 35 コミット前で、派生アプリに効く修正（issue #150 / ADR-0007 等）を含まない |
| 影響 | Business 側の当初規約「Banto 依存は git タグで固定」（CLAUDE.md 第3章）がそのままでは満たせない |
| 回避策 | 依存方式を「同梱＋派生元コミット固定」と決定し、CLAUDE.md 第3章と `docs/template-origin.md` を改訂 |
| 分類 | Banto共通（消費側が最新の修正を固定参照したいときに毎回起きる） |
| Banto Issue | 未起票 |
| 状態 | 未対応（Banto 側で定期的にタグを打つ運用にすれば解消する） |

---

### [2026-08-20] Phase 2 — `FieldDef` に説明文（hint）の欄が無い

| 項目 | 内容 |
|---|---|
| 事象 | `@banto/forms` の `FieldDef`（`packages/forms/src/types.ts`）に、入力欄の下に出す説明文の欄が無い。案件番号の「空欄で保存すると自動採番されます」のような**入力規則の説明**を出す場所が `placeholder` しかない |
| 影響 | 説明が入力例と同じ位置に出るため、値を1文字でも入れると説明が消える。自動採番のように「空欄のまま保存してよい」ことを伝えたい欄と相性が悪い |
| 回避策 | `placeholder` で代用した。`FieldDef` に `hint` を足すのはパッケージ契約をアプリ都合で広げることになる（conventions §4/§5）ため見送り |
| 分類 | Banto共通（入力規則の説明が要るフォームはドメインを問わず出る） |
| Banto Issue | 未起票 |
| 状態 | 未対応 |

---

### [2026-08-20] Phase 2 — `items` が「手本」として重く、最小構成のリソース例が無い

| 項目 | 内容 |
|---|---|
| 事象 | `docs/recipes/add-resource.md` はリソース追加の正式手順を「`items` のルート一式をコピーして書き換える」と定めているが、`items/+page.svelte` は 885 行あり、CSV インポート/エクスポート・添付ファイル・クライアント/サーバーのモード切り替え・グループ化が1ファイルに同居している。**単純な CRUD 画面に必要な部分だけを取り出す作業**が発生する |
| 影響 | コピー元をそのまま使うと不要機能が付いてくるため、結局どの行が CRUD の骨格でどれがデモ機能かを読み分ける必要があった（顧客・案件の一覧はサーバーモードのみで足りるため、モード切り替えとクライアントグリッドを落とした） |
| 回避策 | `items` の +page.svelte からは列定義・グリッド配線・RBAC の扱いだけを写し、`new` / `[id]` は items 版をほぼそのまま使った。読み込みと後片付けを別 effect に分ける必要がある点は `ItemsServerGrid.svelte` のコメントが明示していて助かった |
| 分類 | Banto共通（テンプレート利用者は必ずこの読み分けを通る） |
| Banto Issue | 未起票 |
| 状態 | 未対応（提案：`items` を全部入りのまま残しつつ、最小 CRUD の骨格だけを別リソース or レシピ内の抜粋として示す） |

---

### [2026-08-20] Phase 2 — サービスを1つ足すとテストヘルパ6箇所の `Services` 初期化を全て直す必要がある

| 項目 | 内容 |
|---|---|
| 事象 | `rest::Services` は M-review 2026-08 M-13 で位置引数から構造体へ集約済みだが、`rest/tests.rs` の各ルータヘルパが `Services { .. }` を**フィールド列挙で**構築しているため、サービスを1つ足すと8箇所がコンパイルエラーになる |
| 影響 | Phase 2 で 2 サービス追加した際、テストヘルパ8箇所 + `banto-serve.rs` + `src-tauri` の `AppState` 3箇所を機械的に修正した。エラーは全てコンパイル時に出るので**見落としは起きない**が、単純作業が発生する |
| 回避策 | そのまま修正した（`..Default::default()` を使うには `Services` に `Default` が要るが、各サービスは DB ハンドルを持つため既定値を作れない） |
| 分類 | Banto共通 |
| Banto Issue | 未起票 |
| 状態 | 対応不要の可能性あり（コンパイルエラーで必ず捕捉されるため実害は小さい。記録のみ） |

---

### [2026-08-20] Phase 2 — 両経路対称の機械検査（rule 8）は有効に働いた（肯定的な記録）

| 項目 | 内容 |
|---|---|
| 事象 | 新しい mutating 操作を REST / Tauri の両方に追加する際、`scripts/verify-architecture.mjs` の `DUAL_PATH` マニフェストに登録しないと CI が落ちる仕組みが、**片側だけ実装するミスを実際に防ぐ設計**として機能した。`TAURI_READ` / `REST_READ` の分類も、読み取り系を対称強制の対象外にする根拠が明示されていて迷わなかった |
| 影響 | conventions §1 の規約を「読んで守る」ではなく「守らないと落ちる」形にできている |
| 回避策 | — |
| 分類 | Banto共通（他ドメインでも同じ恩恵） |
| Banto Issue | — |
| 状態 | 対応不要 |

---

### [2026-08-20] Phase 2 — `src-tauri` は clippy の対象外で、upstream のコードが現行 Rust の lint に引っかかる

| 項目 | 内容 |
|---|---|
| 事象 | `cargo clippy -p admin-template --all-targets -- -D warnings` を実行すると、テンプレート由来の `attachments_upload_body` の doc コメント3行が `clippy::doc_lazy_continuation`（Rust 1.94）で落ちる。CI の Tauri check は `cargo check + test` のみで clippy を回さないため、この状態が検出されない |
| 影響 | 派生アプリ側で「clippy を全クレートに広げる」と、自分が書いていないコードのエラーから直面する。Business では CLAUDE.md 第3章（同梱コードを Business 都合で書き換えない）に従い修正していない |
| 回避策 | 修正せず記録のみ。Business のコードは `-D warnings` で通ることを個別に確認した |
| 分類 | Banto共通 |
| Banto Issue | 未起票 |
| 状態 | 未対応 |

---

### [2026-08-20] Phase 3 — DataProvider の命名規約がリソース名を Rust 識別子に縛る

| 項目 | 内容 |
|---|---|
| 事象 | `@banto/admin-core` の Tauri DataProvider は `${resource}_list` / `_get` / `_create` / `_update` / `_delete` の規約で `invoke()` する。したがって**リソース名にハイフンを使えない**（`work-logs_list` は Rust の識別子として不正でコマンドを定義できない）。この制約は `docs/recipes/add-resource.md` にも `providers/tauri.ts` の doc にも明示が無く、`work-logs` で一通り実装してから発覚した |
| 影響 | リソース名を `work_logs` に変え、REST パスも `/api/work_logs/...` に揃え直した（画面の URL は `/work-logs` のまま）。Banto 本体のリソースは `items` / `users` / `attachments` と全て単語1つなので、この制約に当たるのは複数語のリソースを持つ派生アプリだけ |
| 回避策 | リソース識別子を Rust の識別子と同形（`work_logs` / `cost_rates` / `work_categories`）に統一した |
| 分類 | Banto共通（複数語リソースは業務ドメインでは普通に出る） |
| Banto Issue | 未起票 |
| 状態 | 未対応（提案：レシピか `providers/tauri.ts` の doc に「リソース名は Rust の識別子として妥当な綴りにする」と1行明記する） |

---

### [2026-08-20] Phase 3 — 日付ユーティリティが Banto 内で重複しており、派生アプリは3つ目のコピーを書くことになる

| 項目 | 内容 |
|---|---|
| 事象 | 依存を足さない方針（conventions §3）で Hinnant の civil-date アルゴリズムを自前実装している箇所が、`apps/admin-template/core/src/db.rs` と `crates/banto-admin-services/src/backup.rs` の**2箇所に重複**している（どちらも private）。Business は日付の逆変換（日付→通算日）と日数加算が要ったため、`dates.rs` に3つ目を書いた |
| 影響 | 同じアルゴリズムが同一リポジトリ内に3つ存在する。どれかにバグがあっても他へ伝播しない |
| 回避策 | Business 側で `dates.rs` を新設し、往復変換で存在しない日（2026-02-30 / 平年の 2/29）も弾けるようにした上でテストを付けた |
| 分類 | Banto共通 |
| Banto Issue | 未起票 |
| 状態 | 未対応（提案：`banto-core` に業務日付ユーティリティを置き、db.rs / backup.rs もそれを使う。ADR-0002 の「依存を足さない」は自前実装を1箇所に集めることと矛盾しない） |

---

### [2026-08-20] Phase 4 — `pg_smoke` のスキーマリセットがテンプレート由来のテーブル名を直書きしている

| 項目 | 内容 |
|---|---|
| 事象 | `apps/admin-template/core/tests/pg_smoke.rs` の `reset_schema` が `DROP TABLE IF EXISTS attachments, audit_log, users, settings, items` とテンプレート由来のテーブルだけを列挙している。派生アプリがテーブルを足すと、`_sqlx_migrations` だけ消えて実テーブルが残るため、**2回目以降のローカル実行でマイグレーションが「既に存在する」で失敗する** |
| 影響 | CI は毎回新しい `postgres:16` コンテナなので緑のまま。手元で PostgreSQL を立てて2回流したときだけ落ちるので、原因に見当が付きにくい |
| 回避策 | Business 側のテーブル（`customers` 〜 `expenses`）を `reset_schema` の DROP に追記した。以後リソースを足すたびにここも足す運用になる |
| 分類 | Banto共通（派生アプリは必ずテーブルを足す） |
| Banto Issue | 未起票 |
| 状態 | 未対応（提案：`information_schema` から現在のスキーマのテーブルを列挙して落とす、あるいはテスト用に専用スキーマを作って `DROP SCHEMA ... CASCADE` する。どちらでもテーブル名の直書きが消える） |

---

### [2026-08-20] Phase 4 — テーマにフォントサイズのスケールが無く、アプリ側が存在しない変数を書いても気付けない

| 項目 | 内容 |
|---|---|
| 事象 | `@banto/theme` は色・余白・角丸・影をトークン化しているが、フォントサイズは `--banto-font-size`（基準値）1つだけで `-sm` / `-md` / `-lg` のスケールが無い。テンプレート同梱の画面は `0.85rem` のような実寸を直書きしている。他のトークンの命名から類推して `var(--banto-font-size-sm)` と書いてしまい、**未定義の custom property は宣言ごと無効になるだけでエラーにならない**ため、ビルドも lint も型検査も通ってしまった（実際 Phase 2〜3 の画面で14箇所書いてしまい、Phase 4 で気付いて実寸へ直した） |
| 影響 | 見た目は「効いていないだけ」で崩れないため、レビューでも気付きにくい |
| 回避策 | テンプレート同梱の画面と同じく実寸（`0.85rem` / `1rem` / `1.15rem`）で書き直した |
| 分類 | Banto共通 |
| Banto Issue | 未起票 |
| 状態 | 未対応（提案：他のトークンと同じ粒度でフォントサイズのスケールを定義するか、`verify-architecture.mjs` に「`--banto-*` 変数の参照が theme の定義に存在するか」の検査を足す。後者は色や余白のタイポも同時に捕まえられる） |

---

### [2026-08-20] Phase 5 — DataProvider に載らない操作のクライアントが毎回同じ配線をコピーしている

| 項目 | 内容 |
|---|---|
| 事象 | `${resource}_list/_get/_create/_update/_delete` の CRUD 契約に収まらない操作（確定・取消のような id + 動詞、単一の設定、検索系）は、テンプレート同梱の `usersAdmin.ts` / `backupsAdmin.ts` / `serverAdmin.ts` / `attachmentsAdmin.ts` がそれぞれ**同じ 80 行程度の配線**（`ErrorBody` 判定 → `ProviderError` 正規化、`invoke` ラッパ、CSRF ヘッダ + Bearer + `fetch` ラッパ、デモモード判定）を持っている。Business も請求の確定・取消・候補・発行者設定のために5つ目のコピー（`invoicesAdmin.ts`）を書いた |
| 影響 | 1操作足すたびにコピーが増える。エラー正規化の分岐が5箇所に散っており、`ProviderError` の扱いを直すときに全部を直す必要がある |
| 回避策 | テンプレートの `usersAdmin.ts` の構造をそのまま踏襲して5つ目を書いた（独自の書き方をするより、直すときに一括で追える方がよい） |
| 分類 | Banto共通（CRUD に収まらない操作はどの派生アプリでも出る） |
| Banto Issue | 未起票 |
| 状態 | 未対応（提案：`@banto/admin-core` に「認証済みで CSRF ヘッダ付きの Tauri/HTTP 二経路呼び出し」だけを担う薄いヘルパ（例 `createCommandClient()`）を置く。DataProvider の契約は広げず、transport の重複だけを畳む） |

---

### [2026-08-20] Phase 6 — 導出値の読み取りが DataProvider の規約にそのまま載った（肯定的な記録）

| 項目 | 内容 |
|---|---|
| 事象 | 入金管理では「請求書1件の入金状況」「未入金一覧」という**テーブルを持たない導出値**を読む必要があった。`${resource}_get` / `${resource}_list` の規約に合わせて `settlements`（id = 請求書 id）と `outstanding` という読み取り専用の疑似リソースにしたところ、専用クライアントを足さずに `getOne` / `getList` でそのまま取得できた |
| 影響 | Phase 5 で書いた `invoicesAdmin.ts`（CRUD に載らない操作の専用クライアント）を増やさずに済んだ。`ListParams` を受け取って無視する `outstanding_list` は `work_categories` と同じ形で、規約の範囲内 |
| 回避策 | 不要（規約にそのまま載った） |
| 分類 | Banto共通（肯定的な記録） |
| Banto Issue | — |
| 状態 | 対応不要。ただし `docs/recipes/add-resource.md` は「テーブルを持つリソース」を前提に書かれているので、**読み取り専用の導出リソースも同じ規約で書ける**ことを1行足すと、派生アプリが専用クライアントへ逃げずに済む |

---

### [2026-08-20] Phase 7 — バックアップ復元の必須テーブル検査にデモリソース `items` が直書きされている

| 項目 | 内容 |
|---|---|
| 事象 | `crates/banto-admin-services/src/backup.rs` の `REQUIRED_TABLES` が `["items", "settings", "users", "audit_log"]` と直書きされている。`items` は Banto テンプレートの**デモリソース**で、Banto 自身の派生手順（`docs/plan.md` Phase 0「不要なデモリソース削除」）が削除を促しているテーブル。削除して `DROP TABLE items` を流すと、以後に作ったバックアップが全て「必須テーブルが無い」で `Validation` になり、**復元が丸ごと機能しなくなる** |
| 影響 | 派生アプリがデモを片付けた瞬間にバックアップ復元（spec M17）が壊れる。作成側は成功するので、壊れていることに気付くのは復元しようとした時＝最悪のタイミング |
| 回避策 | 未定（第2章に従い、回避実装を作り込む前に確認を取る）。候補は (a) `items` テーブルだけ空のまま残す、(b) 必須テーブルからアプリ固有の1件を外し `settings` / `users` / `audit_log` の3つ（＝Banto が必ず作るもの）で判定する |
| 分類 | Banto共通 |
| Banto Issue | 未起票 |
| 状態 | 確認待ち。検査の意図は「Banto の DB かどうかの粗い判定」なので、判定対象は Banto 自身が必ず作るテーブルに限るのが筋に見える |

---

### [2026-08-20] Phase 7 — `NumberField` の空欄が `''` で返り、そのまま送ると「保存しても何も起きない」

| 項目 | 内容 |
|---|---|
| 事象 | `@banto/forms` の `NumberField` は空欄を **`''`（空文字）** として返す（`onInput: (value: number \| '') => void` が明示的な契約）。これを `BantoForm` の `onSubmit(store.values)` からそのまま DataProvider へ渡すと、Rust 側の `Option<i64>` に対して `invalid type: string "", expected i64` で **422** になる |
| 影響 | **任意入力の数値欄を空のまま保存すると、画面上は何も起きない。** 本文のデシリアライズ失敗なのでフィールド単位のエラーにならず、`setServerErrors` で入力欄へ戻すこともできない。案件の見積額・契約額、工数の適用レート（**空欄ならサーバが既定レートを引く**のが要件 F-W2 の仕様）が該当し、実際に壊れていた。`items` デモは必須の数値欄しか持たなかったため、テンプレート同梱の E2E では一度も踏まなかった |
| 回避策 | アプリ側に `src/lib/banto/formValues.ts` の `normalizeFormValues()` を置き、送信直前に number 型の空欄を `null` へ潰す。全ての `new` / `[id]` ページで通す |
| 分類 | Banto共通 |
| Banto Issue | 未起票 |
| 状態 | 回避済み。`''` と `null` の使い分けはドメイン判断（キーごと省略したい API もある）なので `NumberField` の契約自体は妥当に見えるが、**`BantoForm` が `onSubmit` へ渡す前に潰すか、少なくともレシピに1行書く**のが筋。派生アプリは全員これを踏み、しかも「保存が無反応」という最も原因を追いにくい形で踏む |

---

### [2026-08-20] Phase 7 — カレンダー表示のコンポーネントが Banto に無い

| 項目 | 内容 |
|---|---|
| 事象 | 月カレンダー（月グリッドに日別の集計を置く画面）を作ろうとしたが、`@banto/*` に該当する部品が無い（`grid-svelte` は表形式、`charts` はグラフ、`tree-svelte` は階層）。日付の升目・週の折り返し・前後月の埋め草・曜日見出しといった、ドメインに依存しない部分から自前で書く必要があった |
| 影響 | Business 固有ではない。工数・案件のような業務データに限らず、**日付を持つレコードを月で俯瞰したい要求は業種を問わず出る**（`banto-industrial` でも点検予定・稼働実績などで同じものが要りそう、という指摘が利用者からあった）。各派生アプリが同じ月グリッドを書き直すことになる |
| 回避策 | Business 側の app 層に `src/lib/components/calendar/`（`month.ts` / `types.ts` / `MonthGrid.svelte`）として実装。**昇格しやすい形を最初から保つ**ことで回避のコストを抑えた: ①`$lib` import を持たない（§5）②文言は解決済み文字列を `messages` props で受け取る（ADR-0005 レイヤ①注入方式）③業務の型を知らず `CalendarCell` という表示専用の契約だけを受け取る ④色は `--banto-chart-*` 等の theme 変数のみ（§9）⑤`new Date()` を呼ばず「今日」は props で受け取る。業務データからの翻訳は `components/business/WorkCalendar.svelte` に隔離した |
| 分類 | Banto共通 |
| Banto Issue | 未起票 |
| 状態 | **昇格は保留（利用者と合意済み）。** まず Business で実運用（Phase 7）に使い、必要な機能が出揃ってから `calendar-svelte`（スコープ付きの名前はまだ存在しないので、ここでは意図的にスコープ無しで書く）として切り出すかを決める。先に汎用化すると、使う前に想像で API を決めることになる。切り出す場合の移設先は上記3ファイルのみで、`WorkCalendar.svelte` は Business に残る |

---

### [2026-08-20] Phase 7 — app 層に単体テストの実行環境が無い

| 項目 | 内容 |
|---|---|
| 事象 | `packages/*` は vitest を持つが、`apps/admin-template` の `package.json` には `test` スクリプトが無い。テンプレートの app 層はロジックを持たない前提（`items` デモは薄い CRUD 画面だけ）だったため、テスト対象が無かったものと思われる |
| 影響 | 派生アプリが app 層に純粋なロジック（今回は月グリッドの日付計算）を置いた瞬間、それを検査する手段が無い。ルートの `pnpm test` は `--recursive --if-present` なので、スクリプトが無い app は**黙って飛ばされる**（CI も緑のまま） |
| 回避策 | `apps/admin-template` に `vitest` を devDependency として足し、`vitest.config.ts` を別ファイルで置いた（`vite.config.ts` は SvelteKit / Paraglide / Tailwind のプラグインを読むので、テストのためだけに `svelte-kit sync` 済みを要求してしまう）。対象は `src/**/*.test.ts` のみで、`.svelte` のテストは扱わない（CLAUDE.md 第6章） |
| 分類 | Banto共通 |
| Banto Issue | 未起票 |
| 状態 | 回避済み。テンプレートが app 層に最小の `test` スクリプトを持っていれば、派生アプリは「テストを書く場所がある」ところから始められる。設定ファイルを分ける理由も含めてレシピに1行あるとよい |

---

### [記録例・削除可] Phase 2 — Grid の列幅がリソース定義から指定できない

| 項目 | 内容 |
|---|---|
| 事象 | `@banto/grid-svelte` でリソース定義側から初期列幅を指定する手段が見当たらない |
| 影響 | Customer一覧で顧客名が省略表示され、毎回手動リサイズが必要 |
| 回避策 | コンポーネント側で `onMount` 後に幅を上書き |
| 分類 | Banto共通 |
| Banto Issue | 未起票 |
| 状態 | 未対応 |

---

## Phase 別サマリ

各 Phase 完了時に更新する。

| Phase | 記録件数 | うち Banto共通 | 主な傾向 |
|---|---|---|---|
| 0 | 2 | 2 | 派生（rename / バージョン固定）の導線に穴 |
| 2 | 5 | 5 | フォームの表現力（説明文）とデモ資産の重さ。機械検査は有効に機能 |
| 3 | 2 | 2 | 命名規約の暗黙の制約と、ユーティリティの重複 |
| 4 | 2 | 2 | 派生アプリがテーブルを足すと壊れるテスト補助と、未定義でも黙って通るテーマ変数 |
| 5 | 1 | 1 | CRUD 契約に載らない操作のクライアント配線が毎回コピーになる |
| 6 | 1 | 1 | 導出値の読み取りは規約にそのまま載った（肯定的な記録） |
| 7 | 4 | 4 | デモリソース削除を前提にしていない箇所・デモが踏まなかった経路の穴と、app 層に無い部品（カレンダー）／無い足場（単体テスト） |

---

## 総括（Phase 7 完了後に記入）

### Banto が汎用基盤として成立していたか

TODO

### 産業ドメインに寄りすぎていた箇所

TODO

### 事業管理ドメインで新たに必要になった共通機能

TODO

### Banto へ還元した項目

TODO
