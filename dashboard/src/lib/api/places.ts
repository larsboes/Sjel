import { jsonInit, request } from './core';

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
  create: (body: {
    name?: string;
    query?: string;
    link?: string;
    note?: string;
    address?: string;
    city?: string;
    country_code?: string;
    kind?: string;
    latitude?: number;
    longitude?: number;
  }) => request<{ ok: boolean; id: string; place: unknown }>('/places/api/places', jsonInit('POST', body)),
};
