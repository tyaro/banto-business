/**
 * 参照フィールド（案件・顧客）の選択肢。
 *
 * ## なぜ要るのか
 *
 * 案件と顧客は `type: 'number'` で、**内部 id を手で打ち込む**作りだった。
 * PC では一覧を別に開いて番号を見れば済んでいたが、**スマホでは実質使えない**
 * —— 数値キーボードしか出ないので案件名も打てず、id を確かめるには
 * アプリを行き来するしかない。Phase 8 でスマホが操作の主体になる以上、
 * ここは名前で選べないと成立しない（実機で判明、`docs/domain/sync.md` 前提）。
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

/** 1度に読む上限。個人事業の規模では上限に当たらない想定。 */
const LIMIT = 500;

interface Referenceable {
	id: number;
	code: string;
	name: string;
}

/** `コード / 名前`。コードだけだと選べず、名前だけだと同名が区別できない。 */
function toOption(row: Referenceable): FieldOption {
	return { value: row.id, label: `${row.code} / ${row.name}` };
}

async function load(resource: string, sortField: string): Promise<FieldOption[]> {
	const params: ListParams = {
		// 新しいものほど上に来るようにする（直近の案件へ入力することが多い）。
		sort: [{ field: sortField, direction: 'desc' }],
		filters: [],
		pagination: { offset: 0, limit: LIMIT }
	};
	const result = await getDataProvider().getList<Referenceable>(resource, params);
	return result.rows.map(toOption);
}

let projects = $state<FieldOption[]>([]);
let customers = $state<FieldOption[]>([]);

/** 案件の選択肢（リソース定義の `get options()` から読む）。 */
export function projectOptions(): FieldOption[] {
	return projects;
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
		projects = await load('projects', 'code');
	} catch {
		projects = [];
	}
}

/** 顧客の選択肢を読み直す（同上）。 */
export async function loadCustomerOptions(): Promise<void> {
	try {
		customers = await load('customers', 'code');
	} catch {
		customers = [];
	}
}
