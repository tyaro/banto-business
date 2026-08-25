<script lang="ts">
	/**
	 * ダッシュボード。
	 *
	 * Banto テンプレート同梱の `items` デモ（商品データから作ったチャート群と
	 * ドックレイアウト）は Phase 7 の実運用前に削除した。現時点で置いているのは
	 * 初期セットアップの道しるべ（P2-1、未完了の間だけ）と回収の当たり
	 * （未入金・期限超過、要件 F-Y7）の2つ。
	 *
	 * 業務データを使った指標を並べるのは、Phase 7 の実運用評価で「毎日何を見たいか」
	 * が決まってから（`docs/plan.md` 第18章 Phase 7）。使う前に指標を推測して
	 * 並べても、結局見ないパネルが残る。
	 */
	import * as m from '$lib/paraglide/messages';
	import PageHeader from '$lib/components/ui/PageHeader.svelte';
	import OutstandingPanel from '$lib/components/business/OutstandingPanel.svelte';
	import SetupGuidePanel from '$lib/components/business/SetupGuidePanel.svelte';
</script>

<div class="page">
	<PageHeader title={m['nav.dashboard']()} description={m['dashboard.summary']()} />

	<!-- P2-1: 未完了の間だけ、未入金パネルより上に表示する（自身が判断する）。 -->
	<SetupGuidePanel />

	<OutstandingPanel />
</div>

<style>
	.page {
		display: flex;
		flex-direction: column;
		gap: 1.25rem;
	}
</style>
