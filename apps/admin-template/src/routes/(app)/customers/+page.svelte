<script lang="ts">
	/**
	 * 顧客一覧（docs/recipes/add-resource.md 手順8）。
	 *
	 * クライアント/サーバー両モードの切り替えは持たず、**サーバー
	 * モードのみ**にしている: 顧客・案件は「1万件のデモデータを手元で
	 * ソートして見せる」性質のリソースではなく、DataProvider 側で
	 * sort/filter/paging を解決する経路（spec §4.1/§4.2）だけで足りるため。
	 * 列はスキーマから `columnsFromSchema` で導出し、手書きは行リンク列
	 * だけに留める（M23）。
	 */
	import {
		BantoGrid,
		GridState,
		columnsFromSchema,
		type FilterState,
		type GridColumn,
		type SortState
	} from '@banto/grid-svelte';
	import { createWindowedListResource } from '@banto/admin-core';
	import { goto } from '$app/navigation';
	import { base } from '$app/paths';
	import { Plus } from '@lucide/svelte';
	import * as m from '$lib/paraglide/messages';
	import { gridMessages, columnValidationMessages } from '$lib/banto/i18n';
	import { customersSchema, DAY_END_OF_MONTH } from '$lib/banto/resources/customers';
	import { sessionStore } from '$lib/session.svelte';
	import { canWriteResources } from '$lib/permissions';
	import PageHeader from '$lib/components/ui/PageHeader.svelte';

	interface CustomerRow {
		id: number;
		code: string;
		name: string;
		closingDay: number;
		paymentMonthOffset: number;
		paymentDay: number;
		[key: string]: unknown;
	}

	const canWrite = $derived(canWriteResources(sessionStore.role));

	/** 締日・支払日の表示: 99 は「末日」と読ませる（Phase 1 決定 C-8）。 */
	const dayFormat = (value: unknown): string =>
		Number(value) === DAY_END_OF_MONTH ? m['customers.dayEndOfMonth']() : String(value ?? '');

	const baseColumns: GridColumn<CustomerRow>[] = [
		{
			id: 'open',
			header: m['resource.colActions'](),
			accessor: () => '',
			width: 80,
			editable: false
		},
		{ id: 'id', header: 'ID', accessor: (row) => row.id, width: 72, editable: false },
		...columnsFromSchema<CustomerRow>(customersSchema, {
			overrides: {
				code: { width: 120 },
				name: { width: 240 },
				closingDay: { width: 110, format: dayFormat },
				paymentDay: { width: 110, format: dayFormat },
				paymentMonthOffset: { width: 110 },
				updatedAt: { width: 130 }
			},
			messages: columnValidationMessages()
		})
	];

	// spec M10 RBAC: viewer はインライン編集不可。
	const columns = $derived(
		canWrite
			? baseColumns
			: baseColumns.map((column) => (column.editable ? { ...column, editable: false } : column))
	);

	// `columns` は $derived だが、GridState は生成時の列定義を保持して以降
	// 自前で管理する。RBAC による editable の差は
	// 生成時点で確定しているため、初期値の捕捉で問題ない。
	// svelte-ignore state_referenced_locally
	const gridState = new GridState<CustomerRow>(columns);
	const windowed = createWindowedListResource<CustomerRow>('customers');

	// 直近の可視範囲。sort/filter 変更時にどの範囲を取り直すかに使う。
	let visibleRange = { start: 0, end: 100 };

	// `ItemsServerGrid.svelte` と同じく、読み込みと後片付けを **別々の**
	// effect にする（1つにまとめると `windowed.params` に依存してしまい、
	// sort/filter のたびに cleanup が走って invalidate 購読が切れる）。
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

	function openRow(row: CustomerRow) {
		goto(`${base}/customers/${row.id}`);
	}
</script>

<div class="page">
	<PageHeader title={m['customers.resourceLabel']()} description={m['customers.description']()}>
		{#snippet actions()}
			{#if canWrite}
				<a class="banto-btn banto-btn--primary" href={`${base}/customers/new`}>
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
