<script lang="ts">
	/**
	 * work-logs の編集・削除（docs/recipes/add-resource.md 手順8）。
	 * `items/[id]` と同じ形。添付ファイル欄は持たない（Phase 3 で経費に
	 * 領収書を付ける段階まで用途が無いため）。
	 */
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { base } from '$app/paths';
	import { BantoForm, createFormStore } from '@banto/forms';
	import type { FormSchema } from '@banto/forms';
	import { createFormResource, getResource, isProviderError } from '@banto/admin-core';
	import * as m from '$lib/paraglide/messages';
	import { formValidationMessages } from '$lib/banto/i18n';
	import { sessionStore } from '$lib/session.svelte';
	import { canWriteResources } from '$lib/permissions';
	import PageHeader from '$lib/components/ui/PageHeader.svelte';
	import EmptyState from '$lib/components/ui/EmptyState.svelte';
	import ErrorState from '$lib/components/ui/ErrorState.svelte';
	import LoadingState from '$lib/components/ui/LoadingState.svelte';

	const resource = getResource('work_logs');
	const schema = resource.schema as FormSchema;

	// spec M10 RBAC: viewer は閲覧できるが保存・削除はできない。
	const canWrite = $derived(canWriteResources(sessionStore.role));

	// Rust 側のコマンドは `id: i64` を宣言しており、Tauri の serde は文字列を
	// 数値に強制変換しないため、ルートパラメータ（常に文字列）を数値へ変換
	// してから DataProvider へ渡す（`items/[id]` と同じ理由）。
	const rawId = page.params.id ?? '';
	const parsedId = Number(rawId);
	const idValid = rawId !== '' && Number.isInteger(parsedId);

	const formResource = idValid ? createFormResource(resource.name, parsedId) : null;
	let store = $state(createFormStore(schema, undefined, formValidationMessages()));
	let storeReady = $state(false);

	async function loadForm() {
		if (!formResource) return;
		await formResource.load();
		if (formResource.initialValues) {
			store = createFormStore(schema, formResource.initialValues, formValidationMessages());
			storeReady = true;
		}
	}

	$effect(() => {
		void loadForm();
	});

	const isNotFoundError = $derived.by(() => {
		if (!idValid) return true;
		const err = formResource?.error;
		return isProviderError(err) && err.body.kind === 'not_found';
	});

	async function handleSubmit(values: Record<string, unknown>) {
		if (!formResource || !canWrite) return;
		const result = await formResource.submit(values);
		if (result.ok) {
			goto(`${base}/work-logs`);
		} else {
			store.setServerErrors(result.fieldErrors);
		}
	}

	async function handleDelete() {
		if (!formResource || !canWrite) return;
		if (!window.confirm(m['resource.deleteConfirm']())) return;
		const removed = await formResource.remove();
		// 削除が拒否される場合がある（例: 案件が紐づく顧客）。その場合は
		// 一覧へ遷移せずページに留まり、エラー表示をそのまま見せる。
		if (removed) goto(`${base}/work-logs`);
	}
</script>

<div class="page">
	<PageHeader title={m['resource.editTitle']({ resource: resource.label })} />

	<div class="form-panel">
		{#if isNotFoundError}
			<EmptyState
				title={m['resource.notFoundTitle']({ resource: resource.label })}
				description={m['resource.notFoundDesc']()}
			>
				{#snippet action()}
					<a class="banto-btn banto-btn--secondary" href={`${base}/work-logs`}
						>{m['common.backToList']()}</a
					>
				{/snippet}
			</EmptyState>
		{:else if formResource?.loading}
			<LoadingState label={m['common.loading']()} />
		{:else if formResource?.error}
			<ErrorState title={m['resource.loadError']()} description={m['resource.loadErrorDesc']()}>
				{#snippet action()}
					<div class="error-actions">
						<button
							type="button"
							class="banto-btn banto-btn--secondary"
							onclick={() => void loadForm()}
						>
							{m['common.reload']()}
						</button>
						<a class="banto-btn banto-btn--ghost" href={`${base}/work-logs`}
							>{m['common.backToList']()}</a
						>
					</div>
				{/snippet}
			</ErrorState>
		{:else if storeReady}
			<BantoForm
				{schema}
				{store}
				onSubmit={handleSubmit}
				submitting={(formResource?.saving ?? false) || !canWrite}
				submitLabel={m['common.save']()}
			>
				{#if canWrite}
					<button type="button" class="banto-btn banto-btn--danger" onclick={handleDelete}>
						{m['common.delete']()}
					</button>
				{/if}
			</BantoForm>
		{/if}
	</div>
</div>

<style>
	.page {
		display: flex;
		flex-direction: column;
		gap: 1rem;
		max-width: 720px;
	}

	.form-panel {
		background: var(--banto-surface);
		border: 1px solid var(--banto-border);
		border-radius: var(--banto-radius-lg);
		box-shadow: var(--banto-shadow-sm);
		padding: 1.25rem;
	}

	.error-actions {
		display: flex;
		align-items: center;
		gap: 0.5rem;
	}
</style>
