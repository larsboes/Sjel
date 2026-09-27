# transit

HAFAS journey search, split-ticket solving, German rail ticket extraction, trip persistence, and
fuzzy/triggered trip-search sessions. Ported from a private LifeOS-mono service; the source
bundled transit + a Gemini CV generator into one 764-line `main.rs` — only the transit concern is
here, rebuilt clean (see Redactions + Verdict). `transit` and `capabilities/scouting` share one
database and a `crate_index` (see Architecture); the cross-capability correlation story that
motivates that sharing lives in `capabilities/store/README.md`, and PRD Q45 (2026-08-27) moved
it from a Postgres schema per capability to a table prefix per capability in one SQLite file
(libs/sjel-store/README.md).

## Verdict

**Adopt the architecture, strip what doesn't belong, add the tests that were missing.** The
HAFAS client + split-ticket solver + ticket extractor are real, working capability — but the
source service bundled four unrelated concerns into one 764-line `main.rs` (transit search,
Postgres Trips CRUD, ticket import, and — genuinely out of place — a Gemini-powered CV/resume
generator reading a personal `master_cv.json`). Only the transit concern is ported here. The CV
generator is not ported at all, anywhere — it has no business in a transit service and was
flagged as such the first time this codebase was evaluated.

**The other real gap being fixed, not just carried forward:** `hafas.rs` (a hand-rolled client
against bahn.de's undocumented internal API) and the split-ticket DP solver had **zero tests**
in the source despite being the riskiest code here — reverse-engineered private API, hand-rolled
date math, a real algorithm. This port adds direct test coverage where there was none (`cargo
test -- --list` for the current count): unit coverage of the split-ticket solver's DP logic (chaining across
stops, falling back to direct when a split isn't actually cheaper, handling an unreachable
destination), fixture-based parsing tests for `hafas.rs`'s response handling, and full
store-layer coverage of trip sessions (stable-session-id determinism, upsert idempotency,
cheapest-first ranking, status preservation across fare refreshes, session/manual isolation) —
none of that existed before.

## Architecture

```
main.rs        -- CLI: suggest / search / split / import / plan subcommands
  config.rs     -- resolves default_from_eva/default_to_eva/default_time/database_url (overlay config, see below)
  hafas.rs      -- HAFAS client: station search + journey search against bahn.de's undocumented
                    API, plus the split-ticket DP solver (cheapest chain of pairwise HAFAS queries)
  travel.rs     -- shared types (Station/Leg/Journey/SplitResult), folded in as a module --
                    same call as scouting's opportunity.rs, ~40 lines for one consumer
  extractor.rs  -- pure-Rust PDF/email/regex parsing of German rail ticket confirmations
                    (already well-tested in the source; ported close to as-is)
  store.rs      -- persistence under the table prefix `transit`
                    (transit_trips/transit_trip_legs/transit_trip_sessions) -- see Trip
                    persistence + Fuzzy trip-search sessions below
server.rs      -- transit-server: second binary, Axum HTTP API fronting the same lib -- see
                    HTTP server (transit-server) below
```

`main.rs`'s `plan` subcommand carries its own small set of pure helpers (date-window sampling
via proleptic-Gregorian day arithmetic, soft-destination resolution through `suggest_stations`,
session-summary JSON) rather than splitting them into a new module — same "one focused binary"
shape the CLI kept through the whole rebuild.

No CV generator (see Redactions). Cargo resolves this package through Axon's
root workspace and its single lockfile, which is what guarantees that the
`Journey` types shared with scouting cross the `serde` boundary using the same
compiled dependency instances.

## Commands

```bash
cargo build -p transit && cargo test -p transit                 # no server needed; a temp file per test
cargo test -p transit db_tests::                                # the store suite alone

transit suggest --query "Bonn"                                  # station name -> EVA candidates
transit search  --from 8000044 --to 8000207 --time 2026-08-15T09:00:00
transit split   --from 8000044 --to 8000207 --time 2026-08-15T09:00:00   # cheapest split-ticket chain
transit import  ticket.pdf                                       # extract booking details from a confirmation

# Fuzzy/triggered trip-search session ("I feel like a trip in September"):
transit plan --from 8000044 --destinations "Valencia,Copenhagen" \
  --date-from 2026-09-01 --date-to 2026-09-30 --intent "in Sept, open"   # search + record session
transit plan --from 8000044 --destinations "Valencia,Copenhagen" \
  --date-from 2026-09-01 --date-to 2026-09-30 --max-queries 12 --dry-run # resolve + sample, no search
transit plan --show trip:session:<id>                                   # re-list a session's ranked trips

# Search exactly the days somebody already worked out are possible, instead of
# sampling a window. capabilities/calendar's feasible windows are the intended
# producer; transit just honours whatever list it is handed:
transit plan --from 8000044 --destinations "Muenchen" \
  --dates 2026-08-14,2026-08-15,2026-08-16

# from capabilities/scouting -- fare-search as a scored, stored opportunity source:
cargo run --bin scout -- --adapter transit_fare --date-from 2026-08-15T08:00:00
```

`--from`/`--to`/`--time` fall back to `default_from_eva`/`default_to_eva`/`default_time` in the
overlay config if set; otherwise they're required and the CLI errors with a clear message rather
than silently defaulting to someone else's stations.

## HTTP server (transit-server)

A second binary (`server.rs`), Axum-based, fronting the same `hafas`/`store`/`config` lib code
the CLI uses — added for `dashboard` as a named consumer; see
`dashboard/README.md` for why this reopened the prior
"no HTTP server" call. No `/discover` or cross-capability proxy routes — `transit-server` serves
only transit's own API (see `server.rs`'s own comment on that boundary).

**Port:** `SJEL_PORT` (exported by the runner from the manifest) wins, then `TRANSIT_PORT`
for runs outside the runner, then the shipped default `3000`. Binds loopback only, via
`libs/sjel-server`.

**Endpoints:**

- `GET /health`, `GET /api/health` — liveness + aggregated status (`{status, service, version,
  store}`, where `store` is `"ok"`/`"offline"` from a live `TransitStore::open` probe)
- `GET /api/suggest?q=<query>` — station name -> EVA candidates (wraps `suggest_stations`)
- `GET /api/search?from=<eva>&to=<eva>&time=<iso>` — journey search (wraps `search_connections`)
- `GET /api/split?from=<eva>&to=<eva>&time=<iso>` — cheapest split-ticket chain (wraps
  `search_split_tickets`). Each segment carries `train_match` and the `expected_trains` it was
  judged against, and the chain carries `confidence` plus `unpriced_pairs`/`queried_pairs` — see
  "What a split-ticket chain does and does not promise" below
- `GET /api/trips?session_id=<id>&limit=<n>` — stored trips with their legs, cheapest-first.
  Both parameters optional; `limit` defaults to 100, clamped to 500. The reply carries `count`,
  `returned` and `truncated`, so a bounded read says what it left behind rather than implying
  it returned everything

**Running:**

```bash
cargo run --bin transit-server
```


## How likely the journey holds together

`Journey.reliability` is the product of catching every transfer and the last leg then
arriving within six minutes. Every factor is an exceedance read off punctuality's stored
histogram at the threshold that transfer actually has — no fitted curve, no constants.
Each leg additionally carries its own `on_time_probability`, so a consumer can point at
the weak leg rather than render one opaque score.

The point of asking at the transfer's own buffer is that the answer moves. Live on
2026-08-19, one regional-to-ICE journey through a single transfer at Köln Hbf:

| Buffer | P(catch) | n | Final leg on time | Reliability |
|---|---|---|---|---|
| 10 min | 0.8747 | 1237 | 0.4829 | 0.4224 |
| 16 min | 0.9422 | 1211 | 0.4829 | 0.4550 |

A fixed six-minute figure would have given both journeys the same transfer term and made
the six extra minutes invisible.

**It is a floor, not an estimate.** Two assumptions are stated on the type and neither can
be settled from this data: each onward train departs on schedule, so a transfer term is an
upper bound on that transfer's risk; and consecutive legs are independent, which two legs
of one line delayed by one cause are not. `min_sample` reports the thinnest cell in the
product, because the number is only as measured as its weakest term.

**A missing term costs the whole number, deliberately.** A journey with two transfers where
only one can be scored returns `null`, not a product over the one that answered — which
would read *higher* than the truth and be indistinguishable from a journey that really had
one transfer. The third journey in that live run is exactly this case: its first leg is an
`STR` tram, which DB's published dataset has no cell for, so it scores nothing.

**The type punctuality is asked for comes off the train's label, not off either backend's
product class.** That was a bug until 2026-08-20 and it read as a vocabulary mismatch: the
lookup sent `verkehrsmittel.kategorie` on `dbweb` and `produktGattung` on `dbnav`, and
neither is punctuality's `train_type`. Measured over 104 legs on 16 routes, both backends,
against the 109 distinct types in the cells:

| | reads | for the RE5 Bonn → Köln | consequence |
|---|---|---|---|
| `dbweb` `kategorie` | HAFAS class code | `DRB` — and `DRB` again for the RB26 | no cell; one code over two populations of 10.8M and 14.4M |
| `dbnav` `produktGattung` | product class | `RB` | a cell, from the wrong population — an RE journey answered with RB statistics |
| `train_name` | the label itself | `RE5` → `RE` | the cell that describes this train |

All eleven prefixes seen live — `ICE`, `RE`, `IC`, `S`, `RB`, `EC`, `RJ`, `FEX`, `FLX`,
`EUR`, `ECE` — exist in the ingested cells, so nothing needs translating. `train_type_of`
is that read, and a category equivalence table was deliberately **not** built: `dbweb`
already threw the distinction away, so no correct one exists.

`dbweb` regional legs therefore stay unscored, and now say so. It names those trains by
their bare number (`"28510"`), so there is no label to read — the journey comes back with
`reliability: null` beside `unscored_legs: [{leg_index, train_name, train_category}]`.
That separates three states a bare null used to flatten: punctuality is down, punctuality
has no cell, and nobody asked.

## Ordering by the traveller's own trade-off

`GET /api/search` returns the backend's own order by default, and that order is a
judgement nobody can inspect. When `capabilities/traveler` carries a *stated*
journey weight block, every journey gains a `ranking`: a 0..1 score, its rank, the
weights that produced it, and one factor per reason.

| Factor | Scored against |
| --- | --- |
| `price` | the cheapest in the same answer |
| `duration` | the shortest in the same answer |
| `changes` | the fewest in the same answer |
| `reliability` | `reliability.probability`, absolute |

The first three are **relative** because there is no absolute good fare: a good
price is one that is good compared to the alternatives on this route on this day.
The consequence is that a score is not comparable between two searches, which is
why `weights` rides along with it rather than leaving a bare number.

Reliability is **absolute** because it is already a probability, and re-scaling it
against the set would turn "these are all reliable" into "one of these is best".

**A factor that could not be computed is dropped and the rest are re-normalised.**
A journey whose reliability is unknown does not score zero on it — "nobody measured
this" and "this is unreliable" are different answers, and zeroing the term would
bury a good journey for a missing measurement. The live effect is visible: with
`punctuality` down, every journey comes back carrying three factors instead of
four.

Absence degrades, and here it degrades by doing nothing. With nothing stated on the
profile, or `traveler` unreachable, or its body unparseable, the journeys are
returned in the backend's own order exactly as they were before this module
existed. A search never fails, and never slows to a crawl, for want of a
preference.

The sort is stable, so two journeys the weights cannot separate keep the backend's
own relative order. That is the difference between ranking as a refinement and
ranking as a reshuffle.

### One weight set is not enough, so a trip can override it

The profile's journey weights are the *usual* setting. The operator's own point is
that it depends on the trip, so `/api/search` takes two optional overrides and they
resolve in one place (`transit::ranking::resolve_weights`) so a button and a CLI
cannot disagree about what `cheapest` means:

| Parameter | Form |
| --- | --- |
| `priority` | `cheapest`, `fastest`, `fewest_changes`, `reliable`, `balanced` |
| `weights` | `price:0.5,duration:0.2,changes:0.1,reliability:0.2`, all four, summing to 1.0 |

Passing both is a **400** rather than a precedence rule: a request saying two
different things is a caller bug, and guessing which it meant is how a UI and an API
start disagreeing. Saying neither means the profile applies, and `ranking.source`
reports `profile` or `request` so a reader can tell which produced the order in front
of them.

A phrase is resolved by a **deterministic vocabulary** rather than a model
(`PHRASE_WORDS`), because the mapping is checkable by eye and the repo's rule is that
if the answer can be written down, write it down. German is in the vocabulary because
the operator's own sentences are German at least as often as English, and one that
speaks only English silently matches nothing in half of them. A phrase naming two
priorities averages the presets, which is what *cheap and reliable* asked for; a
phrase naming nothing known is refused **with the vocabulary**, because a silent
fallback would look exactly like the sentence having been understood.

The dashboard surfaces all three: a **rank-by** bar of preset buttons, a sentence
field beside it, and `Ranked` as the default order in the journey list. That last one
mattered — the page sorted journeys by price unconditionally, which silently threw
away the ranking the server had just computed from the traveller's own weights. The
page was making a decision it could not explain on top of one it could.

### The origin and the fare come from the profile too

`from` is optional: omitted, the first of the profile's `home_stations` is the
origin. `bc` and `d_ticket` default from the cards the profile says the traveller
holds — an explicit parameter always wins, because a caller naming a BahnCard is
asking about that card. Measured 2026-09-23 on one route: the same search cost
`47.99 / 52.49 / 67.49` with the cards held and `76.99 / 84.99 / 98.47` without.
Every fare the solver priced before this was a second-class single-adult fare with
no discount, because nothing carried them.

### A station name is resolved, or refused

HAFAS takes an EVA id and returns **nothing at all** for a name — an empty journey
list with HTTP 200, which reads exactly like "no trains that day". So a name is
resolved through the same suggest surface the UI uses, and a name that does not
match is a **400 naming it** rather than an empty answer. The check is strict on
purpose: HAFAS's suggest returns `Hannover Karl-Wiechert-Allee` as the first hit for
`Nowhere At All`, and a wrong destination the caller cannot detect is worse than a
refusal. An all-digit input skips the lookup entirely.

## What a split-ticket chain does and does not promise


The solver prices each stop pair with its own call to `search_connections` and takes the first
journey that comes back. Nothing in that makes the returned journey the one the traveller is
sitting on, so a chain could contain a fare for a train leaving two hours later. Buy three of
four tickets in a chain like that and you own three tickets and no trip.

That is a property of pricing each pair independently, and it is not fixed here. What is fixed
is that the result now says so:

| Field | Reads |
|---|---|
| `segments[].train_match` | `exact` (same trains, same order), `partial` (shares some), `different` (shares none, so this fare is for another service), `unknown` (no train number on one side) |
| `segments[].expected_trains` | the trains the direct journey uses over that hop, so the verdict is checkable |
| `confidence` | the chain's worst case: `low` if any segment is `different` |
| `unpriced_pairs` / `queried_pairs` | how many fare lookups came back empty. The chain shown is fully priced by construction, but the search ran against a table with holes, and a cheaper split may never have been visible |

Empty lookups are ordinary, not exceptional: a live Bonn to Frankfurt search on 2026-08-11
returned nothing for 3 of 6 pairs. Before this, that chain was presented as the cheapest one
that exists.

`savings` is `null` when no direct fare came back. It used to be `0.0`, which the dashboard
rendered as "Direct is cheapest" — a claim nobody had checked.


## Which rail backend answers, and why there are two

`SJEL_TRANSIT_BACKEND` picks it: `dbnav` (the default since 2026-08-20) or `dbweb`. Anything
else falls back to the default rather than failing, because an unrecognised value is a typo far
more often than it is an intent.

`dbnav` is the default because of punctuality, not stability. punctuality keys its cells on DB's
open-data vocabulary; `dbnav`'s `produktGattung` is that vocabulary and `dbweb`'s `kategorie` is
HAFAS's own, so on `dbweb` the identical RE5 arrives as `DRB`, finds no cell, and takes the whole
journey's `reliability` down with it — one missing term voids the product by design. `dbweb` is
fixed too (see the category table below), but the path nobody chose should not be the one that
needs a translation to answer.

| | `dbweb` | `dbnav` |
|---|---|---|
| Endpoint | `bahn.de/web/api` | `app.services-bahn.de` (the DB Navigator app API) |
| Headers | browser UA, plain JSON | `X-Correlation-ID`, versioned vendor media types |
| db-vendo-client's rating | "less stable", aggressive IPv4/IPv6 blocking | more stable, with its own open 403 reports |
| Times | naive local (`2026-09-15T09:04:00`) | offset-carrying (`...+02:00`) |
| Coordinates | absent | present on every station |

### Pointing it somewhere else

Three variables replace the endpoints, one per address, defaulting to the real ones:

| Variable | Replaces |
|---|---|
| `SJEL_TRANSIT_DBNAV_FAHRPLAN_URL` | dbnav journey search, the default backend's |
| `SJEL_TRANSIT_FAHRPLAN_URL` | dbweb journey search |
| `SJEL_TRANSIT_ORTE_URL` | station suggest, which is dbweb's regardless of backend |

Empty or unset falls back to the real endpoint rather than requesting `""`, because a shell that
exports unconditionally sets an empty value when it has nothing to put there.

This exists for the published demo. Every answer this capability gives comes from a live query,
which made it undemonstrable — recording a real timetable publishes something that stops being
true within the hour. `tools/demo-origin` serves bahn.de-shaped payloads instead, and because
the override replaces only the address, the real parser still does the work and what the demo
records is genuinely this capability's output. The same seam makes the client testable offline.

One variable per endpoint rather than one base URL: the two backends live on different hosts
under different path prefixes, so a single prefix would have to be split apart again by whoever
was replacing it.

The point of the second one is that everything here sits on a reverse-engineered endpoint that
can die without notice — the sibling `db` profile's host lost its DNS in May 2026 and took
BetterBahn down with it. Two backends behind one seam means that costs a config change instead
of a rewrite.

Both answer journey search and split-ticketing. The split solver was `dbweb`-only until
2026-08-19, not for access reasons but shape ones: it does not read parsed `Journey` values, it
reads the raw response to learn which train covers which stop pair, and `dbnav` names all of
those fields differently. `DirectJourney` is that seam now — the three facts the solver needs
(cut points, the train per stop pair, the through fare) are read per backend, and the pairwise
pricing, the DP, train matching and contract boundaries are not.

One difference is load-bearing rather than cosmetic. `dbnav` stamps times with their offset, and
`normalize_datetime` refuses those — an offset-carrying string splits into four colon-separated
fields, not three. Since a stop's stamp is handed straight back as the moment to price from, the
offset is dropped when the stop is read. Written from the field list instead of the capture, this
port would have searched fine and then failed every priced pair, which surfaces as "no split
exists" rather than as a malformed query.

Verified live on 2026-08-19, one journey: both backends returned the
same fare, the same `partial` confidence, the same 3 queried pairs with 1 unpriced, and the
same `exact` train match over trains 28510 and 857.

## Reading a ticket: two backends, and what each is actually for

`document_backend` in the overlay picks the reader. `builtin` is pdf_extract and
mailparse; `xberg` shells out to the [xberg](https://github.com/xberg-io/xberg) CLI
(`cargo install xberg-cli`; it builds a bundled Tesseract, so it needs `cmake`).

The reason for xberg is narrow and measured, and it is not the one this started as.
Adopted 2026-08-11 after testing both against the same generated DB confirmation:

| | builtin | xberg |
|---|---|---|
| PDF with a text layer | reading order preserved | preserved in Markdown, **scrambled in plain text** |
| Image | hard error | reads it, correctly, with `ocr_language: deu` |
| Journey table | flattened to a line | still flattened to a line |
| Key-value block | flattened | recovered as a Markdown table |

So xberg is adopted for the image path, which builtin can never serve, and for nothing
else. It does **not** recover a journey table, which is what the switch was originally
meant to fix.

Two findings worth keeping. Its plain-text mode reorders a PDF, putting a journey table's
dates twenty lines from its station names, so extraction parses from Markdown when a
backend produces it. And the default OCR language reads German umlauts as noise: `Züge`
came back as `Ztige`, `möglich` as `méglich`. `ocr_language` defaults to `deu` here.

## Why the parser reads table rows

No reader recovers a journey table as a table, so the parser matches the row *shape*
instead: date, departure time, station, arrival time, station, train. That shape survives
every path tested, including xberg's OCR output, which emits one cell per line.

This replaced a real defect rather than an inconvenience. Reading stations with the
von/nach patterns on flattened table text produced two legs running from "Bahnhof" to
"Bahnhof Zug Gleis" -- the table's own header row -- with `ok: true` and nothing reported
missing. The labelled-station path is still there for a ticket with no table.

## Scheduled and real-time are different fields now

`parse_journeys_from_response` read `sollzeit` with an `or_else` onto `istzeit` and stored
the result in one field. Two things were wrong with that.

The fallback was dead. This endpoint does not serve `istzeit` at all; the real-time key is
`echtzeit`. So the `or_else` never once fired, and every journey carried its scheduled time
as though it were the actual one. The same failure shape as the `id`/`tripId` bug in
Gotchas below: a wrong key name that shows up as silence rather than an error.

And even with the right key, folding them loses the delay. A `Leg` now carries
`scheduled_departure`, `realtime_departure`, `scheduled_arrival`, `realtime_arrival` and
`cancelled` separately. `departure_time`/`arrival_time` stay as the field to plan around,
real-time when there is one. `None` means bahn.de gave no real-time value, which is not the
same as no delay.

Key names were captured from a live response on 2026-08-11 rather than guessed, which is
how the `istzeit` mistake was caught before it shipped. Verified the same evening against
real disruption: ICE 619 scheduled 00:20 and running 00:50, ICE 22 scheduled 23:44 running
00:05, and a tram whose real-time equalled its schedule. Cancellation reads
`originCancelled`/`destinationCancelled`, also captured from a real section, though no
cancelled train was in that response to confirm the flag end to end.

## Trip persistence

`store.rs`'s `TransitStore` owns `transit_trips`/`transit_trip_legs`/`transit_trip_sessions` in
the shared SQLite file — same table-prefix convention every store-owning capability follows, see
libs/sjel-store/README.md. A recorded journey can come from three
places, all tagged via `trigger_reason`:

- **`manual`** — a direct `transit search`/`transit split` CLI call. One-shot, on-demand, no
  scoring involved.
- **`auto`** — `capabilities/scouting`'s `transit_fare` adapter (see that capability's
  `adapters/transit_fare.rs`), invoked via `scout --adapter transit_fare --date-from <ISO
  datetime>`. Origin/destination come from `default_from_eva`/`default_to_eva` in *this*
  capability's own overlay config (no separate route config on the scouting side — deliberately,
  same "no baked-in route" philosophy as `search`/`split`). Every found `Journey` gets recorded
  here **and** converted into a `scouting.opportunities` row (via a pure `journey_to_opportunity`
  conversion, no I/O) — the detailed structured record lives here, the scored/ranked/dismissable
  view lives in scouting, both derived from the same fetch.
- **`session`** — `transit plan` (see "Fuzzy trip-search sessions" below). A user-intent session
  ("Valencia or Copenhagen, in September, open") owns a `trip_sessions` row recording the
  candidate destination set + date window; every journey its fan-out finds is recorded here with
  `trigger_reason = "session"` and `session_id` pointing back at the owning session row.
  Different code path from the `auto` background scan (the "triggered" query vs the "constant
  background scan" — see `capabilities/store/README.md`'s driving queries), same underlying
  `trips`/`trip_legs` store — the two never collide because `transit_fare` only ever fetches the
  one configured default route, a session fetches many.

`record_journey()` upserts on the journey's own HAFAS-assigned id (trusting the upstream id, same
pattern `scouting::store` uses for `Opportunity.id`) and replaces the leg set wholesale inside
one transaction — a re-recorded journey never accumulates stale legs alongside fresh ones, and
**deliberately does not touch `session_id` on the `ON CONFLICT` path** (a journey re-found by a
*different* session refreshes its fare/duration but keeps its prior owner — same "don't clobber a
prior decision on re-fetch" principle `scouting::store`'s status-preserving upsert already uses,
proven by `replanning_session_refreshes_fares_without_losing_status`).

`trips.status` (`new`/`dismissed`/`saved`) exists in the schema but nothing reads/sets it back
yet — same honest "scaffolding only" gap `scouting.source_state.cursor` already carries.

## Fuzzy trip-search sessions

`transit plan` is a *triggered* search — "in September I feel like a trip" — deliberately a
different code path from the constant background scan (`transit_fare` / `scout --adapter
transit_fare`). The background scan watches one configured route continuously; a session widens
both axes on demand: *where* (a soft destination set, not one station) and *when* (a date window,
not one search time). Four shapes layer together:

1. **Soft-destination expansion.** `--destinations Valencia,Copenhagen` is a comma-list of city
   *names*, not EVA codes; the CLI resolves each via `HafasClient::suggest_stations` (bahn.de's
   `/reiseloesung/orte` autocomplete) and takes up to `--candidates-per-dest` EVA matches per name
   (default 1 = the city's main station). A name resolving to nothing is non-fatal — the
   candidate set just shrinks, with a stderr warning.
2. **Date-window sampling.** `--date-from 2026-09-01 --date-to 2026-09-30` is a *range*, not one
   search time. The sampler picks evenly-spaced anchors across the window (stride stretched to fit
   `--max-queries` / candidates) **and always includes the final day** — the cheap fares a "maybe
   mid-September" query wants are often near month-end. No `chrono` dependency: proleptic-Gregorian
   day arithmetic via the classic Hinnant `days_from_civil`/`civil_from_days` pair (~25 lines,
   covered by unit tests in `main.rs`), matching the crate's deliberately stay-sync-and-minimal
   stance (see `Cargo.toml`'s header comment).
3. **Or an explicit day list.** `--dates 2026-08-14,2026-08-15,...` replaces the window and the
   sampler both: those days get searched, in that set, and nothing else. It exists because
   sampling a month is guesswork when something else already knows which days are possible —
   `capabilities/calendar` computes exactly that from the operator's real availability and
   publishes it as a list of days. Transit does not call calendar and does not know why one day
   is in the list and another is not; the caller moves the list across (the one-liner is in
   `capabilities/calendar/README.md`'s why-block). Mutually exclusive with
   `--date-from`/`--date-to` — a silent precedence rule between a window and a day list would
   hide which one ran. `--max-queries` still caps the fan-out: a long list is thinned to the
   budget, keeping the first and last day, the same shrink-to-fit contract the window sampler
   gives. The session row records the span the given days cover, and `--dry-run` prints
   `date_source: "explicit"` so a caller can verify the constraint actually took effect.
4. **Session ownership.** `stable_session_id()` hashes origin + the *sorted* candidate EVA set +
   date window + (trimmed, lowercased) intent into a deterministic id, so re-running the *same*
   plan updates the same `trip_sessions` row with fresh fares instead of accumulating duplicate
   sessions. `trip_sessions.candidates` stores the resolved set as JSON; `date_start`/`date_end`
   store the window — these describe the *user intent*, not the *journey*, so they live on the
   session row, not on `trips`. `trips.session_id` is a nullable FK so a manual/auto trip keeps
   its shape (`NULL`).

The fan-out is plain sequential `search_connections` calls with the same 250ms inter-request
cadence `split` already uses — no async/`tokio` (see `hafas.rs`'s split-ticket comment for why a
personal, low-frequency CLI tool doesn't pay for an async runtime here). `--max-queries` caps
candidates × sampled dates before any search fires, so a wide window never silently hammers
bahn.de. `--dry-run` resolves candidates + samples dates and prints the planned session shape
without searching (sanity-check a fuzzy intent for the cost of N light `suggest` calls).
`--show <session-id>` re-lists an existing session's ranked trips read-only; ranking is
cheapest-first, NULL prices last, shortest-duration tiebreak.

## Config resolution

Identical shape to `capabilities/scouting/src/config.rs`:

1. `$SJEL_TRANSIT_CONFIG` — explicit override, full path to a JSON file
2. `$SJEL_PERSONAL_ROOT/config/transit.json` — the overlay
3. `capabilities/transit/transit.config.json` — local, gitignored, dev fallback

Unlike scouting, there is **no baked-in station-pair default at all**. The source service
hardcoded `8000044`/`8098160` (real Bonn/Berlin EVA codes) as CLI argument defaults —
`default_from_eva`/`default_to_eva`/`default_time` exist purely as an opt-in convenience: set
your own home route in the overlay if you want `search`/`split`/`plan` runnable with fewer
flags, or leave them unset and the CLI requires `--from`/`--to`/`--time` explicitly, erroring with
a clear message rather than silently defaulting to someone else's stations. `scout --adapter
transit_fare` reuses this same config for its route (see "Trip persistence" above) — it has no
separate route config of its own.

`database_path` comes from `sjel_config::database_path`: `SJEL_DB_PATH`, else
`<overlay>/data/axon/axon.db`. It is a deployment fact, not a capability one, so a `database_url`
left in `transit.json` is ignored — a file per capability would drop the cross-capability joins
the shared instance existed for. Nothing to redact any more: a path carries no password, which is
why `config::redact_database_url()` is gone.

## What's not ported

**ONNX delay-risk prediction.** The source service loaded a `tract-onnx` model
(`infra/data/model.onnx`) to score each `Journey`'s delay risk. Axon has no such model artifact —
the training pipeline that produced it (`tools/delay-analyzer` in the source monorepo) was rated
quarry-for-patterns-only in the original evaluation, never adopted. Carrying a heavy ML runtime
dependency for a field that would only ever return a hardcoded fallback constant is the exact
"machinery with nothing behind it" pattern this repo already strips elsewhere (scouting's CV
generator). `Journey.delay_risk_score` stayed in the schema for exactly that reason, and is
now filled -- not by a model, by a measurement. `transit-server` asks
`capabilities/punctuality` over HTTP (never by linking it, CONTRIBUTING.md#schemas-and-dependency-direction) for the
share of that train type's stops at the destination, in the arrival hour, that ran at
least six minutes off schedule. See `src/punctuality.rs`.

Two things that number is not. It is not a prediction: it is what happened at that
station, in that hour, over seven months of DB's own published history. And it is not
the probability the trip works out -- it says nothing about catching a transfer, and a
journey that misses one arrives on a different train than the one it describes.

The dependency degrades and never fails. If punctuality is not running, or has no cell
with enough observations, the score is `null` and the search returns what it always
returned. Verified by stopping the service mid-session: HTTP 200, five journeys, every
score null.

## Redactions applied during the port

- Default EVA station codes (`8000044`/`8098160`) — removed entirely, not just moved to config
  (see Config resolution above: no fallback exists unless you set one yourself).
- The Gemini CV generator (`handle_generate_cv`, which read the source project's
  `master_cv.json`; today's `capabilities/cv` keeps its master in the overlay as YAML) — not
  ported, anywhere.
- The Postgres-backed Trips/legs/packing CRUD — landed as `store.rs` (see "Trip persistence"
  above), once `capabilities/postgres` was actually running. "Packing" (whatever bundling logic
  the source's CRUD layer did beyond trips/legs) was not re-examined — only the shape the store's
  own schema actually specifies got built.
- `main.rs`'s four-unrelated-concerns junk-drawer shape — split into `config.rs`/`hafas.rs`/
  `travel.rs`/`extractor.rs`, 764 lines down to a clean CLI dispatcher (`main.rs`'s suggest/
  search/split/import/plan core + its date-sampling helpers — 561 lines, still one focused
  binary, not a four-concern monolith).

## Gotchas

- **`hafas.rs`'s spoofed browser User-Agent is a deliberate exception to the "self-identifying
  UA" pattern** used elsewhere (`scouting`'s `source.rs`/`cfp_conferences`/`luma` all send
  `Axon-Transit/0.1 (+...)`-style strings). bahn.de's endpoint here is undocumented and
  ungated *only because it looks like ordinary browser traffic* — there's no ToS/robots.txt
  contract being honored by identifying honestly here; a self-identifying UA would plausibly
  just get blocked outright. Named here rather than hidden, same as `scouting/adapters/meetup.rs`'s
  own spoofed-UA gotcha.
- **A real bug found and fixed during this port, not carried forward:** `HafasClient::new()`
  originally built a bare `reqwest::blocking::Client::new()` with **no request timeout** — a
  slow or hung response from bahn.de blocks the calling thread forever. This is exactly what
  happened during the port's own live smoke test (a `search` call stalled past 600 seconds with
  no recovery). Fixed with an explicit 15s timeout; verified against the real live API afterward
  (`suggest --query "Bonn"` and `search --from 8000044 --to 8000207 ...` both complete normally
  in well under a second).
- **A second real bug, found the same way (live verification, not code review) while wiring
  `transit_fare` into scouting:** `parse_journeys_from_response` read a journey's id from the
  `"id"` JSON field — which doesn't exist on the real bahn.de response (the actual field is
  `"tripId"`). Every journey in a real search silently got `id=""`, which meant every result
  collapsed into the same row on the scouting-side upsert (`Opportunity.id` built from
  `journey.id`) — 5 distinct real journeys became 1 stored row, no error, no warning. The
  fixture `hafas.rs`'s own tests were checked against used `"id"` too, so `cargo test` stayed
  green through the entire life of this bug — a fixture that encodes the same wrong assumption
  as the code it's testing won't catch it; only a live call did. Fixed
  (`hafas.rs`'s `id:` field), fixture corrected to `"tripId"`, and a regression test added
  (`missing_trip_id_field_yields_empty_id_not_a_wrong_value`) asserting a response shaped with
  the wrong key produces an empty id, not a silently-accepted wrong value.
- **The `trigger_reason` CHECK constraint migration:** `trips.trigger_reason`'s CHECK started as
  the inline `CHECK (trigger_reason IN ('manual','auto'))` from the original `CREATE TABLE`.
  `trip_sessions` added a third value (`'session'`); `CREATE TABLE IF NOT EXISTS` won't touch an
  existing table's constraints. `init_schema` migrates with a targeted `DROP CONSTRAINT IF
  EXISTS trips_trigger_reason_check` + re-`ADD` of the three-value check — idempotent on a fresh
  install (`IF EXISTS` no-ops) and verified against the live schema. A first attempt catalog-walked
  `pg_constraint` in a `DO` block to find the constraint by content; that bit back with a real
  catalog-column bug (`con.namespace` is `con.connamespace`) caught only because the test
  connected to a real instance — same "fixtures won't catch this" lesson as the `tripId` bug
  above. The walker was dead weight anyway (Phase 2's inline check auto-names to exactly
  `trips_trigger_reason_check` via Postgres's deterministic `<table>_<column>_check` convention),
  so it was dropped rather than debugged.
- No LICENSE file in the source repo; the split-ticket concept was originally mined from
  `betterbahn` (AGPL-3.0) and `besser-bahn` (WTFPL) — both are logged as inspiration-only
  (not vendored) in `upstreams.toml`; the DP algorithm here is an original implementation,
  not copied.
- `extractor.rs` was already solid in the source (10 tests) and is ported close to as-is —
  the redaction/rewrite effort here went into `hafas.rs`/the split-ticket solver, not this file.

## Known gaps, carried forward honestly

- No live-network test exists for `search`/`split`/`plan` in the automated suite, only manual
  verification during this port and the Postgres-wiring follow-up (both real live calls against
  bahn.de, not just fixtures — the `tripId` and `connamespace` bugs above were caught exactly
  because of this) — unlike `scouting`'s `euro_hackathons` adapter, which has a cached-fixture
  test pattern. `hafas.rs`'s automated tests stay fixture-based against captured response shapes.
- ~~No backup coverage for `transit.trips`/`trip_legs`/`trip_sessions`~~ — closed 2026-08-27 by
  PRD Q45. The gap was that raw-copy could not safely reach a live Postgres data directory, so
  the tables needed a `pg_dump`-based mechanism nobody had written. These tables are now
  `transit_trips`, `transit_trip_legs` and `transit_trip_sessions` in the one SQLite file, and
  `capabilities/store` declares the contract that covers every capability's tables at once.
- `trips.status` exists in the schema but nothing reads or sets it back yet — no CLI command
  consumes it (see "Trip persistence" above).
- Split-ticket segment fares are still each found by a *separate* connection search, so
  `train_match` reports whether a segment's fare belongs to the planned train rather than
  guaranteeing it does. A chain with any `different` segment is not buyable as shown, and the
  dashboard withholds the booking link for exactly that case. Making the segment search return
  the planned train by construction is the real fix and is not done.
- Session journeys live in `transit.trips` tagged `trigger_reason = "session"` and do **not**
  currently also surface in `scouting.opportunities` — that would need a scouting-side adapter
  pulling from transit sessions, which would reverse the established dependency direction
  (`scouting → transit`, never the reverse — the path dependency and the comment above it
  in `capabilities/scouting/Cargo.toml`). Deferred until the correlation
  layer's shape is real (see `capabilities/store/README.md`'s still-open correlation-layer question), where
  the join direction is a design decision, not a side effect of "show trip results in the backlog
  too." Session results are queryable through `transit plan --show` and directly against
  `transit.trip_sessions`/`transit.trips`.
- `transit plan`'s "nearby" expansion is intentionally **not** near-radius expansion —
  `--destinations` resolves each name to its main station(s) only. bahn.de's `/reiseloesung/orte`
  endpoint has no proximity/radius search; real geo-radius candidate expansion (every station
  within 50km of Valencia) would need a station-coordinates table + a separate query, not worth
  the machinery until "or nearby" actually matters enough to warrant it. `--candidates-per-dest
  N>1` is the cheap partial substitute (take the top-N autocomplete matches, which for a city
  name tends to surface its major satellite stations too).
- `transit plan` samples one time-of-day per date (`--time`, default `08:00`). Real fare
  sensitivity to departure time-of-day (morning vs evening) is not modelled — a fuller fuzzy
  search would sample a few hours per date. Deferred: each extra time-of-day multiplies the
  query count, and the date-window sampling already gives genuine price variation across the
  window's cheapest days; time-of-day-sampling is additive, not a re-architecture, whenever the
  signal is worth the extra bahn.de calls.
- Session trips with `total_price = NULL` are common for regional/short-distance journeys
  (Bonn→Köln, the live-verified example): bahn.de returns no `angebotsPreis` for trains covered
  by the Deutschland-Ticket / no single-trip pricing. These still rank (NULLs sort last) and the
  journey legs/times are fully recorded — the cost signal just isn't there to compare, which is
  bahn.de's reality, not a bug. The cheap-fare ranking matters most for long-distance (ICE/IC)
  where single-trip prices vary; short-distance sessions find journeys but rarely "deals."

## Upstream reference

`troyriverabusiness/scout` — inspiration for the embeddings+scoring architecture the *scouting*
side uses (mined for ideas, not vendored; no code copied). See `upstreams.toml`. The split-ticket
concept is mined from `betterbahn` (AGPL-3.0) and `besser-bahn` (WTFPL) — original DP
implementation here, not copied. See Gotchas above.
