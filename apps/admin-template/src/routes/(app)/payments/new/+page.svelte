<script lang="ts">
	/** 入金の登録（Phase 6）。画面本体は `PaymentEditor` と共通。 */
	import { base } from '$app/paths';
	import * as m from '$lib/paraglide/messages';
	import PageHeader from '$lib/components/ui/PageHeader.svelte';
	import PaymentEditor from '$lib/components/business/PaymentEditor.svelte';
</script>

<div class="page">
	<PageHeader title={m['payments.newTitle']()} description={m['payments.newDescription']()}>
		{#snippet actions()}
			<a class="banto-btn banto-btn--ghost" href={`${base}/payments`}>
				{m['common.backToList']()}
			</a>
		{/snippet}
	</PageHeader>

	<PaymentEditor paymentId={null} />
</div>

<style>
	.page {
		display: flex;
		flex-direction: column;
		gap: 1rem;
		max-width: 1000px;
	}
</style>
