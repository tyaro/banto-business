<script lang="ts">
	/**
	 * カレンダー（Phase 7 準備）。工数の入力漏れと、締日・支払期限・入金を
	 * 月で俯瞰する。集計はすべてサーバ側（`calendar.rs`）。
	 *
	 * 「今日」をここで一度だけ決めて下へ渡す。各コンポーネントが個別に
	 * `new Date()` を呼ぶと、日付をまたぐ瞬間に画面内で食い違う。
	 */
	import * as m from '$lib/paraglide/messages';
	import PageHeader from '$lib/components/ui/PageHeader.svelte';
	import WorkCalendar from '$lib/components/business/WorkCalendar.svelte';

	/**
	 * 業務日付（JST 等のローカル日付。CLAUDE.md 4）。`toISOString()` は UTC を
	 * 返すので使えない —— 日本時間の朝9時までが前日になってしまう。
	 */
	function localToday(): string {
		const now = new Date();
		const year = now.getFullYear();
		const month = String(now.getMonth() + 1).padStart(2, '0');
		const day = String(now.getDate()).padStart(2, '0');
		return `${year}-${month}-${day}`;
	}

	const today = localToday();
</script>

<div class="page">
	<PageHeader title={m['nav.calendar']()} description={m['calendar.description']()} />
	<WorkCalendar {today} />
</div>

<style>
	.page {
		display: flex;
		flex-direction: column;
		gap: 1rem;
	}
</style>
