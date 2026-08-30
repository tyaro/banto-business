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
	 *
	 * ## 一括保存（アルファ実機フィードバック、2026-08-30）
	 *
	 * 行ごとの保存ボタンは廃止し、変更した行をまとめて1回で送る。理由は
	 * 単純に「数分類ぶん変えるたびに何回も保存ボタンを押すのが面倒」という
	 * 実機での指摘。dirty 判定は各行の draft と読み込み直後の値
	 * （`categories` は保存後まで動かない）の突き合わせだけで済むので、
	 * 追加の「変更前スナップショット」state は持たない。
	 *
	 * 空欄（未設定に戻す）は**現状バックエンドが受けられない** ——
	 * `CostRateValues.hourly_rate` / `CostRateInput.hourly_rate` が
	 * `i64`（非 Optional）で、`set_cost_rate` は常に upsert のみを行い、
	 * 行を消す（未設定に戻す）経路が無い（`core/src/masters.rs`）。そのため
	 * 空欄の dirty 行は保存前にブロックし、専用のエラーメッセージを出す。
	 * null を受けられるようになったら、ここのバリデーションを外し
	 * 「空欄 → 行削除」に置き換えること。
	 */
	import { getDataProvider, isProviderError, notify } from '@banto/admin-core';
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
	let saving = $state(false);
	let savingCode = $state<string | null>(null);
	/** 行単位のバリデーション/保存エラー（分類コード → メッセージ）。 */
	let rowErrors = $state<Record<string, string>>({});
	/** 保存処理そのものが（バリデーション以外で）落ちたときの画面全体向けメッセージ。 */
	let saveError = $state('');

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
			rowErrors = {};
			saveError = '';
		} catch {
			failed = true;
		} finally {
			loading = false;
		}
	}

	$effect(() => {
		void load();
	});

	/** 読み込み直後の値と比べて変わっている行。 */
	const dirtyCategories = $derived(
		categories.filter((c) => drafts[c.code] !== (c.hourlyRate === null ? '' : String(c.hourlyRate)))
	);

	/**
	 * 行1件分のバリデーション。CLAUDE.md 1.1（金額は整数のみ）に従い、
	 * 小数・負値・NaN は弾く。空欄は「未設定に戻す」の意味だが、
	 * バックエンドが null を受けられないため専用メッセージでブロックする
	 * （上部の doc コメント参照）。
	 */
	function validateDraft(raw: string): string | null {
		const trimmed = raw.trim();
		if (trimmed === '') return m['costRates.cannotUnset']();
		if (!/^\d+$/.test(trimmed)) return m['costRates.invalidValue']();
		return null;
	}

	async function saveAll() {
		if (!canWrite || saving) return;
		const dirty = dirtyCategories;
		if (dirty.length === 0) return;

		// 送信前に全dirty行を検査する。1行でも弾かれたら1件も送らない —
		// 送信途中で止まると「どこまで保存されたか」が draft と食い違う。
		const validationErrors: Record<string, string> = {};
		for (const category of dirty) {
			const message = validateDraft(drafts[category.code] ?? '');
			if (message) validationErrors[category.code] = message;
		}
		if (Object.keys(validationErrors).length > 0) {
			rowErrors = validationErrors;
			saveError = '';
			return;
		}

		rowErrors = {};
		saveError = '';
		saving = true;
		try {
			const provider = getDataProvider();
			for (const category of dirty) {
				savingCode = category.code;
				try {
					await provider.update('cost_rates', category.code, {
						hourlyRate: Number(drafts[category.code])
					});
				} catch (err) {
					if (isProviderError(err) && err.body.kind === 'validation') {
						rowErrors = {
							[category.code]: err.body.field_errors.map((e) => e.message).join(' / ')
						};
					} else {
						saveError = m['costRates.saveFailedAt']({ name: category.name });
					}
					// drafts は保持したまま中断（再編集できるように）。
					return;
				}
			}
			notify('success', m['costRates.saveSuccess']());
			await load();
		} finally {
			saving = false;
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
									disabled={!canWrite || saving}
									placeholder={m['costRates.unset']()}
									aria-invalid={!!rowErrors[category.code]}
									bind:value={drafts[category.code]}
								/>
								{#if savingCode === category.code}
									<span class="row-status">{m['costRates.savingRow']()}</span>
								{/if}
								{#if rowErrors[category.code]}
									<p class="row-error" role="alert">{rowErrors[category.code]}</p>
								{/if}
							</td>
						</tr>
					{/each}
				</tbody>
			</table>

			{#if canWrite}
				<div class="save-bar">
					{#if saveError}
						<p class="note note--error" role="alert">{saveError}</p>
					{/if}
					<button
						type="button"
						class="banto-btn banto-btn--primary"
						disabled={dirtyCategories.length === 0 || saving}
						onclick={() => void saveAll()}
					>
						{m['costRates.saveAll']({ count: dirtyCategories.length })}
					</button>
				</div>
			{/if}
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

	.note--error {
		color: var(--banto-danger);
	}

	.panel {
		background: var(--banto-surface);
		border: 1px solid var(--banto-border);
		border-radius: var(--banto-radius-lg);
		box-shadow: var(--banto-shadow-sm);
		padding: 1rem;
		display: flex;
		flex-direction: column;
		gap: 1rem;
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
		vertical-align: top;
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

	input[type='number'][aria-invalid='true'] {
		border-color: var(--banto-danger-solid, var(--banto-danger));
	}

	.row-status {
		display: block;
		margin-top: 0.25rem;
		color: var(--banto-text-muted);
		font-size: 0.8rem;
	}

	.row-error {
		margin: 0.25rem 0 0;
		color: var(--banto-danger);
		font-size: 0.8rem;
	}

	.save-bar {
		display: flex;
		flex-direction: column;
		align-items: flex-end;
		gap: 0.5rem;
	}
</style>
