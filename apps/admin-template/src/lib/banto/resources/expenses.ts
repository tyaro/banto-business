/**
 * `expenses` リソース定義（docs/recipes/add-resource.md 手順7）。
 *
 * フィールド名は Rust の `ExpenseInput`
 * （apps/admin-template/core/src/expenses.rs）と1対1。
 *
 * `taxCategory` は**仕入側**の区分。顧客への再請求は一律 10%
 * （Phase 1 決定 B-5）で、そちらは Phase 5 の請求明細が持つ。
 */
import type { FieldOption, FormSchema } from '@banto/forms';
import type { ResourceDefinition } from '@banto/admin-core';
import * as m from '$lib/paraglide/messages';
import { projectOptions, expenseCategoryOptions } from '$lib/banto/referenceOptions.svelte';

/** 仕入側の税区分。Rust の `expenses::TAX_CATEGORIES` と同じ4値。 */
export const TAX_CATEGORIES: { code: string; label: () => string }[] = [
	{ code: 'STANDARD_10', label: () => m['expenses.taxStandard10']() },
	{ code: 'REDUCED_8', label: () => m['expenses.taxReduced8']() },
	{ code: 'EXEMPT', label: () => m['expenses.taxExempt']() },
	{ code: 'OUT_OF_SCOPE', label: () => m['expenses.taxOutOfScope']() }
];

export function taxCategoryLabel(code: unknown): string {
	return TAX_CATEGORIES.find((c) => c.code === code)?.label() ?? String(code ?? '');
}

const taxOptions = (): FieldOption[] =>
	TAX_CATEGORIES.map((c) => ({ value: c.code, label: c.label() }));

const integerValidate = (value: unknown): string | null =>
	Number.isInteger(Number(value)) ? null : m['validation.integer']();

export const expensesSchema: FormSchema = {
	fields: [
		{
			name: 'projectId',
			get label() {
				return m['expenses.fieldProjectId']();
			},
			// 内部 id を手で打たせない（`referenceOptions` の doc を参照）。
			// スマホでは数値キーボードしか出ず、案件名で探せないため。
			type: 'select',
			required: true,
			get options() {
				return projectOptions();
			}
		},
		{
			name: 'spentOn',
			get label() {
				return m['expenses.fieldSpentOn']();
			},
			type: 'date',
			required: true
		},
		{
			name: 'expenseCategoryCode',
			get label() {
				return m['expenses.fieldExpenseCategoryCode']();
			},
			// コードを手打ちさせない（`referenceOptions` の doc を参照）。
			type: 'select',
			required: true,
			get options() {
				return expenseCategoryOptions();
			}
		},
		{
			name: 'payee',
			get label() {
				return m['expenses.fieldPayee']();
			},
			type: 'text',
			max: 120
		},
		{
			name: 'amount',
			get label() {
				return m['expenses.fieldAmount']();
			},
			type: 'number',
			required: true,
			min: 0,
			max: 9999999999,
			validate: integerValidate
		},
		{
			name: 'taxCategory',
			get label() {
				return m['expenses.fieldTaxCategory']();
			},
			type: 'select',
			get options() {
				return taxOptions();
			}
		},
		{
			name: 'description',
			get label() {
				return m['expenses.fieldDescription']();
			},
			type: 'text',
			max: 500
		},
		{
			name: 'billable',
			get label() {
				return m['expenses.fieldBillable']();
			},
			type: 'checkbox'
		},
		{
			name: 'invoiced',
			get label() {
				return m['expenses.fieldInvoiced']();
			},
			type: 'checkbox'
		},
		{
			name: 'updatedAt',
			get label() {
				return m['expenses.fieldUpdatedAt']();
			},
			type: 'date',
			readonly: true
		}
	]
};

export const expensesResource: ResourceDefinition = {
	name: 'expenses',
	get label() {
		return m['expenses.resourceLabel']();
	},
	icon: '🧾',
	schema: expensesSchema,
	capabilities: { list: true, create: true, edit: true, delete: true }
};
