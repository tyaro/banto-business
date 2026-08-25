/**
 * 経費フォームの `taxCategory`（税区分）を、分類選択の変化に合わせて
 * 既定値で埋める（P2-2、`docs/mobile-ui-plan.md`）。
 *
 * サーバ（`core/src/expenses.rs`）は既に「`taxCategory` が空なら
 * `expense_categories.default_tax_category` を使う」実装を持つ。今回は
 * フォーム UX だけを足す —— 選ぶたびに画面上でも既定値が見えて、直せる
 * ようにする。
 *
 * ## 確定済みの挙動（変更禁止。要確認事項として承認済み）
 *
 * - **分類変更で毎回上書き。** 新規・編集どちらでも、経費分類を選ぶ/変える
 *   たびに税区分へその分類の既定値を入れる。手動選択した後でも、分類を
 *   変えればまた既定に戻る（画面に見えるので直せる）。
 * - **編集画面の初期表示（ロード直後）では上書きしない。** 保存済みの値を
 *   そのまま見せる。呼び出し側が「最後に見た分類コード」を読み込んだ行の
 *   値で初期化することでこれを実現する（`new`/`[id]` 両ページで共有する
 *   このヘルパ自体はロード有無を知らない）。
 * - **既定が引けない分類（データ不整合等）では何もしない。** 税区分を
 *   空にしたり、直前の値を消したりしない。
 */

/**
 * 経費分類コードの変化を追跡し、変わるたびに税区分の既定値を返す
 * トラッカー。
 *
 * `new` ページは `''`（未選択）から、`[id]` ページはロードした行の
 * `expenseCategoryCode` から始める —— これが「編集の初期表示では
 * 上書きしない」を満たす（`sync` の最初の呼び出しが no-op になる）。
 */
export class ExpenseTaxCategoryTracker {
	#lastSeenCode: string;

	constructor(initialCategoryCode: string) {
		this.#lastSeenCode = initialCategoryCode;
	}

	/**
	 * 現在の経費分類コードを渡す。**分類が前回と変わっていて、かつ既定値が
	 * 引ける場合だけ**その既定の税区分コードを返す。それ以外（変わって
	 * いない／既定が引けない）は `null`。
	 *
	 * `lookupDefault` を注入するのは、選択肢の読み込み元
	 * （`referenceOptions.svelte.ts`）に依存させず、このモジュールを
	 * `.svelte.ts` ではないプレーンな TypeScript のまま vitest で
	 * テストできるようにするため。
	 */
	sync(categoryCode: string, lookupDefault: (code: string) => string | null): string | null {
		if (categoryCode === this.#lastSeenCode) return null;
		this.#lastSeenCode = categoryCode;
		return lookupDefault(categoryCode);
	}
}
