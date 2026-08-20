<script lang="ts">
	/**
	 * 事業者情報の設定（適格請求書の発行者。`docs/tax-calculation.md` 2）。
	 *
	 * **admin のみ。** 登録番号・住所・振込先はアプリ全体の設定であり、
	 * 閲覧者に配る情報ではない（`settings` と同じ床）。
	 *
	 * ここに入れた値は**確定時に Invoice へスナップショットされる**（F-I7）。
	 * 後から変更しても、既に発行した請求書の記載は変わらない。その旨を画面にも
	 * 明記する（原価レート設定と同じ考え方）。
	 */
	import * as m from '$lib/paraglide/messages';
	import {
		getIssuerSettings,
		updateIssuerSettings,
		type IssuerSettings
	} from '$lib/banto/invoicesAdmin';
	import { isProviderError } from '@banto/admin-core';
	import { sessionStore } from '$lib/session.svelte';
	import PageHeader from '$lib/components/ui/PageHeader.svelte';
	import EmptyState from '$lib/components/ui/EmptyState.svelte';
	import ErrorState from '$lib/components/ui/ErrorState.svelte';
	import LoadingState from '$lib/components/ui/LoadingState.svelte';

	const ROUNDING_MODES = ['FLOOR', 'ROUND', 'CEIL'] as const;

	const isAdmin = $derived(sessionStore.role === 'admin');

	let settings = $state<IssuerSettings | null>(null);
	let loading = $state(true);
	let failed = $state(false);
	let saving = $state(false);
	let savedAt = $state(0);
	let errorMessage = $state('');

	async function load() {
		if (!isAdmin) {
			loading = false;
			return;
		}
		loading = true;
		failed = false;
		try {
			settings = await getIssuerSettings();
		} catch {
			failed = true;
		} finally {
			loading = false;
		}
	}

	$effect(() => {
		void load();
	});

	async function save() {
		if (!settings) return;
		saving = true;
		errorMessage = '';
		try {
			settings = await updateIssuerSettings(settings);
			savedAt = savedAt + 1;
		} catch (err) {
			if (isProviderError(err) && err.body.kind === 'validation') {
				errorMessage = err.body.field_errors.map((e) => e.message).join(' / ');
			} else {
				errorMessage = m['issuer.saveError']();
			}
		} finally {
			saving = false;
		}
	}

	const roundingLabel = (code: string): string => {
		switch (code) {
			case 'FLOOR':
				return m['issuer.roundingFloor']();
			case 'ROUND':
				return m['issuer.roundingRound']();
			default:
				return m['issuer.roundingCeil']();
		}
	};
</script>

<div class="page">
	<PageHeader title={m['issuer.title']()} description={m['issuer.description']()} />

	{#if !isAdmin}
		<EmptyState title={m['issuer.adminOnlyTitle']()} description={m['issuer.adminOnlyDesc']()} />
	{:else if loading}
		<LoadingState label={m['common.loading']()} />
	{:else if failed || !settings}
		<ErrorState title={m['issuer.loadError']()} description={m['resource.loadErrorDesc']()} />
	{:else}
		<section class="panel">
			<label class="field">
				<span>{m['issuer.fieldName']()}</span>
				<input class="banto-input" type="text" bind:value={settings.name} />
			</label>
			<label class="field">
				<span>{m['issuer.fieldRegistrationNumber']()}</span>
				<input
					class="banto-input"
					type="text"
					bind:value={settings.registrationNumber}
					placeholder={m['issuer.registrationPlaceholder']()}
				/>
			</label>
			<label class="field">
				<span>{m['issuer.fieldAddress']()}</span>
				<input class="banto-input" type="text" bind:value={settings.address} />
			</label>
			<label class="field">
				<span>{m['issuer.fieldBankAccount']()}</span>
				<input class="banto-input" type="text" bind:value={settings.bankAccount} />
			</label>
			<label class="field">
				<span>{m['issuer.fieldRoundingMode']()}</span>
				<select class="banto-input" bind:value={settings.roundingMode}>
					{#each ROUNDING_MODES as mode (mode)}
						<option value={mode}>{roundingLabel(mode)}</option>
					{/each}
				</select>
				<small>{m['issuer.roundingHint']()}</small>
			</label>

			{#if errorMessage}
				<p class="note note--error">{errorMessage}</p>
			{:else if savedAt > 0}
				<p class="note note--muted">{m['issuer.saved']()}</p>
			{/if}

			<div class="actions">
				<button type="button" class="banto-btn banto-btn--primary" onclick={save} disabled={saving}>
					{m['common.save']()}
				</button>
			</div>
		</section>

		<p class="note note--muted">{m['issuer.snapshotNote']()}</p>
	{/if}
</div>

<style>
	.page {
		display: flex;
		flex-direction: column;
		gap: 1rem;
		max-width: 720px;
	}

	.panel {
		display: flex;
		flex-direction: column;
		gap: 0.75rem;
		background: var(--banto-surface);
		border: 1px solid var(--banto-border);
		border-radius: var(--banto-radius-lg);
		box-shadow: var(--banto-shadow-sm);
		padding: 1.25rem;
	}

	.field {
		display: flex;
		flex-direction: column;
		gap: 0.25rem;
		font-size: 0.85rem;
	}

	.field small {
		color: var(--banto-text-muted);
	}

	.actions {
		display: flex;
		gap: 0.5rem;
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
