# Travel system PRD — ideas for the next sessions

Written 2026-08-12, after a day of building. Companion to
`travel-improvement-plan.md` (what was done) and
`travel-system-brainstorm-2026-08-11.md` (the original 45-idea brainstorm).

> **Read §8 first.** This is a snapshot of 2026-08-12 and the ranked lists below have
> been overtaken — nearly everything in §4 shipped between the 12th and the 19th. §8 is
> maintained; §4 is kept as the reasoning that produced the work, not as a backlog.

This file is the forward one. What to build next, what not to, and what I
could not verify. Everything marked **verified** was checked with a tool this
session. Everything marked **unverified** is exactly that, and is not to be
repeated as fact.

---

## 1. Where this actually stands

Built and pushed on 2026-08-11 (18 commits):

| Area | State |
|---|---|
| Rail search | HAFAS against bahn.de's internal API, plus a split-ticket DP solver carrying per-segment `train_match` and a chain `confidence` |
| Rail realtime | `scheduled_*` and `realtime_*` split, `cancelled` flag. Verified live on real 30-minute delays |
| Delay history | 8 months, 117M stop observations, 470,782 cells. Arbitrary-threshold queries answerable |
| Ride lookup | `punctuality ride` returns one train's actual stops on one day |
| Ticket intake | `POST /api/tickets/extract`, table-row parsing, xberg backend reads images |
| Plan state | Booking records, typed payloads, conditional writes, budget, outcome record, places projection |
| Calendar loop | Booked stages sync back as entries; free-cancellation dates become deadlines |
| Agent surface | `axon capability call` speaks all verbs, `/routes` serves derived request schemas |
| Intent | `trips draft-intent` turns a sentence into a form, local model only |

**The fragile part is the rail half, not the flight half.** `hafas.rs` targets the
endpoint db-vendo-client calls `dbweb`, which upstream rates "less stable" with
aggressive IPv4/IPv6 blocking. (The "possibly shut off soon" flag this document
originally pinned here belongs to the sibling `db` profile — corrected
2026-08-12, see §4.2.) Everything above sits on it. See §5.1.

---

## 2. Deterministic or AI? The question, answered by measurement

This got answered by accident today, three times, and the answer is not
"it depends".

**Deterministic wherever the answer is checkable.** Every AI failure observed
today was in exactly this territory:

- Asked for "somewhere warm in October" with no year, the on-device model
  answered **2023**. Twice. (verified)
- It listed `dates` as unresolved *while returning correct dates* for a sentence
  that plainly gave them. Trusting that self-report threw good data away.
  (verified)
- Asked to extract events from a city page with a verbatim quote per date, it
  returned five real page headings with five sequential invented dates, and
  **passed** a naive quote-grounding check 5/5. Requiring the quote to support
  the date took it to 0/5. (verified)

That last one is the general law, and worth stating plainly because it outlives
this project: **a quoted span existing in a document does not verify the claim
attached to it.** Any grounding check has to test the specific assertion.

**AI where the input is unstructured and the output is checkable.** Sentence to a
form, layout to text, prose to fields. In each of those the model proposes words
while a deterministic path decides what survives. That is literally what
`intent.rs` does now: the model returns dates, a date-validity check decides
whether they live, and `unresolved` is rewritten to match.

**The rule to build against:** if you can write the check, write the check and let
the model do the typing. If you cannot write the check, do not use the model,
because you will not be able to tell when it is wrong. Fare comparison, transfer
buffers and feasibility are all checkable, so none of them want a model.

---

<!-- The bolded lead-ins in the service lists and in "Do not build" are a reference
     index, not emphasis: each entry is looked up by name rather than read in order.
     Em dashes and rule-separated sections are this document's established register,
     carried over from the 2026-08-11 session that shipped it. -->

## 3. API access reality

### 3.1 Verified myself, tonight

| Service | Status |
|---|---|
| **`mcp.kiwi.com`** | **Open, unauthenticated.** `serverInfo` = `kiwicom-flight-search` 1.28.1, `/.well-known/oauth-protected-resource` 404s. JSON-RPC over HTTPS with SSE framing, callable from Rust with reqwest plus a small `data:` line parser. No key, no SDK |
| **Booking.com** (in-session connector) | Works. Real Munich prices for 14 to 16 Sept 2026, €41.77 to €543, **with coordinates and booking URLs**. Coordinates matter, because trips matches places by coordinate |
| **Expedia** (in-session connector) | Works, with real data bugs. See §5.4 |

### 3.2 The services you named

Researched, with sources; see §7 for what could not be confirmed.

- **Google Flights.** No official API. QPX Express was killed in 2018 and never
  replaced. `robots.txt` disallows `/travel/flights/search`.
- **ITA Matrix.** Google-owned, powers much of the industry, and has no public
  programmatic access. The web UI remains free to use by hand.
- **Skyscanner.** A partner API exists, its stated acceptance criteria exclude
  "students and other individuals working on a non-commercial basis". You would
  spend a week being declined.
- **Booking.com Demand API.** Partner-gated. But the in-session connector above
  already gives you accommodation search, which is the part you actually wanted.
- **Amadeus Self-Service.** Reported decommissioned 17 July 2026. Consistent
  across trade press and migration write-ups, **not first-party confirmed**.

### 3.3 The number that settles the flight question

On CGN→LIS, Kiwi and Expedia landed **0.3% apart** on the identical Ryanair
pairing. Date flexibility on the same trip moved the price **40 to 54%**: €223
fixed, €171 at ±3 days, €103 across a full October window.

Provider breadth is the low-value axis. Skyscanner's one irreproducible property
is comprehensiveness, and comprehensiveness is worth the least to one person
flying a handful of times a year on three or four corridors. Kiwi already exposes
the flexible-date knobs in a single call.

---

## 4. Ideas, ranked within each area

Effort assumes you are writing it. Durability is "how long before this needs
re-work", which matters more than effort.

### 4.1 Flights

### F1. Airline confirmation parsing in `extractor.rs`
*Half a day per carrier. The most durable thing in this document.*
Flights you actually booked land in trips with PNR, legs, times and price paid.
Your own transaction record, not a vendor's inventory: no ToS surface, no bot
detection, no rate limit, and the data is yours permanently. Confirmation formats
change rarely and fail visibly when they do. Start with whoever you fly out of
CGN. Extends work that already exists and is already tested.

### F2. A Kiwi adapter in `trips`
*An afternoon. Months to years of durability. Built 2026-08-12, Axon `38ce6ac`:
`kiwi.rs` + `GET /api/flights/search`, both hazards below handled in the types
(UTC twins per segment via station-time's country lookup;
`hidden_ground_transfers` computed from segments). Live: CGN→STN €50 with
correct 06:15Z→07:35Z instants.*
~150 lines: reqwest POST to `mcp.kiwi.com`, parse the SSE `data:` line, map to a
`transport` plan item with `mode: "flight"`. The point is not searching, which you can already do in a
session. The point is that the result becomes durable local state next to your
rail legs, and that trips can act unattended.

Two hazards, both verified in live responses:
- Timestamps are **naive local with no UTC offset**. CGN 08:15 → STN 08:35 reads
  as 20 minutes while `durationSeconds` says 4800. Any buffer arithmetic against
  a DB rail leg must normalise via airport timezone first.
- `route[]` is **lossy**. An itinerary with segments CGN→STN then LGW→BCN reports
  `route` as `["CGN","STN","BCN"]`, silently hiding a 75 km Stansted-to-Gatwick
  self-transfer. Trust `segments[]`, never `route[]`.

It fails loudly, which is the important difference from your bahn.de client. A
designed JSON-RPC surface gives you an error or a schema mismatch rather than
quietly wrong prices. Put it behind a trait so losing it costs one file.

### F3. Flexible-date grid
*A day, on top of F2. Built 2026-08-12, Axon `7b382a3`: `GET /api/flights/grid`,
one widened search reduced to cheapest-per-day, self-transfer days flagged.
First live call: €36–83 across seven CGN→STN days, a 57% swing.*
Since date flexibility is the 40-to-54% axis and provider breadth is the 0% axis,
this is where the money actually is. One route, a ±N day grid, cheapest-per-day.
Rate-limit yourself hard: `mcp.kiwi.com` publishes no limit (see §7).

### F4. Pivot routing over the friend graph
*Added 2026-08-12, from the Mallorca story: CGN/VIE to Sweden, cheap ticket to
Mallorca, a free night at a friend's, onward flight a day or two later. Built
the same night, Axon `f002fb6`: `GET /api/flights/pivot`, pivots from the
overlay's trips.json (schema example shows the shape — add your real ones
there). First probe reported the truth: on that date the direct was €37 and
the pivot €1010. The trick only wins on the right dates; now it gets checked.*
Not general virtual interlining. That is Kiwi's core product and a
combinatorial trap; rebuilding it loses. The personal version is tractable and
is something no commercial engine can ever offer: a preference graph of pivot
cities where sleeping is free or wanted (friends, family), each with a
willingness window (+1, +2 nights), and the search enumerates
origin→pivot→destination over those offsets via the Kiwi adapter. A handful of
pivots times a few offsets is hundreds of calls, not millions, and the pivot
night prices at €0 lodging. The risk primitive already exists: R3's
contract-boundary penalty applies verbatim, because separate tickets mean no
through-protection when the first leg is late. Preferences are data the solver
reads (a preferences table next to the plan store), never prompt text.
Depends on F2 and X1.

### 4.2 Rail

The research landed 2026-08-12: five questions, five answers, every source
fetched that day, and the claims decisions rest on re-checked adversarially.
Three things settled, then the ideas they reorder.

**No official DB fare API exists at any price. That is now enumerated, not
assumed.** The DB API Marketplace catalog is 23 products; not one mentions
fares or ticket sales. The closest, Timetables, is departure boards and train
runs, free at 60 requests/minute under CC BY 4.0. Fares exist officially only
behind the sales-partner gate: a direct DB-API connection ("PST-Schnittstelle"),
a connected technical service provider, or agency access via GDS. That channel
is addressed to tour operators, travel agencies and airlines, and it opens with
a cooperation-inquiry form, which is how Trainline sells Sparpreise and you
never will. DELFI open data is schedule-only. §5.2 stands, with one honest
caveat: a partner-only API that is not publicly cataloged can never be
positively excluded. (verified)

**One correction to this document.** The "current backend possibly shut off
soon" flag in db-vendo-client's README belongs to the `db` profile, not `dbweb`.
What `dbweb` actually carries is "less stable" plus "aggressive blocking
(IPv4/IPv6)". §1 and §5.1 said otherwise and are corrected in place. The
distinction matters less than it looks: the vendo host behind the `db` profile
already lost its DNS in May 2026, and that single event is what has kept
BetterBahn broken since June. These endpoints do die.

**Sparpreis prices do fall, just rarely and unpredictably, and no dataset says
how often.** Mechanics from practitioner observation: contingents get re-steered
when sales lag a train's forecast, cheaper buckets are sometimes released late
or re-released after selling out, and DB runs promo windows (12% on all national
Sparpreise at the October 2025 booking start, verified from DB's own press
release). The rise side is measured: vzbv logged 289,154 queries over 690 days
and found short-term Super Sparpreise about 2.8× long-term ones, and mid-range
bookings +36% median year over year in Q1 2026. Nobody tracks a single train's
price path, so the drop rate stays anecdotal. (verified, except the drop rate,
which nobody can)

### R1. A `dbnav` backend behind the trait — now concrete
*Days: a second request/response layer, not a refactor. Still the
highest-value defensive change here. Seam built 2026-08-12, Axon `9fc0cfa`,
and the premise inverted on live contact: the canonical db-vendo-client
itself gets OPS_BLOCKED from this network on dbnav while "less stable" dbweb
serves every probe. The backend selection exists (env-switched, dbweb
default); the dbnav variant refuses with that evidence rather than carrying a
parser written against a response nobody can produce. Finish trigger: one
re-probe of the canonical body; the day it serves journeys, build the parser
from the capture.*
`dbnav` is the DB Navigator app API at `app.services-bahn.de`, a host that
itself churned in May 2026, with its own open 403-blocking issues (#50, #46 on
db-vendo-client) and a 60/min limit. So not a hard swap: implement it as a
second backend behind the existing trait and keep `dbweb` primary until it dies.
The concrete deltas, read from the profile source: new paths
(`/mob/angebote/fahrplan` for journeys, `/mob/location/search`,
`/mob/bahnhofstafel/`, `/mob/zuglauf/`), an `X-Correlation-ID` header of two
UUIDs joined by `_`, Accept and Content-Type set to versioned vendor media types
(`application/x.db.vendo.mob.verbindungssuche.v9+json`; DB has bumped that
version before), a German-keyed POST body (`reiseHin.wunsch` with
`abgangsLocationId`/`zielLocationId` lid strings, `zeitWunsch`, `maxUmstiege`),
and a different response schema that loses stopovers in boards. (verified from
the repo; the exact body field list came back partially paraphrased, so read
`p/dbnav/` directly before coding)

Live probe 2026-08-12, from this network: the endpoint answers — a hand-rolled
journeys request got HTTP 400 `{"code":"VALIDIERUNG"}`, not the 403 the tracker
issues report. So no IP blocking here today, and the remaining R1 work is
faithful request shaping against the profile source, not fighting a wall.

### R2. Teach the solver the fare rules it prices against
*A day. Turns the split solver from price-guesser into fare-modeler. Built
2026-08-12, Axon `3d4d676`: BahnCard/class/D-Ticket ride every query, so the
vendor's engine returns discount-correct fares per Fahrkarte. Live A/B:
€32.99 → €24.74 with bc=25, the exact 25%.*
All from the Beförderungsbedingungen, Stand 01.01.2026, fetched and full-text
searched:

- BahnCard applies per Fahrkarte, so per split leg: BC25 gives 25% on Flexpreis
  and the whole Sparpreis family; BC50 gives 50% only on Flexpreis and 25% on
  Sparpreise. A solver assuming 50% on Sparpreis legs is wrong by design.

- Per-ticket floors: Sparpreis ab €21.99, Super Sparpreis ab €17.99. Every
  extra split leg carries its own floor, which is what erodes three-way splits.

- A Deutschlandticket zeroes pure regional legs — price that and nothing else
  as free. What the zero costs is R3's subject.

### R3. Price the risk at contract boundaries, not just the fare
*Half a day, on top of R2. What BetterBahn does not do and ours should. Built
2026-08-12, Axon `3329872`, one deliberate deviation: boundaries carry FACTS
(station, same-train, UTC buffer, delay share) and no probability — the
punctuality module's own doctrine says arrival-delay data cannot produce
transfer risk, so the calibrated penalty stays gated on L1/L2 outcomes.*
Every split ticket is its own Beförderungsvertrag (BB 1.3.4 — one transaction
counts as one contract only when multiple tickets are issued "aus technischen
Gründen"). DB's own FAQ is explicit for the D-Ticket case: two separate
contracts, no continuous passenger rights, and a late regional feeder does NOT
release the Sparpreis Zugbindung. A through Sparpreis, by contrast, already
includes regional feeders (Produktklasse C) inside the one contract. Zugbindung
release at ≥20 min expected delay and 25%/50% compensation at 60/120 min attach
per ticket. So the chain `confidence` should carry a risk penalty per contract
boundary, priced against the per-station delay history we already have.
Split-ticketing itself is not prohibited: full-text search of the BB found no
clause against consecutive tickets. Two things stay unverified. Whether the
train must actually stop at the split station (third-party guides say so, the
BB does not; moot for us, since candidates come from stopovers anyway), and
whether bahn.de flags deliberately split baskets as separate contracts per EU
2021/782 Art. 12, which decides through-liability on one-transaction bookings.

### R4. A Sparpreis watch as a cron, not a daemon
*Hours, on the existing client. Demoted from "do not build" to "barely build".*
Watching a booked train is dead weight: drops are rare, unquantified, and a
bought Super Sparpreis is non-exchangeable anyway. What has defensible value is
watching a not-yet-booked trip for the two real events: late-released cheap
contingents and DB promo windows. One cron, alert on drop below threshold,
rate-limited like everything else on that client. Prior art: sparpreis.guru
already displays price development per connection; read it before building even
the cron.

### What BetterBahn actually does (and what to steal)

BetterBahn — the split-ticket tool, ~2.5k stars, AGPL — is narrower than our
solver: it anchors on one pasted bahn.de connection, takes split candidates only
from that journey's own stopovers, tries exactly ONE split point (two segments,
no DP, no multi-split), and matches sub-journeys by departure time within 60
seconds rather than by train identity. Our DP with `train_match` is strictly
more general. Worth stealing, all verified in its source: carrying BahnCard and
D-Ticket into every sub-query; a hand-maintained whitelist of the nine IC/ICE
corridors the D-Ticket covers; honest "cannot price this" flags instead of
silent drops; and per-segment bahn.de deep links that encode station IDs, exact
departure, class and BahnCard tariff codes — the difference between a result
and a bookable result. It has been broken since June 2026 because its API
host's DNS vanished, which is both a warning and the argument for R1's
two-backends-behind-a-trait shape.

### 4.3 Accommodation

### A1. Booking.com results as `stay` plan items
*Hours. Built 2026-08-12, Axon `7bdcbd4`.*
The connector returns coordinates and a booking URL. `stay` is already a valid
`item_type`. This is a mapping function and a write, nothing more. It makes a
plan hold where you are sleeping next to how you are getting there, which is
currently the biggest hole in a trip plan. As built: `stay` joined the declared
payloads (`check_in`, `check_out`, `latitude`, `longitude` required, provider
fields riding along), validated on write with the missing field named, schema
and README updated, demo seeder fixed to the new shape. Rebuilt, restarted and
probed live the same night: a stay missing its latitude gets a 400 naming the
field, the declared shape gets a 201 and reads back intact.

### A2. Accommodation near the stage, not the city
*Hours, after A1. Ran live 2026-08-12.*
You already have a 75 km great-circle matcher and destination coordinates. A
search anchored on the stage's destination coordinate beats one anchored on a
city name, and the Booking connector takes coordinates directly. First live
run exposed the real gap: upcoming stages often carry NO coordinate (October
Berlin, DevFest Hamburg both don't), while past plans do — so the flow
resolves the anchor from the places projection first. The recipe lives in the
trips README; the October Berlin plan now holds a five-offer `option_set`
plus a stay candidate (Moxy Ostbahnhof, 8.6, €707 for six nights), both
written through the declared-payload gate on the live server.

### 4.4 Cross-cutting

### X1. Timezone-correct leg arithmetic
*Hours, and it blocks F2 and any door-to-door comparison. Built 2026-08-12,
Axon `94c9865`.*
Kiwi returns naive local times. DB returns Europe/Berlin wall-clock. Nothing in
the system currently normalises either. Every buffer, every "can I make this
connection", every duration comparison is wrong the moment a leg crosses a
timezone. This is small, unglamorous, and everything else depends on it.

As built, the premise sharpened: bahn.de serves each stop's naive time in that
stop's OWN local zone, measured live (Köln 09:43 CEST → London 13:57 BST reads
as 4h14m by subtraction; true elapsed 5h14m). New lib `station-time` maps a
station's UIC country prefix to its IANA zone; legs now carry additive
`departure_utc`/`arrival_utc` while the naive display strings stay
byte-identical. Unknown prefixes yield absent fields, never a guess. Verified
live post-restart: the Eurostar leg reports 10:29Z→12:57Z. F2 consumes the
same lib with an airport-timezone lookup as its own second source.

### X2. The grounding verifier
*A day, and much better motivated than it was yesterday. Built 2026-08-12,
Axon `5ae2361`.*
Today's probe showed exactly how a `source_text` check fails: the quote was real
and the claim was invented. `cloud-content-analysis` has that weakness right now.
Build the check that tests whether a quote supports its specific assertion, and
measure the rate on twenty real analyses before deciding anything else.

As built: for date claims the check needed no model — the quote either
carries the claimed day and month (10.08., 10. August, August 10, ISO) or it
does not. Unsupported dates are demoted to null before they can become
calendar proposals, and every demotion logs its verdict. The twenty-analysis
measurement reads those logs once twenty exist; the store held exactly one
real analysis on build night.

### X3. Trip cost roll-up
*Hours. Built 2026-08-12, Axon `c8545b5`: the travel page renders
"Budget X · spent Y" from `budget_cents` + finance's `trip_spending`, absent
when neither exists, finance-down tolerated silently. svelte-check clean.*
`budget_cents` exists on a plan and finance already tags postings with
`axon-trip-id` and returns a per-plan spend summary. Nothing compares them. One
read, one subtraction, one line in the UI.

### X4. Window-and-event anchored planning ("when could I go")
*Added 2026-08-12. For trips whose timeframe is fuzzy: "sometime in autumn",
"whenever that event is on". Built the same night, Axon `8d6b8fb`:
`GET /api/flights/when` prices a span (≤42 days) and joins your calendar —
first live answer: September to Lisbon, €47 on a free day, the
Barcamp/Hackathon days ranked last by name.*
Inverts the calendar loop. Booked stages already sync INTO the calendar; this
reads the calendar back OUT to find the free windows, prices each window with
F3's grid, and lets a scouting event anchor a candidate. The answer becomes a
ranked list of (window, price, anchor) instead of "pick dates first, then
search". `draft-intent` already parses the fuzzy sentence and its date-validity
check extends naturally from exact dates to window constraints. The pieces all
exist; this is a join, not a new engine. Depends on F3.

### 4.5 The learning loop

### L1. Fill in one outcome record

Not code. After your next trip, `POST
/api/plans/:id/outcome`. Two of those and the whole learning idea is either
validated or deleted, which is what it was built to answer.

### L2. Transfer-buffer calibration
*Gated on L1.*
You now have per-station delay distributions at arbitrary thresholds and a place
to record whether you actually made the connection. That pair is the honest
version of "is this transfer safe". Do not start before L1 produces data.

---

## 5. Known fragility

**5.1 `hafas.rs` sits on the shakier bahn.de endpoint.** db-vendo-client maps
`bahn.de/web/api` to its `dbweb` profile and rates it "less stable" with
"aggressive blocking (IPv4/IPv6)". The "current backend possibly shut off soon"
flag belongs to the sibling `db` profile, whose host in fact lost its DNS in May
2026 — this document originally pinned that flag on `dbweb` and was corrected
2026-08-12. Its `dbnav` profile is rated more stable, with open 403 issues of
its own. Watching that repository costs nothing and is your early warning.
Adding a second backend (R1, now scoped) is the single highest-value defensive
change available.

**5.2 There is no authorized German rail *fare* API at any price.** None of DB's
official products return prices. That is precisely why the reverse-engineered
client is defensible for rail and has no excuse for flights, where an open
channel exists.

**5.3 The cancellation flag is unconfirmed end to end.** `originCancelled` and
`destinationCancelled` were captured from a real section, but no cancelled train
was in that response.

**5.4 Expedia's connector has real data bugs.** Verified: returned USD for a Bonn
user with no currency parameter in its schema, quoted bag fees in GBP, labelled a
DUS departure as city "Cologne", and reported `seats_left: 0` on bookable
results. Second opinion only, never a source of truth.

---

## 6. Do not build

Each of these is tempting and each has a specific reason.

- **A Google Flights scraper** (`fast-flights` or any fork). Your bahn.de failure
  mode with worse odds. The project's own tracker shows a cookie-consent wall
  breaking all searches, open since May 2026.
- **A browser extension that re-runs searches in the background.** Once the
  extension issues the fetch, the "I'm only reading a page I opened" position is
  gone, and Google's robots.txt disallows `/travel/flights/search`. Manifest V3
  makes the background work awkward besides.
- **Self-hosting MOTIS or OpenTripPlanner for rail+air.** You would be solving
  the half that is already solved and importing the half that does not exist.
  MOTIS has `AIRPLANE` as a first-class mode; across all of Transitous the only
  air feeds are three small Italian regional ones. A live MUC→FCO query restricted
  to air returns four itineraries **routed via Sardinia**. Confidently wrong is
  worse than empty.
- **Duffel.** Search is free only up to a 1500:1 search-to-book ratio. You would
  book zero and search constantly, permanently on the wrong side of it.
- **SerpApi or Bright Data.** €300 to €500 a year to price-check six flights, and Bright
  Data's core business is residential proxying, which is the opposite of your
  privacy constraint.
- **Applying to Skyscanner, Kiwi Tequila or Travelpayouts.** All three decline
  individuals; Skyscanner says so in writing.
- **Anything built on Amadeus Self-Service.** Reported dead since July 2026.
- **Your own rail price-watch daemon.** Google Flights does this free for air with
  email alerts. For DB the research answered it: drops exist but are rare and
  unquantified, so nothing bigger than R4's single cron is justified.

---

## 7. What I could not verify

- **`mcp.kiwi.com` has no published rate limit, quota, SLA or acceptable-use
  policy.** I confirmed it answers unauthenticated with two live calls and that
  its robots.txt allows ClaudeBot. I did **not** confirm what happens under
  sustained automated use, nor whether Kiwi permits persisting results. Rate-limit
  hard, cache aggressively, treat withdrawal as a matter of when.
- **The Amadeus shutdown date.** Strongly reported, not first-party confirmed;
  their developer portal renders as a JavaScript shell to a non-browser client.
- **The universal "no official DB fare API".** Rests on the 23-product
  marketplace catalog enumerated 2026-08-12 plus targeted searches; a
  partner-only API that is not publicly cataloged cannot be positively excluded.
- **Whether the train must stop at the split station.** Asserted by third-party
  guides only; the Beförderungsbedingungen contain no such clause.
- Anything in §4.2 marked as such.

---

## 8. Status, 2026-08-19

**This document is a 2026-08-12 snapshot and its old §8 was wrong by the 19th.** A
session that read it nearly rebuilt three things that already existed. Everything the
ranked lists above propose is now built except where noted here, so read this section
before that one.

Shipped 2026-08-12, the same night this was written: F2 Kiwi adapter (`38ce6ac`), F3
flexible-date grid (`7b382a3`), F4 pivot routing (`f002fb6`), R2 fare rules
(`3d4d676`), R3 contract boundaries (`3329872`), A1 stays (`7bdcbd4`), A2 stage-anchored
accommodation (`b3eda5c`), X1 timezone-correct legs (`94c9865`), X2 grounding verifier
(`5ae2361`), X3 cost roll-up (`c8545b5`), X4 window planning (`8d6b8fb`).

Shipped 2026-08-12 to 08-17, all three of which the old §8 still listed as next:

- **R1 `dbnav` second backend.** Seam `9fc0cfa`, journeys against a real capture
  `aec53b4`, and the split solver `cdacfbf` on 2026-08-19 — the last of it. Both
  backends now answer journey search and split-ticketing, verified live on the same
  query returning the same €73.99, the same `partial` confidence and the same `exact`
  train match.
- **F1 airline confirmation parsing** (`8add6fd`). `parse_airline_legs` reads
  Eurowings, Ryanair, Lufthansa, easyJet, Wizz Air and Condor confirmations into
  `mode: "flight"` legs.
- **R4 Sparpreis watch** (`518c14c`), as the single cron the research argued for, with
  its own capability manifest on a 12h schedule.

Shipped 2026-08-19:

- **Journey reliability** (`f35d04e`, closes the transit issue this document's §4.5
  gestured at). The product of catching each transfer at its own buffer and the last
  leg arriving within six minutes, every factor an exceedance off punctuality's stored
  histogram. No fitted curve: the originating issue proposed one with three guessed
  constants and it was not built. Live, the same Köln transfer reads 0.8747 at a
  10-minute buffer and 0.9422 at 16.

### What is actually left

1. **The dbweb train-type vocabulary mismatch.** Found 2026-08-19 and the highest-value
   item here, because it silently costs numbers the system already computes.
   punctuality's `train_type` is DB's open-data vocabulary (`RB`, `RE`, `ICE`); dbnav's
   `produktGattung` matches it, dbweb's `kategorie` does not — the identical RE5 comes
   back as `DRB`, which has no cell. So on dbweb every regional leg silently loses its
   `delay_risk_score`, and journey reliability scores nothing at all. Deliberately not
   mapped by guess. What it needs is a checked equivalence table.
2. **A parquet-reader test.** Found while bumping arrow-rs: punctuality's 29 unit tests
   cover the hour/weekend maths and never open a file, so the one component whose whole
   job is parsing someone else's binary format is untested. Needs a committed miniature
   fixture, since the real monthly files are private and 100+ MB.
3. **L1, fill one outcome record.** Still not code, still the gate on the whole learning
   loop, and now the gate on two things rather than one: L2 transfer-buffer calibration
   *and* the independence assumption journey reliability states rather than corrects.
   After the next real trip, `POST /api/plans/:id/outcome`.
4. **The cancellation flag, end to end.** `originCancelled`/`destinationCancelled` are
   captured from a real section and fixture-tested, but no cancelled train has been in
   a response yet. Opportunistic: it costs nothing to keep the vocabulary and wait.
5. **`bodies_without_schemas` as a build gate.** Fourteen routes still have undeclared
   bodies. Failing the build on all of them today blocks work rather than directing it.
6. **Obsidian export.** Still gated on there being something machine-derived worth
   exporting, which is still L1.

**Nothing here is a search-engine problem any more.** Every remaining item is either a
measurement the system cannot yet make honestly, or a gap in what checks the system's
own work — which is a different and healthier list than the one this document opened
with.
