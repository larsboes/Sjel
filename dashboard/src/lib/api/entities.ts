import { jsonInit, request } from './core';

export type EntityKind = 'person' | 'organisation' | 'place' | 'self';

export interface EntityField {
  kind: EntityKind;
  key: string;
  label: string;
  field_type: 'text' | 'bool' | 'date' | 'number' | 'enum' | 'emails' | 'phones' | 'url' | 'tags';
  options: string[];
  data_class: 'C0' | 'C1' | 'C2' | 'C3';
  builtin: boolean;
}

export interface EntityFact {
  id: string;
  entity_id: string;
  predicate: 'home_base' | 'away';
  place: string;
  latitude: number | null;
  longitude: number | null;
  valid_from: string | null;
  valid_to: string | null;
  note: string | null;
  source: string;
  created_at: string;
}

export interface Entity {
  id: string;
  kind: EntityKind;
  name: string;
  note_ref: string | null;
  revision: number;
  created_at: string;
  updated_at: string;
  values: Record<string, { value: unknown; source: string; updated_at: string }>;
  facts: EntityFact[];
}

export interface LocatedPerson {
  entity_id: string;
  name: string;
  predicate: 'home_base' | 'away';
  place: string;
  latitude: number | null;
  longitude: number | null;
  sleeping_option: 'none' | 'ask' | 'yes' | null;
  sleeping_note: string | null;
}

/** One linked system's current view of an entity (entities server.rs, entity_sources). */
export interface EntitySource {
  system: 'google' | 'obsidian';
  external_id: string;
  name?: string;
  values?: Record<string, unknown>;
  home?: string | null;
  error?: string;
}

/** What entities shows about one side of a duplicate pair (duplicates.rs, profile). */
export interface DuplicateProfile {
  name: string;
  lives_in: string | null;
  company: unknown;
  role: unknown;
  relation: unknown;
  emails: string[] | null;
  phones: string[] | null;
  birthday: string | null;
  sources: string[];
  has_note: boolean;
}

export interface DuplicateCandidate {
  a: { id: string; profile: DuplicateProfile };
  b: { id: string; profile: DuplicateProfile };
  strength: 'strong' | 'name' | 'partial';
  reasons: string[];
  evidence: { same: string[]; different: string[] };
  verdict: { same: boolean | null; why: string } | null;
}

/** capabilities/entities. C2: names, places and contact details. */
export const entities = {
  fields: (kind?: EntityKind) =>
    request<{ fields: EntityField[] }>(`/entities/api/fields${kind ? `?kind=${kind}` : ''}`).then(
      (r) => r.fields,
    ),
  declareField: (field: Omit<EntityField, 'builtin'>) =>
    request<EntityField>('/entities/api/fields', jsonInit('POST', field)),
  list: (kind?: EntityKind, q?: string) => {
    const query = new URLSearchParams();
    if (kind) query.set('kind', kind);
    if (q) query.set('q', q);
    return request<{ entities: Entity[] }>(
      `/entities/api/entities${query.size ? `?${query}` : ''}`,
    ).then((r) => r.entities);
  },
  get: (id: string) => request<Entity>(`/entities/api/entities/${encodeURIComponent(id)}`),
  create: (body: { kind: EntityKind; name: string; note_ref?: string; values?: Record<string, unknown> }) =>
    request<Entity>('/entities/api/entities', jsonInit('POST', body)),
  patch: (
    id: string,
    body: {
      name?: string;
      values?: Record<string, unknown>;
      expected_revision?: number;
      /** Who the written values belong to; `operator` unless taking a source's value back. */
      source?: 'operator' | 'google' | 'obsidian';
    },
  ) => request<Entity>(`/entities/api/entities/${encodeURIComponent(id)}`, jsonInit('PATCH', body)),
  remove: (id: string) =>
    request<void>(`/entities/api/entities/${encodeURIComponent(id)}`, { method: 'DELETE' }),
  addFact: (
    id: string,
    body: { predicate: 'home_base' | 'away'; place: string; valid_from?: string; valid_to?: string; note?: string },
  ) =>
    request<{ fact: EntityFact; geocode: { status: string; reason?: string } }>(
      `/entities/api/entities/${encodeURIComponent(id)}/facts`,
      jsonInit('POST', body),
    ),
  removeFact: (id: string, factId: string) =>
    request<void>(
      `/entities/api/entities/${encodeURIComponent(id)}/facts/${encodeURIComponent(factId)}`,
      { method: 'DELETE' },
    ),
  /** Probable duplicates, strongest first; judge=true asks the on-device model about
   *  first-name-only pairs that share a field. */
  duplicates: (limit = 10, judge = true) =>
    request<{ total: number; candidates: DuplicateCandidate[] }>(
      `/entities/api/duplicates?limit=${limit}&judge=${judge}`,
    ),
  merge: (keep: string, other: string, name?: string, pick: string[] = []) =>
    request<Entity>(
      `/entities/api/entities/${encodeURIComponent(keep)}/merge`,
      jsonInit('POST', { other, name, pick }),
    ),
  /** What the linked Google contact and Obsidian note say now, for comparing. */
  sources: (id: string) =>
    request<{ sources: EntitySource[] }>(`/entities/api/entities/${encodeURIComponent(id)}/sources`).then(
      (r) => r.sources,
    ),
  markDistinct: (a: string, b: string) =>
    request<void>('/entities/api/duplicates/distinct', jsonInit('POST', { a, b })),
  located: (day?: string) =>
    request<{ day: string; located: LocatedPerson[] }>(
      `/entities/api/located${day ? `?day=${day}` : ''}`,
    ),
};
