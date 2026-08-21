/**
 * `work_logs` リソース定義（docs/recipes/add-resource.md 手順7）。
 *
 * リソース名にハイフンを使わないのは、`@banto/admin-core` の DataProvider が
 * Tauri コマンドを `${resource}_list` の規約で呼ぶため（`work-logs_list` は
 * Rust の識別子として不正）。REST パスも同じ綴りに揃えてある。
 *
 * フィールド名は Rust の `WorkLogInput`
 * （apps/admin-template/core/src/work_logs.rs）と1対1。
 *
 * **金額計算はフロントで行わない**（AGENTS.md 第1章）。内部原価は
 * サーバ側が `floor(分 × 単価 ÷ 60)` で確定した値を読み取り専用で表示する。
 */
import type { ResourceDefinition } from '@banto/admin-core';
import type { FormSchema } from '@banto/forms';
import * as m from '$lib/paraglide/messages';
import { projectOptions } from '$lib/banto/referenceOptions.svelte';

const integerValidate = (value: unknown): string | null =>
	Number.isInteger(Number(value)) ? null : m['validation.integer']();

export const workLogsSchema: FormSchema = {
	fields: [
		{
			name: 'projectId',
			get label() {
				return m['workLogs.fieldProjectId']();
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
			name: 'workedOn',
			get label() {
				return m['workLogs.fieldWorkedOn']();
			},
			type: 'date',
			required: true
		},
		{
			name: 'workCategoryCode',
			get label() {
				return m['workLogs.fieldWorkCategoryCode']();
			},
			type: 'text',
			required: true
		},
		{
			name: 'minutes',
			get label() {
				return m['workLogs.fieldMinutes']();
			},
			type: 'number',
			required: true,
			min: 1,
			max: 1440,
			validate: integerValidate
		},
		{
			name: 'appliedRate',
			get label() {
				return m['workLogs.fieldAppliedRate']();
			},
			type: 'number',
			min: 0,
			max: 1000000,
			validate: integerValidate,
			// 空欄なら作業分類の既定レートをサーバが引く（要件 F-W2）。
			get placeholder() {
				return m['workLogs.rateHint']();
			}
		},
		{
			name: 'internalCost',
			get label() {
				return m['workLogs.fieldInternalCost']();
			},
			type: 'number',
			// サーバが確定する値。フロントで計算も編集もしない。
			readonly: true
		},
		{
			name: 'description',
			get label() {
				return m['workLogs.fieldDescription']();
			},
			type: 'text',
			max: 500
		},
		{
			name: 'invoiced',
			get label() {
				return m['workLogs.fieldInvoiced']();
			},
			type: 'checkbox'
		},
		{
			name: 'updatedAt',
			get label() {
				return m['workLogs.fieldUpdatedAt']();
			},
			type: 'date',
			readonly: true
		}
	]
};

export const workLogsResource: ResourceDefinition = {
	name: 'work_logs',
	get label() {
		return m['workLogs.resourceLabel']();
	},
	icon: '⏱️',
	schema: workLogsSchema,
	capabilities: { list: true, create: true, edit: true, delete: true }
};
