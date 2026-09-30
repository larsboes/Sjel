# traveler

The traveller profile: the hard limits a search must obey, the ranking weights
that are the traveller's rather than the repo's, the interests that were already
being typed into a form and read by nothing, and one provenance entry per field
saying whether a value was stated, derived, proposed, or is still the built-in
default.

## Why this exists

Sjel could search and could rank, and the ranking was the same for everyone.
`capabilities/trips/src/plan_search.rs` built destination candidates from
calendar windows, cities, events, climate and presence, then weighed them with
four `const` values — `WEIGHT_BUDGET_FIT` 0.35, `WEIGHT_FEASIBILITY` 0.30,
`WEIGHT_SEASON` 0.20, `WEIGHT_EVENTS` 0.15. It declared a fifth factor,
`FACTOR_RETROSPECTIVE` ("How the last trip here went"), with a doc comment saying
it was *"declared and NOT computed in v1"*.

Meanwhile the personal signal was being captured and not read:

| Signal | Where it was | What read it |
| --- | --- | --- |
| `plan.interests` — free text on every plan | a column on every plan | the page that displayed it |
| The options offered and the one chosen | `option_set` plan items | nothing |
| Real trips — dates, destinations, companions | imported from the vault | nothing |
| Retrospectives | endpoint, form, published formula | zero rows |

Nothing here is a search-engine problem. What was missing was a place for "who is
travelling" to live, and a reason for a solver to ask.

## The profile

Six parts, and every field carries its provenance:

- **`hard`** — limits, not trade-offs. `earliest_departure`, `latest_arrival`,
  `max_changes`, `min_transfer_buffer_min`, `modes`, `avoid_overnight_travel`,
  `home_stations`, `home_airports`, `cards`. The two `home_*` fields are lists,
  best first, because a home is not one platform: the operator named three stations
  and a second airport, and a single value would have silently dropped them.
- **`soft`** — the *destination* ranking weights, keyed **exactly** as
  `plan_search`'s factors are: `budget_fit`, `feasibility`, `season`, `events`,
  `retrospective`. They must sum to 1.0 and a write that does not is refused
  with the block and the sum named.
- **`journey`** — the *connection* ranking weights: `price`, `duration`,
  `changes`, `reliability`. A separate block because the two rank different
  things — is this a good place to go, versus is this a good way to get there —
  and one block would carry keys that mean nothing at one of the grains.
  `plan_search` re-normalises over the factors it could compute, so a key that
  never applies does not merely go unused: it silently takes weight from the
  ones that do.
- **`interests`** — free text, matched against `scouting`'s already-scored
  opportunities.
- **`pace`** — `slow` | `balanced` | `packed`.
- **`anchors`** — what a trip is anchored on, in preference order: `event`,
  `social`, `activity`, `work`, `rest`.

`basis` is a map covering **every** field in `BASIS_KEYS` with one of four
values: `default` (nobody has looked), `stated` (the operator typed it),
`derived` (computed from stored rows), `vault` (proposed from TELOS, not yet
confirmed). A `basis` that omits a field, or names one that does not exist, is
refused — because `0.30` chosen by a person and `0.30` because nobody has looked
yet behave identically and mean completely different things.

## HTTP surface

| Route | What it does |
| --- | --- |
| `GET /api/profile` | The profile plus `stored`. **Never 404s**: an unstated profile comes back with `stored: false` and `revision: 0` |
| `PUT /api/profile` | Replace it. `{ profile, expected_revision? }`. 400 names the rule broken; 409 `stale_profile` carries `current_revision` |
| `GET /api/profile/derived` | What the stored trips actually show — trip length, lead time, destinations, company shape, months, modes, and the plan ids every number came from. Computed on read, stored nowhere |

`GET /routes` serves the manifest, including the write body's schema derived from
the struct serde already deserializes.

## The derived baseline

A read-only projection over `trips`' own tables, the same shape
`capabilities/places/src/layers.rs` uses for its travel layer and for the same
reason: the rows belong to `trips`, and a second copy would be a second thing to
keep true. It answers what the stored history actually shows, which nothing did
before — the stored plans, their dates, companions and interests were all captured
and all unread.

Three limits ride in the response as `notes`, because the counts cannot say them
themselves:

- **Attendance.** A plan is not a trip. Only a retrospective recording
  `not_taken` excludes one; nothing records that any other plan was attended.
- **Lead time.** Measured only over plans whose row was created *before* the trip
  started. The vault import stamps `created_at` with the import date, so an
  imported trip's row age is how long ago the import ran. The first live run
  published a **negative** median before this restriction existed — the row's age
  rendered as a measurement.
- **Destination identity.** Destinations are as stored, so one city typed two
  ways counts as two and a multi-stop plan is a single name.

It answers with absences rather than failing when `trips` has never run on the
machine: `null` for a `Spread` rather than `0`, the distinction `punctuality`
already makes between "no evidence" and "evidence that says zero".

## Absence degrades, it never fails

The whole upgrade rests on two seams, one per grain:
`TravelProfile::stated_weights()` and `stated_journey_weights()` return `None`
when nothing has been stated, and a consumer that gets `None` keeps its own
defaults. A search against a fresh install therefore ranks exactly as
`plan-search-v1` did and returns journeys in the backend's own order, and a
search against a traveller capability that is down behaves the same way. This is
the rule `capabilities/punctuality` already states for an unscored leg, applied
to a whole capability: a missing profile is not a reason for a journey search to
fail.

It is also why adding the `journey` block changed no behaviour on the day it
landed. The column arrived on an existing row with every `journey.*` provenance
reading `default`, so nothing was stated and nothing ranked.

## Decisions

- **D1 — its own capability, not a table in `trips`** (2026-09-23). Four
  capabilities have a reason to read this: `trips` (ranking), `transit` (search
  filtering), `scouting` (event scoring) and `calendar` (window feasibility).
  Putting it in `trips` would give three of them a dependency on a peer, and
  `transit` is today the lower layer. A capability depends on another's contract,
  not its code — so the readers reach this over HTTP and none links a crate.
- **D2 — weights are keyed to `plan_search`'s factor keys, not to a new
  vocabulary** (2026-09-23). A translation table between two names for the same
  factor is where they drift. `plan_search.rs` drops any factor it cannot compute
  and re-normalises the rest, so naming its keys here means a weight is read by
  key and nothing else has to change on the day the fifth factor lands.
- **D3 — `default` is a provenance, not a missing entry** (2026-09-23). The
  alternative — a field absent from `basis` means "default" — makes two different
  states indistinguishable from a typo, and a typo in the honesty mechanism is
  the worst place to have one.
- **D4 — no permissive CORS; the shared origin guard refuses** (2026-09-23).
  This serves where the traveller lives, when they refuse to travel, and later
  which kinds of company they travel in. Following `capabilities/places` rather
  than `trips`: refusing the request outright is what stops a hostile page's
  "simple" cross-site write, and withholding a header does not. The dashboard
  reaches this through its own same-origin proxy, so nothing needs the header.
- **D5 — no `requires = ["trips"]`, ever** (2026-09-23). The profile is derived
  from plans, so a dependency reads naturally. It would also turn "trips is
  restarting" into "the profile is unavailable", which under D-nothing-degrades
  is a search that silently loses its constraints. Derivation reads rows and
  tolerates their absence.

## Known gaps

Stated here rather than left to be discovered:

- **The vault proposal is not built.** D1 in the design conversation chose
  "profile is the interface, the vault proposes, the operator confirms". This
  release ships the interface. `TELOS/Personal/Events Profile.md` is marked
  *"Draft profile, awaiting Lars's corrections"* and nothing reads it into this
  store yet.
- **The derived baseline is built, and nothing reads it.** It publishes what the
  history shows; no ranking consumes it. The offered-versus-chosen half is not in
  it either — the offered options sit on one plan, and comparing them to what was
  picked needs the same treatment.
- **Companion patterns are not built**, and `places_person_places` holds rows all
  in state `proposed`, with the person's name in a plain column rather than only
  in the id. `GET /api/plans` still serves `travelers`.
- **One consumer reads the profile, and it is the journey half.**
  `capabilities/transit/src/ranking.rs` reads the `journey` block over HTTP and
  orders `/api/search` by it, dropping any factor it could not compute. The
  *destination* half still has no reader: `plan_search` is `plan-search-v1` with
  its constants, which is what `plan-search-v2` would change.
