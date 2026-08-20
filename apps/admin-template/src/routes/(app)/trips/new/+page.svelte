<script lang="ts">
	/**
	 * 出張の登録と一括生成（要件 F-T1 / U-1: 出張1回の入力が1画面で完結）。
	 *
	 * 通常の新規作成ページと違い、リソースのスキーマに加えて**生成用の入力**
	 * を持つ。生成は「登録時に1回だけ」の操作なので、編集ページには出さない
	 * （更新のたびに再生成できるように見えると、手で直した工数を失う）。
	 *
	 * 生成件数・金額の計算はサーバ側が確定した結果を表示するだけで、
	 * ここでは計算しない（AGENTS.md 第1章: フロントで金額計算をしない）。
	 */
	import { goto } from '$app/navigation';
	import { base } from '$app/paths';
	import { BantoForm, createFormStore } from '@banto/forms';
	import type { FormSchema } from '@banto/forms';
	import { createFormResource, getResource } from '@banto/admin-core';
	import * as m from '$lib/paraglide/messages';
	import { formValidationMessages } from '$lib/banto/i18n';
	import PageHeader from '$lib/components/ui/PageHeader.svelte';
	import LoadingState from '$lib/components/ui/LoadingState.svelte';

	const resource = getResource('trips');
	const schema = resource.schema as FormSchema;

	const formResource = createFormResource('trips');
	const store = createFormStore(schema, undefined, formValidationMessages());

	// 一括生成の入力。0 のままの項目は生成されない（サーバ側の仕様）。
	let travelMinutesOneWay = $state(0);
	let onsiteMinutesPerDay = $state(0);
	let transportAmount = $state(0);
	let lodgingAmountPerNight = $state(0);
	let generateBillable = $state(true);

	$effect(() => {
		void formResource.load();
	});

	async function handleSubmit(values: Record<string, unknown>) {
		const result = await formResource.submit({
			...values,
			generate: {
				travelMinutesOneWay,
				onsiteMinutesPerDay,
				transportAmount,
				lodgingAmountPerNight,
				billable: generateBillable
			}
		});
		if (result.ok) {
			goto(`${base}/trips`);
		} else {
			// サーバ側の検証エラー。生成入力のエラーは `generate.` 接頭辞付きで
			// 返るため、該当欄が無いフォームでも全文を残す。
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
				submitLabel={m['trips.generateSubmit']()}
			>
				<section class="generate">
					<h2>{m['trips.generateSection']()}</h2>
					<p class="hint">{m['trips.generateHint']()}</p>

					<div class="grid">
						<label>
							{m['trips.fieldTravelMinutes']()}
							<input type="number" min="0" max="1440" bind:value={travelMinutesOneWay} />
						</label>
						<label>
							{m['trips.fieldOnsiteMinutes']()}
							<input type="number" min="0" max="1440" bind:value={onsiteMinutesPerDay} />
						</label>
						<label>
							{m['trips.fieldTransportAmount']()}
							<input type="number" min="0" bind:value={transportAmount} />
						</label>
						<label>
							{m['trips.fieldLodgingAmount']()}
							<input type="number" min="0" bind:value={lodgingAmountPerNight} />
						</label>
					</div>

					<label class="checkbox">
						<input type="checkbox" bind:checked={generateBillable} />
						{m['trips.fieldGenerateBillable']()}
					</label>
				</section>
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

	.generate {
		border-top: 1px solid var(--banto-border);
		margin-top: 1rem;
		padding-top: 1rem;
	}

	.generate h2 {
		font-size: var(--banto-font-size-md);
		margin: 0 0 0.25rem;
	}

	.hint {
		color: var(--banto-text-muted);
		font-size: var(--banto-font-size-sm);
		margin: 0 0 0.75rem;
	}

	.grid {
		display: grid;
		gap: 0.75rem;
		grid-template-columns: repeat(auto-fit, minmax(14rem, 1fr));
	}

	.grid label,
	.checkbox {
		display: flex;
		flex-direction: column;
		font-size: var(--banto-font-size-sm);
		gap: 0.25rem;
	}

	.checkbox {
		align-items: center;
		flex-direction: row;
		gap: 0.5rem;
		margin-top: 0.75rem;
	}

	input[type='number'] {
		background: var(--banto-surface);
		border: 1px solid var(--banto-border);
		border-radius: var(--banto-radius-sm);
		color: var(--banto-text);
		padding: 0.4rem 0.5rem;
	}
</style>
