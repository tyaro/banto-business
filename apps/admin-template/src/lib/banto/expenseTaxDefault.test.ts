import { describe, expect, it } from 'vitest';
import { ExpenseTaxCategoryTracker } from './expenseTaxDefault';

const defaults: Record<string, string> = {
	TRANSPORT: 'STANDARD_10',
	ENTERTAINMENT: 'STANDARD_10'
};
const lookup = (code: string): string | null => defaults[code] ?? null;

describe('ExpenseTaxCategoryTracker', () => {
	it('分類変更で税区分が既定に変わる', () => {
		const tracker = new ExpenseTaxCategoryTracker('');
		expect(tracker.sync('TRANSPORT', lookup)).toBe('STANDARD_10');
	});

	it('同じ分類の再設定では触らない', () => {
		const tracker = new ExpenseTaxCategoryTracker('TRANSPORT');
		// 初回は「変わっていない」ので null。
		expect(tracker.sync('TRANSPORT', lookup)).toBeNull();
		// 一度変わった後、同じ値を渡し直しても再び発火しない。
		const other = new ExpenseTaxCategoryTracker('');
		expect(other.sync('TRANSPORT', lookup)).toBe('STANDARD_10');
		expect(other.sync('TRANSPORT', lookup)).toBeNull();
	});

	it('未知の分類では触らない', () => {
		const tracker = new ExpenseTaxCategoryTracker('');
		expect(tracker.sync('NOPE', lookup)).toBeNull();
	});

	it('初期コードから変化していなければ触らない（編集画面のロード直後）', () => {
		// [id] ページの想定: ロードした行の分類コードを初期値として渡す。
		const tracker = new ExpenseTaxCategoryTracker('TRANSPORT');
		expect(tracker.sync('TRANSPORT', lookup)).toBeNull();
	});

	it('分類を変えるたびに毎回上書きする（手動選択後でも）', () => {
		const tracker = new ExpenseTaxCategoryTracker('');
		expect(tracker.sync('TRANSPORT', lookup)).toBe('STANDARD_10');
		// TRANSPORT のままなら再発火しない。
		expect(tracker.sync('TRANSPORT', lookup)).toBeNull();
		// 別の分類に変えると、たとえユーザーが taxCategory を手動で変えていても
		// また既定値を返す（呼び出し側が setValue で上書きする）。
		expect(tracker.sync('ENTERTAINMENT', lookup)).toBe('STANDARD_10');
	});
});
