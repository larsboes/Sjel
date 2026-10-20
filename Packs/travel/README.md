# travel pack

One skill, **`travel`**, for driving Sjel's travel capabilities as a single workflow: calendar,
transit, punctuality, scouting and trips. It carries the ordering between them and the joins that
fail quietly, not their route tables — each capability serves its own manifest at `GET /routes`,
and a copy here would be a copy that goes stale.

## Skills

- `travel` routes a question to the capability that owns it, runs the calendar → transit → trips
  order, and reaches every service through `sjel capability call`. Its one reference,
  `references/traps.md`, holds the failures that return an empty or plausible answer instead of
  an error.

Since 2026-09-05 `trips` composes that same order server-side for one shape of question:
`POST /api/plan-search` answers 202 with a job number, ranks candidates on visible factors and
writes nothing until `POST /api/plan-search/:id/adopt` (PRD Q91,
`capabilities/trips/README.md`). The skill's hand-run order stays the path for everything the
job does not cover, and it stays the explanation of why the order is what it is. Two facts a
caller of this Pack needs: `trips` now answers 403 to a browser `Origin` it does not serve the
dashboard from, which `sjel capability call` never trips because it sends no `Origin`; and the
job reads `places` for climate normals and companion presence, which `pack.toml`'s
`capabilities` list does not yet name — `sjel capability list` is the current answer, this
manifest is not.

## Activate

```sh
sjel pack deploy claude travel
sjel pack deploy codex travel
```

## Deployment status

Not deployed to any harness as of 2026-09-10 — recorded, not a build decision: the capability
chain it drives (calendar / transit / punctuality / scouting / trips) is present in this
machine's registry, so the Pack is deployable where its capabilities run. It awaits the Phase 2
profile decision; travel is a profile member candidate.

## Ownership boundary

Sjel owns the workflow. The active overlay owns every value that makes it personal: home and
destination stations, the geo policy behind scouting's event routing, the calendar credential and
home timezone, and the shared database file. Nothing in this Pack names a station, a city or
a route.

The capability READMEs stay the contracts. This Pack points at them rather than restating them,
so a change to a capability is a change in one place.

## Known limits

- **No trip outcome is recorded.** `POST /api/plans/:id/outcome` waits for the next real trip.
  Until then the transfer buffer stays uncalibrated, the journey-reliability independence
  assumption stays untested, and nothing is worth an Obsidian export.
- **The retrospective's companion half** needs a key that is the people register's person id
  rather than a name, and a class column a mechanism reads. Neither exists. The origin refusal
  on trips, its first precondition, shipped on 2026-09-05.
- **No cancelled train has appeared live.** `originCancelled` and `destinationCancelled` are
  tested only against fixtures.
- **Planning gaps the PRD named.** Nothing composes a route, nothing answers "where should I base
  myself", and accommodation has no source.
