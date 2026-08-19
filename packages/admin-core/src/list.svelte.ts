/**
 * `createListResource` composable (spec §3.4): reactive list state wired to
 * the registered DataProvider, auto-reloading when the resource is
 * invalidated. Mirrors refine's `useTable`.
 *
 * Runes constraint: this class never creates an `$effect` itself (no effect
 * context in a plain module); components wire `$effect`/cleanup around
 * `load()`/`dispose()`.
 */
import { isProviderError, ProviderError } from './errors';
import { onInvalidate } from './invalidate';
import { getDataProvider, notify } from './registry.svelte';
import type { ListParams } from './types';

export interface CreateListResourceOptions {
	initialParams?: Partial<ListParams>;
}

export class ListResource<T> {
	rows: T[] = $state([]);
	totalCount = $state(0);
	loading = $state(false);
	error: ProviderError | null = $state(null);
	params: ListParams = $state({ sort: [], filters: [] });

	#resource: string;
	#unsubscribe: () => void;
	// Monotonically increasing request token (known race, M5): two rapid
	// invalidates each start a `load()`; without a token, a slow-resolving
	// earlier call can overwrite rows/totalCount/error written by a faster
	// *later* call, resurrecting stale data. Only the response matching the
	// current (latest) token is allowed to write state.
	#requestToken = 0;

	constructor(resource: string, options: CreateListResourceOptions = {}) {
		this.#resource = resource;
		this.params = { sort: [], filters: [], ...options.initialParams };
		this.#unsubscribe = onInvalidate(resource, () => {
			void this.load();
		});
	}

	/** Fetch rows for the current `params`. Does not auto-run; call once from the component. */
	async load(): Promise<void> {
		const token = ++this.#requestToken;
		this.loading = true;
		this.error = null;
		try {
			const result = await getDataProvider().getList<T>(this.#resource, this.params);
			if (token !== this.#requestToken) return; // a newer load() already superseded this one
			this.rows = result.rows;
			this.totalCount = result.totalCount;
		} catch (err) {
			if (token !== this.#requestToken) return;
			const providerError = isProviderError(err)
				? err
				: new ProviderError({ kind: 'other', message: String(err) });
			this.error = providerError;
			notify('error', providerError.message);
		} finally {
			if (token === this.#requestToken) this.loading = false;
		}
	}

	/** Stop reacting to invalidate() calls for this resource. Call from the owning component's cleanup. */
	dispose(): void {
		this.#unsubscribe();
	}
}

export function createListResource<T>(
	resource: string,
	options?: CreateListResourceOptions
): ListResource<T> {
	return new ListResource<T>(resource, options);
}
