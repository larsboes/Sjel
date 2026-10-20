import type { DataClass } from './comms';
import { jsonInit, request } from './core';

/** How binding an entry is, orthogonal to `kind`. The calendar capability
 * caps an entry's feasibility impact by this: `possible` can never block a
 * day, `committed` lets the kind decide. */
export type CalendarCommitment = "possible" | "planned" | "committed";

export interface CalendarEntry {
  id: string;
  kind: string;
  commitment: CalendarCommitment;
  title: string;
  starts_at: string;
  ends_at: string;
  all_day: boolean;
  location: string | null;
  notes: string | null;
  source: string;
  external_id: string | null;
  rhythm_id: string | null;
  payload: unknown;
  created_at: string;
  updated_at: string;
  /** What this entry is worth protecting, stated on every entry list and single-entry
   *  read/write (`capabilities/calendar/src/content.rs`, `classification()`: "where the
   *  operator is and when is personal, whatever the event itself is" → `c1`). */
  data_class?: DataClass;
}

export interface CalendarEntryTombstone {
  id: string;
  deleted_at: string;
}

export interface CalendarEntryChanges {
  cursor: string;
  upserted: CalendarEntry[];
  deleted: CalendarEntryTombstone[];
}

export interface CalendarNewEntry {
  kind: string;
  /** Omitted means `possible` server-side. The form always sends it. */
  commitment?: CalendarCommitment;
  title: string;
  starts_at: string;
  ends_at: string;
  all_day?: boolean;
  location?: string | null;
  notes?: string | null;
  source?: string;
  external_id?: string | null;
  rhythm_id?: string | null;
  payload?: unknown;
}

export interface CalendarUpdateEntry {
  kind?: string;
  commitment?: CalendarCommitment;
  title?: string;
  starts_at?: string;
  ends_at?: string;
  all_day?: boolean;
  location?: string | null;
  notes?: string | null;
}

export interface CalendarContext {
  id: string;
  kind: string;
  title: string;
  details: string;
  valid_from: string;
  valid_until: string;
  source: string;
  created_at: string;
  updated_at: string;
}

export interface CalendarNewContext {
  kind: string;
  title: string;
  details?: string;
  valid_from: string;
  valid_until: string;
  source?: string;
}

export interface CalendarUpdateContext {
  kind?: string;
  title?: string;
  details?: string;
  valid_from?: string;
  valid_until?: string;
}

/** A run of days travel is possible at all — the calendar capability's own
 * verdict, not something the UI recomputes. */
export interface CalendarFeasibleWindow {
  starts_on: string;
  /** Exclusive, like every end in this capability. */
  ends_before: string;
  days: string[];
  verdict: "free" | "needs-travel-day" | "conflicts";
  days_needing_travel_day: string[];
}

export interface CalendarWindows {
  from: string;
  to: string;
  min_days: number;
  windows: CalendarFeasibleWindow[];
}

/** Calendar's explanation of how one external opportunity fits a real time
 * window. The dashboard renders this evidence; it never recomputes conflicts. */
export interface CalendarCandidateVerdict {
  id: string;
  verdict: 'free' | 'needs-travel-day' | 'conflicts';
  starts_at: string;
  ends_at: string;
  already_in_calendar: boolean;
  evidence: Array<{
    entry_id: string;
    kind: string;
    commitment: CalendarCommitment;
    title: string;
    starts_at: string;
    ends_at: string;
    all_day: boolean;
    impact: 'free' | 'needs-travel-day' | 'conflicts';
  }>;
}

export interface CalendarRhythm {
  id: string;
  kind: string;
  title: string;
  location: string | null;
  byweekday: string[];
  start_time: string | null;
  end_time: string | null;
  valid_from: string;
  valid_until: string;
  active: boolean;
  created_at: string;
  updated_at: string;
}

export interface CalendarNewRhythm {
  kind: string;
  title: string;
  location?: string | null;
  byweekday: string[];
  start_time?: string | null;
  end_time?: string | null;
  valid_from: string;
  valid_until: string;
}

export interface CalendarUpdateRhythm {
  kind?: string;
  title?: string;
  location?: string;
  byweekday?: string[];
  start_time?: string;
  end_time?: string;
  valid_from?: string;
  valid_until?: string;
  active?: boolean;
}

export type GoogleImportReviewStatus =
  | 'importable'
  | 'likely-duplicate'
  // The pre-rename value stays in the union for one release: a tab opened before the
  // 2026-10-20 rename is still asking the server it was loaded from.
  | 'already-in-sjel'
  | 'already-in-axon'
  | 'cancelled'
  | 'invalid';

/** One Google event in a read-only, date-bounded import review. The revision
 * is returned to the server with the selection, preventing a changed event
 * from being imported behind the operator's back. */
export interface CalendarGoogleImportCandidate {
  google_event_id: string;
  google_updated: string | null;
  title: string;
  starts_at: string | null;
  ends_at: string | null;
  all_day: boolean | null;
  location: string | null;
  html_link: string | null;
  recurring_event_id: string | null;
  status: GoogleImportReviewStatus;
  reason: string | null;
  duplicate_group: string | null;
}

export interface CalendarGoogleImportPreview {
  calendar_id: string;
  home_timezone: string;
  from: string;
  to: string;
  fetched: number;
  at_event_limit: boolean;
  candidates: CalendarGoogleImportCandidate[];
}

export interface CalendarGoogleImportReport {
  fetched: number;
  created: number;
  refreshed: number;
  unchanged: number;
  skipped: Array<{ google_event_id: string; reason: string }>;
}

/** A deliberate per-entry permission to publish an Sjel entry to Google.
 * Its presence is the opt-in; it does not itself contact Google. */
export interface CalendarGoogleExportOptIn {
  entry_id: string;
  google_calendar_id: string;
  google_event_id: string | null;
  pushed_at: string | null;
  created_at: string;
}

export interface CalendarGoogleExportReport {
  calendar_id: string;
  home_timezone: string;
  opted_in: number;
  inserted: number;
  patched: number;
  pushed: Array<{
    entry_id: string;
    title: string;
    operation: 'inserted' | 'patched';
    google_event_id: string | null;
  }>;
  skipped: Array<{ google_event_id: string; reason: string }>;
  dry_run: boolean;
}

/** A proposed journey derived from dated Calendar entries at one place. This
 * remains recomputed evidence until the operator explicitly materialises it. */
export interface CalendarTripDraft {
  place: string;
  starts_on: string;
  /** Exclusive, following Calendar's own time model. */
  ends_before: string;
  entry_ids: string[];
  titles: string[];
  commitment: CalendarCommitment;
}

export interface CalendarTripDrafts {
  from: string;
  to: string;
  max_gap_days: number;
  home: string | null;
  drafts: CalendarTripDraft[];
  unclustered: Array<{ entry_id: string; title: string; reason: string }>;
}

export interface CalendarTripMaterialization {
  plan_id: string;
  created: boolean;
  reason?: string;
}

export const calendar = {
  proposals: (from: string, to: string) =>
    request<CalendarEntry[]>(
      `/calendar/api/proposals?from=${encodeURIComponent(from)}&to=${encodeURIComponent(to)}`,
    ),
  tripDrafts: {
    list: (from: string, to: string, maxGapDays = 5) =>
      request<CalendarTripDrafts>(
        `/calendar/api/trip-drafts?${new URLSearchParams({ from, to, max_gap_days: String(maxGapDays) })}`,
      ),
    materialize: (entryIds: string[], title?: string) =>
      request<CalendarTripMaterialization>(
        '/calendar/api/trip-drafts/materialize',
        jsonInit('POST', { entry_ids: entryIds, title: title?.trim() || null }),
      ),
  },
  google: {
    exports: () => request<CalendarGoogleExportOptIn[]>('/calendar/api/google/exports'),
    previewExport: () =>
      request<CalendarGoogleExportReport>('/calendar/api/google/export', jsonInit('POST', { dry_run: true })),
    export: () =>
      request<CalendarGoogleExportReport>('/calendar/api/google/export', jsonInit('POST', { dry_run: false })),
    optInExport: (entryId: string) =>
      request<CalendarGoogleExportOptIn>(
        `/calendar/api/entries/${encodeURIComponent(entryId)}/google-export`,
        jsonInit('PUT', {}),
      ),
    optOutExport: (entryId: string) =>
      request<void>(
        `/calendar/api/entries/${encodeURIComponent(entryId)}/google-export`,
        { method: 'DELETE' },
      ),
    drafts: (from: string, to: string) =>
      request<CalendarEntry[]>(
        `/calendar/api/google/drafts?from=${encodeURIComponent(from)}&to=${encodeURIComponent(to)}`,
      ),
    previewImport: (from: string, to: string) =>
      request<CalendarGoogleImportPreview>(
        '/calendar/api/google/import-preview',
        jsonInit('POST', { from, to }),
      ),
    importSelected: (
      from: string,
      to: string,
      selected: Array<{ google_event_id: string; google_updated: string | null }>,
    ) =>
      request<CalendarGoogleImportReport>(
        '/calendar/api/google/import-selected',
        jsonInit('POST', { from, to, selected }),
      ),
  },
  windows: (from: string, to: string) =>
    request<CalendarWindows>(
      `/calendar/api/windows?from=${encodeURIComponent(from)}&to=${encodeURIComponent(to)}`,
    ),
  verdicts: (candidates: Array<{ id: string; starts_at: string; ends_at?: string | null }>) =>
    request<{ verdicts: CalendarCandidateVerdict[] }>(
      '/calendar/api/verdicts',
      jsonInit('POST', { candidates }),
    ),
  entries: {
    list: (from: string, to: string, kind?: string) => {
      const params = new URLSearchParams({ from, to });
      if (kind) params.set('kind', kind);
      return request<CalendarEntry[]>(`/calendar/api/entries?${params}`);
    },
    changes: (since?: string) => {
      const params = new URLSearchParams();
      if (since) params.set('since', since);
      const q = params.toString();
      return request<CalendarEntryChanges>(`/calendar/api/entries/changes${q ? `?${q}` : ''}`);
    },
    get: (id: string) =>
      request<CalendarEntry>(`/calendar/api/entries/${encodeURIComponent(id)}`),
    create: (entry: CalendarNewEntry) =>
      request<CalendarEntry>('/calendar/api/entries', jsonInit('POST', entry)),
    upsertExternal: (entry: CalendarNewEntry) =>
      request<CalendarEntry>('/calendar/api/entries/external', jsonInit('PUT', entry)),
    update: (id: string, entry: CalendarUpdateEntry) =>
      request<CalendarEntry>(
        `/calendar/api/entries/${encodeURIComponent(id)}`,
        jsonInit('PATCH', entry),
      ),
    delete: (id: string) =>
      request<void>(
        `/calendar/api/entries/${encodeURIComponent(id)}`,
        { method: 'DELETE' },
      ),
  },
  contexts: {
    list: (from: string, to: string) =>
      request<CalendarContext[]>(
        `/calendar/api/contexts?from=${encodeURIComponent(from)}&to=${encodeURIComponent(to)}`,
      ),
    create: (context: CalendarNewContext) =>
      request<CalendarContext>('/calendar/api/contexts', jsonInit('POST', context)),
    update: (id: string, context: CalendarUpdateContext) =>
      request<CalendarContext>(
        `/calendar/api/contexts/${encodeURIComponent(id)}`,
        jsonInit('PATCH', context),
      ),
    delete: (id: string) =>
      request<void>(
        `/calendar/api/contexts/${encodeURIComponent(id)}`,
        { method: 'DELETE' },
      ),
  },
  rhythms: {
    list: () => request<CalendarRhythm[]>('/calendar/api/rhythms'),
    get: (id: string) =>
      request<CalendarRhythm>(`/calendar/api/rhythms/${encodeURIComponent(id)}`),
    create: (rhythm: CalendarNewRhythm) =>
      request<{ rhythm: CalendarRhythm; instances_created: number }>(
        '/calendar/api/rhythms',
        jsonInit('POST', rhythm),
      ),
    update: (id: string, rhythm: CalendarUpdateRhythm) =>
      request<{ rhythm: CalendarRhythm; future_instances_affected: number }>(
        `/calendar/api/rhythms/${encodeURIComponent(id)}`,
        jsonInit('PATCH', rhythm),
      ),
    delete: (id: string, deleteInstances?: boolean) =>
      request<void>(
        `/calendar/api/rhythms/${encodeURIComponent(id)}${deleteInstances ? '?delete_instances=true' : ''}`,
        { method: 'DELETE' },
      ),
    materialize: (id: string) =>
      request<{ instances_created: number }>(
        `/calendar/api/rhythms/${encodeURIComponent(id)}/materialize`,
        { method: 'POST' },
      ),
  },
};
