<script lang="ts">
	/**
	 * expenses の編集・削除（docs/recipes/add-resource.md 手順8）。
	 *
	 * 領収書の添付欄を持つ（要件 F-E3）。ここに置く領収書は**案件へ紐付ける
	 * ための参照コピーであって正本ではない**（CLAUDE.md 1.6）。正本は会計ソフト
	 * 側にあり、電子帳簿保存法の検索要件・訂正削除履歴はこちらでは満たさない。
	 */
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import { base } from '$app/paths';
	import { BantoForm, createFormStore } from '@banto/forms';
	import type { FormSchema } from '@banto/forms';
	import { createFormResource, getResource, isProviderError } from '@banto/admin-core';
	import { AttachmentsPanel } from '@banto/attachments';
	import * as m from '$lib/paraglide/messages';
	import {
		loadProjectOptions,
		loadExpenseCategoryOptions,
		expenseCategoryDefaultTaxOf
	} from '$lib/banto/referenceOptions.svelte';
	import { normalizeFormValues } from '$lib/banto/formValues';
	import { ExpenseTaxCategoryTracker } from '$lib/banto/expenseTaxDefault';
	import { isAttachmentsAvailable } from '$lib/banto/attachmentsAdmin';
	import { attachmentsClient } from '$lib/banto/attachmentsClient';
	import { formValidationMessages } from '$lib/banto/i18n';
	import { sessionStore } from '$lib/session.svelte';
	import { canWriteResources } from '$lib/permissions';
	import PageHeader from '$lib/components/ui/PageHeader.svelte';
	import EmptyState from '$lib/components/ui/EmptyState.svelte';
	import ErrorState from '$lib/components/ui/ErrorState.svelte';
	import LoadingState from '$lib/components/ui/LoadingState.svelte';

	const resource = getResource('expenses');
	const schema = resource.schema as FormSchema;

	// spec M10 RBAC: viewer は閲覧できるが保存・削除はできない。
	const canWrite = $derived(canWriteResources(sessionStore.role));

	// Rust 側のコマンドは `id: i64` を宣言しており、Tauri の serde は文字列を
	// 数値に強制変換しないため、ルートパラメータ（常に文字列）を数値へ変換
	// してから DataProvider へ渡す。
	const rawId = page.params.id ?? '';
	const parsedId = Number(rawId);
	const idValid = rawId !== '' && Number.isInteger(parsedId);

	const formResource = idValid ? createFormResource(resource.name, parsedId) : null;
	let store = $state(createFormStore(schema, undefined, formValidationMessages()));
	let storeReady = $state(false);
	// P2-2: ロード直後は上書きしない — 「最後に見た分類コード」を読み込んだ
	// 行の値で初期化することで、store を作り直した直後の1回目の同期を
	// no-op にする（`docs/mobile-ui-plan.md`、ヘルパの doc を参照）。
	let taxTracker = new ExpenseTaxCategoryTracker('');

	async function loadForm() {
		if (!formResource) return;
		// 案件・経費分類の選択肢（`referenceOptions`）。画面を開くたびに
		// 読み直す —— 起動時に1度だけだと、直後に作った値が出てこない。
		void loadProjectOptions();
		void loadExpenseCategoryOptions();
		await formResource.load();
		if (formResource.initialValues) {
			store = createFormStore(schema, formResource.initialValues, formValidationMessages());
			const loadedCode = String(formResource.initialValues.expenseCategoryCode ?? '');
			taxTracker = new ExpenseTaxCategoryTracker(loadedCode);
			storeReady = true;
		}
	}

	$effect(() => {
		void loadForm();
	});

	$effect(() => {
		if (!storeReady) return;
		const code = String(store.values.expenseCategoryCode ?? '');
		const defaultTax = taxTracker.sync(code, expenseCategoryDefaultTaxOf);
		if (defaultTax !== null) store.setValue('taxCategory', defaultTax);
	});

	const isNotFoundError = $derived.by(() => {
		if (!idValid) return true;
		const err = formResource?.error;
		return isProviderError(err) && err.body.kind === 'not_found';
	});

	async function handleSubmit(values: Record<string, unknown>) {
		if (!formResource || !canWrite) return;
		const result = await formResource.submit(normalizeFormValues(schema, values));
		if (result.ok) {
			goto(`${base}/expenses`);
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
		if (removed) goto(`${base}/expenses`);
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
					<a class="banto-btn banto-btn--secondary" href={`${base}/expenses`}
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
						<a class="banto-btn banto-btn--ghost" href={`${base}/expenses`}
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

	<!--
		領収書の添付（要件 F-E3）。レコードが読めた後にだけ出す（`storeReady` は
		`idValid` を含むが、読みやすさのため明示しておく）。まだ存在しない
		／読めなかったレコードに対して一覧を取りに行かせない。
		デモモードでは丸ごと隠す（`isAttachmentsAvailable()` が false）。
		「利用できません」と出すより、フォームの下に何も無い方が壊れて見えない。
	-->
	{#if idValid && storeReady && isAttachmentsAvailable()}
		<AttachmentsPanel
			client={attachmentsClient}
			resource="expenses"
			resourceId={String(parsedId)}
			{canWrite}
		/>
	{/if}
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
