/**
 * Sidebar navigation definition.
 *
 * From M2, entries for CRUD pages are derived from resource definitions
 * (spec §3.1); manual entries like the ones below remain possible.
 */
import * as m from '$lib/paraglide/messages';

/** Icon resolution key (visual-refresh-design.md §5.1). Resolved to an actual
 *  icon component only in the display layer ($lib/components/navIcons.ts) -
 *  this module stays UI-agnostic. */
export type NavIconKey =
	| 'dashboard'
	| 'customers'
	| 'projects'
	| 'work-logs'
	| 'trips'
	| 'expenses'
	| 'cost-rates'
	| 'items'
	| 'tree'
	| 'users'
	| 'audit-log'
	| 'settings';

/** Paraglide message key for a nav entry's visible label (i18n layer ②,
 *  ADR-0005). The label itself is resolved at render time via `m[labelKey]()`
 *  so it tracks the active locale — see `pageTitle` below and Sidebar.svelte. */
export type NavLabelKey =
	| 'nav.dashboard'
	| 'nav.customers'
	| 'nav.projects'
	| 'nav.workLogs'
	| 'nav.trips'
	| 'nav.expenses'
	| 'nav.costRates'
	| 'nav.items'
	| 'nav.tree'
	| 'nav.users'
	| 'nav.auditLog'
	| 'nav.settings';

export interface NavItem {
	path: string;
	labelKey: NavLabelKey;
	icon: NavIconKey;
	/** Spec M10 RBAC: only shown to the `admin` role. Undefined/false = visible to every role. */
	adminOnly?: boolean;
}

export const navItems: NavItem[] = [
	{ path: '/dashboard', labelKey: 'nav.dashboard', icon: 'dashboard' },
	// Business ドメイン（Phase 2 基本マスター）。顧客 → 案件の順は
	// docs/plan.md 第18章 Phase 2 の依存順（Customer → Project）に合わせる。
	{ path: '/customers', labelKey: 'nav.customers', icon: 'customers' },
	{ path: '/projects', labelKey: 'nav.projects', icon: 'projects' },
	// Phase 3（工数・経費）。入力頻度の高い順に並べる（工数 → 出張 → 経費）。
	{ path: '/work-logs', labelKey: 'nav.workLogs', icon: 'work-logs' },
	{ path: '/trips', labelKey: 'nav.trips', icon: 'trips' },
	{ path: '/expenses', labelKey: 'nav.expenses', icon: 'expenses' },
	{ path: '/cost-rates', labelKey: 'nav.costRates', icon: 'cost-rates' },
	{ path: '/items', labelKey: 'nav.items', icon: 'items' },
	{ path: '/tree', labelKey: 'nav.tree', icon: 'tree' },
	{ path: '/users', labelKey: 'nav.users', icon: 'users', adminOnly: true },
	{ path: '/audit-log', labelKey: 'nav.auditLog', icon: 'audit-log', adminOnly: true },
	{ path: '/settings', labelKey: 'nav.settings', icon: 'settings' }
];

// "Banto" is the product brand (owner-fixed): kept as a component constant,
// never entered into the dictionary (PR-B2 scope rule).
const BRAND = 'Banto';

export function pageTitle(pathname: string): string {
	const item = navItems.find(
		(entry) => pathname === entry.path || pathname.startsWith(entry.path + '/')
	);
	return item ? m[item.labelKey]() : BRAND;
}
