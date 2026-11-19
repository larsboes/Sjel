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
