# Unused-dependency trial

The 2026-11-08 source-reviewed cargo-machete trial used revision
`734317a34f6ca0ea1e1faa11a7dbc31a3f414ed6` of
[cargo-machete](https://github.com/bnjbvr/cargo-machete).

`cargo-machete-lock.patch` preserves the trial's lock-only updates to crossbeam-epoch
and memmap2. OSV found advisories in their original locked versions; the patched lock
passed the same scan. Apply it with `git apply --unidiff-zero` to an archive of that
revision before building.
Do not treat this dated result as a substitute for a current dependency scan.

The reviewed invocation uses `--json --skip-target-dir`, without `--fix` or
`--with-metadata`. First-party source performs no network requests in this mode and
does not invoke Cargo metadata. Dependency source was not exhaustively audited.

Thirty initial candidates were inspected. Two API dependencies became used by replacing
duplicate request types with shared contracts; 28 unused declarations were removed.
Both product workspaces and the graph benchmark then reported no candidates. This is
heuristic evidence, supported by compilation and tests, rather than proof that every
remaining dependency is necessary.

Put the reviewed executable on PATH and run `tools/maintenance --unused --json`.
The report discovers workspaces from tracked manifests. It reports unavailable tooling
as incomplete coverage and never applies fixes.
