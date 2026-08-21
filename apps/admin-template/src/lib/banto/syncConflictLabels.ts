/**
 * 衝突を人が読める形にする（Phase 8、`docs/domain/sync.md` 11.11）。
 *
 * `SyncRow` は**DB の列がそのまま**入っている（`sync/rows.rs` の目録）。
 * `payment_month_offset` のような名前と、17 列すべてを並べても選べない。
 *
 * ## 差のある列だけを出す
 *
 * 同じ値の列を並べても選ぶ材料にならない。`updated_at` は必ず食い違う
 * （両方で直したのだから）ので、**入力欄として存在する列だけ**に絞る ——
 * リソース定義に無い `id` / `created_at` / `updated_at` は自動的に外れる。
 *
 * ## ラベルはリソース定義から引く
 *
 * 画面の入力欄と同じ日本語が出るようにする。ここに専用の対訳表を作ると、
 * 項目名を直したときに片方だけ古いままになる。列名は snake_case、
 * リソース定義は camelCase なので変換して引く。
 *
 * マスタ3表（作業分類・経費分類・原価レート）はリソース定義を持たない
 * （設定画面が直接扱う）ので、その場合は列名をそのまま出す。**出せないより
 * よい** —— 衝突するのは稀で、出たときに何の列かは分かる。
 */
import type { FormSchema } from '@banto/forms';
import * as m from '$lib/paraglide/messages';
import { customersSchema } from './resources/customers';
import { projectsSchema } from './resources/projects';
import { tripsSchema } from './resources/trips';
import { workLogsSchema } from './resources/work-logs';
import { expensesSchema } from './resources/expenses';
import type { SyncRow } from './syncAdmin';

/** 同期対象の表 → 入力欄の定義（持たない表は `undefined`）。 */
const SCHEMAS: Record<string, FormSchema | undefined> = {
	customers: customersSchema,
	projects: projectsSchema,
	trips: tripsSchema,
	work_logs: workLogsSchema,
	expenses: expensesSchema,
	work_categories: undefined,
	expense_categories: undefined,
	cost_rates: undefined
};

/** 表の見出し。ナビの項目名と同じ言葉にする。 */
export function tableLabel(table: string): string {
	switch (table) {
		case 'customers':
			return m['nav.customers']();
		case 'projects':
			return m['nav.projects']();
		case 'trips':
			return m['nav.trips']();
		case 'work_logs':
			return m['nav.workLogs']();
		case 'expenses':
			return m['nav.expenses']();
		case 'cost_rates':
			return m['nav.costRates']();
		default:
			return table;
	}
}

function toCamel(column: string): string {
	return column.replace(/_([a-z])/g, (_, letter: string) => letter.toUpperCase());
}

/** 列の見出し。入力欄と同じ言葉。引けなければ列名のまま。 */
export function columnLabel(table: string, column: string): string {
	const field = SCHEMAS[table]?.fields.find((entry) => entry.name === toCamel(column));
	return field?.label ?? column;
}

/**
 * その行が何なのかを一言で。`name` があればそれ、無ければ `code`、
 * どちらも無ければ主キー。
 *
 * 両側で名前まで変わっていることがあるので、**相手側の行**からも探す。
 */
export function rowTitle(mine: SyncRow, theirs: SyncRow): string {
	for (const row of [mine, theirs]) {
		for (const column of ['name', 'destination', 'code']) {
			const value = row.values[column];
			if (typeof value === 'string' && value !== '') return value;
		}
	}
	return mine.key;
}

export interface Difference {
	column: string;
	label: string;
	mine: string;
	theirs: string;
}

/** 表示用の値。null は空欄として出す（`null` と書くと値のように見える）。 */
function display(value: string | number | null): string {
	if (value === null) return '—';
	return String(value);
}

/**
 * 差のある列だけを、入力欄の並び順で返す。
 *
 * 入力欄に無い列（`id` / `created_at` / `updated_at`）は落とす。
 * リソース定義を持たない表では全列を見る（落とす基準が無いため）。
 */
export function differences(table: string, mine: SyncRow, theirs: SyncRow): Difference[] {
	const schema = SCHEMAS[table];
	const columns = schema
		? schema.fields.map((field) => field.name).map(toSnake)
		: Object.keys(mine.values).sort();

	return columns
		.filter((column) => mine.values[column] !== theirs.values[column])
		.map((column) => ({
			column,
			label: columnLabel(table, column),
			mine: display(mine.values[column] ?? null),
			theirs: display(theirs.values[column] ?? null)
		}));
}

function toSnake(field: string): string {
	return field.replace(/[A-Z]/g, (letter) => `_${letter.toLowerCase()}`);
}

/** 片方だけで削除されているか。差の一覧には出さず、別に強調する。 */
export function deletedOnOneSide(mine: SyncRow, theirs: SyncRow): 'mine' | 'theirs' | null {
	const mineGone = mine.values.deleted_at !== null && mine.values.deleted_at !== undefined;
	const theirsGone = theirs.values.deleted_at !== null && theirs.values.deleted_at !== undefined;
	if (mineGone === theirsGone) return null;
	return mineGone ? 'mine' : 'theirs';
}
