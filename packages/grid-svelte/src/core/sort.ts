/**
 * Pure sorting logic for the grid's client mode (spec §4.1, §4.3).
 * No Svelte imports — usable standalone and easy to unit test.
 */
import type { GridColumn, SortDirection, SortState } from '../types';

/** Extract a cell's raw value via the column's accessor. */
export function getColumnValue<TRow>(row: TRow, column: GridColumn<TRow>): unknown {
	return typeof column.accessor === 'function'
		? column.accessor(row)
		: (row[column.accessor] as unknown);
}

function isNullish(value: unknown): boolean {
	return value === null || value === undefined;
}

/** Compare two non-null values: numbers numerically, dates by time, strings via localeCompare. */
function compareNonNull(a: unknown, b: unknown): number {
	if (a instanceof Date && b instanceof Date) return a.getTime() - b.getTime();
	if (typeof a === 'number' && typeof b === 'number') return a - b;
	if (typeof a === 'string' && typeof b === 'string') return a.localeCompare(b);
	return String(a).localeCompare(String(b));
}

function compareForSort<TRow>(
	a: TRow,
	b: TRow,
	column: GridColumn<TRow>,
	direction: SortDirection
): number {
	const va = getColumnValue(a, column);
	const vb = getColumnValue(b, column);
	const aNull = isNullish(va);
	const bNull = isNullish(vb);
	// Nulls/undefined always sort last, regardless of direction.
	if (aNull && bNull) return 0;
	if (aNull) return 1;
	if (bNull) return -1;

	const base = column.comparator ? column.comparator(va, vb) : compareNonNull(va, vb);
	return direction === 'asc' ? base : -base;
}

/**
 * Stable multi-column sort. Returns a new array; `rows` is not mutated.
 * Sort priority follows the order of entries in `sort`.
 */
export function sortRows<TRow>(
	rows: TRow[],
	sort: SortState[],
	columns: GridColumn<TRow>[]
): TRow[] {
	if (sort.length === 0) return rows.slice();

	const columnMap = new Map(columns.map((column) => [column.id, column]));
	const indexed = rows.map((row, index) => ({ row, index }));

	indexed.sort((a, b) => {
		for (const entry of sort) {
			const column = columnMap.get(entry.field);
			if (!column) continue;
			const result = compareForSort(a.row, b.row, column, entry.direction);
			if (result !== 0) return result;
		}
		// Explicit index tie-breaker guarantees stability independent of the
		// host engine's Array#sort implementation.
		return a.index - b.index;
	});

	return indexed.map((entry) => entry.row);
}
