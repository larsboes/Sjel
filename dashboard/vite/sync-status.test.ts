import { describe, expect, test } from 'bun:test';
import type { SyncStatus } from '../src/lib/mac-bridge';
import { clockTime, fieldDiff, needsReview, showValue, statusLine, statusParts } from '../src/lib/sync-status';

// Sync step 2 (PRD §10 A5): the words of the app's sync status line and the conflicts view's
// field diff. The store and flush rules behind them are tested in src-tauri/src/sync.rs.

const quiet: SyncStatus = {
  offline: false,
  offline_since: null,
  showing_from: null,
  pending: 0,
  conflicts: 0,
  failed: 0,
  store_error: null,
};

const time = (ms: number) => `t${ms}`;

describe('statusLine', () => {
  test('says nothing when the canonical node answers and nothing waits', () => {
    expect(statusLine(quiet)).toBeNull();
    expect(statusParts(quiet)).toEqual([]);
  });

  test('offline names the time of the data shown', () => {
    expect(statusLine({ ...quiet, offline: true, offline_since: 5, showing_from: 1 }, time)).toBe(
      'Offline — showing data from t1',
    );
  });

  test('offline before any copy was shown says the canonical node does not answer', () => {
    expect(statusLine({ ...quiet, offline: true, offline_since: 5 }, time)).toBe('Offline — the canonical node does not answer');
  });

  test('counts conflicts, refusals and waiting changes, with singular and plural', () => {
    expect(statusLine({ ...quiet, pending: 1 })).toBe('1 change waiting');
    expect(statusLine({ ...quiet, pending: 3 })).toBe('3 changes waiting');
    expect(statusLine({ ...quiet, conflicts: 1 })).toBe('1 conflict');
    expect(
      statusLine({ ...quiet, offline: true, showing_from: 1, conflicts: 2, failed: 1, pending: 4 }, time),
    ).toBe('Offline — showing data from t1 · 2 conflicts · 1 change refused by the canonical node · 4 changes waiting');
  });

  test('a store that did not open is said, not hidden', () => {
    expect(statusLine({ ...quiet, store_error: 'disk full' })).toBe('Offline copy unavailable: disk full');
  });

  test('review is needed for conflicts and refusals, not for waiting changes', () => {
    expect(needsReview({ ...quiet, pending: 2 })).toBe(false);
    expect(needsReview({ ...quiet, conflicts: 1 })).toBe(true);
    expect(needsReview({ ...quiet, failed: 1 })).toBe(true);
  });

  test('clockTime is HH:MM', () => {
    expect(clockTime(new Date(2026, 8, 25, 7, 5).getTime())).toBe('07:05');
  });
});

describe('fieldDiff', () => {
  test('shows each named field that differs, mine next to the canonical node value', () => {
    const diff = fieldDiff({
      body: { b: 120, label: 'Schrank', hinweis: null },
      current: { item: { id: 'schrank', b: 90, label: 'Schrank', hinweis: 'alt', revision: 2 }, state: 'owned' },
    });
    expect(diff).toEqual([
      { field: 'b', mine: 120, theirs: 90 },
      { field: 'hinweis', mine: null, theirs: 'alt' },
    ]);
  });

  test('skips server-owned keys and treats absent as null', () => {
    const diff = fieldDiff({
      body: { id: 'x', revision: 1, expected_revision: 1, pending: true, bild: null, tags: ['a'] },
      current: { item: { tags: ['a'] }, state: null },
    });
    expect(diff).toEqual([]);
  });

  test('without the canonical node item every field shows with theirs null', () => {
    expect(fieldDiff({ body: { b: 1 }, current: null })).toEqual([{ field: 'b', mine: 1, theirs: null }]);
  });

  test('showValue prints empty as a dash and structures as JSON', () => {
    expect(showValue(null)).toBe('—');
    expect(showValue('')).toBe('—');
    expect(showValue('Lampe')).toBe('Lampe');
    expect(showValue(120)).toBe('120');
    expect(showValue(['a'])).toBe('["a"]');
  });
});

describe('a queued item edit', () => {
  test('resolves with queued: true instead of failing, and sends If-Match across the bridge', async () => {
    const { inventory } = await import('../src/lib/api');
    const g = globalThis as unknown as { window?: Record<string, unknown> };
    const seen: Record<string, unknown>[] = [];
    g.window = {
      __TAURI_INTERNALS__: {
        invoke: async (_cmd: string, args: Record<string, unknown>) => {
          seen.push(args);
          return {
            status: 202,
            content_type: 'application/json',
            body: JSON.stringify({ queued: true, outbox_id: 7, state: 'pending' }),
          };
        },
        transformCallback: () => 0,
      },
    };
    try {
      const result = await inventory.patchItem('schrank', { b: 120 }, 1);
      expect(result).toEqual({ queued: true, outbox_id: 7, state: 'pending' });
      const request = seen[0].request as { headers: [string, string][] };
      expect(request.headers).toContainEqual(['If-Match', '"1"']);
    } finally {
      delete g.window;
    }
  });
});
