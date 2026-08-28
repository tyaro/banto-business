<script lang="ts">
	/**
	 * パスワード入力欄 + 表示/非表示トグル（目のアイコン）。
	 *
	 * アルファ実機フィードバック: スマホでの入力ミスに気付けるよう、値を
	 * 一時的に平文表示できるようにする。app 層専用コンポーネント（@banto/* は
	 * 改変しない — CLAUDE.md 2章。`packages/forms` の `PasswordField.svelte` は
	 * パッケージ側が描画する password 欄なので対象外）。
	 *
	 * 既存の login ページが使っていた
	 * `<label>{文言}<input class="banto-input" .../></label>` 構造を踏襲する:
	 * ラベルテキストは `<label>` 直下のテキストノードのまま残し、トグル
	 * ボタンは `<input>` より後ろに置く。`<label>` の暗黙のラベル付け対象は
	 * 「ツリー順で最初にラベル可能な子孫」= `<input>` のままなので、
	 * `getByLabel('パスワード')` 等の既存 E2E ロケータがそのまま効く。
	 * トグルボタンは `type="button"` なのでフォーム送信を起こさない。
	 */
	import { Eye, EyeOff } from '@lucide/svelte';
	import * as m from '$lib/paraglide/messages';

	interface Props {
		label: string;
		value: string;
		autocomplete?: 'current-password' | 'new-password';
		disabled?: boolean;
	}

	let {
		label,
		value = $bindable(),
		autocomplete = 'current-password',
		disabled = false
	}: Props = $props();

	let visible = $state(false);
</script>

<label class="password-field">
	{label}
	<span class="input-wrap">
		<input
			class="banto-input"
			type={visible ? 'text' : 'password'}
			bind:value
			{autocomplete}
			{disabled}
		/>
		<button
			type="button"
			class="toggle"
			aria-label={visible ? m['auth.hidePassword']() : m['auth.showPassword']()}
			aria-pressed={visible}
			{disabled}
			onclick={() => (visible = !visible)}
		>
			{#if visible}
				<EyeOff size={18} aria-hidden="true" />
			{:else}
				<Eye size={18} aria-hidden="true" />
			{/if}
		</button>
	</span>
</label>

<style>
	.password-field {
		display: flex;
		flex-direction: column;
		gap: 0.3rem;
		font-size: 0.875rem;
		color: var(--banto-text-muted);
	}

	.input-wrap {
		position: relative;
		display: block;
	}

	/* 右端にトグルを収める分、右パディングを広げる（.banto-input のトーンは
	   維持したまま app.css の定義を上書きしない - ここは幅と余白の調整のみ）。 */
	.input-wrap .banto-input {
		width: 100%;
		padding-right: 2.5rem;
		box-sizing: border-box;
	}

	.toggle {
		position: absolute;
		top: 50%;
		right: 0.25rem;
		transform: translateY(-50%);
		display: inline-flex;
		align-items: center;
		justify-content: center;
		/* タッチターゲット44px目安（モバイルでの誤タップ対策）。入力欄自体の
		   高さ（--banto-control-height、標準密度で36px）より大きくなるが、
		   フォームの行間ガター内に収まるので視覚的な衝突は無い。 */
		width: 44px;
		height: 44px;
		border: none;
		border-radius: var(--banto-radius-md);
		background: transparent;
		color: var(--banto-text-muted);
		cursor: pointer;
		transition:
			background var(--banto-duration-fast) var(--banto-ease-out),
			color var(--banto-duration-fast) var(--banto-ease-out);
	}

	.toggle:hover:not(:disabled) {
		background: var(--banto-surface-hover);
		color: var(--banto-text);
	}

	.toggle:focus-visible {
		outline: none;
		box-shadow: var(--banto-focus-ring);
	}

	.toggle:disabled {
		cursor: not-allowed;
		opacity: 0.5;
	}
</style>
