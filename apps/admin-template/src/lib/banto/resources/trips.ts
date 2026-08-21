/**
 * `trips` リソース定義（docs/recipes/add-resource.md 手順7）。
 *
 * 一括生成（要件 F-T1）の入力はこのスキーマに含めない — 生成は「登録時に
 * 1回だけ」の操作で、編集フォームに出すと更新のたびに再生成できるように
 * 見えてしまうため。生成用の入力は新規作成ページが専用 UI で持つ。
 */
import type { ResourceDefinition } from '@banto/admin-core';
import type { FormSchema } from '@banto/forms';
import * as m from '$lib/paraglide/messages';
import { projectOptions } from '$lib/banto/referenceOptions.svelte';

const integerValidate = (value: unknown): string | null =>
	Number.isInteger(Number(value)) ? null : m['validation.integer']();

export const tripsSchema: FormSchema = {
	fields: [
		{
			name: 'projectId',
			get label() {
				return m['trips.fieldProjectId']();
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
			name: 'destination',
			get label() {
				return m['trips.fieldDestination']();
			},
			type: 'text',
			required: true,
			min: 1,
			max: 120
		},
		{
			name: 'startOn',
			get label() {
				return m['trips.fieldStartOn']();
			},
			type: 'date',
			required: true
		},
		{
			name: 'endOn',
			get label() {
				return m['trips.fieldEndOn']();
			},
			type: 'date',
			required: true
		},
		{
			name: 'onsiteDays',
			get label() {
				return m['trips.fieldOnsiteDays']();
			},
			type: 'number',
			required: true,
			min: 0,
			max: 60,
			validate: integerValidate
		},
		{
			name: 'nights',
			get label() {
				return m['trips.fieldNights']();
			},
			type: 'number',
			required: true,
			min: 0,
			max: 60,
			validate: integerValidate
		},
		{
			name: 'note',
			get label() {
				return m['trips.fieldNote']();
			},
			type: 'text',
			max: 500
		},
		{
			name: 'updatedAt',
			get label() {
				return m['trips.fieldUpdatedAt']();
			},
			type: 'date',
			readonly: true
		}
	]
};

export const tripsResource: ResourceDefinition = {
	name: 'trips',
	get label() {
		return m['trips.resourceLabel']();
	},
	icon: '🚄',
	schema: tripsSchema,
	capabilities: { list: true, create: true, edit: true, delete: true }
};
