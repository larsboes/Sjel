import { describe, expect, test } from 'bun:test';
import { createClientStateStore, type RendererStateStore } from './client-state';

function fixture() {
  let snapshot: Record<string, unknown> = {
    snapshot: { records: [{ id: 'fictional-1', title: 'Initial' }] },
    draft: {},
    ui: {},
  };
  const writes: unknown[][] = [];
  const listeners = new Set<() => void>();
  const notify = () => { for (const listener of listeners) listener(); };
  const host: RendererStateStore = {
    get: path => path === '' ? snapshot : snapshot[path.slice(1)],
    getSnapshot: () => snapshot,
    getServerSnapshot: () => snapshot,
    set(path, value) {
      writes.push([path, value]);
      snapshot = { ...snapshot, [path.slice(1)]: value };
      notify();
    },
    update(updates) {
      writes.push([updates]);
      snapshot = { ...snapshot, ...Object.fromEntries(Object.entries(updates).map(([path, value]) => [path.slice(1), value])) };
      notify();
    },
    subscribe(listener) { listeners.add(listener); return () => { listeners.delete(listener); }; },
  };
  return { host, client: createClientStateStore(host), writes };
}

describe('renderer state authority', () => {
  test('permits local drafts and UI updates without changing the host snapshot', () => {
    const { client } = fixture();
    const original = client.get('/snapshot');
    client.set('/draft', { title: 'Proposed' });
    client.update({ '/ui': { selected: 'fictional-1' } });
    expect(client.get('/draft')).toEqual({ title: 'Proposed' });
    expect(client.get('/ui')).toEqual({ selected: 'fictional-1' });
    expect(client.get('/snapshot')).toBe(original);
  });

  test('rejects root replacement and writes outside local namespaces', () => {
    const { client, writes } = fixture();
    for (const path of ['', '/', '/snapshot', '/snapshot/records/0', '/drafts', '/draft~1snapshot', '/currentScreen', '/navStack']) {
      expect(() => client.set(path, 'changed')).toThrow();
    }
    expect(writes).toEqual([]);
  });

  test('rejects malformed pointers and prototype traversal before delegation', () => {
    const { client, writes } = fixture();
    for (const path of ['draft/title', '/draft/~2', '/ui/__proto__/x', '/draft/constructor/prototype/x']) {
      expect(() => client.set(path, true)).toThrow();
    }
    client.set('/draft/a~1b~0c', 'literal pointer segment');
    expect(writes).toEqual([['/draft/a~1b~0c', 'literal pointer segment']]);
  });

  test('rejects an entire mixed batch before any write or notification', () => {
    const { client, writes } = fixture();
    let notifications = 0;
    client.subscribe(() => notifications++);
    expect(() => client.update({ '/draft': { title: 'Proposed' }, '/snapshot': {} })).toThrow();
    expect(writes).toEqual([]);
    expect(notifications).toBe(0);
    expect(client.get('/draft')).toEqual({});
  });

  test('freezes nested records and arrays through all read methods', () => {
    const { client } = fixture();
    for (const read of [() => client.get('/snapshot'), () => client.getSnapshot().snapshot, () => client.getServerSnapshot!().snapshot]) {
      const state = read() as { records: { title: string }[] };
      expect(Object.isFrozen(state)).toBe(true);
      expect(Object.isFrozen(state.records)).toBe(true);
      expect(() => { state.records[0].title = 'Forged'; }).toThrow();
      expect(() => { state.records.push({ title: 'Injected' }); }).toThrow();
    }
  });

  test('detaches renderer-owned values before storing them', () => {
    const { client } = fixture();
    const draft = { fields: { title: 'Proposed' } };
    client.set('/draft', draft);
    draft.fields.title = 'Changed after submission';
    expect(client.get('/draft')).toEqual({ fields: { title: 'Proposed' } });
    const ui = { selected: ['fictional-1'] };
    client.update({ '/ui': ui });
    ui.selected.push('fictional-2');
    expect(client.get('/ui')).toEqual({ selected: ['fictional-1'] });
  });

  test('host updates remain possible and subscriptions unsubscribe', () => {
    const { host, client } = fixture();
    const before = client.getSnapshot();
    let notifications = 0;
    const unsubscribe = client.subscribe(() => notifications++);
    host.set('/snapshot', { records: [{ id: 'fictional-1', title: 'Refreshed' }] });
    expect(client.getSnapshot()).not.toBe(before);
    expect(client.get('/snapshot')).toEqual({ records: [{ id: 'fictional-1', title: 'Refreshed' }] });
    expect(notifications).toBe(1);
    unsubscribe();
    host.set('/snapshot', { records: [] });
    expect(notifications).toBe(1);
  });

  test('preserves the absence of optional server snapshots', () => {
    const { host } = fixture();
    delete host.getServerSnapshot;
    expect(createClientStateStore(host).getServerSnapshot).toBeUndefined();
  });
});
