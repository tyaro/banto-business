/**
 * The list of resources this app registers with `initBanto()` - the single
 * place a new resource gets added (docs/recipes/add-resource.md step 7).
 * setup.ts passes this array unchanged to all three provider environments,
 * so registering here is all it takes for Tauri, LAN-browser, and demo mode
 * alike.
 */
import type { ResourceDefinition } from '@banto/admin-core';
import { customersResource } from './customers';
import { itemsResource } from './items';
import { projectsResource } from './projects';

export const resources: ResourceDefinition[] = [
	// Business ドメイン（Phase 2 基本マスター）。`items` は Banto テンプレート
	// 同梱のデモリソースで、Phase 2 完了後に削除する
	// （docs/domain/open-questions.md の items デモの扱い）。
	customersResource,
	projectsResource,
	itemsResource
];
