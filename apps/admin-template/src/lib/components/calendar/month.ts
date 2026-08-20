/**
 * 月グリッドの日付計算（純粋関数のみ）。
 *
 * **ドメインを知らない層。** ここと `MonthGrid.svelte` は業務データを一切
 * 参照せず、`@banto/*` へ切り出せる形を保つ（`docs/conventions.md` §5 の
 * 「パッケージはアプリ固有 import を持たない」を、まだ app 層にいる段階から
 * 自主的に守る）。昇格の判断は後で行う — `docs/banto-feedback.md` 参照。
 *
 * ## 「今日」を自分で取らない
 *
 * この層は `Date.now()` / `new Date()`（引数なし）を呼ばない。今日の日付は
 * 常に呼び出し側から `YYYY-MM-DD` で受け取る。理由は2つ:
 *
 * 1. サーバ側（`calendar.rs`）も業務日付を持たない設計で、境界の考え方を
 *    揃えたい。
 * 2. 実行時刻に依存するとテストが日付で落ちる。
 *
 * ## タイムゾーン
 *
 * 日付は業務日付（時刻の意味を持たない `YYYY-MM-DD`。CLAUDE.md 4）なので、
 * 計算はすべて **UTC で構築し UTC で読む**（`Date.UTC` / `getUTCDay`）。
 * ローカル時刻の `new Date('2026-08-01')` 系を混ぜると、UTC より西の
 * タイムゾーンで1日ずれる。
 */

/** 日曜=0 … 土曜=6。`Date` の `getUTCDay()` と同じ番号。 */
export type Weekday = 0 | 1 | 2 | 3 | 4 | 5 | 6;

/** 週の開始曜日。日本の業務暦は月曜始まりが通例なので既定は 1（月曜）。 */
export type WeekStart = 0 | 1;

const ISO_DATE = /^(\d{4})-(\d{2})-(\d{2})$/;
const ISO_MONTH = /^(\d{4})-(\d{2})$/;

/** `YYYY-MM-DD` を UTC のミリ秒へ。読めなければ `null`。 */
function toUtcMillis(date: string): number | null {
	const match = ISO_DATE.exec(date);
	if (!match) return null;
	const [, year, month, day] = match;
	const millis = Date.UTC(Number(year), Number(month) - 1, Number(day));
	if (Number.isNaN(millis)) return null;
	// `Date.UTC(2026, 1, 30)` は 3/2 に繰り上がってしまうので、往復させて
	// 元の文字列と一致するかで存在しない日付を弾く（2月30日など）。
	return toIsoDate(millis) === date ? millis : null;
}

/** UTC ミリ秒を `YYYY-MM-DD` へ。 */
function toIsoDate(millis: number): string {
	return new Date(millis).toISOString().slice(0, 10);
}

const DAY_MS = 24 * 60 * 60 * 1000;

/** その日の曜日。読めない日付は `null`。 */
export function weekdayOf(date: string): Weekday | null {
	const millis = toUtcMillis(date);
	if (millis === null) return null;
	return new Date(millis).getUTCDay() as Weekday;
}

/** 土日か。祝日は扱わない（祝日表は毎年変わるので持たない）。 */
export function isWeekend(date: string): boolean {
	const weekday = weekdayOf(date);
	return weekday === 0 || weekday === 6;
}

/** `YYYY-MM` を月初の `YYYY-MM-DD` へ。読めなければ `null`。 */
export function firstDayOfMonth(month: string): string | null {
	const match = ISO_MONTH.exec(month);
	if (!match) return null;
	const monthNumber = Number(match[2]);
	if (monthNumber < 1 || monthNumber > 12) return null;
	return `${match[1]}-${match[2]}-01`;
}

/** 月を `offset` か月ずらした `YYYY-MM`。読めなければ `null`。 */
export function shiftMonth(month: string, offset: number): string | null {
	const match = ISO_MONTH.exec(month);
	if (!match) return null;
	const year = Number(match[1]);
	const monthIndex = Number(match[2]) - 1;
	if (monthIndex < 0 || monthIndex > 11) return null;
	const total = year * 12 + monthIndex + offset;
	const shiftedYear = Math.floor(total / 12);
	const shiftedMonth = total - shiftedYear * 12 + 1;
	if (shiftedYear < 1 || shiftedYear > 9999) return null;
	return `${String(shiftedYear).padStart(4, '0')}-${String(shiftedMonth).padStart(2, '0')}`;
}

/** その日が属する月（`YYYY-MM`）。 */
export function monthOf(date: string): string | null {
	return toUtcMillis(date) === null ? null : date.slice(0, 7);
}

/**
 * 月グリッドの升目。週の配列（各週 7 日）を返す。
 *
 * 前後の月の日で端を埋める（グリッドは常に 7 の倍数）。埋めないと月の
 * 初週・最終週だけ列がずれて、曜日が縦に揃わなくなる。
 *
 * 週数は月によって 4〜6 週になる（2月がちょうど 28 日かつ週初に始まると
 * 4 週）。固定 6 週にはしない — 使わない行が常に 1〜2 行ぶら下がる。
 */
export function monthGrid(month: string, weekStart: WeekStart = 1): string[][] {
	const first = firstDayOfMonth(month);
	if (first === null) return [];
	const firstMillis = toUtcMillis(first);
	if (firstMillis === null) return [];

	const next = shiftMonth(month, 1);
	const nextFirst = next === null ? null : firstDayOfMonth(next);
	const nextMillis = nextFirst === null ? null : toUtcMillis(nextFirst);
	if (nextMillis === null) return [];

	// 月初を含む週の先頭まで戻る。
	const firstWeekday = new Date(firstMillis).getUTCDay();
	const lead = (firstWeekday - weekStart + 7) % 7;
	let cursor = firstMillis - lead * DAY_MS;

	const weeks: string[][] = [];
	// 月末を含む週の末尾まで進む。`cursor < nextMillis` で月内の日が残って
	// いる限り週を足し、最後の週だけ翌月の日で埋まる。
	while (cursor < nextMillis) {
		const week: string[] = [];
		for (let i = 0; i < 7; i += 1) {
			week.push(toIsoDate(cursor));
			cursor += DAY_MS;
		}
		weeks.push(week);
	}
	return weeks;
}

/**
 * グリッドの曜日見出しの並び。`weekStart` から 7 日ぶんの曜日番号を返す。
 * 見出しの文字そのものは呼び出し側が渡す（この層は文言を持たない）。
 */
export function weekdayOrder(weekStart: WeekStart = 1): Weekday[] {
	return Array.from({ length: 7 }, (_, i) => ((weekStart + i) % 7) as Weekday);
}

/** 分を `3.5h` のような時間表記へ。丸めは小数第1位で四捨五入。 */
export function formatHours(minutes: number): string {
	// 金額ではないので小数でよい（CLAUDE.md 1.1 は金額の規約）。
	// 分を10倍して整数で丸め、桁を戻す（浮動小数の誤差を表示に出さない）。
	const tenths = Math.round((minutes * 10) / 60);
	const whole = Math.trunc(tenths / 10);
	const fraction = Math.abs(tenths % 10);
	return fraction === 0 ? `${whole}h` : `${whole}.${fraction}h`;
}
