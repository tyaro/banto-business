/**
 * 参照フィールド（案件・顧客・作業分類・経費分類）の選択肢。
 *
 * ## なぜ要るのか
 *
 * 案件と顧客は `type: 'number'` で、**内部 id を手で打ち込む**作りだった。
 * PC では一覧を別に開いて番号を見れば済んでいたが、**スマホでは実質使えない**
 * —— 数値キーボードしか出ないので案件名も打てず、id を確かめるには
 * アプリを行き来するしかない。Phase 8 でスマホが操作の主体になる以上、
 * ここは名前で選べないと成立しない（実機で判明、`docs/domain/sync.md` 前提）。
 *
 * ## 見出しに内部番号を出さない
 *
 * 顧客コードも案件番号も**自動採番される内部の識別子**（決定 C-9）。選ぶ
 * ときに読む意味が無く、見出しに出すと「毎回コードを気にする作り」に見えて
 * しまう。名前だけを出す。
 *
 * ## なぜ「選択肢を注入する」形にするのか
 *
 * リソース定義（`resources/*.ts`）は静的なモジュールで、`get options()` で
 * 選択肢を返す仕組みが既にある（税区分・案件状態がそれ）。そこへ**読み込み
 * 済みの配列を返すだけ**にしておけば、`@banto/forms` 側に手を入れずに済む
 * （`CLAUDE.md` 第3章：同梱した Banto のコードを Business 都合で書き換えない）。
 *
 * `SelectField` は `def.options` から**値の型をそのまま**返すので
 * （`option.value` を引いて渡す実装）、`value: number` を入れておけば
 * フォームには数値が入り、`normalizeFormValues` に手を入れる必要も無い。
 *
 * ## 読み込みの契機
 *
 * フォーム画面が開くたびに読み直す。起動時に1度だけにすると、案件を作った
 * 直後の工数入力でその案件が出てこない。件数は個人事業の規模なので、
 * 画面を開くたびの1リクエストは問題にならない。
 *
 * 失敗しても画面は壊さない（選択肢が空のまま）。ここで例外を投げると、
 * 選択肢の取得に失敗しただけでフォーム全体が出なくなる。
 */
import { getDataProvider, type ListParams } from '@banto/admin-core';
import type { FieldOption } from '@banto/forms';
import * as m from '$lib/paraglide/messages';

/** 1度に読む上限。個人事業の規模では上限に当たらない想定。 */
const LIMIT = 500;

interface Referenceable {
	id: number;
	code: string;
	name: string;
}

interface ProjectRow extends Referenceable {
	customerId: number;
}

/**
 * 選択肢の見出し。
 *
 * **内部番号（顧客コード・案件番号）は出さない。** どちらも自動採番される
 * 内部の識別子で、選ぶときに読む意味が無い —— 出すと「毎回コードを気にする
 * 作り」に見えてしまう。
 *
 * 案件は `顧客名 / 案件名`。案件名だけだと、どの顧客の案件か分からない
 * （「保守」「定期点検」のような名前は顧客をまたいで重なる）。顧客名を
 * 引けなかった場合は案件名だけにする —— 見出しのために選択肢を落とさない。
 */
function customerOption(row: Referenceable): FieldOption {
	return { value: row.id, label: row.name };
}

function projectOption(row: ProjectRow, customerNames: Map<number, string>): FieldOption {
	const customer = customerNames.get(row.customerId);
	return { value: row.id, label: customer ? `${customer} / ${row.name}` : row.name };
}

async function fetchRows<T>(resource: string, direction: 'asc' | 'desc'): Promise<T[]> {
	const params: ListParams = {
		sort: [{ field: 'code', direction }],
		filters: [],
		pagination: { offset: 0, limit: LIMIT }
	};
	const result = await getDataProvider().getList<T>(resource, params);
	return result.rows;
}

let projects = $state<FieldOption[]>([]);
let customers = $state<FieldOption[]>([]);

/** 案件の選択肢（リソース定義の `get options()` から読む）。 */
export function projectOptions(): FieldOption[] {
	return projects;
}

/**
 * 案件 id → 見出し。一覧の `案件` 列を数字ではなく名前で描くために使う。
 *
 * `columnsFromSchema` も select 列を選択肢のラベルで描く仕組みを持っているが、
 * **選択肢を組み立てる時点（＝画面の生成時）に確定した値**を使う。選択肢は
 * 後から非同期に届くので、そちらには載らない。この関数は**セルを描くたび**に
 * 呼ばれる `format` から読むので、届いた時点で表示が入れ替わる。
 *
 * 見つからないときは数字のまま返す —— 消してしまうと、選択肢の読み込みに
 * 失敗したときに「案件が空欄の行」に見えてしまう。
 */
export function projectLabel(value: unknown): string {
	const id = Number(value);
	const found = projects.find((option) => option.value === id);
	return found ? found.label : String(value ?? '');
}

/** 顧客の選択肢（同上）。 */
export function customerOptions(): FieldOption[] {
	return customers;
}

/**
 * 案件の選択肢を読み直す。フォーム画面の `$effect` から呼ぶ。
 *
 * 失敗は握りつぶす（選択肢が空になるだけ）。詳細は上の doc を参照。
 */
export async function loadProjectOptions(): Promise<void> {
	try {
		// 顧客名を見出しに使うので顧客も引く。件数は個人事業の規模なので、
		// 1回の呼び出しが2リクエストになること自体は問題にならない。
		const [rows, customerRows] = await Promise.all([
			// 新しい案件ほど上に来るようにする（直近の案件へ入力することが多い）。
			fetchRows<ProjectRow>('projects', 'desc'),
			fetchRows<Referenceable>('customers', 'asc')
		]);
		const names = new Map(customerRows.map((row) => [row.id, row.name]));
		projects = rows.map((row) => projectOption(row, names));
	} catch {
		projects = [];
	}
}

/** 顧客の選択肢を読み直す（同上）。 */
export async function loadCustomerOptions(): Promise<void> {
	try {
		// 顧客はコード順（＝登録順）。案件と違い「直近」に寄せる理由が無い。
		customers = (await fetchRows<Referenceable>('customers', 'asc')).map(customerOption);
	} catch {
		customers = [];
	}
}

/**
 * 作業分類・経費分類の選択肢。
 *
 * 案件・顧客と事情は同じ —— `type: 'text'` のままだと**分類コードを手打ち**
 * することになり、コード表を覚えていないと入力できない（スマホでは致命的）。
 * 名前で選ばせる。値はコード（TEXT 主キー）そのまま。
 *
 * ## 停止中の分類も選択肢に残す
 *
 * 有効（`active`）だけに絞ると、停止済み分類が付いた**過去の行を編集した
 * ときに選択が空に見え、保存すると別の値に化ける**。サーバ側も削除済み以外は
 * 受ける（`work_logs.rs` は active を見ない）ので、こちらも全件出し、停止中
 * だけ末尾に印を付けて区別する。新規で停止中を選べてしまうのは許容する ——
 * 印が付いていて気付けるし、個人事業の規模で分類を停止すること自体が稀。
 */
interface CategoryRow {
	code: string;
	name: string;
	active: number;
}

/** 経費分類のみが持つ列（P2-2、`docs/mobile-ui-plan.md`）。 */
interface ExpenseCategoryRow extends CategoryRow {
	defaultTaxCategory: string;
}

function categoryOption(row: CategoryRow): FieldOption {
	return {
		value: row.code,
		label: row.active === 0 ? m['categories.inactiveOption']({ name: row.name }) : row.name
	};
}

async function fetchCategoryRows<T extends CategoryRow>(resource: string): Promise<T[]> {
	const params: ListParams = {
		sort: [{ field: 'sort_order', direction: 'asc' }],
		filters: [],
		pagination: { offset: 0, limit: LIMIT }
	};
	const result = await getDataProvider().getList<T>(resource, params);
	return result.rows;
}

let workCategories = $state<FieldOption[]>([]);
let expenseCategories = $state<FieldOption[]>([]);

/** 作業分類の選択肢（リソース定義の `get options()` から読む）。 */
export function workCategoryOptions(): FieldOption[] {
	return workCategories;
}

/**
 * 作業分類コード → 名前。一覧の `作業分類` 列をコードではなく名前で描く。
 * `projectLabel` と同じ理屈（セルを描くたびに呼ばれる `format` から読む）。
 */
export function workCategoryLabel(value: unknown): string {
	const found = workCategories.find((option) => option.value === value);
	return found ? found.label : String(value ?? '');
}

/** 作業分類の選択肢を読み直す。失敗は握りつぶす（`loadProjectOptions` と同じ）。 */
export async function loadWorkCategoryOptions(): Promise<void> {
	try {
		workCategories = (await fetchCategoryRows<CategoryRow>('work_categories')).map(categoryOption);
	} catch {
		workCategories = [];
	}
}

/** 経費分類の選択肢（同上）。 */
export function expenseCategoryOptions(): FieldOption[] {
	return expenseCategories;
}

/** 経費分類コード → 名前（同上）。 */
export function expenseCategoryLabel(value: unknown): string {
	const found = expenseCategories.find((option) => option.value === value);
	return found ? found.label : String(value ?? '');
}

/**
 * 経費分類コード → 既定の税区分（P2-2、`docs/mobile-ui-plan.md`）。
 *
 * サーバ側（`core/src/expenses.rs`）は `taxCategory` が空なら
 * `expense_categories.default_tax_category` を使う実装を既に持つ —
 * これはその値をフォーム側にも見せるための読み取り専用の写し。
 * 未読み込み・未知のコードは `null`（呼び出し側は「何もしない」扱いにする）。
 */
const expenseCategoryDefaultTax = new Map<string, string>();

export function expenseCategoryDefaultTaxOf(code: string): string | null {
	return expenseCategoryDefaultTax.get(code) ?? null;
}

/** 経費分類の選択肢を読み直す（同上）。既定税区分の写しも同時に更新する。 */
export async function loadExpenseCategoryOptions(): Promise<void> {
	try {
		const rows = await fetchCategoryRows<ExpenseCategoryRow>('expense_categories');
		expenseCategories = rows.map(categoryOption);
		expenseCategoryDefaultTax.clear();
		for (const row of rows) {
			expenseCategoryDefaultTax.set(row.code, row.defaultTaxCategory);
		}
	} catch {
		expenseCategories = [];
		expenseCategoryDefaultTax.clear();
	}
}
