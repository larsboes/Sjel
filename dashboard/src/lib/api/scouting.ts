import type { DataClass } from './comms';
import { jsonInit, request } from './core';
import type { DiscoverResponse } from './travel';

export type OpportunityStatus = 'new' | 'saved' | 'dismissed';

export type EventRouteKind = 'local' | 'travel_candidate' | 'online' | 'unresolved';

export interface EventRoute {
  route: EventRouteKind;
  basis:
    | 'source_metadata'
    | 'location_text'
    | 'coordinates'
    | 'country'
    | 'timezone'
    | 'operator_override'
    | 'missing_policy'
    | 'missing_evidence';
  reason: string;
  distance_km: number | null;
}

export interface ScoutingOpportunity {
  id: string;
  opportunity_type: string;
  source: string;
  title: string;
  city: string;
  starts_at: string;
  ends_at: string;
  location: string;
  score: number;
  matched_focus: string;
  rationale: string;
  url: string;
  vault_link: string | null;
  status: OpportunityStatus;
  country_code: string | null;
  latitude: number | null;
  longitude: number | null;
  event_route: EventRoute | null;
  /** What this row is worth protecting, from the source that fetched it.
   *
   *  Declared per source in `scouting.json` and resolved by the server, never by a row:
   *  an opportunity whose source declares nothing is `c1`, the fail-closed default, which
   *  is what every hardcoded adapter answers. `c0` means somebody said so. */
  data_class: DataClass;
}

export interface ScoutingSource {
  id: string;
  adapter: string;
  enabled: boolean;
  configured: boolean;
  root_path: string | null;
  url: string | null;
  opportunity_type: string;
  /** The declaration behind every row this source fetched. `c1` here is either what the
   *  source declared or what its silence means — `GET /opportunities` cannot tell those
   *  apart and neither can this. */
  data_class: DataClass;
}

export const scouting = {
  discover: (opts: { adapter: string; location?: string; query?: string }) => {
    const params = new URLSearchParams({ adapter: opts.adapter });
    if (opts.location) params.set('location', opts.location);
    if (opts.query) params.set('query', opts.query);
    return request<DiscoverResponse>(`/discover?${params.toString()}`);
  },
  // /scouting, not /scout-server: every capability is proxied at /<capability name>,
  // derived from the registry rather than hand-listed (see vite.config.ts).
  health: () => request<unknown>('/scouting/health'),
  sources: () =>
    request<{ sources: ScoutingSource[]; count: number }>('/scouting/sources'),
  opportunities: (includeDismissed = true) =>
    request<{ opportunities: ScoutingOpportunity[]; count: number; store_total: number }>(
      `/scouting/opportunities?include_dismissed=${includeDismissed}`,
    ),
  setStatus: (id: string, status: OpportunityStatus) =>
    request<{ id: string; status: OpportunityStatus }>(
      `/scouting/opportunities/${encodeURIComponent(id)}/status`,
      jsonInit('POST', { status }),
    ),
};
