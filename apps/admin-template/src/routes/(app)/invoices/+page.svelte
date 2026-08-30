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
	import { projectOptions, loadProjectOptions } from '$lib/banto/referenceOptions.svelte';

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
		{
			id: 'open',
			header: m['resource.colActions'](),
			accessor: () => '',
			cell: (row) => ({ text: m['resource.openRow'](), href: `${base}/invoices/${row.id}` }),
			width: 80
		},
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
			// 支払条件が未設定の顧客の確定済み請求書は期日が NULL のままになる
			// （アルファ実使用からのフィードバック、2026-08-27）。空欄だと
			// 読み込み中や列ずれと区別しづらいので「—」で明示する
			// （`customers` 一覧・PDF と同じ記法）。
			accessor: (row) => row.dueOn ?? '—',
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

	/**
	 * 案件での絞り込み（アルファ実機フィードバック）。
	 *
	 * `Invoice` は `project_id` を持たない（`CLAUDE.md` 1.3）ので、これは
	 * `invoices` の実カラムではなく擬似フィルタ —— サーバ側
	 * （`core/src/invoices.rs::list`）が `projectId` を受けたときだけ、
	 * その案件の明細を1行でも含む請求書に絞る特別扱いをする。
	 *
	 * `BantoGrid` の `onParamsChange` は毎回 `filters` を丸ごと差し替えて
	 * 渡してくる（グリッド側の列フィルタ変更のたび）ので、直近のグリッド
	 * パラメータを覚えておき、案件セレクトの変更時にもそれへ合成し直す。
	 */
	let selectedProjectId = $state<number | null>(null);
	let lastGridParams: { sort: SortState[]; filters: FilterState[] } = { sort: [], filters: [] };

	$effect(() => {
		void loadProjectOptions();
	});

	$effect(() => {
		void windowed.ensureRange(0, 100);
	});

	$effect(() => {
		return () => windowed.dispose();
	});

	function applyParams(params: { sort: SortState[]; filters: FilterState[] }): void {
		const filters =
			selectedProjectId === null
				? params.filters
				: [...params.filters, { field: 'projectId', op: 'eq' as const, value: selectedProjectId }];
		windowed.setParams({ sort: params.sort, filters });
		void windowed.ensureRange(visibleRange.start, visibleRange.end);
	}

	function handleParamsChange(params: { sort: SortState[]; filters: FilterState[] }): void {
		lastGridParams = params;
		applyParams(params);
	}

	function handleProjectFilterChange(rawValue: string): void {
		selectedProjectId = rawValue === '' ? null : Number(rawValue);
		applyParams(lastGridParams);
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

	<div class="filter-bar">
		<label class="filter-field" for="invoice-project-filter">
			<span>{m['invoices.filterProjectLabel']()}</span>
			<select
				id="invoice-project-filter"
				class="banto-input"
				value={selectedProjectId === null ? '' : String(selectedProjectId)}
				onchange={(event) => handleProjectFilterChange(event.currentTarget.value)}
			>
				<option value="">{m['invoices.filterAllProjects']()}</option>
				{#each projectOptions() as option (option.value)}
					<option value={String(option.value)}>{option.label}</option>
				{/each}
			</select>
		</label>
	</div>

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

	.filter-bar {
		display: flex;
		flex-wrap: wrap;
		align-items: flex-end;
		gap: 0.75rem;
	}

	.filter-field {
		display: flex;
		flex-direction: column;
		gap: 0.25rem;
		min-width: 0;
		width: 100%;
		max-width: 20rem;
		font-size: 0.85rem;
		color: var(--banto-text-muted);
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
