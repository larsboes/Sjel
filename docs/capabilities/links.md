# Typed Ids & `/api/links`

The join contract every capability serves. Sjel is one shell over the capabilities, and its
reason to exist is the joins between them; this contract lets the shell read a join without
knowing which capability holds the other half.

Crate: `sjel-links` (`libs/links`). It holds the id type, the response type and the query check.
It does not depend on axum — each capability writes its own few-line handler, so the response
cannot drift between capabilities.

## Id grammar

Every linkable id is typed `<kind>:<rest>`:

| Kind | Minter | Example |
| --- | --- | --- |
| `trip:plan` | trips | `trip:plan:<nanos hex><seq>` |
| `cal:entry` | calendar | `cal:entry:<nanos hex><seq>` |
| `place` | places, `stable_id` | `place:<hex>` |
| `ent` | entities | `ent:<hex>` |
| `evt` | events | `evt:<hex>` |
| `fin:tx` | finance, `fin:tx:<source_id>` | `fin:tx:<sha256>` |

`Sjel-links` parses `<kind>:<rest>` and refuses an id with no `:` or an empty kind. A finance
row without a `source_id` has no linkable id: `/api/links` does not serve it, and the response
counts what it left out (`unlinkable`), so a short list is never a silent one. The positional
`fin` projection `id` stays as the row key and is never linked to.

`stable_id(kind, identity)` lives in `sjel-links` because the places migration needs it;
`new_id(kind)` is added with the first capability that moves its own minter. Trips and calendar
currently copy the same `<prefix>:<nanos hex><seq>` minter.

## Discovery: `links_to`

A capability that holds references declares the id kinds it can answer for in its `service.toml`:

```toml
links_to = ["trip:plan"]
```

`links_to` reaches the registry JSON through `tools/capability.sh registry` and sjel-status'
`/capabilities`. The shell reads the registry, takes the kind from the id's prefix, and asks
only the capabilities that declare it — no shell code per pair. A capability that does not answer
is named in its group. A replacement for a capability keeps its joins by serving the same route.

The path is under `/api` because finance and calendar declare `proxy_api_only`, and the shell
reaches nothing else on them.

## Response

```json
{ "to": "trip:plan:…", "links": [ … ], "unlinkable": 0 }
```

A link is `{ id, kind, title, at?, meta?, via }`: `via` names the field that holds the reference
(`trip_id`, `payload.plan_id`). `at` and `meta` are omitted when absent. The inspector renders a
linked row the same for every kind, so one renderer serves all joins.

`GET /api/links` without `to`, or with an untyped `to`, answers `400` with a stated reason — never
an empty list.

## Exact links and labelled coincidence

"Same days" and "same place" are inferences, not references. The shell keeps its date-range
queries and shows them under the exact links with their own label, so a reader can always tell a
reference from a guess. The manual "Berlin-Urlaub" calendar entry is the measured case: it shares
the trip's days and holds no reference to it.

## People

A name written elsewhere means a person only after a decision. Four columns named a person as
text; the id sits beside the name rather than replacing it, because about 100 callers read the
name as text.

- **Trips** adds a per-name table `trips_people` (`name_key`, `entity_id`, `decided_at`).
  `sjel_links::name_key` is the key. `entity_id` null means "not a person"; no row means
  undecided. The same shape is `places_people`.
- **Exact matches link without asking.** An exact full name with one match, or an exact email,
  links at once through `POST /entities/api/resolve`; a first-name or ambiguous match goes to the
  review list. No capability matches names itself.
- **A name with no person is offered, never created.** The review list shows "Create person" and
  "Not a person".
- **Mail joins by exact email and stores nothing.** Comms declares `links_to = ["ent"]` and
  answers by asking entities for the person's emails.
- Each capability writes its own exact matches, at startup and after its own writes, and serves
  `GET /api/people/open` and `POST /api/people/decide`. If entities does not answer, names stay
  undecided and nothing fails.

Manual decisions verify the selected ID through entities' `GET /api/entities/{id}` before
writing. A missing entity or a non-person returns `400`; an unavailable verifier, malformed
response, or mismatched response ID returns `503`. Either refusal preserves the saved decision.
An `entity_id: null` decision records "not a person" without contacting entities.

Plan list, detail, create, and update responses project each original traveler name into
`traveler_links`, in the same order. Each entry has `name`, `status` (`resolved`, `not_person`, or `unresolved`), and an
`entity_id` only for a resolved decision. Invalid saved typed IDs remain unresolved. This metadata
is read-only: it is not stored in the plan or forwarded to journey search providers.

Traveler chips open the person inspector by that saved entity ID. Unresolved and not-person
names stay unlinked. Nearby-person and host actions keep the IDs supplied by the located-person
response; a display-name match never creates an inspector target. Name-only meetup participants
also stay unlinked.

## Decisions

| Id | Decision |
| --- | --- |
| D1 | A back-reference route declared in the manifest (`links_to` / `GET /api/links?to=`); no shell code per pair. |
| D2 | Coincidence stays, labelled apart from exact references. |
| D3 | Every linkable id is typed `<kind>:<rest>`; `place_<hex>` → `place:<hex>`, finance links as `fin:tx:<source_id>`, unlinkable rows counted. |
| D4 | The contract is its own crate, `libs/links`; each capability writes its own handler. |
| D5 | One response shape `{ id, kind, title, at?, meta?, via }`, rendered without per-kind code. |
| D6 | Ids mint in one place; `stable_id` in `sjel-links`, `new_id` added with the first mover. |
| D7 | The person id sits beside the name, in a per-name table, not on every row. |
| D8 | Exact matches link without asking; the rest wait on the review list. Name rules are entities'. |
| D9 | A name with no person is offered, never created. |
| D10 | Mail joins by exact email and stores nothing. |
| D11 | The owner writes, and no new vocabulary; one decision covers every row of that name. |

The table records the contract decisions; Git history retains their discussion.

## Claims

All shipped; the paths are the evidence.

| Id | Claim |
| --- | --- |
| LNK-1…4 | `sjel-links` parses and refuses ids; `Link` serializes; `/api/links` 400s an untyped `to`; response is `{to, links, unlinkable}`. |
| LNK-5…6 | `links_to` reaches the registry; the shell asks only declaring capabilities. |
| LNK-7…9 | Finance answers `to=trip:plan:…`; calendar answers from `payload.plan_id`; places answers `to=fin:tx:…`. |
| ~~LNK-10~~ | Withdrawn: inventory holds no reference to entities. |
| LNK-11…12 | Places' `stable_id` returns `place:<hex>`; the migration is idempotent and backs up first. |
| LNK-13 | `connections.ts` holds no capability pair. |
| LNK-14 | `POST /entities/api/resolve` answers `exact`, `first`, `ambiguous` or `none`. |
| LNK-15…17 | Trips, places and comms store/link/answer people as above. |
| LNK-18 | `/people` lists open names; a person lists the trips, places and mail that reference it. |

## Known limits

- `trips_plans.travelers` is served by `GET /api/plans`; the companion summary is not published
  per traveller until the register's person id and a class column exist (see root `ISA.md`).

## Out of scope

- **Trips' own place namespace.** A destination's `obsidian-place:…` id does not resolve to the
  places registry, so a place cannot list the trips that went there. Resolving it is a trips
  decision.
- **Import and export formats.** They are a separate contract; see root `ISA.md`.
