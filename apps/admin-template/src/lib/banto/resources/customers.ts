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
import type { FormSchema } from '@banto/forms';
import * as m from '$lib/paraglide/messages';

/** 月末を表す番兵値。Rust 側 `customers::DAY_END_OF_MONTH` と同じ値。 */
export const DAY_END_OF_MONTH = 99;

/**
 * 締日・支払日は 1〜28 または 99（末日）のみ許す（Phase 1 決定 C-8：
 * 2月に存在しない日を業務日付の元データとして持たないため）。サーバ側
 * （customers.rs::check_day）と同じ規則をクライアントでも先に弾く。
 */
const dayValidate = (value: unknown): string | null => {
	const day = Number(value);
	if (!Number.isInteger(day)) return m['validation.integer']();
	if (day === DAY_END_OF_MONTH) return null;
	return day >= 1 && day <= 28 ? null : m['customers.dayEndOfMonth']();
};

const integerValidate = (value: unknown): string | null =>
	Number.isInteger(Number(value)) ? null : m['validation.integer']();

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
			type: 'number',
			required: true,
			validate: dayValidate
		},
		{
			name: 'paymentMonthOffset',
			get label() {
				return m['customers.fieldPaymentMonthOffset']();
			},
			type: 'number',
			required: true,
			min: 0,
			max: 6,
			validate: integerValidate
		},
		{
			name: 'paymentDay',
			get label() {
				return m['customers.fieldPaymentDay']();
			},
			type: 'number',
			required: true,
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
