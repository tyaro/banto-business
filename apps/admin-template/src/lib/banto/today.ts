/**
 * 「今日」の業務日付。
 *
 * **`toISOString()` は使えない。** あれは UTC を返すので、日本時間の朝9時
 * までが前日になる。業務日付はローカル日付（`CLAUDE.md` 4）なので、
 * ローカルの年月日をそのまま組み立てる。
 *
 * 呼ぶ場所を絞るために切り出してある —— 画面ごとに `new Date()` を書くと、
 * 日付をまたぐ瞬間に画面内で食い違う。1つの画面では**一度だけ**呼んで
 * 下へ渡すこと。
 */
export function localToday(): string {
	const now = new Date();
	const year = now.getFullYear();
	const month = String(now.getMonth() + 1).padStart(2, '0');
	const day = String(now.getDate()).padStart(2, '0');
	return `${year}-${month}-${day}`;
}
