# Graph store spike: SQL edges against an embedded graph engine

Option A against option B from
[Track 2's integration candidates](../../../PLAN.md#integration-candidates-to-narrow-together),
for the coding-agent consumer (selected 2026-10-24). One question: does a coding agent's graph
query need a graph engine, or do edge tables in the SQL engine Sjel already links answer it?

Measured 2026-10-24 on an Apple M4 Pro. One fixture, 30 seeds, 3 runs. This is a spike and does
not adopt anything.

## Method

- **Fixture:** Graphify's graph of this repository, built at `aaf300e7`: 37,200 nodes,
  84,232 edges without self-loops, of which 22,261 are `calls` edges. `graphify-out/` is
  gitignored, so this crate reads it in place and commits no data.
- **Q1, impact:** every node with a `calls` path of 1 to 3 hops into a target ("who is affected
  if this changes").
- **Q2, connection:** the undirected hop count between two nodes over any relation, up to 6 hops.
- **Reference:** a std BFS over the same edges in [`src/main.rs`](src/main.rs). Both engines must
  return the reference's set or distance exactly.
- **A:** libSQL 0.10.0 (`core`, the engine `libs/sjel-turso` links), with a `nodes` and `edges`
  table, indexes on `(dst, rel)` and `src`, and `WITH RECURSIVE`.
- **B:** Grafeo 0.6.0 (`cypher`, `grafeo-file`), persistent, with a property index on `key`,
  `[:calls*1..3]` and `shortestPath`.
- **Seeds:** every k-th called node in key order. Q2 pairs node i with node i + 15.

Run with `cargo run --release` from this directory. The crate declares its own `[workspace]`,
so Grafeo stays out of the Sjel lock file.

## Results

Ranges are over 3 runs. Times are per query.

| | A′ in-memory BFS | A libSQL + CTE | B Grafeo + Cypher |
|---|---|---|---|
| Wrong answers (Q1 + Q2) | reference | 0 / 60 | 0 / 60 |
| Q1 impact, p50 / p95 | < 0.01 / ≤ 0.01 ms | 0.05–0.06 / 0.09–0.15 ms | 0.04–0.05 / 0.07–0.11 ms |
| Q2 connection, p50 / p95 | 4.7–7.9 / 7.2–11.5 ms | 186–246 / 393–449 ms | 24–27 / 34–39 ms |
| Bulk load + index | — | 436–483 ms | 343–506 ms |
| Reopen | — | 0.2–0.5 ms | 81–96 ms |
| On disk | — | 13.7 MB | 3.8 MB |

A′ is the reference BFS timed over in-memory adjacency lists. That is what A becomes when an
application reads the edge table once and walks it in code.

## What this shows

1. **Correctness does not separate the options.** Both engines answer all 60 queries exactly.
2. **Bounded impact queries are equal.** For Q1, the recursive CTE and Cypher both answer in
   under 0.2 ms at p95.
3. **The CTE is the weak point for open-ended paths.** A CTE has no visited set, so it expands a
   node once for every depth at which it reaches it. Grafeo answers Q2 about 8× faster.
4. **SQL plus a BFS in code beats both.** A′ answers Q2 about 4× faster than Grafeo, and a
   process that holds the adjacency in memory does not need a second database.
5. **Grafeo's planner does not reorder the pattern.** `(c)-[:calls*1..3]->(t {key: $k})` took
   42 ms at p50. The same pattern written anchor-first, `(t {key: $k})<-[:calls*1..3]-(c)`,
   took 0.04 ms. The committed query is the anchor-first form.

At this size, for these two questions, a graph engine adds speed that a BFS in code already
provides. It adds no capability. Option A, edge tables in the owning SQL store with path
queries walked in code, covers the coding consumer.

## Limits

- One graph of 37,200 nodes. Only 6 of the 30 Q2 pairs connect within 6 hops, so most Q2 timings
  measure an exhausted search, not an early hit.
- Not tested: incremental updates and deletes, crash recovery, concurrent writers, the memory
  of a resident Grafeo process, vector or hybrid queries, and graph algorithms such as PageRank
  or community detection. These are where Grafeo could still earn a place.
- Grafeo is nine months old (repository created 2026-01-26), and 0.6.0 was published on
  2026-10-23. These numbers describe that release only.
- The Turso Database rewrite was not measured. Its row in [`upstreams.toml`](../../../upstreams.toml)
  is `watch`.
