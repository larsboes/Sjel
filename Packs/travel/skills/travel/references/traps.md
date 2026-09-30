# Travel traps

Seven failures that return an empty or plausible answer rather than an error. Each names the file
it was verified against.

## An EVA number joined across transit and punctuality matches nothing

punctuality stores and returns EVA numbers zero-padded to eight digits (`08000044`). HAFAS, and
therefore transit, returns them unpadded (`8000044`). Compare the two as strings, or query
`punctuality.stop_stats` directly with a transit EVA, and you get zero rows — which is exactly
what "this station has no history" looks like. It presented that way the first time
(`capabilities/punctuality/src/store.rs`, `normalize_eva`).

The HTTP surface is safe in one direction only. `POST /lookup` and `GET /stations?eva=` both run
the argument through `normalize_eva`, so either form goes in. What comes back is the padded
stored form, so a caller that keys a map on a response `eva` and looks it up with transit's
version finds nothing.

A second half of the same trap: a transit station id is not always an EVA. The suggest endpoint
answers a bare `8000044`; a journey search answers
`A=1@O=Bonn Hbf@X=…@Y=…@U=80@L=8000044@i=…@`, where the EVA is the `L=` field. Reading `id` as an
EVA works for one of those and silently finds nothing for the other.
`transit::punctuality::eva_of` is the parser that handles both.

## A station name returns zero journeys with HTTP 200

`transit`'s search used to pass its `from`/`to` straight to HAFAS, which accepts an EVA id and
returns **nothing at all** for a name — an empty array with HTTP 200, indistinguishable from "no
trains that day". So `?from=Köln Hbf&to=Berlin Hbf` was a confident empty answer for a
well-formed query, and the first sign of it was a ranked search that had nothing to rank.

Since 2026-09-23 the server resolves a name through the same suggest surface the UI uses, and
refuses one that does not match rather than guessing. Two things still worth knowing:

- **The resolution is deliberately strict.** Every significant word of the query must appear in
  the resolved station's name. HAFAS's suggest is very fuzzy — `Nowhere At All` returns
  `Hannover Karl-Wiechert-Allee` as its *first* hit, because `Alle` matches `Allee` — and a wrong
  destination the caller cannot detect is worse than a refusal. The cost is that a query spelled
  without an umlaut (`Munchen`) is refused.
- **A raw EVA id still skips the lookup entirely**, which is what to do when the resolution
  refuses something real.

(`capabilities/transit/src/server.rs`, `resolve_station` and `matches_query`.)

## `travel_candidate` is a value, not a route and not a filter

It is one of four values of `event_route.route` on an opportunity (`local`,
`travel_candidate`, `online`, `unresolved`), produced by `classify_opportunity` /
`classify_ranked` in `capabilities/scouting/src/event_route.rs`. There is no
`/travel_candidate` path, and no query parameter narrows to it: scouting's `DiscoverParams`
accepts `adapter`, `location`, `query` and `limit`, and nothing else
(`capabilities/scouting/src/server.rs`). `opp_embeddings` was a fifth parameter until it was
removed as a path-injection sink; the embedding file is named by the config key
`opp_embeddings_path` or the `--opp-embeddings` CLI flag, never by a request.

Narrow client-side, or let `bun tools/travel.ts candidates` do it: the narrowing happens inside
`assessTravelCandidates`, which is the one implementation.

`unresolved` means the geo policy could not decide safely, not "far away". Treating it as a
travel candidate invents a journey out of missing evidence.

## `GET /discover` writes

It reads like a query and is a scan. `discover_handler` hands a mutable `Store` to
`pipeline::run`, which calls `st.upsert(...)` for every scored opportunity
(`capabilities/scouting/src/pipeline.rs`, lines 63-74). So the GET fetches the source over the
network and persists a row for everything it scored. `new_count` and `store_total` in the
response are the tell.

For a read, use `GET /opportunities`: it lists from the store and classifies each row without
writing. That is the one `tools/travel.ts` uses.

## A 404 from `GET /api/split` means "no cheaper split exists"

`hafas_fail` maps `NoSplitFound` to 404 deliberately — answering 500 made the absence of a
bargain look like a broken server (`capabilities/transit/src/server.rs`). `sjel capability call`
exits non-zero on it while printing the body, so a script that reads the exit code alone reports
an outage that did not happen.

A chain that *is* returned is not automatically buyable either. Each segment's fare comes from
its own search, so a segment can be priced off a train two hours later. `segments[].train_match`
says which (`exact` / `partial` / `different` / `unknown`), `confidence` is the chain's worst
case, and `unpriced_pairs` says how many fare lookups came back empty — the chain shown is fully
priced by construction, but the table it was chosen from had holes. `savings` is `null` when no
direct fare came back, not `0.0`.

## `delay_risk_score: null` is ordinary

The field is `share_late_6` from punctuality for the arriving train's type at the destination in
the arrival hour. Expect `null` whenever punctuality is not running, has never ingested, or holds
fewer than 30 observations for that cell. Rather than fall back to a neighbouring hour or a
station average, punctuality answers `null`, because a substituted number is indistinguishable
from a measured one downstream. transit degrades and never fails: same journeys, HTTP 200, score
`null` (`capabilities/transit/src/punctuality.rs`).

It is also not the probability the trip works out. It says nothing about catching a transfer, and
a journey whose first leg is late enough to miss one arrives on a different train than the one the
number describes.

## Calendar's `ends_at` is exclusive and every form shows it inclusive

An all-day entry covering only 2026-08-14 stores `starts_at = "2026-08-14"`,
`ends_at = "2026-08-15"`. A trips plan's date window is inclusive, so handing it to calendar
means converting the `to` bound; forgetting moves the entry, or the query, by a day. The
conversion lives once, in `whenOf` / `whenPatch` / `whenError` in
`dashboard/src/lib/calendar/types.ts` — do not write a second one.
