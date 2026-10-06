---
project: axon-places
type: isa
phase: climbing
progress: 80
principal_stated_goal: "One map with layers: where I bought, where I travelled, where people I know are — photos later."
---

# ISA · places

Capability-scoped state of record. Repo-wide items stay in the root `ISA.md`.

## Problem

Purchases, travel and people each hold location facts, and none can be drawn on a
map. The 2026-08-25 survey measured it: no finance transaction carried a
coordinate, transit persisted station names while discarding the coordinates
HAFAS returns, and almost no person note carried one. Four incompatible place
shapes existed and no geocoder existed anywhere in the repo.

Live-data numbers stay out of this public file by rule. Every claim below that
says "overlay evidence note" was verified against
`<overlay>/data/places/evidence-2026-08-25.md`, which holds the measured
counts.

## Vision

One `/map` route with switchable layers, fed by one place registry and one cached
geocoder. See `README.md` here for the four dated decisions (D1–D4) that shape it.

## Goal

The dashboard `/map` route renders spend, travel and people layers from live
data, and every coordinate on it is traceable to a registry row with a source.

## Features

### F0 · Registry, geocoder, layer endpoints

- [x] PLC-1 — the places server answers `/health`, `/ready` and `/routes` on
  8093, and the route-manifest coverage test passes. Evidence: `/ready` returns
  `{"ok":true,"capability":"places"}` on 8093 via service-runner, coverage test
  green in `bazel test //...` 70/70, 2026-08-25. Falsifier: any served route
  absent from the manifest.
- [x] PLC-2 — `POST /api/geocode` resolves a repeated query from
  `places.geocode_cache` without network egress. Evidence: a full amex re-run
  with the provider URL pointed at a closed port exited cleanly, every lookup
  served from cache, zero errors, cache row count unchanged (2026-08-25;
  counts in the overlay evidence note). Falsifier: a second identical query
  producing a provider request.
- [x] PLC-3 — a geocode query string never contains person-derived text: every
  query is built from Amex address fields, city tokens, station names or vault
  place names only. Evidence: provenance trace over every `geocode()` call site
  plus the registry-token join, 2026-08-25. Falsifier: a cache row whose query
  text did not come from a place field. (The join's raw form returns a small
  number of coincidences — registry name tokens that also occur inside real
  merchant street names — which are street text, not person data; a
  single-word token match is therefore not the falsifier. The specific
  collisions are recorded in the overlay evidence note.)

### F1 · Spend layer

- [x] PLC-4 — every Amex transaction whose raw export row carries a
  provider-resolvable address has a venue-precision place link, no non-Amex
  transaction has one, and unresolvable addresses are counted and reported,
  never guessed (D1). Evidence: venue links + unresolvable addresses +
  address-less rows sum exactly to the raw row count, every raw row matched a
  candidate fingerprint, and no non-Amex venue link exists (2026-08-25; counts
  in the overlay evidence note). The misses are raw-data defects (a phone
  number in the city column, truncated postal codes, mojibake, highway names);
  each miss is cached, so re-runs stay silent. Falsifier: a venue link whose
  `source_id` lacks a raw address row, or an unresolved address that was
  linked anyway.
- [x] PLC-5 — `GET /api/layers/spend` totals equal the finance projection's
  expense totals for linked transactions. Evidence: the API summary total
  equals the SQL sum over the same join, to the cent (2026-08-25; the totals
  themselves are in the overlay evidence note). Falsifier: a spend-layer sum
  that disagrees with `finance.transaction_projection`.

- [x] PLC-9 — an addressed Amex row whose venue geocode misses gets a
  city-precision link from its raw city line, and the fallback can never
  produce a venue-precision link. Evidence: fallback links written at
  city precision only, source `amex-city-fallback`, guarded by the
  provider-derived kind; a country-granularity city line is rejected
  (2026-08-25, round 2; counts in the overlay evidence note). Falsifier: a
  venue-precision link with the fallback source, or a link whose city string
  does not appear in the raw row.
- [x] PLC-10 — the manual assign flow links only currently-unlinked
  transactions whose description matches exactly, and a city-kind place is
  linked at city precision whatever precision was requested (D1). Evidence:
  live assign returned the demoted precision in its response and removed the
  group from `GET /api/unplaced`; server-side kind guard covered by tests
  (2026-08-25, round 2). Falsifier: an assign that relinks an already-linked
  transaction, or a venue-precision link to a city-kind place.
- [x] PLC-11 — a CSV mapping with `location_columns` stores the raw address
  values verbatim on `finance.transaction_candidates`, and a location-less
  re-import never erases them. Evidence: finance test suite including the
  Postgres round-trip tests; live columns present after migration. First live
  capture happens on the next Amex re-import — the columns hold no values
  until then (2026-08-25, round 2). Falsifier: an import through a
  location-enabled mapping leaving the columns NULL, or a re-import erasing
  captured values.

### F2 · Travel layer

- [x] PLC-6 — every EVA code in `transit.trips`/`transit.trip_legs` resolves to
  a station place with coordinates. Evidence: every distinct EVA resolved,
  none unresolved, 2026-08-25 — after the backfill learned to query transit
  suggest with the bare EVA, which is what resolves the HAFAS meta-stations
  that `punctuality.stations` lacks. Falsifier: a leg whose endpoints cannot
  be drawn.

### F3 · People layer (companion register, PRD §8.2)

- [x] PLC-7 — no `person_places` row reaches `confirmed` state without an
  explicit confirm call, **and a confirmed row can always be withdrawn.**
  Evidence: `propose_person_place` hardcodes `'proposed'`; the only writer of
  `'confirmed'` is `review_person_place` behind the two-variant `Review` enum,
  called solely from the confirm route; the live table held only proposed rows,
  2026-08-25. Since 2026-09-05 that writer is also guarded asymmetrically —
  `proposed → confirmed`, `proposed → dismissed` and `confirmed → dismissed`
  apply, `dismissed → confirmed` and any no-op review are refused with the state
  found — so a replayed or stale call cannot resurrect a dismissal, and a
  mis-clicked confirm does not need hand-written SQL to repair.
  Tests: `store::db_tests::a_dismissed_row_cannot_be_confirmed`,
  `store::db_tests::a_confirmed_row_can_still_be_dismissed`.
  Falsifier: a backfill or proposal path that writes `state = 'confirmed'`, or a
  guard that refuses `confirmed → dismissed`.
- [x] PLC-13 — the register reaches a planner only as an aggregate. Evidence:
  `GET /api/people/presence` selects `person`, counts the distinct set and
  returns `known_companions` and `overlap_days`; the person string never leaves
  the function. Test: `store::db_tests::presence_answers_a_count_and_no_identity`
  asserts the serialised body carries no person, no place name, no row id and no
  confidence. Falsifier: any identity field on that route's reply, or a
  caller-chosen radius.
- [x] PLC-8 — `axon_demo` contains no `person_places` rows. Evidence:
  `axon_demo` has no `places` schema at all, 2026-08-25. Falsifier: any row in
  that table in the demo database.
- [x] PLC-12 — `backfill travelers` derives register proposals from the
  travelers named on trip plans: one `proposed` row per traveler × plan, date
  range = the plan window, and its printed output carries counts only, never
  a name. Evidence: round-2 run wrote proposals for every traveler-plan pair
  with a located destination, all `state='proposed'` through
  `propose_person_place`; output inspected for names (2026-08-25; counts in
  the overlay evidence note). Falsifier: a confirmed write from this path, or
  a person name in its output.

### F4 · Climate normals (PRD §8.2, README D5)

- [x] PLC-13 — a climate request carries a coordinate pair rounded to two
  decimals and the fixed ten-year period only: six query parameters
  (`latitude`, `longitude`, `start_date`, `end_date`, `daily`, `timezone`) and
  nothing else. No plan date, no person, no amount. Evidence:
  `the_archive_request_carries_only_a_rounded_coordinate_and_the_fixed_period`
  captures the stub's request line and asserts the parameter list exactly, both
  coordinates at two decimals, and the ten complete calendar years rather than
  any caller-supplied date (2026-09-05). Falsifier: a seventh query parameter in
  a captured request, or a coordinate with more than two decimals.
- [x] PLC-14 — a place's normals leave the host at most once. Evidence:
  `stored_normals_answer_without_a_provider_request` fetches once, then calls
  `fetch_normals` again with a URL pointing at a closed port and gets the stored
  rows back with the stub's request counter still at 1 (2026-09-05). The refresh
  path is the explicit `--force` flag, run by a human. Falsifier: a second
  provider request for a place that already carries rows, without `--force`.
- [x] PLC-15 — no `kind = 'address'` coordinate ever reaches the provider.
  Evidence: `fetch_normals` refuses before a client is built, whichever path
  names the place;
  `an_address_kind_place_is_refused_before_a_request_is_built` asserts the stub
  saw zero requests and that the refusal names the city alternative
  (2026-09-05). Falsifier: an address-kind row in the fetch loop, or a request
  built for one.

## Not yet specified

- **Photos layer.** Blocked on an indexer that respects PRD N4 (index and link,
  never copy). EXIF geotags would also be the best travel-history source.
- **MCC capture.** Round 2 captures the address columns (PLC-11); the Trade
  Republic `mcc_code` column is still discarded at import. It is category
  data, not location — its consumer would be categorization, not this map.
- **Self-hosted tiles.** The basemap still egresses to tiles.openfreemap.org,
  the seam `upstreams.toml` already declares swappable to PMTiles.
- **Register proposals from calendar.** §8.2 names calendar shared events as a
  derivation source; trips `travelers` derivation shipped in round 2 (PLC-12),
  calendar has no attendee field yet to derive from.
- **Origin guard hardening.** The default browser-origin allowlist accepts any
  `.ts.net` host (mirroring the dashboard's own allowedHosts). Pinning to this
  tailnet's name is one overlay env away
  (`SJEL_PLACES_ALLOWED_ORIGIN_HOSTS`), unset by default.

## Test Strategy

| isc | type | check | threshold | tool | anchors_to |
| --- | --- | --- | --- | --- | --- |
| PLC-1 | command | `curl :8093/routes` + `cargo test -p sjel-places` route coverage | pass | cargo | F0 |
| PLC-2 | command | geocode twice, count provider requests | 1 | psql | F0 |
| PLC-3 | command | join geocode_cache queries against registry tokens | 0 rows | psql | D3 |
| PLC-4 | command | count venue links vs raw Amex address rows | equal | psql | D1 |
| PLC-5 | command | compare layer sums to projection sums | equal | psql | F1 |
| PLC-6 | command | legs with unresolvable EVA endpoints | 0 | psql | F2 |
| PLC-7 | code inspect | grep write paths for `confirmed`; run the two review tests | review only, and withdrawable | rg, cargo test | D4 |
| PLC-8 | command | `SELECT count(*)` in axon_demo | 0 | psql | D4 |
| PLC-13 | code inspect | fields on `GET /api/people/presence` | count + overlap only | cargo test | D4 |
| PLC-9 | command | fallback-source links with precision='venue' | 0 rows | psql | D1 |
| PLC-10 | command | assign to a city-kind place, read response precision | city | curl | D1 |
| PLC-11 | command | `cargo test -p sjel-finance -- db_tests::`; column presence | pass | cargo | F1 |
| PLC-12 | command | run `backfill travelers` twice; grep output for names | 0 names, idempotent | rg | D4 |
| PLC-13 | command | `cargo test -p sjel-places climate::db_tests`; captured query-string parameter list | exactly 6, 2 decimals | cargo | F4 |
| PLC-14 | command | fetch twice, count provider requests | 1 | cargo | F4 |
| PLC-15 | command | `climate fetch --place <address row>`; count provider requests | 0, non-zero exit | cargo | F4 |

## Anti-claims

- [x] A1 — no geocode provider request carries a person name, an amount or a
  date. Evidence: `GeocodeQuery` carries place text only, every call site
  traced (privacy review 2026-08-25); a transport error no longer prints the
  request URL (`a_transport_error_never_carries_the_query_address`). Falsifier:
  a provider query log line containing any of them.
- [x] A2 — places never writes outside its own schema. Evidence: doctrine
  review swept every INSERT/UPDATE/CREATE target in the crate, 2026-08-25;
  cross-schema access is SELECT-only. Falsifier: an INSERT or UPDATE against
  another capability's schema anywhere in this crate.

## Decisions

- **2026-08-25 — D1–D4** (principal's call, crystallize session): venue-where-
  possible spend granularity, places as its own capability, external geocoding
  with a permanent cache, and the companion register now. Full records with
  measurement context in `README.md` here.

- **2026-08-25 — port 8093, after two collisions.** 8091 belongs to
  foundation-models (repo), 8092 to interior. A repo-only
  `grep '^port' capabilities/*/service.toml` misses overlay service.tomls —
  sweep both roots before assigning a port. (interior was in the overlay when
  this was written and moved here 2026-08-30, PRD Q59; the sweep rule is what
  survives, and it survives because the overlay still holds capabilities.)
- **2026-08-25 — PLC-3 and PLC-4 falsifiers corrected against measurement.**
  The raw registry-token join flags street-name coincidences, and a large
  share of the raw addresses are unresolvable raw-data defects. The claims now
  state what the system guarantees (place-only provenance; report-never-guess)
  instead of a proxy that fails on noise. The measured collisions and miss
  classes are in the overlay evidence note.
- **2026-08-25 — live-data numbers live in the overlay, never here.** This
  file is public. Counts, totals and name tokens measured against the
  operator's data go to `<overlay>/data/places/evidence-2026-08-25.md`; the
  public claims keep only the falsifier and the shape of the evidence.

- **2026-08-25 (round 2) — three principal calls.** Forward-import capture
  lives on `finance.transaction_candidates` (finance keeps what it sees;
  places reads candidates; no requires-cycle). The unplaced backlog gets a
  manual place-this flow on `/map` now, grouped by exact description, rather
  than waiting or guessing from merchant strings. Commits land locally; the
  principal pushes.

## Log

- 2026-08-25 · Scaffolded from the map-layers crystallize session.
- 2026-08-25 · Built: capability + backfills + dashboard `/map` + transit
  coordinate persistence. Registry, transaction links and the first register
  proposals backfilled from live data (counts: overlay evidence note).
  `bazel test //...` fully green; dashboard build + bundle guard green.
  Enabled on this machine; serving on 8093.
- 2026-08-25 · Round 2: forward-import capture (finance), city fallback,
  travelers derivation, reverse geocoding, unplaced endpoints + manual assign
  flow on `/map`. Live-data numbers scrubbed from the public files into the
  overlay evidence note. Gates fully green; three-lens review fixed and
  re-verified; privacy sweep of the public diff clean (counts: overlay
  evidence note, round-2 section).
