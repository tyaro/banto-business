/**
 * The list of resources this app registers with `initBanto()` - the single
 * place a new resource gets added (docs/recipes/add-resource.md step 7).
 * setup.ts passes this array unchanged to all three provider environments,
 * so registering here is all it takes for Tauri, LAN-browser, and demo mode
 * alike.
 */
import type { ResourceDefinition } from '@banto/admin-core';
import { customersResource } from './customers';
import { expensesResource } from './expenses';
import { projectsResource } from './projects';
import { tripsResource } from './trips';
import { workLogsResource } from './work-logs';

export const resources: ResourceDefinition[] = [
	// Business ドメイン。Banto テンプレート同梱の `items` デモは
	// Phase 7（実運用）の前に削除した。
	customersResource,
	projectsResource,
	workLogsResource,
	tripsResource,
	expensesResource
];
