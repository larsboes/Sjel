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

## TypeScript cleanup

The follow-up used the already-installed TypeScript compiler before adding another
analysis package. From each package directory:

```sh
bun --no-env-file node_modules/typescript/bin/tsc --noEmit --noUnusedLocals --noUnusedParameters
```

Clipper also requires `--module es2020`, as its existing check script does. On
2026-11-08, Obsidian Access and Pi Subagents reported no unused declarations. Web
Access reported four: two imports, an unused response parameter and write-only provider
state. Those were removed, and all three package configurations now enforce both checks.
The repository's 16 local-adaptation tests pass; upstream Web Access tests are not vendored.

Clipper initially reported 69 diagnostics. Inspection removed 15 findings, including
write-only highlight/window state, unused imports, a redundant memoized frontmatter
wrapper and uncalled error/conversation helpers. The existing typecheck, 257 tests and
Chrome build pass. The remaining 54 diagnostics include callback parameters and deliberate
compile-time checks; they are candidates, not a proven defect count. No blanket fix was applied.

Compiler checks do not detect unused public exports, unreferenced entry points or
cross-file clones. Knip and jscpd remain optional discovery candidates; no new package
was installed and no general duplication percentage is claimed.
