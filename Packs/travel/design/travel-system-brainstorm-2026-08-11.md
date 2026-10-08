# Travel system — brainstorm roadmap

_Generated 2026-08-11 by a 13-agent pass over `~/Developer/Axon`: 4 code probes, 5 ideation lenses (booking, learning loop, integrations, agent-API, cloud AI), 3 adversarial critics, 1 synthesis. 45 raw ideas in, 20 survived._

**Every claim below cites a file the agents actually read this session.** Where something says "I read it this session", that was the sub-agent, not you or me — spot-check before acting on a line that surprises you.

## The finding

The travel system's actual intelligence — the 75 km candidate match, the six-state triage, the leg-to-rail-query construction — lives in a 3440-line Svelte file, while the capabilities underneath it accept any JSON as a plan item, let a stage claim `booked` with nothing behind it, and return HTTP 200 with an empty array where real data exists. Nothing here is missing a feature; what's missing is contracts an assistant can read and a transport it can drive, and the four cheapest fixes in the whole set are honesty fixes, not new capability.

## Do first — the four honesty fixes

### 1. axon capability call learns PATCH/PUT/DELETE and stops eating error bodies

**Owner:** the `axon` script at the repo root (tooling, bash 3.2) · **Effort:** ~1 hour

`capability_call` in the repo-root `axon` entry point (now `sjel`) dispatches `case "$method" in get) … post) … *) exit 1`, and both arms use `curl -sf`. I read it this session and confirmed it. So PATCH /api/plans/:id, both trips DELETEs and calendar's idempotent PUT /api/entries/external are unreachable through the only sanctioned non-browser transport — and `-f` throws away the response body, so an agent that sends an invalid plan gets exit 22 and never sees the `{"error": …}` the capability carefully wrote. Add three method arms, swap `-sf` for `-s --fail-with-body` (installed curl is 8.7.1, well past the 7.76 minimum — checked), keep the non-zero exit, update both usage strings.

**First slice.** The three method arms plus --fail-with-body, usage strings at axon:31 and :137 updated in the same commit.

**Why now.** Every other agent-facing item in this roadmap is harder to build and impossible to test by hand until this lands. Half of trips' lifecycle is currently unreachable and every validation error is deleted in transit.

**What the critics changed.** All three critics kept it; two promoted it to first in the whole set. Cost critic added the curl-version check, which I ran: 8.7.1, supports the flag.

### 2. Split-ticket segments must say which train they were priced for

**Owner:** capabilities/transit (hafas.rs, the solver that already owns split logic) · **Effort:** a day

hafas.rs:225-262 prices each segment with a *fresh* search_connections per stop pair and takes `journeys.first().total_price` — never checking that the returned journey is the train the traveller will sit on. Split points are section boundaries only (halte[0] and halte.last() per non-WALK section), failed pairwise queries vanish silently into the DP via `if let Ok(...)`, and `savings` reports 0.0 rather than unknown when the direct fare is missing. The dashboard renders that as `Ticket 1..N` with a savings percentage. Emit `train_match: exact|partial|unknown` per segment from data the solver already holds, report unpriced pairs instead of routing around them, change `savings` to Option<f64>, and add a chain-level `confidence: partial` flag.

**First slice.** train_match per segment plus the savings type change, in one commit, with one fixture test per case (exact, partial, unknown, chain-with-unpriced-pair).

**Why now.** This is the only defect in the stack that costs money rather than time: buy three of four tickets in an unsound chain and you have a broken itinerary. It is also the hard precondition for any per-segment buy surface.

**What the critics changed.** Boundary and prior-art both raised it from 'peer' to 'prerequisite'; cost critic promoted it above the deep-link idea. Added: the chain-level confidence flag, so the buy surface can refuse to render N links rather than relying on the reader spotting a per-segment badge.

### 3. Delete the transit endpoint that lies

**Owner:** capabilities/transit (its own store, its own already-served route) · **Effort:** ~1 hour for the 501, a day for the real read

capabilities/transit/src/server.rs handle_list_trips — I read it this session — opens the store, comments 'TransitStore doesn't have a list_all_trips method yet. Return count for now.', and returns `{"count": n, "trips": []}` with HTTP 200. To a human reading Known gaps that is honest scaffolding; to any programmatic caller it is indistinguishable from 'there are no trips'. The data is right there: list_session_trips already reads trips with legs, ordered price NULLS LAST. Generalise it to `list_trips(session_id: Option<&str>, limit)` and serve real rows.

**First slice.** Ship the 501-with-a-body naming what is missing today, within the hour, so the lie stops. `list_trips` plus the ?session_id filter follows in the same day.

**Why now.** A confident wrong answer is the worst failure mode an API has, and this is the only one in the repo. It also makes the fuzzy `transit plan` session invisible to every non-shell caller.

**What the critics changed.** Kept by all three. Cost critic added the split: land the 501 immediately regardless of when the store method lands. POST /api/plan stays deferred — putting a long-running fan-out behind HTTP is a separate and larger contract question.

### 4. Typed plan-item payload variants, so an agent can write an itinerary item

**Owner:** capabilities/trips — it owns plan_items, the CHECK constraint and the schema file · **Effort:** a day

schemas/trip-plan.schema.json declares `"payload": {"type": "object"}` — unconstrained — and store.rs:571 validates only that item_type is one of eight strings before serialising whatever arrived into TEXT. The UNIQUE (plan_id, item_type, external_id) upsert is what makes writes idempotent, but the namespacing that gives it meaning (`calendar:`, `scouting:`, `wikipedia:`, raw journey id, raw event URL) exists only in five functions inside a 3440-line Svelte file. An agent's invented shape gets a 201. Make planItem a discriminated union keyed on item_type with an external_id `pattern` per type, validate the declared variants in create_item, keep unmodelled types permissive and say so in the schema.

**First slice.** Three variants: journey, event, and option_set (the offered-but-rejected set, folded in from the choice-capture idea so it arrives typed instead of as a sixth undocumented dashboard convention). Everything else stays permissive. Audit the dashboard's five existing write paths against the declared variants first — an hour of reading — so a narrower variant does not break the travel page on deploy.

**Why now.** His stated priority is an assistant that can drive this API, and this is the single thing standing between an agent and a valid plan-item write. It is also where the option_set capture has to land, because those rejected fares are not requeryable at yesterday's prices — waiting means the eventual reader is born with an empty table.

**What the critics changed.** Kept by all three. Boundary critic folded option_set in here and asked for a 400 naming the field on rejection, which is also why the general error-code taxonomy got killed. Cost critic added the pre-audit of the five existing write paths.

## High leverage

### 1. Calendar syncs booked trip stages back into entries

**Owner:** capabilities/calendar — it owns entries, the trips HTTP client, the base URL and the ledger · **Effort:** a day

Calendar can turn clustered events into a trips plan — server.rs:758-806, with config.trips_base_url, a 20s client, a per-row GET /api/plans/{id} probe and an idempotence ledger. Nothing runs the other way, so a plan whose stage is `booked` produces no calendar entry, POST /api/verdicts calls that week free, and GET /api/windows offers it as a feasible travel window. The system can propose a trip on top of a trip it created. Add POST /api/trip-plans/:plan_id/sync: read the plan over trips' public API and write one all-day entry per stage in option_selected or booked through calendar's own idempotent PUT /api/entries/external, commitment committed/planned, external_id `trip:stage:<stage-id>`. In the same pass, read booking plan items and emit a kind:"deadline" entry for any free_cancellation_until.

**First slice.** Booked stages only, all-day entries from stage.date, no deletion path, plus one button on the travel page beside the stage status control so the endpoint has a caller. Verify by materializing a draft, marking a stage booked, syncing, and confirming /api/windows stops offering that week.

**Why now.** It is a live logic bug with a user-visible wrong answer, and every expensive part of the implementation already exists in calendar.

**What the critics changed.** Kept by all three, and it absorbed the competing trips→calendar push wholesale — that direction would have given two capabilities an HTTP client for each other and put the deadline entry in a second external_id namespace with no ledger. Cost critic added the button, since an endpoint nothing invokes is not shipped.

### 2. Persist the punctuality histogram, then ask it the buffer question

**Owner:** capabilities/punctuality owns the statistic and the storage; capabilities/transit owns the per-transfer composition over the HTTP contract it already uses · **Effort:** a day

stats.rs builds a 128-bucket delay histogram per (station, train type, hour, weekend) cell and exposes share_at_least(minutes) exact for any threshold. store.rs:98-112 then persists seven scalars and drops the array — which is why the only exceedance anyone can ever ask about is six minutes, and why transit's own punctuality.rs:100-107 says 'Transfer risk is a different quantity and this data cannot produce it'. Persist the bucket array (~512 bytes × ~400k cells, order 200 MB), accept an optional at_least_minutes per stop on POST /lookup, return share_delay_at_least under the same MIN_SAMPLE=30 null rule.

**First slice.** Punctuality only. Persist the buckets, add the parameter, re-ingest, then verify by curl that at_least_minutes:6 reproduces the stored share_late_6 to the float. Nothing in transit changes and no ranking moves until that equality holds. Second slice: transit computes scheduled slack per transfer from legs[] and attaches transfer_risks[], with the four honest caveats in the field's doc comment (ignores the departing train's own delay, ignores platform-change walking time, ignores that a missed connection reroutes rather than ends the trip, measures arrivals only).

**Why now.** It converts a documented impossibility into a measured number without touching HAFAS, credentials or any external API, and it is the cheap alternative that has to be lived with before the expensive transfer-survival pass is justified.

**What the critics changed.** Kept by all three; absorbed the model-written-sentence variant, whose own author conceded the record may read fine as a table. Cost critic asked for the full bucket array rather than threshold-specific columns, so a new question is a query and not a re-ingest of 4.2 GB of parquet.

### 3. Ticket extraction gets an HTTP caller

**Owner:** capabilities/transit owns the parser; capabilities/trips owns the resulting booking record · **Effort:** hours

`transit import <file>` parses a PDF or .eml — trains, stations, times, price, confirmation number, ten passing unit tests — prints pretty JSON and forgets it: main.rs:325-352 never constructs a store, and confirmation_number has no column anywhere. Put the existing function behind POST /api/tickets/extract, returning today's ExtractedTicket verbatim. Later slices: a dashboard drop-target that shows every parsed field for review and, on explicit confirm, writes a booking plan item.

**First slice.** One axum handler over extract_from_bytes, one route-manifest line, verified with curl against a real DB ticket PDF. No storage, no UI.

**Why now.** It is the cheapest possible probe of an unknown everything downstream depends on: the ten tests run against synthetic strings, so nobody knows how the parser behaves on an actual ticket, and an afternoon answers it. Never auto-write — extractor.rs:180-199 emits one leg per train number all sharing origin, destination and times, assigns dates positionally, takes the first price match, and fabricates <year>-01-01 when nothing parses. That is review material behind a human and wrong behind a scanner.

**What the critics changed.** Three ideas proposed this identically; merged. Prior-art critic's boundary sentence is the one to keep: transit is a lib crate with `pub mod extractor` so comms *could* link it and must not — punctuality.rs:1-5 states the rule, a capability depends on another's contract, not its code. The Gmail-sweep wiring stays deferred until real tickets prove the parse.

### 4. Make `booked` mean something: a booking artifact in trips

**Owner:** capabilities/trips · **Effort:** a day

stage.status = "booked" is a string the API accepts with nothing behind it — update_plan validates title, destinations, date order and plan status, then takes the stages array wholesale, and delete_item removes a plan item without touching the stage that pointed at it. No order reference, no fare name, no refundability, no free-cancellation deadline, no ticket file exists anywhere in the schema. Add `booking` to the plan_items CHECK, give it a constrained payload via schemas/trip-booking.schema.json (provider, order_ref, fare_name, refundable, free_cancellation_until, amount_cents+currency, ticket_file_ref, traveler_name_present as a boolean rather than the name), then refuse a `booked` stage whose selected_option_id has no matching booking item, and demote a stage whose booking is deleted.

**First slice.** The CHECK migration plus the schema file plus the server-side gate. No new endpoint, no UI — the write path is the existing POST /api/plans/:id/items.

**Why now.** It is the load-bearing dependency for the calendar deadline sync, the budget-versus-actual join and the outcome record. Ordering hazard: the dashboard's stage dropdown already offers 'booked', so land the item_type and schema before the refusal gate or you turn an existing UI action into a 400.

**What the critics changed.** Kept by all three unchanged. Cost critic flagged the consumer risk honestly — with no UI the write path is curl — but the gate has value with zero data entry, so it ships on that alone.

### 5. tools/travel.ts — move the caller, not the logic

**Owner:** tools/ — the declared TypeScript home, ~15 bun tools there already carry .test.ts siblings · **Effort:** a day

The triage that makes the travel page useful is 254 lines of pure TypeScript in dashboard/src/lib/travel/travel-candidates.ts: the haversine match, DESTINATION_MATCH_RADIUS_KM = 75, the deterministic tiebreak, the six-state machine. No capability computes any of it, and svelte.config.js is adapter-static with an explicit file-copy-deployment rationale, so a SvelteKit server route is not available. Add a bun CLI that imports those same pure functions and resolves capability base URLs through tools/capability.sh registry.

**First slice.** `axon travel candidates --json` reproducing assessTravelCandidates over live scouting, trips and calendar, verified by diffing its output against what the page renders for the same data. Move travel-candidates.ts to a shared module both the dashboard and tools/ import rather than having a bun tool reach into the dashboard's source tree.

**Why now.** It makes the composition agent-reachable without moving any state, adding any endpoint, or reopening trips/README.md:37-38 — and it is the empirical answer to whether a capability-side context endpoint is needed at all.

**What the critics changed.** Kept by all three, and it is why both rival proposals died. I verified the import surface myself: travel-candidates.ts imports only `import type` from ../api, which is erased at runtime, so a bun tool can import it cleanly. The one failure mode to guard is a divergent second copy of the 75 km rule — import, never reimplement.

### 6. Optimistic concurrency on plan writes

**Owner:** capabilities/trips, internal to its own write path · **Effort:** hours

PATCH /api/plans/:id accepts `stages` wholesale, so the only way to change one stage is read-modify-write of the whole array — which is exactly what updateStage() does, with a lost-update race the API does not require. A browser holds that read for milliseconds; an agent holds it across turns while it calls transit and reasons. Accept an optional expected_updated_at on PATCH and DELETE, compare inside the existing transaction, 409 with `{error, code:"stale_plan", current_updated_at}` on mismatch.

**First slice.** PATCH and DELETE in one commit — the check is the same three lines and a stale delete is worse than a stale patch. Absent field keeps today's behaviour so nothing in the dashboard breaks.

**Why now.** updated_at is already required on the plan schema, the pattern is already proven three times in-repo (comms' 409-on-hash-mismatch, calendar rejecting changed revisions, finance's preview→confirm), and it is the generic cheap version of the stale-read protection the booking-intent choreography was reaching for.

**What the critics changed.** Kept by all three; cost critic pulled DELETE forward into the same commit and named it the replacement for the killed booking-intent hash routes.

### 7. Close the ungated cloud path in libs/inference

**Owner:** libs/inference · **Effort:** hours

ResolvedRole::embed() and ::rerank() are transport-agnostic and consult no cloud policy — has_cloud_policy() is only ever called from comms' cloud handlers. So calling role.embed() on an https role reaches a cloud provider with no preview, no redaction, no approval, no budget check and no ledger entry. Make both refuse a non-loopback endpoint unless the same policy chain that gates analyze() has passed.

**First slice.** The refusal, and nothing else. No schema change, no table moves.

**Why now.** It is present-tense and reachable today, not a hypothetical. It costs nothing while every configured embedding role is local, and any future travel candidate-scoring feature reaching for cloud embeddings would otherwise be building on a path that bypasses the entire review discipline.

**What the critics changed.** Split out of the shared-cloud-ledger proposal by both critics who touched it — the double-spend half is a bug with no trigger (no surviving idea declares a second cloud role), this half is a live bypass. The ledger migration becomes a written precondition in the decision record instead.

### 8. A planned figure in trips; finance keeps every actual cent

**Owner:** capabilities/trips owns the intended figure; capabilities/finance keeps the actuals and the journal · **Effort:** hours

finance already validates axon-trip-id, writes it into the hledger posting, and returns per-plan TripSpendingSummary (personal spend, gross outflow, reimbursed, outstanding) from GET /api/dashboard. trip-plan.schema.json has no monetary property at all, so 'what did I mean this to cost' has no home and the two halves of the answer are one HTTP call apart and never compared. Accept budget_cents and currency on PATCH, render budget/spent/outstanding beside each plan — plus a line for any trip_spending row whose trip_id matches no plan.

**First slice.** budget_cents + currency: one migration, one schema property, one header field. Nothing calls finance yet.

**Why now.** The orphan-row line is worth more than the budget field: validate_reference (allocation.rs:139-152) checks only that the id is a bounded symbolic string, so a typo'd axon-trip-id is permanent and currently invisible, and this is the only cheap instrument that surfaces it. Do not make finance validate the plan id at write time — its README argues deliberately for the opaque identifier.

**What the critics changed.** Kept by all three unchanged.

### 9. Where he actually goes back to — and the place-identity bug it exposes

**Owner:** capabilities/trips — pure projection of its own rows, no new table · **Effort:** hours

PlaceField.commitTypedPlace() slugifies typed text into `place:<slug>` with kind city and null coordinates whenever the user does not pick a transit.suggest station. So the same city typed twice differently is two places, and every coordinate-dependent behaviour downstream — the 75 km candidate match, the map, nearby places — degrades silently. GET /api/places on trips, computed on read over its own plans rows, returning visits/first/last per place plus a merge_candidates block listing distinct place ids whose names normalize to one string.

**First slice.** The merge_candidates detector alone: group destinations by normalized name, report every set of distinct ids sharing one. A handful of lines, and it tells you on day one how bad the identity problem is. Visit counts and create-form seeding follow only if the counts turn out to mean anything.

**Why now.** Best value-per-hour in the learning lens, and unlike everything else there it has a consumer on the day it ships.

**What the critics changed.** Boundary and cost kept it; prior-art split it so the bug detector ships alone and the convenience half waits.

### 10. Write down the four travel seams that should not be built

**Owner:** capabilities/trips/decisions/ — its README already carries the alternatives table · **Effort:** hours

One decision record under capabilities/trips/decisions/: (1) no Axon-side leave-home scheduler — home-assistant here is a README and a service.toml with no src/, and a departure trigger belongs in an HA automation reading the calendar entry the sync above now produces; (2) Axon never executes a booking — hafas.rs:32-41 states the endpoint is ungated only because it looks like browser traffic and there is no ToS contract to honor, and DB MCP servers that exist all wrap timetable data and book nothing either; (3) no flight search — zero matches for flight/airline/airport across transit, mature flight MCP servers are already reachable from his assistant, and `airport` is already a placeRef kind with no producer; (4) no cloud AI for itinerary ranking — the cloud path validates source as feed|mail in four places, has one TASK_VERSION and no shared budget ledger, and deterministic-entity-redaction-v2's [person] rule only fires after a salutation, so an itinerary (all proper nouns, no salutation, six-char PNRs below both thresholds) would pass through essentially intact. Plus the two preconditions: the day a second capability declares a cloud_* role, the attempts ledger must move out of comms' schema first.

**First slice.** The four entries, each three or four sentences with the file reference that makes it checkable. No code.

**Why now.** Two open issues in the whole repo and no travel roadmap means these rejections currently live only in someone's memory and get relitigated every time someone reads the travel page. The convention explicitly permits records for what was deliberately chosen against.

**What the critics changed.** Kept by all three; it absorbed the no-automated-purchase record from the killed booking-intent idea and the why-the-redactor-is-wrong-for-itineraries analysis. Boundary critic's nit: state entry (3) as a rejection with a factual note about the unused airport kind, not as a roadmap line.

### 11. Derive request/response schemas into GET /routes with schemars

**Owner:** libs/route-manifest owns the mechanism; capabilities/trips is the first consumer · **Effort:** a weekend

libs/route-manifest serves {method, path, summary} and nothing else; its own doc comment concedes that required query parameters have to be smuggled into an English sentence, and undeclared_routes is one-directional text matching on `.route(` — which is why punctuality's /stations summary describes one of that endpoint's two modes with a green test suite. Add optional request_schema/response_schema derived by schemars from the structs serde already deserializes, emit them in /routes, and extend the compile-time test so a POST or PATCH declaring no request schema fails the build.

**First slice.** trips only: schemars derive on CreatePlan and CreatePlanItem, two request_schema entries, the test tightened for trips' router alone. Other capabilities untouched.

**Why now.** His stated priority is agent-drivability, and this is the difference between an agent discovering that POST /api/plans/:id/items exists and knowing what to send it. Derived rather than hand-written, and explicitly no checked-in openapi.yaml — a second home for a fact the Rust structs own is exactly how the /stations summary got to be wrong.

**What the critics changed.** Kept by all three; it absorbed and killed the rival proposal that would have pointed a $ref at schemas/trip-plan.schema.json, which describes the stored plan and not the write bodies. Two costs to budget inside the weekend: manifest()'s output shape changes for every capability serving /routes, and the opaque serde_json::Value payload needs an explicit schemars annotation.

## Worth it later

### 1. Every stored price says when and under what assumptions it was observed

**Owner:** capabilities/transit · **Effort:** hours for the recording; separate work per profile parameter changed

record_journey's ON CONFLICT refreshes total_price and deliberately leaves created_at alone; there is no priced_at, updated_at or TTL anywhere, so `plan --show` prints ten-week-old fares indistinguishably from fresh ones. Separately, fahrplan_payload hardcodes KLASSE_2, one ERWACHSENER with KEINE_ERMAESSIGUNG, anzahl 1, and deutschlandTicketVorhanden false on every query — so every fare ever stored is a second-class single-adult no-discount fare that does not record that it is one. Add an append-only transit.trip_prices (trip_id, observed_at, total_price, pricing_profile) series and a PricingProfile struct read from transit's overlay config with today's values as defaults.

**First slice.** Both migrations together, byte-identical HAFAS requests on day one, and observed_at surfaced in `plan --show` so a stale fare says it is stale.

**Why now.** The recording half is unconditionally right and cheap. Changing any profile value is live reverse-engineering against an undocumented private API — one parameter per commit, each with a captured before/after response committed as a fixture, starting with deutschlandTicketVorhanden because it has a testable prediction: regional legs with is_regional true should stop returning NULL.

**What the critics changed.** All three critics split this. The watcher (`transit watch --session`, a scheduled fare-watch job) is cut until he has re-run a stored session by hand twice; the flip-the-fare-parameters half is cut into per-parameter commits.

### 2. Ride-level actuals from the parquet punctuality already downloads

**Owner:** capabilities/punctuality — same dataset, same licence, same file layout, one grain finer · **Effort:** a day

punctuality's ingest projects five columns out of seventeen. The raw parquet cache in the overlay carries train_number, line_number, train_line_ride_id, train_line_station_num, station_name, arrival_planned_time, arrival_change_time, departure_planned_time, departure_change_time and final_destination_station — the probe read the footer of data-2026-03.parquet directly. So DB's own published history already answers 'how late was ICE 611 at Bonn Hbf on 14 March' and it is decompressed past and discarded on every ingest.

**First slice.** CLI only: `punctuality ride --train-type ICE --train-number 611 --date 2026-03-14 --eva 8000044` printing the matching stop rows as JSON. One projection constant, one filtered pass, no schema change, no HTTP route.

**Why now.** It is the prefill source for the outcome record and the calibration source for any transfer work. The honesty requirements are the feature: publication lags a trip by up to five weeks, collection is 98.92% complete with 196 named missing hours, and the raw dir is a prunable cache — so an absent row is a stated null, never a guess.

**What the critics changed.** Kept by all three. Kill criterion is built in: run it against a journey he actually took and see whether the row is there.

### 3. A trip outcome record, filled by curl before it is filled by a form

**Owner:** capabilities/trips — the outcome belongs on the same row as the intent it measures · **Effort:** hours for the endpoint

StageStatus::Completed exists in store.rs:49 and in the UI dropdown, and nothing anywhere sets it; the past/upcoming split is pure date arithmetic. POST /api/plans/:id/outcome accepting per-stage optional actuals — actual departure and arrival, transfer_made per transfer, actual_price_cents, one free-text line — refusing any stage with no selected_option_id, because with nothing chosen there is nothing to compare against and the record would be a hoard.

**First slice.** The endpoint alone, filled by hand after the next two trips. No form, no prefill from punctuality, no prefill from finance.

**Why now.** The kill criterion — if he does not fill it in twice, the whole learning lens is answered — cannot be run cheaply if running it costs a weekend of Svelte form work first.

**What the critics changed.** Boundary and prior-art kept it whole; cost critic stripped the form out of the first slice, which is what makes the kill test affordable.

### 4. A grounding verifier: reject any claim the document does not contain

**Owner:** whichever crate holds the bounded-document code (comms today) · **Effort:** a day

cloud-content-analysis.schema.json requires source_text on every important_dates entry — the model must quote what it read the date from — and cloud_dispatch.rs only length-bounds it. Nothing checks the quoted span appears in the document, so the repo's one anti-hallucination mechanism is satisfiable by inventing the quote too. Add verify_grounding(document, claims) as a pure function and wire it in immediately after the field-by-field re-bounding that already distrusts the response.

**First slice.** important_dates[].source_text only, in comms only, as a `grounded: bool` on the stored analysis and a badge in the existing review modal. No refusal, no schema change — measure the rate across at least twenty real analyses first and record it.

**Why now.** Tolerable for a mail summary, not tolerable the moment a model output carries a price or a departure time. The measured number is the argument for or against every other cloud idea here — write the prediction down before running it.

**What the critics changed.** Kept by all three; framed as an experiment with a written-down prediction rather than a feature, and it becomes the gate on any later model path.

### 5. Geo-ranked station resolution, model only for what distance cannot separate

**Owner:** capabilities/transit · **Effort:** hours

resolve_candidates takes the top N stations bahn.de returns, in bahn.de's own order, so 'Frankfurt' silently fans a whole session's queries at whichever station came first. transit's own Station struct already carries latitude and longitude from /reiseloesung/orte and the code ignores them. Rank the returned stations by great-circle distance from a reference point (the origin, or a --near flag), keep take-the-top as the fallback when coordinates are absent, record the basis the way scouting's event_route records its basis.

**First slice.** Distance ranking using the coordinates already in the response. No inference call.

**Why now.** It fixes disambiguation and the README's named geo-radius gap at once, with arithmetic that is auditable where a model's index pick is not. A closed-set model selection (an integer in [0,len), so a hallucinated station is structurally impossible) is a much smaller claim to justify afterwards, for the cases distance genuinely cannot separate — Hbf versus Süd for a stated purpose.

**What the critics changed.** Boundary and cost kept the model version as the safest model use in the set; prior-art pointed out the deterministic input already exists. Merged: deterministic first, model as a strictly smaller follow-on.

### 6. Fix the `day` column nothing maintains

**Owner:** capabilities/trips · **Effort:** hours

plan_items has a `day` column and an index on (plan_id, day, created_at), and every write path copies day from the source rather than deciding it: a saved journey gets stamped activePlan.date_start even when its stage has a different date, and every saved place gets day: null with no path in the system that ever fills it. Use the owning stage's date, and add PATCH /api/plans/:id/items/:item_id {day} so an undated item can be moved by hand.

**First slice.** Exactly that, and stop. Use it for one real multi-day trip.

**Why now.** It is a straight bug fix on a column an index already exists for, and it is the same endpoint any later automatic day-planning would call — so it surfaces whether the bag-of-items problem is even real before anything else is built.

**What the critics changed.** All three critics picked the deterministic half and cut the model permutation, which is a weekend of careful machinery for a job that arises once or twice a year in his travel pattern.

### 7. Realtime and cancellation fields survive the HAFAS parse

**Owner:** capabilities/transit · **Effort:** hours of work, unbounded calendar time waiting to catch a live disruption

parse_journeys_from_response reads halte[].abfahrt.sollzeit with an istzeit fallback and discards everything else, so scheduled and actual collapse into one field and a cancelled train is invisible. Split scheduled_departure/realtime_departure and the same for arrival, add a cancelled flag, persist them on trip_legs.

**First slice.** The parse only, with every field name taken from a committed capture of a real delayed or cancelled train and cross-read against db-vendo-client's parsers so a missing field is distinguishable from a misnamed one. Nothing consumes them yet.

**Why now.** The repo has already been burned once by exactly this — "id" versus "tripId" collapsed five real journeys into one stored row with cargo test green throughout — so fixture-first is not optional here.

**What the critics changed.** All three cut the second half: a re-check horizon promoting disruptions into tasks needs a polling loop transit does not have and a way for transit to learn what a booking is. That is the on-train-companion product upstreams.toml already declined.

### 8. The Obsidian export slice trips specified a month before its primitive shipped

**Owner:** capabilities/trips — its own state, its own configured vault folder, path in the overlay · **Effort:** a day

trips/README.md:91-105 specifies it in four numbered requirements and it has never been built; libs/markdown-root/src/region.rs exists with find/apply and a Conflict type, and its own header notes trips specified this in prose first; finance/src/obsidian.rs::write_block is a working reference for the same conflict rule. POST /api/export/obsidian/:plan_id writing one note per plan, regenerating only inside an axon:begin/end fence, returning 409 naming the note on a human edit inside the markers.

**First slice.** Single plan, frontmatter plus one region, conflict as a 409, no /all and no deletion. Prove the write-preserve-reimport round trip against a real note reconciled by plan.source.reference.

**Why now.** Gated: a plan he typed is not news to him, so this is worth building once the outcome record above has produced something machine-derived to put inside the fence — chosen versus offered, planned versus actual, cost against budget. Everything outside the fence stays byte-for-byte, and the human half of a retrospective is never generated.

**What the critics changed.** Boundary and prior-art kept it as the clearest adopt-what-exists item in the set; cost critic added the gate on there being content worth exporting.

### 9. A Packs/travel skill carrying the ordering and the traps

**Owner:** a new Packs/travel, following the Packs/home-automation skill-drives-an-HTTP-API pattern · **Effort:** a day

One skill whose reference file holds what no route manifest can: the calendar-windows → transit-plan → trips-plan ordering, which capability answers which question, and the traps — punctuality zero-pads EVA numbers to eight digits while transit returns them unpadded, so joining without normalize_eva returns zero rows and looks exactly like 'no data'; scouting's travel_candidate is a response field, not a route; GET /discover mutates state. Point at the capability READMEs for each trap rather than copying them.

**First slice.** SKILL.md plus one reference file with the three-step ordering and the EVA-padding gotcha. No tooling, no scripts.

**Why now.** Ship it after the CLI fix and the /api/trips fix, because a skill that teaches a broken dispatcher and an endpoint that lies teaches the wrong thing. The ordering is stable across those changes; the exact call syntax is not.

**What the critics changed.** Prior-art and cost kept it; boundary critic trimmed most of the proposed content as a third copy of facts that already have homes, and cut the separate MCP decision record — capabilities/printing/README.md already declines an MCP wrapper as 'a wrapper of a wrapper', and a second copy is exactly the open-ended decision doc the rules forbid.

### 10. On-device OCR, so a photo of a ticket stops being a hard error

**Owner:** capabilities/foundation-models gains the route and stays stateless; capabilities/transit keeps the extraction · **Effort:** a day

transit's extractor hard-errors on every image: 'Image files require OCR - not yet supported.' A grep for OCR, Vision, VNRecognizeText or tesseract across the repo returns nothing. capabilities/foundation-models is the only Swift process here; give it an OCR route over Apple's Vision framework returning recognized text with per-line boxes, and route the extractor's image branch through it into today's existing regexes.

**First slice.** The OCR route (one more handler in a 290-line main.swift) plus swapping the extractor's hard error for a call to it. No model reasoning, no new schema.

**Why now.** It turns 'screenshot of a ticket' from unsupported into as-good-as-a-PDF without a pixel leaving the machine. A bitmap cannot be deterministically redacted, so images stay blocked from the cloud tier — write that into the decision record. Naming nit: do not put it under /v1/, which implies an OpenAI compatibility it does not have.

**What the critics changed.** All three cut the second half — model-structured leg pairing with a HAFAS re-query gate is PDF layout parsing and live reverse-engineering of an undocumented API in one commit. Revisit only after the extract route has shown, on real tickets, how bad the regex parse actually is.

### 11. One pre-trip obligation rule, with a kill condition written first

**Owner:** capabilities/tasks owns the record; the composition edge owns the lead-time rule · **Effort:** hours

tasks has exactly one producer in the whole repo (the feed page promoting a mail) and its partial unique index on (source_capability, source_id) makes repeated promotion safe by construction. For each stage still in `planning`, one task 'Book <origin> → <destination>' due N days before stage.date, source_id `trip:plan:<id>:book-stage-<stage-id>`, fired by a button and never on plan create.

**First slice.** One rule, one button, one lead time as a named constant with a comment saying why it is not config yet — the dashboard is a static bundle and cannot read overlay config, and putting the policy in trips would give itinerary state the obligation rules it must not own.

**Why now.** Blocked on the open question below: tasks' README says a due date bounds a task rather than placing it, and calendar's KNOWN_KINDS says a deadline is 'a dated action or due date; visible evidence, never a time block'. Those describe the same object, and shipping producers into both gives him two inboxes with no rule for which to read. Kill condition: if two generated tasks in a row are closed as dropped rather than done, delete the rule instead of tuning the lead time. Passport and visa rules stay rejected — they need per-person and per-country facts that must never enter this repo, and a wrong visa deadline is worse than none.

**What the critics changed.** All three weakened it; prior-art critic found the tasks-versus-calendar-deadline collision the original missed.

## Ambitious

### 1. Transfer survival computed from ride-level history

**Owner:** capabilities/punctuality for the population statistic; capabilities/trips for his own instances — two owners because merging them would put personal history into a public-safe capability · **Effort:** weeks

With train_line_ride_id and train_line_station_num in the parquet, the actual arrival of every incoming ride and the actual departure of every outgoing ride at the same station in the same hour are both present, so whether a b-minute planned buffer would have survived is computable over millions of historical pairs. A second aggregate keyed (eva, incoming type, outgoing type, hour, weekend, buffer bucket), with POST /transfers under the same MIN_SAMPLE=30 null rule — and, separately, his own made/missed transfers from the outcome record as the calibration check.

**First slice.** Not the aggregate: `punctuality transfers --eva <code> --month <YYYY-MM>` for one station, one month, offline, printing survival by buffer bucket, purely to measure how thin the cells get.

**Why now.** Explicitly gated: do not start until the histogram work above has shipped, its share_delay_at_least has been read against transfers he actually made, and it has been found insufficient in a specific nameable way. Pairing arrivals against departures is order 10^9 synthetic pairs per month file and the cell key multiplies cell count by roughly 150 against a floor of 30 observations — most cells will be null.

**What the critics changed.** Kept by all three as genuinely different from the histogram work, with prior-art and cost both adding the gate so the expensive pass is never started on enthusiasm.

### 2. Natural-language trip intent to a draft that resolves nothing

**Owner:** capabilities/trips — a draft is pre-plan state, and it emits candidate names while resolving a name to an EVA stays transit's job · **Effort:** a day for the experiment

There is exactly one way to start a trip today: a form needing an origin picked from transit.suggest, destinations, dates and modes typed field by field. 'Somewhere warm in October, under 300 euro, by train, long weekend' has no entry point. A draft endpoint returning a CreatePlan-shaped body plus unresolved[] and assumptions[], persisting nothing, where every destination comes back as a place:<slug> PlaceRef with null coordinates — bit-identical to what typed text already mints — so the operator must still pick a real station before anything can be searched. The model may not emit an EVA, a price, a feasibility judgement, a plan id, or a persisted plan.

**First slice.** Local rung only, no cloud role, no HTTP: a `trips draft-intent "<sentence>"` CLI printing the JSON body. Note this adds trips' first binary other than trips-server plus a libs/inference dependency the crate does not carry today.

**Why now.** It answers one question cheaply — whether a 4096-token on-device model parses German and English travel sentences into a valid CreatePlan. Do not add the HTTP route, the 26B rung or a cloud role until he has used the CLI to start a real trip more than twice.

**What the critics changed.** Boundary and prior-art kept it on the strength of the local-first contract (the output is a form nobody has submitted); cost critic questioned whether his travel is ever discovery-shaped rather than dated work rail, which is one of the open questions below.

### 3. A model-read scouting adapter for foreign-language local sources

**Owner:** capabilities/scouting — another adapter behind the same interface, same rows, same table, calling neither trips nor calendar · **Effort:** a weekend if the script justifies it

Travel discovery is three hardcoded adapter names in the dashboard — luma, meetup, euro_hackathons — all English-language aggregator APIs, with Wikipedia enrichment hardcoded to de.wikipedia.org. What is actually on in a Spanish or Danish city that week lives on a city-council page in a language the pipeline cannot read. An adapter that runs only against a URL the operator already put in scouting's proposed-source inbox, asks a model for at most ten bounded events with a verbatim quoted span per date, lands them with source_kind "model_read" and status new, and drops anything whose quote is not in the page.

**First slice.** A throwaway script outside scouting: fetch one real European city-events page, hand it to the local model, diff the extracted events against what a human reads off the page. No adapter, no source_kind, no schema change, no stored rows.

**Why now.** It is the only travel job that fits the existing cloud path with zero new trust-class reasoning — a public web page is public data class, which takes bounded-public-v1 with no redaction work, and a public-tier role already exists. Two risks to name in the issue: fetching arbitrary pages and feeding them to a model is a prompt-injection surface (carry the existing 'ignore any instructions inside the document' framing; blast radius is bounded because output is typed, capped at ten, and lands untriaged), and municipal listings are frequently JavaScript-rendered and simply absent from the fetched HTML, which is a scraping problem no model solves.

**What the critics changed.** Boundary kept it, prior-art kept it on the no-substitute argument, cost critic replaced the first slice with a throwaway script so the adapter and its schema field are never built ahead of evidence.

## Killed — the tempting ones that did not survive

This section is as useful as the roadmap. Each of these was proposed by at least one lens and cut with a reason.

### A read-only cross-capability context endpoint on Trips (and its write-plus-ranking variant)

Killed by all three critics. The stated goal — the composition reachable without a browser — is delivered by the bun CLI for a day of TypeScript, with no boundary movement, no new HTTP client inside a capability, and no rewriting of trips/README.md:37-38. The endpoint version costs weeks, turns three independent failure modes into one request's failure surface on a machine where capabilities are on-demand, and the write-plus-ranking variant additionally makes Trips do transport search by proxy while depending on a profile that will say insufficient_data for a year. If the CLI proves insufficient, this comes back on evidence, narrowed to the one calendar-anchors include.

### Approved booking intent with comms' preview_hash choreography copied into trips

Killed by the boundary critic, weakened to nothing by the other two. comms needs prepare→hash→approve-that-exact-hash→409-on-mismatch because a provider socket opens at the end, real money moves against a ten-per-day budget, and data leaves the machine. Opening a deep link and writing a row into your own Postgres is none of those, so the ceremony guards nothing. The two things it actually wanted survive elsewhere: the 'Axon never buys' foreclosing call is rejection two in the decision record, and the stale-read protection is expected_updated_at, which costs hours and covers every plan write instead of one flow.

### Trips pushing a free-cancellation deadline into calendar

The observation is real — calendar ships `deadline` in KNOWN_KINDS and a Commitment::Committed that nothing sets — but the direction creates a cycle. Calendar already holds the trips HTTP client, the base URL and the idempotence ledger; trips has no outbound HTTP client at all (no reqwest in its Cargo.toml). Merged into the calendar-side sync, which reads booking items in the same pass it reads stages, through the same idempotent route, keyed by the same ledger.

### Stable error codes beside the prose

Killed by two critics and undercut by its own examples. All four cases in its first slice return 400, and 400 already tells an agent 'fix your input, do not retry' — the codes would discriminate between four flavours of the same next move. The case that would justify the mechanism, a retryable 500 versus a permanent one, is exactly the one the proposal excludes. It is also strictly downstream of the CLI fix: until --fail-with-body lands, the agent never receives the body it would be reading. Revisit when an agent is observed looping on a permanently invalid write.

### Naming existing schemas/ files from the route manifest

Killed by two critics on a false premise. The claim was that trips' three write routes are already described by schemas/trip-plan.schema.json — but that file describes the stored plan, and the write bodies are CreatePlan, UpdatePlan and CreatePlanItem, which differ (no id, no timestamps, different optionality). A $ref pointing at a file that does not describe the request is worse than no schema, because it would be believed. The schemars-derived version does it from the structs serde already deserializes; the one good mechanism here — a compile-time test that a declared body schema exists — moved there.

### A traveler-profile endpoint that learns his price/time/transfer tradeoff

Two critics cut it to nothing. It is a projection over rows that have not started accumulating, and at ten to thirty relevant bookings a year its honest answer is insufficient_data for at least a year. Building the weight machinery now is designing an analysis before there is anything to analyse. The capture itself survives — folded into the typed-payload work as an option_set variant — and the question can be answered with a query against those rows whenever the count clears a threshold he writes down in advance.

### Extracting the redactor into libs/cloud-review now, and structured-projection-v1

Not killed by vote — two critics wanted the verbatim move — but it has no consumer in this roadmap. Every surviving model rung is local-only or comms-only, so the move would be a lib extraction for a caller that does not exist, which is the rule the repo applies to itself. The genuinely valuable finding, that deterministic-entity-redaction-v2 is structurally wrong for an itinerary (no salutation so [person] never fires; a six-character PNR falls between the token and number thresholds), goes into the decision record now so nobody reaches for the wrong tool. The move happens the day a second consumer calls it.

### A shared cloud budget ledger, and a capabilities/cloud-review process

The arithmetic is right — the daily cap keys on provider_role, which names a real provider account, while the count reads only comms' own attempts table — but it is a bug with no trigger, because nothing in this roadmap declares a second cloud_* role. The half that is live today, embed()/rerank() reaching an https endpoint with no policy check at all, was split out and promoted. The ledger migration becomes a written precondition in the decision record: the day a second capability declares a cloud role, that migration lands first.

### answers = [...] in every service.toml

Weakened to a gate by two critics and then gated out. The diagnosis is exact — the descriptive first line of every service.toml is a bash comment toml.sh cannot read — but /routes already answers 'what does this capability do' at finer grain across 26 capabilities, and the Pack skill can carry ordering and traps a two-item array structurally cannot. If the Pack ships and agents still guess wrong about which capability owns which question, add the field to the five travel manifests in the same commit as that finding. A declared-but-unread manifest key is worse than a comment because it looks authoritative.

### Model-written risk sentences, model-permuted day plans, model-structured ticket extraction

Three separate model garnishes, all cut for the same reason: each sits on top of a deterministic record that has not been built or read yet. The transfer-risk sentence is a paraphrase of a table its own author said may read fine as a table. The day-plan permutation is careful machinery — structural verification, relative day indices, immutable transport items — for a job that arises once or twice a year in his travel pattern, before anything in the system has ever set `day` deliberately. The model ticket extraction bundles PDF layout parsing with live HAFAS reverse-engineering in one commit and calls it a weekend.

### Auto-intake of booking confirmations from the Gmail sweep

Deferred, not for boundary reasons — the comms→transit-over-HTTP path is clean — but because the parser is not fit for a scanner. extractor.rs emits one leg per train number all sharing origin, destination and times (Bonn→Köln→Berlin becomes two legs both labelled Bonn→Berlin), assigns dates and times positionally so a price-valid-until timestamp can win, takes the first price match rather than the total, fabricates <year>-01-01 when nothing parses, and sets ok:true unconditionally. Behind a human reviewing every field that is fine. Behind a scanner it silently writes wrong itineraries. Revisit after the extract route has been run against real tickets.

### A fare-watch scheduled job, and disruption alerts polling booked legs

Both cut for the same shape: a timer over a consumer that has not proved itself. Nothing in the repo shows he has re-run a stored trip session even once, so a watcher has nothing to watch; and a disruption re-check needs either transit learning what a booking is (the violation trips' README names) or a new sweep tool. upstreams.toml already records the position on the latter: 'Axon keeps the broader trip plan; it does not need to absorb every on-train interaction.'

## Open questions — yours to settle

**1.** Dated pre-trip obligations — one home or two? tasks' README says a due date bounds a task rather than placing it; calendar's KNOWN_KINDS says a `deadline` is 'a dated action or due date; visible evidence, never a time block'. Those describe the same object. If calendar wins, the sync endpoint emits both the travel-day block and the book-by deadline in one pass, the feasibility matrix sees them, and the tasks producer is never built. If tasks wins, the free-cancellation deadline has to move there too. Shipping producers into both gives you two inboxes and no rule for which to read.

**2.** Is the buy step allowed to stay manual forever? The bahn.de deep-link idea rests on a grammar nobody in this repo has verified exists — the booking URLs BetterBahn handles are `vbid` UUIDs bahn.de mints in its own flow, and its shipped tool *parses* such a link rather than constructing one. Thirty minutes in a browser settles whether a constructible from/to/time URL exists. If it does not: do you want the inverted flow (paste the bahn.de link back, Axon parses it into a booking record), or does the buy step stay a plain retype and Axon's job end at 'here is the connection'?

**3.** Is this system for dated work rail, or for discovery? Your travel is mostly DT trips and events with a known destination and a known date — for which calendar's windows plus scouting's candidates already answer the 'when am I free / what is on' half deterministically. The NL-intent draft, the foreign-language event adapter, and eventually the day-plan permutation only pay off in the other mode: 'somewhere warm in October, under 300 euro'. If that mode is roughly annual, three of the four model ideas here never earn their build.

**4.** Is a bun CLI an acceptable permanent agent surface, or does an assistant need capability-side endpoints? Everything in the do-first tier makes the *capabilities* drivable — verbs, schemas, error bodies, typed writes. The cross-capability composition is different: today it is 254 lines of pure TypeScript, and the cheap answer keeps it there and adds a second caller. That is right if your assistant always has a shell. If you want an agent that talks only HTTP to Axon, the composition eventually has to move into a capability, which reopens a decision trips' README currently argues — and that is your call, not a technical one.
