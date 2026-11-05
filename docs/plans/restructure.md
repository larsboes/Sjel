# Restructure and sweep

After the dashboard goes, the repository gets smaller, and every file changes once: layout, code
comments and documentation in the same pass. Lane A in [`PLAN.md`](../../PLAN.md), after
[`gpui.md`](gpui.md) retirement.

## 1. Delete

Done: `dashboard/` with `src-tauri`, the Tauri plugins in `plugins/`, the dashboard bundle on
the Pages site, the `sjel-status` static serve and `matches_dashboard_nav`. `demo/` stays: its
seeded stack feeds `tools/demo-shots`. Open:

- Done: Prose describing the retired dashboard swept across tracked READMEs and architecture docs.
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
| Interface contracts, data flows | `website/content/capabilities/<name>.md` | READMEs |
| History, measurements, who decided | The commit message | Anywhere tracked |
| An invariant the code cannot express | A one- or two-line code comment | READMEs |
| What a config value means, or its unit | One comment line on the key | Rationale paragraphs |
| A dependency verdict | One `upstreams.toml` row | READMEs, `Cargo.toml` comments |

Decisions stay with their owner, not in one central log. A central log goes stale because nobody
who changes the code reads it. `PLAN.md` keeps only decisions that span areas.

### `upstreams.toml`

It stays the one registry of dependency and influence verdicts (145 entries: 65 `adopt`, 31
`inspiration`, 24 `reject`, 15 `watch`, 6 `quarry`, 2 `evaluate`, 1 `adapt`, 1 `overlay`). A verdict
and its review date are data and stay. Shape per entry: verdict, review date, one or two sentences
of why (median 334 characters, maximum 632), and a link to `research/` when the analysis is longer.
Every `watch` and `evaluate` entry carries a `revisit` condition, or it becomes `reject`.

### Baseline

Tracked files, `git grep`, 2026-10-23 and 2026-10-25:

- 609 comment lines in `*.rs`, `*.toml`, `*.sh` carry a date. 337 say "measured" or "decided".
- 3,143 of 8,862 TOML lines were comments. Worst: `profiles.toml` 268 of 313, `toolchain.toml` 168
  of 456, `osv-scanner-installed.toml` 155 of 386, `Cargo.toml` 93 of 190, `systems.toml` 64 of 180,
  and 922 comment lines across all `service.toml` files. Now: `profiles.toml` 11 of 87,
  `toolchain.toml` 25 of 319, `systems.toml` 12 of 126, `Cargo.toml` 3 of 102, `upstreams.toml`
  29 comment lines, `osv-scanner-installed.toml` 6 of 246, and 51 of 455 across all `service.toml`
  files.
- "Axon" appears 804 times in 299 files.
- 78 sites in `libs/`, `capabilities/` and `tools/` cited IDs of the retired `ISA.md` (for example
  "ISA F10", "ISA.md C4"); 0 remain in those directories, `apps/`, `schemas/`, `website/` and the root config.
- Over 800 lines: `capabilities/comms/README.md`, `capabilities/calendar/README.md`.

### Order

1. A ratchet in `tools/doctor` or CI: each count above may fall, never rise.
2. Done for `Cargo.toml`, `apps/gpui/Cargo.toml`, `profiles.toml`, `toolchain.toml`,
   `systems.toml`, `upstreams.toml`, `osv-scanner-installed.toml`, and `capabilities/*/service.toml`.
3. Done: `tools/isa-hygiene.sh`, its test, the doctor ISA count, and every ISA ID citation in code,
   config and docs.
4. Code comments and READMEs, module by module, in the same pass as the layout change. READMEs
   also move to the neutral register (`on-documentation.md`, Register).
   Done: secondary capability README sweep (`ec859210`, refined in `cc44c287`).
5. "Axon" goes in the lines each pass touches, the rest in one rename commit. The compatibility
   paths need their own migration: `X-Axon-*` headers are still accepted until the phone app
   ships (the gate rewrites them), and `axon.toml` and `axon.local.toml` are still read after the
   `sjel.*` names (`tools/manifest-resolution.test.sh`). Persisted paths need a data migration.
   Check with `git grep -n -i axon`.

### Planning outside the plans

READMEs and manifests still hold open work. Rule: a README describes what is built and why, and
open work goes to the owning area plan or an issue (`on-documentation.md`, Plans).

Found 2026-10-25 by heading and by dated lines (resolved in 2026-11-03 sweep):

- Done: "Not built" sections in `capabilities/inventory`, `entities`, `software`, `people-registry`,
  `scouting`, `agent`, and `libs/agent` were removed or migrated to `## Considered and declined`.
- Done: `capabilities/trips/decisions/travel-seams-not-built.md` moved to `capabilities/trips/README.md`
  (`## Considered and declined`) and the decisions directory removed.
- Done: `places` ("Deliberately not built") moved to `## Considered and declined`.
- Done: `capabilities/trips/README.md`, `capabilities/transit/README.md`, `tools/sjel-cli/README.md`, `capabilities/documents/README.md`, `capabilities/finance/README.md`, `capabilities/scouting/README.md`, `capabilities/agentbox/README.md`, `capabilities/comms/README.md`, and `capabilities/calendar/README.md` swept to standard shape (`## Why this shape:`, `## Considered and declined`, no dated narratives or migration logs).
- Most dated lines in READMEs remaining: none above baseline.
- Most dated lines in TOML: `profiles.toml` (35), `toolchain.toml` (13). Review dates in
  `upstreams.toml` and waiver dates in `osv-scanner*.toml` are data, not history, and stay.

**README shape after the sweep.** What it is, `## Why this shape: <topic>`, how to run it, a link
to the technical manual in `website/content/capabilities/`, `## Considered and declined`. No history, no
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

1. After the dashboard is deleted (fewer paths to move) — done (`143003fd`).
2. Announce a freeze. Commit or finish open work first — done (`ec859210` through `42237f97`).
3. One generated commit: `git mv` per crate, then path rewrites in manifests, `service.toml`
   commands, tools, CI and docs. A script does the rewrite, and it stays in `tools/` for review
   (`tools/migrate-layout.ts`). Done in this change.
4. `cargo check --workspace`, `tools/doctor`, the CI legs, and a service restart prove it — done.
5. Then the API crates and crate-shape changes, one capability per commit. Done for
   `trips`: `crates/trips-api` (`trips-api`) extracts pure wire models and route constants
   shared with `sjel-trips` and `sjel-gpui-core`.
   Done for `calendar`: `crates/calendar-api` (`calendar-api`) extracts pure wire models,
   commitment enums, export types, and route constants shared with `sjel-calendar` and
   `sjel-gpui-core`.
   Done for `documents`: `crates/documents-api` (`documents-api`) extracts pure wire models,
   duplicate review decisions, split suggestion/partition contracts, and route constants
   shared with `sjel-documents` and `sjel-gpui-core`.
   Done for `finance`: `crates/finance-api` (`finance-api`) extracts pure wire models,
   dashboard projections, transactions, review candidates, subscriptions, and route constants
   shared with `sjel-finance` and `sjel-gpui-core`.
   Done for `inventory`: `crates/inventory-api` (`inventory-api`) extracts pure wire models,
   wishlist items, wish groups, mail proposals, comparison models, and route constants
   shared with `sjel-inventory` and `sjel-gpui-core`.
   Done for `projects`: `crates/projects-api` (`projects-api`) extracts pure wire models,
   project summaries, records, tasks, mail proposals, inventory links, and route constants
   shared with `sjel-projects` and `sjel-gpui-core`.
   Done for `entities`: `crates/entities-api` (`entities-api`) extracts pure wire models,
   entity records, fields, facts, duplicate pairs, vocabularies, and route constants
   shared with `sjel-entities` and `sjel-gpui-core`.
   Done for `import`: `crates/import-api` (`import-api`) extracts pure wire models,
   import plans, routes, skipped tallies, background jobs, status ledgers, and route constants
   shared with `sjel-import` and `sjel-gpui-core`.
   Done for `software`: `crates/software-api` (`software-api`) extracts pure wire models,
   app records, decision models, and route constants
   shared with `sjel-software` and `sjel-gpui-core`.
   Done for `soundscape`: `crates/soundscape-api` (`soundscape-api`) extracts pure wire models,
   presets, scale definitions, scene parameters, layer mixes, sessions, claims, and route constants
   shared with `sjel-soundscape` and `sjel-gpui-core`.
   Done for `status`: `crates/status-api` (`status-api`) extracts pure wire models,
   health and liveness models, registry entries, agent gates and approval models, backup records
   and responses, search hits, and route constants shared with `sjel-status` and `sjel-gpui-core`.
   Done for `feed`: `crates/feed-api` (`feed-api`) extracts pure wire models,
   feed entries, ranking relevance and evaluation, reader detail projections, data classification,
   cloud review and processing states, quality flags, refresh stats, and route constants
   shared with `sjel-gpui-core` and comms/feed services.
   Done for `web-links`: `crates/web-links-api` (`web-links-api`) extracts pure wire models,
   saved destinations, site groups, review candidates, history scan bounds, decision requests,
   and route constants shared with `sjel-web-links` and `sjel-gpui-core`.
   Done for `scouting`: `crates/scouting-api` (`scouting-api`) extracts pure wire models,
   event route kinds, routing bases, sources, opportunities, scored results, discover responses,
   calendar candidate and verdict contracts, and route constants shared with `sjel-scouting`
   and `sjel-gpui-core`.
   Done for `assistant`: `crates/assistant-api` (`assistant-api`) extracts pure wire models,
   conversation turns, assistant requests, cloud model provider discovery roles, key statuses,
   replies, streaming events, steer instructions, and route constants shared with `sjel-assistant`
   and `sjel-gpui-core`.
   Done for `transit`: `crates/transit-api` (`transit-api`) extracts pure wire models,
   stations, legs, journeys, punctuality cells, reliability, rankings, factors, train matches,
   split-ticket segments and confidences, contract boundaries, priorities presets, and route
   constants shared with `sjel-transit` and `sjel-gpui-core`.
   Done for `distribution`: `crates/distribution-api` (`distribution-api`) extracts pure wire models,
   feature metadata, package files, package receipts, installed packages/features, available features,
   release catalogues, sources, assets, validation helpers, and limits shared with `sjel-cli`
   and `sjel-gpui-core`.
   Done for `interior`: `crates/interior-api` (`interior-api`) extracts pure wire models,
   RoomPlan dimensions, walls, openings, fixtures, uncertainties, layout items, placements, spaces,
   alignments, reconciliations, proposals, jobs, review requests, and route constants shared with
   `sjel-interior` and `sjel-gpui-core`.
   Done for `devices`: `crates/devices-api` (`devices-api`) extracts pure wire models,
   pairing challenges, claims, join requests, devices, responses, key code / comparison code /
   QR parsing routines, and route constants shared with `sjel-devices` and `sjel-gpui-core`.
   Done for `setup`: `crates/setup-api` (`setup-api`) extracts pure wire models,
   probe marks, setup probes, steps, plans, verification and apply envelopes, LAN status,
   and route constants shared with `sjel-status`, `sjel-cli`, and `sjel-gpui-core`.
   Done for `media`: `crates/media-api` (`media-api`) extracts pure wire models,
   volume rows, ledger statuses, ingest runs and items, perceptual hashes and reviews,
   geotags, and route constants shared with `sjel-media` and `sjel-gpui-core`.
   Done for `places`: `crates/places-api` (`places-api`) extracts pure wire models,
   places, visits, person links, geocode requests and responses, search filters,
   and route constants shared with `sjel-places` and `sjel-gpui-core`.
   Done for `traveler`: `crates/traveler-api` (`traveler-api`) extracts pure wire models,
   profile constraints, ranking weights, paces, anchors, derived baseline projections,
   TELOS proposals, and route constants shared with `sjel-traveler` and `sjel-trips`.
   Done for `punctuality`: `crates/punctuality-api` (`punctuality-api`) extracts pure wire models,
   lookup queries, batch requests/responses, stop statistics, station results, coverage info,
   normalization routines, and route constants shared with `sjel-punctuality` and `sjel-transit`.
   Done for `operator-profile`: `crates/operator-profile-api` (`operator-profile-api`) extracts pure wire models,
   profile entries, statement inputs, field values, harness targets, classifications, requests, validation routines,
   and route constants shared with `sjel-operator-profile`.
   Done for `agent`: `crates/agent-api` (`agent-api`) extracts pure wire models,
   session requests and envelopes, terminal inputs and resizes, streams, tier definitions, session/terminal metadata,
   and route constants shared with `sjel-agent-capability` and `sjel-gpui-core`.
6. Retired legacy non-crate directories `capabilities/` and `libs/`: `agentbox`, `host-firewall`,
   and `shell` moved to `tools/`, `overlay.ts` consolidated into `tools/lib/overlay.ts`, and
   doctor updated.

Naming (decided 2026-10-25): the directory name equals the package name. The `sjel-` prefix stays
only on binaries a user types (`sjel`, `sjel-agent`, `sjel-mcp`).

- **Efficiency.** Run `simplify` on each module as it moves.

## 4. The public page

The live demo is gone (decided 2026-11-03). The public site is `website/`, laid out after
Vector's repository (decided 2026-11-03): `website/content/` holds the user docs (architecture,
capabilities, operations) and `docs/` keeps the contributor plans. `website/build` renders the
landing page, the docs with a sidebar, and one reference page per unit (contract, routes read
from each `const ROUTES` block, coupling, README), then fails on a broken internal link or a
private marker. No capability runs in the Pages job.

Open: screenshots and short animations of the GPUI app in the hero and the Two surfaces section.

Screenshots are picked by hand once the GPUI port is done (decided 2026-10-28).
`sjel demo up | mac <page> | down` (`tools/demo-shots`) captures the Mac app against the demo
stack in a container, beside a live node. The seed covers trips with held connections, places
and companions, the feed and Inbox, and mail in every stream, dated relative to the run.

Open:

1. [TODO: deferred] iPhone screenshots. The simulator must join the container's node, not the Mac's live one,
   so the demo stack needs the LAN listener and the `devices` capability.
2. [TODO: deferred] Mac screenshots and short animations of the GPUI app in the hero and the Two surfaces section.

Done:

- Finance draws key hints without overlapping the last visible row (fixed with bounded flex-1, scroll containers, and bottom padding in `apps/gpui/ui/src/finance.rs`).
- Seeded calendar events during trips carry the destination city and guarantee at least two events per trip, allowing Calendar to cluster trip drafts (`tools/demo-seed.ts`).
- `POST /places/api/places` validates `kind` upfront and maps CHECK constraint failures to 400 Bad Request (`capabilities/places/src/server.rs`).
