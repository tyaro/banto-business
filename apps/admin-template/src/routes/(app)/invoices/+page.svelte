<script lang="ts">
	/**
	 * 請求書の一覧（Phase 5）。
	 *
	 * 他のリソースと違い `$lib/banto/resources` に登録していない — 明細を
	 * 内包するため編集が `BantoForm` の1リソース1フォームに収まらず、専用の
	 * 編集画面を持つ（`docs/recipes/add-resource.md` 手順7のリソース定義は
	 * フォームを前提にしている）。一覧は他と同じサーバーモードの `BantoGrid`。
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

	interface InvoiceRow {
		id: number;
		invoiceNumber: string | null;
		customerId: number;
		status: string;
		issuedOn: string | null;
		dueOn: string | null;
		totalAmount: number;
		[key: string]: unknown;
	}

	const canWrite = $derived(canWriteResources(sessionStore.role));

	/** 金額は円で表示する（計算はサーバ側。AGENTS.md 第1章）。 */
	const amountFormat = (value: unknown): string =>
		value === null || value === undefined || value === ''
			? ''
			: `¥${Number(value).toLocaleString()}`;

	const statusLabels: Record<string, () => string> = {
		DRAFT: m['invoices.statusDraft'],
		ISSUED: m['invoices.statusIssued'],
		CANCELLED: m['invoices.statusCancelled']
	};
	const statusFormat = (value: unknown): string => {
		const label = statusLabels[String(value)];
		return label ? label() : String(value ?? '');
	};

	// 列は手書き。スキーマ由来の `columnsFromSchema` を使わないのは、このリソース
	// がフォームスキーマを持たないため（上の doc コメント参照）。
	const baseColumns: GridColumn<InvoiceRow>[] = [
		{ id: 'open', header: m['resource.colActions'](), accessor: () => '', width: 80 },
		{ id: 'id', header: 'ID', accessor: (row) => row.id, width: 72 },
		{
			id: 'invoiceNumber',
			header: m['invoices.fieldInvoiceNumber'](),
			accessor: (row) => row.invoiceNumber ?? '',
			width: 150
		},
		{
			id: 'status',
			header: m['invoices.fieldStatus'](),
			accessor: (row) => row.status,
			format: statusFormat,
			width: 110
		},
		{
			id: 'customerId',
			header: m['invoices.fieldCustomerId'](),
			accessor: (row) => row.customerId,
			width: 90
		},
		{
			id: 'issuedOn',
			header: m['invoices.fieldIssuedOn'](),
			accessor: (row) => row.issuedOn ?? '',
			width: 120
		},
		{
			id: 'dueOn',
			header: m['invoices.fieldDueOn'](),
			accessor: (row) => row.dueOn ?? '',
			width: 120
		},
		{
			id: 'totalAmount',
			header: m['invoices.fieldTotalAmount'](),
			accessor: (row) => row.totalAmount,
			format: amountFormat,
			width: 140
		}
	];

	const columns = baseColumns.map((column) => ({ ...column, editable: false }));

	// `columns` は $derived ではない（RBAC でインライン編集を切り替える列が
	// 無いため）ので、GridState にそのまま渡せる。
	const gridState = new GridState<InvoiceRow>(columns);
	const windowed = createWindowedListResource<InvoiceRow>('invoices');

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

	function openRow(row: InvoiceRow) {
		goto(`${base}/invoices/${row.id}`);
	}
</script>

<div class="page">
	<PageHeader title={m['invoices.resourceLabel']()} description={m['invoices.description']()}>
		{#snippet actions()}
			{#if canWrite}
				<a class="banto-btn banto-btn--primary" href={`${base}/invoices/new`}>
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
