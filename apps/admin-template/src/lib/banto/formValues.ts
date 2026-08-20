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
 * そこで送信直前に、**number 型の空欄を `null` に潰す**。値を省略するのではなく
 * `null` を送るのは、更新（PUT）で「空にする」と「触っていない」を区別する必要が
 * あるため（Rust 側はどちらも `Option<i64>` の `None` として受ける）。
 *
 * 対象を number 型に限るのは意図的。text / date の空欄は `Option<String>` に
 * `Some("")` として入り、サービス層が `None` へ畳む既存の扱いがあるので、
 * ここで触ると二重に意味を変えてしまう。
 */
import type { FormSchema } from '@banto/forms';

export function normalizeFormValues(
	schema: FormSchema,
	values: Record<string, unknown>
): Record<string, unknown> {
	const numericFields = new Set(
		schema.fields.filter((field) => field.type === 'number').map((field) => field.name)
	);
	const out: Record<string, unknown> = { ...values };
	for (const name of numericFields) {
		if (out[name] === '') out[name] = null;
	}
	return out;
}
