/**
 * Vite's own `vite/client` types already declare a generic `'*?raw'`
 * wildcard module (any import ending in `?raw` resolves to `string`), but
 * that ambient file is only pulled into the TS program via an explicit
 * `/// <reference types="vite/client" />` somewhere - which nothing in this
 * app currently has (SvelteKit's generated `ambient.d.ts` only references
 * `@sveltejs/kit`). Declaring the exact suffix this app actually uses here
 * keeps `daily.md?raw` (items/report/+page.svelte) type-checked without
 * relying on that being pulled in indirectly.
 */
declare module '*.md?raw' {
	const content: string;
	export default content;
}
