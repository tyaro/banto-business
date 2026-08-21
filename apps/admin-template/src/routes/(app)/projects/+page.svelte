<script lang="ts">
	/**
	 * 案件一覧（docs/recipes/add-resource.md 手順8）。
	 *
	 * クライアント/サーバー両モードの切り替えは持たず、**サーバー
	 * モードのみ**にしている: 案件・案件は「1万件のデモデータを手元で
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
	import { projectsSchema, projectStatusLabel } from '$lib/banto/resources/projects';
	import { sessionStore } from '$lib/session.svelte';
	import { canWriteResources } from '$lib/permissions';
	import PageHeader from '$lib/components/ui/PageHeader.svelte';

	interface ProjectRow {
		id: number;
		code: string;
		customerId: number;
		name: string;
		status: string;
		contractAmount: number | null;
		[key: string]: unknown;
	}

	const canWrite = $derived(canWriteResources(sessionStore.role));

	/** 金額は円で表示する。未入力（null）の行は空欄のままにする。 */
	const amountFormat = (value: unknown): string =>
		value === null || value === undefined || value === ''
			? ''
			: `¥${Number(value).toLocaleString()}`;

	const baseColumns: GridColumn<ProjectRow>[] = [
		{
			id: 'open',
			header: m['resource.colActions'](),
			accessor: () => '',
			width: 80,
			editable: false
		},
		...columnsFromSchema<ProjectRow>(projectsSchema, {
			overrides: {
				code: { width: 120 },
				customerId: { width: 90 },
				name: { width: 280 },
				// 状態はコードのまま出さず表示名に変換する（用語集の表記に統一）。
				status: { width: 120, format: (value) => projectStatusLabel(value) },
				estimateAmount: { width: 130, format: amountFormat },
				contractAmount: { width: 130, format: amountFormat },
				startedOn: { width: 120 },
				dueOn: { width: 120 },
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
	const gridState = new GridState<ProjectRow>(columns);
	const windowed = createWindowedListResource<ProjectRow>('projects');

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

	function openRow(row: ProjectRow) {
		goto(`${base}/projects/${row.id}`);
	}
</script>

<div class="page">
	<PageHeader title={m['projects.resourceLabel']()} description={m['projects.description']()}>
		{#snippet actions()}
			{#if canWrite}
				<a class="banto-btn banto-btn--primary" href={`${base}/projects/new`}>
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
