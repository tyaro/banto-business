<script lang="ts">
	/** 入金の編集（Phase 6）。画面本体は `PaymentEditor` と共通。 */
	import { page } from '$app/state';
	import { base } from '$app/paths';
	import * as m from '$lib/paraglide/messages';
	import PageHeader from '$lib/components/ui/PageHeader.svelte';
	import PaymentEditor from '$lib/components/business/PaymentEditor.svelte';
	import EmptyState from '$lib/components/ui/EmptyState.svelte';

	// Rust 側のコマンドは `id: i64` を宣言しており、Tauri の serde は文字列を
	// 数値に強制変換しないため、ルートパラメータを数値へ変換してから渡す。
	const rawId = page.params.id ?? '';
	const parsedId = Number(rawId);
	const idValid = rawId !== '' && Number.isInteger(parsedId);
</script>

<div class="page">
	<PageHeader title={m['payments.editTitle']()}>
		{#snippet actions()}
			<a class="banto-btn banto-btn--ghost" href={`${base}/payments`}>
				{m['common.backToList']()}
			</a>
		{/snippet}
	</PageHeader>

	{#if idValid}
		<PaymentEditor paymentId={parsedId} />
	{:else}
		<EmptyState
			title={m['resource.notFoundTitle']({ resource: m['payments.resourceLabel']() })}
			description={m['resource.notFoundDesc']()}
		/>
	{/if}
</div>

<style>
	.page {
		display: flex;
		flex-direction: column;
		gap: 1rem;
		max-width: 1000px;
	}
</style>
