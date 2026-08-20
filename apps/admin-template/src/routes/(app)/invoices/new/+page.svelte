<script lang="ts">
	/**
	 * 請求書の作成（要件 F-I1 / U-3）。
	 *
	 * 顧客と期間を指定して**未請求の工数・経費**を候補に出し、選んだものを
	 * 明細にして Draft を作る。候補は**1件の工数・経費につき1行**で出す —
	 * `invoice_lines.source_id` が単一 id なので、まとめてしまうと確定時に
	 * どの行へ請求済みを立てるか辿れなくなる。まとめたい場合は作成後の明細
	 * 編集で行を整理する。
	 *
	 * **金額計算はフロントでしない**（AGENTS.md 第1章）。候補の金額は
	 * サーバが `floor(分 × 請求単価 ÷ 60)`（決定 B-2）や税抜換算（決定 C-4）で
	 * 確定した値をそのまま使う。請求単価が未設定の案件は金額 0 で出るので、
	 * 案件マスタで単価を設定してから取り込む。
	 */
	import { goto } from '$app/navigation';
	import { base } from '$app/paths';
	import { getDataProvider, isProviderError } from '@banto/admin-core';
	import type { ListParams } from '@banto/admin-core';
	import * as m from '$lib/paraglide/messages';
	import {
		invoiceCandidates,
		type CandidateLine,
		type InvoiceDetail
	} from '$lib/banto/invoicesAdmin';
	import { sessionStore } from '$lib/session.svelte';
	import { canWriteResources } from '$lib/permissions';
	import PageHeader from '$lib/components/ui/PageHeader.svelte';
	import ErrorState from '$lib/components/ui/ErrorState.svelte';
	import LoadingState from '$lib/components/ui/LoadingState.svelte';

	interface CustomerOption {
		id: number;
		code: string;
		name: string;
	}

	const canWrite = $derived(canWriteResources(sessionStore.role));

	let customers = $state<CustomerOption[]>([]);
	let customerId = $state<number | null>(null);
	let from = $state('');
	let to = $state('');
	let candidates = $state<CandidateLine[]>([]);
	let selected = $state<boolean[]>([]);
	let searched = $state(false);
	let loading = $state(true);
	let busy = $state(false);
	let failed = $state(false);
	let errorMessage = $state('');

	async function loadCustomers() {
		loading = true;
		failed = false;
		try {
			const params: ListParams = {
				sort: [],
				filters: [],
				pagination: { offset: 0, limit: 500 }
			};
			const result = await getDataProvider().getList<CustomerOption>('customers', params);
			customers = result.rows;
			customerId = customers[0]?.id ?? null;
		} catch {
			failed = true;
		} finally {
			loading = false;
		}
	}

	$effect(() => {
		void loadCustomers();
	});

	async function search() {
		if (customerId === null) return;
		busy = true;
		errorMessage = '';
		try {
			candidates = await invoiceCandidates(customerId, from, to);
			selected = candidates.map(() => true);
			searched = true;
		} catch (err) {
			errorMessage = messageOf(err);
		} finally {
			busy = false;
		}
	}

	async function create(withCandidates: boolean) {
		if (customerId === null || !canWrite) return;
		busy = true;
		errorMessage = '';
		try {
			const lines = withCandidates
				? candidates
						.filter((_, index) => selected[index])
						.map((candidate) => ({
							projectId: candidate.projectId,
							itemName: candidate.itemName,
							quantity: candidate.quantity,
							unitPrice: candidate.unitPrice,
							taxCategory: candidate.taxCategory,
							sourceType: candidate.sourceType,
							sourceId: candidate.sourceId,
							note: candidate.note
						}))
				: [];
			const created = await getDataProvider().create<InvoiceDetail>('invoices', {
				customerId,
				closingOn: to || null,
				dueOn: null,
				correctedInvoiceId: null,
				note: null,
				lines
			});
			goto(`${base}/invoices/${created.id}`);
		} catch (err) {
			errorMessage = messageOf(err);
			busy = false;
		}
	}

	function messageOf(err: unknown): string {
		if (isProviderError(err)) {
			if (err.body.kind === 'validation') {
				return err.body.field_errors.map((e) => `${e.field}: ${e.message}`).join(' / ');
			}
			// `not_found` などメッセージを持たない種別もあるので、`in` で絞る。
			if ('message' in err.body) return err.body.message;
		}
		return m['invoices.saveError']();
	}

	const yen = (value: number): string => `¥${value.toLocaleString()}`;

	const selectedCount = $derived(selected.filter(Boolean).length);
</script>

<div class="page">
	<PageHeader title={m['invoices.newTitle']()} description={m['invoices.newDescription']()}>
		{#snippet actions()}
			<a class="banto-btn banto-btn--ghost" href={`${base}/invoices`}>
				{m['common.backToList']()}
			</a>
		{/snippet}
	</PageHeader>

	{#if loading}
		<LoadingState label={m['common.loading']()} />
	{:else if failed}
		<ErrorState title={m['invoices.loadError']()} description={m['resource.loadErrorDesc']()} />
	{:else}
		{#if errorMessage}
			<p class="note note--error">{errorMessage}</p>
		{/if}

		<section class="panel">
			<div class="fields">
				<label class="field">
					<span>{m['invoices.fieldCustomerId']()}</span>
					<select class="banto-input" bind:value={customerId}>
						{#each customers as customer (customer.id)}
							<option value={customer.id}>{customer.code} {customer.name}</option>
						{/each}
					</select>
				</label>
				<label class="field">
					<span>{m['invoices.periodFrom']()}</span>
					<input class="banto-input" type="date" bind:value={from} />
				</label>
				<label class="field">
					<span>{m['invoices.periodTo']()}</span>
					<input class="banto-input" type="date" bind:value={to} />
				</label>
			</div>
			<div class="actions">
				<button
					type="button"
					class="banto-btn banto-btn--secondary"
					onclick={search}
					disabled={busy || customerId === null || !from || !to}
				>
					{m['invoices.searchCandidates']()}
				</button>
				{#if canWrite}
					<button
						type="button"
						class="banto-btn banto-btn--ghost"
						onclick={() => void create(false)}
						disabled={busy || customerId === null}
					>
						{m['invoices.createEmpty']()}
					</button>
				{/if}
			</div>
		</section>

		{#if searched}
			<section class="panel">
				<h2>{m['invoices.candidatesTitle']()}</h2>
				{#if candidates.length === 0}
					<p class="note note--muted">{m['invoices.noCandidates']()}</p>
				{:else}
					<div class="table-scroll">
						<table>
							<thead>
								<tr>
									<th></th>
									<th>{m['invoices.colProject']()}</th>
									<th>{m['invoices.colItemName']()}</th>
									<th class="num">{m['invoices.colAmount']()}</th>
									<th>{m['invoices.colNote']()}</th>
								</tr>
							</thead>
							<tbody>
								{#each candidates as candidate, index (candidate.sourceType + candidate.sourceId)}
									<tr
										class:warn={candidate.minutes !== null && candidate.billingHourlyRate === null}
									>
										<td>
											<input type="checkbox" bind:checked={selected[index]} />
										</td>
										<td>{candidate.projectCode} {candidate.projectName}</td>
										<td>{candidate.itemName}</td>
										<td class="num">{yen(candidate.amount)}</td>
										<td class="candidate-note">{candidate.note}</td>
									</tr>
								{/each}
							</tbody>
						</table>
					</div>
					<p class="note note--muted">{m['invoices.candidatesNote']()}</p>
					{#if canWrite}
						<div class="actions">
							<button
								type="button"
								class="banto-btn banto-btn--primary"
								onclick={() => void create(true)}
								disabled={busy || selectedCount === 0}
							>
								{m['invoices.createFromCandidates']({ count: selectedCount })}
							</button>
						</div>
					{/if}
				{/if}
			</section>
		{/if}
	{/if}
</div>

<style>
	.page {
		display: flex;
		flex-direction: column;
		gap: 1rem;
		max-width: 1000px;
	}

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

	.panel h2 {
		margin: 0;
		font-size: 1rem;
		font-weight: 600;
	}

	.fields {
		display: grid;
		grid-template-columns: repeat(auto-fit, minmax(14rem, 1fr));
		gap: 0.75rem;
	}

	.field {
		display: flex;
		flex-direction: column;
		gap: 0.25rem;
		font-size: 0.85rem;
	}

	.actions {
		display: flex;
		gap: 0.5rem;
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

	.candidate-note {
		color: var(--banto-text-muted);
	}

	tr.warn .candidate-note {
		color: var(--banto-warning);
	}

	.note {
		margin: 0;
		font-size: 0.85rem;
	}

	.note--muted {
		color: var(--banto-text-muted);
	}

	.note--error {
		color: var(--banto-danger);
	}
</style>
