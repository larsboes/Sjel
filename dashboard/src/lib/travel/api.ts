/**
 * Travel route functions that are not in the shared client: the plan search and
 * companions clients, and the climate normals, retrospective and cost roll-up
 * clients.
 *
 * `dashboard/README.md` says `src/lib/api.ts` is the one place that knows both
 * upstream shapes and error shapes; the parallel-work rule says new route
 * functions go in a per-domain module. Both are kept: `request` and `jsonInit`
 * are imported from the shared client rather than copied, because
 * `request` also unwraps a 200 whose body carries `{"error": …}` — that is a
 * contract, not boilerplate, and a second copy of it is a second place for it
 * to drift.
 */
// Relative, not the `$lib` alias: this module is reached from the Home
// decision-kind modules under `src/lib/home`, which the registry test imports
// under plain `bun test`, where a Vite alias may not resolve. Same reasoning
// `tools/dashboard-nav-links.test.ts` records for `nav.ts`.
import { request, jsonInit } from '../api';
import type { PlaceRef, Retrospective, TransportMode } from '../api';

export interface DateWindow {
  from: string;
  /** Inclusive, like the trips route's own `date_to`. */
  to: string;
}

export interface PlanSearchRequest {
  origin: PlaceRef;
  /** `YYYY-MM`. Exactly one of `month` and `date_window`. */
  month?: string;
  date_window?: DateWindow;
  min_days?: number;
  budget_cents?: number;
  currency?: string;
  modes?: TransportMode[];
  interests?: string;
  max_candidates?: number;
}

/** One visible reason behind a candidate's score. */
export interface ScoreFactor {
  key: string;
  label: string;
  score: number;
  /** Re-normalised across the factors that could be measured; they sum to 1. */
  weight: number;
  rationale: string;
}

export interface FeasibleWindow {
  starts_on: string;
  /** Exclusive, as calendar serves it. */
  ends_before: string;
  days: string[];
  verdict: string;
  days_needing_travel_day: string[];
}

export interface CandidateEvent {
  id: string;
  title: string;
  starts_at: string | null;
  url: string;
  distance_km: number | null;
}

export interface RankedCandidate {
  destination: PlaceRef;
  place_id: string;
  window: FeasibleWindow;
  mode: TransportMode;
  estimated_cost_cents: number | null;
  currency: string;
  cost_basis: string;
  priced_at: string | null;
  events: CandidateEvent[];
  season: { month: number; score: number; best_month: number | null } | null;
  /** `null` when no factor could be measured — not a measured zero. */
  score: number | null;
  factors: ScoreFactor[];
  /**
   * Name-free by construction: places answers a count and an overlap, and trips
   * turns that into a sentence. It reaches this page and never the plan.
   */
  companion_hint: string | null;
  why: string[];
}

/** `"ok"`, `"absent"` or `"error: <reason>"` per upstream. */
export interface PlanSearchReach {
  calendar: string;
  places: string;
  transit: string;
  scouting: string;
  climate: string;
}

export interface PlanSearchResult {
  revision: string;
  window_source: 'calendar' | 'caller';
  windows: FeasibleWindow[];
  reach: PlanSearchReach;
  degraded: string[];
  considered: number;
  priced: number;
  unpriced: number;
  candidates: RankedCandidate[];
  observed_at: string;
}

export type PlanSearchJob =
  | { id: number; state: 'running'; since_ms: number }
  | { id: number; state: 'done'; result: PlanSearchResult }
  | { id: number; state: 'failed'; error: string };

export const planSearch = {
  start: (body: PlanSearchRequest) =>
    request<{ job: number }>('/trips/api/plan-search', jsonInit('POST', body)),
  status: (job: number, signal?: AbortSignal) =>
    request<PlanSearchJob>(
      `/trips/api/plan-search/${encodeURIComponent(String(job))}`,
      signal ? { signal } : undefined,
    ),
  /** Writes one `option_set` item onto a plan that already exists. */
  adopt: (job: number, planId: string) =>
    request<{ id: string; item_type: string; external_id: string }>(
      `/trips/api/plan-search/${encodeURIComponent(String(job))}/adopt`,
      jsonInit('POST', { plan_id: planId }),
    ),
};

export type { Retrospective };

// ─── Climate (places) ────────────────────────────────────────────────────────

/** One calendar month of a place's normal. Every measure is nullable, because a
 *  month the provider reported nothing for stores nothing — a 0 would read as a
 *  measurement. */
export interface ClimateMonth {
  month: number;
  t_max_mean: number | null;
  t_min_mean: number | null;
  rain_days_mean: number | null;
  precipitation_mm_mean: number | null;
  daylight_hours_mean: number | null;
  sunshine_hours_mean: number | null;
  days_observed: number;
  /** Computed on read by the server against `best_months_rule`. */
  best_month: boolean;
  /** Whether the plan's own window touches this month. */
  in_window: boolean;
}

export interface ClimateMatch {
  id: string;
  name: string;
  kind: string;
  latitude: number | null;
  longitude: number | null;
  /** Reported whenever the match was by distance, so a wrong 60 km match is
   *  visible rather than silent. */
  distance_km: number | null;
}

export interface ClimateResult {
  /** The place id or the `lat,lon` pair exactly as sent, so a caller never has
   *  to re-associate a result with its request. */
  key: string;
  resolved_by: "id" | "registry" | "nearest" | null;
  matched_place: ClimateMatch | null;
  reason: string | null;
  source: string | null;
  period: { start: string; end: string; years: number } | null;
  fetched_at: string | null;
  months: ClimateMonth[];
}

export interface ClimateBatch {
  best_months_rule: string;
  attribution: string;
  results: ClimateResult[];
}

/** A coordinate pair to ask about. */
export interface ClimateKey {
  latitude: number;
  longitude: number;
}

/**
 * Normals for up to eight coordinates in one request.
 *
 * The pairs go into ONE `at=` parameter, semicolon between pairs and comma
 * inside a pair — not a repeated `at=` key. The server reads it with axum's
 * `Query`, which deserializes through serde_urlencoded and cannot fill a
 * sequence from repeated keys, so `?at=..&at=..` would be a 400.
 */
export function climateFor(
  keys: ClimateKey[],
  window?: { from?: string; to?: string },
): Promise<ClimateBatch> {
  const at = keys
    .slice(0, 8)
    .map((key) => `${key.latitude},${key.longitude}`)
    .join(";");
  const search = new URLSearchParams({ at });
  if (window?.from) search.set("from", window.from);
  if (window?.to) search.set("to", window.to);
  return request<ClimateBatch>(`/places/api/climate?${search.toString()}`);
}

// ─── Retrospective (trips) ───────────────────────────────────────────────────

export interface PendingRetrospective {
  plan_id: string;
  title: string;
  destinations: string[];
  date_start: string;
  date_end: string;
  days_since_close: number;
  data_class?: "c0" | "c1" | "c2" | "c3";
}

export interface PendingRetrospectives {
  pending: PendingRetrospective[];
  window_days: number;
}

export interface DestinationFactor {
  key: string;
  n: number;
  mean_again: number;
  factor: number;
  median_overrun_bp: number | null;
  basis: string[];
}

export interface RetrospectiveSummary {
  formula: string;
  bounds: [number, number];
  contract: string;
  by_destination: DestinationFactor[];
  /** A sentence, not data. The companion half is specified and not served; the
   *  string names where the three preconditions are written down. */
  by_companion: string;
}

/** The three fields, and only the three. The server sets `deny_unknown_fields`,
 *  so a fourth is refused by axum's JSON extractor — a 422 with a plain-text
 *  body naming the unknown field, not a silent drop. `request` surfaces
 *  either shape. */
export interface RetrospectiveBody {
  cost_cents: number | null;
  again: "yes" | "no" | "maybe" | "not_taken";
  change_note: string;
}

export const saveRetrospective = (planId: string, body: RetrospectiveBody) =>
  request<Retrospective>(
    `/trips/api/plans/${encodeURIComponent(planId)}/retrospective`,
    jsonInit("POST", body),
  );

export const pendingRetrospectives = () =>
  request<PendingRetrospectives>("/trips/api/retrospectives/pending");

export const retrospectiveSummary = () =>
  request<RetrospectiveSummary>("/trips/api/retrospectives/summary");

// ─── Cost roll-up (trips) ────────────────────────────────────────────────────

/**
 * Finance's four per-trip figures, carried and never flattened into one.
 *
 * On a trip with friends `personal` and `gross_cash_outflow` differ by exactly
 * what comes back, which is the shared-cost surface the PRD kept as runway. One
 * `spent_cents` would answer "was it worth it" wrongly.
 *
 * `ok: false` means finance did not answer. Every figure is then null with a
 * named reason — never 0, which would read as "nothing was spent".
 */
export interface CostActuals {
  ok: boolean;
  reason: string | null;
  /** The unit FINANCE stated for these figures, which is not automatically the
   *  plan's. When the two disagree the figures are null with a reason rather
   *  than relabelled. */
  currency: string | null;
  personal_cents: number | null;
  gross_cash_outflow_cents: number | null;
  reimbursed_cents: number | null;
  outstanding_cents: number | null;
  posting_count: number | null;
}

export interface CostByCurrency {
  currency: string;
  booked_cents: number;
  item_count: number;
}

export interface CostByStage {
  stage_id: string;
  sequence: number;
  origin: string;
  destination: string;
  /** Null when the items on this stage do not agree on a currency — the same
   *  refusal the headline makes, at the grain a stage row is read at. */
  booked_cents: number | null;
  currency: string | null;
  reason: string | null;
  item_count: number;
}

export interface SelectedOptions {
  total: number | null;
  currency: null;
  priced_items: number;
  note: string;
}

export interface CostSource {
  source: string;
  ok: boolean;
  reason: string | null;
}

export interface PlanCost {
  plan_id: string;
  currency: string | null;
  planned_cents: number | null;
  /** Null rather than a sum when the plan's bookings are in more than one
   *  currency: `by_currency` then carries one row each and `reason` says so. */
  booked_cents: number | null;
  booked_reason: string | null;
  actuals: CostActuals;
  selected_options: SelectedOptions;
  by_currency: CostByCurrency[];
  by_stage: CostByStage[];
  unattributed: {
    booked_cents: number | null;
    currency: string | null;
    reason: string | null;
    item_count: number;
  };
  sources: CostSource[];
}

export const planCost = (planId: string) =>
  request<PlanCost>(`/trips/api/plans/${encodeURIComponent(planId)}/cost`);
