<script lang="ts">
	/**
	 * 内部原価レートの設定（要件 F-M5）。
	 *
	 * **このレートは新規入力時の既定値の供給源でしかない**（CLAUDE.md 1.2）。
	 * 変更しても、既に記録した工数の原価は動かない — 各行が記録時点の単価を
	 * 焼き付けているため。その旨を画面にも明記する（設定した本人が
	 * 「過去の採算が変わるのでは」と迷わないように）。
	 *
	 * 読み書きとも**標準の DataProvider 経由**（`work_categories` の一覧と
	 * `cost_rates` の更新）。専用の呼び出し口を作らないのは、Banto が用意した
	 * 経路を使うため（CLAUDE.md 第2章）。作業分類は固定コード表なので、
	 * サーバ側は `ListParams` を受け取っても絞り込みには使わない。
	 */
	import { getDataProvider } from '@banto/admin-core';
	import type { ListParams } from '@banto/admin-core';
	import * as m from '$lib/paraglide/messages';
	import { sessionStore } from '$lib/session.svelte';
	import { canWriteResources } from '$lib/permissions';
	import PageHeader from '$lib/components/ui/PageHeader.svelte';
	import LoadingState from '$lib/components/ui/LoadingState.svelte';
	import ErrorState from '$lib/components/ui/ErrorState.svelte';

	interface WorkCategory {
		code: string;
		name: string;
		excludedFromEffectiveRate: number;
		hourlyRate: number | null;
	}

	const canWrite = $derived(canWriteResources(sessionStore.role));

	let categories = $state<WorkCategory[]>([]);
	let drafts = $state<Record<string, string>>({});
	let loading = $state(true);
	let failed = $state(false);
	let savingCode = $state<string | null>(null);

	async function load() {
		loading = true;
		failed = false;
		try {
			const provider = getDataProvider();
			const params: ListParams = { sort: [], filters: [], pagination: { offset: 0, limit: 100 } };
			const result = await provider.getList<WorkCategory>('work_categories', params);
			categories = result.rows;
			drafts = Object.fromEntries(
				categories.map((c) => [c.code, c.hourlyRate === null ? '' : String(c.hourlyRate)])
			);
		} catch {
			failed = true;
		} finally {
			loading = false;
		}
	}

	$effect(() => {
		void load();
	});

	async function save(code: string) {
		if (!canWrite) return;
		savingCode = code;
		try {
			const provider = getDataProvider();
			await provider.update('cost_rates', code, { hourlyRate: Number(drafts[code] ?? 0) });
			await load();
		} finally {
			savingCode = null;
		}
	}
</script>

<div class="page">
	<PageHeader title={m['costRates.title']()} description={m['costRates.description']()} />

	<p class="note">{m['costRates.snapshotNote']()}</p>

	<div class="panel">
		{#if loading}
			<LoadingState label={m['common.loading']()} />
		{:else if failed}
			<ErrorState title={m['resource.loadError']()} description={m['resource.loadErrorDesc']()}>
				{#snippet action()}
					<button type="button" class="banto-btn banto-btn--secondary" onclick={() => void load()}>
						{m['common.reload']()}
					</button>
				{/snippet}
			</ErrorState>
		{:else}
			<table>
				<thead>
					<tr>
						<th>{m['costRates.colCategory']()}</th>
						<th>{m['costRates.colExcluded']()}</th>
						<th>{m['costRates.colRate']()}</th>
						<th></th>
					</tr>
				</thead>
				<tbody>
					{#each categories as category (category.code)}
						<tr>
							<td>{category.name}</td>
							<td class="muted">
								{category.excludedFromEffectiveRate === 1
									? m['costRates.excludedYes']()
									: m['costRates.excludedNo']()}
							</td>
							<td>
								<input
									type="number"
									min="0"
									max="1000000"
									disabled={!canWrite}
									placeholder={m['costRates.unset']()}
									bind:value={drafts[category.code]}
								/>
							</td>
							<td>
								{#if canWrite}
									<button
										type="button"
										class="banto-btn banto-btn--secondary"
										disabled={savingCode === category.code}
										onclick={() => void save(category.code)}
									>
										{m['costRates.save']()}
									</button>
								{/if}
							</td>
						</tr>
					{/each}
				</tbody>
			</table>
		{/if}
	</div>
</div>

<style>
	.page {
		display: flex;
		flex-direction: column;
		gap: 1rem;
		max-width: 900px;
	}

	.note {
		color: var(--banto-text-muted);
		font-size: 0.85rem;
		margin: 0;
	}

	.panel {
		background: var(--banto-surface);
		border: 1px solid var(--banto-border);
		border-radius: var(--banto-radius-lg);
		box-shadow: var(--banto-shadow-sm);
		padding: 1rem;
	}

	table {
		border-collapse: collapse;
		width: 100%;
	}

	th,
	td {
		border-bottom: 1px solid var(--banto-border);
		padding: 0.5rem 0.75rem;
		text-align: left;
	}

	th {
		color: var(--banto-text-muted);
		font-size: 0.85rem;
		font-weight: 600;
	}

	.muted {
		color: var(--banto-text-muted);
		font-size: 0.85rem;
	}

	input[type='number'] {
		background: var(--banto-surface);
		border: 1px solid var(--banto-border);
		border-radius: var(--banto-radius-sm);
		color: var(--banto-text);
		padding: 0.35rem 0.5rem;
		width: 10rem;
	}
</style>
