import { defineConfig } from 'vitest/config';

/**
 * app 層の**純粋な TypeScript** の単体テスト用。
 *
 * `vite.config.ts` とは別ファイルにしてある。あちらは SvelteKit /
 * Paraglide / Tailwind のプラグインを読み込むため、テストを走らせるだけで
 * `svelte-kit sync` 済みであることや i18n の再生成が要る。ロジックだけを
 * 検査するのにその前提を持ち込むと、CI で「テストは正しいのに環境で落ちる」
 * が起きる。
 *
 * `.svelte` コンポーネントのテストはここでは扱わない（CLAUDE.md 第6章:
 * UI のスナップショットテストは必須としない）。対象は `.test.ts` のみ。
 */
export default defineConfig({
	test: {
		include: ['src/**/*.test.ts'],
		environment: 'node'
	}
});
