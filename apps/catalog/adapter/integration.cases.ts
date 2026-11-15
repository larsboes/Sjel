import { describe, expect, test } from 'bun:test';
import { createStateStore, type StateStore } from '@json-render/core';
import { createClientStateStore } from './client-state';
import { parseView, rendererSpec } from './document';
import { createFixtureState, fixtureBindings, fixtureDocument } from './fixture-state';
import { selectedRow, type ReadSnapshot } from './runtime';

describe('json-render store integration', () => {
  test('real immutable store accepts drafts after shared snapshots are frozen', () => {
    const host = createStateStore({ snapshot: { title: 'Original' }, draft: {}, ui: {} });
    const client: StateStore = createClientStateStore(host);
    const before = client.getSnapshot();
    client.set('/draft/note', 'Proposed');
    client.update({ '/ui/selected': 'fictional-1', '/draft/confirmed': false });
    expect(client.get('/draft/note')).toBe('Proposed');
    expect(client.get('/ui/selected')).toBe('fictional-1');
    expect(client.getSnapshot().snapshot).toBe(before.snapshot);
    host.update({ '/snapshot/title': 'Refreshed' });
    expect(client.get('/snapshot/title')).toBe('Refreshed');
    expect(client.get('/draft/note')).toBe('Proposed');
  });

  test('direct bindings and built-in action destinations cannot overwrite snapshots', () => {
    const { client, host } = createFixtureState();
    const before = host.getSnapshot();
    for (const path of ['/snapshot', '/snapshot/records/rows/0/title', '/currentScreen', '/formValidation']) {
      expect(() => client.set(path, 'Forged')).toThrow();
    }
    expect(() => client.update({ '/draft/note': 'Draft', '/snapshot': {} })).toThrow();
    expect(host.getSnapshot()).toBe(before);
  });

  test('selection resolves against refreshed host rows while drafts survive', () => {
    const { client, host } = createFixtureState();
    const snapshot = client.get('/snapshot/records') as ReadSnapshot;
    const selection = { binding: 'records', identity: snapshot.object.identity.map(key => snapshot.rows[0][key]) };
    client.set('/ui/selection', selection);
    client.set('/draft/note', 'Keep this draft');
    host.set('/snapshot/records', { ...snapshot, rows: snapshot.rows.map((row, i) => i ? row : { ...row, title: 'Updated title' }) });
    const refreshed = client.get('/snapshot/records') as ReadSnapshot;
    expect(selectedRow(refreshed, client.get('/ui/selection'), 'records')?.title).toBe('Updated title');
    expect(client.get('/draft/note')).toBe('Keep this draft');
    host.set('/snapshot/records', { ...refreshed, rows: [] });
    expect(selectedRow(client.get('/snapshot/records') as ReadSnapshot, selection, 'records')).toBeUndefined();
  });
});

describe('candidate view admission', () => {
  const saved = JSON.stringify(fixtureDocument);
  test('saved documents reload without a model and only gain host-authored actions', () => {
    const doc = parseView(saved, fixtureBindings);
    expect(parseView(JSON.stringify(doc), fixtureBindings)).toEqual(doc);
    expect(rendererSpec(doc).elements.draft.on).toEqual({ submit: { action: 'propose' } });
  });

  test('refuses executable content, model-authored actions and unknown fields', () => {
    for (const change of [
      { type: 'script', props: {}, children: [] },
      { ...fixtureDocument.elements.list, on: { click: { action: 'fetch' } } },
      { ...fixtureDocument.elements.list, props: { binding: 'records', url: 'https://example.org' } },
    ]) {
      expect(() => parseView(JSON.stringify({ ...fixtureDocument, elements: { ...fixtureDocument.elements, list: change } }), fixtureBindings)).toThrow('list:');
    }
    expect(() => parseView(JSON.stringify({ ...fixtureDocument, state: { snapshot: {} } }), fixtureBindings)).toThrow('root:');
  });

  test('refuses unknown and write-route bindings with the failing node', () => {
    expect(() => parseView(saved, {})).toThrow('list:');
    expect(() => parseView(saved, { records: { ...fixtureBindings.records, read: 'delete' } })).toThrow('list:');
  });

  test('refuses missing nodes, cycles, repeated references and unreachable content', () => {
    for (const children of [['missing'], ['root'], ['list', 'list']]) {
      expect(() => parseView(JSON.stringify({ ...fixtureDocument, elements: { ...fixtureDocument.elements, root: { type: 'Stack', props: {}, children } } }), fixtureBindings)).toThrow();
    }
    expect(() => parseView(JSON.stringify({ ...fixtureDocument, elements: { ...fixtureDocument.elements, unused: { type: 'Stack', props: {}, children: [] } } }), fixtureBindings)).toThrow('unreachable');
  });

  test('enforces byte, component and depth limits', () => {
    expect(() => parseView(' '.repeat(65537), fixtureBindings)).toThrow('64 KiB');
    const deep = { version: 1, root: 'n0', elements: Object.fromEntries(Array.from({ length: 10 }, (_, i) => [`n${i}`, { type: 'Stack', props: {}, children: i === 9 ? [] : [`n${i + 1}`] }])) };
    expect(() => parseView(JSON.stringify(deep), fixtureBindings)).toThrow('maximum depth');
    const wide = { version: 1, root: 'root', elements: Object.fromEntries(Array.from({ length: 65 }, (_, i) => [i ? `n${i}` : 'root', { type: 'Stack', props: {}, children: [] }])) };
    expect(() => parseView(JSON.stringify(wide), fixtureBindings)).toThrow('too many');
  });
});
