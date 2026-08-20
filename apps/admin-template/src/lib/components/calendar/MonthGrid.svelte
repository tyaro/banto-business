<script lang="ts">
	/**
	 * 月グリッドの描画（ドメイン非依存）。
	 *
	 * この層が知らないもの:
	 * - 業務データ（工数・経費・請求）→ `CalendarCell` に翻訳済みで受け取る
	 * - 文言・ロケール → 解決済み文字列を `messages` で受け取る（ADR-0005 レイヤ①）
	 * - 今日が何日か → `today` prop で受け取る。`new Date()` を呼ばない
	 * - 色 → `--banto-chart-*` などの theme 変数のみ（conventions §9）
	 *
	 * `$lib` import を持たないのも意図的で、`@banto/*` へ切り出すときに
	 * そのまま動くようにしてある（conventions §5）。昇格するかは未決 ——
	 * `docs/banto-feedback.md` を参照。
	 */
	import { monthGrid, weekdayOrder, type WeekStart } from './month';
	import type { CalendarCell, CalendarMessages } from './types';

	interface Props {
		/** `YYYY-MM`。 */
		month: string;
		/** 日付（`YYYY-MM-DD`）→ その日の表示内容。無い日は升目だけ描く。 */
		cells: Record<string, CalendarCell>;
		messages: CalendarMessages;
		/** 週の開始曜日。既定は月曜（日本の業務暦の通例）。 */
		weekStart?: WeekStart;
		/** 今日の `YYYY-MM-DD`。渡さなければ「今日」の枠を出さない。 */
		today?: string | null;
		selected?: string | null;
		onselect?: (date: string) => void;
	}

	let {
		month,
		cells,
		messages,
		weekStart = 1,
		today = null,
		selected = null,
		onselect
	}: Props = $props();

	const weeks = $derived(monthGrid(month, weekStart));
	const headers = $derived(weekdayOrder(weekStart).map((day) => messages.weekdayNames[day] ?? ''));

	/** 表示中の月の日か（前後の月の埋め草は薄く出す）。 */
	const inMonth = (date: string): boolean => date.startsWith(month);

	/** 土日か。祝日は扱わない（祝日表は毎年変わるので持たない）。 */
	const weekendColumn = $derived(weekdayOrder(weekStart).map((day) => day === 0 || day === 6));

	// 8 色で循環させる（`--banto-chart-1..8`）。色そのものは theme が持つ。
	const CHART_COLORS = 8;
	const barColor = (colorIndex: number): string =>
		`var(--banto-chart-${(colorIndex % CHART_COLORS) + 1})`;

	const clampRatio = (ratio: number): number => Math.max(0, Math.min(1, ratio));

	/** その日の升目に出す数字（`2026-08-03` → `3`）。 */
	const dayNumber = (date: string): string => String(Number(date.slice(8, 10)));
</script>

<div class="grid" role="grid" aria-label={messages.gridLabel}>
	<div class="row row--head" role="row">
		{#each headers as header, column (column)}
			<div class="head" class:head--weekend={weekendColumn[column]} role="columnheader">
				{header}
			</div>
		{/each}
	</div>

	{#each weeks as week (week[0])}
		<div class="row" role="row">
			{#each week as date, column (date)}
				{@const cell = cells[date]}
				{@const isToday = today === date}
				<button
					type="button"
					class="cell"
					class:cell--outside={!inMonth(date)}
					class:cell--weekend={weekendColumn[column]}
					class:cell--today={isToday}
					class:cell--selected={selected === date}
					class:cell--flagged={cell?.flagged}
					title={cell?.flagged ? cell.flaggedTitle : undefined}
					role="gridcell"
					aria-selected={selected === date}
					onclick={() => onselect?.(date)}
				>
					<span class="daynum">
						{dayNumber(date)}
						{#if isToday}<span class="sr-only">{messages.today}</span>{/if}
					</span>

					{#if cell?.primary}
						<span class="primary">{cell.primary}</span>
					{/if}

					{#if cell?.bars?.length}
						<span class="bars">
							{#each cell.bars as bar (bar.key)}
								<span
									class="bar"
									title={bar.title}
									style:flex-grow={clampRatio(bar.ratio)}
									style:background={barColor(bar.colorIndex)}
								></span>
							{/each}
						</span>
					{/if}

					{#if cell?.badges?.length}
						<span class="badges">
							{#each cell.badges as badge (badge.key)}
								<span class="badge badge--{badge.tone}" title={badge.title}>{badge.label}</span>
							{/each}
						</span>
					{/if}
				</button>
			{/each}
		</div>
	{/each}
</div>

<style>
	.grid {
		display: flex;
		flex-direction: column;
		gap: 2px;
	}

	.row {
		display: grid;
		grid-template-columns: repeat(7, minmax(0, 1fr));
		gap: 2px;
	}

	.head {
		padding: 0.25rem 0.375rem;
		font-size: 0.75rem;
		font-weight: 600;
		color: var(--banto-text-muted);
		text-align: center;
	}

	.head--weekend {
		color: var(--banto-danger);
	}

	.cell {
		display: flex;
		flex-direction: column;
		align-items: stretch;
		gap: 0.25rem;
		/* 升目の高さを揃えないと、内容の多い日だけ行が伸びて暦に見えなくなる。 */
		min-height: 5.5rem;
		padding: 0.3rem 0.375rem;
		border: 1px solid var(--banto-border);
		border-radius: var(--banto-radius-sm);
		background: var(--banto-surface);
		color: inherit;
		font: inherit;
		text-align: left;
		cursor: pointer;
		transition: background var(--banto-duration-fast) var(--banto-ease-out);
	}

	.cell:hover {
		background: var(--banto-surface-hover);
	}

	.cell:focus-visible {
		outline: var(--banto-focus-ring);
		outline-offset: -1px;
	}

	/* 前後の月の埋め草。消さずに薄くするのは、曜日の並びを保つため。 */
	.cell--outside {
		background: var(--banto-bg);
		color: var(--banto-text-muted);
	}

	.cell--weekend {
		background: var(--banto-surface-subtle);
	}

	.cell--weekend.cell--outside {
		background: var(--banto-bg);
	}

	.cell--today {
		border-color: var(--banto-primary);
		box-shadow: inset 0 0 0 1px var(--banto-primary);
	}

	.cell--selected {
		border-color: var(--banto-primary);
		background: var(--banto-surface-raised);
	}

	/* 注意を引きたい日（例: 工数の入力が無い過去の平日）。 */
	.cell--flagged {
		background: var(--banto-warning-tint);
	}

	.daynum {
		font-size: 0.8rem;
		font-variant-numeric: tabular-nums;
	}

	.primary {
		font-size: 0.95rem;
		font-weight: 600;
		font-variant-numeric: tabular-nums;
	}

	.bars {
		display: flex;
		gap: 1px;
		height: 4px;
		border-radius: 2px;
		overflow: hidden;
	}

	.bar {
		display: block;
		flex-basis: 0;
		min-width: 2px;
	}

	.badges {
		display: flex;
		flex-wrap: wrap;
		gap: 2px;
		margin-top: auto;
	}

	.badge {
		border-radius: var(--banto-radius-sm);
		padding: 0 0.25rem;
		font-size: 0.7rem;
		line-height: 1.4;
		font-variant-numeric: tabular-nums;
	}

	.badge--neutral {
		background: var(--banto-surface-subtle);
		color: var(--banto-text-muted);
	}

	.badge--accent {
		background: var(--banto-primary);
		color: var(--banto-on-solid);
	}

	.badge--success {
		background: var(--banto-success-tint);
		color: var(--banto-success-tint-text);
	}

	.badge--warning {
		background: var(--banto-warning-tint);
		color: var(--banto-warning-tint-text);
	}

	.badge--danger {
		background: var(--banto-danger-tint);
		color: var(--banto-danger-tint-text);
	}

	.sr-only {
		position: absolute;
		width: 1px;
		height: 1px;
		padding: 0;
		margin: -1px;
		overflow: hidden;
		clip-path: inset(50%);
		white-space: nowrap;
	}
</style>
