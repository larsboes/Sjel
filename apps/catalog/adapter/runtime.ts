import { createStateStore } from '@json-render/core';
import { createClientStateStore } from './client-state';
import { rowsFrom, resolveBinding, type Binding, type ObjectType, type Row } from '../src/bindings';

export type ReadSnapshot = { object: ObjectType; rows: Row[] };

/** Create separate host and client handles over classified read snapshots and local drafts. */
export function createViewState(bindings: Readonly<Record<string, Binding>>, responses: Readonly<Record<string, unknown>>) {
  const snapshots: Record<string, ReadSnapshot> = {};
  for (const [id, binding] of Object.entries(bindings)) {
    snapshots[id] = { object: resolveBinding(binding).object, rows: rowsFrom(binding, responses[id]) };
  }
  const host = createStateStore(structuredClone({ snapshot: snapshots, draft: { note: '' }, ui: { selection: null } }));
  return { host, client: createClientStateStore(host) };
}

/** Resolve a local selection against current host rows rather than storing a mutable row copy. */
export function selectedRow(snapshot: ReadSnapshot, selection: unknown, binding: string): Row | undefined {
  if (!selection || typeof selection !== 'object' || !('binding' in selection) || selection.binding !== binding || !('identity' in selection)) return;
  const identity = JSON.stringify(selection.identity);
  return snapshot.rows.find(row => JSON.stringify(snapshot.object.identity.map(key => row[key])) === identity);
}
