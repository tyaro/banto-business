<script lang="ts">
	/**
	 * 入金の登録・編集と消込（Phase 6、要件 F-Y1〜F-Y4）。新規と編集で同じ画面を
	 * 使う（違いは初期値と保存先だけなので、2つ書くと消込のルール表示がずれる）。
	 *
	 * **金額計算はフロントでしない**（AGENTS.md 第1章）。残額も入金状態も
	 * サーバが導出した値を表示するだけ。充当額の入力欄に「残額を入れる」補助は
	 * 付けるが、それは入力の写し取りであって計算ではない。
	 *
	 * 充当先は**その顧客の未入金の確定済み請求書**だけを出す。Draft は請求して
	 * いないので充当できず、取消済みも同様（サーバ側でも拒否する）。
	 */
	import { goto } from '$app/navigation';
	import { base } from '$app/paths';
	import { getDataProvider, isProviderError } from '@banto/admin-core';
	import type { ListParams } from '@banto/admin-core';
	import { Plus, Trash2 } from '@lucide/svelte';
	import * as m from '$lib/paraglide/messages';
	import LoadingState from '$lib/components/ui/LoadingState.svelte';
	import ErrorState from '$lib/components/ui/ErrorState.svelte';

	interface CustomerOption {
		id: number;
		code: string;
		name: string;
	}

	interface Settlement {
		invoiceId: number;
		invoiceNumber: string | null;
		customerId: number;
		dueOn: string | null;
		remainingAmount: number;
	}

	interface AllocationInput {
		invoiceId: number;
		allocatedAmount: number;
		differenceReason: string | null;
		differenceAmount: number;
		note: string | null;
	}

	interface PaymentDetail {
		id: number;
		customerId: number;
		paidOn: string;
		amount: number;
		method: string | null;
		note: string | null;
		unallocatedAmount: number;
		allocations: (AllocationInput & { id: number })[];
	}

	interface Props {
		/** 既存の入金を編集するなら id。新規なら null。 */
		paymentId: number | null;
	}

	const { paymentId }: Props = $props();

	const DIFFERENCE_REASONS = [
		'TRANSFER_FEE',
		'WITHHOLDING',
		'DISCOUNT',
		'OVERPAYMENT',
		'OTHER'
	] as const;

	let customers = $state<CustomerOption[]>([]);
	let outstanding = $state<Settlement[]>([]);
	let customerId = $state<number | null>(null);
	let paidOn = $state('');
	let amount = $state(0);
	let method = $state('');
	let note = $state('');
	let allocations = $state<AllocationInput[]>([]);
	let unallocated = $state<number | null>(null);
	let loading = $state(true);
	let failed = $state(false);
	let notFound = $state(false);
	let saving = $state(false);
	let errorMessage = $state('');

	/** この顧客の未入金請求書（充当先の候補）。 */
	const invoiceOptions = $derived(
		outstanding.filter((row) => customerId !== null && row.customerId === customerId)
	);

	async function load() {
		loading = true;
		failed = false;
		notFound = false;
		errorMessage = '';
		try {
			const provider = getDataProvider();
			const params: ListParams = {
				sort: [],
				filters: [],
				pagination: { offset: 0, limit: 500 }
			};
			const customerList = await provider.getList<CustomerOption>('customers', params);
			customers = customerList.rows;
			const outstandingList = await provider.getList<Settlement>('outstanding', params);
			outstanding = outstandingList.rows;

			if (paymentId === null) {
				customerId = customers[0]?.id ?? null;
				allocations = [];
				unallocated = null;
			} else {
				const detail = await provider.getOne<PaymentDetail>('payments', paymentId);
				customerId = detail.customerId;
				paidOn = detail.paidOn;
				amount = detail.amount;
				method = detail.method ?? '';
				note = detail.note ?? '';
				unallocated = detail.unallocatedAmount;
				allocations = detail.allocations.map((allocation) => ({
					invoiceId: allocation.invoiceId,
					allocatedAmount: allocation.allocatedAmount,
					differenceReason: allocation.differenceReason,
					differenceAmount: allocation.differenceAmount,
					note: allocation.note
				}));
			}
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

	function addAllocation() {
		const first = invoiceOptions[0];
		allocations = [
			...allocations,
			{
				invoiceId: first?.invoiceId ?? 0,
				// 残額をそのまま写す（入力の手間を省くだけで、計算ではない）。
				allocatedAmount: first?.remainingAmount ?? 0,
				differenceReason: null,
				differenceAmount: 0,
				note: null
			}
		];
	}

	function removeAllocation(index: number) {
		allocations = allocations.filter((_, i) => i !== index);
	}

	function remainingOf(invoiceId: number): number | null {
		return outstanding.find((row) => row.invoiceId === invoiceId)?.remainingAmount ?? null;
	}

	async function save() {
		if (customerId === null) return;
		saving = true;
		errorMessage = '';
		const values = {
			customerId,
			paidOn,
			amount,
			method: method || null,
			note: note || null,
			allocations
		};
		try {
			const provider = getDataProvider();
			if (paymentId === null) {
				const created = await provider.create<{ id: number }>('payments', values);
				goto(`${base}/payments/${created.id}`);
				return;
			}
			await provider.update('payments', paymentId, values);
			await load();
		} catch (err) {
			errorMessage = messageOf(err);
		} finally {
			saving = false;
		}
	}

	async function remove() {
		if (paymentId === null) return;
		if (!window.confirm(m['resource.deleteConfirm']())) return;
		saving = true;
		errorMessage = '';
		try {
			await getDataProvider().deleteOne('payments', paymentId);
			goto(`${base}/payments`);
		} catch (err) {
			errorMessage = messageOf(err);
			saving = false;
		}
	}

	function messageOf(err: unknown): string {
		if (isProviderError(err)) {
			if (err.body.kind === 'validation') {
				return err.body.field_errors.map((e) => `${e.field}: ${e.message}`).join(' / ');
			}
			if ('message' in err.body) return err.body.message;
		}
		return m['payments.saveError']();
	}

	const yen = (value: number | null): string =>
		value === null ? '—' : `¥${value.toLocaleString()}`;

	const reasonLabel = (code: string): string => {
		switch (code) {
			case 'TRANSFER_FEE':
				return m['payments.reasonTransferFee']();
			case 'WITHHOLDING':
				return m['payments.reasonWithholding']();
			case 'DISCOUNT':
				return m['payments.reasonDiscount']();
			case 'OVERPAYMENT':
				return m['payments.reasonOverpayment']();
			default:
				return m['payments.reasonOther']();
		}
	};
</script>

{#if notFound}
	<ErrorState
		title={m['resource.notFoundTitle']({ resource: m['payments.resourceLabel']() })}
		description={m['resource.notFoundDesc']()}
	/>
{:else if loading}
	<LoadingState label={m['common.loading']()} />
{:else if failed}
	<ErrorState title={m['payments.loadError']()} description={m['resource.loadErrorDesc']()} />
{:else}
	{#if errorMessage}
		<p class="note note--error">{errorMessage}</p>
	{/if}

	<section class="panel">
		<div class="fields">
			<label class="field">
				<span>{m['payments.fieldCustomerId']()}</span>
				<select class="banto-input" bind:value={customerId}>
					{#each customers as customer (customer.id)}
						<option value={customer.id}>{customer.code} {customer.name}</option>
					{/each}
				</select>
			</label>
			<label class="field">
				<span>{m['payments.fieldPaidOn']()}</span>
				<input class="banto-input" type="date" bind:value={paidOn} />
			</label>
			<label class="field">
				<span>{m['payments.fieldAmount']()}</span>
				<input class="banto-input num" type="number" bind:value={amount} />
			</label>
			<label class="field">
				<span>{m['payments.fieldMethod']()}</span>
				<input class="banto-input" type="text" bind:value={method} />
			</label>
			<label class="field field--wide">
				<span>{m['payments.fieldNote']()}</span>
				<input class="banto-input" type="text" bind:value={note} />
			</label>
		</div>
		{#if unallocated !== null && unallocated !== 0}
			<p class="note note--warn">{m['payments.unallocated']({ amount: yen(unallocated) })}</p>
		{/if}
	</section>

	<section class="panel">
		<header class="panel-header">
			<h2>{m['payments.allocationsTitle']()}</h2>
			<button
				type="button"
				class="banto-btn banto-btn--ghost"
				onclick={addAllocation}
				disabled={invoiceOptions.length === 0}
			>
				<Plus size={16} />
				{m['payments.addAllocation']()}
			</button>
		</header>

		{#if invoiceOptions.length === 0 && allocations.length === 0}
			<p class="note note--muted">{m['payments.noOpenInvoices']()}</p>
		{/if}

		{#if allocations.length > 0}
			<div class="table-scroll">
				<table>
					<thead>
						<tr>
							<th>{m['payments.colInvoice']()}</th>
							<th class="num">{m['payments.colRemaining']()}</th>
							<th class="num">{m['payments.colAllocated']()}</th>
							<th>{m['payments.colReason']()}</th>
							<th class="num">{m['payments.colDifference']()}</th>
							<th>{m['invoices.colNote']()}</th>
							<th></th>
						</tr>
					</thead>
					<tbody>
						{#each allocations as allocation, index (index)}
							<tr>
								<td>
									<select class="banto-input" bind:value={allocation.invoiceId}>
										{#each invoiceOptions as option (option.invoiceId)}
											<option value={option.invoiceId}>
												{option.invoiceNumber ?? option.invoiceId}
											</option>
										{/each}
									</select>
								</td>
								<td class="num">{yen(remainingOf(allocation.invoiceId))}</td>
								<td class="num">
									<input
										class="banto-input num"
										type="number"
										bind:value={allocation.allocatedAmount}
									/>
								</td>
								<td>
									<select class="banto-input" bind:value={allocation.differenceReason}>
										<option value={null}>{m['payments.reasonNone']()}</option>
										{#each DIFFERENCE_REASONS as reason (reason)}
											<option value={reason}>{reasonLabel(reason)}</option>
										{/each}
									</select>
								</td>
								<td class="num">
									<input
										class="banto-input num"
										type="number"
										bind:value={allocation.differenceAmount}
									/>
								</td>
								<td>
									<input class="banto-input" type="text" bind:value={allocation.note} />
								</td>
								<td>
									<button
										type="button"
										class="banto-btn banto-btn--ghost"
										onclick={() => removeAllocation(index)}
										aria-label={m['payments.removeAllocation']()}
									>
										<Trash2 size={16} />
									</button>
								</td>
							</tr>
						{/each}
					</tbody>
				</table>
			</div>
		{/if}
		<p class="note note--muted">{m['payments.differenceNote']()}</p>
	</section>

	<div class="actions">
		<button type="button" class="banto-btn banto-btn--primary" onclick={save} disabled={saving}>
			{m['common.save']()}
		</button>
		{#if paymentId !== null}
			<button type="button" class="banto-btn banto-btn--danger" onclick={remove} disabled={saving}>
				{m['common.delete']()}
			</button>
		{/if}
	</div>
{/if}

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
		grid-template-columns: repeat(auto-fit, minmax(12rem, 1fr));
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
