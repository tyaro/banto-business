import { describe, expect, it, vi } from 'vitest';
import {
	firstDayOfMonth,
	formatHours,
	isWeekend,
	monthGrid,
	monthOf,
	shiftMonth,
	weekdayOf,
	weekdayOrder
} from './month';

describe('weekdayOf', () => {
	it('numbers days the same way Date.getUTCDay does', () => {
		// 2026-08-20 は木曜。
		expect(weekdayOf('2026-08-20')).toBe(4);
		expect(weekdayOf('2026-08-23')).toBe(0); // 日曜
		expect(weekdayOf('2026-08-22')).toBe(6); // 土曜
	});

	it('rejects malformed and non-existent dates', () => {
		for (const bad of ['2026-02-30', '2026-13-01', '2026-8-1', '20260801', '', 'nope']) {
			expect(weekdayOf(bad), bad).toBeNull();
		}
	});

	/**
	 * 業務日付は時刻を持たないので、実行環境のタイムゾーンが変わっても同じ
	 * 曜日にならなければならない。`new Date('2026-08-20')` のようなローカル
	 * 時刻での解釈に戻すと、UTC より西では1日ずれてこのテストが落ちる。
	 */
	it('does not shift by timezone', () => {
		for (const timezone of ['America/Los_Angeles', 'Pacific/Kiritimati', 'Asia/Tokyo']) {
			vi.stubEnv('TZ', timezone);
			expect(weekdayOf('2026-08-20'), timezone).toBe(4);
			expect(weekdayOf('2026-08-23'), timezone).toBe(0);
		}
		vi.unstubAllEnvs();
	});
});

describe('isWeekend', () => {
	it('is true for saturday and sunday only', () => {
		expect(isWeekend('2026-08-22')).toBe(true); // 土
		expect(isWeekend('2026-08-23')).toBe(true); // 日
		expect(isWeekend('2026-08-21')).toBe(false); // 金
		expect(isWeekend('2026-08-24')).toBe(false); // 月
	});
});

describe('firstDayOfMonth / monthOf', () => {
	it('opens a month and reads a date back to its month', () => {
		expect(firstDayOfMonth('2026-08')).toBe('2026-08-01');
		expect(monthOf('2026-08-31')).toBe('2026-08');
	});

	it('rejects malformed input', () => {
		for (const bad of ['2026-13', '2026-00', '2026-8', '2026', '']) {
			expect(firstDayOfMonth(bad), bad).toBeNull();
		}
		expect(monthOf('2026-02-30')).toBeNull();
	});
});

describe('shiftMonth', () => {
	it('moves within a year', () => {
		expect(shiftMonth('2026-08', 1)).toBe('2026-09');
		expect(shiftMonth('2026-08', -1)).toBe('2026-07');
	});

	it('crosses year boundaries in both directions', () => {
		expect(shiftMonth('2026-12', 1)).toBe('2027-01');
		expect(shiftMonth('2026-01', -1)).toBe('2025-12');
		expect(shiftMonth('2026-01', -13)).toBe('2024-12');
		expect(shiftMonth('2026-12', 13)).toBe('2028-01');
	});

	it('rejects malformed input', () => {
		expect(shiftMonth('2026-13', 1)).toBeNull();
		expect(shiftMonth('nope', 1)).toBeNull();
	});
});

describe('weekdayOrder', () => {
	it('starts on monday by default and on sunday when asked', () => {
		expect(weekdayOrder()).toEqual([1, 2, 3, 4, 5, 6, 0]);
		expect(weekdayOrder(0)).toEqual([0, 1, 2, 3, 4, 5, 6]);
	});
});

describe('monthGrid', () => {
	it('always returns whole weeks', () => {
		for (const month of ['2026-01', '2026-02', '2026-08', '2028-02', '2027-05']) {
			for (const week of monthGrid(month)) {
				expect(week, month).toHaveLength(7);
			}
		}
	});

	/** 月初を含む週の先頭から始まり、月末を含む週の末尾で終わる。 */
	it('pads the edges with days from the adjacent months', () => {
		// 2026-08-01 は土曜なので、月曜始まりだと 7/27(月) から始まる。
		const weeks = monthGrid('2026-08', 1);
		expect(weeks[0][0]).toBe('2026-07-27');
		expect(weeks[0][6]).toBe('2026-08-02');
		const lastWeek = weeks[weeks.length - 1];
		expect(lastWeek).toContain('2026-08-31');
		expect(lastWeek[6]).toBe('2026-09-06');
	});

	it('honours a sunday week start', () => {
		const weeks = monthGrid('2026-08', 0);
		expect(weeks[0][0]).toBe('2026-07-26');
		expect(weeks[0][6]).toBe('2026-08-01');
	});

	it('covers every day of the month exactly once', () => {
		for (const [month, days] of [
			['2026-08', 31],
			['2026-02', 28],
			['2028-02', 29],
			['2026-04', 30]
		] as const) {
			const inMonth = monthGrid(month)
				.flat()
				.filter((date) => date.startsWith(month));
			expect(new Set(inMonth).size, month).toBe(days);
			expect(inMonth.length, month).toBe(days);
		}
	});

	/**
	 * 週数は固定しない。ちょうど 4 週に収まる月（週初に始まる平年の2月）は
	 * 4 行、はみ出す月は 5〜6 行。
	 */
	it('uses as many weeks as the month actually needs', () => {
		// 2027-02-01 は月曜、平年の2月なので 28 日ちょうど = 4 週。
		expect(monthGrid('2027-02', 1)).toHaveLength(4);
		expect(monthGrid('2026-08', 1)).toHaveLength(6);
		expect(monthGrid('2026-09', 1)).toHaveLength(5);
	});

	it('returns nothing for a malformed month', () => {
		expect(monthGrid('2026-13')).toEqual([]);
		expect(monthGrid('nope')).toEqual([]);
	});
});

describe('formatHours', () => {
	it('renders whole and fractional hours', () => {
		expect(formatHours(0)).toBe('0h');
		expect(formatHours(60)).toBe('1h');
		expect(formatHours(90)).toBe('1.5h');
		expect(formatHours(210)).toBe('3.5h');
		expect(formatHours(30)).toBe('0.5h');
	});

	it('rounds to one decimal place', () => {
		expect(formatHours(65)).toBe('1.1h'); // 1.0833… → 1.1
		expect(formatHours(61)).toBe('1h'); // 1.0166… → 1.0
		expect(formatHours(59)).toBe('1h'); // 0.9833… → 1.0
	});
});
