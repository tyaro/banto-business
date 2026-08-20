<script lang="ts">
	/**
	 * 月カレンダー（Phase 7 準備）。
	 *
	 * `components/calendar/` の汎用グリッドに、業務データを翻訳して渡す層。
	 * **業務の語彙はここまで**で、グリッド側は工数も請求も知らない
	 * （`calendar/types.ts` 参照）。
	 *
	 * ## 何を強調するか
	 *
	 * 「工数の入力が無い**過去の平日**」だけを強調する（`flagged`）。
	 * 未来の平日は入力が無くて当たり前なので対象にしない。土日も外す ——
	 * 祝日は扱わない（祝日表は毎年変わるので持たない。休日に働いた日は
	 * 工数が入るので見えるし、逆は強調しないだけで済む）。
	 *
	 * ## 期限超過の色分けはしない
	 *
	 * `CLAUDE.md` 1.5 のとおり Overdue は導出値だが、カレンダーは過去も未来も
	 * 同じ面に描くので「その日が期限」以上の意味を升目に持ち込まない
	 * （`calendar.rs` の同名の節と同じ理由）。超過の一覧はダッシュボードの
	 * 未入金パネルが持つ。
	 */
	import { untrack } from 'svelte';
	import { getDataProvider } from '@banto/admin-core';
	import type { ListParams } from '@banto/admin-core';
	import { getBantoMode } from '$lib/banto/setup';
	import { base } from '$app/paths';
	import * as m from '$lib/paraglide/messages';
	import LoadingState from '$lib/components/ui/LoadingState.svelte';
	import ErrorState from '$lib/components/ui/ErrorState.svelte';
	import MonthGrid from '$lib/components/calendar/MonthGrid.svelte';
	import type { CalendarBadge, CalendarBar, CalendarCell } from '$lib/components/calendar/types';
	import { formatHours, isWeekend, monthOf, shiftMonth } from '$lib/components/calendar/month';

	/** `calendar.rs::CalendarDay` の写し。 */
	interface CalendarDay {
		id: string;
		date: string;
		workedMinutes: number;
		workLogCount: number;
		projects: { projectId: number; projectCode: string; projectName: string; minutes: number }[];
		expenseCount: number;
		expenseAmount: number;
		tripCount: number;
		invoiceClosingCount: number;
		invoiceDueCount: number;
		invoiceDueRemaining: number;
		paymentCount: number;
		paymentAmount: number;
	}

	interface Props {
		/** 今日の `YYYY-MM-DD`。ページ側が一度だけ決めて渡す。 */
		today: string;
	}

	let { today }: Props = $props();

	// 単体ブラウザのデモモードには業務DBが無いので引きに行かない
	// （`OutstandingPanel` と同じ扱い）。
	const available = getBantoMode() !== 'demo';

	// 表示中の月。初期値は「今日の月」だが、その後はユーザーが送った月に
	// 従う。`untrack` で包むのは、`today` を読んだことで初期化式が
	// リアクティブに見えてしまうのを避けるため（`today` は呼び出し側が
	// 一度だけ決める値で、日付をまたいでも画面内で勝手に戻したくない）。
	let month = $state(untrack(() => monthOf(today) ?? today.slice(0, 7)));
	let days = $state<CalendarDay[]>([]);
	let loading = $state(true);
	let failed = $state(false);
	let selected = $state<string | null>(null);

	async function load(target: string) {
		if (!available) {
			loading = false;
			return;
		}
		loading = true;
		failed = false;
		try {
			const params: ListParams = {
				sort: [],
				// サーバ側は `month` フィルタだけを見る（`calendar.rs`）。
				filters: [{ field: 'month', op: 'eq', value: target }],
				pagination: { offset: 0, limit: 31 }
			};
			const result = await getDataProvider().getList<CalendarDay>('calendar', params);
			days = result.rows;
		} catch {
			failed = true;
		} finally {
			loading = false;
		}
	}

	$effect(() => {
		void load(month);
	});

	function step(offset: number) {
		const next = shiftMonth(month, offset);
		if (next !== null) {
			month = next;
			selected = null;
		}
	}

	const byDate = $derived(Object.fromEntries(days.map((day) => [day.date, day])));

	/**
	 * 案件ごとの色は「その月に出てくる順」で割り当てる。案件 id を色数で
	 * 割ると、id の飛び方次第で隣り合う案件が同じ色になる。
	 */
	const colorByProject = $derived.by(() => {
		const order = new Map<number, number>();
		for (const day of days) {
			for (const slice of day.projects) {
				if (!order.has(slice.projectId)) order.set(slice.projectId, order.size);
			}
		}
		return order;
	});

	const yen = (value: number): string => `¥${value.toLocaleString()}`;

	function badgesFor(day: CalendarDay): CalendarBadge[] {
		const badges: CalendarBadge[] = [];
		if (day.expenseCount > 0) {
			badges.push({
				key: 'expenses',
				label: m['calendar.badgeExpense']({ count: day.expenseCount }),
				title: m['calendar.expenseTitle']({
					count: day.expenseCount,
					amount: yen(day.expenseAmount)
				}),
				tone: 'neutral'
			});
		}
		if (day.tripCount > 0) {
			badges.push({
				key: 'trips',
				label: m['calendar.badgeTrip']({ count: day.tripCount }),
				title: m['calendar.tripTitle']({ count: day.tripCount }),
				tone: 'accent'
			});
		}
		if (day.invoiceClosingCount > 0) {
			badges.push({
				key: 'closing',
				label: m['calendar.badgeClosing']({ count: day.invoiceClosingCount }),
				title: m['calendar.closingTitle']({ count: day.invoiceClosingCount }),
				tone: 'neutral'
			});
		}
		if (day.invoiceDueCount > 0) {
			badges.push({
				key: 'due',
				label: m['calendar.badgeDue']({ count: day.invoiceDueCount }),
				title: m['calendar.dueTitle']({
					count: day.invoiceDueCount,
					remaining: yen(day.invoiceDueRemaining)
				}),
				// 残額が残っている期限だけ警告色。回収済みなら中立。
				tone: day.invoiceDueRemaining > 0 ? 'warning' : 'neutral'
			});
		}
		if (day.paymentCount > 0) {
			badges.push({
				key: 'payments',
				label: m['calendar.badgePayment']({ count: day.paymentCount }),
				title: m['calendar.paymentTitle']({
					count: day.paymentCount,
					amount: yen(day.paymentAmount)
				}),
				tone: 'success'
			});
		}
		return badges;
	}

	function barsFor(day: CalendarDay): CalendarBar[] {
		if (day.workedMinutes <= 0) return [];
		return day.projects.map((slice) => ({
			key: String(slice.projectId),
			title: `${slice.projectName}（${formatHours(slice.minutes)}）`,
			ratio: slice.minutes / day.workedMinutes,
			colorIndex: colorByProject.get(slice.projectId) ?? 0
		}));
	}

	/** 工数の入力が無い過去の平日か。未来と土日は対象外。 */
	function isMissingWorkLog(date: string, day: CalendarDay | undefined): boolean {
		if (date > today) return false;
		if (isWeekend(date)) return false;
		return (day?.workedMinutes ?? 0) === 0;
	}

	/** 表示中の月の全日。`flagged` の判定に使う。 */
	const allDatesOfMonth = $derived.by(() => {
		const first = `${month}-01`;
		const next = shiftMonth(month, 1);
		if (next === null) return [] as string[];
		const dates: string[] = [];
		// 月末は「翌月1日の前日」。日数を自前で数えるより取り違えが起きない。
		const end = Date.UTC(Number(next.slice(0, 4)), Number(next.slice(5, 7)) - 1, 1);
		let cursor = Date.UTC(Number(month.slice(0, 4)), Number(month.slice(5, 7)) - 1, 1);
		if (Number.isNaN(cursor) || Number.isNaN(end)) return [first];
		while (cursor < end) {
			dates.push(new Date(cursor).toISOString().slice(0, 10));
			cursor += 24 * 60 * 60 * 1000;
		}
		return dates;
	});

	const cells = $derived.by(() => {
		const out: Record<string, CalendarCell> = {};
		// 升目の集合はグリッド側が月から組み立てるが、強調したい「空の日」は
		// サーバから行が来ないので、こちらで月の全日を舐めて作る必要がある。
		for (const date of allDatesOfMonth) {
			const day = byDate[date];
			const missing = isMissingWorkLog(date, day);
			if (!day && !missing) continue;
			out[date] = {
				primary: day && day.workedMinutes > 0 ? formatHours(day.workedMinutes) : undefined,
				bars: day ? barsFor(day) : [],
				badges: day ? badgesFor(day) : [],
				flagged: missing,
				flaggedTitle: missing ? m['calendar.noWorkLogged']() : undefined
			};
		}
		return out;
	});

	const monthLabel = $derived(
		m['calendar.monthLabel']({
			year: month.slice(0, 4),
			month: String(Number(month.slice(5, 7)))
		})
	);

	const totals = $derived({
		minutes: days.reduce((sum, day) => sum + day.workedMinutes, 0),
		expense: days.reduce((sum, day) => sum + day.expenseAmount, 0),
		payment: days.reduce((sum, day) => sum + day.paymentAmount, 0),
		missing: allDatesOfMonth.filter((date) => isMissingWorkLog(date, byDate[date])).length
	});

	const gridMessages = $derived({
		weekdayNames: [
			m['calendar.weekdaySun'](),
			m['calendar.weekdayMon'](),
			m['calendar.weekdayTue'](),
			m['calendar.weekdayWed'](),
			m['calendar.weekdayThu'](),
			m['calendar.weekdayFri'](),
			m['calendar.weekdaySat']()
		],
		today: m['calendar.today'](),
		gridLabel: m['calendar.gridLabel']({ month: monthLabel })
	});

	const selectedDay = $derived(selected === null ? undefined : byDate[selected]);
</script>

<section class="calendar">
	<header class="bar">
		<div class="nav">
			<button type="button" class="banto-btn" onclick={() => step(-1)}>
				{m['calendar.previousMonth']()}
			</button>
			<strong class="month">{monthLabel}</strong>
			<button type="button" class="banto-btn" onclick={() => step(1)}>
				{m['calendar.nextMonth']()}
			</button>
			<button
				type="button"
				class="banto-btn"
				onclick={() => {
					month = monthOf(today) ?? month;
					selected = null;
				}}
			>
				{m['calendar.thisMonth']()}
			</button>
		</div>

		{#if available && !loading && !failed}
			<dl class="totals">
				<div>
					<dt>{m['calendar.totalWorked']()}</dt>
					<dd>{formatHours(totals.minutes)}</dd>
				</div>
				<div>
					<dt>{m['calendar.totalExpense']()}</dt>
					<dd>{yen(totals.expense)}</dd>
				</div>
				<div>
					<dt>{m['calendar.totalPayment']()}</dt>
					<dd>{yen(totals.payment)}</dd>
				</div>
				<div class:warn={totals.missing > 0}>
					<dt>{m['calendar.totalMissing']()}</dt>
					<dd>{m['calendar.missingDays']({ count: totals.missing })}</dd>
				</div>
			</dl>
		{/if}
	</header>

	{#if !available}
		<p class="note">{m['calendar.unavailable']()}</p>
	{:else if loading}
		<LoadingState label={m['common.loading']()} />
	{:else if failed}
		<ErrorState title={m['calendar.loadError']()} description={m['resource.loadErrorDesc']()} />
	{:else}
		<MonthGrid
			{month}
			{cells}
			{today}
			{selected}
			messages={gridMessages}
			onselect={(date) => (selected = selected === date ? null : date)}
		/>

		{#if selected}
			<div class="day">
				<h3>{selected}</h3>
				{#if selectedDay}
					<ul class="lines">
						{#if selectedDay.workedMinutes > 0}
							<li>
								{m['calendar.totalWorked']()}: {formatHours(selectedDay.workedMinutes)}
								<ul class="sub">
									{#each selectedDay.projects as slice (slice.projectId)}
										<li>
											<a href={`${base}/projects/${slice.projectId}`}>{slice.projectName}</a>
											— {formatHours(slice.minutes)}
										</li>
									{/each}
								</ul>
							</li>
						{/if}
						{#if selectedDay.expenseCount > 0}
							<li>
								{m['calendar.expenseTitle']({
									count: selectedDay.expenseCount,
									amount: yen(selectedDay.expenseAmount)
								})}
							</li>
						{/if}
						{#if selectedDay.tripCount > 0}
							<li>{m['calendar.tripTitle']({ count: selectedDay.tripCount })}</li>
						{/if}
						{#if selectedDay.invoiceClosingCount > 0}
							<li>
								{m['calendar.closingTitle']({ count: selectedDay.invoiceClosingCount })}
							</li>
						{/if}
						{#if selectedDay.invoiceDueCount > 0}
							<li>
								{m['calendar.dueTitle']({
									count: selectedDay.invoiceDueCount,
									remaining: yen(selectedDay.invoiceDueRemaining)
								})}
							</li>
						{/if}
						{#if selectedDay.paymentCount > 0}
							<li>
								{m['calendar.paymentTitle']({
									count: selectedDay.paymentCount,
									amount: yen(selectedDay.paymentAmount)
								})}
							</li>
						{/if}
					</ul>
				{:else}
					<p class="note">{m['calendar.noWorkLogged']()}</p>
				{/if}
				<a class="banto-btn banto-btn--primary" href={`${base}/work-logs/new?date=${selected}`}>
					{m['calendar.addWorkLog']()}
				</a>
			</div>
		{/if}
	{/if}
</section>

<style>
	.calendar {
		display: flex;
		flex-direction: column;
		gap: 0.75rem;
	}

	.bar {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		justify-content: space-between;
		gap: 0.75rem;
	}

	.nav {
		display: flex;
		align-items: center;
		gap: 0.5rem;
	}

	.month {
		min-width: 8rem;
		text-align: center;
		font-size: 1rem;
	}

	.totals {
		display: flex;
		flex-wrap: wrap;
		gap: 1rem;
		margin: 0;
	}

	.totals div {
		display: flex;
		align-items: baseline;
		gap: 0.375rem;
	}

	.totals dt {
		color: var(--banto-text-muted);
		font-size: 0.8rem;
	}

	.totals dd {
		margin: 0;
		font-weight: 600;
		font-variant-numeric: tabular-nums;
	}

	.totals .warn dd {
		color: var(--banto-warning-tint-text);
	}

	.day {
		display: flex;
		flex-direction: column;
		align-items: flex-start;
		gap: 0.5rem;
		border: 1px solid var(--banto-border);
		border-radius: var(--banto-radius-md);
		background: var(--banto-surface);
		padding: 0.75rem 1rem;
	}

	.day h3 {
		margin: 0;
		font-size: 0.95rem;
		font-variant-numeric: tabular-nums;
	}

	.lines,
	.sub {
		margin: 0;
		padding-left: 1.1rem;
		font-size: 0.875rem;
	}

	.sub {
		color: var(--banto-text-muted);
	}

	.note {
		margin: 0;
		color: var(--banto-text-muted);
		font-size: 0.85rem;
	}
</style>
