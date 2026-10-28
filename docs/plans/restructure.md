# Restructure and sweep

After the dashboard goes, the repository gets smaller, and every file changes once: layout, code
comments and documentation in the same pass. Lane A in [`PLAN.md`](../../PLAN.md), after
[`gpui.md`](gpui.md) retirement.

## 1. Delete

Gated by [`gpui.md`](gpui.md#3-what-retiring-the-dashboard-still-takes) §3.

- `dashboard/`, then `dashboard/src-tauri` when its own gate passes.
- `demo/` and its publishing workflow. The demo site is the dashboard.
- The Tauri plugins in `plugins/` (`roomplan`, `device-identity`, `local-network`,
  `foundation-models`) once nothing builds against them. Each one is ported or retired first.
- The 78 references to `dashboard/`, `matches_dashboard_nav`, and the `sjel-status` static serve.
- Svelte skills and tooling that only the dashboard uses (`svelte-*` skills, `bun` checks over
  `dashboard/`). Check every consumer before removing one.

## 2. Sweep

The rule is in `Packs/harness/skills/sjel/references/on-documentation.md`. Remove every code
comment, config comment and README paragraph that repeats a decision, a rule or a history that
Git or the code already holds.

### Where each kind of text lives

| Text | Home | Not in |
|---|---|---|
| Order, priority, open work, decisions not yet built | `PLAN.md`, `docs/plans/<area>.md` | READMEs, manifests, code |
| A built decision and the reasoning needed to change it | The owner README, `## Why this shape: <topic>` | Code comments, TOML |
| A rejected alternative | The owner README, `## Considered and declined` | Plans |
| Interface contracts, data flows | `docs/capabilities/<name>.md` | READMEs |
| History, measurements, who decided | The commit message | Anywhere tracked |
| An invariant the code cannot express | A one- or two-line code comment | READMEs |
| What a config value means, or its unit | One comment line on the key | Rationale paragraphs |
| A dependency verdict | One `upstreams.toml` row | READMEs, `Cargo.toml` comments |

Decisions stay with their owner, not in one central log. A central log goes stale because nobody
who changes the code reads it. `PLAN.md` keeps only decisions that span areas.

### `upstreams.toml`

It stays the one registry of dependency and influence verdicts (152 entries: 73 `adopt`, 31
`inspiration`, 23 `reject`, 15 `watch`, 6 `quarry`, 2 `evaluate`, 1 `adapt`, 1 `overlay`). A verdict
and its review date are data and stay. The `why` text is the bloat: median 640 characters,
maximum 3,856. Target shape per entry: verdict, review date, one or two sentences of why, and a
link to `research/` when the analysis is longer. `watch` and `evaluate` entries get a revisit
condition, or they become `reject`.

### Baseline

Tracked files, `git grep`, 2026-10-23 and 2026-10-25:

- 609 comment lines in `*.rs`, `*.toml`, `*.sh` carry a date. 337 say "measured" or "decided".
- 3,143 of 8,862 TOML lines are comments. Worst: `profiles.toml` 268 of 313, `toolchain.toml` 168 of
  456, `osv-scanner-installed.toml` 155 of 386, `Cargo.toml` 93 of 190, `systems.toml` 64 of 180,
  and 922 comment lines across all `service.toml` files.
- "Axon" appears 804 times in 299 files.
- 17 lines in code and config cite IDs of the retired `ISA.md` (for example "ISA F10", "ISA.md
  C4"). `tools/sjel-cli/src/doctor/checks.rs` still counts ISA claims, and `tools/isa-hygiene.sh`
  scans ISA files.
- Over 800 lines: `capabilities/comms/README.md`, `capabilities/calendar/README.md`.

### Order

1. A ratchet in `tools/doctor` or CI: each count above may fall, never rise.
2. TOML first, one comment line per key at most. Then `upstreams.toml` in its target shape.
3. Remove the ISA leftovers: the doctor count, `tools/isa-hygiene.sh` and its test, and the 17
   citations (state the rule they point to, or delete them).
4. Code comments and READMEs, module by module, in the same pass as the layout change. READMEs
   also move to the neutral register (`on-documentation.md`, Register).
5. "Axon" goes in the lines each pass touches, the rest in one rename commit. The compatibility
   paths need their own migration: `X-Axon-*` headers are still accepted until the phone app
   ships (the gate rewrites them), and `axon.toml` and `axon.local.toml` are still read after the
   `sjel.*` names (`tools/manifest-resolution.test.sh`). Persisted paths need a data migration.
   Check with `git grep -n -i axon`.

### Planning outside the plans

READMEs and manifests still hold open work. Rule: a README describes what is built and why, and
open work goes to the owning area plan or an issue (`on-documentation.md`, Plans).

Found 2026-10-25 by heading and by dated lines:

- "Not built" sections: `capabilities/inventory`, `entities`, `software`, `people-registry`,
  `knowledge-graph` ("Not measured, not built"), `scouting` ("Candidate sources (queued, not
  built)"), `agent` ("What is not built"), `libs/agent` ("Not here yet").
- `capabilities/trips/decisions/travel-seams-not-built.md`: a decisions directory, which
  CONTRIBUTING.md ("Decisions live with their owner") does not allow.
- `places` ("Deliberately not built") is a declined-alternatives list. It moves to
  `## Considered and declined`, not to a plan.
- Most dated lines in READMEs: `tools/sjel-cli` (32), `capabilities/trips` (23), `transit` (20),
  `comms`, `documents`, `agentbox` (16 each), `finance` (14), `scouting` (13).
- Most dated lines in TOML: `profiles.toml` (35), `toolchain.toml` (13). Review dates in
  `upstreams.toml` and waiver dates in `osv-scanner*.toml` are data, not history, and stay.

**README shape after the sweep.** What it is, `## Why this shape: <topic>`, how to run it, a link
to the technical manual in `docs/capabilities/`, `## Considered and declined`. No history, no
open-work list, no measurements without the command that reproduces them.

## 3. Layout

Decided 2026-10-25: one workspace, flat `crates/`, one API crate per capability, and a hybrid
process model later.

### Findings that drive it (measured 2026-10-25)

- Two Rust workspaces for one product: the root (56 members, `Cargo.lock` 6,847 lines) and
  `apps/gpui` (`Cargo.lock` 9,969 lines). Only 382 package versions appear in both.
- Client and server each define the wire types. `apps/gpui/core/src/shell.rs` has 96 `pub struct`s,
  `interior.rs` 32 and `systems.rs` 24. Only `libs/roomplan-contract` is shared by both sides.
- "Capability" names 26 Rust crates and 21 directories of third-party software or scripts.
- 23 capability processes use 258 MB of memory in total. `target/release` holds 33 binaries in
  13 GB.
- 5 capabilities are only a `main.rs` of 357–1,931 lines: `agent`, `assistant`, `sjel-status`,
  `soundscape`, `host-net`.
- `comms` (43k lines) and `tools/sjel-cli` (40k lines) are single crates. Eight crates in `libs/`
  have 228–380 lines each.

Reference practice: one workspace, a virtual root manifest, a flat `crates/` directory in which
each directory name equals its package name, `publish = false`, and workspace-wide dependencies and
lints ([Large Rust Workspaces](https://matklad.github.io/2021/08/22/large-rust-workspaces.html)).
rust-analyzer and Zed use this layout.

### Target

```
Cargo.toml          virtual manifest, members = ["crates/*"]
crates/             every Rust crate: capabilities, their -api crates, libs, the GPUI app,
                    Rust tools
services/           third-party software and scripts Sjel supervises (manifests only)
apps/ios/           the Xcode host project for the iPhone app
Packs/ schemas/ docs/ research/ tools/ (scripts only)
```

- **Kind lives in the manifest, not the path.** A crate declares whether it is a capability, a
  library, an app or a tool. `service.toml` already exists for capabilities. CONTRIBUTING.md
  ("Three architectural nouns") changes in the same commit as the move.
- **API crates.** `crates/<cap>-api` holds serde types and route constants with no server
  dependencies, so it builds for iOS. The capability and `apps/gpui/core` both depend on it, and
  the copied structs in `core` are deleted. `libs/roomplan-contract` is the pattern.
- **Crate shape.** Every capability is a library plus a `main.rs` of about 20 lines. This allows
  tests in the same process and a later single daemon.
- **Splits and merges.** Split `comms` and `sjel-cli` along their existing modules where a module
  has its own consumers. Merge micro-crates only when they have the same consumers.
- **Hybrid process model, later.** After the move, measure build time and disk. If they justify
  it, one daemon hosts most capability libraries. `agent` and any capability that touches paths
  outside Sjel's data stay separate processes.
- **Build trees.** One release location instead of `./target/release` and
  `~/.cargo-target/release`, as decided 2026-10-24. It touches every `service.toml` `command` and
  the `sjel-mcp` path, so it goes in the same change as the move. `tools/storage/storage` measures
  the trees.

### How the move happens

Other agent sessions edit this checkout at the same time, and a move of every crate breaks
their open edits.

1. After the dashboard is deleted (fewer paths to move).
2. Announce a freeze. Commit or finish open work first.
3. One generated commit: `git mv` per crate, then path rewrites in manifests, `service.toml`
   commands, tools, CI and docs. A script does the rewrite, and it stays in `tools/` for review.
4. `cargo check --workspace`, `tools/doctor`, the CI legs, and a service restart prove it.
5. Then the API crates and crate-shape changes, one capability per commit.

Naming (decided 2026-10-25): the directory name equals the package name. The `sjel-` prefix stays
only on binaries a user types (`sjel`, `sjel-agent`, `sjel-mcp`).

- **Efficiency.** Run `simplify` on each module as it moves.

## 4. The public page

The demo site at `larsboes.github.io/Sjel/` goes away with the dashboard. Decided 2026-10-25
(D2): a new site replaces it, with screenshots and animations of the GPUI app, the
technical docs, and a description of what the app is. It comes after the dashboard is deleted,
at the end of Lane A.

`tools/demo-shots` captures the Mac app against the demo stack in a container, beside a live
node. Open:

1. iPhone screenshots. The simulator must join the container's node, not the Mac's live one,
   so the demo stack needs the LAN listener and the `devices` capability.
2. The seed dates its data around March 2026, so Today, Calendar and Travel show little on a
   later date.
3. Finance draws its key hints over the last visible row (`gpui-finance.png` is cropped).
