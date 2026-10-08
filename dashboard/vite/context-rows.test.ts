import { describe, expect, it } from 'bun:test';
import { contextHref, ordered, shiftDay, spent, type ContextRow } from '../src/lib/context/context';

const row = (key: string, day: string | null, time: string | null, extra: Partial<ContextRow> = {}): ContextRow => ({
  key, day, time, kind: 'event', title: key, meta: '', amount: null, currency: null, inactive: false, item: null, href: null, ...extra,
});

describe('ordered', () => {
  it('sorts by day, all-day rows first, undated days last, and drops repeats', () => {
    const rows = [row('c', '2026-10-09', null), row('x', null, null), row('b', '2026-10-08', '18:00'), row('a', '2026-10-08', '09:00'), row('a', '2026-10-08', '09:00'), row('all', '2026-10-08', null)];
    expect(ordered(rows).map((r) => r.key)).toEqual(['all', 'a', 'b', 'c', 'x']);
  });
});

describe('spent', () => {
  it('sums expenses per currency and leaves out income and transfers', () => {
    const rows = [
      row('e1', '2026-10-08', null, { kind: 'spend', amount: -1250, currency: 'EUR' }),
      row('e2', '2026-10-08', null, { kind: 'spend', amount: -750, currency: 'EUR' }),
      row('in', '2026-10-08', null, { kind: 'spend', amount: 5000, currency: 'EUR' }),
      row('tr', '2026-10-08', null, { kind: 'spend', amount: -9999, currency: 'EUR', inactive: true }),
      row('chf', '2026-10-08', null, { kind: 'spend', amount: -300, currency: 'CHF' }),
    ];
    expect([...spent(rows)]).toEqual([['EUR', 2000], ['CHF', 300]]);
  });
});

describe('days and links', () => {
  it('steps across a month end', () => {
    expect(shiftDay('2026-10-31', 1)).toBe('2026-11-01');
    expect(shiftDay('2026-03-01', -1)).toBe('2026-02-28');
  });
  it('sends each record kind to its context', () => {
    expect(contextHref({ type: 'trip', id: 'trip:plan:1', title: 't', destination: 'd', dates: '' })).toBe('/context?trip=trip%3Aplan%3A1');
    expect(contextHref({ type: 'event', title: 'e', startsAt: '2026-10-08T09:00' })).toBe('/context?day=2026-10-08');
    expect(contextHref({ type: 'layout', id: 'l', name: 'n', pass: true })).toBeNull();
  });
});
