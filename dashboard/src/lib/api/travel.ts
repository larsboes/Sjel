import { jsonInit, request } from './core';
import type { EventRoute, OpportunityStatus } from './scouting';

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
  /** Rename and/or merge payload keys; a null value removes the key (trips `edit_item`). */
  editItem: (planId: string, itemId: string, edit: { title?: string; payload?: Record<string, unknown> }) =>
    request<PlanItem>(
      `/trips/api/plans/${encodeURIComponent(planId)}/items/${encodeURIComponent(itemId)}/fields`,
      jsonInit('PATCH', edit),
    ),
  draftIntent: (sentence: string) =>
    request<IntentDraft>('/trips/api/intent/draft', jsonInit('POST', { sentence })),
  pack: (planId: string) =>
    request<PackView>(`/trips/api/plans/${encodeURIComponent(planId)}/pack`),
  createPackList: (planId: string, name: string) =>
    request<PackList>(
      `/trips/api/plans/${encodeURIComponent(planId)}/pack`,
      jsonInit('POST', { name }),
    ),
  outfits: (planId: string) =>
    request<{ outfits: Outfit[] }>(`/trips/api/plans/${encodeURIComponent(planId)}/outfits`),
  /** Replaces every outfit on the plan; send them all, not a delta. */
  putOutfits: (planId: string, outfits: OutfitInput[]) =>
    request<{ ok: boolean; count: number }>(
      `/trips/api/plans/${encodeURIComponent(planId)}/outfits`,
      jsonInit('PUT', { outfits }),
    ),
  /** Replaces the list's whole item set; send every row, not a delta. */
  putPackItems: (planId: string, listId: string, items: PackItemInput[]) =>
    request<{ ok: boolean; count: number }>(
      `/trips/api/plans/${encodeURIComponent(planId)}/pack/${encodeURIComponent(listId)}/items`,
      jsonInit('PUT', { items }),
    ),
};

/** `capabilities/trips/src/outfit.rs` — pieces worn together, optionally on one day. */
export interface OutfitInput {
  name: string;
  day: string | null;
  pieces: string[];
  note: string | null;
  /** An agent's suggestion the traveler has not accepted yet. */
  proposed?: boolean;
}

export interface Outfit extends OutfitInput {
  /** Pieces no pack list on the plan carries; computed by trips. */
  not_on_pack_list: string[];
}

/** `capabilities/trips/src/pack.rs` — one row of a list, resolved against inventory. */
export interface PackItem {
  item_ref: string;
  label: string | null;
  packed: boolean;
  note: string | null;
  resolved: boolean;
  category: string | null;
  weight_g: number | null;
}

export interface PackItemInput {
  item_ref: string;
  packed: boolean;
  note: string | null;
}

export interface PackList {
  id: string;
  name: string;
  items?: PackItem[];
}

export interface PackView {
  lists: (PackList & { items: PackItem[]; total_weight_g?: number; weights_missing?: number })[];
}

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
