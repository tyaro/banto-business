/**
 * `projects` リソース定義（docs/recipes/add-resource.md 手順7）。
 *
 * フィールド名は Rust の `ProjectInput`
 * （apps/admin-template/core/src/projects.rs）と1対1に対応させる。
 * 状態のコード値も Rust の `ProjectStatus::as_code` と一致させること —
 * 片方だけ増やすと、保存時に「状態の値が不正です」で弾かれる。
 */
import type { ResourceDefinition } from '@banto/admin-core';
import type { FieldOption, FormSchema } from '@banto/forms';
import * as m from '$lib/paraglide/messages';
import { customerOptions } from '$lib/banto/referenceOptions.svelte';

/**
 * 案件の状態（Phase 1 決定 C-12）。Rust 側 `ProjectStatus` と同じ7値。
 * ラベルは locale 解決を遅らせるため関数で持つ（`customers.ts` と同じ理由）。
 */
export const PROJECT_STATUSES: { code: string; label: () => string }[] = [
	{ code: 'PROSPECT', label: () => m['projects.statusProspect']() },
	{ code: 'ORDERED', label: () => m['projects.statusOrdered']() },
	{ code: 'IN_PROGRESS', label: () => m['projects.statusInProgress']() },
	{ code: 'AWAITING_ACCEPTANCE', label: () => m['projects.statusAwaitingAcceptance']() },
	{ code: 'COMPLETED', label: () => m['projects.statusCompleted']() },
	{ code: 'LOST', label: () => m['projects.statusLost']() },
	{ code: 'ON_HOLD', label: () => m['projects.statusOnHold']() }
];

/** 一覧のセル表示用: コード → 表示名。未知のコードはそのまま返す。 */
export function projectStatusLabel(code: unknown): string {
	return PROJECT_STATUSES.find((s) => s.code === code)?.label() ?? String(code ?? '');
}

const statusOptions = (): FieldOption[] =>
	PROJECT_STATUSES.map((s) => ({ value: s.code, label: s.label() }));

const integerValidate = (value: unknown): string | null =>
	Number.isInteger(Number(value)) ? null : m['validation.integer']();

export const projectsSchema: FormSchema = {
	fields: [
		{
			name: 'customerId',
			get label() {
				return m['projects.fieldCustomerId']();
			},
			// 案件の `projectId` と同じ理由で選択式にする（`referenceOptions`）。
			type: 'select',
			required: true,
			get options() {
				return customerOptions();
			}
		},
		{
			name: 'name',
			get label() {
				return m['projects.fieldName']();
			},
			type: 'text',
			required: true,
			min: 1,
			max: 80
		},
		{
			name: 'status',
			get label() {
				return m['projects.fieldStatus']();
			},
			type: 'select',
			required: true,
			get options() {
				return statusOptions();
			}
		},
		{
			name: 'startedOn',
			get label() {
				return m['projects.fieldStartedOn']();
			},
			type: 'date'
		},
		{
			name: 'dueOn',
			get label() {
				return m['projects.fieldDueOn']();
			},
			type: 'date'
		},
		{
			name: 'estimateAmount',
			get label() {
				return m['projects.fieldEstimateAmount']();
			},
			type: 'number',
			min: 0,
			max: 9999999999,
			validate: integerValidate
		},
		{
			name: 'contractAmount',
			get label() {
				return m['projects.fieldContractAmount']();
			},
			type: 'number',
			min: 0,
			max: 9999999999,
			validate: integerValidate
		},
		{
			name: 'scope',
			get label() {
				return m['projects.fieldScope']();
			},
			type: 'text',
			max: 200
		},
		{
			name: 'note',
			get label() {
				return m['projects.fieldNote']();
			},
			type: 'text',
			max: 500
		},
		{
			name: 'updatedAt',
			get label() {
				return m['projects.fieldUpdatedAt']();
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
				return m['projects.fieldCode']();
			},
			type: 'text',
			// 必須にしない: 空欄で保存すると Rust 側が YYYY-NNN を採番する
			// （要件 F-M3）。採番後は通常の編集対象。
			max: 20,
			// FieldDef に hint は無いので、自動採番の説明は placeholder で出す
			// （packages/forms/src/types.ts の FieldDef を変えない ＝
			// conventions §4/§5: パッケージ契約をアプリ都合で広げない）。
			get placeholder() {
				return m['projects.codeAutoHint']();
			}
		}
	]
};

export const projectsResource: ResourceDefinition = {
	name: 'projects',
	get label() {
		return m['projects.resourceLabel']();
	},
	icon: '📁',
	schema: projectsSchema,
	capabilities: { list: true, create: true, edit: true, delete: true }
};
