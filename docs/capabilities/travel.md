# Travel & multi-modal routing architecture

Trips owns itinerary state and composition. Transit owns journey search; Calendar owns availability;
Punctuality provides measured historical reliability, rather than a live delay feed. See
[Trips README](../../capabilities/trips/README.md) for the domain model, and
[travel-traps.md](travel-traps.md) for the seven failures that return an empty or plausible answer
instead of an error.

## 0. The order the capabilities are asked in

Five capabilities answer five different questions, and the order decides whether the answer is
worth anything. This table and the order below were `Packs/travel`'s agent skill until 2026-10-24;
they are the workflow, so they live beside the capabilities they describe.

| Capability | Owns | Worth knowing |
|---|---|---|
| calendar | feasibility verdicts, feasible windows, dated commitments | A verdict is soft. `conflicts` is still returned, never hidden |
| transit | journey search, split-ticket chains, rail ticket extraction | Owns no idea why a day is possible |
| punctuality | measured delay history per station, train type and hour | Measured, not predicted, and not transfer risk |
| scouting | opportunity discovery, scoring and event routing | Owns no time and no itinerary |
| trips | the plan and its items | **Owns no search at all.** It stores what was decided |

1. **calendar** — `GET /api/windows?from=&to=&min_days=` returns the runs of days travel is
   possible in, each carrying the days inside it that would cost a travel day.
2. **transit** — search only those days. Over HTTP that is `GET /api/search?from=&to=&time=`
   per day. For a fuzzy multi-destination fan-out use the CLI: `transit plan --dates` takes the
   day list from step 1 verbatim. There is no `plan` route on the server.
3. **trips** — `POST /api/plans`, then `POST /api/plans/:id/items` for the connection, event or
   stay that was chosen.

Running 2 before 1 samples a month of dates and prices the ones nobody can travel on. Calendar
publishes the days and transit never asks for them; the caller moves the list across. The
argument for that direction is in `capabilities/calendar/README.md` § Calendar hands the days
over. `--dates` is mutually exclusive with `--date-from`/`--date-to`, and `--max-queries` still
caps the fan-out.

Drive it with `sjel capability call <name> <verb> <path> [body]`, or read
`bun tools/travel.ts candidates --json` for the assessment the Travel page renders — scouting
opportunities matched against trip plans within 75 km, then calendar verdicts on whatever did not
match. That script imports the page's own functions, so the 75 km rule has one home.

Stations, home coordinates, calendar credentials and route defaults live in the active overlay.
Never write a capability's tables directly; every cross-capability hand-off here goes over HTTP.

## 1. Intent-driven trip drafting

`trips draft-intent "weekend in Paris by train"` produces a `CreatePlan`-shaped form draft,
with `unresolved`, `assumptions`, and `source_text`. It persists nothing, resolves no station,
and supplies no fare or feasibility verdict. Destinations are place slugs with null coordinates.
The operator reviews the form before a plan is created or searched.

The [CLI](../../capabilities/trips/src/bin/trips-cli.rs) calls a local model and falls back to a
heuristic draft when it cannot reach that model. The [intent module](../../capabilities/trips/src/intent.rs)
validates the model's fields. Actual multi-constraint search is a separate operation implemented
in [plan_search.rs](../../capabilities/trips/src/plan_search.rs), with its own pricing budget and
explicit partial-result warnings.

## 2. Reservation intake & KItinerary integration

Reservation items can retain booking details and evidence in a plan. This does not establish an
automated ticket-extraction pipeline. The KItinerary evaluation is recorded in
[`upstreams.toml`](../../upstreams.toml) under `kitinerary`: HTML containing a JSON-LD
`TrainReservation` was extracted on Linux, while a plain iCalendar `VEVENT` returned no result.
The macOS binary failed because of unresolved library paths. Real household PKPass, PDF, and UIC
barcode extraction was not measured in that evaluation.

Treat those formats as candidates to validate before building automatic intake. Open integration
work belongs in [ISA.md](../../ISA.md).

## 3. Packing & inventory role separation

Trips reads item attributes through the inventory contract; gear import writes through the owning
service's HTTP handler. See [the client](../../capabilities/trips/src/interior_client.rs) and
[pack lists](../../capabilities/trips/src/pack.rs).

Inventory supports `piece`, `slot`, and `gear` item kinds. Pack lists are explicit records; stage-specific
lists bind to the destination ID, with sequence as a repeated-destination tiebreaker. A missing
destination reports a lost binding. Weight totals sum known weights and report how many items
have no weight, so an incomplete list cannot present its total as complete.

## 4. Backend fragility

The rail side rests on reverse-engineered bahn.de profiles; the air side on an open channel. The earlier provider evaluation recorded the following limits. These are historical findings,
not current availability or pricing guarantees; provider provenance is in
[`upstreams.toml`](../../upstreams.toml).

- **`dbweb` (`bahn.de/web/api`) is the shakier profile.** db-vendo-client rates it "less stable" with aggressive blocking; the sibling `db` profile lost DNS in May 2026, and `dbnav` is rated more stable but carries open 403 issues of its own. A second backend is the highest-value defensive change here, not an optimization — it shipped as the `dbnav` profile.
- **No authorized German rail fare API was identified in that evaluation.** The implementation therefore used reverse-engineered rail profiles; this is not evidence that no commercial alternative can exist.
- **`mcp.kiwi.com` publishes no rate limit, quota, SLA or acceptable-use policy**, and whether persisted results are permitted is unconfirmed. Do not infer a service guarantee or permission to persist results from an unauthenticated response.
- **Expedia's connector has real data bugs** (USD returned for a EUR user, GBP bag fees, a DUS departure labelled "Cologne", `seats_left: 0` on bookable results). Second opinion only, never a source of truth.
- **No cancelled train has been observed live**, so `originCancelled`/`destinationCancelled` remain vocabulary rather than a proven signal; they are fixture-tested only.

## 5. What not to build

Each of these was tempting, and the reason it was cut still holds.

| Idea | Why not |
| :--- | :--- |
| A Google Flights scraper | The bahn.de failure mode with worse odds; the tracker's cookie-consent wall has been open since May 2026 |
| A browser extension re-running searches in the background | The "only reading a page I opened" position ends once the extension issues the fetch; robots.txt disallows the path |
| Self-hosting MOTIS or OpenTripPlanner for rail+air | Solves the half that already works and imports the half that does not: a MUC→FCO air query returns four itineraries routed via Sardinia |
| Duffel | Search is free only up to a 1500:1 search-to-book ratio this operator will never meet |
| SerpApi or Bright Data | €300–500 a year to price-check six flights, and residential proxying contradicts the privacy constraint |
| Applying to Skyscanner, Kiwi Tequila or Travelpayouts | All three decline individuals |
| Anything on Amadeus Self-Service | Reported dead since July 2026 |
| A bespoke rail price-watch daemon | Drops exist but are rare and unquantified; nothing bigger than the one scheduled watch is justified |

Cut from the earlier roadmaps for the same reason — machinery ahead of the record it would read: a read-only cross-capability context endpoint on Trips (a CLI covers it), comms' approved-booking-intent choreography copied into trips (nothing leaves the machine, and `expected_updated_at` covers the stale read), a traveler profile whose honest answer would be `insufficient_data` for a year, model-written risk sentences and day-plan permutations over a `day` column nothing maintains, and auto-intake of booking confirmations before `extract` has been run against real tickets.

## 6. Open questions and known gaps

[ISA.md](../../ISA.md) owns unresolved travel work. Verify a suspected gap against the owning
implementation before adding a new producer or a second source of planning state.
