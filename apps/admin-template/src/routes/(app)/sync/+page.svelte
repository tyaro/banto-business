<script lang="ts">
	/**
	 * デバイス間同期（Phase 8、`docs/domain/sync.md` 11節）。
	 *
	 * ## 手動のボタン1つ（2026-08-21 決定、11.8）
	 *
	 * LAN を検知して自動で同期する案は採らない。帰宅したことは本人が
	 * 分かっているので検知の価値が小さい一方、外出先では毎回タイムアウトを
	 * 待つことになる。押したときだけ通信する。
	 *
	 * ## パスワードは保存しない
	 *
	 * PC のパスワードは**アプリのメモリにだけ**置く（11.9）。設定にも
	 * keyring にも書かない —— keyring はそもそも Android ビルドから
	 * 外してある（8節）。入力済みかどうかだけを `hasPassword` で受け取り、
	 * 値そのものは画面へ返ってこない。
	 *
	 * ## デバイス番号は行が在ると変えられない
	 *
	 * 変えても既存行の id は動かないので、後から作る行が相手端末の行と
	 * 同じ id を持つ（`docs/android-build.md` 5.1）。`hasRows` が true の
	 * ときは入力欄を締め、理由を出す。
	 */
	import * as m from '$lib/paraglide/messages';
	import {
		applySyncSettings,
		getSyncSettings,
		isSyncAvailable,
		runSync,
		type SyncOutcome,
		type SyncSettings
	} from '$lib/banto/syncAdmin';
	import { isProviderError } from '@banto/admin-core';
	import { sessionStore } from '$lib/session.svelte';
	import PageHeader from '$lib/components/ui/PageHeader.svelte';
	import EmptyState from '$lib/components/ui/EmptyState.svelte';
	import ErrorState from '$lib/components/ui/ErrorState.svelte';
	import LoadingState from '$lib/components/ui/LoadingState.svelte';

	const isAdmin = $derived(sessionStore.role === 'admin');
	const available = isSyncAvailable();

	let settings = $state<SyncSettings | null>(null);
	let loading = $state(true);
	let failed = $state(false);

	// 入力中の値。`settings` をそのまま bind すると、保存に失敗したときに
	// 画面の値とサーバの値が食い違ったままになる。
	let deviceId = $state(0);
	let peerUrl = $state('');
	let peerUsername = $state('');
	let password = $state('');

	let saving = $state(false);
	let savedAt = $state(0);
	let settingsError = $state('');

	let syncing = $state(false);
	let outcome = $state<SyncOutcome | null>(null);
	let syncError = $state('');

	function adopt(next: SyncSettings) {
		settings = next;
		deviceId = next.deviceId;
		peerUrl = next.peerUrl;
		peerUsername = next.peerUsername;
	}

	async function load() {
		if (!available || !isAdmin) {
			loading = false;
			return;
		}
		loading = true;
		failed = false;
		try {
			adopt(await getSyncSettings());
		} catch {
			failed = true;
		} finally {
			loading = false;
		}
	}

	$effect(() => {
		void load();
	});

	function messageOf(err: unknown, fallback: string): string {
		if (isProviderError(err)) {
			if (err.body.kind === 'validation') {
				return err.body.field_errors.map((e) => e.message).join(' / ');
			}
			if ('message' in err.body && typeof err.body.message === 'string') {
				return err.body.message;
			}
		}
		return fallback;
	}

	async function save() {
		saving = true;
		settingsError = '';
		try {
			adopt(await applySyncSettings({ deviceId, peerUrl, peerUsername }));
			savedAt = savedAt + 1;
		} catch (err) {
			settingsError = messageOf(err, m['sync.saveError']());
		} finally {
			saving = false;
		}
	}

	async function sync() {
		syncing = true;
		syncError = '';
		outcome = null;
		try {
			outcome = await runSync(password || undefined);
			// 受け取れたら手元の控えは捨てる。画面に残し続ける理由が無い
			// （控えはアプリ側が持っており、`hasPassword` で分かる）。
			password = '';
			adopt(await getSyncSettings());
		} catch (err) {
			syncError = messageOf(err, m['sync.runError']());
			// 失敗の理由がパスワードなら、アプリ側の控えも捨てられている。
			try {
				adopt(await getSyncSettings());
			} catch {
				// 設定の読み直しに失敗しても、同期の失敗表示は残す。
			}
		} finally {
			syncing = false;
		}
	}

	const needsPassword = $derived(!settings?.hasPassword && password.trim() === '');
	const notConfigured = $derived(!settings?.peerUrl || !settings?.peerUsername);
</script>

<div class="page">
	<PageHeader title={m['sync.title']()} description={m['sync.description']()} />

	{#if !available}
		<EmptyState title={m['sync.desktopOnlyTitle']()} description={m['sync.desktopOnlyDesc']()} />
	{:else if !isAdmin}
		<EmptyState title={m['sync.adminOnlyTitle']()} description={m['sync.adminOnlyDesc']()} />
	{:else if loading}
		<LoadingState label={m['common.loading']()} />
	{:else if failed || !settings}
		<ErrorState title={m['sync.loadError']()} description={m['resource.loadErrorDesc']()} />
	{:else}
		<section class="panel">
			<h2>{m['sync.runHeading']()}</h2>
			<p class="note note--muted">
				{settings.lastSyncedAt
					? m['sync.lastSynced']({ at: settings.lastSyncedAt })
					: m['sync.neverSynced']()}
			</p>

			{#if notConfigured}
				<p class="note note--muted">{m['sync.configureFirst']()}</p>
			{:else}
				{#if !settings.hasPassword}
					<label class="field">
						<span>{m['sync.fieldPassword']()}</span>
						<input class="banto-input" type="password" bind:value={password} />
						<small>{m['sync.passwordHint']()}</small>
					</label>
				{/if}

				<div class="actions">
					<button
						type="button"
						class="banto-btn banto-btn--primary"
						onclick={sync}
						disabled={syncing || needsPassword}
					>
						{syncing ? m['sync.running']() : m['sync.run']()}
					</button>
				</div>
			{/if}

			{#if syncError}
				<p class="note note--error">{syncError}</p>
			{:else if outcome}
				<p class="note">
					{m['sync.result']({
						pulled: outcome.pulledApplied,
						pushed: outcome.pushedApplied
					})}
				</p>
			{/if}

			{#if settings.openConflicts > 0}
				<p class="note note--warn">
					{m['sync.openConflicts']({ count: settings.openConflicts })}
				</p>
			{/if}
		</section>

		<section class="panel">
			<h2>{m['sync.settingsHeading']()}</h2>

			<label class="field">
				<span>{m['sync.fieldPeerUrl']()}</span>
				<input
					class="banto-input"
					type="text"
					bind:value={peerUrl}
					placeholder={m['sync.peerUrlPlaceholder']()}
				/>
				<small>{m['sync.peerUrlHint']()}</small>
			</label>

			<label class="field">
				<span>{m['sync.fieldPeerUsername']()}</span>
				<input class="banto-input" type="text" bind:value={peerUsername} />
			</label>

			<label class="field">
				<span>{m['sync.fieldDeviceId']()}</span>
				<input
					class="banto-input"
					type="number"
					min="0"
					bind:value={deviceId}
					disabled={settings.hasRows}
				/>
				<small>
					{settings.hasRows ? m['sync.deviceIdLocked']() : m['sync.deviceIdHint']()}
				</small>
			</label>

			{#if settingsError}
				<p class="note note--error">{settingsError}</p>
			{:else if savedAt > 0}
				<p class="note note--muted">{m['sync.saved']()}</p>
			{/if}

			<div class="actions">
				<button type="button" class="banto-btn banto-btn--primary" onclick={save} disabled={saving}>
					{m['common.save']()}
				</button>
			</div>
		</section>

		<p class="note note--muted">{m['sync.scopeNote']()}</p>
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

	.panel h2 {
		margin: 0;
		font-size: 1rem;
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

	.note--warn {
		color: var(--banto-warning);
	}
</style>
