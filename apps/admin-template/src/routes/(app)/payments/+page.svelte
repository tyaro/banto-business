<script lang="ts">
	/**
	 * 入金の一覧（Phase 6）。請求書と同じく `$lib/banto/resources` には登録して
	 * いない — 充当（`allocations`）を内包するため `BantoForm` の1リソース1
	 * フォームに収まらず、専用の編集画面を持つ。
	 */
	import {
		BantoGrid,
		GridState,
		type FilterState,
		type GridColumn,
		type SortState
	} from '@banto/grid-svelte';
	import { createWindowedListResource } from '@banto/admin-core';
	import { goto } from '$app/navigation';
	import { base } from '$app/paths';
	import { Plus } from '@lucide/svelte';
	import * as m from '$lib/paraglide/messages';
	import { gridMessages } from '$lib/banto/i18n';
	import { sessionStore } from '$lib/session.svelte';
	import { canWriteResources } from '$lib/permissions';
	import PageHeader from '$lib/components/ui/PageHeader.svelte';

	interface PaymentRow {
		id: number;
		customerId: number;
		paidOn: string;
		amount: number;
		method: string | null;
		[key: string]: unknown;
	}

	const canWrite = $derived(canWriteResources(sessionStore.role));

	/** 金額は円で表示する（計算はサーバ側。AGENTS.md 第1章）。 */
	const amountFormat = (value: unknown): string =>
		value === null || value === undefined || value === ''
			? ''
			: `¥${Number(value).toLocaleString()}`;

	// 一覧はインライン編集を持たないので、`editable: false` を最後に一括で付ける。
	const baseColumns: GridColumn<PaymentRow>[] = [
		{
			id: 'open',
			header: m['resource.colActions'](),
			accessor: () => '',
			cell: (row) => ({ text: m['resource.openRow'](), href: `${base}/payments/${row.id}` }),
			width: 80
		},
		{
			id: 'paidOn',
			header: m['payments.fieldPaidOn'](),
			accessor: (row) => row.paidOn,
			width: 120
		},
		{
			id: 'customerId',
			header: m['payments.fieldCustomerId'](),
			accessor: (row) => row.customerId,
			width: 90
		},
		{
			id: 'amount',
			header: m['payments.fieldAmount'](),
			accessor: (row) => row.amount,
			format: amountFormat,
			width: 140
		},
		{
			id: 'method',
			header: m['payments.fieldMethod'](),
			accessor: (row) => row.method ?? '',
			width: 120
		}
	];

	const columns: GridColumn<PaymentRow>[] = baseColumns.map((column) => ({
		...column,
		editable: false
	}));

	const gridState = new GridState<PaymentRow>(columns);
	const windowed = createWindowedListResource<PaymentRow>('payments');

	let visibleRange = { start: 0, end: 100 };

	$effect(() => {
		void windowed.ensureRange(0, 100);
	});

	$effect(() => {
		return () => windowed.dispose();
	});

	function handleParamsChange(params: { sort: SortState[]; filters: FilterState[] }): void {
		windowed.setParams(params);
		void windowed.ensureRange(visibleRange.start, visibleRange.end);
	}

	function handleVisibleRangeChange(range: { start: number; end: number }): void {
		visibleRange = range;
		void windowed.ensureRange(range.start, range.end);
	}

	function openRow(row: PaymentRow) {
		goto(`${base}/payments/${row.id}`);
	}
</script>

<div class="page">
	<PageHeader title={m['payments.resourceLabel']()} description={m['payments.description']()}>
		{#snippet actions()}
			{#if canWrite}
				<a class="banto-btn banto-btn--primary" href={`${base}/payments/new`}>
					<Plus size={16} />
					{m['resource.create']()}
				</a>
			{/if}
		{/snippet}
	</PageHeader>

	<div class="grid-panel">
		<BantoGrid
			mode="server"
			state={gridState}
			messages={gridMessages()}
			rows={windowed.rows}
			totalRows={windowed.totalCount}
			{columns}
			getRowId={(row) => row.id}
			onRowClick={openRow}
			onParamsChange={handleParamsChange}
			onVisibleRangeChange={handleVisibleRangeChange}
		/>
	</div>

	<p class="row-count">{m['resource.rowCount']({ count: windowed.totalCount })}</p>
</div>

<style>
	.page {
		display: flex;
		flex-direction: column;
		gap: 1rem;
		height: 100%;
	}

	.grid-panel {
		flex: 1;
		min-height: 24rem;
		background: var(--banto-surface);
		border: 1px solid var(--banto-border);
		border-radius: var(--banto-radius-lg);
		box-shadow: var(--banto-shadow-sm);
		overflow: hidden;
	}

	.row-count {
		margin: 0;
		color: var(--banto-text-muted);
		font-size: 0.85rem;
	}
</style>
