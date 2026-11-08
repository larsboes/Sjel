# Maintenance priorities and reusable audit tooling

Reviewed and implemented 2026-11-08. Code unification and maintenance precede agent
expansion. [`PLAN.md`](../PLAN.md) owns priority; area plans own remaining acceptance.

## Findings and changes

The initial census found 14 area plans and 4,003 lines including the priority index.
The review sampled source-backed claims across all areas; it did not validate every
acceptance check or remote issue. Age and length alone did not establish obsolescence.

| Finding | Result |
|---|---|
| API extraction described as future work despite 26 API crates | Finance and Inventory now use existing shared contracts instead of local request and comparison copies |
| Schemars requirements repeated across manifests | Compatible consumers inherit the workspace requirement and retain explicit features |
| Unused direct dependencies | Thirty cargo-machete candidates inspected: two API dependencies made useful, 28 unused declarations removed |
| Stale operational paths | Forty references repaired after checking their targets; proposed and historical paths retained for contextual review |
| Finance app and website implementation still listed as open | Plans shortened to shipped references, constraints and remaining acceptance |
| Research index described the retired dashboard and GPUI prototype | Index corrected |
| GPUI formatting and Bun update automation described as absent | Obsolete tasks removed; existing coverage retained |
| Inference review did not leave reusable evidence | Completed reviews now persist applicable receipts |
| Nested Cargo ignored the explicit scratch target | Hermetic wrapper exports the selected target consistently; regression suite covers both flag spellings and refusals |
| Website generators implicitly loaded environment files | Launcher disables that loading; scratch generation and link/payload checks pass |

The two product workspaces remain separate; the graph benchmark is a third workspace.
Forty root package/directory mismatches were observed initially. Renaming everything or
merging workspaces has no measured benefit here and stays below removing duplicate behavior.
Concurrent GPUI and restructure work was preserved.

`route-manifest` already owns schema derivation, `sjel-server` startup/authentication and
`sjel-http` outbound defaults. UI reuse already has GPUI Kit, NSH primitives, TextView and
native media views. Extend these owners when contracts match. A generic CRUD engine,
new API framework or UI replacement has no demonstrated need in this pass.

## Repeatable maintenance

```sh
tools/maintenance --json
tools/maintenance --json --unused --freshness --licenses
```

Default execution inventories Git-tracked manifests and plans offline. Optional analyzers:

| Flag | Coverage and limit |
|---|---|
| `--unused` | cargo-machete, read-only, without fixes or Cargo metadata; findings require source inspection |
| `--freshness` | crates.io direct requirements and Bun outdated; compatible updates and requirement changes are distinct; git pins explicitly unchecked |
| `--licenses` | cargo-deny offline license inventory for the current Rust host; no compatibility verdict or automatic allowlist |

The local run completed all requested analyzers across three Cargo workspaces and five
locked Bun projects. It listed 26 API contracts and 23 unresolved path candidates after
repair. Candidates include proposals, historical references and concurrent plans; they
are not automatically defects. No source, lockfile or installation changes are applied.
Missing or failed requested analyzers exit 2.

[Cargo-machete provenance](benchmarks/maintenance/README.md) records reviewed source,
patched transitive dependencies and limitations. All three workspaces subsequently
reported no unused-dependency candidates. No duplicate-code percentage or build-speed
improvement was measured. Knip and jscpd remain optional discovery candidates; neither
was installed or added as a gate. Generated code, fixtures and extension entry points
must be modeled before acting on such reports. Cargo's existing dependency and feature
inspection precedes adding cargo-hakari or another graph service.

## Reduce audit work without adding a second gate

OSV, Gitleaks, Grype, CodeQL and Socket already cover distinct parts of the repository.
Keep those owners. [OSV](https://google.github.io/osv-scanner/) matches known advisories;
[Socket](https://docs.socket.dev/docs/socket-for-github) examines package behavior. Neither
establishes that natural-language instructions are benign. Secret detection does not
replace execution restrictions.

The [Cisco trial](benchmarks/skill-intake/README.md) used reviewed pinned source, a patched
lockfile and network-disabled containers. Both detectors passed four benign cases. Sjel
held all six malicious cases; Cisco's local analyzers held one. Keep Sjel intake and do
not add Cisco as a routine gate. This synthetic smoke set is not a held-out efficacy
benchmark. Local semantic review was not evaluated, and the source review did not audit
every transitive dependency. Reproduction inputs and results are tracked.

The retained intake layer stages once, hashes paths/bytes/executable bits, reviews those
bytes and publishes that snapshot. Inference receipts now bind source, content, approved
baseline, model/backend settings and policy fingerprint. Every staged text part must be
read before recording a completed model review. Legacy receipts require review again.
Holding findings still hold; explicit approvals retain their existing scope. Policy
source changes conservatively invalidate receipts, including nonfunctional edits.
Advisory scans remain fresh independently of content-review reuse. Live inference and
harness acceptance remain open; fixture tests do not close them.

Other researched options have explicit boundaries:

- [Snyk Agent Scan](https://github.com/snyk/agent-scan): defer; service authentication and
  potential MCP-server execution do not fit the narrow offline trial.
- [Cargo Vet](https://mozilla.github.io/cargo-vet/): defer a mandatory rollout until an
  audit-import coverage trial demonstrates less work than the obligation it introduces.
- [cargo-deny](https://embarkstudios.github.io/cargo-deny/checks/index.html): adopt optional
  license inventory; keep OSV as advisory owner. Legal allowlists require their own decision.
- Renovate: retain existing Dependabot coverage; no unmet coordinated-update need established.
- A second document registry: reject; use existing site link checks and tracked path inventory.

Dependency verdicts live in [`upstreams.toml`](../upstreams.toml). The
[ColluSkill preprint](https://arxiv.org/abs/2608.09732) supports examining interactions between
skills; its experiments do not measure this installation or justify claiming complete protection.

## Verification and remaining acceptance

Fresh repository/installed-package OSV and secret scans passed under existing exceptions.
The CLI suite, scoped Finance/Inventory tests, root Clippy, hermetic wrapper regression
suite, skill-install regressions, GPUI compilation and website link/payload checks passed.
Four native macOS sandbox tests fail in this restricted environment; their assertions
remain intact. A later broad run completed the remaining test suites, but its wrapper
exited during a concurrent edit, so it is not a clean full-replay receipt.

Doctor still reports the old host-patch audit receipt and, in the latest run, a missing
ledger-owned Anatomy installation across four harnesses. Fresh dependency scans cannot
impersonate a host-patch run. The external backup destination was unavailable; no restore
proof was manufactured. Native UI acceptance and finance outcome quality remain open.

The data paths remain distinct: `content-item::cloud_admission` refuses `c2`/`c3` for
Sjel-initiated cloud inference; MCP `agent_policy` separately permits filtered
`trusted_view` for trusted harness tiers. This pass changes neither policy nor skill rules.

Interceptor research initialization exited 1 without diagnostics; public research used the
available web tool. Published tool capabilities are not local performance results unless
explicitly marked above. No private content was uploaded for evaluation.
