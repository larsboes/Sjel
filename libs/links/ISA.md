---
project: sjel-links
type: isa
phase: climbing
progress: 95
principal_stated_goal: "our connection is the outstanding point … so our system is interchangeable"
---

# ISA · links

Lib-scoped state of record for `libs/links` (crate `sjel-links`). Repo-wide items stay in the
root `ISA.md`. The why is in the private vault, `Projects/Sjel/Connected shell.md`.

## Problem

Sjel is one shell over 45 capabilities, and its reason to exist is the joins between them. On
2026-10-06 the dashboard inspector started showing joins (`73dec7ee`), but it hard-codes three
of them in `dashboard/src/lib/inspector/connections.ts`. Each new pair means more shell code,
and a capability replaced by another tool loses its joins.

Measured 2026-10-06:

- Three stored references cross a capability boundary: finance's `axon-trip-id` tag → trips,
  calendar `payload.plan_id` → trips (entries trips wrote, `source = "trips"`), and places'
  `transaction_places.source_id` → finance (456 rows). Trip travellers and places'
  `person_places.person` are names, not entity ids.

  Correction, 2026-10-06: this list first named trips `place_id` → places and inventory
  `entity_id` → entities. Both came from a census of struct fields, not of stored data. Trips
  stores no place id: its destinations carry its own namespace (`obsidian-place:…`), and
  `place_id` exists only in a computed response (`capabilities/trips/src/bases.rs`).
  Inventory's `entity_id` is its own item id inside a sync mutation. Places → finance was
  missing; a count of stored columns found it.
- Most ids carry their type: `trip:plan:…`, `cal:entry:…`, `ent:…`, `evt:…`. Two do not.
  Places mints `place_<16 hex>` from `stable_id` (`capabilities/places/src/store.rs:1010`).
  Finance serves `transaction_{index}_{posting}_{currency}`
  (`capabilities/finance/src/analytics.rs:246`), where `index` is the position in the journal,
  so one inserted earlier transaction renumbers every later one.
- Finance's stable identity is `source_id`, the SHA-256 candidate fingerprint. Places already
  links transactions by it (`capabilities/places/src/store.rs:233`). It is `Option`: a
  hand-written journal entry has none.
- `service.toml` is single-line `key = value` or `key = ["a", "b"]` on purpose
  (`tools/lib/toml.sh`, header), and every field reaches the shell as a string through
  `tools/capability.sh registry` and sjel-status' `Service` struct.

## Decisions

> [!done] D1 — answered 2026-10-06: **a back-reference route, declared in the manifest.**
> A capability that holds references declares the id kinds it can answer for, as
> `links_to = ["trip:plan"]` in its `service.toml`. It serves `GET /api/links?to=<id>` and answers
> with its own rows that reference that id. The shell reads the registry, takes the kind from
> the id's prefix, and asks only the capabilities that declare it. No shell code per pair.
>
> The path is under `/api` because finance and calendar declare `proxy_api_only`, and the shell
> reaches nothing else on them.
>
> A replacement for a capability keeps its joins by serving the same route. This is the
> "interchangeable" half of the goal. The rejected options: join strings in the manifest (a
> mini-language `toml.sh` cannot check, and each capability needs a filter route anyway), and a
> typed declaration in `/routes` (discovery would need every capability running, and 27 of 45
> do not link `route-manifest`).

> [!done] D2 — answered 2026-10-06: **coincidence stays, labelled apart.**
> "Same days" and "same place" are inferences, not references. The shell keeps its date-range
> queries and shows them under the exact links with their own label. A reader can always tell a
> reference from a guess. The manual "Berlin-Urlaub" entry is the measured case: it shares the
> trip's days and holds no reference to it.

> [!done] D3 — answered 2026-10-06: **every linkable id is typed `<kind>:<rest>`, and the two
> that are not get migrated.**
> Places: `place_<hex>` becomes `place:<hex>`. `stable_id` changes its separator, and a
> migration rewrites places' own tables once, idempotently, through `sjel_store::migrate_once`.
> Trips was named here too; it stores no place id (Problem, correction). The hash is unchanged, so a re-derived id matches a
> migrated one.
>
> Finance: a transaction's linkable id is `fin:tx:<source_id>`. A row without `source_id` has
> no linkable id. `/api/links` does not serve it, and the response counts what it left out
> (`unlinkable`), so a short list is never a silent one. The positional `id` stays as the
> projection's row key and is never linked to.
>
> Overruled my recommendation (declare `id_prefix`, migrate nothing). The reason given: one id
> rule everywhere, rather than a table of exceptions the shell has to consult.

> [!done] D4 — answered 2026-10-06: **the contract is its own crate, `libs/links`.**
> It holds the id type, the response type and the query check. It does not depend on axum: each
> capability writes its own few-line handler. Every capability that serves `/api/links` links it,
> so the response cannot drift between capabilities.

> [!done] D5 — decided 2026-10-06 without a question: **one response shape, rendered without
> per-kind code.**
> A link is `{ id, kind, title, at?, meta?, via }`: `via` names the field that holds the
> reference (`trip_id`, `payload.plan_id`). The inspector renders that row the same for every
> kind. It keeps per-kind code only for the item's own header. Reason: a per-kind renderer for
> linked rows is the shell code per pair that D1 removes.

> [!done] D6 — decided 2026-10-06 without a question: **ids mint in one place.**
> Trips (`generated_id`, `capabilities/trips/src/store.rs:302`) and calendar each copy the same
> `<prefix>:<nanos hex><seq>` minter. `sjel-links` owns `stable_id(kind, identity)` now, because
> the places migration needs it. `new_id(kind)` is added with the first capability that moves its
> minter. No forced sweep.

### People (asked 2026-10-06)

Measured 2026-10-06, distinct values, counts only. Entities holds 293 people. Four columns
elsewhere name a person as text:

| Column | Values | Exact name, one person | First name, one person | First name, several | No match |
|---|---|---|---|---|---|
| `trips_plans.travelers` | 17 | 5 | 6 | 3 | 3 |
| `places_person_places.person` | 15 | 3 | 5 | 3 | 4 |
| `comms_triage_items.from_addr` | 215 | — | — | — | 4 match a person's email |
| `comms_feed_items.author` | 672 | 0 | 0 | 0 | all |

Feed authors are writers of what the operator reads, not people the operator knows. They are
out of scope.

> [!done] D7 — answered 2026-10-06: **the id sits beside the name.**
> Trips keeps `travelers` as written and adds `traveler_ids`, a JSON map from a name to an
> `ent:` id, or to `null` for "not a person". A name with no key is undecided. Places keeps
> `person` and adds `entity_id` and `entity_state` (`open`, `linked`, `not_person`). No caller
> of `travelers` changes: about 100 uses read it as text. Rejected: aliases in entities (every
> answer would need entities up) and replacing names with ids (every caller and a migration).
>
> Refined 2026-10-06 while building, same choice: the id lives in a per-name table in each
> capability (`trips_people`, `places_people`: `name_key`, `entity_id`, `decided_at`), not in a
> column on every row. D11 rules one decision per name, and a table keyed by name enforces it;
> a map on each plan would let two plans disagree about the same "Lucia". The plans and
> `person_places` tables do not change. `entity_id` null records "not a person"; no row means
> undecided. The key is `sjel_links::name_key`, which entities' duplicate finder now uses too.

> [!done] D8 — answered 2026-10-06: **exact matches link without asking; the rest wait.**
> An exact full name with one match, or an exact email, links at once. A first-name or
> ambiguous match goes to the review list. The name rules are entities' own
> (`capabilities/entities/src/duplicates.rs`, `norm_name` and the first-name rule), served as
> `POST /entities/api/resolve`. No capability matches names itself.

> [!done] D9 — answered 2026-10-06: **a name with no person is offered, never created.**
> The review list shows "Create person" and "Not a person". Nothing becomes an entity on its
> own, so a typo or a group ("family") does not become a person.

> [!done] D10 — answered 2026-10-06: **mail joins by exact email, and stores nothing.**
> Comms declares `links_to = ["ent"]` and answers by asking entities for the person's emails,
> then matching `from_addr`. Mail is never on the review list: an email matches or it does not.

> [!done] D11 — decided 2026-10-06 without a question: **the owner writes, and no new
> vocabulary.** Each capability links its own exact matches, at startup and after its own
> writes. If entities does not answer, the names stay undecided and nothing fails. A
> capability that declares `ent` in `links_to` and stores names also serves
> `GET /api/people/open` and `POST /api/people/decide`; the shell finds them through
> `links_to`, as it finds `/api/links`. One decision covers every row of that name in that
> capability, and the list shows how many rows that is.

## Goal

Opening a trip in the inspector shows the transactions and calendar entries that reference it,
read through `/api/links` from capabilities discovered in the registry, with no trip-specific code
in the shell. The same holds for a place, an entity and a calendar entry.

## Criteria

### F0 · The contract

- [x] LNK-1 — `sjel-links` parses `<kind>:<rest>` and refuses an id with no `:` or an empty kind.
- [x] LNK-2 — `Link` serializes to `{id, kind, title, at, meta, via}`; `at` and `meta` are
      omitted when absent.
- [x] LNK-3 — `GET /api/links` without `to`, or with an untyped `to`, answers 400 with a stated
      reason, never an empty list.
- [x] LNK-4 — the response is `{ to, links: [...], unlinkable: n }`.

### F1 · Discovery

- [x] LNK-5 — `links_to` reaches the registry JSON from `tools/capability.sh registry` and from
      sjel-status' `/capabilities`.
- [x] LNK-6 — the shell asks only the capabilities whose `links_to` holds the id's kind. A
      capability that does not answer is named in its group.

### F2 · The four measured references

- [x] LNK-7 — finance answers `to=trip:plan:…` with `fin:tx:<source_id>` rows; `unlinkable`
      counts tagged rows without `source_id`.
- [x] LNK-8 — calendar answers `to=trip:plan:…` from `payload.plan_id`.
- [x] LNK-9 — places answers `to=fin:tx:…` from `transaction_places.source_id`. Replaces "trips
      answers `to=place:…`", which had no stored reference to answer from.
- [ ] LNK-10 — withdrawn 2026-10-06: inventory holds no reference to entities (see Problem).

### F3 · Migration

- [x] LNK-11 — places' `stable_id` returns `place:<hex>`; the same identity hashes to the same
      hex as before.
- [x] LNK-12 — the migration rewrites `place_` ids in places once; a second run
      changes nothing; a backup of the shared store precedes the first run on a real machine.

### F4 · Shell

- [x] LNK-13 — `connections.ts` holds no capability pair; exact links come from `/api/links`, and
      the date-range groups are labelled as coincidence.

### F5 · People

- [x] LNK-14 — `POST /entities/api/resolve` answers each name with `exact`, `first`,
      `ambiguous` or `none` and its candidates, and each email with the person it belongs to.
- [x] LNK-15 — trips stores `traveler_ids`, links exact matches itself, serves
      `/api/people/open`, `/api/people/decide`, and answers `/api/links?to=ent:…`.
- [x] LNK-16 — places does the same on `person_places`. A dismissed row is neither a link nor
      open (ISA PLC-7); proposed and confirmed rows are linked and say which.
- [ ] LNK-17 — comms answers `/api/links?to=ent:…` from `from_addr` and the person's emails.
- [ ] LNK-18 — `/people` lists the open names across capabilities, with create, link and
      not-a-person; a person in the inspector lists the trips, places and mail that reference it.

## Verified

2026-10-06, against live services through the dashboard proxy:

- The registry carries `links_to` for calendar and finance, and nothing else.
- `GET /calendar/api/links?to=trip:plan:…` returns the Berlin trip's two legs with
  `via: payload.plan_id`. The manual "Berlin-Urlaub" entry is not among them: it holds no reference.
- `GET /finance/api/links` answers 400 for no `to`, for an untyped `to`, and for a kind finance does
  not declare. For the Berlin trip it returns an empty list: no transaction in the ledger carries an
  `axon-trip-id` tag yet, on any trip. The join is tested in `capabilities/finance/src/links.rs`.
- In the inspector, the trip lists its legs under "Calendar" and the manual entry under "Same days"
  (inferred). A leg lists its trip as a reference, and the trip is not repeated as an inference.

The places migration ran on 2026-10-06 after `tools/backup.sh store` wrote
`store-20261006T061313Z.tar.gz` (integrity check ok, upload confirmed). It renamed 989 values in
eight columns: `place_` in `places.id` and the five `place_id` columns (966), `pp_` (19) and `visit_`
(4). `pp` and `visit` rename too because they mint through the same `stable_id`. Afterwards no
value has the old shape and `pragma_foreign_key_check` returns nothing. `GET /places/api/places`
and the spend layer still answer. A ledger transaction at Phantasialand now shows its place in the
inspector, through `GET /places/api/links?to=fin:tx:<source_id>`.

LNK-8 has one consequence for calendar: `Entry::payload` was inert evidence for every provider.
`payload.plan_id` is now read on rows with `source = 'trips'`, and on no others.

## Out of scope

- People by name. Trip travellers and `person_places.person` are strings; turning them into
  `ent:` ids is the entities capability's decision, not this contract's.
- Trips' own place namespace. A destination's `obsidian-place:…` id does not resolve to the
  places registry, so a place cannot list the trips that went there. Resolving it is a trips
  decision.
- Import and export formats. They are a separate contract, in the same note's direction section.
