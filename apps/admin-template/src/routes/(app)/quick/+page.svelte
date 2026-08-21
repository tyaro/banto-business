<script lang="ts">
	/**
	 * クイック入力（工数を1件）。Phase 8 ステップ6。
	 *
	 * ## なぜ「外側画面専用」ではないのか
	 *
	 * `docs/domain/sync.md` 8.1 は「畳んだ外側は細く、一覧グリッドを縮めても
	 * 使えない」という前提で専用画面を想定していた。**実機で確かめたところ
	 * 外側でも普通のUIが使える**（Pixel 10 Fold、2026-08-21）ので、前提の方を
	 * 直した。
	 *
	 * それでも作る価値はある —— 通常の工数フォームは7項目あり、外出先で
	 * 「さっきの作業を1件入れる」には多い。ここは**押す回数を減らす**ことだけ
	 * を目的にする。畳んだ外側でも当然使えるが、開いていても速い。
	 *
	 * ## 減らし方
	 *
	 * | 項目 | 通常フォーム | ここ |
	 * | --- | --- | --- |
	 * | 日付 | 入力 | 今日が既定（変えられる） |
	 * | 案件 | 選ぶ | **前回の値を覚えている** |
	 * | 作業分類 | コードを手打ち | 選ぶ（前回の値を覚えている） |
	 * | 時間 | 分を入力 | よく使う長さを押す or 分を入力 |
	 * | 適用レート | 入力 | **出さない**（空欄＝分類の既定。要件 F-W2） |
	 * | 請求済み | 入力 | **出さない**（新規は必ず未請求） |
	 *
	 * 覚えるのは端末ごと（`ui-settings`）。PC とスマホで直前の案件が違うのが
	 * 普通なので、同期する設定にはしない。
	 *
	 * ## 保存しても画面を離れない
	 *
	 * 続けて入れることが多い（午前の作業と午後の作業など）。案件と分類は
	 * 残し、時間とメモだけ空にする。今日の合計を出して、入れ忘れと二重入力に
	 * 気付けるようにする。
	 */
	import * as m from '$lib/paraglide/messages';
	import { getDataProvider, isProviderError, type ListParams } from '@banto/admin-core';
	import { getUiSettings } from '$lib/banto/setup';
	import { projectOptions, loadProjectOptions } from '$lib/banto/referenceOptions.svelte';
	import { localToday } from '$lib/banto/today';
	import { canWriteResources } from '$lib/permissions';
	import { sessionStore } from '$lib/session.svelte';
	import PageHeader from '$lib/components/ui/PageHeader.svelte';
	import EmptyState from '$lib/components/ui/EmptyState.svelte';

	/** 端末ごとに覚える直前の値（同期しない。上の doc を参照）。 */
	const LAST_PROJECT_KEY = 'quick.lastProjectId';
	const LAST_CATEGORY_KEY = 'quick.lastWorkCategory';

	/** よく使う長さ。30分刻みで、半日・1日は押し間違えが怖いので入れない。 */
	const MINUTE_PRESETS = [30, 60, 90, 120, 180, 240];

	interface WorkCategory {
		code: string;
		name: string;
		active: number;
		sortOrder: number;
	}

	interface WorkLogRow {
		minutes: number;
	}

	const canWrite = $derived(canWriteResources(sessionStore.role));

	// 「今日」は画面で一度だけ決める（`today.ts` の doc を参照）。
	const today = localToday();

	let categories = $state<WorkCategory[]>([]);
	let projectId = $state<number | null>(null);
	let workCategoryCode = $state('');
	let workedOn = $state(today);
	let minutes = $state<number | null>(null);
	let description = $state('');

	let saving = $state(false);
	let errorMessage = $state('');
	let savedCount = $state(0);
	let todayMinutes = $state(0);

	async function load() {
		await loadProjectOptions();
		try {
			const params: ListParams = {
				sort: [{ field: 'sort_order', direction: 'asc' }],
				filters: [],
				pagination: { offset: 0, limit: 100 }
			};
			const result = await getDataProvider().getList<WorkCategory>('work_categories', params);
			categories = result.rows.filter((row) => row.active !== 0);
		} catch {
			categories = [];
		}
		await restoreLastUsed();
		await refreshTodayTotal();
	}

	/**
	 * 直前の値を戻す。**選択肢に無い値は戻さない** —— 案件が削除されている
	 * と、選ばれていないのに選ばれているように見える。
	 */
	async function restoreLastUsed() {
		try {
			const ui = getUiSettings();
			const [lastProject, lastCategory] = await Promise.all([
				ui.get(LAST_PROJECT_KEY),
				ui.get(LAST_CATEGORY_KEY)
			]);
			const projectValue = Number(lastProject);
			if (lastProject && projectOptions().some((option) => option.value === projectValue)) {
				projectId = projectValue;
			}
			if (lastCategory && categories.some((row) => row.code === lastCategory)) {
				workCategoryCode = lastCategory;
			}
		} catch {
			// 覚えていた値が読めなくても入力はできる。
		}
		if (workCategoryCode === '' && categories.length > 0) {
			workCategoryCode = categories[0].code;
		}
	}

	/** 今日の合計（分）。入れ忘れと二重入力に気付くための目安。 */
	async function refreshTodayTotal() {
		try {
			const params: ListParams = {
				sort: [],
				filters: [{ field: 'workedOn', op: 'eq', value: workedOn }],
				pagination: { offset: 0, limit: 200 }
			};
			const result = await getDataProvider().getList<WorkLogRow>('work_logs', params);
			todayMinutes = result.rows.reduce((total, row) => total + (row.minutes ?? 0), 0);
		} catch {
			todayMinutes = 0;
		}
	}

	$effect(() => {
		void load();
	});

	function hoursOf(total: number): string {
		return (total / 60).toFixed(1);
	}

	async function save() {
		if (projectId === null || workCategoryCode === '' || !minutes || minutes <= 0) {
			errorMessage = m['quick.incomplete']();
			return;
		}
		saving = true;
		errorMessage = '';
		try {
			await getDataProvider().create('work_logs', {
				projectId,
				workedOn,
				workCategoryCode,
				minutes,
				// 空欄なら作業分類の既定レートをサーバが引く（要件 F-W2）。
				appliedRate: null,
				description: description === '' ? null : description,
				invoiced: false
			});
			savedCount = savedCount + 1;

			// 続けて入れる前提で、案件と分類は残す。
			minutes = null;
			description = '';
			await rememberLastUsed();
			await refreshTodayTotal();
		} catch (err) {
			if (isProviderError(err) && err.body.kind === 'validation') {
				errorMessage = err.body.field_errors.map((e) => e.message).join(' / ');
			} else {
				errorMessage = m['quick.saveError']();
			}
		} finally {
			saving = false;
		}
	}

	async function rememberLastUsed() {
		try {
			const ui = getUiSettings();
			await Promise.all([
				ui.set(LAST_PROJECT_KEY, String(projectId)),
				ui.set(LAST_CATEGORY_KEY, workCategoryCode)
			]);
		} catch {
			// 覚えられなくても保存は済んでいる。次回また選べばよい。
		}
	}
</script>

<div class="page">
	<PageHeader title={m['quick.title']()} description={m['quick.description']()} />

	{#if !canWrite}
		<EmptyState title={m['quick.readOnlyTitle']()} description={m['quick.readOnlyDesc']()} />
	{:else}
		<section class="panel">
			<label class="field">
				<span>{m['workLogs.fieldProjectId']()}</span>
				<select class="banto-input" bind:value={projectId}>
					<option value={null}>{m['quick.chooseProject']()}</option>
					{#each projectOptions() as option (option.value)}
						<option value={option.value}>{option.label}</option>
					{/each}
				</select>
			</label>

			<label class="field">
				<span>{m['workLogs.fieldWorkCategoryCode']()}</span>
				<select class="banto-input" bind:value={workCategoryCode}>
					{#each categories as category (category.code)}
						<option value={category.code}>{category.name}</option>
					{/each}
				</select>
			</label>

			<label class="field">
				<span>{m['workLogs.fieldWorkedOn']()}</span>
				<input
					class="banto-input"
					type="date"
					bind:value={workedOn}
					onchange={() => refreshTodayTotal()}
				/>
			</label>

			<div class="field">
				<span>{m['workLogs.fieldMinutes']()}</span>
				<div class="presets">
					{#each MINUTE_PRESETS as preset (preset)}
						<button
							type="button"
							class="banto-btn preset"
							class:preset--on={minutes === preset}
							onclick={() => (minutes = preset)}
						>
							{m['quick.minutesPreset']({ minutes: preset })}
						</button>
					{/each}
				</div>
				<input class="banto-input" type="number" min="1" max="1440" bind:value={minutes} />
			</div>

			<label class="field">
				<span>{m['workLogs.fieldDescription']()}</span>
				<input class="banto-input" type="text" bind:value={description} />
			</label>

			{#if errorMessage}
				<p class="note note--error">{errorMessage}</p>
			{:else if savedCount > 0}
				<p class="note note--muted">{m['quick.saved']({ count: savedCount })}</p>
			{/if}

			<div class="actions">
				<button
					type="button"
					class="banto-btn banto-btn--primary save"
					onclick={save}
					disabled={saving}
				>
					{m['quick.save']()}
				</button>
			</div>

			<p class="note note--muted">
				{m['quick.dayTotal']({ date: workedOn, hours: hoursOf(todayMinutes) })}
			</p>
		</section>
	{/if}
</div>

<style>
	.page {
		display: flex;
		flex-direction: column;
		gap: 1rem;
		max-width: 480px;
	}

	.panel {
		display: flex;
		flex-direction: column;
		gap: 0.85rem;
		background: var(--banto-surface);
		border: 1px solid var(--banto-border);
		border-radius: var(--banto-radius-lg);
		box-shadow: var(--banto-shadow-sm);
		padding: 1.25rem;
	}

	.field {
		display: flex;
		flex-direction: column;
		gap: 0.3rem;
		font-size: 0.85rem;
	}

	/* 押しやすさを優先する画面なので、入力欄も指の幅に合わせる。 */
	.field :global(.banto-input) {
		min-height: 2.75rem;
	}

	.presets {
		display: flex;
		flex-wrap: wrap;
		gap: 0.4rem;
	}

	.preset {
		min-height: 2.75rem;
		flex: 1 1 4.5rem;
	}

	.preset--on {
		border-color: var(--banto-primary);
		color: var(--banto-primary);
	}

	.actions {
		display: flex;
	}

	/* 保存は片手で押せるように、幅いっぱい・高めに。 */
	.save {
		flex: 1;
		min-height: 3rem;
	}

	.note {
		margin: 0;
		font-size: 0.85rem;
	}

	.note--muted {
		color: var(--banto-text-muted);
	}

	.note--error {
		color: var(--banto-danger);
	}
</style>
