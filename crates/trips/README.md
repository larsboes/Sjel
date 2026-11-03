# trips

Persistent user intent and itinerary state for Sjel's cross-capability Trips workspace.
It owns neither transport search nor event discovery: clients compose `transit` and `scouting`,
and read bounded `calendar` entries as trip fixpoints. Trips stores only the references the
operator explicitly adds to the itinerary, with inert provider payloads.

The technical manual for Travel and Trips is in [`website/content/capabilities/travel.md`](../../website/content/capabilities/travel.md).

## Contract

The public shape is [`schemas/trip-plan.schema.json`](../../schemas/trip-plan.schema.json).
`trips.plans` records general places rather than only stations, up to four destinations,
a date window, travelers and allowed transport modes. Its explicit stages can use different
dates, travelers, modes and booking states. `trips.plan_items` records selected transport
options, events, activities, places, stays, images, notes and option sets. The payload is JSON
text so provider-specific evidence is preserved without making provider fields part of the
durable plan contract.

Four item types promise a defined shape, are validated on write, and name the missing field on rejection:

- `transport` — `{mode, journey}`. Holds one connection in the plan.
- `option_set` — `{query, options, observed_at?}`. Retains offered fares including unchosen ones,
  preserving pricing evidence that cannot be re-queried later.
- `booking` — `{provider, order_ref, …}`. Records purchase confirmation, order reference,
  fare name, refundability and cancellation deadline.
- `stay` — `{check_in, check_out, latitude, longitude, …}`. Lodging details with required coordinates
  for downstream geographic matching.

A **meetup** is an `activity` whose payload carries `with`: a non-empty list of `{person}`,
where `person` matches a person note name in the People registry. Its `status` is one of
`idea`, `asked`, `confirmed`, `declined`, `done`. Only those two fields are validated when `with`
is present.

Other types remain open. `event` is written by three producers with different payloads
(scouting opportunities, search results, calendar anchors), so declaring one rigid shape would
reject valid callers.

### HTTP surface

The manifest declares the capability port. The current machine-readable surface is served at `GET /routes`:

- `GET /health`, `GET /ready`, `GET /routes`
- `GET|POST /api/plans`
- `GET|PATCH|DELETE /api/plans/:id`
- `POST /api/plans/:id/items`
- `POST /api/plans/:id/resolve-places` — resolves coordinates for unlocated places; `{"refresh": true}` replaces existing ones
- `PATCH|DELETE /api/plans/:plan_id/items/:item_id`
- `POST /api/plans/:id/outcome` — records how a booked stage performed against its selected option
- `GET /api/plans/:id/cost` — calculates intent, committed cost, offered fares, and actual spending
- `POST /api/plans/:id/retrospective` — three-field close-out record
- `GET /api/retrospectives/pending` — closed trips inside the 45-day window without a retrospective
- `GET /api/retrospectives/summary` — destination retrospective weights
- `GET /api/places`
- `GET /api/flights/search`, `GET /api/flights/grid`, `GET /api/flights/when`, `GET /api/flights/pivot`
- `POST /api/plan-search` · `GET /api/plan-search/:id` · `POST /api/plan-search/:id/adopt`

Foreign browser origins are refused with 403 via `crates/sjel-server/src/origin.rs` (`SJEL_TRIPS_ALLOWED_ORIGIN_HOSTS`
configures allowed hosts). Requests with no `Origin` header pass for local server-to-server calls.

## Why this shape: Derived coordinates

A destination requires coordinates to render on maps, anchor accommodation searches, and calculate
scouting distances or stage climate normals. `src/place_resolve.rs` derives coordinates on plan writes
(`POST`/`PATCH /api/plans`):

1. **Registry first, geocoder second.** Exact name matches query `GET /api/places?q=<name>` instantly
   without network calls. Non-exact names fall through to Nominatim via `POST /api/geocode` at rate limits.
2. **Saves never fail over missing coordinates.** When `places` is unavailable, the plan is stored
   with null coordinates. `POST /api/plans/:id/resolve-places` allows manual or scheduled retry.
3. **Clients cannot erase existing coordinates.** Saving an unchanged destination name with null
   coordinates preserves the stored coordinates.
4. **Coordinates are not route changes.** `same_route` compares identity and name; updating coordinates
   does not regenerate stages or drop existing bookings.

## Why this shape: Retrospective and cost roll-up

`POST /api/plans/:id/outcome` and `POST /api/plans/:id/retrospective` address different needs:

- **Outcome**: Evaluates one booked stage against the specific option chosen (`selected_option_id`).
- **Retrospective**: Per-plan review after completion (`cost_cents`, `again`, `notes`). Stored in
  `trips_retrospectives` with indexed destination grouping and validated columns:
  `CHECK (again IN ('yes','no','maybe','not_taken'))`. `not_taken` records planned trips that did
  not occur without skewing destination satisfaction metrics.

`GET /api/plans/:id/cost` joins three distinct sources:

- **Intent**: Plan budget (`trips_plans.budget_cents`).
- **Committed**: Confirmed booking and stay costs in integer minor units.
- **Offered**: Uncommitted fare options, reported separately without mixing currencies or speculative amounts.
- **Actual**: Retrieved from `GET /finance/api/trips/:id/spending` over loopback. If Finance is unreachable
  or currencies conflict, figures degrade cleanly to null with explanatory reasons rather than falling back to zero.

## Why this shape: Candidate ranking and plan search

`POST /api/plan-search` generates initial journey candidates from search criteria. It runs asynchronously,
returning 202 with a job identifier, composing feasible calendar windows, the place registry, scouting
opportunities, transit fares, and climate data.

Candidates are ranked across four factors (`budget_fit`: 0.35, `feasibility`: 0.30, `season`: 0.20,
`events`: 0.15) under `plan-search-v1`. Unmeasurable factors are excluded and weights re-normalized.
Month searches without calendar inputs fail explicitly because feasible windows are required to bound transit queries.
Adopted candidates write durable `option_set` records via `POST /api/plan-search/:id/adopt`.

## Why this shape: Packing and gear attributes

Pack lists integrate with `crates/interior`:

- Gear attributes are stored in `interior_item` with seven dedicated columns (`weight_g`, `category`,
  `packable`, `waterproof`, `quick_dry`, `pack_location`, `trip_types`).
- `GET /api/plans/:id/pack` returns item attributes and calculates `total_weight_g` alongside a
  `weights_missing` count, ensuring incomplete lists do not present misleading partial sums.
- `trips gear import --apply` synchronizes gear definitions via Interior's `POST /api/items` idempotently.

## Why this shape: Plan safety copies

Plans project markdown safety copies under `<overlay>/data/trips/plans/` (`SJEL_TRIPS_EXPORT_ROOT` redirects it):

- One markdown file per plan, carrying `sjel_trip_id`, `sjel_schema`, and `sjel_revision` (`updated_at`) frontmatter.
- Safety copies exist for disaster recovery and offline inspection, preserving item payloads verbatim.
- Safety copies update after non-GET mutation requests and via manual CLI `trips export`. Deleted or renamed
  plans are cleaned up to ensure backup state matches database reality.

## Related tools and alternatives

| Tool | Focus | Relationship to Sjel |
|---|---|---|
| [TREK](https://github.com/liketrek/TREK) | Self-hosted group trip planning and expense sharing | Reference for collaborative trips. Sjel exchanges neutral exports rather than duplicating real-time multi-user UI. |
| [TripIt](https://www.tripit.com/web/free) | Automated itinerary generation from forwarded emails | Reference for booking parsing. Sjel avoids mandatory external cloud sync. |
| [Besser Bahn](https://github.com/chuk-development/Besser-Bahn) | Android live rail companion with delay prediction and split ticketing | Complementary during travel; Sjel retains connections within the overall itinerary. |
| [BetterBahn](https://betterbahn.de) | Split-ticket calculation and route inspection | Algorithmic inspiration for domestic fare optimization. |

## Considered and declined

- **No leave-home scheduler in Sjel.** A departure scheduler belongs to the home automation runtime (Home Assistant in the private overlay), not to a travel capability. Sjel produces the calendar `away` entries that an external automation reads.
- **Sjel never executes a booking.** The carrier's own checks and refund terms apply in a browser. The operator pastes confirmation evidence into a `booking` plan item (`provider`, `order_ref`), and Sjel never buys tickets directly.
- **No flight search.** Flight search is external. Sjel holds flights in a plan (`PlaceKind::Airport`, `TransportMode::Flight`, `transport` items), but does not own a flight provider search engine.
- **No cloud AI for itinerary ranking.** Trust tier boundaries and pseudonymization rules keep itineraries local. The redaction rules for cloud derivatives are designed for generic text, not itinerary data.
- **No model-read scouting adapter for foreign-language sources.** Extracted events from untrusted HTML require verifiable ground truth; arbitrary model scraping without server-rendered ground truth is declined.

## Build

```bash
cargo test -p sjel-trips
cargo build --locked --release --bin trips-server
```
