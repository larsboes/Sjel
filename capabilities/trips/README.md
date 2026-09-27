# trips

Persistent user intent and itinerary state for Axon's cross-capability Trips workspace.
It owns neither transport search nor event discovery: the dashboard composes `transit`
and `scouting`, and reads bounded `calendar` entries as trip fixpoints. Trips stores only
the references the operator explicitly adds to the itinerary, with inert provider payloads.

## Contract

The public shape is [`schemas/trip-plan.schema.json`](../../schemas/trip-plan.schema.json).
`trips.plans` records general places rather than only stations, up to four destinations,
a date window, travelers and allowed transport modes. Its explicit stages can use different
dates, travelers, modes and booking states. `trips.plan_items` records selected transport
options, events, activities, places, stays, images, notes and option sets. The payload is JSON
text so provider-specific evidence can be preserved without making provider fields part of the
durable plan contract.


That freedom cost a caller the ability to know what to send: any JSON was accepted, so a
guessed shape got a 201 and a row nobody could read back. Four item types now promise a shape,
are validated on write, and name the missing field on rejection:

- `transport` — `{mode, journey}`. One producer, one shape, and the item to write for "hold
  this connection in the plan".
- `option_set` — `{query, options, observed_at?}`. Every fare a search offered, including the
  ones not taken. It exists because an unchosen fare cannot be queried back later at
  yesterday's price, so an unrecorded option set is gone rather than merely unwritten.
- `booking` — `{provider, order_ref, …}`. What makes a stage's `booked` status mean something:
  the order reference, fare name, refundability and cancellation deadline of a purchase made
  elsewhere. (Declared 2026-08-11; this list previously stopped at two.)
- `stay` — `{check_in, check_out, latitude, longitude, …}`. Where you sleep, next to how you
  get there. Declared for its intended producer, accommodation search results entered through
  the agent surface (in-repo, the demo seeder is the one writer); coordinates are required
  because the place matching downstream runs on them, and the provider's URL, price and
  rating ride along unvalidated.

A **meetup** is an `activity` whose payload carries `with`: a non-empty list of
`{person}`, where `person` is the note name under `Atlas/People/`, the same key the places
companion register uses. Its `status` is one of `idea`, `asked`, `confirmed`, `declined`,
`done`. Only those two fields are checked, and only when `with` is present. `activity`
stays permissive for every other writer, and a new `item_type` would have meant rebuilding
the items table's `CHECK` over rows that exist nowhere else (2026-09-25).

Every other type stays permissive on purpose. `event` alone is written by three producers with
three different shapes (a scouting opportunity, a whole search result, a calendar anchor), so
declaring one shape for it would reject two of them. A variant is declared where there is
exactly one shape to promise, and nowhere else.

The accommodation flow, for the agent surface: anchor the search on the stage's
**destination coordinate**, not the city name — the provider takes coordinates directly, and a
coordinate is what lets the result match places later. When the stage carries no coordinate
(imported and drafted plans often don't), resolve one from `GET /api/places` before falling
back to geocoding: a place already visited usually has it. Write the offers as one
`option_set` (query, anchor provenance, every offer with its coordinate and URL), then the
chosen candidate as a `stay`. First run 2026-08-12 against a real plan: the October Berlin
stage had no coordinate, the December Berlin place did, and the search rode that one.

The Travel workspace exposes plan editing for title, start, up to four destinations, dates,
interests, travelers and transport modes through the existing `PATCH /api/plans/:id` contract.
Deletion uses `DELETE /api/plans/:id` behind a two-step UI confirmation. Deleting an imported
Axon plan never deletes its source Obsidian note.

For an active plan, the dashboard queries Calendar with the plan's inclusive date window
converted to Calendar's exclusive `to` bound. Planned and committed events at a destination
are shown before fresh discovery results as fixpoints. Adding one to the itinerary remains
an explicit action and records `calendar:<entry-id>` as the external id; opening Travel does
not copy or mutate calendar data.

The Travel overview also consumes Scouting's `travel_candidate` route. A future event matches
an upcoming plan only when its start day is inside the plan window and its complete coordinate
is within 75 km of a complete destination coordinate (great-circle distance; never city-string
equality or a missing-coordinate `0,0` guess). Unmatched dated candidates are sent to Calendar's
batch verdict endpoint. `free` and `needs-travel-day` remain actionable with the cost named;
`conflicts` is displayed as a no and cannot seed a plan. A matched event is added through Trips'
idempotent plan-item API only after an explicit click. A viable unmatched event only pre-fills
the existing plan form; the operator still supplies/reviews the route before saving.

This join remains computed in the dashboard because that is already the documented composition
edge for Trips, Scouting, and Calendar. It persists no recommendation and reads no foreign store.

HTTP surface on the manifest-declared port. `GET /routes` is the machine-readable
version and is the one a caller should trust; this list was ten bullets covering
eleven of the twenty pairs the router served, and is corrected here to all
twenty-four:

- `GET /health`, `GET /ready`, `GET /routes`
- `GET|POST /api/plans`
- `GET|PATCH|DELETE /api/plans/:id`
- `POST /api/plans/:id/items`
- `PATCH|DELETE /api/plans/:plan_id/items/:item_id`
- `POST /api/plans/:id/outcome` — how one booked stage went against the option it
  was chosen under
- `GET /api/plans/:id/cost` — what the trip was meant to cost, what was
  committed to, and what was actually paid
- `POST /api/plans/:id/retrospective` — the plan's three-field close-out record
- `GET /api/retrospectives/pending` — closed trips inside the 45-day window with
  no record yet: what the dashboard ladder raises
- `GET /api/retrospectives/summary` — the feed-forward weight, per destination
- `GET /api/places`
- `GET /api/flights/search`, `GET /api/flights/grid`, `GET /api/flights/when`,
  `GET /api/flights/pivot`
- `GET /api/import/obsidian/scan`
- `POST /api/import/obsidian`
- `POST /api/import/obsidian/all`
- `POST /api/plan-search` · `GET /api/plan-search/:id` · `POST /api/plan-search/:id/adopt`

`GET /routes` is the current list; the names above are the ones worth knowing by heart.

**Foreign origins are refused.** Since 2026-09-05 (PRD Q91) every route here answers 403 to a
browser whose `Origin` is not one the dashboard is served from, using the shared predicate in
`libs/sjel-server/src/origin.rs` (set `SJEL_TRIPS_ALLOWED_ORIGIN_HOSTS` to name the
deployment's hosts). The reason is the plan-search body: it carries the operator's feasible
calendar windows and a companion hint, and `CorsLayer::permissive()` made every route above
it readable by any page open in the operator's browser. It also closes an older leak —
`GET /api/flights/when` returns calendar entry titles in `collisions`. A request with **no**
`Origin` still passes, which is how `capabilities/calendar` POSTs into this capability. New
routes must be registered above the `.layer()` call in `build_router`: axum wraps only the
routes added before it.

### The retrospective, and why it is not the outcome route

PRD Q84 (2026-09-05) rules the retrospective, the roll-up and the feed-forward weight below.
`POST /api/plans/:id/outcome` and `POST /api/plans/:id/retrospective` answer
different questions at different grains, and both are kept. The outcome record
measures ONE booked stage against the option it was chosen under, and it refuses
a stage with no `selected_option_id` because there is then nothing to compare
against. The retrospective is per-plan judgement written when the trip is over:
what it cost, would I do it again, what would I change. Closing the second does
not close the first.

It is a table rather than a twelfth plan-item type for two reasons that stand
alone. The summary groups by destination ACROSS plans, which is a query with an
index rather than a `json_extract` scan of every item row. And the three fields
are ruled and closed, so `again` earns
`CHECK (again IN ('yes','no','maybe','not_taken'))` and `cost_cents` earns
INTEGER — neither of which a JSON payload carries. The `outcome` payload next
door is deliberately open precisely because nobody knew its fields yet.

`not_taken` is the fourth word and it is not a judgement (added 2026-09-23). A
trip that was planned and did not happen had no way to be recorded: the ladder
kept asking, and the only answers were a verdict about a place nobody visited or
archiving the plan — which also hides the planning that did happen. It scores
nothing, contributes to neither `mean_again` nor `n`, and puts no plan id in
`basis`, because "I did not go" is not evidence about a destination in either
direction. That widening needed a real migration: `CREATE TABLE IF NOT EXISTS`
folds a widened `CHECK` only on a file that never had the old one, and every
deployed machine has the old one, so `store.rs::widen_again_vocabulary` rebuilds
the table — rename, recreate, copy, drop — inside the migration's own
transaction.

The money is denominated exactly once. `cost_cents` is in the PLAN's `currency`;
the retrospective carries no currency column, and a plan with none refuses a cost
with a message naming the fix.

### The cost roll-up

`GET /api/plans/:id/cost` joins three sources and stores nothing. It takes no
parameters: the window is the plan's own dates, so nobody can ask for a partial
total.

The sources are kept apart on purpose. **Intent** is `trips_plans.budget_cents`.
**Committed** is `booking.amount_cents` and `stay.amount_cents` — integer minor
units with an ISO code beside them. **Offered** is `option_set` and `transport`
prices, which the schema gives no currency field at all, so they are reported in
their own block labelled offered-not-paid and are never summed in: adding a float
of unknown currency to an integer minor-unit sum produces a number that is wrong
in a way nobody can see. Bookings in two currencies are reported per currency
with `booked_cents: null` and a stated reason, rather than added. The same rule
holds at every grain: a stage row and the unattributed row each carry their own
`currency`, and each answers `booked_cents: null` with its own `reason` when the
prices attributed to it disagree. A priced item that names no currency on a plan
that names none either is money in an unknown unit, so it refuses the total
instead of falling out of it — the headline used to answer a confident `0` while
that item's money still showed in a stage row.

**Actual** comes from `GET /finance/api/trips/:id/spending` over loopback with a
3 s timeout, and the response carries **all four** of finance's figures — paid,
gross out, reimbursed, still owed — rather than one flattened total. On a trip
with friends those four differ, and that difference is the shared-cost surface.
When finance does not answer, every figure is null with a named reason and
`ok: false`. Never `0`. `actuals.currency` carries the unit finance stated, and
when that disagrees with the plan's the figures are unknown with a reason rather
than relabelled: they are somebody else's numbers. The degrade rule lives in
`src/finance_client.rs` so a handler cannot quietly copy a softer one.

`flight_when` used to be the counter-example here, degrading an unreachable calendar
into "every day free" with nothing in the body saying so. It no longer does: since
2026-09-05 those days come back `DayLoad::Unknown`, sort behind every measured band,
and the reply carries `calendar` and `degraded` (PRD Q91).

**The mechanism is done and the inputs are empty, which is the honest state to record.**
Measured 2026-09-05 against a copy of the live database: 0 of 13 plans carry a budget,
0 `booking` items exist, and 0 of 1,338 finance projection rows carry a trip id. So every
headline figure this route publishes is unknown for every live plan today, and the actuals
block was proven against a stub and against a closed port rather than against finance.
Budget input now reaches the wire end to end — the field, `PATCH /api/plans/:id`,
`schemas/trip-plan.schema.json`, and clearing as well as setting — so the first half is
data entry. The second half is allocation, which is a finance task and not a trips one.

Attribution to a stage uses `payload.stage_id`, else an `external_id` equal to a
stage's `selected_option_id`, and otherwise counts the price in `unattributed`.
There is deliberately no third, date-based guess: a stay spanning three stages has
no single right answer, and inventing one would hide the gap that block exists to
show. A stage id is also not durable — `generated_stages` remints ids on a real
route change — so a missing match is a normal outcome rather than an error.

### Why `by_companion` is not served

`GET /api/retrospectives/summary` publishes a factor per DESTINATION only. The
companion half was designed and cut, and the reason belongs here rather than in a
commit message: a route keyed by a traveler name, carrying a score and a basis of
plan ids that resolve to destinations and date ranges, is person + place + date
range readable by any page in the operator's browser. `places` — which owns the
companion register — layers `refuse_foreign_origins` on its whole router exactly
because that register is C2, and the shipped invariant for this same data (places
ISA PLC-12) is falsified by "a person name in its output".

Three preconditions, recorded so this is a plan and not a rediscovery. **The first
is met as of 2026-09-05** and the other two are not:

1. ~~an origin refusal shipped on trips~~ — done, see *Foreign origins are refused*
   above. It was the load-bearing one: until then this router ended in
   `CorsLayer::permissive()` and refused no origin anywhere;
2. a key that is the register's person id rather than a raw name;
3. a class column that a mechanism reads, rather than a label a body asserts
   about itself.

**Found and not fixed here:** `trips_plans.travelers` is *already* served by
`GET /api/plans`. The origin refusal narrows who can ask, and it does not close
the defect — a traveler name is still in a response body this capability has no
class column for. This contract declines to amplify it and does not pretend to
have closed it.

Rows live in the shared SQLite file — `SJEL_DB_PATH`, else
`$SJEL_PERSONAL_ROOT/data/axon/axon.db` — under the table prefix `trips`, so the three
tables are `trips_plans`, `trips_plan_items` and `trips_retrospectives`
(libs/sjel-store/README.md). `trips_retrospectives` is one row per plan —
`plan_id` is the PRIMARY KEY, so a second POST is a correction rather than a
second row — with `ON DELETE CASCADE`, which is enforced because
`PRAGMA foreign_keys = ON` is set per connection. No personal station,
destination or credential is tracked here.

The plan-search result is deliberately **not** a table. It is a §6.2 derived aggregate, C1:
it holds a companion COUNT and never a register row, and it is never persisted and never
projected. What is worth keeping becomes durable only through
`POST /api/plan-search/:id/adopt`, which writes one `option_set` row in integer minor units.
`schemas/trip-plan.schema.json` is **extended** for it rather than merely cited, because the
projection stamps its name into every projected vault file's frontmatter — so the revision,
the degradation list, the query destinations and the per-option cost, score and factor fields
are declared there rather than written past it. `revision` is declared optional: making it
required would have retroactively invalidated the twelve `option_set` rows already in the live
database and every row the 12-hourly fare watcher writes.

Obsidian scanning is enabled by `$SJEL_PERSONAL_ROOT/config/trips.json`, shaped like
[`schemas/trips.json.example`](../../schemas/trips.json.example). The scanner stays inside
that configured root and considers only Markdown notes with `category: trip`. Scanning is
read-only. Import is explicit, idempotent by vault-relative path and requires a chosen
origin instead of guessing one. This contract does not scan Comms keeper notes or Scouting
opportunities; those vault surfaces remain owned by their respective capabilities.

## A sentence to a draft

Until 2026-09-05 there was one way to start a trip: a form needing an origin picked from
`transit.suggest`, destinations, dates and modes typed field by field, so "Somewhere warm in
October, under 300 euro, by train" had no entry point at all.

`POST /api/plan-search` is that entry point, as a form rather than as a sentence (PRD Q91,
2026-09-05). It answers 202 with a job number because fares take seconds each; the job
composes calendar's feasible windows, the place registry, scouting's opportunities, transit's
fares and — when it exists — climate, then ranks candidates on four visible factors
(`budget_fit` 0.35, `feasibility` 0.30, `season` 0.20, `events` 0.15) in the shape the feed
evaluator publishes, revisioned as `plan-search-v1`. A fifth slot, `retrospective`, is
declared and not computed; taking it bumps the revision to `plan-search-v2`, which is what
having one is for.

The job map is in-process, capped at `MAX_JOBS = 10`, evicts only finished jobs and carries
`JOB_DEADLINE_S = 180` — `capabilities/interior`'s shape, for its stated reason: a ranked
option space is a list of proposals rather than a fact about the trip, so it may die with the
process. A panicking search finishes its own entry as failed, because ten panics wedged this
route at 503 until a restart. The window taken is the **best feasible** one — ranked by
verdict band, then by fewer days needing a travel day, then earlier — rather than the first
the calendar happened to list.

Measured 2026-09-05: one live search considered 40 destinations, shortlisted 8 for pricing and
finished in 5 seconds against the 180-second deadline. The opportunity table held 6 rows
starting that day or later, so the events factor contributes close to nothing today — declared
rather than pretended. Transit's HAFAS client carries a 15 s timeout and no inter-request
pause, so the job keeps a 250 ms cadence itself (`src/upstream.rs`, `FARE_PAUSE`) and reports
`priced` against `considered`.

Two rules make the ranking readable. A factor that could **not be measured is absent** from
`factors[]` and the remaining weights re-normalise to 1, so no number in the response is a
guess wearing a measurement's clothes; a candidate with no measurable factor at all carries
`score: null` rather than a zero nobody can tell apart from a measurement. `degraded[]` names
every input that was missing, and scores are comparable inside one response and not across
two. And a **month search with no calendar fails** — "a month search needs feasible windows"
— because calendar → transit is the load-bearing order; an explicit `date_window` degrades
instead, reports `window_source: "caller"` and drops the feasibility factor.

The one fare source behind this route is transit, which prices rail. A search for a mode it
cannot price is **not** priced with a rail fare: no fare probe is made, `degraded[]` says
"no fare source covers &lt;mode&gt; in this search", and every candidate comes back
`cost_basis: "unpriced"`. Flights are priced by `GET /api/flights/search`, which is a
different route with a different upstream.

The sentence front door is still the CLI below.

```bash
trips draft-intent "Munich for a conference the 14th to the 16th of September 2026, by train"
```

It prints a `CreatePlan`-shaped body plus what it could not settle, and persists nothing.
Every destination comes back as a `place:<slug>` with null coordinates, bit-identical to
what the dashboard mints from typed text, so a station still has to be picked before
anything can be searched. The model emits no EVA, no price, no feasibility and no plan id;
if it returns them anyway they are dropped, because nothing reads them.

CLI only and no HTTP route, deliberately. The question is whether a small local model turns
a travel sentence into a valid form, and until that has started a real trip more than twice
it does not need a surface.

### What the model got wrong, and what catches it

Measured against the on-device rung on 2026-08-11, so these are observations rather than
worries:

- Asked for "somewhere warm in October" with no year, it answered **2023**, twice. A plan
  quietly created for a date three years past is worse than a blank field, so a date is
  kept only when it is well-formed and not in the past.
- It lists `dates` as unresolved almost every time, **including when it has just returned
  correct ones** — for the Munich sentence above it answered 2026-09-14/16 and called dates
  unresolved in the same object. Trusting that claim discarded dates the sentence plainly
  gave.

So neither the dates nor the self-report is trusted. The check decides and rewrites
`unresolved` to match, which is the whole shape of this: the model proposes words, a
deterministic path decides what survives. A prompt is not a validation layer.

## Gear waited for a column, and the wait is the record

Pack lists were built on 2026-09-05 and **reverted before the merge** (`815750c`), because
`interior_item` could not carry them (PRD Q92). Measured that day: 47 rows, all furniture
(29 `piece`, 18 `slot`), `CHECK (kind IN ('piece','slot'))`, and **no column** for
`weight_g`, `category`, `packable`, `waterproof`, `quick_dry`, `pack_location` or
`trip_types`. Every pack list answered a null weight with a stated reason, which is honest
and useless. Shipping the two tables anyway would have frozen a half-shape in a database
this repo has no versioned migration path to reshape — and the live file already carries two
empty pack tables no merged commit put there, from a worktree release build whose migration
ran against the real database (`CONTRIBUTING.md`, *Validate the changed boundary*).

**Both halves landed on 2026-09-07 (B51), in the order the deferral named.** Interior's
`interior_item` took the seven columns and `kind` accepts `gear`
(`capabilities/interior/src/store.rs`), then the reviewed pack half came back from history.
`trips gear import` is still a proposal reader by default and now writes with `--apply`,
through interior's own `POST /api/items` and never SQL, though both capabilities open the
same file: that handler answers 409 for an id it already has, so a re-run is idempotent
without this side tracking anything.

`GET /api/plans/:id/pack` carries the attributes now rather than nulls with a reason:
`weight_g`, `pack_location`, `category`, `packable`, `waterproof`, `quick_dry` and
`trip_types` per item, and per list a `total_weight_g` **beside a `weights_missing` count** —
a sum over an incomplete list, printed alone, is a number a reader will trust.
`gear_attributes` stays in the body rather than being removed: a client reading a deployment
whose interior predates B51 still gets a truthful `false`.

Measured 2026-09-05 and unchanged since: the import read 65 overlay notes, 61 carrying all
seven fields across 13 distinct trip types. That measurement is why `template_key` is free
text over the notes' own vocabulary rather than a template table — a template is a filter
over attributes the data already carries, and a second copy of a filter drifts.

**What shipped is the write PATH. No import has been run.** Measured 2026-09-08 on a copy of
the deployment's database: `interior_item` holds 29 `piece` and 18 `slot` rows and **no
`gear` row at all**, no row carries any of the seven attributes, and no `interior_item_state`
row carries the import's note. The 65 notes are still only notes. B51 lifted a refusal, which
is not the same event as a write, and a reader of "gear import shipped" can take it for one —
so the distinction is written here rather than left to be inferred. Running
`trips gear import --apply` is the operator's, against a live interior.

The path itself was checked rather than assumed, on the same date and against a temp
database, never the deployment's: the body this side sends deserializes into interior's
`Item`, and the row comes back `kind = gear` with all seven attributes. What has no test on
either side is the two capabilities agreeing — interior's handler is not exercised by any
trips test and cannot be, because a capability may not depend on a sibling's crate.
`interior_client`'s own tests pin the half trips owns: the body is one flat object, because
interior flattens the item and a nested one would be a 422 per proposal.

## Why a capability

Trip planning is a bounded domain with its own persistent state. Putting plan items in
`transit` would make transport own events; putting them in `scouting` would make
discovery own journeys. The dashboard remains a shell: it coordinates calls and renders
the workspace, while this capability owns the data.

## Related tools and why Axon is not all of them

Axon is useful when a plan should remain local-first and connect personal context across
`trips`, `transit`, `scouting` and the vault. It should still point to a narrower or more
mature tool when that tool already owns the immediate job:

| Tool | Individually good at | Relationship to Axon |
|---|---|---|
| [TREK](https://github.com/liketrek/TREK) | Self-hosted, real-time group planning with invitations, reservations, shared costs, packing lists, documents and a PWA | A serious whole-product alternative for collaborative trips. Axon should exchange neutral exports later rather than recreate TREK's group surface |
| [TripIt](https://www.tripit.com/web/free) | Turning forwarded booking confirmations into one itinerary | The strongest reference for automatic intake. It remains a cloud handoff because using it sends travel and booking data to an external service |
| [Besser Bahn](https://github.com/chuk-development/Besser-Bahn) | Android-first live rail assistance, connection predictions, disruption alerts and split-ticket booking links | Better during a running rail journey; Axon keeps the result in the broader trip plan |
| [BetterBahn](https://betterbahn.de) | A focused, inspectable split-ticket workflow | Product and algorithm inspiration. Upstream currently provides local/self-hosted use, not an official hosted calculator |
| Plan Bahn (`troyriverabusiness/msg-code-create`, gone) | The earlier Vue, FastAPI and LangGraph take on agent-assisted rail planning | Lineage only, and no longer reachable in any form. Upstream was already unavailable at the 2026-07-29 re-check; the one vendored copy, which lived inside the Event Horizon repository, went with that repository on 2026-08-17. Named without a link because the URL resolves to nothing, and the dashboard does not present it as a handoff |

The dashboard renders the useful subset as contextual disclosures: planning shows TREK and
TripIt; connection search shows Besser Bahn and BetterBahn. MapLibre, OpenFreeMap and
Wikimedia are providers used by Axon, so their attribution stays next to the data they
render rather than being mislabeled as alternatives.

## Obsidian sync boundary

Obsidian integration belongs at this capability's contract, not in the Svelte dashboard
and not as direct database access from a vault script. The import slice scans and
previews existing trip notes, then materializes a selected note as a plan with its source
reference preserved.

### The export slice, shipped

This paragraph specified it a month before anything implemented it, and PRD Q47
(2026-08-27) then made it a requirement rather than a roadmap item: `trips_plan_items`
holds 21 rows that exist nowhere else, and a capability holding only-copy rows projects
them to files. Every plan is now one Markdown file under `Resources/Axon/Trips/`, in the
vault named by `<overlay>/config/trips.json`.

The four points above survive with one correction. Point 3 said "a marked Axon-owned
section", which is right for a note a human already writes and wrong here: no human note
exists per trip, so PRD Q31 (2026-08-23) ruled this pattern B — a whole generated file,
in the one vault folder a human never edits. So the file carries an
`` header instead of region markers, and a file at that path
without the header is refused rather than overwritten, which is Q31's promotion path.
Points 1, 2 and 4 hold: one file per plan, `axon_trip_id`/`axon_schema`/`axon_revision` in
frontmatter, and every path and personal value in the overlay.

`axon_revision` is the plan's `updated_at` — the same token `expected_updated_at` uses,
rather than a second notion of "which revision is this" that nothing could check against.

The mechanism is shared, per Q49: `libs/markdown-root`'s `projection` module owns
placement, containment, the header and the do-not-write-identical-bytes rule.
`src/projection.rs` owns only what a plan looks like.

What it is for is reconstruction, not reading (Q46: B14's projections are safety copies,
not reading surfaces). Each item's payload is written out verbatim as JSON, because an
unchosen fare cannot be re-queried later at yesterday's price and prose about a booking
reference does not restore a booking.

Two triggers, and both are needed:

- The server re-exports after **any** successful non-GET request, as a layer rather than
  a line in nine handlers — a tenth mutation route added later would otherwise stop
  projecting silently. A failure is logged and the request still succeeds: refusing a plan
  write because the vault is unreachable trades a durable row for a missing file.
- `trips export-vault` (`--dry-run` to look first) is the copy a human can take when the
  server is down, which is exactly when it matters.

Both run the same function, so both also sweep: a projection whose plan was renamed,
deleted or never existed is removed, because a stale safety copy is the one somebody
would restore from. Archived plans **are** exported — `list_plans` hides them from the
dashboard, and letting that filter reach the safety copy would make archiving a silent
data loss.

Two-way synchronization comes only after that export shape has been used. It should accept
only explicit fields such as notes, places and links, validate them into `PlanItem` data,
and record a conflict instead of silently choosing between two changed revisions. Axon's
database remains authoritative for trip identity, stages and item IDs; the vault remains
the authoritative writing surface for human notes.

## Build

```bash
cargo test -p trips
cargo build --locked --release --bin trips-server
```
