<script lang="ts">
	/**
	 * 初期セットアップの道しるべ（P2-1、`docs/mobile-ui-plan.md`）。
	 *
	 * 導入日に「発行者情報 → 原価レート → 顧客 → 案件 → 工数」の依存順を
	 * アプリ自身が教える。dashboard の最上部（未入金パネルより上）に、
	 * 5項目すべてが済むまでだけ表示する。
	 *
	 * - **admin のみ**取得する（発行者情報の有無を含むため `issuer` と同じ床）。
	 *   admin 以外・デモモード・取得失敗はどれも「何も表示しない」に畳む
	 *   （`setupGuideAdmin.ts` の doc を参照。dashboard を壊さないため）。
	 * - 済判定は5項目とも `setup_status` サーバ側の導出値をそのまま使う。
	 *   ここでは金額計算はもちろん、日付比較のような判定も一切行わない
	 *   （`OutstandingPanel` と同じ方針）。
	 */
	import { base } from '$app/paths';
	import * as m from '$lib/paraglide/messages';
	import { sessionStore } from '$lib/session.svelte';
	import { isAdmin } from '$lib/permissions';
	import { getSetupStatus, type SetupStatus } from '$lib/banto/setupGuideAdmin';

	let status = $state<SetupStatus | null>(null);

	async function load() {
		status = isAdmin(sessionStore.role) ? await getSetupStatus() : null;
	}

	$effect(() => {
		void load();
	});

	interface Item {
		key: string;
		done: boolean;
		label: string;
		href: string;
	}

	const items = $derived.by((): Item[] => {
		if (!status) return [];
		return [
			{
				key: 'issuer',
				done: status.issuerDone,
				label: m['setupGuide.itemIssuer'](),
				href: `${base}/issuer`
			},
			{
				key: 'rates',
				done: status.ratesDone,
				label: m['setupGuide.itemRates'](),
				href: `${base}/cost-rates`
			},
			{
				key: 'customers',
				done: status.customersDone,
				label: m['setupGuide.itemCustomers'](),
				href: `${base}/customers`
			},
			{
				key: 'projects',
				done: status.projectsDone,
				label: m['setupGuide.itemProjects'](),
				href: `${base}/projects`
			},
			{
				key: 'workLogs',
				done: status.workLogsDone,
				label: m['setupGuide.itemWorkLogs'](),
				href: `${base}/quick`
			}
		];
	});
</script>

{#if status && !status.allDone}
	<section class="panel" aria-labelledby="setup-guide-heading">
		<header class="panel-header">
			<h2 id="setup-guide-heading">{m['setupGuide.title']()}</h2>
		</header>
		<p class="note note--muted">{m['setupGuide.description']()}</p>
		<ol class="checklist">
			{#each items as item (item.key)}
				<li>
					<a href={item.href} class="item" class:done={item.done}>
						<span class="mark" aria-hidden="true">{item.done ? '✓' : '○'}</span>
						<span class="label">{item.label}</span>
						<span class="state">
							{item.done ? m['setupGuide.statusDone']() : m['setupGuide.statusPending']()}
						</span>
					</a>
				</li>
			{/each}
		</ol>
	</section>
{/if}

<style>
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

	.panel-header h2 {
		margin: 0;
		font-size: 1rem;
		font-weight: 600;
	}

	.note {
		margin: 0;
		font-size: 0.85rem;
	}

	.note--muted {
		color: var(--banto-text-muted);
	}

	.checklist {
		display: flex;
		flex-direction: column;
		gap: 0.375rem;
		margin: 0;
		padding: 0;
		list-style: none;
	}

	/* タッチターゲット目安 44px（`docs/mobile-ui-plan.md` の前提）。 */
	.item {
		display: flex;
		align-items: center;
		gap: 0.625rem;
		min-height: 44px;
		padding: 0.375rem 0.625rem;
		border: 1px solid var(--banto-border);
		border-radius: var(--banto-radius-md);
		color: inherit;
		text-decoration: none;
	}

	.item:hover {
		background: var(--banto-surface-hover);
	}

	.mark {
		font-size: 1rem;
		width: 1.25rem;
		text-align: center;
		color: var(--banto-text-muted);
	}

	.item.done .mark {
		color: var(--banto-success);
	}

	.label {
		flex: 1;
		font-size: 0.9rem;
	}

	.state {
		font-size: 0.8rem;
		color: var(--banto-text-muted);
	}

	.item.done .state {
		color: var(--banto-success);
	}

	.item.done .label {
		color: var(--banto-text-muted);
		text-decoration: line-through;
	}
</style>
