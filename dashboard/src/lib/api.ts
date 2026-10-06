import { bridgeErrorText, inTauri, macRequest, NOT_CONFIGURED, type MacResponse } from './mac-bridge';

export class ApiError extends Error {
  status: number;
  constructor(status: number, message: string) {
    super(message);
    this.name = 'ApiError';
    this.status = status;
  }
}

/**
 * The message inside an error response body. Every capability server answers
 * `{"error": "..."}`, so without this the UI shows a reader the raw JSON —
 * which is what the feed's paste box did on a 404 until this was fixed.
 */
function errorMessage(body: string): string {
  try {
    const parsed = JSON.parse(body);
    if (parsed && typeof parsed === 'object' && typeof parsed.error === 'string') return parsed.error;
  } catch {
    // Not JSON — the raw body is the best message available, unless it is a web page.
  }
  // An HTML page is a host's or proxy's page, not the capability's message: printing it put a
  // whole GitHub 404 page on the demo's People page (2026-09-27).
  if (isHtml(body)) return '';
  return body;
}

function isHtml(body: string): boolean {
  return /<\s*!doctype\s+html|<\s*html[\s>]/i.test(body);
}

/**
 * The capability a proxied path belongs to. Every capability call goes through
 * `/<name>/…`, which is the dev proxy's uniform rule, so the first segment is the
 * name without anything having to declare it twice.
 */
function capabilityFrom(path: string): string | null {
  const segment = path.replace(/^\/+/, '').split(/[/?#]/)[0];
  return segment && !segment.startsWith('api') ? segment : null;
}

/**
 * What to show a reader when a request fails.
 *
 * The case worth naming: when a capability's process is not running, the dev proxy
 * cannot reach it and answers with a bare 5xx and an empty, non-JSON body. Every
 * page then rendered `Request failed (500)`, which says nothing at all — it reads
 * as a bug in the page rather than as a service that was never started, and it
 * sends the reader looking in the wrong place. There is nothing wrong in that
 * situation except that nobody started the thing, so it says so, with the command.
 */
export function describeFailure(status: number, body: string, path: string): string {
  const capability = capabilityFrom(path);
  // A 404 that answers with a web page came from the host or a proxy, not from the capability:
  // the capability is not served here (the demo site has no entities, for one). A 404 with no
  // body or a JSON body stays a wrong route, below.
  if (capability && status === 404 && isHtml(body)) {
    return `${capability} is not available here (404)`;
  }
  const named = errorMessage(body).trim();
  if (named) return capability ? `${capability}: ${named}` : named;
  if (capability && status >= 500) {
    return `${capability} is not running — start it with: tools/service-runner.sh start ${capability}`;
  }
  return capability ? `${capability}: request failed (${status})` : `Request failed (${status})`;
}

/** Status and body text of one answer, whichever way it travelled. */
export interface Answer {
  ok: boolean;
  status: number;
  text: string;
}

/**
 * In the Tauri app a relative path has no server behind it, so a node path goes
 * through the native bridge (`./mac-bridge.ts`). An absolute URL, such as the
 * Wikipedia lookups below, is not a Mac path and stays a plain fetch.
 */
export async function send(path: string, init?: RequestInit): Promise<Answer> {
  if (inTauri() && path.startsWith('/')) {
    let answer: MacResponse;
    try {
      answer = await macRequest(path, init);
    } catch (error) {
      const message = bridgeErrorText(error);
      if (message.startsWith(NOT_CONFIGURED)) {
        throw new ApiError(0, 'The Sjel node address is not set. Set it in Sjel connection (footer).');
      }
      throw new ApiError(0, message);
    }
    return { ok: answer.status >= 200 && answer.status < 300, status: answer.status, text: answer.body };
  }
  const res = await fetch(path, init);
  if (!res.ok) return { ok: false, status: res.status, text: await res.text().catch(() => '') };
  return { ok: true, status: res.status, text: await res.text() };
}

export async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await send(path, init);
  if (!res.ok) {
    throw new ApiError(res.status, describeFailure(res.status, res.text, path));
  }
  const text = res.text;
  if (!text) return undefined as T;

  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch {
    const capability = capabilityFrom(path);
    const html = /<\s*!doctype\s+html|<\s*html[\s>]/i.test(text);
    const message = html
      ? `${capability ?? "API"} did not return JSON; its service is unavailable here`
      : `${capability ?? "API"} returned invalid JSON`;
    throw new ApiError(res.status, message);
  }
  if (parsed && typeof parsed === 'object' && 'error' in parsed && parsed.error) {
    throw new ApiError(res.status, String(parsed.error));
  }
  return parsed as T;
}

export const jsonInit = (method: string, body: unknown): RequestInit => ({
  method,
  headers: { 'Content-Type': 'application/json' },
  body: JSON.stringify(body),
});

// ─── Types ───────────────────────────────────────────────────────────────────

export type PlaceKind = 'address' | 'airport' | 'city' | 'station' | 'venue';
export type TransportMode = 'bike' | 'bus' | 'car' | 'ferry' | 'flight' | 'train' | 'walk';

export interface PlaceRef {
  id: string;
  name: string;
  kind?: PlaceKind;
  address?: string | null;
  latitude?: number | null;
  longitude?: number | null;
}

export interface Station extends PlaceRef {
  kind?: 'station';
}

export interface TripStage {
  id: string;
  sequence: number;
  origin: PlaceRef;
  destination: PlaceRef;
  date: string | null;
  transport_modes: TransportMode[];
  travelers: string[];
  status: 'planning' | 'option_selected' | 'booked' | 'completed' | 'open';
  selected_option_id: string | null;
  branch_note?: string | null;
}

export interface IntentDraft {
  draft: {
    title: string;
    origin: PlaceRef | null;
    destinations: PlaceRef[];
    date_start: string | null;
    date_end: string | null;
    interests: string;
    transport_modes: TransportMode[];
    travelers: string[];
  };
  unresolved: string[];
  assumptions: string[];
  source_text: string;
}

export interface PlanSource {
  kind: string;
  reference: string;
}

export interface TripPlan {
  id: string;
  title: string;
  origin: PlaceRef;
  destinations: PlaceRef[];
  date_start: string;
  date_end: string;
  interests: string;
  status: 'draft' | 'saved' | 'archived';
  travelers: string[];
  transport_modes: TransportMode[];
  budget_cents: number | null;
  currency: string | null;
  stages: TripStage[];
  cover_image_url: string | null;
  source: PlanSource | null;
  created_at: string;
  updated_at: string;
}

export type PlanItemType =
  | 'journey'
  | 'transport'
  | 'event'
  | 'activity'
  | 'place'
  | 'stay'
  | 'image'
  | 'note'
  | 'option_set'
  | 'booking'
  | 'outcome';

export interface PlanItem {
  id: string;
  plan_id: string;
  item_type: PlanItemType;
  day: string | null;
  external_id: string;
  title: string;
  payload: unknown;
  created_at: string;
}

/**
 * One plan's close-out record: exactly the three fields PRD 8.2 rules, plus the
 * plan key and the stamp. `currency` is echoed from the plan and is not stored
 * on the row — `cost_cents` is denominated in the plan's own currency, so the
 * money is named exactly once.
 *
 * Declared here rather than imported, because this module deliberately has no
 * imports at all: the Home decision-kind modules under `dashboard/src/lib/home`
 * must stay importable under plain `bun test`.
 */
export interface Retrospective {
  plan_id: string;
  cost_cents: number | null;
  currency: string | null;
  again: 'yes' | 'no' | 'maybe' | 'not_taken';
  change_note: string;
  filled_at: string;
}

export interface PlanDetails extends TripPlan {
  items: PlanItem[];
  retrospective: Retrospective | null;
}

export interface ObsidianTripCandidate {
  reference: string;
  title: string;
  summary: string;
  date_start: string | null;
  date_end: string | null;
  destination: PlaceRef | null;
  status: string;
  travelers: string[];
  transport_modes: TransportMode[];
  cover: string | null;
  issues: string[];
  imported_plan_id: string | null;
}

export interface ObsidianImportAllResult {
  imported: TripPlan[];
  existing: TripPlan[];
  skipped: Array<{
    reference: string;
    title: string;
    issues: string[];
  }>;
}

export type Provenance = 'default' | 'stated' | 'derived' | 'vault';

export interface HardConstraints {
  earliest_departure: string | null;
  latest_arrival: string | null;
  max_changes: number | null;
  min_transfer_buffer_min: number | null;
  modes: string[];
  avoid_overnight_travel: boolean;
  home_stations: string[];
  home_airports: string[];
  cards: string[];
}

export interface SoftWeights {
  budget_fit: number;
  feasibility: number;
  season: number;
  events: number;
  retrospective: number;
}

export interface JourneyWeights {
  price: number;
  duration: number;
  changes: number;
  reliability: number;
}

export type Pace = 'slow' | 'balanced' | 'packed';

export interface TravelProfile {
  id: string;
  hard: HardConstraints;
  soft: SoftWeights;
  journey: JourneyWeights;
  interests: string;
  pace: Pace;
  anchors: string[];
  basis: Record<string, Provenance>;
}

export interface TravelProfileResponse {
  profile: TravelProfile;
  stored: boolean;
  revision: number;
}

export interface DerivedTravelStats {
  notes: string[];
  counts: {
    total_plans: number;
    eligible_plans: number;
    not_taken_plans: number;
  };
  basis: string[];
  lead_time_days: {
    min: number;
    p25: number;
    median: number;
    p75: number;
    max: number;
  } | null;
  trip_length_days: {
    min: number;
    p25: number;
    median: number;
    p75: number;
    max: number;
  } | null;
  company_shape: {
    solo: number;
    pair: number;
    group: number;
    unrecorded: number;
  };
  modes: Record<string, number>;
  popular_destinations: Array<{ name: string; count: number }>;
}

export interface BaseRequest {
  from: PlaceRef;
  anchor: PlaceRef;
  from_date: string;
  anchor_date: string;
  max_candidates?: number;
}

export interface LegQuote {
  cents: number;
  currency: string;
}

export interface CompanionsNearBase {
  count: number;
  person_ids: string[];
}

export interface BaseCandidate {
  place_id: string;
  name: string;
  latitude: number | null;
  longitude: number | null;
  reach: LegQuote | null;
  onward: LegQuote | null;
  travel_cents: number | null;
  known_companions: number | null;
  overlap_days: number | null;
  stay_cents: number | null;
  stay_reason: string;
  why: string[];
}

export interface BaseResult {
  from: string;
  anchor: string;
  from_date: string;
  anchor_date: string;
  candidates: BaseCandidate[];
  considered: number;
  priced: number;
  degraded: string[];
  observed_at: string;
}

export interface ScoredResult {
  id: string;
  rank: number;
  score: number;
  source: string;
  title: string;
  date: string | null;
  location: string | null;
  city: string | null;
  matched_focus: string | null;
  rationale: string;
  url: string;
  opportunity_type: string;
  status: OpportunityStatus;
  vault_link: string | null;
  event_route: EventRoute | null;
}

export interface DiscoverResponse {
  adapter: string;
  opportunity_type: string;
  total_scored: number;
  new_count: number;
  vault_links: number;
  store_total: number;
  results: ScoredResult[];
}

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

export interface AxonStatusHealth {
  ok: boolean;
  version: string;
  uptime_seconds: number;
  capabilities: Record<string, { up: boolean; url: string }>;
}

/**
 * What one search asked for instead of the profile's usual weights.
 *
 * The server refuses more than one of these, so exactly one is sent: a phrase the
 * traveller typed beats a preset they pressed, because it is the more specific
 * thing to say. The names and the numbers behind them live in
 * `capabilities/transit/src/ranking.rs` -- the UI owns the labels, the server owns
 * the arithmetic, and a preset defined twice is how the two start disagreeing.
 */
export interface JourneyOverride {
  priority?: string;
  phrase?: string;
}

function overrideQuery(override?: JourneyOverride): string {
  if (!override) return '';
  if (override.phrase) return `&phrase=${encodeURIComponent(override.phrase)}`;
  if (override.priority) return `&priority=${encodeURIComponent(override.priority)}`;
  return '';
}

/**
 * The presets the server knows, with the labels the page shows.
 *
 * `id` is the contract (`transit::ranking::PRIORITIES`); `label` is presentation.
 * Deliberately no numbers here: the weights are the server's, and a copy of them
 * would be a second home for the same fact.
 */
export const JOURNEY_PRIORITIES: { id: string; label: string; hint: string }[] = [
  { id: 'balanced', label: 'Balanced', hint: 'no term dominates' },
  { id: 'cheapest', label: 'Cheapest', hint: 'fare first' },
  { id: 'fastest', label: 'Fastest', hint: 'door to door' },
  { id: 'fewest_changes', label: 'Fewest changes', hint: 'direct where possible' },
  { id: 'reliable', label: 'Most reliable', hint: 'measured, not fitted' },
];

export const transit = {
  suggest: (q: string) => request<Station[]>(`/api/suggest?q=${encodeURIComponent(q)}`),
  // Journey, not a narrower shape of its own: /api/search and /api/split's segments are
  // the same serialized type on the server, start_station and end_station included. The
  // old client declared a second interface without those two fields, so a caller could
  // not name where a journey actually started.
  // `from` null omits the parameter, and transit then starts from the profile's first
  // home station (`capabilities/transit/src/server.rs`, RouteQuery::origin).
  search: (from: string | null, to: string, time: string, override?: JourneyOverride) =>
    request<Journey[]>(
      `/api/search?${from === null ? '' : `from=${encodeURIComponent(from)}&`}to=${encodeURIComponent(to)}&time=${encodeURIComponent(time)}${overrideQuery(override)}`,
    ),
  split: (from: string, to: string, time: string) =>
    request<SplitResult>(
      `/api/split?from=${encodeURIComponent(from)}&to=${encodeURIComponent(to)}&time=${encodeURIComponent(time)}`,
    ),
};

/// Calendar pulls a plan's stages and booking deadlines only when asked
/// (capabilities/calendar/src/server.rs, sync_trip_plan), and until 2026-09-25
/// nothing asked, so no trip ever reached the calendar. A stage or booking change
/// asks. Fire-and-forget: the plan write already succeeded, and a calendar that is
/// down must not turn it into an error.
function syncTripToCalendar(planId: string): void {
  request<unknown>(
    `/calendar/api/trip-plans/${encodeURIComponent(planId)}/sync`,
    jsonInit('POST', {}),
  ).catch((err) => console.warn('calendar trip sync failed', err));
}

export const trips = {
  list: () => request<TripPlan[]>('/trips/api/plans'),
  create: (plan: {
    title: string;
    origin: PlaceRef;
    destinations: PlaceRef[];
    date_start: string;
    date_end: string;
    interests: string;
    travelers: string[];
    transport_modes: TransportMode[];
  }) => request<TripPlan>('/trips/api/plans', jsonInit('POST', plan)),
  update: (
    id: string,
    patch: Partial<
      Pick<
        TripPlan,
        | 'title'
        | 'origin'
        | 'destinations'
        | 'date_start'
        | 'date_end'
        | 'interests'
        | 'status'
        | 'travelers'
        | 'transport_modes'
        | 'stages'
        | 'cover_image_url'
        | 'budget_cents'
        | 'currency'
      >
    >,
  ) =>
    request<TripPlan>(
      `/trips/api/plans/${encodeURIComponent(id)}`,
      jsonInit('PATCH', patch),
    ).then((plan) => {
      if ('stages' in patch || 'status' in patch) syncTripToCalendar(id);
      return plan;
    }),
  get: (id: string) => request<PlanDetails>(`/trips/api/plans/${encodeURIComponent(id)}`),
  delete: (id: string) =>
    request<void>(`/trips/api/plans/${encodeURIComponent(id)}`, { method: 'DELETE' }),
  addItem: (
    planId: string,
    item: {
      item_type: PlanItemType;
      day: string | null;
      external_id: string;
      title: string;
      payload: unknown;
    },
  ) =>
    request<PlanItem>(
      `/trips/api/plans/${encodeURIComponent(planId)}/items`,
      jsonInit('POST', item),
    ).then((saved) => {
      if (item.item_type === 'booking') syncTripToCalendar(planId);
      return saved;
    }),
  deleteItem: (planId: string, itemId: string) =>
    request<void>(
      `/trips/api/plans/${encodeURIComponent(planId)}/items/${encodeURIComponent(itemId)}`,
      { method: 'DELETE' },
    ),
  scanObsidian: () =>
    request<ObsidianTripCandidate[]>('/trips/api/import/obsidian/scan'),
  importObsidian: (reference: string, origin: PlaceRef) =>
    request<TripPlan>(
      '/trips/api/import/obsidian',
      jsonInit('POST', { reference, origin }),
    ),
  importAllObsidian: (origin: PlaceRef) =>
    request<ObsidianImportAllResult>(
      '/trips/api/import/obsidian/all',
      jsonInit('POST', { origin }),
    ),
  bases: (req: BaseRequest) =>
    request<BaseResult>('/trips/api/bases', jsonInit('POST', req)),
  setItemDay: (planId: string, itemId: string, day: string | null) =>
    request<PlanItem>(
      `/trips/api/plans/${encodeURIComponent(planId)}/items/${encodeURIComponent(itemId)}`,
      jsonInit('PATCH', { day }),
    ),
  draftIntent: (sentence: string) =>
    request<IntentDraft>('/trips/api/intent/draft', jsonInit('POST', { sentence })),
};

export const traveler = {
  profile: () => request<TravelProfileResponse>('/traveler/api/profile'),
  updateProfile: (profile: TravelProfile, expected_revision?: number) =>
    request<TravelProfileResponse>(
      '/traveler/api/profile',
      jsonInit('PUT', { profile, expected_revision }),
    ),
  derived: () => request<DerivedTravelStats>('/traveler/api/profile/derived'),
};

export const wikimedia = {
  placeImage: (title: string) => {
    const params = new URLSearchParams({
      action: 'query',
      format: 'json',
      origin: '*',
      prop: 'pageimages|info',
      inprop: 'url',
      piprop: 'thumbnail|name',
      pithumbsize: '1280',
      pilicense: 'free',
      redirects: '1',
      titles: title,
    });
    return request<unknown>(`https://de.wikipedia.org/w/api.php?${params}`);
  },
  nearby: (latitude: number, longitude: number) => {
    const params = new URLSearchParams({
      action: 'query',
      format: 'json',
      origin: '*',
      generator: 'geosearch',
      ggscoord: `${latitude}|${longitude}`,
      ggsradius: '10000',
      ggslimit: '12',
      prop: 'pageimages|info|coordinates|description',
      inprop: 'url',
      piprop: 'thumbnail|name',
      pithumbsize: '640',
    });
    return request<unknown>(`https://de.wikipedia.org/w/api.php?${params}`);
  },
};

/**
 * Whether a separately-priced segment is priced for the train you will be on.
 * `different` is the one that costs money: that fare buys a seat on another
 * service, so the tickets do not add up to the journey you planned.
 */
export type TrainMatch = "exact" | "partial" | "different" | "unknown";

export interface SplitSegment {
  journey: Journey;
  train_match: TrainMatch;
  /** The trains the direct journey uses over this hop, in route order. */
  expected_trains: string[];
}

export interface SplitResult {
  segments: SplitSegment[];
  /** Null when the direct fare is unknown, which is not the same as free. */
  original_price: number | null;
  split_price: number;
  /** Null when there is no direct fare to compare against. */
  savings: number | null;
  confidence: "exact" | "partial" | "low";
  /**
   * Stop pairs the solver wanted a fare for and got none. The chain shown is
   * fully priced, but a cheaper split may never have been visible to it.
   */
  unpriced_pairs: number;
  queried_pairs: number;
}

/**
 * What happened to comparable trains at this journey's destination: the arriving
 * train's type, in the arrival hour, weekday or weekend.
 *
 * Not a forecast for this journey and not transfer risk -- transit's punctuality
 * module says why this data cannot produce either. Absent means punctuality has
 * no cell, or the cell was thinner than its sample floor, or the service is not
 * running. None of the three is a low risk, so absence renders as unknown and
 * never as punctual.
 */
export interface ArrivalPunctuality {
  station_name: string | null;
  train_type: string;
  hour: number;
  weekend: boolean;
  /** Observations behind every other field. What separates a statistic from a coincidence. */
  n: number;
  mean_delay: number;
  p50: number;
  p90: number;
  share_late_6: number;
  cancel_rate: number;
}

export interface Journey {
  id: string;
  start_station: Station;
  end_station: Station;
  legs: ConnectionLeg[];
  total_duration_minutes: number;
  total_price: number | null;
  /** `arrival_punctuality.share_late_6`, flattened. Prefer the cell: it carries `n`. */
  delay_risk_score: number | null;
  arrival_punctuality?: ArrivalPunctuality | null;
  /**
   * How likely the whole journey holds together, from the same measured history.
   * Absent whenever any term is unknown -- a product with a guessed factor in it
   * is not a measurement, and rendering absence as a low risk invents one.
   */
  reliability?: JourneyReliability | null;
  /**
   * Where this journey placed against the traveller's own weights, and why.
   *
   * Absent when nothing has been stated on the profile, when traveler is
   * unreachable, or when the caller overrode nothing and there is nothing to
   * apply. Absence means the order is the backend's own -- which is why the page
   * must not re-sort when this is present.
   */
  ranking?: JourneyRanking | null;
}

export interface JourneyReliability {
  probability: number;
  threshold_minutes: number;
  final_leg_on_time: number;
  min_sample: number;
}

/**
 * The envelope is deliberately the same `{key, label, score, weight, rationale}`
 * shape `plan_search` publishes, so a reader meets one vocabulary for "why is this
 * ranked here" at both grains.
 */
export interface JourneyRanking {
  score: number;
  rank: number;
  factors: RankFactor[];
  weights: {
    price: number;
    duration: number;
    changes: number;
    reliability: number;
  };
  /** `profile` or `request` -- where those four numbers came from. */
  source: string;
}

export interface RankFactor {
  key: string;
  label: string;
  score: number;
  weight: number;
  rationale: string;
}

export interface ConnectionLeg {
  origin: { name: string; id: string };
  destination: { name: string; id: string };
  /** The time to plan around: real-time where HAFAS has one, scheduled otherwise. */
  departure_time: string;
  arrival_time: string;
  /**
   * Scheduled and real-time kept apart, exactly as transit sends them.
   *
   * These were on the wire on every search and absent from this interface, which
   * is why a leg running twenty minutes late rendered identically to one on time.
   * A null real-time value means HAFAS offered none -- not that there is no delay.
   */
  scheduled_departure?: string | null;
  realtime_departure?: string | null;
  scheduled_arrival?: string | null;
  realtime_arrival?: string | null;
  /** HAFAS marked the leg cancelled. Previously invisible: it arrived as an ordinary leg. */
  cancelled?: boolean;
  train_name: string;
  train_number: string;
  train_category: string;
  is_regional: boolean;
  platform?: string | null;
}

// ─── Interior ────────────────────────────────────────────────────────────────

/** One measured or wanted thing. Mirrors `capabilities/interior/src/store.rs::Item`. */
export interface InteriorItem {
  id: string;
  kind: 'piece' | 'slot';
  label: string;
  b: number | null;
  t: number | null;
  h: number | null;
  h_min: number | null;
  laenge: number | null;
  anzahl: number | null;
  unsicher: string[];
  zustaende: string[];
  preis_cent: number | null;
  kosten_min_cent: number | null;
  kosten_max_cent: number | null;
  link: string | null;
  quelle: string | null;
  gemessen_am: string | null;
  mitnahme: string | null;
  prioritaet: string | null;
  ziel: string | null;
  hinweis: string | null;
  begruendung: string | null;

  /**
   * What this piece states it needs (PRD Q61 / B26).
   *
   * Fill any of them and the clearance check measures the piece against exactly these and stops
   * guessing from its name — the guess that once checked a coffee table against a sofa's rules,
   * because `^couch` caught `couchtisch`. Leave them null and the name still decides.
   *
   * `opens` and `expands_dir` are in the piece's own orientation and turn with `rot`.
   */
  opens: 'nord' | 'sued' | 'ost' | 'west' | null;
  open_clear: number | null;
  wall_ok: boolean | null;
  expands_dir: 'nord' | 'sued' | 'ost' | 'west' | null;
  /** Total extent when unfolded, not the increase — a product page names the total. */
  expands_to: number | null;
  access_sides: number | null;
  access_clear: number | null;
  /** Meant to stand free. Affects the search ranking only, never a verdict. */
  raumtrenner: boolean | null;
  /**
   * A picture of this piece, as a path below the private interior asset root.
   *
   * Served by `GET /interior/api/media/<path>` on request only — the reason the capability may
   * sit in a public repository is that the bundle carries no photograph.
   */
  bild: string | null;

  /**
   * What the piece is FOR — `kleidung`, `schlafen`, `kochen`, `elektronik`. Free text: the list
   * comes from the notes and no rule runs on it. This is what makes a garment a garment; the
   * `kind` stays `piece`, because a shirt is a thing you own and not a need with target sizes.
   */
  category: string | null;
  /**
   * Clothing, as the label writes it — `M`, `42`, `60x60`. Free text on purpose: an invented
   * scale (`S..XXL`) would be the next shop that does not fit it.
   */
  groesse: string | null;
  /** `weiss`, `dunkelblau`, `gestreift`. Free, for the same reason. */
  farbe: string | null;
  /**
   * When it is worn — `["ganzjahr"]`, `["winter"]`, `["uebergang"]`. A list, because a coat
   * is winter AND transition, not one or the other.
   */
  saison: string[];
  /**
   * How often the row was written; the server owns it (PRD §10 A5). Send the value you read as
   * `revision` to `saveItem`/`patchItem`, and a write that raced another device fails with
   * `InteriorConflict` instead of overwriting it.
   */
  revision: number;
}

/**
 * What `PUT`/`PATCH /api/items/:id` answer on success. In the app, an edit made while the Mac
 * cannot be reached is queued on the device instead (`src-tauri/src/sync.rs`): the answer then
 * carries `queued: true`, and `revision` only once the queue has sent it (`state: 'sent'`).
 */
export type InteriorWriteResult =
  | { id: string; ok: boolean; revision: number; queued?: undefined }
  | { queued: true; outbox_id: number; state: 'pending' | 'sent' | 'conflict' | 'failed'; revision?: number };

/**
 * One inventory row. `pending` and `conflict` are set only in the app, by the device's outbox:
 * `pending` means the item shows an edit that has not reached the Mac yet.
 */
export interface InteriorInventoryRow {
  item: InteriorItem;
  state: InteriorState | null;
  pending?: boolean;
  conflict?: boolean;
}

/**
 * A write carried a stale revision: another device changed the entry after this page read it.
 *
 * `current` is the entry as it stands now, so the page can show it instead of retrying and
 * overwriting. It is read again after the 409 because `request()` keeps only the message of an
 * error body; `null` if that re-read fails.
 */
export class InteriorConflict extends ApiError {
  current: { item: InteriorItem; state: InteriorState | null } | null;
  constructor(message: string, current: InteriorConflict['current']) {
    super(409, message);
    this.name = 'InteriorConflict';
    this.current = current;
  }
}

/** `init` with `If-Match: "<revision>"` added, or unchanged when no revision is known. */
function ifMatch(init: RequestInit, revision: number | undefined): RequestInit {
  if (revision === undefined) return init;
  return { ...init, headers: { ...(init.headers as Record<string, string>), 'If-Match': `"${revision}"` } };
}

/** One conditional item write. A 409 becomes `InteriorConflict`; it is never retried. */
async function writeItem(id: string, init: RequestInit, revision: number | undefined): Promise<InteriorWriteResult> {
  try {
    return await request<InteriorWriteResult>(`/inventory/api/items/${encodeURIComponent(id)}`, ifMatch(init, revision));
  } catch (caught) {
    if (!(caught instanceof ApiError) || caught.status !== 409) throw caught;
    const current = await request<{ item: InteriorItem; state: InteriorState | null }[]>('/inventory/api/inventory')
      .then((rows) => rows.find((row) => row.item.id === id) ?? null)
      .catch(() => null);
    throw new InteriorConflict(caught.message, current);
  }
}

export type InteriorState = 'owned' | 'wanted' | 'gone';

export interface InteriorViolation {
  rule: string;
  severity: 'hart' | 'weich';
  item: string | null;
  message: string;
  /**
   * The rule's own wording from the flat's `rules.toml`, for a house rule (R1…R8).
   *
   * Absent on invariants like `kollision` or `raumgrenze`: no flat declares that two pieces
   * may not overlap, so there is no text to quote. `message` says what this layout gets
   * wrong; `text` says what makes it a rule at all.
   */
  text?: string;
  measured: number | null;
  required: number | null;
}

/** A rule the flat declares and the engine does not check. */
export interface InteriorUncheckedRule {
  rule: string;
  text: string;
  /**
   * Why it did not run. Two kinds, one consequence.
   *
   * "not implemented" — the flat declares it, the engine has no check for it.
   * "not applicable"  — the engine has one, but a measurement or declaration is missing. This
   *                     is the dangerous kind: it reads as a pass. The wardrobe sat in the light
   *                     corridor with no measured height in three layouts, and R3 skipped it in
   *                     silence.
   */
  grund: string;
}

/**
 * How much room a check has left — recorded whether it fires or not.
 *
 * A verdict is one bit, and one bit cannot say whether a layout passes by 2 cm or by 40. Both
 * report `pass`, and only one survives a tape measure that was a centimetre out.
 */
export interface InteriorReserve {
  rule: string;
  item?: string;
  /** The other side of the comparison where there is one: the opening, the built-in, the route. */
  bezug?: string;
  measured: number;
  required: number;
  grenze: 'mindestens' | 'hoechstens';
  /** What is left. **Negative means missed by that much.** */
  slack: number;
  /** `cm`, `seiten`, `plaetze`, `stunden`. Only `cm` feeds `engste_reserve_cm`. */
  einheit: string;
  hart: boolean;
  /** The measurement hit its horizon: it is *at least* this much, not exactly. */
  gedeckelt: boolean;
  /**
   * Does a POSITIVE value here say anything about how much room is left?
   *
   * True for almost every rule. False for the room boundary: a wardrobe against the wall sits
   * a centimetre from it because that is where it belongs. Counting that would make every
   * sensibly furnished flat report "1 cm spare".
   */
  bindend: boolean;
}

/** How much measurement error a verdict survives. */
export type InteriorHaltbarkeit =
  | { art: 'faellt_durch' }
  | { art: 'nichts_geraten' }
  | { art: 'bis'; cm: number }
  | { art: 'ueber_horizont'; horizont_cm: number };

export interface InteriorRobustheit {
  layout: string;
  nominal_pass: boolean;
  engste_reserve_cm: number | null;
  haelt: InteriorHaltbarkeit;
  /** Which rules break first, one centimetre past `haelt`. */
  kippt_an: string[];
  geraten: { reference: string; label: string; fields: string[] }[];
  /** Pieces carrying their own `size:` in the layout — the perturbation does not reach them. */
  nicht_variiert: string[];
}

/** Does the piece get through the door? */
export type InteriorTuerpass =
  | { art: 'passt'; luft_cm: number; tuer_cm: number }
  | { art: 'passt_nicht'; fehlen_cm: number; tuer_cm: number }
  | { art: 'zerlegt_getragen'; fehlen_cm: number; tuer_cm: number }
  | { art: 'kein_eingang' };

export interface InteriorEinbringung {
  reference: string;
  b: number;
  t: number;
  tuer: InteriorTuerpass;
  erreichbar: boolean;
  schritte?: number;
  dreht: boolean;
  grund?: string;
}

export interface InteriorSonnenstunde {
  tag: string;
  stunde_lokal: number;
  hoehe_grad: number;
  azimut_grad: number;
  getroffen: string[];
}

export interface InteriorSonne {
  layout: string;
  stunden: InteriorSonnenstunde[];
  treffer_je_stueck: Record<string, number>;
  /** Glazing without measured heights throws no light here — and must. */
  ohne_glashoehen: string[];
}

export interface InteriorDeklaration {
  id: string;
  label: string;
  erklaert: boolean;
  geraten_als: string;
  geerbte_schwellen: [string, number][];
  in_layouts: string[];
  vorschlag?: {
    open_clear: number | null;
    access_sides: number | null;
    access_clear: number | null;
    expands_dir: string | null;
    expands_to: number | null;
    toml: string;
  };
  folgen?: {
    layouts: number;
    bestanden_vorher: number;
    bestanden_nachher: number;
    geaendert: string[];
  };
}

export interface InteriorKaufposten {
  id: string;
  label: string;
  prioritaet: string | null;
  preis_cent: number | null;
  in_layouts: string[];
  kumuliert_cent: number;
  erreichbar_nach_monaten: number | null;
}

export interface InteriorKaufreihenfolge {
  posten: InteriorKaufposten[];
  /** Needs with no price at all. They are **not** in the order — zero would buy them first. */
  ohne_preis: InteriorKaufposten[];
  saldo: { median_cent: number; monate: number; von: string; bis: string } | null;
  /**
   * Priority words in the data this ranking does not know.
   *
   * They sort last and are *reported* rather than silently ranked. That is the lesson from the
   * bug itself: the first version knew three of the flat's eight words and still looked sorted.
   */
  unbekannte_prioritaeten: string[];
}

/**
 * A long computation, addressed by number.
 *
 * The exhaustive search checks millions of candidates and takes minutes. An HTTP request held
 * open that long is a bet on every proxy in between, which is why the search was reachable
 * only from the command line until 2026-08-31 — the most expensive calculation in the
 * capability, missing from the surface that needs it.
 */
export type InteriorAuftragsstand =
  | { zustand: 'laeuft'; seit_ms: number }
  | { zustand: 'fertig'; ergebnis: unknown }
  | { zustand: 'gescheitert'; grund: string };

export interface InteriorHit {
  places: Record<string, [number, number]>;
  soft: number;
  bottleneck_cm: number;
  engste_reserve_cm: number | null;
  /** On the Pareto front: not worse than another under any weighting of the four goals. */
  pareto: boolean;
  wandkontakt_cm: number;
}

export interface InteriorSearchReport {
  base: string;
  moved: string[];
  step: number;
  candidates_after_filter: number;
  fully_checked: number;
  hits: InteriorHit[];
  elapsed_ms: number;
}

export interface InteriorComposed {
  places: [string, [number, number], number][];
  pass: boolean;
  hard: string[];
  soft: string[];
  wandkontakt_cm: number;
  bottleneck_cm: number;
  free_m2: number;
  engste_reserve_cm: number | null;
  pareto: boolean;
}

/**
 * The measured room itself, as `room.toml` states it.
 *
 * `masse` is the main room's outer extent in centimetres, computed by the capability from the
 * same polygon every check rasterises — the page shows it and does not measure it, for the same
 * reason it does not decide a verdict.
 */
export interface InteriorModel {
  flat: { id: string; name: string };
  area_m2: number;
  masse: { b: number; t: number };
  /** 0 means not measured. `room.toml` invents no placeholder ceiling height. */
  hoehe: number;
  polygon: [number, number][];
  katalog_groesse: number;
  todo: string[];
}

export interface InteriorLayoutSummary {
  id: string;
  name: string;
  pass: boolean;
  hard: number;
  soft: number;
  corridors: { from: string; to: string; width_cm: number | null }[];
  occupied_m2: number;
}

/** The full check plus the finished plan. The SVG is built by the capability, never here. */
export interface InteriorLayoutDetail {
  layout: { name: string; id: string; item: InteriorPlacedItem[] };
  check: {
    layout: string;
    pass: boolean;
    hard: InteriorViolation[];
    soft: InteriorViolation[];
    uncertainties: { reference: string; label: string; fields: string[] }[];
    /**
     * Declared, not measured — so "passes" means "passes the rules that were checked".
     *
     * The real flat declares two of these (glare at the desk, the sightline from the door to
     * the bed). Before 2026-08-31 they fell out silently and a partial verdict read as a
     * complete one.
     */
    nicht_geprueft: InteriorUncheckedRule[];
    /** Pieces in this layout that have already been decided against (`prioritaet: verworfen`). */
    veraltet: string[];
    /** Every measurement with its threshold — the ones that passed too. */
    reserven: InteriorReserve[];
    /**
     * The tightest hard measurement in cm: by how much this layout passes.
     *
     * `null` when no hard rule measured in centimetres — not `0`, which would read as "passes
     * by nothing" where "nothing was measured" is meant. Negative when a hard rule is broken.
     */
    engste_reserve_cm: number | null;
    metrics: {
      room_area_m2: number;
      occupied_area_m2: number;
      free_area_m2: number;
      corridors: { from: string; to: string; width_cm: number | null }[];
    };
  };
  svg: string;
}

export interface InteriorWishlist {
  items: InteriorItem[];
  summe_untere_kante_cent: number;
  summe_obere_kante_cent: number;
  posten_ohne_preis: number;
  monatssaldo: { median_cent: number; monate: number; von: string; bis: string } | null;
  monate_bis_bezahlt: number | null;
}

/**
 * The room planner.
 *
 * Every verdict, every corridor width and the plan SVG itself come from these endpoints.
 * Nothing on the page recomputes a clearance rule: a second implementation of one is exactly
 * the drift the capability exists to prevent (PRD B27).
 */
export interface InteriorRoomPlanReference {
  flat: string;
  status: "raw-only";
  revision: string | null;
  asset: { format: "usdz"; byte_length: number; sha256: string; url: string };
  observation: {
    export_observation: { room_groups: number; mesh_assets: number; category_counts: Record<string, number> };
  };
  manifest: unknown;
}

export interface InteriorRoomPlanRevisions {
  flat: string;
  revisions: import("$lib/roomplan").RoomPlanDraft[];
}

export const interior = {
  roomplanReference: () => request<InteriorRoomPlanReference>('/interior/api/roomplan/reference'),
  roomplanRevisions: () => request<InteriorRoomPlanRevisions>('/interior/api/roomplan/revisions'),
  reviewRoomplanRevision: (revisionId: string, decision: 'accept' | 'reject', note?: string) =>
    request<{ decision: string }>('/interior/api/roomplan/revisions/' + encodeURIComponent(revisionId) + '/review', jsonInit('POST', { decision, note })),
  roomplanAssetUrl: () => '/interior/api/roomplan/asset',
  model: () => request<InteriorModel>('/interior/api/model'),
  layouts: () => request<InteriorLayoutSummary[]>('/interior/api/layouts'),
  layout: (name: string) =>
    request<InteriorLayoutDetail>(`/interior/api/layouts/${encodeURIComponent(name)}`),
  inventory: () => request<InteriorInventoryRow[]>('/inventory/api/inventory'),
  wishlist: () => request<InteriorWishlist>('/inventory/api/wishlist'),
  /**
   * Replace an entry. Pass the `revision` you read: a stale one throws `InteriorConflict`.
   * Without it the later write wins, which a client that can be offline must not rely on.
   */
  saveItem: (id: string, item: InteriorItem, revision?: number) =>
    writeItem(id, jsonInit('PUT', item), revision),

  /**
   * Change named fields and leave the rest alone.
   *
   * Prefer this over `saveItem` from a form. `PUT` replaces the whole row, and an entry has 40
   * fields while a form shows six — everything it does not send would be blanked. An explicit
   * `null` clears a field; an absent key leaves it. An unknown key is refused rather than
   * ignored, which is the same stance `deny_unknown_fields` takes on import.
   *
   * Pass the `revision` the form was opened with. If another device saved in between, this
   * throws `InteriorConflict` with the current values and writes nothing.
   */
  patchItem: (id: string, patch: Partial<InteriorItem>, revision?: number) =>
    writeItem(id, jsonInit('PATCH', patch), revision),

  /** Create an entry. `state` is required: without it the row joins to nothing and is invisible. */
  createItem: (item: Partial<InteriorItem> & { id: string; kind: 'piece' | 'slot'; label: string; state: InteriorState; note?: string }) =>
    request<{ id: string; state: InteriorState; ok: boolean }>(
      '/inventory/api/items',
      jsonInit('POST', item),
    ),

  /**
   * Append a state change. Appends, never overwrites — a wish that gets bought is a second row,
   * and that span is what the wishlist later joins to money with (PRD B25).
   *
   * `changed: false` means the state already held: not an error, and not an invented second row.
   */
  setState: (id: string, state: InteriorState, note?: string) =>
    request<{ id: string; state: InteriorState; changed: boolean }>(
      `/inventory/api/items/${encodeURIComponent(id)}/state`,
      jsonInit('POST', { state, note }),
    ),

  stateHistory: (id: string) =>
    request<{ state: InteriorState; since: string; note: string | null }[]>(
      `/inventory/api/items/${encodeURIComponent(id)}/state`,
    ),

  /**
   * What a change would do to the verdicts, without writing it.
   *
   * The clearance fields are exactly the ones whose effect you cannot see before setting them:
   * declaring which side the wardrobe opens costs 2 or 4 layouts depending on the direction,
   * and nothing said so until it was computed by hand. The form shows the same arithmetic
   * before you save.
   */
  /** Replace a layout's positions and get the fresh verdict + plan back. The file's header survives. */
  saveLayout: (id: string, items: InteriorPlacedItem[]) =>
    request<InteriorLayoutDetail>(
      `/interior/api/layouts/${encodeURIComponent(id)}`,
      jsonInit('PUT', { items }),
    ),

  /**
   * Verdict and plan for an arrangement, without writing it.
   *
   * Needed for rotation: at 90° width and depth swap, and `opens`/`expands_dir` turn with the
   * piece. Rebuilding that in the browser would be a second copy of `footprint` and
   * `Seite::gedreht` — so the page asks instead of guessing.
   */
  previewLayout: (id: string, items: InteriorPlacedItem[]) =>
    request<InteriorLayoutDetail>(
      `/interior/api/layouts/${encodeURIComponent(id)}/preview`,
      jsonInit('POST', { items }),
    ),

  /**
   * Where a piece's top-left corner may sit, as run-lengths per grid row.
   *
   * Hard edges for dragging. The page receives a LIST and computes nothing: the main room is a
   * hexagon, so clamping to a bounding box would park furniture in the notch the bathroom
   * occupies, and doing it properly means `point_in_polygon` — which belongs in one place.
   */
  allowedPositions: (layout: string, ref: string, rot: number) =>
    request<InteriorAllowed>(
      `/interior/api/layouts/${encodeURIComponent(layout)}/allowed?ref=${encodeURIComponent(ref)}&rot=${rot}`,
    ),

  /**
   * Create a layout. Never overwrites an existing one.
   *
   * Three ways to start, one body: empty (neither `von` nor `items`), a copy of another layout
   * (`von`), or a finished arrangement (`items`). `notiz` becomes the file's first comment line
   * and is where the reasoning goes; left out, the capability writes the date and the fact that
   * the API made it, rather than claiming a provenance nobody has.
   */
  createLayout: (plan: {
    id: string;
    name?: string;
    von?: string;
    items?: InteriorPlacedItem[];
    notiz?: string;
  }) =>
    request<InteriorLayoutDetail & { id: string }>(
      '/interior/api/layouts',
      jsonInit('POST', plan),
    ),

  /**
   * Take a layout out of the list. It moves to `layouts/archiv/`; nothing is deleted.
   *
   * The file's header carries why a piece was ruled out (PRD Q60), and that argument is needed
   * exactly when the same piece comes up again. Deleting the file deletes the argument.
   */
  archiveLayout: (id: string) =>
    request<{ id: string; archiviert: string; geloescht: boolean }>(
      `/interior/api/layouts/${encodeURIComponent(id)}`,
      { method: 'DELETE' },
    ),

  /** Record where the pieces actually stand, as opposed to what a layout proposes. */
  savePlacements: (items: InteriorPlacedItem[]) =>
    request<{ flat: string; gesetzt: number }>(
      '/interior/api/placements',
      jsonInit('PUT', { items }),
    ),

  /**
   * Where the pieces actually stand, as rows.
   *
   * The as-is layer, and not a proposal: a placement is one row per piece, a layout is a file.
   * The `ref` the page drags is called `item_id` here, because that is what the column is named.
   */
  placements: (flat: string) =>
    request<Array<{ item_id: string; flat: string; x: number; y: number; rot: number }>>(
      `/interior/api/placements/${encodeURIComponent(flat)}`,
    ),

  /**
   * What the as-is arrangement looks like, and whether it would pass.
   *
   * A second endpoint beside `previewLayout` because the arrangement arrives in the body: a
   * placement is a row in the store, not a file whose header carries the reason a piece stands
   * where it does (PRD Q60). The plan and the verdict are the same computation either way.
   */
  previewPlacements: (items: InteriorPlacedItem[]) =>
    request<InteriorLayoutDetail>('/interior/api/placements/preview', jsonInit('POST', { items })),

  /** Hard edges for dragging in the as-is layer. The arrangement it must fit around is the body. */
  allowedPositionsFor: (items: InteriorPlacedItem[], ref: string, rot: number) =>
    request<InteriorAllowed>(
      '/interior/api/placements/allowed',
      jsonInit('POST', { items, ref, rot }),
    ),

  /** URL for a picture stored in the overlay. Nothing is embedded; it is fetched when shown. */
  mediaUrl: (path: string) =>
    `/interior/api/media/${path.split('/').map(encodeURIComponent).join('/')}`,

  impact: (id: string, patch: Partial<InteriorItem>) =>
    request<InteriorImpact>(
      `/interior/api/items/${encodeURIComponent(id)}/impact`,
      jsonInit('POST', patch),
    ),

  /** Up to how much measurement error the verdict holds, and what breaks first past it. */
  toleranz: (layout: string) =>
    request<InteriorRobustheit>(
      `/interior/api/layouts/${encodeURIComponent(layout)}/toleranz`,
    ),

  /** When over the year each piece stands in direct sun. Needs `[lage]` in the flat's room.toml. */
  sonne: (layout: string) =>
    request<InteriorSonne>(`/interior/api/layouts/${encodeURIComponent(layout)}/sonne`),

  /** Does each piece get through the door and to its place? */
  einbringung: (layout: string) =>
    request<InteriorEinbringung[]>(
      `/interior/api/layouts/${encodeURIComponent(layout)}/einbringung`,
    ),

  /** The question before buying: does a piece this size fit through the entrance at all? */
  passt: (b: number, t: number, zerlegbar = false) =>
    request<InteriorTuerpass>(
      `/interior/api/passt?b=${b}&t=${t}&zerlegbar=${zerlegbar}`,
    ),

  /** Who is still judged by name, what the line would say, and what it changes. */
  deklaration: () => request<InteriorDeklaration[]>('/interior/api/deklaration'),

  /** Which need first, cumulative, and when it is reached out of the monthly balance. */
  kaufen: () => request<InteriorKaufreihenfolge>('/interior/api/kaufen'),

  /** Start a search. Answers with a job number, not a result. */
  search: (layout: string, move_refs: string[], step: number, limit: number) =>
    request<{ auftrag: number }>(
      '/interior/api/search',
      jsonInit('POST', { layout, move_refs, step, limit }),
    ),

  /** Have the machine lay out the whole flat. Also a job: the beam search takes minutes. */
  compose: (refs: string[], step: number, beam: number, limit: number) =>
    request<{ auftrag: number }>(
      '/interior/api/compose',
      jsonInit('POST', { refs, step, beam, limit }),
    ),

  auftrag: (id: number) =>
    request<{ id: number; stand: InteriorAuftragsstand }>(
      `/interior/api/auftraege/${id}`,
    ),
};

export interface InteriorPlacedItem {
  ref: string;
  x: number;
  y: number;
  rot: number;
  /** Overrides the catalogue footprint — the second state of a folding piece. */
  size?: [number, number] | null;
  kind?: string | null;
}

export interface InteriorAllowed {
  reference: string;
  rot: number;
  w: number;
  d: number;
  step: number;
  /** Per row: the inclusive x ranges the top-left corner may take. */
  rows: { y: number; x: [number, number][] }[];
}

export interface InteriorImpact {
  item: string;
  layouts: number;
  bestanden_vorher: number;
  bestanden_nachher: number;
  geaendert: {
    layout: string;
    vorher: { pass: boolean; hard: string[]; soft: string[] };
    nachher: { pass: boolean; hard: string[]; soft: string[] };
  }[];
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

export interface CapabilityView {
  name: string;
  kind: 'process' | 'container';
  scope: 'capability' | 'spine';
  port: string;
  panel_port: string;
  panel_path: string;
  autostart: string;
  requires: string[];
  /** Id kinds this capability answers `GET /api/links` for (libs/links/ISA.md D1). Optional
   *  because a sjel-status built before 2026-10-06 does not send it. */
  links_to?: string[];
  /** null when the capability declares no health surface — unknown, not down. */
  up: boolean | null;
  health_url: string | null;
}

/** One row of another capability that references a typed id (libs/links, `Link`). */
export interface Link {
  id: string;
  kind: string;
  title: string;
  at?: string;
  meta?: string;
  /** The field that holds the reference, such as `trip_id`. */
  via: string;
}

/** The body of `GET /<capability>/api/links?to=<id>`. */
export interface Links {
  to: string;
  links: Link[];
  /** Rows that reference the id but have no linkable id of their own. */
  unlinkable: number;
}

export const links = {
  /** Every capability is proxied at `/<name>`, and `/api` passes even where `proxy_api_only`. */
  find: (capability: string, to: string) =>
    request<Links>(`/${encodeURIComponent(capability)}/api/links?to=${encodeURIComponent(to)}`),
};

/** A capability that serves its own UI (CONTRIBUTING.md#three-architectural-nouns) declares a panel port. */
export const hasPanel = (c: CapabilityView): boolean => c.panel_port !== '';

/**
 * Where a panel lives, as seen from THIS browser.
 *
 * Composed here rather than served by sjel-status, because a panel is loaded by a
 * browser and has to be reachable on the host that browser is already on. Serving
 * `127.0.0.1:<port>` to a shell opened at `localhost` makes the two different sites:
 * Chrome then partitions the frame's storage, and a framework whose client init touches
 * storage never hydrates, so the panel renders blank while every request returns 200.
 * Over Tailscale the same absolute address would point at the phone itself.
 */
export function panelUrl(c: CapabilityView): string {
  return `${location.protocol}//${location.hostname}:${c.panel_port}${c.panel_path}`;
}

/** One unit in the committed self-model. `service` exists only where a manifest does. */
export interface SelfUnit {
  name: string;
  kind: 'capability' | 'lib' | 'spine' | 'pack' | 'unknown';
  service?: { kind: string; port?: string; requires: string[]; image?: string };
  code?: { files: number; nodes: number };
}

/** Compile-time coupling: what is pulled into what. Not `requires`, which is runtime. */
export interface SelfCoupling {
  from: string;
  to: string;
  kinds: string[];
  evidence: string[];
}

export interface SelfModel {
  schema: number;
  generator: string;
  units: SelfUnit[];
  coupling: SelfCoupling[];
  upstreams: Array<{ name: string; verdict: string }>;
  graph: { present: boolean; nodes: number; external: number; stale: string[]; unmatched: string[] };
}

/**
 * The self-model as served: the committed artifact plus this machine's live state.
 *
 * Two keys, not one merged object, mirroring the endpoint. `model` is a fact about the
 * repo and is identical on every machine; `live` is a fact about this machine right now.
 * `up` is `null` where a capability declares nothing to poll — unknown, not down.
 */
export interface SelfModelResponse {
  model: SelfModel;
  live: Record<string, boolean | null>;
}

/**
 * Version identity for one of the two repos this installation is made of.
 *
 * Read-only by design: the shell shows which Sjel is running and links to it, and
 * deliberately cannot tag, commit or push — a browser button that writes to git needs a
 * gate in front of it, and there is no versioning scheme to write against yet.
 *
 * `tag` is null on a repo that has never been tagged, which today is both of them.
 * `describe` then degrades to a short sha, and that is the honest answer rather than a
 * missing one. `ahead`/`behind` are null when the branch tracks no upstream — not 0,
 * which would claim it is in sync with something.
 */
/** One operator-pinned link from the overlay's links.toml. Names and URLs only. */
export interface PinnedLink {
  name: string;
  url: string;
  note?: string;
}

export interface RepoStatus {
  name: string;
  role: 'spine' | 'overlay';
  remote_url: string | null;
  branch: string | null;
  describe: string | null;
  tag: string | null;
  commits_since_tag: number | null;
  ahead: number | null;
  behind: number | null;
  dirty: boolean;
  last_commit_date: string | null;
  error: string | null;
}

// `UpstreamEntry` / `UpstreamAudit` were here until 2026-08-28, typing the payload of
// `GET /sjel-status/upstreams` for the `/upstreams` page. Both are gone with that endpoint
// and that route: their `status` field ('ok' | 'na' | 'warn' | 'fail') was
// `tools/upstream-checker`'s verdict, and PRD Q41 retired the checker.
//
// The verdict half is not lost and never came from here: `SelfModel.upstreams` below
// carries `{name, verdict}` for every manifest entry, read from `upstreams.toml`
// by `tools/self generate`, and `/self` renders it.

/** How a capability's last backup stands against its own declared thresholds. */
export type BackupState =
  /** Has a backup contract and no receipt at all. Outranks every threshold. */
  | 'never'
  | 'ok'
  /** Past `advise_days`: you could take one. */
  | 'due'
  /** Past `stale_days`: you have a problem. */
  | 'overdue'
  /** The capability declares neither threshold, so nothing knows what timely means for
   *  its data — and Sjel will not invent a cadence to fill the gap. */
  | 'unknown';

/** An in-flight or finished run, as the server remembers it. Null when sjel-status has
 *  not been asked for a backup of this capability since it started. */
export interface BackupRun {
  state: 'running' | 'succeeded' | 'failed';
  started_at: number;
  finished_at?: number;
  detail?: string;
}

export interface BackupArchiveIdentity {
  name: string;
  bytes: number;
  sha256: string;
}

export interface BackupRunAttempt {
  id: number;
  capability: string;
  target: string;
  started_at: string;
  started_epoch: number;
  finished_at: string | null;
  finished_epoch: number | null;
  exit_code: number | null;
  archive: BackupArchiveIdentity | null;
  detail: string;
  log_path: string;
}

export interface BackupTargetView {
  id: string;
  kind: string;
  path: string;
  host: string;
  present: string; // "true" | "false" | "unchecked" | "unknown"
  declared_by: string[];
  seen_at: string;
  verified_at: string | null;
  verified_verdict: string | null; // "verified" | "failed" | "unchecked" | null
  verified_detail: string | null;
  interval_hours: number | null;
}

/**
 * One capability's backup standing.
 *
 * `attempt` carries the durable record of what the last run actually did, including failures
 * that left no fresh receipt behind.
 */
export interface BackupStatus {
  capability: string;
  state: BackupState;
  /** A run stops this capability while it takes a cold copy. Say so before confirming. */
  holds_service: boolean;
  advise_days: number | null;
  stale_days: number | null;
  last_success: string | null;
  age_seconds: number | null;
  bytes: number | null;
  contents: string | null;
  run: BackupRun | null;
  attempt?: BackupRunAttempt | null;
}

/** One thing wrong with this machine, as the hourly host watch found it.
 *
 *  It filed a task until PRD Q48 (2026-08-27). A runaway process is machine state, not
 *  an action a human wrote, so the findings became host-watch's own rows and the ladder
 *  ranks them directly. There is nothing to mark done: the next run closes a finding
 *  when the condition clears, which is the only honest close for a condition. */
export interface HostWatchFinding {
  id: string;
  /** The condition, e.g. `cpu:ApplicationsStorageExtension`. Names the command, never
   *  the pid, so a reboot does not look like a new problem. */
  key: string;
  title: string;
  /** What to run to look at it, and what to run if it is stuck. Multi-line. */
  note: string;
  first_seen: string;
  last_seen: string;
}

/** One reclaimable class in the storage report, as `tools/storage` names it. */
export interface StorageClass {
  name: string;
  bytes: number;
  /** Whether the overlay's policy lets `apply` reclaim it. `false` is report-only, and
   *  the tool never runs an empty reclaim command even here. */
  applicable: boolean;
  /** Over the policy's `class_flag_gb`. Loud on this machine, not necessarily wrong: a
   *  class can be over the flag with the disk nowhere near full. */
  flagged: boolean;
}

/** A path the policy reports and never reclaims, with the reason it is safe to leave. */
export interface StorageProtected {
  path: string;
  bytes: number;
  reason: string;
}

/** `sjel storage report --json`, passed through verbatim.
 *
 *  Served by sjel-status rather than by `tools/storage`, which is operator machinery with
 *  no server — it measures, prints and exits. `state` is `ok`, `warn` or `critical`, and
 *  it is the volume's state, never a class being large: a class over the flag on a machine
 *  with free space is not a fault (the tool's own rule, tested in
 *  tools/sjel-cli/src/host_watch/pure.rs). */
/** What an agent may do on one capability (ISA F10). */
export type AgentMode = 'off' | 'read-only' | 'ask' | 'auto';

/** One write that waits for the owner, or was decided. */
export interface AgentApproval {
  id: string;
  capability: string;
  method: string;
  path: string;
  preview: string;
  created_at: number;
  state: 'pending' | 'allowed' | 'denied' | 'used';
}

/** One logged agent call. Never a body or a query. */
export interface AgentCall {
  at: number;
  capability: string;
  method: string;
  path: string;
  status: number;
  decision: string;
}

export interface AgentView {
  enrolled: boolean;
  modes: AgentMode[];
  default_mode: AgentMode;
  capabilities: {
    capability: string;
    mode: AgentMode;
    confirm: string[];
    get_writes: string[];
  }[];
  pending: AgentApproval[];
  calls: AgentCall[];
}

export interface StorageReport {
  disk: { used: number; free: number; total: number; target: string };
  state: string;
  classes: StorageClass[];
  protected: StorageProtected[];
  expected_service: Array<{ kind: string; name: string; note: string }>;
}

/** Who owns moving one class of installed software. The tool's own vocabulary, verbatim:
 *  `scheduled` a capability moves it · `manual` a verb exists and nothing schedules it ·
 *  `unowned` nothing moves it · `self` the vendor does. */
export type UpdateOwner = 'scheduled' | 'manual' | 'unowned' | 'self';

/** current · stale · unknown · n/a. `unknown` is a claim the tool refuses to make rather
 *  than a soft "current" — a `--offline` report says so on every row it did not check. */
export type UpdateStatus = 'current' | 'stale' | 'unknown' | 'n/a';

/** One class of installed software and who moves it, as `tools/sjel-cli/src/updates/report.rs`'s `SURFACES`
 *  declares it. Sent with the report so the panel renders the tool's reasoning rather
 *  than a second copy of the ownership table. */
export interface UpdateSurface {
  id: string;
  title: string;
  owner: UpdateOwner;
  ownerDetail: string;
  /** Whether `apply` moves this class. A `self` class is never actionable: that would be
   *  a second updater for a binary that already has one. */
  actionable: boolean;
  why: string;
}

/** One row: a package, a crate, an integration or a class's own receipt line. */
export interface UpdateRow {
  surface: string;
  name: string;
  owner: UpdateOwner;
  ownerDetail: string;
  installed?: string;
  latest?: string;
  status: UpdateStatus;
  /** The command that moves it. Present only on rows the tool can act on. */
  action?: string;
  note?: string;
}

/** What `apply` recorded, written by the tool and carried on every report.
 *
 *  This is how the panel learns an outcome it did not wait for: the apply route answers
 *  `202` before the work starts, because a cargo step compiles for minutes and a held
 *  request would time out while the install succeeded. */
export interface UpdateApply {
  at?: string;
  class?: string;
  steps?: number;
  failed?: number;
  stillStale?: number;
  state?: 'running' | 'done' | 'failed';
  /** tools/audit's verdict, taken immediately after the apply: `clean` · `finding(s)` ·
   *  `scanner-missing` · `could not run`. A plain string rather than a union on purpose —
   *  a fifth verdict the tool grows must be shown, not dropped. Absent on a receipt written
   *  before the field existed. */
  audit?: string;
}

/** `sjel update --json`, passed through verbatim.
 *
 *  Served by sjel-status rather than by `tools/updates`, which is operator machinery with
 *  no server — the same arrangement `storage` above has, and for the same reason. A
 *  non-zero exit is not an error: `report` exits 1 whenever anything is stale, which is
 *  the answer the panel exists to show. */
export interface UpdatesReport {
  generatedAt: string;
  offline: boolean;
  lastApply: UpdateApply | null;
  surfaces: UpdateSurface[];
  rows: UpdateRow[];
}

/** One Pack skill (or the agents/ tree) as one harness currently holds it. */
export interface PackUnitView {
  pack: string;
  skill: string;
  /** current · outdated · drifted · missing · collision · invalid · not-deployed · migration-required */
  status: string;
  detail?: string;
}

/** A directory at a harness skill root that no Pack ledger claims. */
export interface PackStrayView {
  name: string;
  /** `copy` is a promote candidate; `external` and `symlink` are owned by another installer. */
  kind: "copy" | "external" | "symlink";
  detail?: string;
}

export interface HarnessView {
  id: string;
  label: string;
  installed: boolean;
  /** `materialized` copies and can drift; `registry` reads the Pack source in place. */
  model: "materialized" | "registry";
  marker: string;
  destination: string | null;
  cli: string;
  units: PackUnitView[];
  unowned: PackStrayView[];
}

export interface PacksView {
  measuredAt: string;
  harnesses: HarnessView[];
  unsupported: { id: string; label: string; why: string }[];
}

export const axonStatus = {
  health: () => request<AxonStatusHealth>('/sjel-status/api/sjel-status/health'),
  capabilities: () => request<CapabilityView[]>('/sjel-status/api/sjel-status/capabilities'),
  backups: () =>
    request<{ backups: BackupStatus[]; attempt_error: string | null }>(
      '/sjel-status/api/sjel-status/backups',
    ),
  /** Accepts the run and returns — it does not wait for it. Poll `backups()` for the
   *  outcome, which is also what lets a slow run survive a page refresh. */
  backup: (name: string, target?: string) =>
    request<{ name: string; accepted: boolean; target?: string; run_id?: number; holds_service: boolean }>(
      `/sjel-status/api/sjel-status/capabilities/${encodeURIComponent(name)}/backup`,
      {
        method: 'POST',
        ...(target
          ? {
              body: JSON.stringify({ target }),
              headers: { 'Content-Type': 'application/json' },
            }
          : {}),
      },
    ),
  backupTargets: () =>
    request<{ targets: BackupTargetView[] }>('/sjel-status/api/sjel-status/backup/targets'),
  backupRuns: (limit = 50) =>
    request<{ runs: BackupRunAttempt[]; running: number }>(
      `/sjel-status/api/sjel-status/backup/runs?limit=${limit}`,
    ),
  setBackupPolicy: (target: string, interval_hours: number | null) =>
    request<{ target: BackupTargetView | null }>('/sjel-status/api/sjel-status/backup/policy', {
      method: 'POST',
      body: JSON.stringify({ target, interval_hours }),
      headers: { 'Content-Type': 'application/json' },
    }),
  verifyBackupTarget: (target: string) =>
    request<{ target: string; verdict: string; detail: string }>(
      '/sjel-status/api/sjel-status/backup/verify',
      {
        method: 'POST',
        body: JSON.stringify({ target }),
        headers: { 'Content-Type': 'application/json' },
      },
    ),
  self: () => request<SelfModelResponse>('/sjel-status/api/sjel-status/self'),
  repos: () => request<{ repos: RepoStatus[] }>('/sjel-status/api/sjel-status/repos'),
  links: () => request<{ links: PinnedLink[] }>('/sjel-status/api/sjel-status/links'),
  /** Open findings from the hourly host watch. Served here rather than by host-watch
   *  itself because that capability is a scheduled job with no port: it runs, writes to
   *  its own table, and exits. Same shape as `backups()`, which publishes a job's
   *  receipts for the same reason. */
  hostWatch: (signal?: AbortSignal) =>
    request<{ findings: HostWatchFinding[] }>(
      '/sjel-status/api/sjel-status/host-watch',
      signal ? { signal } : undefined,
    ).then((response) => response.findings),
  /** What fills the disk, from the tool's own `report --json`. Served here because
   *  `tools/storage` is operator machinery with no server — it measures, prints and exits
   *  — for the same reason `hostWatch()` above is. A non-zero exit is not an error: it is
   *  how `report` says free space is below the policy's critical threshold. */
  storage: (signal?: AbortSignal) =>
    request<StorageReport>('/sjel-status/api/sjel-status/storage', signal ? { signal } : undefined),
  /** Software installed outside this checkout, from the tool's own `report --json`.
   *
   *  A non-zero exit is not an error here either: `report` exits 1 whenever anything is
   *  stale, which is exactly the answer this call exists to fetch. */
  updates: (signal?: AbortSignal) =>
    request<UpdatesReport>('/sjel-status/api/sjel-status/updates', signal ? { signal } : undefined),
  /** Start one class of software moving.
   *
   *  `202` means STARTED, not finished: the handler spawns the tool detached because a
   *  cargo step compiles for minutes, and the outcome arrives on the next `updates()` call
   *  as `lastApply`. Polling the report is the whole progress mechanism — there is no
   *  second endpoint and no state held in sjel-status. */
  updatesApply: (className: string) =>
    request<{ started: boolean; class: string }>('/sjel-status/api/sjel-status/updates/apply', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ class: className }),
    }),
  /** The agent's reach (ISA F10): each capability's mode, writes waiting, latest calls. */
  agent: (signal?: AbortSignal) =>
    request<AgentView>('/sjel-status/api/sjel-status/agent', signal ? { signal } : undefined),
  setAgentMode: (capability: string, mode: AgentMode) =>
    request<{ capability: string; mode: AgentMode }>('/sjel-status/api/sjel-status/agent/mode', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ capability, mode }),
    }),
  decideAgentWrite: (id: string, allow: boolean) =>
    request<AgentApproval>(`/sjel-status/api/sjel-status/agent/approvals/${encodeURIComponent(id)}`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ allow }),
    }),
  /** Every Pack skill against every agent harness. Served here rather than by `packs`
   *  itself because that capability is `kind = "data"`: it owns the deployment ledgers and
   *  nothing starts, so it has no port. Same reason as `hostWatch()` above. */
  packs: (signal?: AbortSignal) =>
    request<PacksView>('/sjel-status/api/sjel-status/packs', signal ? { signal } : undefined),
  /** Append a `watch` row to upstreams.toml. A 400 carries the tool's one-line refusal. */
  watchUpstream: (url: string, summary: string, name?: string) =>
    request<{ name: string; url: string; verdict: string; license: string; summary: string }>(
      '/sjel-status/api/sjel-status/upstreams/watch',
      {
        method: 'POST',
        body: JSON.stringify({ url, summary, ...(name ? { name } : {}) }),
        headers: { 'Content-Type': 'application/json' },
      },
    ),
  start: (name: string, signal?: AbortSignal) =>
    request<{ name: string; up: boolean; detail: string }>(
      `/sjel-status/api/sjel-status/capabilities/${encodeURIComponent(name)}/start`,
      { method: 'POST', signal },
    ),
  stop: (name: string) =>
    request<{ name: string; up: boolean; detail: string }>(
      `/sjel-status/api/sjel-status/capabilities/${encodeURIComponent(name)}/stop`,
      { method: 'POST' },
    ),
};

/** One graphify node inside a unit: a file or a symbol the extractor found. */
export interface UnitGraphNode {
  id: string;
  label: string;
  source_file: string;
  file_type: string;
  community: number;
}

/**
 * One unit's slice of the graphify graph, capped by the server.
 *
 * `total` and `truncated` are the honest part of the contract: the full graph
 * is thousands of nodes, a drill-down returns the busiest few hundred, and a
 * reader has to be able to tell a complete unit from a sampled one.
 */
export interface UnitGraph {
  unit: string;
  prefixes: string[];
  total: number;
  returned: number;
  truncated: boolean;
  cap: number;
  nodes: UnitGraphNode[];
  edges: Array<{ from: string; to: string; label?: string }>;
}

export const knowledgeGraph = {
  unit: (name: string) =>
    request<UnitGraph>(`/knowledge-graph/api/graph/unit/${encodeURIComponent(name)}`),
};

/** One sample from macmon pipe/serve. Every field present on Apple Silicon. */
export interface MacmonSample {
  all_power: number;
  ane_power: number;
  cpu_power: number;
  cpu_usage_pct: number;
  ecpu_usage: [number, number];  // [frequency_mhz, utilisation_0_1]
  gpu_power: number;
  gpu_ram_power: number;
  gpu_usage: [number, number];   // [frequency_mhz, utilisation_0_1]
  memory: {
    ram_total: number;
    ram_usage: number;
    swap_total: number;
    swap_usage: number;
  };
  pcpu_usage: [number, number];  // [frequency_mhz, utilisation_0_1]
  ram_power: number;
  sys_power: number;
  temp: {
    cpu_temp_avg: number;
    gpu_temp_avg: number;
  };
  timestamp: string;
}

// macmon — sudoless Apple Silicon performance monitor. Not an Sjel capability.
// Proxied at /macmon → http://localhost:9911 (see vite.config.ts).
export const macmon = {
  json: () => request<MacmonSample>('/macmon/json'),
};

// ─── Comms feed ──────────────────────────────────────────────────────────────

export type FeedStream = 'news' | 'media';
export type FeedKind =
  | 'youtube'
  | 'instagram'
  | 'podcast'
  | 'article'
  | 'mail'
  | 'github'
  | 'arxiv'
  | 'reddit';
export type FeedStatus = 'new' | 'keeper' | 'dismissed';

export interface FeedRelevance {
  profile_key: string;
  profile_label: string;
  score: number;
  rationale: string;
  mode: 'reranked' | 'semantic' | 'lexical';
  profile_revision: string;
}

export interface FeedEvaluationFactor {
  key: string;
  label: string;
  score: number;
  weight: number;
  rationale: string;
  context: {
    kind: 'trip' | string;
    id: string;
    label: string;
    date_start: string | null;
    date_end: string | null;
    matched_terms: string[];
  } | null;
}

export interface FeedEvaluation {
  overall_score: number;
  explanation: string;
  mode: 'reranked' | 'semantic' | 'lexical' | 'unscored';
  item_revision: string;
  context_revision: string;
  evaluator_revision: string;
  evaluated_at: string;
  factors: FeedEvaluationFactor[];
}

export interface CommsEvaluationStatus {
  evaluator_revision: string;
  context_revision: string;
  ledger: {
    evaluated: number;
    reranked: number;
    semantic: number;
    lexical: number;
    unscored: number;
  };
  summarizer: {
    // The unattended rung: the small local model every drain runs on. Named
    // here rather than the strong one because that is what an empty feed is a
    // question about.
    provider: string;
    model: string;
    configured: boolean;
    reachable: boolean;
    // The rung only an explicit press engages. Declared even though no
    // component reads it yet: a field on the wire that the interface does not
    // know about is how five provider fields went missing before (#C11).
    strong: {
      provider: string;
      model: string;
      configured: boolean;
      reachable: boolean;
    };
    // The durable half of the capacity alert, on the wire since the drains
    // started counting streaks and undeclared here until now.
    capacity: {
      alert_after: number;
      consecutive_aborts: number;
      alerting: boolean;
      last_abort_at: string | null;
      last_success_at: string | null;
    };
  };
  relevance: {
    provider: string;
    model: string;
    configured: boolean;
    reachable: boolean;
    profile_count: number;
    active_mode: 'reranked' | 'semantic' | 'lexical';
  };
  reranker: {
    provider: string;
    model: string;
    configured: boolean;
    reachable: boolean;
  };
  travel_context: {
    enabled: boolean;
    source: string;
    upcoming_count: number;
    reachable: boolean;
    from_cache: boolean;
    refreshed_at: string;
    plans: Array<{
      id: string;
      label: string;
      date_start: string;
      date_end: string;
    }>;
  };
}

export interface FeedOrigin {
  source_id: string;
  source_ref: string;
  label: string | null;
}

export interface FeedStageProvenance {
  stage: "extraction" | "normalization" | "summary" | "ranking";
  tier: "legacy" | "deterministic" | "model" | "human";
  revision: string;
  completed_at: string;
}

export interface FeedQualityFlag {
  feed_id: string;
  title: string | null;
  url: string;
  status: FeedStatus;
  content_status: "full" | "thin" | "none" | "unknown";
  signal:
    | "content_status"
    | "extraction_path"
    | "retention"
    | "boilerplate_leakage"
    | "summary_attempts"
    | "ranking_basis"
    | string;
  reason: string;
  evidence: string;
  derived_at: string;
}

export interface FeedQualityRefresh {
  reviewed: number;
  flagged_items: number;
  flag_count: number;
  bounded_to: number;
  days: number;
  provider_calls: 0;
}

interface FeedEntryBase {
  id: string;
  stream: FeedStream;
  kind: FeedKind;
  title: string | null;
  url: string;
  author: string | null;
  summary: string | null;
  /** The opening of this item's digest, sent only when it has no summary of its own.
   *
   *  Separate from `summary` because the two have different producers: the enrichment drain
   *  writes summaries on the light rung and has no cloud door, so an item past the 4,096-token
   *  window gets a digest and never a summary. Collapsing them server-side would make the card
   *  claim a summary nothing produced. */
  digest_preview: string | null;
  day: string; // YYYY-MM-DD grouping key
  created_at: string;
  status: FeedStatus;
}

export interface FeedEntry extends FeedEntryBase {
  /** What this item is worth protecting, as `GET /comms/feed` states it since 2026-09-06.
   *  An unclassified row answers `c1`, the undeclared default, so the LIST never omits it.
   *
   *  Optional all the same, and the `?` is the contract gap rather than caution: comms'
   *  `FeedFullItem` -- what `POST /ingest` and `GET /feed/:id` answer with -- carries no
   *  class, so `toListEntry` in `routes/feed/+page.svelte` builds a list row from a detail
   *  that has none. A reader must therefore treat `undefined` as "not stated" and fail
   *  closed, the way finance's `item_is_quotable` already does. It stops being optional the
   *  day the detail contract states one too.
   *
   *  On `FeedEntry` and not on `FeedEntryBase` for the same reason, and because the shared
   *  reader (`ContentItemDetail`) carries the richer `ContentDataClass` shape under this
   *  exact name -- one name for two shapes is how a component comes to read the wrong one. */
  data_class?: DataClass;
  relevance: FeedRelevance | null;
  evaluation: FeedEvaluation | null;
}

export interface FeedEntryDetail extends FeedEntryBase {
  transcript: string | null;
  relevance: FeedRelevance[];
  evaluation: FeedEvaluation | null;
  content_status: "full" | "thin" | "none" | "unknown";
  /** Which client handed the content over; null when the server fetched it. */
  captured_via: string | null;
  processing: FeedStageProvenance[];
  origins: FeedOrigin[];
}

export type ContentSource = 'feed' | 'mail' | 'calendar';

/**
 * Which capability owns each content source.
 *
 * The contract is shared; the data is not. A source is served by the capability
 * that stores it, and every one of them exposes the same `/content/:source/:id`
 * shape — so the reader resolves an item from its source alone and no caller
 * carries a per-capability special case. Adding a source is one line here.
 */
const CONTENT_BASE: Record<ContentSource, string> = {
  feed: '/comms',
  mail: '/comms',
  calendar: '/calendar/api',
};

/** Any content item, from whichever capability owns that source. */
export function contentItem(source: ContentSource, id: string, signal?: AbortSignal) {
  return request<ContentItemDetail>(
    `${CONTENT_BASE[source]}/content/${source}/${encodeURIComponent(id)}`,
    signal ? { signal } : undefined,
  );
}
export type DataClass = 'c0' | 'c1' | 'c2' | 'c3';

/** Who decided, ascending. `legacy` means nobody did — a row from before its
 * table had a class, or an item no collector declared anything about. */
export type ClassificationMethod = 'legacy' | 'deterministic' | 'model' | 'human';

export interface ContentDataClass {
  value: DataClass;
  label: 'Public' | 'Mine' | 'Others' | 'Secret';
  rationale: string;
  method: ClassificationMethod;
  version: string;
}

export interface ContentProcessingPolicy {
  local_processing: 'allowed' | 'blocked';
  cloud_handling: 'eligible' | 'pseudonymization_required' | 'blocked';
  pseudonymization_required: boolean;
  rationale: string;
}

export interface CloudProcessingState {
  status: 'not_prepared' | 'staged' | 'stale';
  preview_hash: string | null;
  approved_at: string | null;
  dispatch_status: 'not_queued' | 'queued' | 'running' | 'succeeded' | 'failed';
  job_id: string | null;
  provider_role: string | null;
  queued_at: string | null;
  provider_calls: number;
  task: 'content-analysis-v1' | null;
  started_at: string | null;
  completed_at: string | null;
  last_error: string | null;
  result: CloudContentAnalysis | null;
}

export interface CloudContentAnalysis {
  schema_version: 'cloud-content-analysis-v1';
  summary: string;
  importance: 'low' | 'medium' | 'high';
  importance_rationale: string;
  important_dates: Array<{ label: string; date: string | null; source_text: string }>;
  action_items: Array<{ text: string; due_date: string | null }>;
  topics: string[];
}

/// Why a configured cloud role cannot be picked right now. Kept as a union
/// rather than a bare string so a new server reason fails the build here
/// instead of rendering as an unlabelled token in the roster.
export type CloudProviderUnavailableReason =
  | 'missing_credential'
  | 'billing_expired_or_unknown'
  | 'budget_unavailable'
  | 'daily_request_limit_reached';

/// Every field `CloudProviderOut` serializes, in the order the server writes
/// them. The interface used to declare eight of the fifteen, so the budget,
/// the context ceiling, the credit expiry and the reason a role was refused
/// existed on the wire and nowhere in the UI.
export interface CloudProvider {
  role: string;
  name: string;
  model: string;
  provider_label: string;
  location: 'cloud';
  data_tier: 'public' | 'pseudonymized_personal';
  billing_mode: 'free_only' | 'prepaid_credit';
  failover_priority: number;
  max_requests_per_day: number;
  /// Null when the call ledger could not be read; that is a distinct state
  /// from zero calls made, and `budget_unavailable` is the reason it carries.
  requests_used_today: number | null;
  requests_remaining_today: number | null;
  max_input_tokens: number;
  credit_expires_on: string | null;
  available: boolean;
  unavailable_reason: CloudProviderUnavailableReason | null;
}

export interface RedactionFinding {
  entity_type: string;
  marker: string;
  count: number;
}

export interface CloudDerivativePreview {
  schema_version: 'cloud-derivative-preview-v1';
  source: ContentSource;
  id: string;
  source_revision: string;
  preview_hash: string;
  original_data_class: DataClass;
  derivative_data_class: 'c0' | 'c1';
  transformation: 'bounded-public-v1' | 'deterministic-entity-redaction-v3';
  document: string;
  redaction_count: number;
  redactions: RedactionFinding[];
  /** PRD Q9b's receipt: the sentence a human reads on a call that was reduced,
   *  or null when nothing was removed. Composed by comms, not here, so every
   *  surface says the same thing about the same call. */
  redaction_receipt: string | null;
  entity_detection: 'not-required' | 'local-deterministic-v3';
  truncated: boolean;
  approval_required: true;
  provider_calls: 0;
  limitations: string[];
}

export interface MailContentExtension {
  category: MailCategory;
  rationale: string;
  classification_method: ClassificationMethod;
  classification_version: string;
  gmail_action: 'archive' | 'trash' | 'restore' | null;
  gmail_action_at: string | null;
  purge_after: string | null;
  gmail_location: 'inbox' | 'archive' | 'trash' | 'missing' | null;
  gmail_observed_at: string | null;
  gmail_sync_status: 'synced' | 'queued' | 'retrying' | 'attention' | null;
  gmail_sync_action: 'archive' | 'trash' | 'restore' | null;
  gmail_sync_error: string | null;
  /** The doctrine's one state label, mirrored from Gmail. Separate from `status`
   *  on purpose: status is what Sjel decided about a proposal, waiting is what you
   *  decided about the conversation. */
  waiting: boolean;
  waiting_since: string | null;
}

/** Versioned reader contract shared by Feed sources and mail proposals. */
export interface ContentItemDetail {
  schema_version: 'content-item-v2';
  source: ContentSource;
  id: string;
  /** The source's own type discriminator, not a shared enum — a feed article is
   *  an `article`, a calendar entry is a `nightlife` or `work_onsite`. Open
   *  string, exactly as the schema types it: a source may add a kind without a
   *  dashboard release, and every reader already falls back for unknown ones. */
  kind: string;
  title: string | null;
  url: string;
  author: string | null;
  summary: string | null;
  content: string | null;
  content_label: string;
  day: string;
  created_at: string;
  /** Each source's triage axis, in one field: feed keeps or dismisses, mail
   *  moves through Gmail states, calendar commits. */
  status: FeedStatus | TriageStatus | CalendarCommitment;
  content_status: 'full' | 'thin' | 'none' | 'unknown';
  data_class: ContentDataClass;
  processing_policy: ContentProcessingPolicy;
  cloud_processing: CloudProcessingState;
  relevance: FeedRelevance[];
  evaluation: FeedEvaluation | null;
  processing: FeedStageProvenance[];
  origins: FeedOrigin[];
  links: ContentLink[];
  /** What the local model wrote about this item. Deliberately not `summary`,
   *  which is what the *source* said it is — calendar reads that from the
   *  entry's own description, and a generated paragraph written over it would
   *  destroy the only verbatim text an entry has. A reader wanting the short
   *  version prefers `digest.text` and falls back to `summary`. */
  digest: ContentDigest | null;
  mail: MailContentExtension | null;
  calendar: CalendarContentExtension | null;
}

/** Why there is no digest text. `skipped_short` is a verdict about the source —
 *  it is already shorter than any honest digest of it — not a failure. */
export type DigestState =
  | 'generated'
  | 'skipped_short'
  | 'remote_refused'
  /** The class enters no prompt at all — `c3`, or a class outside the
   *  vocabulary. Stricter than `remote_refused`, which is about where the
   *  endpoint is: no model was asked, local included. */
  | 'local_refused'
  | 'unconfigured'
  | 'http_error'
  | 'model_error'
  /** The server took the request and then ran out of room for it. A fact about
   *  the machine rather than the request, so it is worth retrying. */
  | 'capacity_aborted'
  | 'empty_response'
  | 'timeout';

/** The rung the length ladder landed on. Derived from `source_chars`, never
 *  chosen directly — see libs/summarize/README.md. */
export type DigestShape = 'none' | 'brief' | 'standard' | 'sectioned';

/** `detailed` moves the shape exactly one rung up that same ladder. It is not a
 *  separate instruction to the model, which is why it stays inspectable. */
export type DigestDepth = 'standard' | 'detailed';

export interface ContentDigest {
  text: string | null;
  state: DigestState;
  shape: DigestShape;
  depth: DigestDepth;
  /** The operator's focus terms, as typed. Shown back so a differently-shaped
   *  digest is explained rather than mysterious. */
  focus: string[];
  producer: string;
  source_chars: number;
  /** Entities the deterministic redactor removed before this text was stored.
   *  Non-zero only for Private content. */
  redactions: number;
  attempts: number;
  last_error: string | null;
  /** Mermaid source, validated against the known diagram headers before it was
   *  stored — a string the renderer cannot draw never gets here. */
  diagram: string | null;
  diagram_state: string | null;
  diagram_error: string | null;
  /** The table pulled out of the source, or null. Deliberately data rather than
   *  a chart specification: the reader compiles one, so a model never reaches
   *  the rendering layer. Every value appeared verbatim in the source text
   *  before it was admitted. */
  chart: ContentChartData | null;
  /** `generated`, or `skipped_short` when the source holds no comparable
   *  numbers — the answer for most prose, and not a failure. */
  chart_state: string | null;
  chart_error: string | null;
  generated_at: string;
}

/** One measure over a handful of categories. One series is the maximum by
 *  design: the figure palette is a low-chroma print palette that cannot carry
 *  categorical identity, so a chart drawn from it must not need to. */
export interface ContentChartData {
  title: string;
  category_label: string;
  measure_label: string;
  unit: string | null;
  /** Derived from the categories, never chosen by a model: an ordered run of
   *  three or more gets a line, everything else bars. */
  mark: "bar" | "line";
  note: string;
  rows: { category: string; value: number }[];
}

/** A named way out of an item: source page, the mail that carried the ticket,
 *  a map, a vault note. `kind` is a presentation hint and stays open — an
 *  unknown one still renders a working link. */
export interface ContentLink {
  label: string;
  kind: string;
  url: string;
}

/** What only a calendar entry has. No score on purpose: a decided item is not
 *  ranked, so `commitment` is its triage axis. */
export interface CalendarContentExtension {
  starts_at: string;
  /** Exclusive, as everywhere in calendar. */
  ends_at: string;
  all_day: boolean;
  commitment: 'possible' | 'planned' | 'committed';
  location: string | null;
  /** The operator's own note — why they care, not what the thing is. */
  notes: string | null;
  /** Which adapter contributed the row (`manual`, `luma`, `google`) — not the
   *  item's `source`, which is always `calendar`. Decides which actions are
   *  honest: an entry imported from Google must not offer to export back. */
  entry_source: string;
  /** Set when materialized from a rhythm. Such an instance is not exported
   *  individually, and any patch detaches it from its rhythm. */
  rhythm_id: string | null;
}

// One item's place in a collector run. Derived server-side per request, so a
// run never becomes stale state on the entry itself.
export interface FeedRun {
  feed_id: string;
  source_id: string;
  label: string | null;
  run_key: string;
  run_started: string | null;
}

export interface VaultLinkCandidate {
  id: string;
  source_id: string;
  source_ref: string;
  label: string | null;
  url: string;
  imported: boolean;
}

export interface FeedSource {
  id: string;
  adapter: 'github-trending' | 'arxiv' | string;
  enabled: boolean;
  source_url: string;
  query_configured: boolean;
  limit: number;
  last_run_at: string | null;
}

export interface FeedSourceScan {
  fetched: number;
  new_count: number;
  sources: Array<{
    source_id: string;
    adapter: string;
    fetched: number;
    new_count: number;
    known_count: number;
  }>;
}

export type MailCategory =
  | 'aktiv'
  | 'issue'
  | 'feed'
  | 'werbung'
  | 'belege'
  | 'steuern'
  | 'sonstiges';

export type TriageStatus =
  | 'proposed'
  | 'approved'
  | 'executed'
  | 'archived'
  | 'trashed'
  | 'missing'
  | 'dismissed';

export interface TriageItem {
  id: string;
  from_addr: string | null;
  subject: string | null;
  snippet: string | null;
  stream: MailCategory;
  rationale: string;
  classification_method: ClassificationMethod;
  classification_version: string;
  data_class: DataClass;
  data_class_rationale: string;
  data_classification_method: ClassificationMethod;
  data_classification_version: string;
  status: TriageStatus;
  gmail_action: 'archive' | 'trash' | 'restore' | null;
  gmail_action_at: string | null;
  purge_after: string | null;
  gmail_location: 'inbox' | 'archive' | 'trash' | 'missing' | null;
  gmail_observed_at: string | null;
  gmail_sync_status: 'synced' | 'queued' | 'retrying' | 'attention' | null;
  gmail_sync_action: 'archive' | 'trash' | 'restore' | null;
  gmail_sync_error: string | null;
  /** The doctrine's one state label, mirrored from Gmail. Separate from `status`
   *  on purpose: status is what Sjel decided about a proposal, waiting is what you
   *  decided about the conversation. */
  waiting: boolean;
  waiting_since: string | null;
  internal_date: string | null;
  relevance: FeedRelevance[];
  /** The local model rung's verdict, when a shadow pass has stored one. Null for
   *  a thread the deterministic rules decided, and for every thread until a pass
   *  has run. Declared here rather than in `lib/mail/api.ts` because TriageItem
   *  is the reader contract for GET /triage and this file owns it; the client
   *  FUNCTIONS all live in the stream's own module. */
  model?: TriageModelVerdict | null;
}

/** One stored verdict from the local mail classification rung.
 *
 *  `urgency_validated` is false until the frozen corpus carries a measured
 *  urgency band error. While it is false, urgency is shown and ranks nothing:
 *  a number that would reorder the ladder passes the same door the stream does. */
export interface TriageModelVerdict {
  mode: 'shadow' | 'applied' | 'held';
  /** `generated` when the model answered. `local_refused` when the class refuses
   *  every prompt, `skipped_over_window` when the source is too long for the
   *  light local model, and the transport states otherwise. */
  state: string;
  rule_stream: MailCategory;
  model_stream: MailCategory | null;
  confidence_bp: number | null;
  urgency_bp: number | null;
  urgency_validated: boolean;
  rationale: string | null;
  urgency_rationale: string | null;
  data_class: DataClass;
  /** Set when apply refused this proposal because it would raise the data class.
   *  Names the class it would raise to. */
  held_reason: string | null;
  classification_version: string;
  applied_at: string | null;
}

export interface TriageSweepResult {
  fetched: number;
  new_count: number;
  skipped: number;
  /** Threads whose subject or snippet was redacted before being stored. */
  redacted: number;
  events: { considered: number; proposals: number; no_event: number; refused: number; failed: number };
  total_stored: number;
  next_cursor: string | null;
  exhausted: boolean;
}

/** Freshness of the unattended inbox sweep. Counts, times and an error class;
 *  no mail reaches this shape. `last_success_at` is deliberately separate from
 *  `last_run_at` — a failing run still ran, and "when did collection last
 *  actually work" is the question a red schedule raises. */
export interface TriageSweepStatus {
  enabled: boolean;
  every_minutes: number;
  max_threads: number;
  quiet_hours: { start: number; end: number } | null;
  last_run_at: string | null;
  last_success_at: string | null;
  last_failure_at: string | null;
  last_error: 'auth' | 'quota' | 'network' | 'unknown' | null;
  considered_count: number;
  new_count: number;
  consecutive_failures: number;
}

/** What a redaction pass removed, by kind and count — never the values. */
export interface TriageRedactResult {
  reviewed: number;
  in_scope: number;
  changed: number;
  dry_run: boolean;
  entity_types: Record<string, number>;
  audit: { id: string; digest: string }[];
  transformation: string;
  provider_calls: number;
}

export interface TriageRelevanceResult {
  scored: number;
  profile_count: number;
  mode: 'reranked' | 'semantic' | 'lexical' | null;
  local_only: true;
}

export interface TriageDataClassRefreshResult {
  reviewed: number;
  updated: number;
  preserved_human: number;
  classifier_version: string;
  content_inputs: ['sender', 'subject', 'category'];
  provider_calls: 0;
}

export interface TriageBulkResult {
  succeeded: string[];
  failures: Array<{ id: string; error: string }>;
  gmail_changed: boolean;
  /** Rows whose stored subject and snippet this batch permanently redacted,
   *  because the category it set on them raises the data class. Zero for every
   *  action except `categorize` into `belege` or `steuern`. */
  narrowed: number;
}

export interface GmailMaintenanceResult {
  retried: number;
  recovered: number;
  retry_failures: number;
  reconciled: number;
  changed: number;
  read_failures: number;
  missing: number;
  content_fetched: false;
}

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
  /** What this entry is worth protecting, as the three entry LISTS state it since
   *  2026-09-08: `/api/entries`, `/api/proposals` and `/api/google/drafts`.
   *
   *  Calendar declares one class for the whole source rather than one per row —
   *  `capabilities/calendar/src/content.rs`, `classification()`: "where the operator is
   *  and when is personal, whatever the event itself is". So this is c1 on every row a
   *  list serves, and the `?` is the contract gap, not caution: `GET /api/entries/:id`,
   *  the create and the patch answer with the bare row and state no class. A reader
   *  treats `undefined` as "not stated", which is not the same claim as any of the four. */
  data_class?: DataClass;
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

export const comms = {
  feed: (opts: { stream?: FeedStream; days?: number; includeDismissed?: boolean } = {}) => {
    const params = new URLSearchParams();
    if (opts.stream) params.set('stream', opts.stream);
    if (opts.days != null) params.set('days', String(opts.days));
    if (opts.includeDismissed) params.set('include_dismissed', 'true');
    const qs = params.toString();
    return request<FeedEntry[]>(`/comms/feed${qs ? `?${qs}` : ''}`);
  },
  entry: (id: string, signal?: AbortSignal) =>
    request<FeedEntryDetail>(
      `/comms/feed/${encodeURIComponent(id)}`,
      signal ? { signal } : undefined,
    ),
  /** Kept as the comms-shaped alias so existing comms callers read naturally;
   *  the routing itself lives in `contentItem` so there is one map, not two. */
  content: (source: ContentSource, id: string, signal?: AbortSignal) =>
    contentItem(source, id, signal),
  /** Generate or refine an item's digest. Synchronous on the server: this is a
   *  button the operator is watching, and answering early would show them the
   *  previous digest. */
  digest: (
    source: ContentSource,
    id: string,
    body: { depth?: DigestDepth; focus?: string[] } = {},
  ) =>
    request<ContentDigest>(
      `/comms/content/${source}/${encodeURIComponent(id)}/digest`,
      jsonInit('POST', body),
    ),
  chart: (source: ContentSource, id: string) =>
    request<ContentDigest>(
      `/comms/content/${source}/${encodeURIComponent(id)}/chart`,
      { method: 'POST' },
    ),
  diagram: (source: ContentSource, id: string) =>
    request<ContentDigest>(
      `/comms/content/${source}/${encodeURIComponent(id)}/diagram`,
      { method: 'POST' },
    ),
  refreshDigests: (source: ContentSource, limit?: number) =>
    request<{ source: string; digested: number }>(
      '/comms/content/digests/refresh',
      jsonInit('POST', limit === undefined ? { source } : { source, limit }),
    ),
  prepareCloudPreview: (source: ContentSource, id: string) =>
    request<CloudDerivativePreview>(
      `/comms/content/${source}/${encodeURIComponent(id)}/cloud-preview`,
      { method: 'POST' },
    ),
  approveCloudPreview: (source: ContentSource, id: string, preview_hash: string) =>
    request<CloudProcessingState>(
      `/comms/content/${source}/${encodeURIComponent(id)}/cloud-approval`,
      jsonInit('POST', { preview_hash }),
    ),
  cloudProviders: () => request<CloudProvider[]>('/comms/content/cloud-providers'),
  queueCloudDerivative: (
    source: ContentSource,
    id: string,
    preview_hash: string,
    provider_role: string,
  ) =>
    request<CloudProcessingState>(
      `/comms/content/${source}/${encodeURIComponent(id)}/cloud-queue`,
      jsonInit('POST', { preview_hash, provider_role }),
    ),
  runCloudJob: (jobId: string) =>
    request<CloudProcessingState>(
      `/comms/content/cloud-jobs/${encodeURIComponent(jobId)}/run`,
      jsonInit('POST', {}),
    ),
  runs: (days = 7) => request<FeedRun[]>(`/comms/feed/runs?days=${days}`),
  evaluationStatus: () =>
    request<CommsEvaluationStatus>('/comms/feed/evaluation/status'),
  qualityFlags: (limit = 500) =>
    request<FeedQualityFlag[]>(`/comms/feed/quality?limit=${limit}`),
  refreshQualityFlags: (days = 3650) =>
    request<FeedQualityRefresh>(
      '/comms/feed/quality/refresh',
      jsonInit('POST', { days }),
    ),
  // Answers once the item is stored; the summary is written behind the response, so a
  // freshly ingested entry legitimately comes back with `summary: null`.
  ingest: (url: string) => request<FeedEntryDetail>('/comms/ingest', jsonInit('POST', { url })),
  refreshRelevance: (days = 90) =>
    request<{
      scored: number;
      evaluated: number;
      considered: number;
      skipped_current: number;
      profile_count: number;
      mode: 'reranked' | 'semantic' | 'lexical' | null;
      evaluator_revision: string;
      travel_context: {
        upcoming_count: number;
        reachable: boolean;
        from_cache: boolean;
        refreshed_at: string;
      };
    }>(
      '/comms/feed/relevance/refresh',
      jsonInit('POST', { days }),
    ),
  scanVaultLinks: () =>
    request<VaultLinkCandidate[]>('/comms/vault-links/scan', { method: 'POST' }),
  sources: () => request<{ sources: FeedSource[] }>('/comms/sources'),
  scanSources: (source_id?: string) =>
    request<FeedSourceScan>(
      '/comms/sources/scan',
      jsonInit('POST', { source_id: source_id ?? null }),
    ),
  importVaultLink: (source_id: string, url: string) =>
    request<FeedEntryDetail>(
      '/comms/vault-links/import',
      jsonInit('POST', { source_id, url }),
    ),
  setStatus: (id: string, status: FeedStatus) =>
    request<void>(`/comms/feed/${encodeURIComponent(id)}/status`, jsonInit('POST', { status })),
  triage: (status?: TriageStatus) =>
    request<TriageItem[]>(
      `/comms/triage${status ? `?status=${encodeURIComponent(status)}` : ''}`,
    ),
  setTriageStatus: (id: string, status: 'proposed' | 'approved' | 'dismissed') =>
    request<void>(
      `/comms/triage/${encodeURIComponent(id)}/status`,
      jsonInit('POST', { status }),
    ),
  setTriageCategory: (id: string, stream: MailCategory) =>
    request<{ ok: boolean; narrowed: boolean }>(
      `/comms/triage/${encodeURIComponent(id)}/stream`,
      jsonInit('POST', { stream }),
    ),
  setTriageDataClass: (id: string, data_class: DataClass) =>
    request<void>(
      `/comms/triage/${encodeURIComponent(id)}/data-class`,
      jsonInit('POST', { data_class }),
    ),
  applyGmailAction: (id: string, action: 'archive' | 'trash' | 'restore') =>
    request<{ ok: true; action: 'archive' | 'trash' | 'restore'; gmail_changed: boolean; gmail_confirmed: true }>(
      `/comms/triage/${encodeURIComponent(id)}/gmail`,
      jsonInit('POST', { action }),
    ),
  decideGmailJob: (id: string, decision: 'retry' | 'cancel') =>
    request<{ ok: true; state: 'completed' | 'canceled'; gmail_changed?: boolean }>(
      `/comms/triage/${encodeURIComponent(id)}/gmail-job`,
      jsonInit('POST', { decision }),
    ),
  reconcileGmail: () =>
    request<GmailMaintenanceResult>('/comms/triage/reconcile', jsonInit('POST', {})),
  triageSweepStatus: (signal?: AbortSignal) =>
    request<TriageSweepStatus>('/comms/triage/sweep/status', signal ? { signal } : undefined),
  sweepTriage: (limit = 100, cursor?: string | null) =>
    request<TriageSweepResult>(
      '/comms/triage/sweep',
      jsonInit('POST', { limit, cursor: cursor ?? null }),
    ),
  refreshTriageRelevance: (limit = 200) =>
    request<TriageRelevanceResult>(
      '/comms/triage/relevance/refresh',
      jsonInit('POST', { limit }),
    ),
  refreshTriageDataClasses: (limit = 500) =>
    request<TriageDataClassRefreshResult>(
      '/comms/triage/data-class/refresh',
      jsonInit('POST', { limit }),
    ),
  /** Remediate rows stored before the sweep redacted Private mail in place.
   *  Idempotent: a second run reports `changed: 0`. */
  redactTriage: (limit = 500, dryRun = false) =>
    request<TriageRedactResult>(
      '/comms/triage/redact',
      jsonInit('POST', { limit, dry_run: dryRun }),
    ),
  bulkTriage: (
    ids: string[],
    action:
      | 'dismiss'
      | 'categorize'
      | 'set-data-class'
      | 'archive'
      | 'trash'
      // The doctrine's one state label. It only labels — it does not archive,
      // and archiving does not set it.
      | 'waiting'
      | 'clear-waiting',
    stream?: MailCategory,
    data_class?: DataClass,
  ) =>
    request<TriageBulkResult>(
      '/comms/triage/bulk',
      jsonInit('POST', { ids, action, stream: stream ?? null, data_class: data_class ?? null }),
    ),
};

/** One action, read out of a note under the vault's `Projects/**\/Tasks/`.
 *
 *  This replaced a database row on 2026-08-27 (PRD Q48): the `tasks`
 *  capability retired and the Action kind went back to the vault, which is
 *  where the vault contract §5.1b had assigned it all along.
 *
 *  Every field here has a reader below — `due` and `priority` rank the row,
 *  `title` and `summary` render it, `projects` labels it, `done` decides
 *  whether it is a decision at all. The note carries six more frontmatter
 *  keys that nothing on this page reads, so the server does not serve them. */
export interface Task {
  /** Vault-relative path. The identity that survives a machine. */
  id: string;
  title: string;
  done: boolean;
  due: string | null;
  /** 1 high · 2 medium · 3 low. The server defaults a blank key to 2, the same
   *  fallback the vault's own Tasks.base applies. */
  priority: number;
  summary: string | null;
  /** Display names of the note's `projects:` links. */
  projects: string[];
  /** `obsidian://open?…` — where the operator goes to act on it. */
  uri: string;
  /** What the note is worth protecting, decided by `content-item`'s vault classifier:
   *  the folder sets it, the note's own `class:` key overrides it (PRD Q9a).
   *
   *  Not optional, unlike the feed's and the calendar's: `/api/tasks` is the only route
   *  the vault server serves, so there is no second path a task can arrive by without
   *  one. Never `c0` — publishing is an act, and no folder means "already public". */
  data_class: DataClass;
  /** Why that class, in the classifier's words. The one branch the value cannot show is a
   *  note whose `class:` key is not a class at all: it is refused rather than honoured or
   *  escalated, so it reads as its folder's default and only this sentence says so. */
  data_class_rationale: string;
}

export type TaskStatus = 'open' | 'done';

/** Read-only, and the whole namespace is one call.
 *
 *  There is no create and no patch because the vault server has no write
 *  route: a task is written, edited and marked done in Obsidian, in a note a
 *  human owns. Sjel reads the vault and does not write to it (PRD §5.5). */
export const vault = {
  tasks: (status?: TaskStatus, signal?: AbortSignal) =>
    request<{ tasks: Task[] }>(
      `/vault/api/tasks${status ? `?status=${status}` : ''}`,
      signal ? { signal } : undefined,
    ).then((response) => response.tasks),
};


// ─── Entities (PRD Q117) ─────────────────────────────────────────────────────

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

// ─── Finance ─────────────────────────────────────────────────────────────────

export type BillingCycle = 'weekly' | 'monthly' | 'quarterly' | 'yearly' | 'one_off';
export type SubscriptionState = 'considering' | 'trial' | 'active' | 'covered' | 'paused' | 'cancelled';

/** Append-only. A price change adds one of these; it never edits the one before. */
export interface PricePoint {
  valid_from: string;
  amount_cents: number;
  currency: string;
  cycle: BillingCycle;
  /** Which tier: "Pro", "Max", "2TB". Absent for a subscription with only one. */
  plan?: string | null;
  reason: string;
}

/** Append-only, same reasoning. `paused → active` is two rows, not an edit. */
export interface StateChange {
  effective: string;
  state: SubscriptionState;
  note: string;
}

export interface Subscription {
  id: string;
  name: string;
  source_path: string;
  category: string | null;
  value_rating: number | null;
  prices: PricePoint[];
  states: StateChange[];
}

/** Computed from the series at a date, never read from a stored total. */
export interface Burn {
  at: string;
  currencies: Array<{
    currency: string;
    monthly_cents: number;
    annual_cents: number;
  }>;
  billing_count: number;
  covered_count: number;
  unknown_price_count: number;
  total_count: number;
}

export interface WritebackResult {
  ok: boolean;
  written: number;
  unchanged: number;
  conflicts: string[];
  not_imported: string[];
  /** Subscriptions with no note in the vault, written as whole generated files
   *  under Resources/Sjel/ instead (PRD Q31). Since the 2026-08-23 vault
   *  reorganisation moved every finance note to the overlay, this is where the
   *  price and state series actually land. */
  projected: {
    created: number;
    updated: number;
    unchanged: number;
    refused: string[];
    removed: string[];
  };
}

export type FinanceTransactionKind = 'income' | 'expense' | 'transfer';
export type SpendingPurpose = 'day_to_day' | 'trip' | 'work' | 'housing' | 'other';
export type CandidateState = 'pending' | 'confirmed' | 'rejected' | 'duplicate';
export type CsvDateFormat =
  | 'iso_year_month_day'
  | 'day_month_year_dots'
  | 'day_month_year_slashes';

export interface TransactionCandidate {
  id: string;
  fingerprint: string;
  booked_at: string;
  description: string;
  amount_cents: number;
  currency: string;
  source_account: string;
  source_reference: string | null;
  proposed_account: string;
  confidence_basis_points: number;
  state: CandidateState;
  transfer_match_ids: string[];
}

export interface CsvMapping {
  delimiter: string;
  decimal_separator: string;
  date_column: string;
  amount_column: string;
  description_column: string;
  categorization_columns: string[];
  reference_column?: string | null;
  currency_column?: string | null;
  default_currency: string;
  source_account: string;
  default_outflow_account: string;
  default_inflow_account: string;
  categorization_rules: CsvCategorizationRule[];
  row_filter?: CsvRowFilter | null;
  amount_sign: 'as_provided' | 'invert';
  amount_rounding?: 'reject' | 'half_away_from_zero';
  date_formats: CsvDateFormat[];
  row_policy: 'strict' | 'required_fields';
}

export interface CsvCategorizationRule {
  description_contains_any: string[];
  description_starts_with_any: string[];
  field_equals_any?: CsvFieldEquals[];
  direction: 'any' | 'outflow' | 'inflow';
  account: string;
  confidence_basis_points: number;
}

export interface CsvFieldEquals {
  column: string;
  values: string[];
}

export interface CsvRowFilter {
  column: string;
  include_values: string[];
}

export interface CsvImportPreview {
  preview_id: string;
  candidate_count: number;
  duplicate_rows: number;
  preserved_repetitions: number;
  ignored_non_transaction_rows: number;
  outflow_count: number;
  inflow_count: number;
}

export interface CsvMappingProfile {
  label: string;
  mapping: CsvMapping;
}

export interface InvestmentCsvMapping {
  delimiter: string;
  decimal_separator: string;
  date_column: string;
  instrument_column: string;
  quantity_column: string;
  activity_type_column?: string | null;
  position_activity_values: string[];
  non_position_activity_values: string[];
  reference_column?: string | null;
  price_column?: string | null;
  currency_column?: string | null;
  default_currency: string;
  instrument_aliases: Record<string, string>;
}

export interface InvestmentCsvMappingProfile {
  source_key: string;
  label: string;
  coverage: HoldingsCoverage;
  mapping: InvestmentCsvMapping;
}

export type HoldingsCoverage = 'complete' | 'partial';

export interface InvestmentHolding {
  instrument: string;
  quantity: { mantissa: string; scale: number };
  latest_unit_price: { mantissa: string; scale: number } | null;
  currency: string;
}

export interface InvestmentPreview {
  snapshot_id: string;
  activity_count: number;
  duplicate_rows: number;
  ignored_non_position_rows: number;
  closed_positions: number;
  holdings: InvestmentHolding[];
}

export interface ReviewedHoldingsSnapshot {
  schema_version: number;
  snapshot_id: string;
  reviewed_at: string;
  coverage: HoldingsCoverage;
  holdings: InvestmentHolding[];
  sources: Array<{
    source_key: string;
    snapshot_id: string;
    reviewed_at: string;
    coverage: HoldingsCoverage;
  }>;
}

export interface PortfolioValuation {
  currency: string;
  value: { mantissa: string; scale: number };
  priced_holdings: number;
  unpriced_holdings: number;
}

export interface FinanceTransaction {
  id: string;
  date: string;
  description: string;
  kind: FinanceTransactionKind;
  account: string;
  category: string;
  amount_cents: number;
  currency: string;
  source_id: string | null;
  purpose: SpendingPurpose | null;
  trip_id: string | null;
  cash_amount_cents: number;
  shared_cents: number;
  reimbursement_for: string | null;
}

export interface SharedExpenseSummary {
  source_id: string;
  candidate_id: string;
  date: string;
  description: string;
  account: string;
  category: string;
  purpose: SpendingPurpose | null;
  trip_id: string | null;
  gross_cents: number;
  personal_cents: number;
  shared_cents: number;
  reimbursed_cents: number;
  outstanding_cents: number;
  currency: string;
}

/** Per-trip cost roll-up; `trip_id` is the trips plan id (tag `axon-trip-id`). */
export interface TripSpendingSummary {
  trip_id: string;
  personal_spending_cents: number;
  gross_cash_outflow_cents: number;
  reimbursed_cents: number;
  outstanding_cents: number;
  expense_posting_count: number;
}

export interface FinanceDashboard {
  summary: {
    income_cents: number;
    personal_spending_cents: number;
    gross_cash_outflow_cents: number;
    reimbursement_received_cents: number;
    personal_result_cents: number;
    external_cash_inflow_cents: number;
    external_cash_movement_cents: number;
    savings_rate_percent: number | null;
    currency: string;
  };
  quality: {
    expense_posting_count: number;
    categorized_expense_posting_count: number;
    personal_spending_cents: number;
    categorized_personal_spending_cents: number;
    categorization_count_percent: number | null;
    categorization_value_percent: number | null;
    first_transaction_date: string | null;
    latest_transaction_date: string | null;
    observed_months: number;
    expected_months: number;
  };
  trend: Array<{
    month: string;
    income_cents: number;
    personal_spending_cents: number;
    gross_cash_outflow_cents: number;
    reimbursement_received_cents: number;
    personal_result_cents: number;
    external_cash_inflow_cents: number;
    external_cash_movement_cents: number;
  }>;
  category_trend: Array<{
    month: string;
    category: string;
    amount_cents: number;
  }>;
  transactions: FinanceTransaction[];
  sankey: Array<{
    month: string;
    source: string;
    target: string;
    amount_cents: number;
    account: string;
    category: string;
  }>;
  accounts: string[];
  categories: string[];
  investment: ReviewedHoldingsSnapshot | null;
  portfolio_values: PortfolioValuation[];
  shared_expenses: SharedExpenseSummary[];
  purpose_spending: Array<{
    purpose: SpendingPurpose | null;
    personal_spending_cents: number;
    expense_posting_count: number;
  }>;
  trip_spending: TripSpendingSummary[];
  balance_snapshot: ManualBalanceSnapshot | null;
  tracked_net_worth: TrackedNetWorth | null;
  source_freshness: Array<{
    source: string;
    label: string;
    as_of: string | null;
    age_days: number | null;
    freshness: 'current' | 'stale' | 'missing';
    coverage: 'complete' | 'partial' | 'missing';
  }>;
  commitment_as_of: string;
  current_commitment_monthly_cents: number;
  commitments: RecurringCommitment[];
  planning: FinancePlanningReport;
}

export interface FinancePlanningReport {
  as_of: string;
  currency: string;
  baseline: {
    months: string[];
    monthly_income_cents: number;
    monthly_spending_cents: number;
    forecast_base_cents: number;
    monthly_result_cents: number;
    savings_rate_percent: number | null;
    behavior: Array<{ behavior: 'fixed' | 'variable' | 'discretionary' | 'exceptional' | 'unclassified'; monthly_cents: number }>;
    classified_value_percent: number | null;
  };
  forecasts: Array<{
    as_of: string;
    historical_base_cents: number;
    commitments_cents: number;
    subscriptions_cents: number;
    adjustments_cents: number;
    projected_spending_cents: number;
    projected_result_cents: number;
    savings_rate_percent: number | null;
  }>;
  liquidity: {
    currency: string;
    liquid_assets_cents: number;
    liabilities_cents: number;
    invested_cents: number | null;
    net_worth_cents: number | null;
    cash_share_percent: number | null;
    largest_priced_holding_percent: number | null;
    runway_months: number | null;
    target_cash_cents: number;
    cash_buffer_cents: number;
    complete: boolean;
  } | null;
  subscriptions: {
    monthly_cents: number;
    annual_cents: number;
    billing_count: number;
    covered_count: number;
    unknown_price_count: number;
    anomalies: Array<{
      subscription_id: string;
      subscription_name: string;
      kind: string;
      detail: string;
    }>;
  };
  card_decision: {
    annual_eligible_spend_cents: number;
    annual_fx_spend_cents: number;
    usage_reviewed: boolean;
    spend_source: 'manual' | 'reviewed_transactions';
    spend_period_start: string | null;
    spend_period_end: string | null;
    provisional: boolean;
    options: Array<{
      id: string;
      label: string;
      currency: string;
      annual_fee_cents: number;
      annual_face_value_cents: number;
      annual_benefit_value_cents: number;
      annual_unvalued_face_value_cents: number;
      annual_reward_value_cents: number;
      annual_fx_cost_cents: number;
      annual_net_value_cents: number;
      break_even_eligible_spend_cents: number | null;
      points_per_currency_unit_milli: number;
      point_value_milli_cents: number;
      point_value_assumption: string;
      terms_checked_on: string;
      source_urls: string[];
      benefits: Array<{
        id: string;
        label: string;
        annual_face_value_cents: number;
        annual_personal_value_cents: number;
      }>;
    }>;
  } | null;
  loyalty: Array<{
    id: string;
    label: string;
    points: number;
    point_value_milli_cents: number;
    estimated_value_cents: number;
    transferable: boolean;
    assumption: string;
    as_of: string | null;
    expires_on: string | null;
    transfer_path: string | null;
    source_urls: string[];
  }>;
  caveats: string[];
}

export interface RecurringCommitment {
  id: string;
  label: string;
  account: string;
  monthly_cents: number;
  currency: string;
  valid_from: string;
  valid_until: string | null;
}

export type BalanceCoverage = 'complete' | 'partial';
export type BalanceKind = 'asset' | 'liability';

export interface ManualBalance {
  id: string;
  label: string;
  kind: BalanceKind;
  amount_cents: number;
}

export interface ManualBalanceSnapshot {
  schema_version: number;
  as_of: string;
  updated_at: string;
  currency: string;
  coverage: BalanceCoverage;
  balances: ManualBalance[];
}

export interface ManualBalanceUpdate {
  as_of: string;
  currency: string;
  coverage: BalanceCoverage;
  balances: ManualBalance[];
}

export interface TrackedNetWorth {
  currency: string;
  value: { mantissa: string; scale: number };
  manual_balance_cents: number;
  portfolio_included: boolean;
  complete: boolean;
}

export const finance = {
  subscriptions: (signal?: AbortSignal) =>
    request<Subscription[]>('/finance/api/subscriptions', signal ? { signal } : undefined),
  /** `at` is the whole point: the price series makes "what will this cost in
   *  October" a different answer from "what does it cost today". */
  burn: (at?: string, signal?: AbortSignal) =>
    request<Burn>(
      `/finance/api/subscriptions/burn${at ? `?at=${encodeURIComponent(at)}` : ''}`,
      signal ? { signal } : undefined,
    ),
  appendPrice: (id: string, price: PricePoint) =>
    request<{ ok: boolean; id: string; created: boolean }>(
      `/finance/api/subscriptions/${encodeURIComponent(id)}/price`,
      jsonInit('POST', price),
    ),
  appendState: (id: string, change: StateChange) =>
    request<{ ok: boolean; id: string; created: boolean }>(
      `/finance/api/subscriptions/${encodeURIComponent(id)}/state`,
      jsonInit('POST', change),
    ),
  importVault: () =>
    request<{ ok: boolean; created: number; already_present: number }>(
      '/finance/api/import/obsidian',
      { method: 'POST' },
    ),
  /** Conflicts come back named, never resolved — the caller shows them. */
  writeback: () => request<WritebackResult>('/finance/api/writeback', { method: 'POST' }),
  dashboard: (filters: {
    start?: string;
    end?: string;
    account?: string;
    category?: string;
    currency?: string;
  } = {}, signal?: AbortSignal) => {
    const query = new URLSearchParams();
    for (const [key, value] of Object.entries(filters)) if (value) query.set(key, value);
    return request<FinanceDashboard>(
      `/finance/api/dashboard${query.size ? `?${query}` : ''}`,
      signal ? { signal } : undefined,
    );
  },
  /** The travel page wants only the per-trip slice, never the whole projection. */
  tripSpending: (signal?: AbortSignal) =>
    request<FinanceDashboard>(
      '/finance/api/dashboard',
      signal ? { signal } : undefined,
    ).then((dashboard) => dashboard.trip_spending),
  rebuildLedger: () =>
    request<{ ok: boolean; rows: number }>('/finance/api/ledger/rebuild', { method: 'POST' }),
  csvMappings: () =>
    request<CsvMappingProfile[]>('/finance/api/import/csv/mappings'),
  investmentMappings: () =>
    request<InvestmentCsvMappingProfile[]>('/finance/api/import/investments/mappings'),
  previewInvestments: (content: string, mapping: InvestmentCsvMapping) =>
    request<InvestmentPreview>(
      '/finance/api/import/investments/preview',
      jsonInit('POST', { content, mapping }),
    ),
  confirmInvestments: (
    content: string,
    mapping: InvestmentCsvMapping,
    sourceKey: string,
    expectedSnapshotId: string,
    coverage: HoldingsCoverage,
  ) =>
    request<{ ok: boolean; created: boolean; snapshot: ReviewedHoldingsSnapshot }>(
      '/finance/api/import/investments/confirm',
      jsonInit('POST', {
        content,
        mapping,
        source_key: sourceKey,
        expected_snapshot_id: expectedSnapshotId,
        coverage,
      }),
    ),
  candidates: () => request<TransactionCandidate[]>('/finance/api/import/candidates'),
  previewCsv: (content: string, mapping: CsvMapping) =>
    request<CsvImportPreview>(
      '/finance/api/import/csv/preview',
      jsonInit('POST', { content, mapping }),
    ),
  importCsv: (content: string, mapping: CsvMapping, expectedPreviewId: string) =>
    request<{
      ok: boolean;
      created: number;
      already_present: number;
      duplicate_rows: number;
      preserved_repetitions: number;
      ignored_non_transaction_rows: number;
    }>(
      '/finance/api/import/csv',
      jsonInit('POST', {
        content,
        mapping,
        expected_preview_id: expectedPreviewId,
      }),
    ),
  reviewCandidate: (
    id: string,
    decision: 'confirm' | 'reject',
    account?: string,
    sourceAccount?: string,
  ) =>
    request<{ ok: boolean; id: string; state: CandidateState; journal_written: boolean }>(
      `/finance/api/import/candidates/${encodeURIComponent(id)}/review`,
      jsonInit('POST', { decision, account, source_account: sourceAccount }),
    ),
  confirmCandidates: (items: { id: string; account: string }[]) =>
    request<{ ok: boolean; confirmed: number; journal_writes: number }>(
      '/finance/api/import/candidates/confirm-batch',
      jsonInit('POST', { items }),
    ),
  reclassifyCandidates: (items: { id: string; account: string }[]) =>
    request<{ ok: boolean; reviewed: number; reclassified: number }>(
      '/finance/api/import/candidates/reclassify-batch',
      jsonInit('POST', { items }),
    ),
  reconcileTransfer: (id: string, counterpartId: string) =>
    request<{
      ok: boolean;
      canonical_id: string;
      duplicate_id: string;
      journal_written: boolean;
    }>(
      `/finance/api/import/candidates/${encodeURIComponent(id)}/reconcile-transfer`,
      jsonInit('POST', { counterpart_id: counterpartId }),
    ),
  allocateExpense: (
    id: string,
    personalCents: number,
    purpose: SpendingPurpose,
    tripId: string | null,
  ) =>
    request<{ ok: boolean; id: string; journal_written: boolean }>(
      `/finance/api/import/candidates/${encodeURIComponent(id)}/allocation`,
      jsonInit('POST', {
        personal_cents: personalCents,
        purpose,
        trip_id: tripId,
      }),
    ),
  linkReimbursement: (id: string, expenseCandidateId: string) =>
    request<{
      ok: boolean;
      id: string;
      expense_candidate_id: string;
      journal_written: boolean;
    }>(
      `/finance/api/import/candidates/${encodeURIComponent(id)}/reimbursement`,
      jsonInit('POST', { expense_candidate_id: expenseCandidateId }),
    ),
  updateBalanceSnapshot: (update: ManualBalanceUpdate) =>
    request<{ ok: boolean; snapshot: ManualBalanceSnapshot }>(
      '/finance/api/balance-snapshot',
      jsonInit('POST', update),
    ),
};

// ─── places ──────────────────────────────────────────────────────────────────
// The wire contract of capabilities/places (README.md there, "HTTP surface").
// Layer endpoints answer GeoJSON FeatureCollections so the map consumes them
// without translation; cents are integers, EUR implied, and every coordinate
// pair is [longitude, latitude].

export interface PlacesPoint {
  type: 'Point';
  coordinates: [number, number];
}

export interface PlacesLineString {
  type: 'LineString';
  coordinates: [number, number][];
}

export interface PlacesFeature<G, P> {
  type: 'Feature';
  geometry: G;
  properties: P;
}

export interface PlacesFeatureCollection<G, P> {
  type: 'FeatureCollection';
  features: PlacesFeature<G, P>[];
}

export interface SpendCitySummary {
  city: string;
  country_code: string | null;
  total_cents: number;
  transactions: number;
  latitude: number | null;
  longitude: number | null;
}

export interface SpendSummary {
  /** Summed over LINKED EUR expense rows only (places layers.rs, spend_layer). */
  total_cents: number;
  /** Every EUR expense row in the projection, linked or not — coverage, not visits. */
  transactions: number;
  /** Rows a place link exists for: the population total_cents sums, and the
   *  only valid denominator for an average over placed spend. */
  linked: number;
  venues: number;
  /** Ranked by total, descending. */
  cities: SpendCitySummary[];
}

/** Venue precision exists only where the raw export carried an address (places D1). */
export interface SpendVenueProperties {
  place_id: string;
  name: string;
  city: string | null;
  precision: 'venue';
  total_cents: number;
  transactions: number;
  avg_cents: number;
  first: string;
  last: string;
  top_category: string | null;
}

export interface SpendCityProperties {
  place_id: string;
  city: string;
  country_code: string | null;
  precision: 'city';
  total_cents: number;
  transactions: number;
}

export interface SpendLayer {
  summary: SpendSummary;
  venues: PlacesFeatureCollection<PlacesPoint, SpendVenueProperties>;
  cities: PlacesFeatureCollection<PlacesPoint, SpendCityProperties>;
}

export type TravelPhase = 'past' | 'upcoming';

export interface TravelPointProperties {
  kind: 'trip-destination' | 'station' | 'spend-presence';
  name: string;
  phase: TravelPhase | null;
  plan_id: string | null;
  first: string | null;
  last: string | null;
  visits: number | null;
}

export interface TravelRouteProperties {
  kind: 'transit-leg';
  label: string;
  phase: TravelPhase | null;
}

export interface TravelLayer {
  points: PlacesFeatureCollection<PlacesPoint, TravelPointProperties>;
  routes: PlacesFeatureCollection<PlacesLineString, TravelRouteProperties>;
}

export interface PeoplePinProperties {
  id: string;
  person: string;
  place_name: string;
  since: string | null;
  /** Null means still there. */
  until?: string | null;
  confidence_bp: number;
  source: string;
}

/** Confirmed, currently-valid companion-register rows only (places D4). */
export type PeopleLayer = PlacesFeatureCollection<PlacesPoint, PeoplePinProperties>;

export interface PersonPlaceProposal {
  id: string;
  person: string;
  place_name: string;
  city: string | null;
  latitude: number | null;
  longitude: number | null;
  date_start: string | null;
  date_end: string | null;
  confidence_bp: number;
  source: string;
  state: 'proposed';
}

/** Place text only — never a person name, an amount, or a date (places D3). */
export interface GeocodeStructured {
  street?: string | null;
  postalcode?: string | null;
  city?: string | null;
  country?: string | null;
}

export interface GeocodedPlace {
  place_id: string;
  name: string;
  /** Registry kind derived from the provider response (venue|city|station|
   *  address|region). A city-kind result must not be offered venue precision —
   *  a city bubble never pretends to be a venue (places D1). */
  kind: string;
  latitude: number;
  longitude: number;
  city: string | null;
  country_code: string | null;
}

export interface GeocodeResult {
  status: 'ok' | 'not_found';
  cached: boolean;
  place: GeocodedPlace | null;
}

export interface RegistryPlace {
  id: string;
  name: string;
  kind: string;
  city: string | null;
  country_code: string | null;
  latitude: number | null;
  longitude: number | null;
  source: string;
  external_ref: string | null;
}

/** One exact-description group of EUR expense rows with no place link. The
 *  server ranks by total descending and caps the list at 200 groups. */
export interface UnplacedGroup {
  description: string;
  transactions: number;
  total_cents: number;
  first: string;
  last: string;
}

/** The registry row an assign linked to, echoed by the assign route. */
export interface AssignedPlace {
  id: string;
  name: string;
  kind: string;
  city: string | null;
  country_code: string | null;
  latitude: number | null;
  longitude: number | null;
}

export interface AssignPlaceResult {
  ok: true;
  /** Currently-unlinked rows whose projection description matched exactly.
   *  Links are written with source='manual', ON CONFLICT DO NOTHING. */
  linked: number;
  /** The precision actually written: the server links a city-kind place at
   *  city precision whatever was requested (places D1). */
  precision: 'venue' | 'city';
  place: AssignedPlace;
}

export const places = {
  spendLayer: (signal?: AbortSignal) =>
    request<SpendLayer>('/places/api/layers/spend', signal ? { signal } : undefined),
  travelLayer: (signal?: AbortSignal) =>
    request<TravelLayer>('/places/api/layers/travel', signal ? { signal } : undefined),
  peopleLayer: (signal?: AbortSignal) =>
    request<PeopleLayer>('/places/api/layers/people', signal ? { signal } : undefined),
  proposals: (signal?: AbortSignal) =>
    request<{ proposals: PersonPlaceProposal[] }>(
      '/places/api/people/proposals',
      signal ? { signal } : undefined,
    ),
  confirmProposal: (id: string) =>
    request<{ ok: boolean; state: 'confirmed' }>(
      `/places/api/people/proposals/${encodeURIComponent(id)}/confirm`,
      { method: 'POST' },
    ),
  dismissProposal: (id: string) =>
    request<{ ok: boolean; state: 'dismissed' }>(
      `/places/api/people/proposals/${encodeURIComponent(id)}/dismiss`,
      { method: 'POST' },
    ),
  geocode: (body: { query: string } | { structured: GeocodeStructured }) =>
    request<GeocodeResult>('/places/api/geocode', jsonInit('POST', body)),
  list: (q?: string, kind?: string, signal?: AbortSignal) => {
    const query = new URLSearchParams();
    if (q) query.set('q', q);
    if (kind) query.set('kind', kind);
    return request<{ places: RegistryPlace[] }>(
      `/places/api/places${query.size ? `?${query}` : ''}`,
      signal ? { signal } : undefined,
    );
  },
  unplaced: (signal?: AbortSignal) =>
    request<{ groups: UnplacedGroup[] }>(
      '/places/api/unplaced',
      signal ? { signal } : undefined,
    ),
  /** Exactly one of `place_id` / `geocode_query`. `place_id` links an existing
   *  registry row; `geocode_query` resolves through the cached geocoder and
   *  carries place text only — never a name, an amount, or a date (places D3). */
  assignPlace: (body: {
    description: string;
    place_id: string | null;
    geocode_query: string | null;
    precision: 'venue' | 'city';
  }) => request<AssignPlaceResult>('/places/api/unplaced/assign', jsonInit('POST', body)),
};
