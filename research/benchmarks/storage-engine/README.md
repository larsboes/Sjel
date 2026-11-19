# Storage engine compatibility probe

The probe uses generated files and a local ARM64 Debian image. It compares Turso with the pinned
libSQL engine on the same bytes. The engine runs without network access, with a read-only root,
read-only inputs and a writable temporary filesystem.

Generate the fixture directory from the repository root. It must not already exist.

```sh
sjel cargo run --offline --locked -p sjel-turso --example compatibility-fixtures -- "$scratch/fixtures"
sh research/benchmarks/storage-engine/probe.sh "$scratch/fixtures" "$scratch/tursodb"
```

The tested `tursodb` is from the [v0.8.3 ARM64 Linux release](https://github.com/tursodatabase/turso/releases/tag/v0.8.3).
The archive SHA-256 is `4a832c5125ea9137b020843c61731644666db90c450b027f5d5cffe455c4c3fb`.
GitHub's build provenance verified against the upstream release workflow and source commit
`607bff9fd0e547a25a97b3bf06e83bcc5d99d53c`. The probe runs the extracted binary directly.

| libSQL fixture | Turso read-only open | Turso normal open |
| --- | --- | --- |
| Rollback journal | Query matches, bytes unchanged | Query matches, bytes change on conversion to WAL |
| WAL | Query matches, bytes unchanged | Query matches, bytes unchanged |
| Rollback journal with vector index | Schema parse error, bytes unchanged | Schema parse error, bytes change |
| WAL with vector index | Schema parse error, bytes unchanged | Schema parse error, bytes unchanged |
| WAL after libSQL drops the index | Query matches, bytes unchanged | Query matches, bytes unchanged |

The query returns `3|4|1` for recursive reachability, `json_each` and exact vector ranking. Under WAL
and MVCC, the second CLI process encountered a file lock while the first held a write transaction.
After killing the first process, the committed row survived and the uncommitted row was absent in
both modes.

The deindexed fixture retains all embedding bytes and returns the same exact cosine ranking.
The candidate's experimental `toy_vector_sparse_ivf` index rejects those dense embeddings.
Its [documented similarity function](https://github.com/tursodatabase/turso/blob/v0.8.3/core/index_method/toy_vector_sparse_ivf.rs#L26-L31)
is Jaccard, which changes the ranking contract.

Run the Rust binding probe against the reviewed upstream checkout after its dependencies are
available in the local Cargo cache. It has two connections to one database handle and also builds
the same probe for arm64 iOS.

```sh
sh research/benchmarks/storage-engine/binding-probe.sh "$turso_source" "$scratch/binding"
```

With WAL and `BEGIN IMMEDIATE`, the second connection returned busy, then succeeded on retry after
the first committed. With MVCC and `BEGIN CONCURRENT`, two transactions writing distinct keys
overlapped and both committed. The arm64 iOS probe executable linked against the iPhoneOS SDK with
default Turso features disabled. It was not run on a phone.

The probe uses synthetic data and does not measure power-loss recovery, search latency at production
scale or compatibility with a copy of the deployed store.

## Side-by-side workload

```sh
sh research/benchmarks/storage-engine/binding-probe.sh "$turso_source" "$scratch/workload" workload
```

The optimized Rust probe uses libSQL 0.10.0 and Turso 0.8.3 on the same generated database. It
uses the [graph-store probe](../graph-store/README.md)'s edge schema and scale: 37,200 nodes,
84,232 edges and 22,261 call edges. It checks 120 three-hop queries and 24 exact top-10 queries
over 25,000 dense, 384-dimensional vectors. Four connections each commit 200 counter updates,
first on distinct keys and then on one shared key. Every graph result and ranked vector list
matches libSQL. All three engines preserve all 800 increments in both writer workloads.

Ranges below are from three optimized runs on one Mac. The engine order was fixed. A separate
end-to-end restore run also passed and showed higher graph read tails for Turso.

| Engine | Graph read p95 | Exact cosine p95 | Distinct-key tx/s | Shared-key tx/s |
| --- | ---: | ---: | ---: | ---: |
| libSQL WAL | 0.24–0.49 ms | 17.36–27.23 ms | 20.3–20.5k | 25.2–26.3k |
| Turso WAL | 0.72–0.77 ms | 19.44–19.94 ms | 16.0–17.5k | 15.4–16.4k |
| Turso MVCC | 0.62–0.69 ms | 22.36–22.98 ms | 47.2–47.6k | 20.8–23.5k |

MVCC committed the distinct-key workload without retries. The shared-key workload required
24–38 retries after write conflicts. The adapter checks autocommit before rolling back, since
Turso can already have aborted the transaction.

## Backup and rollback rehearsal

The native probe reopens the Turso-written WAL file directly with libSQL. SQLite 3.54.0 rejected
the MVCC file as a database. With the engine closed, the probe copies the main file and its
sidecars, opens that copy with Turso, switches it to WAL, and checks the counters and vector
results in libSQL. SQLite's `.backup` then produces readable snapshots of both WAL files.
`quick_check` returns `ok` and the restored counters, edge count and embedding count match.
The rehearsal covers closed-file recovery and rollback. A live MVCC backup, a power cut, a
physical iPhone, and a deployed-store fixture remain outside the measured scope.
