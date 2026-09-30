---
name: travel
description: Drives Sjel's travel capabilities as one workflow — feasible days from calendar, journey search and split tickets from transit, measured delay history from punctuality, event discovery from scouting, and plan state in trips. Use when planning a trip, searching connections or fares, checking how late a train usually runs, or turning a discovered event into an itinerary. Do not use for home network or Home Assistant work, and never work from a remembered route or port — ask the service.
allowed-tools: Bash
---

# travel

Five capabilities answer five different questions, and the order they are asked in decides
whether the answer is worth anything. That order, and the joins that quietly return nothing,
are what this skill carries. The routes are not: every capability serves its own manifest.

```sh
axon capability list
axon capability call transit get /routes
```

## Who answers what

| Capability | Owns | Worth knowing |
|---|---|---|
| calendar | feasibility verdicts, feasible windows, dated commitments | A verdict is soft. `conflicts` is still returned, never hidden |
| transit | journey search, split-ticket chains, rail ticket extraction | Owns no idea why a day is possible |
| punctuality | measured delay history per station, train type and hour | Measured, not predicted, and not transfer risk |
| scouting | opportunity discovery, scoring and event routing | Owns no time and no itinerary |
| trips | the plan and its items | **Owns no search at all.** It stores what was decided |

## The order

1. **calendar** — `GET /api/windows?from=&to=&min_days=` returns the runs of days travel is
   possible in, each carrying the days inside it that would cost a travel day.
2. **transit** — search only those days. Over HTTP that is `GET /api/search?from=&to=&time=`
   per day. For a fuzzy multi-destination fan-out use the CLI: `transit plan --dates` takes the
   day list from step 1 verbatim. There is no `plan` route on the server.
3. **trips** — `POST /api/plans`, then `POST /api/plans/:id/items` for the connection, event or
   stay that was chosen.

Running 2 before 1 samples a month of dates and prices the ones nobody can travel on. Calendar
publishes the days and transit never asks for them; the caller moves the list across. The
argument for that direction is in `capabilities/calendar/README.md` § Calendar hands the days
over.

`--dates` is mutually exclusive with `--date-from`/`--date-to`, and `--max-queries` still caps
the fan-out.

## Calling a capability

```sh
axon capability call <name> <get|post|put|patch|delete> <path> [body] [curl-args...]
axon capability url calendar
```

The body for `post`/`put`/`patch` is one JSON string argument; the content type is set for you.
The wrapper uses `curl --fail-with-body`, so a rejected write still prints the capability's own
reason for rejecting it while the exit code stays non-zero. Read the body before calling
anything broken.

## The composition already exists

```sh
bun tools/travel.ts candidates --json
```

Same assessment the Travel page renders: scouting opportunities matched against trip plans
within 75 km, then calendar verdicts on whatever did not match. It imports the page's own
functions rather than reimplementing them. Read it before writing a matching rule of your own —
a second home for the 75 km rule is how the two answers start disagreeing.

## Before writing anything

Read the owning capability's README once. They carry the contracts, the honest gaps and the
reasons: `capabilities/calendar/README.md`, `capabilities/transit/README.md`,
`capabilities/punctuality/README.md`, `capabilities/scouting/README.md`,
`capabilities/trips/README.md`.

Then read `references/traps.md` — six failures that return an empty or plausible answer instead
of an error.

## Boundary

Stations, home coordinates, calendar credentials and route defaults live in the active overlay,
never in this Pack. Never write a capability's tables directly; every cross-capability hand-off
here goes over HTTP.
