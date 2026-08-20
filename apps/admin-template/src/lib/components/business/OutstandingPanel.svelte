<script lang="ts">
	/**
	 * 未入金・期限超過の一覧（Phase 6、要件 F-Y7）。
	 *
	 * **期限超過は状態として持たない**（`CLAUDE.md` 1.5 / 要件 F-Y6）。
	 * `支払期限 < 今日 AND 残額 > 0 AND 状態 ≠ 取消` をサーバが都度評価した結果を
	 * 表示するだけで、こちらでは判定も金額計算もしない。日付比較をフロントで
	 * やるとタイムゾーンで1日ずれる。
	 */
	import { getDataProvider } from '@banto/admin-core';
	import type { ListParams } from '@banto/admin-core';
	import { base } from '$app/paths';
	import * as m from '$lib/paraglide/messages';
	import LoadingState from '$lib/components/ui/LoadingState.svelte';
	import ErrorState from '$lib/components/ui/ErrorState.svelte';

	interface Settlement {
		invoiceId: number;
		invoiceNumber: string | null;
		customerName: string;
		dueOn: string | null;
		totalAmount: number;
		remainingAmount: number;
		settlementStatus: string;
		overdue: boolean;
	}

	let rows = $state<Settlement[]>([]);
	let loading = $state(true);
	let failed = $state(false);

	async function load() {
		loading = true;
		failed = false;
		try {
			const params: ListParams = {
				sort: [],
				filters: [],
				pagination: { offset: 0, limit: 100 }
			};
			const result = await getDataProvider().getList<Settlement>('outstanding', params);
			rows = result.rows;
		} catch {
			failed = true;
		} finally {
			loading = false;
		}
	}

	$effect(() => {
		void load();
	});

	const yen = (value: number): string => `¥${value.toLocaleString()}`;
	const overdueCount = $derived(rows.filter((row) => row.overdue).length);
</script>

<section class="panel">
	<header class="panel-header">
		<h2>{m['outstanding.title']()}</h2>
		{#if !loading && !failed}
			<span class="summary">
				{m['outstanding.summary']({ count: rows.length, overdue: overdueCount })}
			</span>
		{/if}
	</header>

	{#if loading}
		<LoadingState label={m['common.loading']()} />
	{:else if failed}
		<ErrorState title={m['outstanding.loadError']()} description={m['resource.loadErrorDesc']()} />
	{:else if rows.length === 0}
		<p class="note note--muted">{m['outstanding.empty']()}</p>
	{:else}
		<div class="table-scroll">
			<table>
				<thead>
					<tr>
						<th>{m['invoices.fieldInvoiceNumber']()}</th>
						<th>{m['invoices.fieldCustomerId']()}</th>
						<th>{m['invoices.fieldDueOn']()}</th>
						<th class="num">{m['invoices.fieldTotalAmount']()}</th>
						<th class="num">{m['outstanding.remaining']()}</th>
						<th>{m['invoices.fieldStatus']()}</th>
					</tr>
				</thead>
				<tbody>
					{#each rows as row (row.invoiceId)}
						<tr class:overdue={row.overdue}>
							<td>
								<a href={`${base}/invoices/${row.invoiceId}`}>{row.invoiceNumber ?? '—'}</a>
							</td>
							<td>{row.customerName}</td>
							<td>{row.dueOn ?? '—'}</td>
							<td class="num">{yen(row.totalAmount)}</td>
							<td class="num">{yen(row.remainingAmount)}</td>
							<td>
								{#if row.overdue}
									<span class="badge">{m['outstanding.overdue']()}</span>
								{:else if row.settlementStatus === 'PARTIALLY_PAID'}
									{m['outstanding.partiallyPaid']()}
								{:else}
									{m['outstanding.unpaid']()}
								{/if}
							</td>
						</tr>
					{/each}
				</tbody>
			</table>
		</div>
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
		align-items: baseline;
		justify-content: space-between;
		gap: 0.5rem;
	}

	.panel-header h2 {
		margin: 0;
		font-size: 1rem;
		font-weight: 600;
	}

	.summary {
		color: var(--banto-text-muted);
		font-size: 0.85rem;
	}

	.table-scroll {
		overflow-x: auto;
	}

	table {
		width: 100%;
		border-collapse: collapse;
		font-size: 0.85rem;
	}

	th,
	td {
		border-bottom: 1px solid var(--banto-border);
		padding: 0.375rem 0.5rem;
		text-align: left;
	}

	.num {
		text-align: right;
		font-variant-numeric: tabular-nums;
	}

	tr.overdue td {
		color: var(--banto-danger);
	}

	.badge {
		border: 1px solid currentColor;
		border-radius: var(--banto-radius-md);
		padding: 0 0.375rem;
	}

	.note {
		margin: 0;
		font-size: 0.85rem;
	}

	.note--muted {
		color: var(--banto-text-muted);
	}
</style>
