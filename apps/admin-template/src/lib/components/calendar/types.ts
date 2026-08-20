/**
 * 月カレンダーの表示契約（ドメイン非依存）。
 *
 * `MonthGrid.svelte` はこの型しか知らない。工数・経費・請求といった業務の
 * 語彙はここに出てこない —— ホスト側（`business/WorkCalendar.svelte`）が
 * 自分のデータをこの形へ翻訳して渡す。`@banto/*` へ切り出すときに
 * そのまま持って行けるようにするため（`docs/conventions.md` §5）。
 *
 * 文言はすべて**解決済みの文字列**で受け取る（i18n レイヤ①注入方式、
 * ADR-0005）。この層は辞書も Paraglide も参照しない。
 */

/** バッジの色味。意味づけはホストが決め、この層は theme 変数へ写すだけ。 */
export type CalendarTone = 'neutral' | 'accent' | 'success' | 'warning' | 'danger';

/** セル右上に並ぶ小さな印（経費・出張・請求期限・入金など）。 */
export interface CalendarBadge {
	/** `{#each}` のキー。セル内で一意。 */
	key: string;
	/** 表示する短い文字（件数や記号）。 */
	label: string;
	/** ツールチップ／読み上げ用の説明。 */
	title: string;
	tone: CalendarTone;
}

/** セル内に積む横棒（案件ごとの時間配分など）。 */
export interface CalendarBar {
	key: string;
	/** ツールチップ／読み上げ用。 */
	title: string;
	/** 0〜1。セル幅に対する比率。範囲外は描画側で丸める。 */
	ratio: number;
	/**
	 * 系列色の番号（0 始まり）。`--banto-chart-1..8` へ 8 色で循環させる。
	 * 具体的な色を渡さないのは、生の色値をコンポーネントに持ち込まない
	 * ため（conventions §9）。
	 */
	colorIndex: number;
}

/** 1 日ぶんの表示内容。すべて任意 —— 何も無い日は `undefined` を渡す。 */
export interface CalendarCell {
	/** セルの主役になる値（例: `3.5h`）。 */
	primary?: string;
	bars?: CalendarBar[];
	badges?: CalendarBadge[];
	/**
	 * 注意を引きたい日か（例: 工数の入力が無い過去の平日）。
	 * **判定はホスト側**が行う —— 「入力漏れ」の定義は業務ごとに違う。
	 */
	flagged?: boolean;
	/** `flagged` の説明（ツールチップ／読み上げ用）。 */
	flaggedTitle?: string;
}

/** `MonthGrid` が必要とする解決済み文言。 */
export interface CalendarMessages {
	/** 曜日見出し 7 語。**日曜=0 … 土曜=6 の順**で渡す（並べ替えは描画側）。 */
	weekdayNames: readonly string[];
	/** 「今日」の読み上げラベル。 */
	today: string;
	/** グリッド全体の読み上げラベル（例: 「2026年8月のカレンダー」）。 */
	gridLabel: string;
}
