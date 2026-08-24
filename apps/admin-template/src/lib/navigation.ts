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
	| 'calendar'
	| 'quick'
	| 'customers'
	| 'projects'
	| 'work-logs'
	| 'trips'
	| 'expenses'
	| 'cost-rates'
	| 'invoices'
	| 'payments'
	| 'issuer'
	| 'users'
	| 'audit-log'
	| 'sync'
	| 'settings';

/** Paraglide message key for a nav entry's visible label (i18n layer ②,
 *  ADR-0005). The label itself is resolved at render time via `m[labelKey]()`
 *  so it tracks the active locale — see `pageTitle` below and Sidebar.svelte. */
export type NavLabelKey =
	| 'nav.dashboard'
	| 'nav.calendar'
	| 'nav.quick'
	| 'nav.customers'
	| 'nav.projects'
	| 'nav.workLogs'
	| 'nav.trips'
	| 'nav.expenses'
	| 'nav.costRates'
	| 'nav.invoices'
	| 'nav.payments'
	| 'nav.issuer'
	| 'nav.users'
	| 'nav.auditLog'
	| 'nav.sync'
	| 'nav.settings';

/**
 * ナビの3群区分（docs/mobile-ui-plan.md P1-4）。17項目のフラットな縦一列は
 * 頻度の差が 100 倍ある項目を同格に見せるため、スマホの俯瞰性を優先して
 * 見出しで分ける。並び順は従来のまま。
 */
export type NavGroupKey = 'daily' | 'records' | 'master';

export type NavGroupLabelKey = 'nav.groupDaily' | 'nav.groupRecords' | 'nav.groupMaster';

/** 表示順の群定義。Sidebar / 見出しの描画はこの順に従う。 */
export const navGroups: { key: NavGroupKey; labelKey: NavGroupLabelKey }[] = [
	{ key: 'daily', labelKey: 'nav.groupDaily' },
	{ key: 'records', labelKey: 'nav.groupRecords' },
	{ key: 'master', labelKey: 'nav.groupMaster' }
];

export interface NavItem {
	path: string;
	labelKey: NavLabelKey;
	icon: NavIconKey;
	/** 3群区分（上の `navGroups`）。 */
	group: NavGroupKey;
	/** Spec M10 RBAC: only shown to the `admin` role. Undefined/false = visible to every role. */
	adminOnly?: boolean;
}

export const navItems: NavItem[] = [
	{ path: '/dashboard', group: 'daily', labelKey: 'nav.dashboard', icon: 'dashboard' },
	// カレンダー（Phase 7 準備）。ダッシュボードと同じ「俯瞰する」画面なので
	// 直後に置き、入力系（顧客・案件…）より前に出す。
	{ path: '/calendar', group: 'daily', labelKey: 'nav.calendar', icon: 'calendar' },
	// Business ドメイン（Phase 2 基本マスター）。顧客 → 案件の順は
	// docs/plan.md 第18章 Phase 2 の依存順（Customer → Project）に合わせる。
	// クイック入力（Phase 8 ステップ6）。外出先で一番よく開く画面なので、
	// 入力系の先頭に置く。
	{ path: '/quick', group: 'daily', labelKey: 'nav.quick', icon: 'quick' },
	{ path: '/customers', group: 'records', labelKey: 'nav.customers', icon: 'customers' },
	{ path: '/projects', group: 'records', labelKey: 'nav.projects', icon: 'projects' },
	// Phase 3（工数・経費）。入力頻度の高い順に並べる（工数 → 出張 → 経費）。
	{ path: '/work-logs', group: 'records', labelKey: 'nav.workLogs', icon: 'work-logs' },
	{ path: '/trips', group: 'records', labelKey: 'nav.trips', icon: 'trips' },
	{ path: '/expenses', group: 'records', labelKey: 'nav.expenses', icon: 'expenses' },
	// Phase 5（請求）。請求書は工数・経費の下流なので、入力系の後に置く。
	{ path: '/invoices', group: 'records', labelKey: 'nav.invoices', icon: 'invoices' },
	// Phase 6（入金管理）。請求の下流なので請求の直後に置く。
	{ path: '/payments', group: 'records', labelKey: 'nav.payments', icon: 'payments' },
	{ path: '/cost-rates', group: 'master', labelKey: 'nav.costRates', icon: 'cost-rates' },
	// 事業者情報（適格請求書の発行者）は設定の一種で admin 限定。
	{ path: '/issuer', group: 'master', labelKey: 'nav.issuer', icon: 'issuer', adminOnly: true },
	{ path: '/users', group: 'master', labelKey: 'nav.users', icon: 'users', adminOnly: true },
	{
		path: '/audit-log',
		group: 'master',
		labelKey: 'nav.auditLog',
		icon: 'audit-log',
		adminOnly: true
	},
	// Phase 8（デバイス間同期）。設定の一種で admin 限定。設定の直前に置く
	// のは、デバイス番号とPCのアドレスがアプリ全体の設定だから。
	{ path: '/sync', group: 'master', labelKey: 'nav.sync', icon: 'sync', adminOnly: true },
	{ path: '/settings', group: 'master', labelKey: 'nav.settings', icon: 'settings' }
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
