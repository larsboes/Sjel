# Transit: measured reliability, from the daily corridor to anywhere

Sjel answers "which way gets the user there on time", not only "which way is fastest on paper".
It measures how reliable each leg is, watches the places the user is about to be, and changes
the advice as conditions change. The pilot is the Köln–Bonn corridor. Every layer is built so a
second region costs an adapter and configuration, not a redesign.

Three horizons, in this order:

1. **The daily corridor.** Köln Hbf to Bonn on the right bank of the Rhine. A bridge closure moves
   road traffic onto the remaining Rhine crossings, so buses and replacement buses (SEV) on that
   side are unreliable, and rail plus segregated tram routes may arrive earlier despite a longer
   schedule.
2. **Domestic short trips.** Intercity trips to cities such as Berlin, Munich and Hamburg, often
   planned at short notice. Long-distance rail is the best-measured part of the data today.
3. **Spontaneous and international trips.** Trips decided days or hours ahead, including other
   countries, where Sjel has no history and must start from priors and live sources.

## 1. Facts (checked 2026-10-27)

**Live times.** `capabilities/transit/src/hafas.rs` queries bahn.de's undocumented web API and
reads scheduled and realtime (`ez*`) times. That realtime value is DB's own reactive prognosis:
a delay already observed, carried downstream.

**History.** `capabilities/punctuality` folds
[piebro/deutsche-bahn-data](https://github.com/piebro/deutsche-bahn-data) (DB Timetables API,
sampled four times a day, published monthly as parquet) into one 128-bucket delay histogram per
`CellKey { eva, train_type, hour, weekend }` (`capabilities/punctuality/src/ingest.rs`).
`punctuality-ingest` refreshes it daily. Publication lags a journey by up to five weeks
(`capabilities/punctuality/README.md`).

**Journey score.** `capabilities/transit/src/punctuality.rs` `compose_reliability` multiplies,
per transfer, P(arriving train's delay < scheduled buffer), read off the histogram at that exact
buffer, and the final leg's P(delay < 6 min). An unscorable leg leaves the journey unscored
rather than guessed.

**Gaps, each confirmed in code:**

1. **Rail only.** The dataset covers DB stations and trains. Trams, buses and SEV have no
   history, and they are exactly the legs that fail on the pilot corridor.
2. **Cells by train type, not line.** `train_type_of_label("RE5")` returns `"RE"`
   (`transit/src/punctuality.rs`), so every RE line at a station shares one histogram. The
   parquet carries `train_name` (`punctuality/src/ride.rs` reads it), so a line-level key is
   possible. Whether `train_name` names the line for regional trains is unverified.
3. **Independence.** Transfer probabilities are multiplied as independent events. One
   disruption or one congested road delays several legs at once, so the product is too
   optimistic.
4. **No headway.** A missed transfer costs nothing in the score beyond the probability. The cost
   of waiting for the next vehicle (10 minutes on a tram, 30 on a regional bus) is not modelled.
5. **No disruption notices.** HAFAS returns remarks per journey (the warning icons in DB
   Navigator). `hafas.rs` does not parse them.
6. **No departure boards.** `hafas.rs` has no departure or arrival board client, so Sjel cannot
   observe a line it is not searching a journey for.
7. **No regime awareness.** History is pooled over all months. A bridge closure is a regime
   change, and months before it describe a different road network.
8. **No always-on host.** Scheduled jobs run on the Mac, which is not on around the clock. The
   phone cannot poll reliably in the background on iOS. Nothing observes the network while both
   are idle.

## 2. Decisions

- **Pilot corridor: Köln Hbf to Bonn, right bank** (decided 2026-10-27). Concrete home, work and
  target stops live in the overlay, never in this repository.
- **The Mac is not an always-on host** (decided 2026-10-27). Sjel must work for a user whose only
  computer is a laptop in normal use. Collection and pre-departure alerts run on a host that is
  independent of the Mac. The Mac merges what that host collected when it wakes.
- **Store aggregates and the user's own rides. Fetch everything else** (decided 2026-10-27).
  Timetables, live positions, road traffic and map data stay with their providers and are cached
  with a time-to-live only for an upcoming trip or a watched region.
- **Histograms are the unit of exchange** (decided 2026-10-27). They add, so a collector on any
  host and the Mac can merge in any order without double counting, as long as each batch carries
  an idempotency key. This is what makes an occasionally-connected Mac safe.
- **Reliability is judged by calibration** (decided 2026-10-27). A forecast that says 80 % must
  come true about 80 % of the time. DB's own prognosis is the baseline to beat (§6).

## 3. Shape

```mermaid
flowchart TD
  P[Anticipated presence<br/>calendar, trips, places, routine] --> W[Watch set<br/>stops × lines × time windows]
  W --> C[Collector<br/>pure poll function, any host]
  S1[HAFAS boards] --> C
  S2[GTFS-RT, EFA, other regions] --> C
  C --> H[Observation histograms<br/>line × stop × hour × day type × regime]
  R[User's realised rides] --> H
  H --> M[Leg model<br/>line@stop → line → mode@region → mode prior]
  L[Live conditions<br/>remarks, upstream position, road traffic] --> E
  M --> E[Journey evaluator<br/>correlated legs, headway fallbacks]
  E --> D[Decision<br/>P(arrive before the calendar deadline)]
  D --> A[Advice and alerts on the phone]
```

### Layers

1. **Anticipated presence.** Where the user will be, and when, from calendar entries, trip plans,
   places and routine. It produces the watch set. A user who travels spontaneously gets a wider
   watch radius. One who plans gets a narrower, deeper one. v1 is a hand-declared corridor in
   the overlay.
2. **Observation.** One schema for every source:
   `Observation { stop_id, route_id, trip_id, mode, scheduled, observed, realtime, cancelled, source }`.
   IDs follow GTFS, the one transit schema published worldwide. `realtime = false` means a
   delay of zero is not evidence of punctuality, and such rows are kept out of the delay
   histogram in the same way `punctuality` keeps cancellations out.
3. **Leg model.** Hierarchical fallback with shrinkage: line at stop, then line, then mode in
   the region, then a global prior per mode and right-of-way class. Right-of-way comes from
   OpenStreetMap (`railway=tram` with or without `embedded=yes`, `busway`, `lanes:bus`) and GTFS
   `route_type`, so a region with no history still gets a defensible spread. Recent observations
   outweigh old ones, and a declared regime (for example "bridge closed") splits the history.
4. **Live conditions.** HAFAS remarks, the current delay of the incoming vehicle upstream, and
   road traffic on named segments of the routes that use roads. These override history while
   they are fresh.
5. **Journey evaluator.** Samples leg delays with shared shocks for legs that share a cause,
   and models a missed transfer as a wait for the next departure (headway from the timetable).
   The result is a distribution of arrival times, not one number.
6. **Decision.** The objective is P(arrival before the deadline), with the deadline taken from
   the calendar entry the journey serves. The traveller profile (`capabilities/traveler`) sets
   the risk tolerance: a cautious profile optimises a high quantile, a relaxed one the median.

### Placement (proposed)

- `punctuality` keeps measured reliability. It gains a second source (the collector) and a
  line-level, mode-aware key next to the existing rail cells.
- The collector's poll logic is a pure crate with no filesystem or SQLite access, so the same
  code runs on the Mac and compiles to WebAssembly for a remote host.
- `transit` keeps journey search and evaluation: headway, correlation and the deadline objective.
- `places`, `calendar` and `trips` feed anticipated presence through their existing contracts.

## 4. Open decisions

These need an answer before the work package that depends on them starts.

1. **Always-on host** (blocks T5). Candidates:

   | Host | For | Against |
   |---|---|---|
   | Cloudflare Worker with cron triggers and D1 | Free tier, no hardware, available now | Location routine leaves the user's machines. bahn.de may refuse datacenter addresses (unverified). WebAssembly build of the collector. |
   | Home server (future) | Data stays home, same Rust binaries as the Mac | Hardware not bought yet. Home uplink and power are single points of failure. |
   | Small VPS | Plain Linux, same binaries | Monthly cost, one more machine to patch |

   Proposed: write the collector as the pure crate above, run it on the Mac first, and choose the
   host after spike T0 has shown whether bahn.de answers from a datacenter address.
2. **Location data off the user's machines.** A remote collector learns the watch set: which
   stops, at which hours, on which days. That is a daily routine. Decide whether the watch set may
   leave the user's devices in clear text, or whether the remote host only receives stop and line
   IDs padded with decoy stops.
3. **Priority.** `PLAN.md` lists this area under Priority 3. The Travel tab in Priority 1 would
   show its results. Decide whether T1–T4 move up.

## 5. Work packages

### Open

| # | Work | Done when |
|---|---|---|
| T0 | **Spike: what is observable.** For every line on the pilot corridor (RE5, RB48, S12, S19, Stadtbahn 66 and the right-bank trams, buses 606 and 607, SEV), record over three weekday mornings whether bahn.de and the VRS sources return realtime departures, and whether bahn.de answers from a datacenter address. | A table in this plan: line × source → realtime yes, no or partial, with the probe command. Feeds decision 4.1. |
| T1 | **Line-level cells.** Key `punctuality` cells by line where `train_name` carries one, falling back to train type. | `punctuality stats` separates RE5 from other RE lines at Köln Hbf. Existing rail lookups answer unchanged when no line is given. |
| T2 | **Disruption remarks.** Parse HAFAS remarks into each leg and journey. | A journey with a warning in DB Navigator carries the same remark text in `transit` search output. A fixture test pins the parser. |
| T3 | **Headway in the score.** Add the expected cost of a missed transfer, P(miss) × time to the next departure of the same line, to the journey result. | Two journeys with equal catch probability but different headways rank differently. Unit test with fixed inputs. |
| T4 | **Collector on the Mac.** Departure-board client in `hafas.rs`, the `Observation` schema, the pure poll crate, and a scheduled job polling the overlay watch set into line-level histograms. | Two weeks of weekday-morning observations for every line T0 marked observable. A re-run of the same batch leaves the histograms unchanged. |
| T5 | **Collector off the Mac.** Run the T4 crate on the host chosen in decision 4.1. The Mac pulls and merges batches on wake. | The Mac stays closed for three weekdays, and after it wakes the histograms hold those mornings exactly once. |
| T6 | **Realised rides.** After a journey the user took, refresh it from HAFAS and store scheduled and realised times per leg as ground truth. | Each ride the user marks as taken has realised times or an explicit "no realtime" per leg. |
| T7 | **Corridor report.** `sjel transit corridor <name>` compares the corridor's route families by median, P90, P(on time for a given deadline) and sample size. | The pilot corridor's three route families compared from T4 data, every figure printed with its `n`. |
| T8 | **Pre-departure advice on the phone.** For a calendar entry with a location on a watched corridor, recommend a route and departure time, and notify when live conditions change the recommendation. | Works on a device for one real morning, with the Mac closed. |

### Later

- Road traffic for road legs: named segments on the routes, polled only inside watch windows.
- OpenStreetMap right-of-way priors and the hierarchical leg model for regions with no history.
- Correlated sampling in the evaluator (layer 5), replacing the independent product.
- Domestic horizon: reachability for a free weekend from the user's location, ranked by price
  (`sparpreis-watch`), duration and measured reliability.
- International horizon: GTFS and GTFS-RT feeds found through a feed catalogue (Mobility Database
  or Transitland, licence review in `upstreams.toml` first), cached per trip.

## 6. Success measures

| Measure | Probe | Threshold |
|---|---|---|
| Forecasts are calibrated | Realised rides (T6) bucketed by forecast P(on time) | Each bucket with at least 20 rides within 10 points of its forecast |
| Better than DB's own prognosis | Brier score of Sjel's P(on time) against one derived from DB's realtime delay at departure, over the same rides | Lower than the DB baseline |
| Works with the Mac closed | T5 done-when check | Passes |

Not yet measured: every threshold above. No realised rides are recorded today.
