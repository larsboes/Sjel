# travel pack

One skill, **`travel`**, for driving Sjel's travel capabilities as a single workflow: calendar,
transit, punctuality, scouting and trips. It carries the ordering between them and the joins that
fail quietly, not their route tables — each capability serves its own manifest at `GET /routes`,
and a copy here would be a copy that goes stale.

## Skills

- `travel` routes a question to the capability that owns it, runs the calendar → transit → trips
  order, and reaches every service through `axon capability call`. Its one reference,
  `references/traps.md`, holds the failures that return an empty or plausible answer instead of
  an error.

Since 2026-09-05 `trips` composes that same order server-side for one shape of question:
`POST /api/plan-search` answers 202 with a job number, ranks candidates on visible factors and
writes nothing until `POST /api/plan-search/:id/adopt` (PRD Q91,
`capabilities/trips/README.md`). The skill's hand-run order stays the path for everything the
job does not cover, and it stays the explanation of why the order is what it is. Two facts a
caller of this Pack needs: `trips` now answers 403 to a browser `Origin` it does not serve the
dashboard from, which `axon capability call` never trips because it sends no `Origin`; and the
job reads `places` for climate normals and companion presence, which `pack.toml`'s
`capabilities` list does not yet name — `axon capability list` is the current answer, this
manifest is not.

## Activate

```sh
axon pack deploy claude travel
axon pack deploy codex travel
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
