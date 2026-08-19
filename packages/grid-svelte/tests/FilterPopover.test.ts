// @vitest-environment jsdom
/**
 * FilterPopover interaction/dismiss test (spec §4.3, improvement-plan P4-1).
 *
 * improvements.md §8 flagged the popover's "focus trap" as unverified. On
 * inspection the popover does NOT implement a Tab-cycling focus trap; its
 * focus/interaction boundary is instead a DISMISS model: it closes on Escape
 * and on a pointer-down anywhere outside its root, while a pointer-down
 * inside is left alone. These tests pin exactly that behavior (plus the
 * dialog semantics and apply/clear wiring), so the §8 item is resolved by
 * documenting + covering the real contract rather than a trap that isn't there.
 *
 * The outside/inside pointer-down cases dispatch a plain `Event('pointerdown')`
 * rather than a `PointerEvent` (whose constructor jsdom does not provide) -
 * the component only ever reads `event.target`/`event.type`, so a generic
 * bubbling event exercises the exact capture-phase window listener it registers.
 */
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import FilterPopover from '../src/FilterPopover.svelte';
import type { GridColumn } from '../src/types';

afterEach(cleanup);

interface Row {
	name: string;
	price: number;
}

const textColumn: GridColumn<Row> = {
	id: 'name',
	header: '商品名',
	accessor: 'name',
	filterable: true,
	filterType: 'text'
};

function renderPopover() {
	const handlers = {
		onApply: vi.fn(),
		onClear: vi.fn(),
		onClose: vi.fn()
	};
	const { unmount } = render(FilterPopover<Row>, {
		column: textColumn,
		current: undefined,
		...handlers
	});
	return { ...handlers, unmount };
}

function pointerDown(target: EventTarget) {
	target.dispatchEvent(new Event('pointerdown', { bubbles: true }));
}

describe('FilterPopover', () => {
	it('exposes a labelled dialog', () => {
		renderPopover();
		expect(screen.getByRole('dialog', { name: '商品名の絞り込み' })).toBeTruthy();
	});

	it('closes on Escape', async () => {
		const { onClose } = renderPopover();
		await fireEvent.keyDown(document.body, { key: 'Escape' });
		expect(onClose).toHaveBeenCalledTimes(1);
	});

	it('does not close on other keys', async () => {
		const { onClose } = renderPopover();
		await fireEvent.keyDown(document.body, { key: 'a' });
		expect(onClose).not.toHaveBeenCalled();
	});

	it('closes on a pointer-down outside its root', () => {
		const { onClose } = renderPopover();
		const outside = document.createElement('button');
		document.body.appendChild(outside);
		pointerDown(outside);
		expect(onClose).toHaveBeenCalledTimes(1);
		outside.remove();
	});

	it('stays open on a pointer-down inside its root', () => {
		const { onClose } = renderPopover();
		pointerDown(screen.getByRole('dialog').querySelector('input')!);
		expect(onClose).not.toHaveBeenCalled();
	});

	it('removes its window listeners on unmount (no dismiss after close)', () => {
		const { onClose, unmount } = renderPopover();
		unmount();
		// After teardown, a stray outside pointer-down must not reach a
		// leaked listener (the $effect cleanup removed it).
		pointerDown(document.body);
		expect(onClose).not.toHaveBeenCalled();
	});

	it('applies the entered value via the 適用 button', async () => {
		const { onApply, onClear } = renderPopover();
		await fireEvent.input(screen.getByPlaceholderText('値を入力'), { target: { value: 'ペン' } });
		await fireEvent.click(screen.getByRole('button', { name: '適用' }));
		expect(onApply).toHaveBeenCalledWith({ field: 'name', op: 'contains', value: 'ペン' });
		expect(onClear).not.toHaveBeenCalled();
	});

	it('treats applying an empty value as clearing the filter', async () => {
		const { onApply, onClear } = renderPopover();
		await fireEvent.click(screen.getByRole('button', { name: '適用' }));
		expect(onClear).toHaveBeenCalledTimes(1);
		expect(onApply).not.toHaveBeenCalled();
	});

	it('applies on Enter in the value input', async () => {
		const { onApply } = renderPopover();
		const input = screen.getByPlaceholderText('値を入力');
		await fireEvent.input(input, { target: { value: 'ノート' } });
		await fireEvent.keyDown(input, { key: 'Enter' });
		expect(onApply).toHaveBeenCalledWith({ field: 'name', op: 'contains', value: 'ノート' });
	});
});
