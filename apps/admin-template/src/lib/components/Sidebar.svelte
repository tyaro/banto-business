<script lang="ts">
	/**
	 * App shell sidebar (visual-refresh-design.md §8.3). Sections: brand /
	 * grouped nav / footer (collapse toggle). ナビは3群区分
	 * （docs/mobile-ui-plan.md P1-4、`navigation.ts` の `navGroups`）で、
	 * 各群に見出しを付ける。adminOnly 項目は従来どおりロールで**非表示**
	 * （disabled ではない）。`overlayOpen` is owned by (app)/+layout.svelte
	 * (no new global store) and only matters at <=900px, where this
	 * component renders as a fixed slide-in drawer instead of the flex
	 * column.
	 */
	import { page } from '$app/state';
	import { base } from '$app/paths';
	import * as m from '$lib/paraglide/messages';
	import { navGroups, navItems } from '$lib/navigation';
	import { NAV_ICONS } from './navIcons';
	import { settings } from '$lib/settings.svelte';
	import { sessionStore } from '$lib/session.svelte';
	import { isAdmin } from '$lib/permissions';
	import IconButton from './ui/IconButton.svelte';
	import { PanelLeftClose, PanelLeftOpen } from '@lucide/svelte';

	interface Props {
		/** <=900px overlay drawer state (design.md §8.1); not a fold state - see the media query below. */
		overlayOpen?: boolean;
	}

	let { overlayOpen = false }: Props = $props();

	function isActive(path: string): boolean {
		return page.url.pathname === path || page.url.pathname.startsWith(path + '/');
	}

	// Spec M10 RBAC: hide admin-only entries (「ユーザー管理」) rather than
	// showing them disabled - navigation-level hiding, same as
	// routes/(app)/users/+page.ts redirecting a non-admin instead of
	// rendering a 403 screen. 群ごとにフィルタし、空になった群は見出しごと
	// 出さない（現状 viewer でも各群に最低1項目残るが、定義変更に備える）。
	const visibleGroups = $derived(
		navGroups
			.map((group) => ({
				...group,
				items: navItems.filter(
					(item) => item.group === group.key && (!item.adminOnly || isAdmin(sessionStore.role))
				)
			}))
			.filter((group) => group.items.length > 0)
	);
</script>

<aside class:collapsed={settings.sidebarCollapsed} class:overlay-open={overlayOpen}>
	<div class="brand">
		<span class="brand-mark" aria-hidden="true">
			<svg viewBox="0 0 24 24" width="14" height="14">
				<rect x="10" y="3" width="4" height="2" rx="1" />
				<circle cx="12" cy="12" r="6" />
				<rect x="10" y="19" width="4" height="2" rx="1" />
			</svg>
		</span>
		<!-- "Banto" is the product brand (owner-fixed): kept as a component
		     constant, never keyed (PR-B2 scope rule). -->
		<span class="brand-name">Banto Business</span>
	</div>

	<nav class="nav-scroll" aria-label={m['shell.mainNav']()}>
		{#each visibleGroups as group, index (group.key)}
			{#if index > 0}
				<div class="section-divider"></div>
			{/if}
			<div class="nav-section">
				<p class="section-heading" aria-hidden="true">{m[group.labelKey]()}</p>
				{#each group.items as item (item.path)}
					{@const Icon = NAV_ICONS[item.icon]}
					<a
						href={`${base}${item.path}`}
						class="nav-item"
						class:active={isActive(item.path)}
						aria-current={isActive(item.path) ? 'page' : undefined}
						title={settings.sidebarCollapsed ? m[item.labelKey]() : undefined}
					>
						<span class="icon"><Icon size={20} aria-hidden="true" /></span>
						<span class="label">{m[item.labelKey]()}</span>
					</a>
				{/each}
			</div>
		{/each}
	</nav>

	<div class="footer">
		<IconButton
			label={settings.sidebarCollapsed ? m['shell.expandSidebar']() : m['shell.collapseSidebar']()}
			icon={settings.sidebarCollapsed ? PanelLeftOpen : PanelLeftClose}
			onclick={() => settings.toggleSidebar()}
		/>
	</div>
</aside>

<style>
	aside {
		width: var(--banto-shell-sidebar-width);
		flex-shrink: 0;
		display: flex;
		flex-direction: column;
		/* 3群の見出し（P1-4）でナビが縦に伸びても、サイドバーが 100vh を
		   超えてシェル全体（＝文書）を押し広げないよう画面高に固定し、
		   ナビ部分（.nav-scroll）だけを内部スクロールにする。伸ばすと
		   本文のグリッドまで連られて高くなり、クリック時のフォーカス
		   スクロールで行がずれる（E2E がそれで落ちた）。sticky なのは
		   本文が長いときにブランドとナビを画面に残すため。 */
		position: sticky;
		top: 0;
		height: 100vh;
		/* セーフエリア（app.css）。**常設表示のときも要る** —— 開いた Fold の
		   内側はタブレット寸法で、サイドバーが畳まれずに画面の左端＝
		   ステータスバーの真下から始まるため。下はジェスチャーバー。 */
		padding-top: var(--app-safe-top);
		padding-bottom: var(--app-safe-bottom);
		padding-left: var(--app-safe-left);
		box-sizing: border-box;
		background: var(--banto-surface);
		border-right: 1px solid var(--banto-border);
		transition: width var(--banto-duration-base) var(--banto-ease-out);
		/* Glass preset (spec M12): no-op under standard (--banto-backdrop: none). */
		backdrop-filter: var(--banto-backdrop, none);
		-webkit-backdrop-filter: var(--banto-backdrop, none);
	}

	aside.collapsed {
		width: var(--banto-shell-sidebar-width-collapsed);
	}

	.brand {
		display: flex;
		align-items: center;
		gap: 0.55rem;
		height: var(--banto-shell-header-height);
		padding: 0 0.9rem;
		border-bottom: 1px solid var(--banto-border);
		font-weight: 700;
	}

	.brand-mark {
		display: inline-flex;
		align-items: center;
		justify-content: center;
		flex-shrink: 0;
		width: 26px;
		height: 26px;
		border-radius: var(--banto-radius-md);
		background: var(--banto-accent-gradient);
	}

	.brand-mark svg {
		fill: var(--banto-text-inverse);
	}

	.brand-name {
		overflow: hidden;
		white-space: nowrap;
		opacity: 1;
		transition: opacity var(--banto-duration-base) var(--banto-ease-out);
	}

	aside.collapsed .brand-name {
		opacity: 0;
	}

	/* 見出し付きの3群（P1-4）で縦が伸びるぶんはここが吸収する。背の低い
	   画面ではナビだけがスクロールし、ブランドとフッターは固定のまま。 */
	.nav-scroll {
		display: flex;
		flex-direction: column;
		flex: 1 1 auto;
		min-height: 0;
		overflow-y: auto;
	}

	.nav-section {
		display: flex;
		flex-direction: column;
		padding: 0.5rem;
		gap: 2px;
	}

	.section-divider {
		height: 1px;
		margin: 0.25rem 0.9rem;
		background: var(--banto-border);
	}

	.section-heading {
		margin: 0.4rem 0.6rem 0.2rem;
		color: var(--banto-text-muted);
		font-size: 0.7rem;
		font-weight: 700;
		text-transform: uppercase;
		letter-spacing: 0.04em;
		overflow: hidden;
		white-space: nowrap;
		opacity: 1;
		transition: opacity var(--banto-duration-base) var(--banto-ease-out);
	}

	aside.collapsed .section-heading {
		opacity: 0;
	}

	.nav-item {
		position: relative;
		display: grid;
		/* Fixed icon column (design.md §8.3): the icon's X coordinate never
		   moves on collapse - only the label track shrinks with the aside's
		   own width transition above. */
		grid-template-columns: 20px minmax(0, 1fr);
		align-items: center;
		column-gap: 0.6rem;
		padding: 0.5rem 0.6rem;
		border-radius: var(--banto-radius-md);
		color: var(--banto-text-muted);
		text-decoration: none;
		transition:
			background var(--banto-duration-fast) var(--banto-ease-out),
			color var(--banto-duration-fast) var(--banto-ease-out);
	}

	.nav-item:hover {
		background: var(--banto-surface-hover);
		color: var(--banto-text);
	}

	.nav-item.active {
		background: color-mix(in srgb, var(--banto-primary) 14%, transparent);
		/* axe-core wcag2aa color-contrast (visual-refresh-plan.md §7.1): plain
		   --banto-primary on this tint background measures ~4.24:1, just under
		   the 4.5:1 text minimum - --banto-primary-hover (already defined,
		   previously unused) is darker/lighter enough per theme to clear it. */
		color: var(--banto-primary-hover);
		font-weight: 600;
	}

	.nav-item.active::before {
		content: '';
		position: absolute;
		inset-block: 4px;
		left: 0;
		width: 2px;
		border-radius: 2px;
		background: var(--banto-primary);
	}

	/* Glass preset accent (spec M12): the active nav item gets the accent
	   gradient. Scoped by the preset attribute so standard keeps the flat
	   tint above untouched. */
	:global([data-banto-preset='glass']) .nav-item.active {
		background: var(--banto-accent-gradient);
		color: var(--banto-text-inverse);
	}

	.icon {
		display: flex;
		align-items: center;
		justify-content: center;
	}

	.label {
		overflow: hidden;
		white-space: nowrap;
		opacity: 1;
		transition: opacity var(--banto-duration-base) var(--banto-ease-out);
	}

	aside.collapsed .label {
		opacity: 0;
	}

	.footer {
		margin-top: auto;
		padding: 0.5rem;
		border-top: 1px solid var(--banto-border);
	}

	@media (max-width: 900px) {
		aside {
			position: fixed;
			inset: 0 auto 0 0;
			/* fixed + inset が高さを決める。100vh のままだと動的ツールバーの
			   あるモバイルブラウザで実表示より高くなる。 */
			height: auto;
			z-index: 900;
			/* Overlay mode has no fold concept (design.md §8.3): always full
			   width regardless of the persisted collapsed setting. */
			width: var(--banto-shell-sidebar-width);
			box-shadow: var(--banto-shadow-lg);
			transform: translateX(-100%);
			transition: transform var(--banto-duration-base) var(--banto-ease-spring);
		}

		aside.collapsed {
			width: var(--banto-shell-sidebar-width);
		}

		aside.collapsed .label,
		aside.collapsed .brand-name,
		aside.collapsed .section-heading {
			opacity: 1;
		}

		aside.overlay-open {
			transform: translateX(0);
		}

		.footer {
			display: none;
		}
	}
</style>
