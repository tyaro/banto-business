/**
 * Icon resolution for navigation entries (visual-refresh-design.md §5.2).
 *
 * `navigation.ts` stays UI-agnostic and only holds the `NavIconKey` string
 * key; the actual icon component is resolved here, in the display layer.
 */
import type { Component } from 'svelte';
import {
	LayoutDashboard,
	CalendarDays,
	Building2,
	FolderKanban,
	Clock,
	TrainFront,
	Receipt,
	Coins,
	FileText,
	Banknote,
	Building,
	ListTree,
	Users,
	ScrollText,
	RefreshCw,
	Settings
} from '@lucide/svelte';
import type { NavIconKey } from '$lib/navigation';

export const NAV_ICONS: Record<NavIconKey, Component> = {
	dashboard: LayoutDashboard,
	calendar: CalendarDays,
	customers: Building2,
	projects: FolderKanban,
	'work-logs': Clock,
	trips: TrainFront,
	expenses: Receipt,
	'cost-rates': Coins,
	invoices: FileText,
	payments: Banknote,
	issuer: Building,
	tree: ListTree,
	users: Users,
	'audit-log': ScrollText,
	sync: RefreshCw,
	settings: Settings
};
