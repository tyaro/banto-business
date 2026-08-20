<script lang="ts">
	/**
	 * 案件採算の表示（Phase 4、要件 F-P1〜F-P7）。
	 *
	 * **計算はしない**（AGENTS.md 第1章）。サーバが導出した値を並べるだけで、
	 * 粗利も実質時間単価も率もこちらでは組み立てない — フロントで足し引きを
	 * 始めると、丸めの位置がサーバと食い違う。
	 *
	 * 実質時間単価は**2種を必ず並べる**（F-P2）。移動を分母に入れるかで
	 * 数値が大きく変わり、片方だけを見ると受注可否の判断を誤る。
	 */
	import { getDataProvider } from '@banto/admin-core';
	import * as m from '$lib/paraglide/messages';
	import LoadingState from '$lib/components/ui/LoadingState.svelte';
	import ErrorState from '$lib/components/ui/ErrorState.svelte';

	interface Profitability {
		contractAmount: number | null;
		revenue: number;
		workCost: number;
		expenseCost: number;
		billableExpenseCost: number;
		uninvoicedBillableExpenseCost: number;
		totalCost: number;
		grossProfit: number;
		grossMarginBp: number | null;
		invoiceProgressBp: number | null;
		totalMinutes: number;
		excludedMinutes: number;
		effectiveRateIncludingTravel: number | null;
		effectiveRateExcludingTravel: number | null;
		countsTowardProfitability: boolean;
	}

	interface Props {
		projectId: number;
	}

	const { projectId }: Props = $props();

	let data = $state<Profitability | null>(null);
	let loading = $state(true);
	let failed = $state(false);

	async function load() {
		loading = true;
		failed = false;
		try {
			data = await getDataProvider().getOne<Profitability>('profitability', projectId);
		} catch {
			failed = true;
		} finally {
			loading = false;
		}
	}

	$effect(() => {
		void load();
	});

	/** 円。表示整形だけで、計算はしない。 */
	const yen = (value: number | null): string =>
		value === null ? '—' : `¥${value.toLocaleString()}`;

	/** basis point（1/10000）を百分率の文字列にする。小数第2位まで見せるのは
	 *  bp のまま持っている桁を落とさないため（表示のみ・再計算ではない）。 */
	const percent = (bp: number | null): string => (bp === null ? '—' : `${(bp / 100).toFixed(2)}%`);

	/** 分を「12h30m」の形にする（工数一覧と同じ整形）。 */
	const hm = (minutes: number): string => {
		const hours = Math.floor(minutes / 60);
		const rest = minutes % 60;
		return hours > 0 ? `${hours}h${rest > 0 ? `${rest}m` : ''}` : `${rest}m`;
	};

	/** 単価は円/時。 */
	const rate = (value: number | null): string =>
		value === null ? '—' : `¥${value.toLocaleString()}${m['profitability.perHourSuffix']()}`;
</script>

<section class="panel">
	<header class="panel-header">
		<h2>{m['profitability.title']()}</h2>
		<button type="button" class="banto-btn banto-btn--ghost" onclick={() => void load()}>
			{m['common.reload']()}
		</button>
	</header>

	{#if loading}
		<LoadingState label={m['common.loading']()} />
	{:else if failed || !data}
		<ErrorState
			title={m['profitability.loadError']()}
			description={m['profitability.loadErrorDesc']()}
		/>
	{:else}
		{#if !data.countsTowardProfitability}
			<p class="note note--warn">{m['profitability.excludedFromAggregate']()}</p>
		{/if}

		<dl class="metrics">
			<div class="metric">
				<dt>{m['profitability.contractAmount']()}</dt>
				<dd>{yen(data.contractAmount)}</dd>
			</div>
			<div class="metric">
				<dt>{m['profitability.revenue']()}</dt>
				<dd>{yen(data.revenue)}</dd>
			</div>
			<div class="metric">
				<dt>{m['profitability.invoiceProgress']()}</dt>
				<dd>{percent(data.invoiceProgressBp)}</dd>
			</div>
			<div class="metric">
				<dt>{m['profitability.workCost']()}</dt>
				<dd>{yen(data.workCost)}</dd>
			</div>
			<div class="metric">
				<dt>{m['profitability.expenseCost']()}</dt>
				<dd>{yen(data.expenseCost)}</dd>
			</div>
			<div class="metric">
				<dt>{m['profitability.totalCost']()}</dt>
				<dd>{yen(data.totalCost)}</dd>
			</div>
			<div class="metric metric--strong">
				<dt>{m['profitability.grossProfit']()}</dt>
				<dd class:negative={data.grossProfit < 0}>{yen(data.grossProfit)}</dd>
			</div>
			<div class="metric">
				<dt>{m['profitability.grossMargin']()}</dt>
				<dd>{percent(data.grossMarginBp)}</dd>
			</div>
			<div class="metric">
				<dt>{m['profitability.totalMinutes']()}</dt>
				<dd>{hm(data.totalMinutes)}</dd>
			</div>
			<div class="metric">
				<dt>{m['profitability.excludedMinutes']()}</dt>
				<dd>{hm(data.excludedMinutes)}</dd>
			</div>
			<div class="metric metric--strong">
				<dt>{m['profitability.effectiveRateIncludingTravel']()}</dt>
				<dd class:negative={(data.effectiveRateIncludingTravel ?? 0) < 0}>
					{rate(data.effectiveRateIncludingTravel)}
				</dd>
			</div>
			<div class="metric metric--strong">
				<dt>{m['profitability.effectiveRateExcludingTravel']()}</dt>
				<dd class:negative={(data.effectiveRateExcludingTravel ?? 0) < 0}>
					{rate(data.effectiveRateExcludingTravel)}
				</dd>
			</div>
		</dl>

		{#if data.uninvoicedBillableExpenseCost > 0}
			<p class="note">
				{m['profitability.uninvoicedBillable']({
					amount: yen(data.uninvoicedBillableExpenseCost)
				})}
			</p>
		{/if}
		<p class="note note--muted">{m['profitability.revenueBasisNote']()}</p>
	{/if}
</section>

<style>
	.panel {
		display: flex;
		flex-direction: column;
		gap: 0.75rem;
		background: var(--banto-surface);
		border: 1px solid var(--banto-border);
		border-radius: var(--banto-radius-lg);
		box-shadow: var(--banto-shadow-sm);
		padding: 1.25rem;
	}

	.panel-header {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 0.5rem;
	}

	.panel-header h2 {
		margin: 0;
		font-size: 1rem;
		font-weight: 600;
	}

	.metrics {
		display: grid;
		grid-template-columns: repeat(auto-fit, minmax(12rem, 1fr));
		gap: 0.75rem;
		margin: 0;
	}

	.metric {
		display: flex;
		flex-direction: column;
		gap: 0.125rem;
		padding: 0.5rem 0.75rem;
		border: 1px solid var(--banto-border);
		border-radius: var(--banto-radius-md);
	}

	.metric--strong {
		background: var(--banto-surface-subtle);
	}

	.metric dt {
		color: var(--banto-text-muted);
		font-size: 0.85rem;
	}

	.metric dd {
		margin: 0;
		font-size: 1.15rem;
		font-variant-numeric: tabular-nums;
	}

	.negative {
		color: var(--banto-danger);
	}

	.note {
		margin: 0;
		font-size: 0.85rem;
	}

	.note--muted {
		color: var(--banto-text-muted);
	}

	.note--warn {
		color: var(--banto-warning);
	}
</style>
