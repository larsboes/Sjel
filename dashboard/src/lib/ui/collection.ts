/**
 * One dataset, several views, with filter, sort and group: Notion's database pattern
 * (2026-10-08). The state lives in the URL query, so a view can be bookmarked and shared and
 * needs no backend. Each collection prefixes its keys with its id, so two collections on one
 * page do not read each other's state: `tx.view=board&tx.sort=-amount&tx.f.kind=expense`.
 *
 * Pure on purpose: `Collection.svelte` renders, this decides which rows show and in what order.
 */
import type { Snippet } from 'svelte';
import type { Tone } from './Chip.svelte';

export interface Field<T> {
  id: string;
  label: string;
  /** "select" is a value from a fixed set: filterable, groupable, a board column. */
  kind?: 'text' | 'number' | 'date' | 'select';
  /** What sorting, filtering, grouping and search compare. */
  value: (row: T) => string | number | null | undefined;
  /** How a select value reads. Defaults to the value itself. */
  display?: (value: string) => string;
  /** What a select value means (Chip.svelte). Defaults to neutral. */
  tone?: (value: string) => Tone;
  /** Table column. Leave `cell` unset for the default rendering of `value`. */
  width?: string;
  align?: 'start' | 'end';
  cell?: Snippet<[T]>;
  /** False keeps the field out of the table; it still filters, sorts and groups. */
  column?: boolean;
}

export interface CollectionState {
  view: string;
  q: string;
  /** Field id, or null for the order the rows arrived in. */
  sort: string | null;
  desc: boolean;
  /** Select field id the table groups by and the board splits into columns. */
  group: string | null;
  /** Select field id → the values kept. A field with no entry is not filtered. */
  filter: Record<string, string[]>;
}

const RESERVED = ['view', 'q', 'sort', 'group'];

export function readState(params: URLSearchParams, id: string, defaults: CollectionState): CollectionState {
  const key = (name: string) => `${id}.${name}`;
  const sort = params.get(key('sort'));
  const filter: Record<string, string[]> = {};
  const prefix = key('f.');
  for (const name of new Set(params.keys())) {
    if (name.startsWith(prefix)) filter[name.slice(prefix.length)] = params.getAll(name);
  }
  return {
    view: params.get(key('view')) ?? defaults.view,
    q: params.get(key('q')) ?? defaults.q,
    sort: sort === null ? defaults.sort : sort.replace(/^-/, '') || null,
    desc: sort === null ? defaults.desc : sort.startsWith('-'),
    group: params.has(key('group')) ? params.get(key('group')) || null : defaults.group,
    filter: Object.keys(filter).length ? filter : defaults.filter,
  };
}

/** Writes only what differs from the defaults, so an untouched view keeps a clean URL. */
export function writeState(params: URLSearchParams, id: string, state: CollectionState, defaults: CollectionState): void {
  const key = (name: string) => `${id}.${name}`;
  for (const name of [...params.keys()]) {
    if (name.startsWith(`${id}.f.`) || RESERVED.some((r) => name === key(r))) params.delete(name);
  }
  if (state.view !== defaults.view) params.set(key('view'), state.view);
  if (state.q) params.set(key('q'), state.q);
  if (state.sort !== defaults.sort || state.desc !== defaults.desc) {
    params.set(key('sort'), state.sort ? `${state.desc ? '-' : ''}${state.sort}` : '');
  }
  if (state.group !== defaults.group) params.set(key('group'), state.group ?? '');
  for (const [field, values] of Object.entries(state.filter)) {
    for (const v of values) params.append(key(`f.${field}`), v);
  }
}

const text = (v: unknown) => (v == null ? '' : String(v));

const empty = (v: unknown) => v == null || v === '';

function compare(a: unknown, b: unknown): number {
  if (typeof a === 'number' && typeof b === 'number') return a - b;
  return text(a).localeCompare(text(b), undefined, { numeric: true });
}

/** The distinct values of a select field, in first-seen order. */
export function optionsOf<T>(rows: T[], field: Field<T>): string[] {
  return [...new Set(rows.map((r) => text(field.value(r))))];
}

/** The rows the state keeps, sorted, with the group field as the outer sort key. */
export function apply<T>(rows: T[], fields: Field<T>[], state: CollectionState): T[] {
  const byId = new Map(fields.map((f) => [f.id, f]));
  const q = state.q.trim().toLowerCase();
  const filters = Object.entries(state.filter)
    .map(([id, values]) => [byId.get(id), new Set(values)] as const)
    .filter(([f, values]) => f && values.size > 0);
  const kept = rows.filter(
    (row) =>
      filters.every(([f, values]) => values.has(text(f!.value(row)))) &&
      (!q || fields.some((f) => text(f.value(row)).toLowerCase().includes(q))),
  );
  const sort = state.sort ? byId.get(state.sort) : undefined;
  const group = state.group ? byId.get(state.group) : undefined;
  if (!sort && !group) return kept;
  // Groups keep the order their values first appear in, as the board's columns do.
  const groupOrder = group ? new Map(optionsOf(kept, group).map((v, i) => [v, i])) : null;
  return kept
    .map((row, index) => ({ row, index }))
    .sort((a, b) => {
      if (group && groupOrder) {
        const g = groupOrder.get(text(group.value(a.row)))! - groupOrder.get(text(group.value(b.row)))!;
        if (g) return g;
      }
      if (sort) {
        const [av, bv] = [sort.value(a.row), sort.value(b.row)];
        // Empty values sort last in both directions.
        if (empty(av) !== empty(bv)) return empty(av) ? 1 : -1;
        const c = empty(av) ? 0 : compare(av, bv);
        if (c) return state.desc ? -c : c;
      }
      return a.index - b.index;
    })
    .map((x) => x.row);
}
