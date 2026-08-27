/**
 * フォームの値を送信用に整える。
 *
 * `@banto/forms` の `NumberField` は空欄を **`''`（空文字）** として返す
 * （`onInput: (value: number | '') => void` が明示的な契約）。これをそのまま
 * DataProvider へ渡すと、Rust 側の `Option<i64>` に対して
 * `invalid type: string "", expected i64` で **422** になる。しかもこれは
 * フィールド単位の検証エラーではなく本文のデシリアライズ失敗なので、
 * `setServerErrors` で入力欄へ戻すこともできない — 画面上は「保存を押しても
 * 何も起きない」状態になる。
 *
 * `SelectField` も同じ形で `''` を返す —— 未選択なら `FormStore` の初期値
 * （`defaultsFrom`）がそもそも `''`。数値の選択肢（例: 顧客の締日・支払サイト・
 * 支払日、`customers.ts`）を持つ任意項目の select は、未選択のまま送信すると
 * 同じ 422 を踏む。
 *
 * そこで送信直前に、**number / select 型の空欄を `null` に潰す**。値を
 * 省略するのではなく `null` を送るのは、更新（PUT）で「空にする」と
 * 「触っていない」を区別する必要があるため（Rust 側はどちらも
 * `Option<i64>` の `None` として受ける）。
 *
 * select を含めても安全なのは、値の型に関わらず `''` が「未選択」の意味しか
 * 持たないため（文字列値の select、例: `expenses.ts` の `taxCategory` は
 * 元々 `Option<String>` で、サービス層が `None` と `Some("")` を同じに畳む
 * 既存の扱いがあるので、ここで `null` に変えても意味は変わらない）。
 *
 * 対象を number / select に限るのは意図的。text / date の空欄は
 * `Option<String>` に `Some("")` として入り、サービス層が `None` へ畳む
 * 既存の扱いがあるので、ここで触ると二重に意味を変えてしまう。
 */
import type { FormSchema } from '@banto/forms';

export function normalizeFormValues(
	schema: FormSchema,
	values: Record<string, unknown>
): Record<string, unknown> {
	const emptyableFields = new Set(
		schema.fields
			.filter((field) => field.type === 'number' || field.type === 'select')
			.map((field) => field.name)
	);
	const out: Record<string, unknown> = { ...values };
	for (const name of emptyableFields) {
		if (out[name] === '') out[name] = null;
	}
	return out;
}
