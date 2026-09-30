# Travel seams deliberately not built

Four things the Travel workspace suggests but Sjel does not do. Each is a rejection with a
reason, not a backlog item, and each gets re-proposed every time somebody reads the travel page
and notices the gap. They are written down here so the argument happens once.

## No leave-home scheduler in Sjel

`capabilities/home-assistant/` contains three files: `README.md`, `service.toml` and
`home-assistant.env.example`. There is no `src/`, and the README says why: Sjel owns the pinned
runtime and the public configuration shape, while the overlay owns the home's devices, entity
IDs and automations. A departure trigger is an automation over device state at a specific time,
which is what Home Assistant already is, so building a second scheduler here would mean
reimplementing the thing this capability exists to run.

The calendar entry such an automation would fire on already exists.
`POST /api/trip-plans/:plan_id/sync` in `capabilities/calendar/src/server.rs` writes one all-day
`away` entry per stage whose status is `booked` or `option_selected`, keyed
`trip:stage:<stage-id>`, readable over `GET /api/entries`. The automation reads that entry and
decides what leaving the house means; Sjel's side of the seam is producing it.

## Sjel never executes a booking

`capabilities/transit/src/hafas.rs` states in its first six lines what it talks to: bahn.de's
internal, undocumented journey-search API, the same one the website calls from the browser, with
no key, no auth and no public docs. It names exactly two endpoints, journey search and station
lookup. Neither is a purchase endpoint, and the same file's `BROWSER_UA` comment explains that
the endpoint stays reachable only because the traffic looks like an ordinary browser.

So the flow is inverted on purpose, and that inversion is the design rather than a missing
feature. The operator books in a browser, where the carrier's own checks and refund terms apply,
and pastes the confirmation back. `POST /api/tickets/extract` on transit
(`capabilities/transit/src/server.rs`) is that intake: it parses the file, returns the parse and
stores nothing, and its doc comment gives the reason it must not write a booking record by
itself, since the extractor assigns dates positionally and takes the first price match. What the
operator keeps is a `booking` plan item here, which requires `provider` and `order_ref`
(`DECLARED_PAYLOADS` in `capabilities/trips/src/store.rs`) and records only whether a traveler
name was present, never the name.

## No flight search

Nothing in `capabilities/` or `libs/` searches for flights. A case-insensitive grep for flight,
airline, IATA and the usual provider names returns `TransportMode::Flight` in
`capabilities/trips/src/store.rs`, its Obsidian parser mapping in
`capabilities/trips/src/obsidian.rs` that accepts `flight` or `plane`, and otherwise only the
English phrase "in flight" in unrelated comments.

Holding a flight in a plan already works, which is the part worth having. `PlaceKind::Airport`
and `TransportMode::Flight` both exist in `capabilities/trips/src/store.rs`, a stage can declare
`Flight` among its `transport_modes`, and a `transport` plan item carries
`{"mode": "flight", "journey": {...}}`. `validate_payload` in the same file requires those two
keys and does not constrain `mode` against the enum, so that write is accepted today.

Finding the flight is the rejected half. Mature flight search is already reachable from the
operator's assistant, and the cost of owning one more travel API is measured rather than
guessed: `capabilities/transit/README.md`'s Gotchas record two real bugs in the one
reverse-engineered travel API this repo does maintain, a missing request timeout that hung a
call past 600 seconds and a wrong JSON key that silently collapsed five journeys into one stored
row, neither caught by fixtures. That is the recurring maintenance bill, and a handful of trips a
year does not pay it.

## No cloud AI for itinerary ranking

The cloud path in comms gates on trust class before anything leaves the machine.
`tier_allows` in `capabilities/comms/src/cloud_derivative.rs` refuses `vault` content outright,
lets a `public` tier through only for a public original with a `bounded-public-v1` passthrough
derivative, and lets `pseudonymized_personal` through only when a personal original produced a
personal derivative under `deterministic-entity-redaction-v3`. Ranking an itinerary in the cloud
would have to ride that second lane.

That redactor is structurally wrong for an itinerary, and reading it
(`capabilities/comms/src/cloud_derivative.rs`) is how you see it rather than a suspicion. It
works token by token and replaces links, email addresses, IBAN-shaped strings, phone-shaped
tokens, alphanumerics of sixteen characters or more, and any token holding six or more digits. A
person's name is replaced only when something seeds it: `introduces_person` matches a salutation
(dear, hello, hi, hallo, liebe, lieber) or an English self-introduction (I'm, I am, this is, my
name is), `names_a_person_in_apposition` matches a proper noun followed by `from` and a second
proper noun, and the operator's contact list matches by name. An itinerary greets nobody and
introduces nobody, so every proper noun in it, station and city and hotel and venue, passes
through verbatim, and a short alphanumeric booking reference is under every length threshold and
passes too. The apposition rule makes this worse rather than better here: "Departure from
Stuttgart" is two proper nouns around a `from`, so the word that carries the meaning is the one
that goes. Meanwhile what it does catch is the itinerary itself: an ISO date has eight digits
and a hyphen, so `looks_like_phone` rewrites it to `[phone]`, and a seven-digit EVA station code
becomes `[number]`. The pass strips what makes a plan legible and keeps what identifies the
traveller, which is the inverse of what a pseudonymized derivative is supposed to be. It is also
not reachable from here today: `load_content_item` in `capabilities/comms/src/server/content.rs`
accepts only `feed` and `mail`, so a plan is not a cloud-eligible content item at all.

There is a precondition before any of this is worth revisiting, and it is not about itineraries.
The per-role daily provider ceiling is counted by `claim_cloud_job_attempt` in
`capabilities/comms/src/store/cloud.rs`, which counts rows in `content_cloud_attempts` under a
per-role advisory lock, and that table lives in comms' own Postgres schema. The day a second
capability declares a `cloud_*` role, it either reads comms' schema directly, which the same
repo's rule forbids in comms' own README ("rather than opening a second capability's database
schema"), or it keeps a second ledger, which turns a per-role ceiling into a per-capability one
and makes the budget meaningless. Moving the attempts ledger out of comms comes first; the
itinerary question comes after, if at all.

## No model-read scouting adapter for foreign-language sources

Travel discovery is three hardcoded adapters, all English-language aggregator APIs. What is
actually on in a Spanish or Portuguese city that week lives on a city page the pipeline
cannot read. The roadmap sketched an adapter that would ask a model for bounded events with
a verbatim quoted span per date, keeping only events whose quote appears in the page.

Probed on 2026-08-11 with a throwaway script against a real municipal tourism page, before
writing any adapter. Two findings, and both say no.

**The grounding check that was supposed to make this safe does not work.** On the first run
every extracted event passed: five quotes, all genuinely present in the page. They were
navigation and marketing headings — "Welcome to Porto", "Subscribe to Newsletter" — carried
with five sequential invented dates, 2023-10-01 through 10-05. The model satisfied "quote
must appear in the document" perfectly while inventing the entire claim.

Tightening the check to what actually matters, that the quote must support the *date* it is
offered as evidence for, took it from 5/5 to 0/5. The second run invented `2024-12-25` for
all five.

This generalises past this adapter and is the reason it is written here rather than in a
commit message: **a quoted span existing in a document does not verify the claim attached to
it.** Any future grounding check has to test that the quote supports the specific assertion,
not that it exists. The same weakness sits in `cloud-content-analysis`'s `source_text`
requirement today.

**And the input is mostly not there anyway.** The page returned 37 KB of HTML and 1,911
characters of text: the events are JavaScript-rendered. No model fixes a page that was never
fetched.

The probe script was deliberately thrown away rather than committed. If this is revisited,
the thing to build first is the honest grounding check, and the thing to check first is
whether the source is server-rendered at all.
