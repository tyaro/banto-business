<script lang="ts">
	/**
	 * 適格請求書のPDF出力（要件 F-I9）。
	 *
	 * **Banto の Report 機能を使う**（`@banto/report` の Markdown テンプレート +
	 * `ReportView` の印刷）。PDF 生成を自前で書かない（CLAUDE.md 第2章）。
	 *
	 * 表示する値は**確定時にスナップショットしたもの**（F-I7）。発行者名・
	 * 登録番号・税率ごとの内訳は Invoice に焼き付いた値をそのまま出す —
	 * 設定を後から変えても、既に発行した請求書の紙面は変わらない。
	 * 振込先だけは Invoice に列が無いため現在の設定を読む（`issuer.rs` の
	 * doc コメント参照）。
	 *
	 * 確定前（Draft）は出力しない。番号も税額もまだ無く、紙にできない。
	 */
	import { page } from '$app/state';
	import { base } from '$app/paths';
	import { getDataProvider, isProviderError } from '@banto/admin-core';
	import { ReportView } from '@banto/report';
	import * as m from '$lib/paraglide/messages';
	import invoiceTemplate from '$lib/banto/reports/invoice.md?raw';
	import { getIssuerSettings, type InvoiceDetail } from '$lib/banto/invoicesAdmin';
	import { sessionStore } from '$lib/session.svelte';
	import EmptyState from '$lib/components/ui/EmptyState.svelte';
	import ErrorState from '$lib/components/ui/ErrorState.svelte';
	import LoadingState from '$lib/components/ui/LoadingState.svelte';

	const rawId = page.params.id ?? '';
	const parsedId = Number(rawId);
	const idValid = rawId !== '' && Number.isInteger(parsedId);

	let detail = $state<InvoiceDetail | null>(null);
	let bankAccount = $state<string | null>(null);
	let loading = $state(true);
	let failed = $state(false);
	let notFound = $state(false);

	const taxLabels: Record<string, () => string> = {
		STANDARD_10: m['invoices.tax10'],
		REDUCED_8: m['invoices.tax8'],
		EXEMPT: m['invoices.taxExempt'],
		OUT_OF_SCOPE: m['invoices.taxOutOfScope']
	};
	const taxLabel = (code: string): string => (taxLabels[code] ?? m['invoices.taxOutOfScope'])();

	async function load() {
		if (!idValid) {
			notFound = true;
			loading = false;
			return;
		}
		loading = true;
		failed = false;
		try {
			detail = await getDataProvider().getOne<InvoiceDetail>('invoices', parsedId);
			// 振込先は admin しか読めない設定なので、権限が無ければ黙って省く
			// （請求書の他の記載は確定済みの値なので出せる）。
			if (sessionStore.role === 'admin') {
				bankAccount = (await getIssuerSettings()).bankAccount;
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

	const reportData = $derived.by(() => {
		if (!detail) return null;
		const hasReduced = detail.lines.some((line) => line.taxCategory === 'REDUCED_8');
		return {
			customerLabel: detail.customerBillingName ?? detail.customerName,
			invoiceNumber: detail.invoiceNumber ?? '',
			issuedOn: detail.issuedOn ?? '',
			closingOn: detail.closingOn ?? '',
			// 支払条件が未設定の顧客の請求書は期日を導出できないまま確定できる
			// （アルファ実使用からのフィードバック、2026-08-27）。空文字だと
			// 表の行が欠けて見えるので、PDF では未設定を明示する。
			dueOn: detail.dueOn ?? m['invoices.dueOnUnsetOnPrint'](),
			totalTaxable: detail.totalTaxable,
			totalTax: detail.totalTax,
			totalAmount: detail.totalAmount,
			lines: detail.lines.map((line) => ({
				itemName: line.itemName,
				// 軽減税率対象には記号を付し、欄外に注記する
				// （tax-calculation.md 第2章のレイアウト上の注意）。
				reducedMark: line.taxCategory === 'REDUCED_8' ? '※' : '',
				quantity: line.quantity,
				unitPrice: line.unitPrice,
				amount: line.amount,
				taxLabel: taxLabel(line.taxCategory)
			})),
			taxSummaries: detail.taxSummaries.map((summary) => ({
				taxLabel: taxLabel(summary.taxCategory),
				taxableAmount: summary.taxableAmount,
				taxAmount: summary.taxAmount
			})),
			hasReduced,
			issuerName: detail.issuerName ?? '',
			issuerRegistrationNumber: detail.issuerRegistrationNumber ?? '',
			issuerAddress: detail.issuerAddress ?? '',
			bankAccount,
			note: detail.note ?? ''
		};
	});
</script>

<div class="page">
	<a class="back-link" href={`${base}/invoices/${rawId}`}>{m['invoices.printBack']()}</a>

	{#if notFound}
		<EmptyState
			title={m['resource.notFoundTitle']({ resource: m['invoices.resourceLabel']() })}
			description={m['resource.notFoundDesc']()}
		/>
	{:else if loading}
		<LoadingState label={m['common.loading']()} />
	{:else if failed || !detail || !reportData}
		<ErrorState title={m['invoices.loadError']()} description={m['resource.loadErrorDesc']()} />
	{:else if detail.status === 'DRAFT'}
		<EmptyState
			title={m['invoices.printDraftTitle']()}
			description={m['invoices.printDraftDesc']()}
		/>
	{:else}
		{#if detail.status === 'CANCELLED'}
			<p class="note note--warn">{m['invoices.printCancelled']()}</p>
		{/if}
		<ReportView
			template={invoiceTemplate}
			data={reportData}
			title={detail.invoiceNumber ?? m['invoices.resourceLabel']()}
		/>
	{/if}
</div>

<style>
	.page {
		display: flex;
		flex-direction: column;
		gap: 1rem;
	}

	.back-link {
		align-self: flex-start;
		font-size: 0.85rem;
	}

	.note {
		margin: 0;
		font-size: 0.85rem;
	}

	.note--warn {
		color: var(--banto-warning);
	}

	/* 帳票以外は印刷しない（ReportView 自身のツールバーは ReportView が隠す）。 */
	@media print {
		.back-link,
		.note {
			display: none;
		}
	}
</style>
