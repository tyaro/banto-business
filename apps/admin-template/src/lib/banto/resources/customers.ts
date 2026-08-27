/**
 * `customers` リソース定義（docs/recipes/add-resource.md 手順7）。
 *
 * フィールド名は Rust の `CustomerInput`
 * （apps/admin-template/core/src/customers.rs）と1対1に対応させる —
 * `BantoError::Validation` のフィールドエラーがそのまま該当の入力欄へ
 * 戻るため（`items.ts` と同じ流儀）。
 *
 * i18n（ADR-0005 / conventions §13）: ラベルは `get label()` ゲッターで
 * 遅延解決する。このモジュールは locale.ts が client strategy を登録する
 * 前（initBanto 時）に評価されるため、即時に `m['…']()` を呼ぶと英語の
 * baseLocale に固定されてしまう。
 */
import type { ResourceDefinition } from '@banto/admin-core';
import type { FieldOption, FormSchema } from '@banto/forms';
import * as m from '$lib/paraglide/messages';

/** 月末を表す番兵値。Rust 側 `customers::DAY_END_OF_MONTH` と同じ値。 */
export const DAY_END_OF_MONTH = 99;

const MAX_PAYMENT_MONTH_OFFSET = 6;

/**
 * 締日・支払日は空欄（未設定）、または 1〜28・99（末日）のみ許す
 * （Phase 1 決定 C-8：2月に存在しない日を業務日付の元データとして持たない
 * ため）。3項目とも**任意**（アルファ実使用からのフィードバックで
 * 2026-08-27 に必須を撤廃 — 導入時点で支払条件が決まっていない顧客も
 * 登録でき、後から埋められる）。サーバ側（customers.rs::check_day）と
 * 同じ規則をクライアントでも先に弾く。
 *
 * `@banto/forms` の `validateField` は空でも `def.validate` を必ず呼ぶ
 * （`required` の判定とは独立）ので、**空を先に弾いて合法扱いにする**の
 * はこの関数の責務。
 */
const dayValidate = (value: unknown): string | null => {
	if (value === '' || value === null || value === undefined) return null;
	const day = Number(value);
	if (!Number.isInteger(day)) return m['validation.integer']();
	if (day === DAY_END_OF_MONTH) return null;
	return day >= 1 && day <= 28 ? null : m['customers.dayEndOfMonth']();
};

/** 支払サイト（締日から何ヶ月後）の検証。空欄（未設定）は合法。 */
const paymentMonthOffsetValidate = (value: unknown): string | null => {
	if (value === '' || value === null || value === undefined) return null;
	const offset = Number(value);
	if (!Number.isInteger(offset)) return m['validation.integer']();
	return offset >= 0 && offset <= MAX_PAYMENT_MONTH_OFFSET
		? null
		: m['customers.paymentMonthOffsetRange']({ max: MAX_PAYMENT_MONTH_OFFSET });
};

/**
 * 締日・支払日の選択肢: 未設定 + 1〜28日 + 末日。プレースホルダー
 * （`SelectField` が別途出す disabled/hidden の先頭項目）とは別に、
 * **選び直せる「未設定」を実項目として持つ** —— 一度日付を選んだ後でも
 * 未設定へ戻せるようにするため（プレースホルダーは選び直せない）。
 */
const dayOptions = (): FieldOption[] => [
	{ value: '', label: m['customers.optionUnset']() },
	...Array.from({ length: 28 }, (_, i) => i + 1).map((day) => ({
		value: day,
		label: m['customers.optionDay']({ day })
	})),
	{ value: DAY_END_OF_MONTH, label: m['customers.optionEndOfMonth']() }
];

/** 支払サイトの名前（0=当月払い、1=翌月払い、2=翌々月払い、3〜6=n ヶ月後）。 */
function paymentSiteName(offset: number): string {
	if (offset === 0) return m['customers.paymentSiteCurrentMonth']();
	if (offset === 1) return m['customers.paymentSiteNextMonth']();
	if (offset === 2) return m['customers.paymentSiteMonthAfterNext']();
	return m['customers.paymentSiteMonthsLater']({ n: offset });
}

/** 支払サイトの選択肢: 未設定 + 当月払い/翌月払い/翌々月払い/3〜6ヶ月後。 */
const paymentMonthOffsetOptions = (): FieldOption[] => [
	{ value: '', label: m['customers.optionUnset']() },
	...[0, 1, 2, 3, 4, 5, 6].map((offset) => ({ value: offset, label: paymentSiteName(offset) }))
];

/**
 * 一覧グリッドのセル表示用: オフセット → 表示名。未設定（`null`/`undefined`）
 * は「—」（`settings/+page.svelte` の未設定表示と同じ記法。生の記号であって
 * 日本語リテラルではないため conventions §13 の対象外）。
 */
export function paymentMonthOffsetLabel(value: unknown): string {
	if (value === null || value === undefined || value === '') return '—';
	const offset = Number(value);
	return Number.isInteger(offset) ? paymentSiteName(offset) : String(value);
}

export const customersSchema: FormSchema = {
	fields: [
		{
			name: 'name',
			get label() {
				return m['customers.fieldName']();
			},
			type: 'text',
			required: true,
			min: 1,
			max: 60
		},
		{
			name: 'contactPerson',
			get label() {
				return m['customers.fieldContactPerson']();
			},
			type: 'text',
			max: 120
		},
		{
			name: 'billingName',
			get label() {
				return m['customers.fieldBillingName']();
			},
			type: 'text',
			max: 60
		},
		{
			name: 'address',
			get label() {
				return m['customers.fieldAddress']();
			},
			type: 'text',
			max: 120
		},
		{
			name: 'phone',
			get label() {
				return m['customers.fieldPhone']();
			},
			type: 'text',
			max: 120
		},
		{
			name: 'email',
			get label() {
				return m['customers.fieldEmail']();
			},
			type: 'text',
			max: 120
		},
		{
			name: 'closingDay',
			get label() {
				return m['customers.fieldClosingDay']();
			},
			// 任意（2026-08-27、アルファ実使用からのフィードバックで必須を
			// 撤廃）。手打ちの数値ではなく選択式にする —— 「29〜31 は存在
			// しない日」という制約を選択肢自体で表現でき、末日 = 99 という
			// 番兵値の意味も選ばせる時点で説明できる。
			type: 'select',
			get options() {
				return dayOptions();
			},
			validate: dayValidate
		},
		{
			name: 'paymentMonthOffset',
			get label() {
				return m['customers.fieldPaymentMonthOffset']();
			},
			// 任意（同上）。実体は支払サイト（締日から何ヶ月後、0〜6）。
			// アルファ実使用で「支払月」という項目名がそのまま「何月」と
			// 誤解された（ユーザーからのフィードバック）ため、選択式にして
			// 「当月払い」「翌月払い」…という言葉で選ばせる。
			type: 'select',
			get options() {
				return paymentMonthOffsetOptions();
			},
			validate: paymentMonthOffsetValidate
		},
		{
			name: 'paymentDay',
			get label() {
				return m['customers.fieldPaymentDay']();
			},
			// 任意（同上）。
			type: 'select',
			get options() {
				return dayOptions();
			},
			validate: dayValidate
		},
		{
			name: 'note',
			get label() {
				return m['customers.fieldNote']();
			},
			type: 'text',
			max: 500
		},
		{
			name: 'updatedAt',
			get label() {
				return m['customers.fieldUpdatedAt']();
			},
			type: 'date',
			readonly: true
		},
		{
			// **入力欄の末尾に置く。** 自動採番されるので普段は触らない欄で、
			// 先頭にあると「まず何か入れる欄」に見えてしまう（スマホでは特に）。
			// 会計ソフト側のコードに合わせたいときだけ使う。
			name: 'code',
			get label() {
				return m['customers.fieldCode']();
			},
			type: 'text',
			// 必須にしない: 空欄で保存すると Rust 側が C001 を採番する。
			// 案件番号（要件 F-M3）と同じ扱いに揃えた —— 個人事業では顧客
			// コードを自分で決める意味が薄く、毎回考えるのは手間でしかない。
			// 会計ソフト側の得意先コードに合わせたい場合のために入力もできる。
			max: 20,
			// FieldDef に hint は無いので、自動採番の説明は placeholder で出す
			// （案件の code 欄と同じ理由）。
			get placeholder() {
				return m['customers.codeAutoHint']();
			}
		}
	]
};

export const customersResource: ResourceDefinition = {
	name: 'customers',
	get label() {
		return m['customers.resourceLabel']();
	},
	icon: '🏢',
	schema: customersSchema,
	capabilities: { list: true, create: true, edit: true, delete: true }
};
