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

The query returns `3|4|1` for recursive reachability, `json_each` and exact vector ranking. On the
unindexed WAL fixture, the second CLI process encountered a file lock while the first held a write
transaction. After the first process was killed, the committed row survived and its uncommitted row
was absent. This test covers process death and the default CLI file locking. Same-process writers,
power loss, a production-store copy and iOS remain outside the measured scope.
