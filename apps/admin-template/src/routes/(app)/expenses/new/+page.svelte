<script lang="ts">
	/**
	 * expenses の新規作成（docs/recipes/add-resource.md 手順8）。
	 * リソース定義のスキーマをそのまま `BantoForm` に
	 * 渡し、保存は `createFormResource` 経由で DataProvider に投げる。
	 */
	import { goto } from '$app/navigation';
	import { base } from '$app/paths';
	import { BantoForm, createFormStore } from '@banto/forms';
	import type { FormSchema } from '@banto/forms';
	import { createFormResource, getResource } from '@banto/admin-core';
	import * as m from '$lib/paraglide/messages';
	import {
		loadProjectOptions,
		loadExpenseCategoryOptions
	} from '$lib/banto/referenceOptions.svelte';
	import { normalizeFormValues } from '$lib/banto/formValues';
	import { formValidationMessages } from '$lib/banto/i18n';
	import PageHeader from '$lib/components/ui/PageHeader.svelte';
	import LoadingState from '$lib/components/ui/LoadingState.svelte';

	const resource = getResource('expenses');
	const schema = resource.schema as FormSchema;

	const formResource = createFormResource('expenses');
	// i18n layer ② (ADR-0005): Paraglide 由来の検証メッセージを注入する。
	const store = createFormStore(schema, undefined, formValidationMessages());

	// 案件・経費分類の選択肢（`referenceOptions`）。画面を開くたびに
	// 読み直す —— 起動時に1度だけだと、直後に作った値が出てこない。
	$effect(() => {
		void loadProjectOptions();
		void loadExpenseCategoryOptions();
		void formResource.load();
	});

	async function handleSubmit(values: Record<string, unknown>) {
		const result = await formResource.submit(normalizeFormValues(schema, values));
		if (result.ok) {
			// 一覧ではなく作った行の編集画面へ（docs/mobile-ui-plan.md P1-2）。
			// 添付パネルは編集画面にしかないので、レシートをもらった場で
			// 「入力 → その場で撮って添付」が 1 本の流れになる。
			const id = (result.row as { id?: unknown }).id;
			goto(typeof id === 'number' ? `${base}/expenses/${id}` : `${base}/expenses`);
		} else {
			// サーバ側（Rust）の検証エラーを該当の入力欄へ戻す。フィールド名を
			// Rust の Input 構造体と揃えてあるのはこのため。
			store.setServerErrors(result.fieldErrors);
		}
	}
</script>

<div class="page">
	<PageHeader title={m['resource.createTitle']({ resource: resource.label })} />

	<div class="form-panel">
		{#if formResource.loading}
			<LoadingState label={m['common.loading']()} />
		{:else}
			<BantoForm
				{schema}
				{store}
				onSubmit={handleSubmit}
				submitting={formResource.saving}
				submitLabel={m['common.save']()}
			/>
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
</style>
