# Dependency and UI infrastructure review

Reviewed 2026-11-08 against tracked manifests, both product Cargo lockfiles, five Bun
projects, live registries and GitHub metadata. Scope: dependency freshness, the mobile
fork, and source-level refactoring opportunities. This review does not validate an upgrade.

## Verdict

Follow-through: the Bun patch/minor groups, CI install discovery, scanner dependency
checks, five-tree Dependabot coverage and NSH Mail memoization are implemented.
The clipper upgrade required adapting transcript separation to Defuddle's sanitized
markup. WebExtension typings remain at 0.12.1 because 0.12.6 rejects existing
callback signatures. Rusqlite remains at 0.40: the latest published
[`r2d2_sqlite` 0.35.0](https://crates.io/api/v1/crates/r2d2_sqlite/0.35.0/dependencies)
requires rusqlite 0.40. A coordinated pool-manager release is required before
upgrading the SQLite stack; another fork is deferred.

Verified after implementation: 165 focused tooling and Pi adaptation tests, 257
clipper tests and its production build, 30 pi-learn tests, 68 Obsidian access
tests, five subagents tests, and three NSH Mail tests. All five frozen installs
and all five configured Bun security scans pass. Publication hygiene and architecture
freshness pass. Strict NSH UI Clippy still reports three existing warnings in
`nsh_documents.rs` and `primitives/tables.rs`. The full local CI replay fails before
root Bun test collection with `EPERM` and has additional shell-suite failures.
Device testing and frame-time benchmarking were not run. No frame-rate improvement
is claimed from the memoization change.

### Investigation verdict before implementation

Keep the mobile fork. Keep the current released UI stack and NSH revision. Prioritize
repairing CI dependency installation, updating the Bun projects in separate compatibility
groups, and evaluating rusqlite 0.41.1. The clearest immediate UI optimization is caching
NSH Mail's filtered rows. Test the next GPUI Fast and Kit releases against Sjel's existing
benchmarks before adopting them. A wholesale UI rewrite is not justified by this review.

## Published versions and the fork

| Dependency | Tracked version | Live result | Action |
|---|---|---|---|
| GPUI Kit | 0.9.0 | Latest published release | Hold |
| GPUI Fast | 0.1.13 | Latest published release; already the Mac default | Hold |
| GPUI Pre | 0.3.11 | Kit's exact snapshot requirement | Move with Kit and Mobile |
| NSH | `058a745ad2` | Identical to upstream main | Hold git pin |
| GPUI Mobile | fork `bbedd2d` | Upstream still requires Pre 0.3.10 | Keep fork |

Sources: [Kit release](https://github.com/longbridge/gpui-kit/releases/tag/v0.9.0),
[Fast release](https://github.com/longbridge/gpui-fast/releases/tag/v0.1.13),
[NSH commit](https://github.com/longbridge/nsh/commit/058a745ad2d837374b99a9e46715e9b212d9b7a8),
and `apps/gpui/{Cargo.toml,ios/Cargo.toml,Cargo.lock}`. NSH's published 0.4.0
is not a newer substitute for the git pin; the packaging and backend compatibility
problems are recorded in [the UI stack investigation](ui-stack.md).

The fork is one commit ahead of upstream, with no missing upstream commits at review.
Its patch upgrades Pre and WGPU to 0.3.11 and supplies the changed WGPU surface
configuration. [Upstream #46](https://github.com/longbridge/gpui-mobile/pull/46)
is open and unmerged. [Sjel's #47](https://github.com/longbridge/gpui-mobile/pull/47)
was closed as a duplicate, without merging. Closed does not mean incorporated.
[Fork comparison](https://github.com/longbridge/gpui-mobile/compare/7548ebc4b24f00494af9976af8852c7163ded9ce...larsboes:bbedd2d).

Return to a pinned upstream Mobile revision once the compatible patch merges and
the simulator, device build and host embedding checks pass. Deleting the Mobile
dependency entirely requires retiring every remaining GPUI phone screen and its
host integration, as specified in [the GPUI plan](../docs/plans/gpui.md#11-ui-stack).
NSH Documents and Mail alone do not satisfy that condition. GPUI Fast's published
iOS platform uses GPUI Pre iOS; it does not establish a replacement for the Swift
host's Mobile embedding API.

## Rust and Bun upgrade candidates

A read-only crates.io comparison of 66 distinct direct registry dependency names
from `Cargo.toml`, `crates/*/Cargo.toml` and the GPUI manifests found one missing
latest stable direct version: rusqlite 0.40.2 to 0.41.1. This is not a transitive
freshness or vulnerability audit. Older tower-http 0.6.11 also remains in the
graphs; the root already resolves 0.7.1. Upgrade its owning dependency rather than
forcing a graph-wide replacement.

The installed Rust 1.99.1, Bun 1.4.4 and uv 0.13.7 matched their latest upstream
releases during the review:
[Rust](https://github.com/rust-lang/rust/releases/tag/1.99.1),
[Bun](https://github.com/oven-sh/bun/releases/tag/bun-v1.4.4),
[uv](https://github.com/astral-sh/uv/releases/tag/0.13.7). No toolchain upgrade is
indicated by those checks.

[Rusqlite 0.41.0](https://github.com/rusqlite/rusqlite/releases/tag/v0.41.0)
changes column metadata methods to iterators, changes zero-blob sizing, updates
bundled SQLite to 3.54.0, and fixes callback lifetime problems.
[0.41.1](https://github.com/rusqlite/rusqlite/releases/tag/v0.41.1) fixes a
zero-argument deterministic scalar-function panic. The source search found
`column_names()` in `crates/agent/src/sql.rs`; its collection chain already uses
`into_iter()`. No direct `trace_v2`, `busy_handler`, or custom SQL function registration
was found in `crates/`. This narrows migration work but does not prove compatibility
or establish whether a reported upstream defect affects Sjel. Validate the store,
backup/restore and agent SQL boundaries before adopting the new SQLite build.

`bun outdated` completed in all five package directories. Selected results:

| Owner | Candidate versions | Assessment |
|---|---|---|
| `crates/comms/clipper` | defuddle 0.19.2 → 0.19.6; linkedom 0.18.12 → 0.18.15; highlight.js 11.11.1 → 11.12.0; Sass 1.106.1 → 1.108.1 | First update group; run extraction fixtures, type checks and browser build |
| Clipper build tools | TypeScript 5.5.4 → 7.0.2; Vitest 4.1.12 → 5.1.0; webpack-cli 5.1.4 → 7.3.1; sass-loader 16.0.8 → 17.1.0; lucide 0.359.0 → 1.70.0 | Separate migrations, not a single bulk update |
| `pi-web-access` | MCP SDK 1.32.1 → 1.32.2; defuddle 0.19.3 → 0.19.6; undici 8.12.3 → 8.12.4; linkedom 0.16.11 → 0.18.15; unpdf 1.8.1 → 2.1.0 | Separate patch updates from DOM and PDF compatibility changes |
| Pi extension projects | Pi packages 1.4.0 → 1.5.0 across web-access, subagents and obsidian-access; typebox updates subject to release-age policy | Update development and peer compatibility together, then exercise extension loading |
| Remaining Bun tools | smol-toml 1.9.0 → 1.9.1; Biome 2.5.14 → 2.6.6; pi-learn linkedom 0.16.11 → 0.18.15 | Use each owner's checks; subagents also has major Croner and Nano ID candidates |

These versions come from live registry output; availability is not a compatibility
verdict. Respect Bun's existing minimum release age. Sources are the five tracked
`package.json` and `bun.lock` pairs and `bun outdated` in their directories.

`.github/dependabot.yml` watches both Cargo workspaces and GitHub Actions, but no
Bun project. Its GPUI Kit/Pre exclusions are intentional coordination points.
`sjel update` covers software outside the checkout, not repository dependency
freshness. Add a read-only repository report that distinguishes manifest bounds,
resolved versions, compatible updates and major migrations, including git pins.

## Refactoring enabled by the current UI infrastructure

1. **Cache NSH Mail's filtered projection.** In `ui/src/nsh_mail.rs`,
   `thread_list` defines `visible_rows` by filtering and cloning all matching
   proposals. The virtual list calls it for its length and again for each row's
   subject, subtitle and summary. Derive the filtered rows or indices once when
   the source/status/category changes. Verify filtering and selection correctness,
   and count projection work for a large fixture before claiming a speedup.
   Documents and Mail already use NSH `virtual_list`; adding virtualization again
   would not address this cost.
2. **Own NSH feeds by screen lifecycle.** Documents and Mail each have process-wide
   `OnceLock<Feed>`, mutex snapshots, callback slots and one mutable channel sender.
   Opening a second host replaces the sender. Investigate a session-scoped feed
   before adding more windows or NSH screens; test remounts, concurrent hosts and
   late async responses. Share only the runtime-to-engine boundary, not a generic
   screen framework. The existing GPUI `data.rs` adapter is prior art for lifecycle
   and completion handling. NSH needs its own engine adapter.
3. **Use retained rendering at entity boundaries.** Systems currently stores all
   panels in one `SystemsPage` entity. Its independent clocks already use
   `read_quiet`, so suppressing unchanged notifications is already implemented.
   Measure whether separating active panels into child entities avoids unrelated
   layout after telemetry changes. Do not add caching to derived data that is
   already cached (`SystemsDerived`, Packs' `settle_derived`).
4. **Reuse current primitives and migrate the phone incrementally.** Calendar
   already uses Kit DatePicker and TimeField; Documents already uses TextView.
   Continue the existing wrapper rule rather than introducing another widget
   layer. Move phone pages to NSH in the planned order, removing superseded
   phone implementations only after device verification. Moving all desktop
   pages to NSH remains a separate product decision.

Source paths are relative to `apps/gpui/` in this section. Retention requirements
are documented in [Fast's retained-mode guide](https://github.com/longbridge/gpui-fast/blob/main/docs/retained-mode.md).
These source findings need profiling before implementation.

## Unreleased upstream improvements

Fast main is 23 commits ahead of v0.1.13 at review. Relevant merged changes:
[released-entity dependency pruning #106](https://github.com/longbridge/gpui-fast/pull/106),
[child-only cached redraw #117](https://github.com/longbridge/gpui-fast/pull/117),
[lazy tab-order construction #113](https://github.com/longbridge/gpui-fast/pull/113),
and [accessibility tab-count handling #118](https://github.com/longbridge/gpui-fast/pull/118).
These match navigation, complex pages and accessibility workloads. The reported
microbenchmarks are upstream scenarios; no Sjel gain was measured here.

Kit main is 68 commits ahead of v0.9.0. Its
[paragraph highlight merging fix #3846](https://github.com/longbridge/gpui-kit/pull/3846)
is relevant to long rich-text readers. Its
[callout surfaces #3830](https://github.com/longbridge/gpui-kit/pull/3830) do not
implement the editor's inline Markdown concealment requirement. Neither change
justifies reviving the paused editor by assumption.

Prefer a coordinated release. If a concrete defect requires a git evaluation,
pin the full compatible dependency set and compare it with the released baseline
using `apps/gpui/bench-scroll`, page switching, text selection, keyboard navigation,
accessibility and stale-view checks. Keep the Mobile compatibility requirement.

## Verification and implementation order

The [CI run at remote commit 027f03fa](https://github.com/larsboes/Sjel/actions/runs/42486842882)
passed the five GPUI platform-check jobs and Rust formatting/Clippy, but failed
Bun tests, UI checks, Cargo tests and repository gates. Logs explicitly show a
missing pi-learn `linkedom` installation and an uninstalled configured Bun
security scanner. Other failed jobs require their own diagnosis; they are not
attributed to those two errors. The separate Security workflow passed at that
remote commit. These results do not validate the 12 additional local commits.

`tools/doctor` exited 1. Its CLI rebuild failed on shared Cargo target archive
cleanup permissions, then used the older binary. It also reported an existing
host-patch audit finding. No dependency was changed, no application benchmark or
device test ran, and no full local test suite or new vulnerability audit ran.
The context tool returned `self=unavailable-for-Cargo.toml`; tracked manifests
were used directly. Private runtime findings are outside this public review.

Recommended order: repair CI installation and diagnose remaining baseline failures;
update the Bun patch/minor groups and validate rusqlite separately; cache NSH Mail's
projection; complete the phone device checks; then evaluate the next coordinated
Fast/Kit release. Keep the mobile fork until its compatibility exit condition is met.
