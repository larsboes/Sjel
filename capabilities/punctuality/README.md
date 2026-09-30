

# punctuality

How reliable a German train actually is, measured rather than predicted.

This capability folds Deutsche Bahn's own published stop history into per-station
statistics and answers questions about them. It is deliberately the bottom rung of the
intelligence ladder (CONTRIBUTING.md#implementation-languages-and-intelligence): a lookup table over millions of observed stops
is not a placeholder for a model, it is the number a model has to beat. Without it,
"our prediction is good" is not a claim anyone can check.

## Use

```bash
punctuality ingest                          # 2025-12..latest, download + aggregate
punctuality ingest --from 2026-01 --to 2026-03
punctuality stations bonn                   # eva lookup by name
punctuality stats "Bonn Hbf" --type ICE     # per hour, weekday and weekend
punctuality stats 8000044 --min-n 100       # unpadded eva from HAFAS works too
```

`ingest` keeps one materialized aggregate, but does not rescan every parquet file on every
run. It records each source month and its upstream blob identity in
`punctuality_ingest_months`; a newly published month is folded and merged into the existing
set. If an old month changes, disappears, or the ledger is being bootstrapped, ingest rebuilds
the selected window atomically so no month is double-counted. Every run records its window
and timestamp in `punctuality_ingest_runs`.

The four tables live in the shared SQLite file — `SJEL_DB_PATH`, else
`$SJEL_PERSONAL_ROOT/data/axon/axon.db` — under the table prefix `punctuality`, so they are
`punctuality_stop_stats`, `punctuality_stations`, `punctuality_ingest_months` and
`punctuality_ingest_runs`
(libs/sjel-store/README.md). PRD Q45 (2026-08-27) moved them there from a Postgres schema.

## Where the numbers come from

[piebro/deutsche-bahn-data](https://github.com/piebro/deutsche-bahn-data) —
`upstreams.toml` [deutsche-bahn-data], CC-BY-4.0, the data licensed by Deutsche Bahn.
A job calls DB's Station Data and Timetables APIs four times a day and republishes the
result as one parquet file per month. So this is DB's own data under DB's own licence,
not a scrape.

The live Timetables API was the obvious alternative and cannot do this job: it answers
"what is delayed right now" and has no memory. History is the entire point here.

Upstream states three things about the data that the ingest is built around.

Coverage widened on 2025-11-02: before it only the largest ~100 stations, after it all
~5300. `dataset::FIRST_FULL_COVERAGE_MONTH` therefore starts at 2025-12, since 2025-11
is a mixed month. Reaching further back would buy history at the price of making a
station's statistics depend on when it entered the dataset.

Collection is 98.92% complete, with 196 named missing hours. So an absent train never
means "did not run". It may mean nobody was collecting that hour.

Timestamps are already Europe/Berlin local time, with no conversion applied. The ingest
treats them as wall-clock and carries no timezone library; converting them again would
move the rush hour by an hour or two.

## Asking a threshold other than six minutes

`stop_stats` used to persist seven scalars and drop the 128-bucket histogram that produced
them. So the only exceedance anyone could ever ask about was six minutes, which was never a
considered threshold: it was the one that got its own column. `transit`'s own
`punctuality.rs` states that transfer risk cannot be produced from this data. It always
could; the array was thrown away on the way to disk.

The array is stored now (`counts`, ~512 bytes a cell), and a stop in `POST /lookup` may
carry `at_least_minutes`. A four-minute transfer buffer and a twelve-minute one are
different questions, and asking a new one is a query rather than a re-ingest of every
parquet file.

SQLite has no array type, so the column is TEXT holding a JSON array — one of the two
columns in the whole repo that had no native equivalent (PRD Q45). It is NOT NULL: the
nullability existed to describe rows written before the histogram was persisted, and no
SQLite row predates it.

`share_delay_at_least` still has three states and they are still different answers. Absent
means nobody asked. Explicit `null` means the question was asked and the stored histogram
cannot answer it. A number means it was answered. Which of the last two applies is
`Cell::share_at_least_from_counts`'s verdict — it refuses an array of the wrong length —
rather than a nullable column's.

## One train, one day

`ingest` projects the seven columns it needs from each monthly file and folds the rest into
histograms. That is the right shape for "how late is an ICE here at this hour" and cannot
answer "how late was ICE 611 at Dortmund on the 14th", because the ride is gone by the time
the aggregate exists. The columns to answer it were being decompressed and discarded on
every run.

```bash
punctuality ride --type ICE --number 611 --date 2026-07-14 [--eva 8000044]
```

CLI-only and stores nothing: this is a lookup against files already on disk, and giving it
a table would create a second copy of DB's own published data.

Three things it will not pretend. A month that is not in the local cache reads as
`unavailable` with the reason, because the raw directory is prunable. Publication lags a
journey by up to five weeks, so a recent trip is simply not there yet. And collection is
98.92% complete with named missing hours. All three collapse into one rule, which is why
every answer carries the caveat: an absent stop is never evidence that a train did not run.

Two traps found while building it, both of which returned a confident wrong answer rather
than an error. The time columns are timestamps, not strings, so reading them through a
string downcast yields nulls, the date filter matches nothing, and the result is an empty
stop list that looks exactly like "no such ride". And DB writes the same train as `ICE 611`,
`ICE611` and `611` depending on the column, so a plain string compare misses and looks
identical to a train that did not run.

## How the aggregate is built

One histogram per cell, where a cell is (station, train type, hour of day, weekend).
Delays are small integers, so counting them per minute is a complete description of the
distribution: quantiles read off the cumulative counts, memory is bounded, and
histograms add, which is what lets ingest stream file by file and still produce one
exact result. Buckets span [-5, 120] minutes with an open-ended bucket at each end;
99.906% of observed delays land inside that range (measured on 2026-06 before the range
was chosen, not assumed). `Cell::quantile_is_saturated` reports when a quantile fell
into an edge bucket, so a bound never gets printed as if it were a value.

Why not an analytics engine: see § Why this shape: Rust over a second engine, below.

### How the reader proves itself

The parser reads someone else's binary format, and until 2026-08-20 it was the one
component here with no test: the unit tests covered the hour/weekend derivation and the
histogram arithmetic and never opened a file. Verifying the arrow-rs 59.1.0 → 59.2.0 bump
therefore took a throwaway harness pointed at a private 100+ MB publication, and that
evidence was discarded with the harness.

`ingest::tests` now writes a six-row parquet file with `parquet`'s own `ArrowWriter` and
folds it. Generated rather than committed, deliberately: the real files are private and
too large, and a checked-in blob would freeze a byte layout nobody could regenerate once
the crate moved — writing it with the stack that reads it is what makes the test move
*with* an arrow bump rather than merely survive one.

Six rows, one per behaviour worth pinning: two delay readings folding into one cell, a
cancellation counted in `canceled` and kept out of `n`, a row with neither reading nor
flag skipped, a null `eva` skipped, and a second cell at a different station, type and
hour so the key is demonstrably all four fields. The timestamps are `2 * DAY + 9 * HOUR`
and `4 * DAY + 17 * HOUR` from the epoch, so "Saturday 09:00" and "Monday 17:00" are
arithmetic rather than a lookup — and they arrive through a file, which the old tests
never did.

The schema-change path gets its own red: the fixture is rewritten five times, each
missing one required column, and each must come back as `MissingColumn` naming that
column rather than a panic or a silent zero. That path already fired once upstream in
2026-05 and 2026-08 (`upstreams.toml [deutsche-bahn-data]`).

### The statistics, and why these

| Column | What |
|---|---|
| `p50`, `p90` | median and 90th percentile delay in minutes |
| `share_late_6` | share of stops at least 6 minutes late |
| `cancel_rate` | share of scheduled stops cancelled |
| `mean_delay` | kept, but read it next to `p50` |
| `n`, `canceled` | sample size, because a rate without one is decoration |

Six minutes is DB's own punctuality threshold, which makes `share_late_6` comparable to
the figures they publish about themselves.

`mean_delay` is reported but is not the headline, and the reason is visible in the data:
the mean sits well above the median for every station measured here. Delay distributions
are heavily right-skewed, so a handful of hour-late trains drag the mean somewhere no
actual journey lives. The figures from this deployment's ingest are in
`<overlay>/data/punctuality/evidence-2026-09-23.md`. An earlier prototype of this idea aggregated on
`avg_delay` alone; that is the failure mode this table exists to avoid.

**A cancelled stop is never counted as a punctual one.** It has no delay reading, so it
stays out of `n`, the mean and the late share, and lands in `canceled`/`cancel_rate`
instead. Folding it in as "0 minutes late" would let the worst possible outcome improve
every other number.

## Gotchas

EVA numbers are zero-padded to eight digits here (`08000044`). HAFAS, and therefore
`capabilities/transit`, returns them unpadded (`8000044`). Joining the two without
`store::normalize_eva` returns zero rows and looks exactly like "no data for that
station". It presented that way the first time.

The raw cache really is a cache. `<overlay>/data/punctuality/raw` holds the downloaded
parquet at roughly 600 MB per month. Every byte is re-downloadable, it sits outside the
backup set on purpose, and deleting it costs only the next ingest's download time.

A download failure costs the whole run, which is why `ensure_local` retries four times
with backoff. Bodies that size drop occasionally: the first full run died on month six
of seven after five were already parsed. Cached months are skipped, so a re-run after a
failure only re-fetches what is actually missing. Partial downloads land in `.part` and
are only renamed on success, so an interrupted transfer is never parsed as a whole file.

`ingest` holds the whole aggregate in memory before writing, around 400k cells at 512
bytes of histogram each. That is the design rather than an oversight: the alternative is
partial writes, and partial writes cannot produce exact quantiles.

## The contract

`punctuality-server` on `:8085` is how other capabilities read this. `capabilities/transit`
is the first consumer and reaches it over HTTP, never by linking this crate (CONTRIBUTING.md#schemas-and-dependency-direction).

| Endpoint | What |
|---|---|
| `GET /health` | up, plus the window the aggregate currently covers |
| `POST /lookup` | up to 200 stops in one call, `{eva, train_type, hour, weekend}` each |
| `GET /stations?q=` | eva lookup by name fragment |
| `GET /stations?eva=&train_type=` | every hour cell for one station |

`/health` reports coverage and the last ingest timestamp on purpose. A server that answers
but has never ingested is up and useless, and without the window or timestamp "no data for
that train" reads the same as "that train is never late". The scheduled producer is
`capabilities/punctuality-ingest`, which runs the incremental ingest daily.

`/lookup` answers `null` for a stop it knows nothing about, and for any cell thinner
than 30 observations. It never falls back to a neighbouring hour or a station average:
both would answer a question nobody asked, and downstream a substituted number is
indistinguishable from a measured one.

### What transit does with it

`Journey.delay_risk_score` is filled with `share_late_6` at the journey's destination,
for the arriving train's type and arrival hour.

**That is not the probability the trip works out.** It says nothing about catching a
transfer, and a journey whose first leg is late enough to miss a connection arrives on a
different train than the one this describes.

`Journey.reliability` is the field that does try to answer it, added 2026-08-19, and it
is composed entirely from this table: the product of P(the arriving train is less than
the buffer late) at each transfer, asked at that transfer's own buffer, and P(the last
leg arrives within six minutes). No fitted curve and no constants — every factor is an
exceedance read off the stored histogram, which is exactly what persisting `counts`
bought. This paragraph used to say the data could not produce transfer risk at all; that
was true only while the array was being thrown away on the way to disk, as `store.rs`
notes.

Two things it still assumes and this data cannot settle. Each onward train departs on
schedule, so every transfer term is an upper bound on that transfer's risk and the whole
product is a floor on reliability rather than an estimate of it. And consecutive legs are
independent, which two legs of the same line delayed by one cause are not, so the naive
product overstates. Both are stated on the type rather than corrected, because correcting
them needs recorded trip outcomes that do not exist yet. `min_sample` reports the thinnest
cell in the product, because it is only as measured as its weakest term.

Absence degrades and never fails: if this server is down, or has no cell, transit
returns the same journeys with `delay_risk_score: null`. Verified by stopping the
service and searching — HTTP 200, five journeys, every score null.

The rung after that is comparison, not replacement. `bahnvorhersage`
(gitlab.com/bahnvorhersage/bahnvorhersage, GPL-3) publishes a self-hostable predictor
that outputs P(all transfers caught) and P(arrival within 10 minutes). Adopting it as a
capability and scoring it against this baseline is what would justify a model at all —
in either direction.

## Why this shape: Rust over a second engine

Migrated from its dissolved `decisions/` entry on 2026-07-28: this governs one
thing, so it lives with that thing (CONTRIBUTING.md#decisions-live-with-their-owner).

## Decision

`capabilities/punctuality` aggregates ~120M published stop records with the `parquet`
crate (`upstreams.toml` [arrow-rs]) in a plain Rust binary, and writes the ~400k-row
result into the shared store. No embedded analytics engine is added to Sjel, at any layer.

Dated 2026-07-28, when the shared store was `capabilities/postgres`; PRD Q45 (2026-08-27)
made it one SQLite file. The reasoning below is unchanged by that — it is about not adding
a *second* engine beside whichever one the repo already runs, and every "Postgres" in it
now reads as "the shared store".

DuckDB was the obvious candidate and was evaluated seriously, against the live dataset,
before this was written. It stays a fine tool to reach for interactively — it is how the
numbers in this entry were independently checked — it is simply not a dependency the
repo has to carry.

## Why

**Both DuckDB shapes cost more than they buy here.**

`duckdb-rs` bundles a C++ embedded database. That is a large non-Rust dependency in the
crate graph, with its own build, to run one `GROUP BY`. The DuckDB CLI avoids the build
but moves the aggregation into a SQL file outside the type system, where the histogram
rule that a cancellation is not a punctual train — the one decision in this capability
most likely to be got wrong — would live as an untested `WHERE` clause.

**The workload does not actually need a query engine.** Delays are small integers, so a
fixed 128-bucket histogram per cell is a complete description of the distribution, and
histograms add. That gives exact quantiles, bounded memory, and file-by-file streaming
from about eighty lines of arithmetic that unit tests can pin down. Measured first, not
assumed: 99.906% of 2026-06's delays fall in [-5, 120] minutes, which is what makes the
window honest rather than convenient.

**It keeps one storage story.** scouting, transit and comms already keep their tables in
the shared store. Adding an engine would mean another place data lives, another backup
question, another thing to explain.

## What this forecloses

Ad-hoc SQL over the raw stop records is no longer one command away — a new question
about the raw data means either a DuckDB session against the parquet cache (which is
what the cache is for) or new Rust. Accepted: the aggregate answers the questions
`capabilities/transit` actually asks, and those files stay on disk precisely so
exploration never has to go through this crate.

The raw records are also deliberately NOT loaded into the shared store. It is in the
backup set, so anything in that database is in every backup forever;
120M rows of re-downloadable public data would bloat every backup to store what
upstream will hand back on request. The parquet cache is therefore a cache — outside the
backup set, safe to delete, rebuilt by re-running ingest.

## Considered and declined



- **DuckDB via `duckdb-rs`** ([duckdb/duckdb-rs](https://github.com/duckdb/duckdb-rs),
  MIT) — bundled C++ in the crate graph for one aggregation. Declined on cost, not
  quality.
- **DuckDB CLI driven from a shell script** — puts the statistical rules that decide the
  answer in untested SQL, and adds a binary dependency only this capability needs.
- **`pg_parquet` in the Postgres container** — requires a custom image, which trades the
  pinned official `postgres` image for one Sjel has to build and audit itself.
- **Loading raw stops into the shared store** — see above; the backup consequence decides
  it.

## Considered and declined



- **Calling the DB Timetables API directly** — real-time only, no history, and it needs
  an API key. It is what the upstream dataset is built from, so consuming the dataset
  gets the same data plus two years of memory.
- **Grouping by line number as well as train type** — `line_number` is null for every
  long-distance train (ICE/IC/EC), so it would add cells that are empty exactly where
  the interesting delays are.
- **Keeping raw stop records in the shared store** — see the decision entry; it would put
  120M rows of re-downloadable public data into every backup.
