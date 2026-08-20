<script lang="ts">
	/**
	 * 請求書の編集・確定・取消（Phase 5、要件 F-I2〜F-I8）。
	 *
	 * **Draft のときだけ明細を編集できる。** 確定後は編集させない（F-I8）—
	 * 訂正は取消（赤伝）と再発行で行う。この画面はその制約を UI 側でも見せる
	 * （サーバ側でも拒否するので、二重の歯止め）。
	 *
	 * **金額計算はフロントでしない**（AGENTS.md 第1章）。行金額も税額も
	 * サーバが確定した値を表示するだけで、確定前の概算もここでは組み立てない
	 * （端数処理の位置がサーバと食い違うため）。
	 */
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { base } from '$app/paths';
	import { getDataProvider, isProviderError } from '@banto/admin-core';
	import type { ListParams } from '@banto/admin-core';
	import { Plus, Trash2 } from '@lucide/svelte';
	import * as m from '$lib/paraglide/messages';
	import {
		cancelInvoice,
		issueInvoice,
		type InvoiceDetail,
		type InvoiceLineInput
	} from '$lib/banto/invoicesAdmin';
	import { sessionStore } from '$lib/session.svelte';
	import { canWriteResources } from '$lib/permissions';
	import PageHeader from '$lib/components/ui/PageHeader.svelte';
	import EmptyState from '$lib/components/ui/EmptyState.svelte';
	import ErrorState from '$lib/components/ui/ErrorState.svelte';
	import LoadingState from '$lib/components/ui/LoadingState.svelte';

	interface ProjectOption {
		id: number;
		code: string;
		name: string;
		customerId: number;
	}

	const TAX_CATEGORIES = ['STANDARD_10', 'REDUCED_8', 'EXEMPT', 'OUT_OF_SCOPE'] as const;

	const canWrite = $derived(canWriteResources(sessionStore.role));

	const rawId = page.params.id ?? '';
	const parsedId = Number(rawId);
	const idValid = rawId !== '' && Number.isInteger(parsedId);

	interface Settlement {
		settledAmount: number;
		remainingAmount: number;
		settlementStatus: string;
		overdue: boolean;
	}

	let detail = $state<InvoiceDetail | null>(null);
	let settlement = $state<Settlement | null>(null);
	let projects = $state<ProjectOption[]>([]);
	let lines = $state<InvoiceLineInput[]>([]);
	let note = $state('');
	let closingOn = $state('');
	let dueOn = $state('');
	let loading = $state(true);
	let notFound = $state(false);
	let failed = $state(false);
	let saving = $state(false);
	let errorMessage = $state('');

	const isDraft = $derived(detail?.status === 'DRAFT');
	const editable = $derived(canWrite && isDraft);

	function toLineInput(line: InvoiceDetail['lines'][number]): InvoiceLineInput {
		return {
			projectId: line.projectId,
			itemName: line.itemName,
			quantity: line.quantity,
			unitPrice: line.unitPrice,
			taxCategory: line.taxCategory,
			sourceType: line.sourceType,
			sourceId: line.sourceId,
			note: line.note
		};
	}

	async function load() {
		if (!idValid) {
			notFound = true;
			loading = false;
			return;
		}
		loading = true;
		failed = false;
		notFound = false;
		errorMessage = '';
		try {
			const provider = getDataProvider();
			const loaded = await provider.getOne<InvoiceDetail>('invoices', parsedId);
			detail = loaded;
			lines = loaded.lines.map(toLineInput);
			note = loaded.note ?? '';
			closingOn = loaded.closingOn ?? '';
			dueOn = loaded.dueOn ?? '';
			const params: ListParams = {
				sort: [],
				filters: [],
				pagination: { offset: 0, limit: 500 }
			};
			const result = await provider.getList<ProjectOption>('projects', params);
			projects = result.rows.filter((p) => p.customerId === loaded.customerId);
			// 入金状況（Phase 6、要件 F-Y4〜F-Y6）。残額・入金状態・期限超過は
			// すべてサーバ側の導出値で、ここでは日付比較も金額計算もしない。
			settlement =
				loaded.status === 'DRAFT'
					? null
					: await provider.getOne<Settlement>('settlements', parsedId);
		} catch (err) {
			if (isProviderError(err) && err.body.kind === 'not_found') notFound = true;
			else failed = true;
		} finally {
			loading = false;
		}
	}

	$effect(() => {
		void load();
	});

	function addLine() {
		lines = [
			...lines,
			{
				projectId: projects[0]?.id ?? 0,
				itemName: '',
				quantity: 1,
				unitPrice: 0,
				taxCategory: 'STANDARD_10',
				sourceType: 'MANUAL',
				sourceId: null,
				note: null
			}
		];
	}

	function removeLine(index: number) {
		lines = lines.filter((_, i) => i !== index);
	}

	async function save() {
		if (!detail || !editable) return;
		saving = true;
		errorMessage = '';
		try {
			await getDataProvider().update('invoices', detail.id, {
				customerId: detail.customerId,
				closingOn: closingOn || null,
				dueOn: dueOn || null,
				correctedInvoiceId: detail.correctedInvoiceId,
				note: note || null,
				lines
			});
			await load();
		} catch (err) {
			errorMessage = messageOf(err);
		} finally {
			saving = false;
		}
	}

	async function issue() {
		if (!detail || !editable) return;
		if (!window.confirm(m['invoices.issueConfirm']())) return;
		saving = true;
		errorMessage = '';
		try {
			detail = await issueInvoice(detail.id);
			await load();
		} catch (err) {
			errorMessage = messageOf(err);
		} finally {
			saving = false;
		}
	}

	async function cancel() {
		if (!detail || !canWrite) return;
		if (!window.confirm(m['invoices.cancelConfirm']())) return;
		saving = true;
		errorMessage = '';
		try {
			detail = await cancelInvoice(detail.id);
			await load();
		} catch (err) {
			errorMessage = messageOf(err);
		} finally {
			saving = false;
		}
	}

	async function remove() {
		if (!detail || !editable) return;
		if (!window.confirm(m['resource.deleteConfirm']())) return;
		saving = true;
		errorMessage = '';
		try {
			await getDataProvider().deleteOne('invoices', detail.id);
			goto(`${base}/invoices`);
		} catch (err) {
			errorMessage = messageOf(err);
			saving = false;
		}
	}

	/** サーバのエラーをそのまま出す（検証メッセージは日本語で返る）。 */
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

	const yen = (value: number | null | undefined): string =>
		value === null || value === undefined ? '—' : `¥${value.toLocaleString()}`;

	const taxLabel = (code: string): string => {
		switch (code) {
			case 'STANDARD_10':
				return m['invoices.tax10']();
			case 'REDUCED_8':
				return m['invoices.tax8']();
			case 'EXEMPT':
				return m['invoices.taxExempt']();
			default:
				return m['invoices.taxOutOfScope']();
		}
	};

	/** 入金状態の表示名（導出値。invoices.status には持たない）。 */
	const settlementLabel = (status: string): string => {
		switch (status) {
			case 'PAID':
				return m['invoices.settlementPaid']();
			case 'PARTIALLY_PAID':
				return m['outstanding.partiallyPaid']();
			case 'CANCELLED':
				return m['invoices.statusCancelled']();
			default:
				return m['outstanding.unpaid']();
		}
	};

	const statusLabel = (status: string): string => {
		switch (status) {
			case 'DRAFT':
				return m['invoices.statusDraft']();
			case 'ISSUED':
				return m['invoices.statusIssued']();
			default:
				return m['invoices.statusCancelled']();
		}
	};
</script>

<div class="page">
	<PageHeader
		title={detail?.invoiceNumber ?? m['invoices.draftTitle']()}
		description={detail ? `${statusLabel(detail.status)} / ${detail.customerName}` : ''}
	>
		{#snippet actions()}
			<a class="banto-btn banto-btn--ghost" href={`${base}/invoices`}>
				{m['common.backToList']()}
			</a>
			{#if detail && detail.status === 'ISSUED'}
				<a class="banto-btn banto-btn--secondary" href={`${base}/invoices/${detail.id}/print`}>
					{m['invoices.print']()}
				</a>
			{/if}
		{/snippet}
	</PageHeader>

	{#if notFound}
		<EmptyState
			title={m['resource.notFoundTitle']({ resource: m['invoices.resourceLabel']() })}
			description={m['resource.notFoundDesc']()}
		>
			{#snippet action()}
				<a class="banto-btn banto-btn--secondary" href={`${base}/invoices`}>
					{m['common.backToList']()}
				</a>
			{/snippet}
		</EmptyState>
	{:else if loading}
		<LoadingState label={m['common.loading']()} />
	{:else if failed || !detail}
		<ErrorState title={m['invoices.loadError']()} description={m['resource.loadErrorDesc']()} />
	{:else}
		{#if !isDraft}
			<p class="note note--warn">{m['invoices.issuedLocked']()}</p>
		{/if}
		{#if errorMessage}
			<p class="note note--error">{errorMessage}</p>
		{/if}

		<section class="panel">
			<div class="fields">
				<label class="field">
					<span>{m['invoices.fieldClosingOn']()}</span>
					<input class="banto-input" type="date" bind:value={closingOn} disabled={!editable} />
				</label>
				<label class="field">
					<span>{m['invoices.fieldDueOn']()}</span>
					<input class="banto-input" type="date" bind:value={dueOn} disabled={!editable} />
					<small>{m['invoices.dueOnHint']()}</small>
				</label>
				<label class="field field--wide">
					<span>{m['invoices.fieldNote']()}</span>
					<input class="banto-input" type="text" bind:value={note} disabled={!editable} />
				</label>
			</div>
		</section>

		<section class="panel">
			<header class="panel-header">
				<h2>{m['invoices.linesTitle']()}</h2>
				{#if editable}
					<button type="button" class="banto-btn banto-btn--ghost" onclick={addLine}>
						<Plus size={16} />
						{m['invoices.addLine']()}
					</button>
				{/if}
			</header>

			{#if lines.length === 0}
				<p class="note note--muted">{m['invoices.noLines']()}</p>
			{:else}
				<div class="table-scroll">
					<table class="lines">
						<thead>
							<tr>
								<th>{m['invoices.colProject']()}</th>
								<th>{m['invoices.colItemName']()}</th>
								<th class="num">{m['invoices.colQuantity']()}</th>
								<th class="num">{m['invoices.colUnitPrice']()}</th>
								<th class="num">{m['invoices.colAmount']()}</th>
								<th>{m['invoices.colTaxCategory']()}</th>
								<th>{m['invoices.colNote']()}</th>
								{#if editable}<th></th>{/if}
							</tr>
						</thead>
						<tbody>
							{#each lines as line, index (index)}
								<tr>
									<td>
										<select class="banto-input" bind:value={line.projectId} disabled={!editable}>
											{#each projects as project (project.id)}
												<option value={project.id}>{project.code} {project.name}</option>
											{/each}
										</select>
									</td>
									<td>
										<input
											class="banto-input"
											type="text"
											bind:value={line.itemName}
											disabled={!editable}
										/>
									</td>
									<td class="num">
										<input
											class="banto-input num"
											type="number"
											bind:value={line.quantity}
											disabled={!editable}
										/>
									</td>
									<td class="num">
										<input
											class="banto-input num"
											type="number"
											bind:value={line.unitPrice}
											disabled={!editable}
										/>
									</td>
									<td class="num">{yen(detail.lines[index]?.amount)}</td>
									<td>
										<select class="banto-input" bind:value={line.taxCategory} disabled={!editable}>
											{#each TAX_CATEGORIES as code (code)}
												<option value={code}>{taxLabel(code)}</option>
											{/each}
										</select>
									</td>
									<td class="line-note">{line.note ?? ''}</td>
									{#if editable}
										<td>
											<button
												type="button"
												class="banto-btn banto-btn--ghost"
												onclick={() => removeLine(index)}
												aria-label={m['invoices.removeLine']()}
											>
												<Trash2 size={16} />
											</button>
										</td>
									{/if}
								</tr>
							{/each}
						</tbody>
					</table>
				</div>
			{/if}
			<p class="note note--muted">{m['invoices.amountNote']()}</p>
		</section>

		<section class="panel">
			<h2>{m['invoices.totalsTitle']()}</h2>
			{#if detail.taxSummaries.length === 0}
				<p class="note note--muted">{m['invoices.totalsPending']()}</p>
			{:else}
				<table class="totals">
					<thead>
						<tr>
							<th>{m['invoices.colTaxCategory']()}</th>
							<th class="num">{m['invoices.colTaxable']()}</th>
							<th class="num">{m['invoices.colTax']()}</th>
						</tr>
					</thead>
					<tbody>
						{#each detail.taxSummaries as summary (summary.id)}
							<tr>
								<td>{taxLabel(summary.taxCategory)}</td>
								<td class="num">{yen(summary.taxableAmount)}</td>
								<td class="num">{yen(summary.taxAmount)}</td>
							</tr>
						{/each}
					</tbody>
				</table>
			{/if}
			<dl class="grand-totals">
				<div>
					<dt>{m['invoices.totalTaxable']()}</dt>
					<dd>{yen(detail.totalTaxable)}</dd>
				</div>
				<div>
					<dt>{m['invoices.totalTax']()}</dt>
					<dd>{yen(detail.totalTax)}</dd>
				</div>
				<div class="strong">
					<dt>{m['invoices.totalAmount']()}</dt>
					<dd>{yen(detail.totalAmount)}</dd>
				</div>
			</dl>
		</section>

		{#if settlement}
			<section class="panel">
				<h2>{m['invoices.settlementTitle']()}</h2>
				<dl class="grand-totals">
					<div>
						<dt>{m['invoices.settledAmount']()}</dt>
						<dd>{yen(settlement.settledAmount)}</dd>
					</div>
					<div class="strong">
						<dt>{m['outstanding.remaining']()}</dt>
						<dd>{yen(settlement.remainingAmount)}</dd>
					</div>
					<div>
						<dt>{m['invoices.settlementStatus']()}</dt>
						<dd class:overdue={settlement.overdue}>
							{settlement.overdue
								? m['outstanding.overdue']()
								: settlementLabel(settlement.settlementStatus)}
						</dd>
					</div>
				</dl>
				<p class="note note--muted">{m['invoices.settlementNote']()}</p>
			</section>
		{/if}

		{#if canWrite}
			<div class="actions">
				{#if isDraft}
					<button
						type="button"
						class="banto-btn banto-btn--secondary"
						onclick={save}
						disabled={saving}
					>
						{m['common.save']()}
					</button>
					<button
						type="button"
						class="banto-btn banto-btn--primary"
						onclick={issue}
						disabled={saving || lines.length === 0}
					>
						{m['invoices.issue']()}
					</button>
					<button
						type="button"
						class="banto-btn banto-btn--danger"
						onclick={remove}
						disabled={saving}
					>
						{m['common.delete']()}
					</button>
				{:else if detail.status === 'ISSUED'}
					<button
						type="button"
						class="banto-btn banto-btn--danger"
						onclick={cancel}
						disabled={saving}
					>
						{m['invoices.cancel']()}
					</button>
				{/if}
			</div>
		{/if}
	{/if}
</div>

<style>
	.page {
		display: flex;
		flex-direction: column;
		gap: 1rem;
		max-width: 1100px;
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

	.panel-header {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 0.5rem;
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

	.field--wide {
		grid-column: 1 / -1;
	}

	.field small {
		color: var(--banto-text-muted);
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
		vertical-align: middle;
	}

	.num {
		text-align: right;
		font-variant-numeric: tabular-nums;
	}

	.line-note {
		color: var(--banto-text-muted);
		max-width: 16rem;
	}

	.grand-totals {
		display: flex;
		flex-wrap: wrap;
		gap: 1.5rem;
		margin: 0;
	}

	.grand-totals div {
		display: flex;
		flex-direction: column;
		gap: 0.125rem;
	}

	.grand-totals dt {
		color: var(--banto-text-muted);
		font-size: 0.85rem;
	}

	.grand-totals dd {
		margin: 0;
		font-variant-numeric: tabular-nums;
	}

	.overdue {
		color: var(--banto-danger);
	}

	.grand-totals .strong dd {
		font-size: 1.15rem;
		font-weight: 600;
	}

	.actions {
		display: flex;
		gap: 0.5rem;
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

	.note--error {
		color: var(--banto-danger);
	}
</style>
