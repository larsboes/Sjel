import { describe, expect, it } from 'bun:test';
import { apply, dayOf, monthGrid, nearestDay, readState, shiftMonth, writeState, type CollectionState, type Field } from '../src/lib/ui/collection';

type Row = { id: string; kind: string; amount: number | null; name: string };
const rows: Row[] = [
  { id: 'a', kind: 'expense', amount: 30, name: 'Bakery' },
  { id: 'b', kind: 'income', amount: 1000, name: 'Salary' },
  { id: 'c', kind: 'expense', amount: null, name: 'Pending' },
  { id: 'd', kind: 'transfer', amount: 5, name: 'Savings' },
];
const fields: Field<Row>[] = [
  { id: 'name', label: 'Name', value: (r) => r.name },
  { id: 'kind', label: 'Kind', kind: 'select', value: (r) => r.kind },
  { id: 'amount', label: 'Amount', kind: 'number', value: (r) => r.amount },
];
const defaults: CollectionState = { view: 'table', q: '', sort: null, desc: false, group: null, filter: {} };
const ids = (s: Partial<CollectionState>) => apply(rows, fields, { ...defaults, ...s }).map((r) => r.id);

describe('apply', () => {
  it('keeps arrival order when nothing is set', () => {
    expect(ids({})).toEqual(['a', 'b', 'c', 'd']);
  });
  it('filters a select field and searches every field', () => {
    expect(ids({ filter: { kind: ['expense'] } })).toEqual(['a', 'c']);
    expect(ids({ q: 'sal' })).toEqual(['b']);
  });
  it('sorts with empty values last in both directions', () => {
    expect(ids({ sort: 'amount' })).toEqual(['d', 'a', 'b', 'c']);
    expect(ids({ sort: 'amount', desc: true })).toEqual(['b', 'a', 'd', 'c']);
  });
  it('groups in first-seen order, then sorts inside each group', () => {
    expect(ids({ group: 'kind', sort: 'amount', desc: true })).toEqual(['a', 'c', 'b', 'd']);
  });
});

describe('URL state', () => {
  it('round-trips under its own prefix and leaves other keys alone', () => {
    const state: CollectionState = { view: 'board', q: 'x', sort: 'amount', desc: true, group: 'kind', filter: { kind: ['expense', 'income'] } };
    const params = new URLSearchParams('view=transactions&other.q=keep');
    writeState(params, 'tx', state, defaults);
    expect(params.get('view')).toBe('transactions');
    expect(params.get('other.q')).toBe('keep');
    expect(readState(params, 'tx', defaults)).toEqual(state);
  });
  it('writes nothing for the defaults', () => {
    const params = new URLSearchParams();
    writeState(params, 'tx', defaults, defaults);
    expect(params.toString()).toBe('');
  });
  it('can clear a default sort', () => {
    const d = { ...defaults, sort: 'amount', desc: true };
    const params = new URLSearchParams();
    writeState(params, 'tx', { ...d, sort: null, desc: false }, d);
    expect(readState(params, 'tx', d).sort).toBeNull();
  });
});

describe('calendar helpers', () => {
  it('reads a day from a date or a timestamp, and nothing else', () => {
    expect(dayOf('2026-10-08')).toBe('2026-10-08');
    expect(dayOf('2026-10-08T17:00:00Z')).toBe('2026-10-08');
    expect(dayOf(1791495679)).toBeNull();
    expect(dayOf('soon')).toBeNull();
  });
  it('lays a month out in whole weeks from Monday', () => {
    const oct = monthGrid('2026-10'); // 1 Oct 2026 is a Thursday
    expect(oct[0]).toBe('2026-09-28');
    expect(oct.length % 7).toBe(0);
    expect(oct.at(-1)).toBe('2026-11-01');
    expect(monthGrid('2026-02')[0]).toBe('2026-01-26');
  });
  it('opens on the dated row nearest today', () => {
    expect(nearestDay(['2025-11-03', '2026-07-16'], '2026-10-09')).toBe('2026-07-16');
    expect(nearestDay(['2026-10-12', '2025-01-01'], '2026-10-09')).toBe('2026-10-12');
    expect(nearestDay([], '2026-10-09')).toBeNull();
  });
  it('steps months across a year end', () => {
    expect(shiftMonth('2026-12', 1)).toBe('2027-01');
    expect(shiftMonth('2026-01', -1)).toBe('2025-12');
  });
});
