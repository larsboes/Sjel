---
project: sjel
type: isa
phase: climbing
progress: 89
principal_stated_goal: "I want no new issues, I wanna get rid of all issues for sjel and sjel personal and only carry through normal ISAs etc."
---

# ISA · Sjel

Repo-wide state of record, and since 2026-10-15 the only open-work surface: the per-unit
`ISA.md` files were folded in here, their closed history into `ISA-archive.md`, and their live
contracts into `docs/`. Repo-wide work and unit work both live here now.

## Problem

Sjel's work often crosses capability boundaries: the agent gate, the rename, the disk, the
privacy rules. Work that belonged to one capability or Pack had that owner's ISA until
2026-10-15, when eleven per-unit files were folded into this one. Work that crosses owners, or
that no owner holds, needs one record that says what *done* means and how to check it.

Until 2026-08-20 that record was GitHub Issues. An issue has a description and a lifecycle,
but no falsifier, so closing one proves nothing. Two records also disagree as soon as someone
edits one of them. F0 moved the backlog here (archived, `ISA-archive.md`).

## Vision

One surface. Open work is a claim in an ISA with a probe that would falsify it; the
tracker holds nothing, and no automation creates entries in it.

## Out of Scope

- Disabling the issue tracker for the outside world. Sjel is public; an external bug
  report still needs somewhere to land. What changes is that *our* backlog is not there.
- Rewriting closed-issue history. Closed issues stay readable as the record of what was
  once tracked.

## Principles

- **A work item exists because the principal decided it does.** Nothing auto-creates one.
- **Doctrine lives where the tool reads it.** A rule in the README that `tools/doctor`
  contradicts is not a rule.
- **Migrate before closing.** Content that outlives the issue moves into the owning ISA
  or the owning README first; the closing comment names where it went.

## Constraints

- **C1** — `cargo test --workspace --locked`, `bun test` and the `tools/check-*.sh` gates
  before claiming the gates pass. `bun test` is named because it is the only runner that
  reaches `tools/*.test.ts`, and since 2026-09-05 four dashboard build gates live nowhere
  else — `dashboard-tokens`, `dashboard-contrast`, `dashboard-home-bands` and
  `dashboard-number-bindings`, the last covering a class `bun run check` structurally cannot
  see. `tools/check-architecture-fresh.sh` is the only check that catches a stale generated
  `ARCHITECTURE.md`. Was `bazel test //...` until PRD Q44 retired Bazel (2026-08-25), then
  carried `-- --skip postgres_tests::` until PRD Q45 retired the server (2026-08-27) — the
  suites run on temp files and need no skip.
- **C2** — `tools/self check` compares only what tracked files produce, so it passes in a fresh
  clone and gates in CI's repo-gates job. Per-unit code counts are fused on read from
  `graphify-out/` and are not committed (2026-09-29): while they were committed, a machine without
  a graph could see the drift and not repair it, which is what held `main` red behind ten armed
  Dependabot pull requests. A graphless verdict is therefore trustworthy — but this machine's
  graph may still be behind the tree, so `status` can show counts rolled up from a stale one.
  Verify a claim about the gate in a `git worktree` of origin/main, not in place.
- **C3** — sweeps run `rg --no-ignore --hidden --follow`; plain `rg` honours `.gitignore`
  and hides most of the private overlay's `config/`.
- **C4** — `service-runner.sh status` prints the DECLARED image reference, and since
  Q77 (2026-09-02) that reference is a CHANNEL — `:stable`, `:latest`, `:alpine`. It
  names what the capability tracks, never what it is running. The digest is the only version
  fact: `docker image inspect <image>:<tag> --format '{{index .RepoDigests 0}}'`, or
  `docker inspect <name> --format '{{.Image}}'` for the container itself
  (`docker ps --format '{{.Names}} {{.Image}}'` re-prints the channel, so it does not answer
  this). `report_arg_drift` compares ports, mounts, caps and network, never the image, so a
  moved digest shows as no drift at all — `capabilities/container-refresh` is what pulls and
  recreates instead. A backup archive records the digest its container was running as
  `image_digest` in `axon-backup.toml`, and `tools/restore.sh` prints it: an archive's own
  `tag` is the same channel string as every other archive's, so nothing else in it says which
  build wrote the bytes. Was `container list` until Q75 retired apple-container on 2026-09-02.

## Goal

Every open claim in this file is verified by its probe or withdrawn with a reason. No open
issue exists in this repository or in the private overlay, and no automation can open one.
When the last claim of a feature is ticked, the feature moves to `ISA-archive.md`. This file
holds open work, unplaced ideas and the decisions that bind them.

## Features

### F3 · Axon becomes Sjel, staged

Why: the product was renamed Sjel; repo-wide crates, identifiers, and configuration move in stages.

- [x] ISC-10 — what people see says Sjel: the GitHub repository, README title, app `productName` and demo site URL. Falsifier: `gh repo view --json name` is not `Sjel`, or `dashboard/src-tauri/tauri.conf.json` still says `LifeOS`.
- [x] ISC-11 — `sjel` command runs every subcommand. Superseded by ISC-27.
- [x] ISC-12 — iPhone app has a Sjel bundle identifier (`com.larsboes.sjel`) and is paired. Falsifier: `com.lifeos.mobile` remains in `tauri.conf.json`, or app fails `devices/api/devices/me`.
- [x] ISC-13 — internal names move to `SJEL_*` and `sjel-*`. `AXON_*` fallbacks retired. Falsifier: a service that ran before the change does not start after it.
- [x] ISC-27 — `axon` symlink is gone and `sjel` is the only entry point. Falsifier: `axon` still resolves on PATH or at repo root. Probe: `command -v axon`, and `rg '$AXON|"axon"' tools/ Packs/`.
- [x] ISC-28 — signed-request headers are `X-Sjel-*`, legacy `X-Axon-*` accepted until phone ships. Falsifier: paired phone signed request fails after rename. Probe: `libs/sjel-server/src/auth.rs`.
- [ ] ISC-29 — `axon-fda-launcher` is renamed, and its Full Disk Access grant is intact
  afterwards. Falsifier: the binary runs without FDA and cannot read what it needs. Probe: run
  it and confirm the grant. Renaming drops the grant, so re-granting is part of the work.
  Progress: binary and crate are `sjel-fda-launcher`, signed as `dev.sjel.fda-launcher`. Open
  until the grant is given again and a run reads what it needs.
- [x] ISC-30 — "Axon" no longer appears in prose and doctrine. Falsifier: `rg -i axon` over tracked documents returns hits that are not historical record or pinned identifiers. Probe: `rg -ci axon` over `README.md ISA.md CONTRIBUTING.md ARCHITECTURE.md`.

### F5 · The session's work, proven on real devices

Why: prove transport protocols, device pairing, and privacy relays on real hardware.

- [x] ISC-17 — phone on home Wi-Fi reaches Mac `:8443` listener, pins it after code comparison, and reads data with tailnet off. Falsifier: `curl -k https://<LAN address>:8443/health` from another device does not answer 200.
- [x] ISC-18 — assistant drawer calls model ladder (`dashboard/src/lib/intelligence`) and renders answering rung badge (`on-device`, `mac`, `rules`). Falsifier: `rg "intelligence/backends" dashboard/src` finds no caller outside module and test.
- [x] ISC-19 — native Mac companion and CloudKit relay in `apps/mac` host encrypted sync service per [Product Rule 4](CONTRIBUTING.md#product-rules). Falsifier: field name or plaintext value of a C2 record readable in stored record. Verified by `CloudKitRelayTests`.
- [x] ISC-20 — comms review queue sends pseudonymized jobs via `prepare_pseudonymized` ([Product Rule 4](CONTRIBUTING.md#product-rules)). Falsifier: queued job payload carries a raw C2 entity. Verified by `queued_review_job_is_pseudonymized_and_never_leaks_c2_entities`.
- [ ] ISC-21 — the family deployment runs for a week without Lars touching it.
  Falsifier: any fix to it made by Lars in that week.

### F6 · What the old PRD still owes

Why: reconcile outstanding PRD requirements into falsifiable specifications.

- [ ] ISC-22 — the three success criteria are measured again: no raw data about other people in
  the egress log, a trip planned in under 30 minutes, and one surface in place of five apps.
  Falsifier: any of the three has no command or log query that shows its current value.
  Probes:
  - Value 1 (Egress privacy): `comms egress-log --audit` reports 0 raw C2 violations.
  - Value 2 (Trip planning): operator plans one real trip from request to bookable legs. Threshold: under 30 minutes.
  - Value 3 (One surface): operator records each time a replaced app is opened for a covered task over 7 days. Threshold: 0 entries.
- [x] ISC-23 — every outbound model call appears in egress log with token count and cost. Falsifier: a call site reaches a cloud model without writing a log row. Verified by `comms egress-log` CLI and tests.
- [x] ISC-24 — reviewed provider list (`providers.toml`) gates cloud calls. Falsifier: cloud call to unreviewed or expired provider admitted. Verified by `libs/inference/src/providers.rs` tests.
- [ ] ISC-25 — the product rules answer the counter-evidence in `research/`: a cloud request
  carries only the fields its task needs (Staab et al.), and confirmations are rare enough to be
  read (Akhawe and Felt), with the prompt rate measured. Falsifier: rule 4 or 5 unchanged with
  no measurement that answers the source.
  Probes:
  - Field minimization: verified by `cloud_derivative_carries_only_fields_task_needs_answering_staab_et_al`.
  - Prompt rate: divide approvals in `secrets/agent/approvals/` by lines in `secrets/agent/calls/<YYYY-MM>.jsonl` over one calendar month. Threshold: set by principal after first month.

### F8 · An agent reaches what Sjel holds, and only what it should

Why: the data and tools exist (33 registered capabilities), but a local assistant cannot find
or call them without the operator's hands. Measured 2026-09-30, asking "what is the latest mail
from Ollama": `sjel search mail` returned no capability although `capabilities/comms` sweeps
the inbox; `sjel capability call comms get /triage` returned `invalid or missing authentication
token`, because `capability_call` in `sjel` sent no credential at all. Three gaps, one
feature: an agent cannot find a capability, cannot authenticate to it, and nothing limits what
it does once it can.

Built 2026-10-01, in the order the claims are numbered, because each step needed the one
before it.

Guideline, from the principal (2026-09-30): Sjel is easy to use and easy to maintain. The
design underneath may be clever. It is stated in `README.md`, "Design rule: clever inside, easy
outside". It binds this feature: an agent's access must not need a hand-edited config file.

Measured inputs, all 2026-09-30 and all lower bounds:
### F8 · An agent reaches what Sjel holds, and only what it should

Why: local AI agents access capabilities through one authenticated, discovered, read-only path without exposing secrets or destructive writes.

- [x] ISC-33 — `sjel search <word>` finds a registered capability by its README and `/routes`. Falsifier: `sjel search mail` returns no `comms`. Probe: `capability_index_finds_comms_for_mail` in `tools/sjel-cli/src/search.rs`.
- [x] ISC-34 — every registered HTTP capability serves `GET /routes`, or declares a reason. Falsifier: a capability in `sjel capability list` answers `/routes` 404 without reason. Enforced by `tools/check-service-tomls.sh` and `tools/doctor`.
- [x] ISC-35 — every capability has a README. Falsifier: `ls capabilities/*/README.md` misses a directory.
- [x] ISC-36 — `sjel capability call <cap> get <route>` passes token via fd (`-H @<(...)`), never argv or environment. Falsifier: token leaks in `ps` or env. Held by `tools/capability-auth.test.sh`.
- [x] ISC-37 — token read on demand from user secret store (Keychain), with no plain credentials in repo or environment. Falsifier: client requires manual secret file editing or exported env secret.
- [x] ISC-38 — agent identity admits `GET` and `HEAD` only; writes refused. Falsifier: agent `POST` reaches a handler. Probe: `libs/sjel-server/src/agent.rs:192` tests.
- [ ] ISC-39 — a `GET` by an agent identity returns no value of class Secret, and no Others
  value it could not already read. Falsifier: an agent read returns a Secret row.
  Probe: `sjel capability mail` from an agent session returns pseudonymized c1/c2 rows and no c3.
  Operator check: read the two c1 rows where a code word stands beside a 4–8 digit number and confirm neither holds a Secret value.
- [x] ISC-40 — MCP server exposes capabilities' `/routes` as tools with per-capability mode gating. Falsifier: tool list contains write route without explicit grant. Implemented in `sjel-mcp` crate; verified by tests.

### F9 · One pseudonymizer, applied by the gate to every agent read

Why: the gate pseudonymizes responses automatically so capabilities cannot leak personal identities to external models.

- [x] ISC-41 — keyed tokens: value maps to token deterministically across sessions. Verified by `libs/pseudonymize` tests.
- [x] ISC-42 — JSON view tokenizes identity fields, strips c3 objects, preserves structural timestamps/enums. Probe: `libs/pseudonymize` unit tests.
- [x] ISC-43 — gate returns pseudonymized JSON for agent `GET`, returns 403 for `POST` or unadmitted capability. Probe: `libs/sjel-server` tests.
- [x] ISC-44 — `sjel capability mail` returns zero raw sender addresses. Falsifier: `@` in `from_addr`. Probe: `sjel capability mail`.
- [ ] ISC-45 — an agent cannot go around the gate. Falsifier: an agent session accesses unredacted data or secrets without going through the gate.
  Open operator actions:
  1. In a new agent session, verify `Read` of `secrets/**` is refused by the user-level configuration floor.
  2. Complete `tools/setup-tailnet-shell.sh` and verify `/health` 200 through Tailscale Serve.

### F11 · What nothing was moving

Why: report and update software across all host package managers through one coordinated surface.

- [x] ISC-51 — `sjel update` reports every class of installed software with its owner, exits 1 when anything is stale. Falsifier: installed software class unrecorded or stale row without owner. Probe: `sjel update --json`.
- [x] ISC-52 — `apply` moves only what nothing else owns; delegates rest to existing owners (`host-patch`, `agent-integrations`). Falsifier: `apply` runs package manager already owned.
- [x] ISC-53 — report distinguishes job execution from version freshness; never prints unchecked class as current. Falsifier: `--offline` run reports current without querying.
- [x] ISC-54 — pre-release sorting higher is named and not adopted. Falsifier: `apply` installs alpha/beta over stable. Probe: `cargo search tauri-cli` verification.
- [ ] ISC-55 — the Systems page shows what is stale and what nothing owns, from the tool's own
  `--json`, and can start one class moving. Falsifier: the page renders no updates panel, or
  `sjel update --json` exits non-zero without emitting it. Probe: `sjel update --json` and the
  page.
  Operator check: open the Systems page in a browser with operator credential and compare the updates panel row by row with `sjel update --json`.
- [x] ISC-56 — report never recommends upgrade violating constraints or marked deprecated. Falsifier: row offers action whose version peer range forbids.
- [x] ISC-57 — `tools/agent-integrations.sh update interceptor` updates tool and updates marker. Falsifier: verb exits non-zero or marker retains old date.
- [x] ISC-58 — newest stable release decides crate row, not pre-release. Falsifier: crate with newer stable reported current due to pre-release. Probe: crates.io versions query.
- [x] ISC-59 — audit reaches software installed outside checkout, and every apply records findings. Falsifier: installed package unreached by CycloneDX SBOM or `tools/audit`.
- [x] ISC-60 — `apply` removes leftovers via `--prune` and re-resolves only named crates via `--re-resolve`. Falsifier: `--prune` removes package pinned by parent, or `--re-resolve` affects unnamed crates.

### Closed features, in `ISA-archive.md`

Each feature below has every claim ticked. Its text, its test rows and the log entries from
before 2026-10-08 are in [`ISA-archive.md`](ISA-archive.md), under the same heading. A code
comment that cites one of these ISC numbers finds it there.

- **F0** · The tracker stops being the backlog · ISC-1…5
- **F1** · Demo seeding for Comms, Scouting and Transit · ISC-6, ISC-7
- **F2** · Upstream drift · ISC-8, ISC-9, ISC-26
- **F4** · The documents a stranger reads · ISC-14…16
- **F7** · What fills the disk is visible where it is managed · ISC-31, ISC-32
- **F10** · An agent writes, under a mode the owner sets · ISC-46…50
- **F12** · A shared link becomes a row, and a garment is a row like any other · ISC-61…63
- **F13** · The inventory stops living behind the floor plan · ISC-64…66

## Open work folded in (2026-10-15)

Eleven per-unit `ISA.md` files and the root `PLAN.md` were deleted on 2026-10-15. Their open
items are here; their closed history is in [`ISA-archive.md`](ISA-archive.md); their live
contracts are in `docs/`. Each item below is the open remainder, not the shipped whole.

### From `PLAN.md`

- **Capture from the phone.** The phone reaches Sjel only through the LAN listener, which admits
  only requests signed with a paired device's key. An iOS Shortcut cannot sign, so an iOS share
  extension in the phone app is the path; a shared-token endpoint was rejected because it opens
  the signed-only listener.
- **`#go` has no destination.** It stays in the inbox. It goes to `places`, which needs a write
  path for a place from a link or a note.
- **Model suggestions.** Domain rules suggest a tag; the model suggests nothing for what the rules
  leave.
- **Prices a page does not declare.** `inventory wunsch` reads a price only from meta tags. Uniqlo
  declares none and has no JSON-LD (measured 2026-10-13). Open: a per-shop reader, or a price
  typed in at triage.
- **Groups of wishes.** A group with one constraint for all its items ("travel-friendly, fewer
  but better") has no shape in `inventory`.
- **German identifiers to English.** About 700 identifiers in `interior` and `inventory` are
  German; many are JSON fields and database columns, so the move needs a store migration, a
  change to the phone outbox's queued payloads, and an import that still reads the old TOML keys.
- **Axon names.** "axon" appears in 522 tracked files (`axon.toml`, `axonStatus`, `axon-clip`,
  `AxonGlance`). The config file names need a fallback read for machines that still have the old
  name. The doc `axon.db` path in `docs/README.md` was corrected 2026-10-15.
- **Giant pages.** `routes/travel/+page.svelte` (2,021 lines, from 4,197) and
  `routes/feed/[id]/+page.svelte` (2,656, from 2,918). Split each the next time it is touched;
  the `lib/api.ts` split (4,700 → 820 plus `lib/api/*`) and the root-document size are done.

### From the vault PRD `Axon`

The vault `Projects/Sjel/PRD Axon.md` was already `status: archived` and frozen on 2026-09-26,
but its §13 open rows were never migrated. Verified against the tree on 2026-10-15; the rows
below are still real, the rest are recorded as superseded at the end.

- **B49 — the mail-classification corpus labels.** The harness shipped 2026-09-07 and exported
  **121 fixtures, 0 labelled** (`<overlay>/config/comms-mail-stream-shadow.json`). The labels are
  the operator's hand judgement, written before any model output is read. The stream flip needs
  model agreement ≥ control + 10 points and a false-eviction rate ≤ 2%; urgency has its own gate.
  Falsifier: a label file with zero empty labels and a scored confusion.
- **B53 — the private-capability backup contract.** `tools/backup.sh:63` resolves
  `service.toml` only under public `capabilities/`, so a private capability's declared
  `backup_target` is invisible and a private backup would write a valid, empty archive. Use the
  same public-or-overlay manifest resolution and duplicate-name refusal as capability startup,
  then test with a synthetic private manifest. *(The row's `AXON_DB_PATH` half is stale:
  `libs/sjel-config` reads `SJEL_DB_PATH`.)*
- **B23 — the 4,096-token rung ladder.** `SystemLanguageModel.contextSize = 4096` and
  `libs/summarize::fits_context` encode it. When macOS 27 / AFM 3 lands (a 32k window on the PCC
  tier), the "Sectioned" rung may become local. A doctrine revision: the two documents move
  together.
- **B47 — the finance investment layer.** `instruments` holds all 13 records and `targets`
  carries EUR and a null `risk_free_rate_bp`, but `allocations` is empty. Open: decide the target
  cohort (active allocations sum to 10,000 bp ±50), fill a ticker where one is known (a wrong
  ticker prices the wrong company silently), and install the `finance-prices` launchd unit.
- **B28 / B41 — the real flat's own data.** 38 of 42 rows declare nothing; `bett_bestand` is
  210 × 140 against a 100 cm door and does not say `zerlegbar`; R9 needs `[lage]` and glazing
  heights; 13 open needs carry no price. A second real flat is what B28 needs.
- **D19 — rotate the Obsidian Local REST API key.** A credential sits in the overlay's git
  history (5 gitleaks findings, one commit). The frontmatter writer uses the API (Q107), so it is
  now load-bearing rather than dormant. Rotate first; the history purge is a separate call.
- **D15 — the reranking stage.** `bge-reranker-v2-m3-mlx` carries `verdict = "adopt"` and has
  never served a ranking; Ollama 0.33.3 answers 404 to `/api/rerank`. Waits for a local runtime
  that can serve it, or accept first-stage-only ranking and drop the `adopt` verdict.
- **B48 — the trip roll-up numbers.** The budget field, wire and clearing shipped; `by_companion`
  was cut. Open: a budget on a plan and a trip id on a transaction (both data entry). The finance
  per-trip route still answers 200 with five zeros for an untagged trip.
- **B5 — M3–M5.** The always-on agent, one trip end to end, and public. Partly built; undesigned
  in detail.

*Superseded on migration, 2026-10-15:* **B50** (five list contracts publish no data class) —
`show_data_class` exists nowhere in the tree any more, so there is no field to add. The
unnumbered §13 question about `fn erklaert` and the `.base` folder references are vault-side and
stay in the PRD's history.

### media · F4 and three open items

- **F4 — perceptual layer, advisory.** No `media_phashes` or `media_reviews` table exists.
  Falsifier: a near-duplicate is proposed from a phash and reviewed.
- **Interrupted multi-file ingest and orphans.** A crash between path reservation and link, or
  an orphan in `.media-incoming`, still needs manual inspection. Falsifier: kill at each
  filesystem/SQLite boundary and find no unverified final file.
- **`reclaim`'s survivor must live under `--from`.** A file whose only other copy is on a third
  volume is reported `unique` and kept even when that third copy is mounted. Falsifier: name
  the survivor volume and have `reclaim` verify against it.
- **No structure draft for `To Sort/`.** The two drafts in the overlay target removed Inbox
  trees, so the 229 files / 53 GB in `Library/To Sort/` have no reviewed destinations and
  `organize` cannot act. Naming categories is the principal's judgement.

### backup · one claim and three gaps

- **BKP-5 — per-kind verification.** Each target kind counts as verified only when its own check
  passes. Only the local kind is rehearsed; iCloud and ssh are unproven. Falsifier: a restore
  from each kind is observed passing.
- **Concurrency and cancellation.** Two runs against one target, and what a stop does to a
  half-written archive. `tools/backup.sh` refuses a same-second collision; that is not a job model.
- **Run-record retention.** How long a failure history is kept, and whether it is pruned.
- **A `restic`/offsite target kind.** Named as future work, nothing built.

### traveler · four open items

- **F3 — the vault proposes.** `TELOS/Personal/Events Profile.md` is a draft; `scouting` already
  reads it. The route reads the TELOS notes, emits a per-field diff with the quoted span each
  change rests on, and writes nothing until accepted. Gated on which TELOS sections are
  admissible (`CURRENT_STATE` holds a different kind of fact from an interest list).
- **F4 — the offered-versus-chosen half.** Twenty-eight `option_set` items exist on one plan; one
  plan is one observation, and a preference inferred from a single choice is a guess.
- **F6 — companion patterns, pseudonymously.** `places_person_places` holds nineteen rows in
  state `proposed`, names in a plain column. The profile should learn company (solo, pair,
  group, family) through the register's person ids, never names.
- **One profile or one per person.** The store is keyed by id and nothing resolves a current one.
  The day a second profile is needed is the day the question is real.
- *(Superseded 2026-10-15, recorded for the record: per-trip weight overrides and the
  destination-half reader both shipped — `transit/ranking.rs` `override_weights`,
  `trips/plan_search.rs` `FACTOR_RETROSPECTIVE`.)*

### Packs/travel · seven open items

- **L1 — one outcome record.** `POST /api/plans/:id/outcome` after the next real trip. Not code.
  Gates L3's transfer-buffer calibration and the journey-reliability independence assumption.
- **L3 — the retrospective summary's companion half.** Three preconditions, in order: an origin
  refusal shipped on trips (met 2026-09-05); a key that is the register's person id rather than
  a name; a class column a mechanism reads. The last two are not met.
- **`is_regional` disagrees between backends.** Same RE5 Bonn→Köln, 2026-08-20: dbnav `true`,
  dbweb `false`. The flag decides whether a leg is presented as Deutschlandticket-covered, so
  a false negative sells a ticket already held. Needs a spread of routes.
- **Recovering a type for dbweb's regional legs.** Both backends agree on `train_number`; a
  number→label resolution would make dbweb scorable. Nothing holds the mapping today.
- **The cancellation flag end to end.** `originCancelled`/`destinationCancelled` are
  fixture-tested; no cancelled train has appeared live. Opportunistic.
- **Obsidian export.** Gated on there being something machine-derived worth exporting (L1).
- **An open segment has no home in the plan model.** A stage names a destination and date, an
  item a day, and status is draft/saved/archived — none says "this part is undecided, here are
  the branches". What would settle it: a second trip planned this way.

### dashboard · four claims and six design questions

- **DSH-2** — ⌃⌥Space opens Ask from any app and the question arrives in the drawer. Code exists
  (`+layout.svelte` reads `?ask=`, `sjel-status/session.rs` `landing`); only an operator restart
  is outstanding. Falsifier: the question does not arrive.
- **DSH-3** — `/finance` reads right with live data (three `DataTable`s, subscription editor,
  `SidePeek`). A live-data judgement, not a build.
- **DSH-4** — a page header does not overlap its description at narrow widths. `PageHeader.svelte`
  stacks `.lead` below 38rem; recompute before ticking.
- **DSH-7** — `Sjel.app` has an app icon, so its approval notifications show one. No `.icns` or
  `CFBundleIconFile`; `apps/mac/Resources/` holds only the menu-bar PDF.
- **Home: a ranked list, or a view per context.** Waited for declared joins (shipped 2026-10-06);
  now the next candidate.
- **Home's full "now" in the menu bar.** Needs a server endpoint first.
- **A GPUI command palette.** Measure start time and resident memory against a webview before
  building past `apps/gpui-prototype` (neither number exists). Open: replace the Swift ⌃⌥Space
  panel or open beside it; and what it searches beyond Ask.
- **Database views, from Obsidian Bases and Notion.** One collection, many saved views (table,
  board, list, calendar), each with its own filter, sort, group and properties. Saved views and
  per-view formulas do not exist. The planning owner is the first collection that needs it.
- **A graph view.** `libs/links` joins and vault wikilinks as one graph. Renderer decided when the
  node count is measured.
- **Import and export per capability, through one contract.** Each capability names its formats
  in and out, and the shell gives them one place.
- **Joins with nothing to return yet.** No ledger transaction carries a trip tag; layouts publish
  no join; trips keeps its own place namespace.

### places · five items

- **Photos layer.** Blocked on an indexer that respects PRD N4 (index and link, never copy).
  EXIF geotags would also be the best travel-history source.
- **MCC capture.** Round 2 captures address columns; Trade Republic's `mcc_code` is still
  discarded at import.
- **Self-hosted tiles.** The basemap still egresses to `tiles.openfreemap.org`; the seam is
  declared swappable to PMTiles.
- **Register proposals from calendar.** Calendar has no attendee field yet to derive from.
- **Origin guard hardening.** The default allowlist accepts any `.ts.net` host; pinning it is one
  overlay env away (`SJEL_PLACES_ALLOWED_ORIGIN_HOSTS`), unset by default.

### operator-profile · two items

- **An export that writes.** Export is dry-run only; a write needs its own reviewed contract.
- **Harness targets other than Claude Code.** Each needs its user-instruction path verified first.

### cv · one operator-gated claim

- **ISC-67** — `cv linkedin pull` reads the LinkedIn Member Snapshot API on this machine. The
  code exists (`src/linkedin.rs`); blocked on an operator LinkedIn developer app and token.
  Falsifier: `cv linkedin pull` answers without the token error.

### libs/agent · two items

- **Wave 2.** Subagents (`ext::subagents`), a full-screen TUI, line editing, and
  `capabilities/assistant` adopting the loop with read-only Sjel tools and no `bash`.
- **Measure the specialist claim.** A fixed set of Rust tasks in this workspace runs twice on the
  same model, with and without the `skills` and `cargo` extensions; the measure is the share that
  pass `cargo clippy --all-targets` with no new warnings. The task set does not exist yet.

### libs/links · known limits

- `decide` checks that an id is shaped like a person id, not that the person exists.
- A trip's companion chips open a person by name; the trip plan does not carry the decisions.
- **Trips' own place namespace.** A destination's `obsidian-place:…` id does not resolve to the
  places registry, so a place cannot list the trips that went there. See `docs/capabilities/links.md`.

### printing · one question

- **Does KlipperMCP replace the slicer probe?** Open since the capability shipped; never revisited.

### harness · one item

- **HARN-1 — a drifted copy cannot be discarded.** `tools/sjel-cli/src/harnesses/mutate.rs`'s
  `adopt_digest_policy_if_safe` refuses `sync`, `deploy` and `remove` when a materialized
  destination differs from the digest the ledger recorded, so the only verb that clears a drifted
  cell is `accept`, which imports the destination's text into the Pack first. Measured 2026-10-17
  on `design/ui-craftsmanship` in Claude Code, where the destination held the pre-DSH-5 table rule
  and a path that no longer exists (`libs/links/ISA.md`). Probe: a verb that replaces a drifted
  destination and re-records its digest without writing into the Pack.

## Operator checks

Four open claims wait on one action each that an agent session cannot take, by design. Each
line names that action. The claim text above holds the history.

- **ISC-29** — grant Full Disk Access to `sjel-fda-launcher` (signed as
  `dev.sjel.fda-launcher`). Then run it, and confirm that it reads what it needs.
- **ISC-39** — read the two c1 rows in which a code word stands beside a 4–8 digit number
  (`sjel capability mail`), and confirm that neither holds a Secret value.
- **ISC-45** — two actions. In a new session, try a read under `secrets/**`, and confirm that
  the user-level floor refuses it. Then find a socket path that the Tailscale system extension
  may connect to, run `tools/setup-tailnet-shell.sh`, and get `/health` 200 through Tailscale
  Serve.
- **ISC-55** — open the Systems page, and compare the updates panel row by row with
  `sjel update --json`.

ISC-21 is open too, but no single action closes it. It needs seven days of the family
deployment with no fix from Lars.

## Not yet specified

- **knowledge-graph link prediction over the vault.** `knowledge-graph` serves the code
  graph; the vault has one and nothing reads it. A completed run exists over 679 notes
  and 9,001 candidate pairs with structure, text and metadata features engineered
  (the quarry is a completed link-prediction notebook in a private lab checkout, read rather
  than copied; the overlay records where).
  Two things to know before consuming it: the snapshot uses the vault's old `Areas/`
  paths, so it predates the `Knowledge/` rename and the note count has roughly tripled;
  and precision matters more than recall, so the useful output is a short ranked list
  per note, not a score for every pair. No consumer yet — that is what keeps it here
  rather than in Features.
- **Open source or open core.** AGPL-3.0 keeps both possible. Undecided whether parts stay
  closed later.
- **Funding.** Support and sponsorship with a build-in-public video series (decisions, papers,
  measurements, in the style of James Simo's city-builder devlog), hosting, or a company.
  Undecided.
- **One Rust agent for the Sjel app and local coding.** Promoted 2026-10-04: `libs/agent`
  (crate `sjel-agent`) started with c4a13a85; its claims are under "Open work folded from unit
  ISAs". The
  product reasoning is in the vault, `Projects/Sjel/Agent/PRD sjel-agent.md`. The bar this entry
  set for replacing Pi stays in force: one household task and one coding task, measured against
  the existing path, before any Pi retirement. The privacy probes in ISC-39 and ISC-45 are still
  open.
- **The pseudonymizer as its own library.** Grow the data classes and the reversible
  pseudonymizer into a standalone Rust crate with its own README. Performance work (unsafe Rust included)
  only after a benchmark says where the time goes.
- **Private Cloud Compute as a ladder rung**, for pseudonymized prompts only ([product rule 4](CONTRIBUTING.md#product-rules)). The plugin
  reports its availability today and never calls it.
- **A fast structured-decision model as a rung**, the kind Jev is (typesafe.ai, 2026). Candidate,
  not measured.
- **Generative interface from the typed core** ([product rule 6](CONTRIBUTING.md#product-rules)). No design yet.
- **The on-device model path is untested on an eligible device.** The iPhone 14 Pro reports
  `deviceNotEligible`; a 15 Pro or later, or a Simulator, is needed.
- **The code graph's semantic step cannot run here.** graphify's semantic step calls
  `deepseek-ai/deepseek-v4-flash`, retired on 2026-08-07 (not re-checked since). The AST step
  still runs: `graphify-out/graph.json` was rebuilt on 2026-10-08. Since 2026-09-29 (ISC-26)
  `tools/self generate` needs no graph at all. What the graph loses without the semantic step is
  not measured. Commit `2f0feb6` says it regenerated `self.json`; only `ARCHITECTURE.md` changed.
- **From the old PRD, not yet planned.** Device loss as a threat (what a stolen phone exposes).
  A health domain (energy, sleep, training, records). Item intake: **the link half is built**
  (F12, 2026-10-05) — what remains is *scan an object*, which needs the Q63 ladder to end at
  something better than text, and a barcode path, which is `[tauri-plugin-barcode-scanner]`
  today and reads the Mac's pairing QR code. Travel: route composition, "where should I base
  myself", an accommodation source. People: Google Contacts beyond inbound, a TELOS import,
  trips from a person, meetup capture. An autonomy gate for a structured-decision model (apply
  at ≥ 0.95 confidence, otherwise a card). shellcheck as shell analysis. `tools/backup.sh`
  skipping private capability manifests. The PRD's non-goals, which conflict with Sjel ("not a
  product", "tailnet only") and need a new ruling rather than a copy.

**Filed 2026-10-08 from the vault's former `Projects/Axon/`.** The folder merged into
`Projects/Sjel/` that day. These four items were open there and are owned nowhere in this repo.
The vault copies now point here.

- **Bulk import of saved Instagram posts.** `comms` extracts one Instagram URL at a time
  (`capabilities/comms/README.md`, the per-source extractor table). Nothing imports a saved
  collection. The constraint the principal set (2026-08-12): the import must not break
  Instagram's terms of use, so that the account is not banned. Existing open-source importers are
  not surveyed yet.
- **The feed, installable by someone else.** A colleague fetches favourite podcasts daily and
  summarizes them with a local Whisper model, and said many people would want Sjel's feed if it
  were easy to set up (principal, 2026-08-12). The README quick start installs the whole system,
  and nothing installs the feed alone. Not measured: what a feed-only install would need.
- **`OPERATIONAL_RULES.md`, sorted.** From the 2026-08-22 skills audit (claim C7): sort the old
  LifeOS operational rules down to what is worth keeping, put them in the private overlay, and make
  them deployable to the user-level `CLAUDE.md`. The source is only in the overlay's LifeOS backup
  today. Falsifier carried over: a port that carries LifeOS-specific rules across, or edits made
  only in the live `~/.claude` tree.
- **A prior-art skill that searches.** Same audit, claim C8: rethink the prior-art and build-ladder
  skills from the start rather than wrap them. Falsifier carried over: a skill that restates the
  ladder without a way to actually search.

**Filed 2026-10-01 from the Sjel project roast.** Seven items the roast raised that nothing in this
repo owned. Each states what exists in that slot today, read from the tree on that date; none is a
claim yet, and the watch rows they name are in `upstreams.toml` with the questions attached.

- **No booking document reaches a trip.** A `.pkpass`, a forwarded reservation mail and a UIC
  barcode are all text in Sjel today: the Q63 ladder (Apple Vision through `tools/visocr`, xberg,
  Docling) ends at text, and the only barcode path in the tree is `[tauri-plugin-barcode-scanner]`,
  which scans the Mac's pairing QR code. Turning a reservation into a trip is manual.
  `[kitinerary]` is read AND run: on the Linux runtime it installs from Debian as
  `libkitinerary-bin`, reads stdin and writes schema.org JSON-LD, and extracted a JSON-LD
  `TrainReservation` from HTML intact. A plain iCal `VEVENT` came back empty, so an `.ics` is not
  a way in. It is still not adopted into a capability. macOS was measured as the expensive half:
  Homebrew has Qt6 but none of `kpkpass`, `kmime`, `kcalendarcore` or `kcontacts`, and KDE's
  prebuilt macOS arm64 tarball of the extractor downloads at 1.4 MB and **does not run** -- its
  dylibs are absolute paths from KDE's CI machine and that path is its only `LC_RPATH` -- so the
  workable route is a KDE Craft root built from KDE's prebuilt packages. The owner recorded the
  macOS port as a target and this feature as **optional and not core** on 2026-10-02, so nothing
  on the Mac is built; on the Linux runtime, where it is measured working, the install costs
  679 MB. Still unmeasured: a real `.pkpass`, PDF or UIC barcode from this household. `[db-rest]`
  and `[motis]` stay unread.
- **Bank imports are CSV only.** `capabilities/finance/src/import.rs` reviews CSV into the
  plaintext journal and calls CSV an edge format; that is the whole intake. `camt`, `ofx` and
  `mt940` appear nowhere in the tree, so a bank that exports XML has no path in. Unmeasured, and
  prior to any reader: which formats this household's own bank produces. Watched for their
  importer catalogues: `[beancount]`, `[firefly-iii]`.
- **Contacts and calendar speak Sjel's API, not a standard.** There is no vCard (RFC 6350),
  iCalendar (RFC 5545) or CalDAV/CardDAV code, and `capabilities/calendar` reaches Google through
  Google's own API. `[radicale]` is watched. The question that comes first is not which server but
  whether anything outside the phone app and the dashboard would ever read these records;
  `[vaultwarden]` was adopted because its clients already existed.
- **Semantic retrieval is a linear cosine, and the number that would change it is missing.**
  `capabilities/comms/src/relevance.rs` scores candidates in process and `libs/sjel-store` loads no
  vector extension. What is absent is the candidate-set size at retrieval time, which decides
  whether `[sqlite-vec]` is worth anything here. Measure, then delete or promote the row.
- **Dependency vulnerability scanning is five adopted tools, not a missing one.** `[osv-scanner]`,
  `[grype]`, `[trivy]`, `[gitleaks]` and `[semgrep]` are adopted and `osv-scanner.toml` is
  configured; a scanner written here would be a sixth. What is genuinely absent is the hook --
  nothing runs a scan on a schedule and surfaces a finding through the assistant, which is
  `host-watch`-shaped if it is built at all.
- **The pseudonymizer's 100% has a corpus boundary nothing states.** Recall reached 100.0% (48/48
  labels) on the frozen 24-fixture corpus in `deterministic-entity-redaction-v3`, and the
  `[presidio]` comparison is what set it. No document says what would falsify that next: a corpus
  size, a language or a document class where the current rules should be expected to fail. Until
  one does, 100% reads as a property of the redactor rather than of the corpus it was measured on.
- **The dashboard has no written rules for what a reviewer checks by eye.** The UI material filed
  beside the roast makes three points: the data's shape should drive the form (tabular figures,
  right-aligned amounts, chips for closed enumerations), secondary actions belong behind
  progressive disclosure, and most of a finished table is invisible (hover affordances, tooltips on
  every icon, empty and error states). Progressive disclosure is already
  `research/simplicity-and-adoption.md`; the other two have no home. Either they become a checklist
  in `dashboard/README.md` or they stay folklore.
- **A planning owner.** Decided 2026-10-09 (see Decisions): a new capability, working name
  `projects`, owns projects and tasks as markdown files outside the vault, indexed in SQLite.
  The name collides with the dashboard's `/projects` route, which lists capabilities, so one
  of them is renamed. Open before an ISA exists: where the root lives (overlay path or the
  store mount), which part of a vault project note moves (status, outcome, tasks) and which
  stays as knowledge with a link, and how decision-ladder band 620 switches from
  `capabilities/vault` to the new owner. Habits: which `calendar` rhythm fields carry a habit,
  and whether the daily metrics in `Journal/01. Daily Notes` (read by vault `Habits.base`)
  move or stay as journal entries. Vault `Domains/Journal.md` says nothing is written back
  into a journal entry.

## Test Strategy

| isc | type | check | threshold | tool | anchors_to |
| --- | --- | --- | --- | --- | --- |
| ISC-27 | command | `command -v axon`; `rg '$AXON\|"axon"' tools/ Packs/` | no resolution, no call | bash, rg | F3 |
| ISC-28 | code inspect | `libs/sjel-server/src/auth.rs` accepts `X-Sjel-*` and `X-Axon-*` | both accepted until the phone ships | rg | F3 |
| ISC-29 | command | run the launcher; confirm the Full Disk Access grant | reads what it needs | bash | F3 |
| ISC-30 | command | `rg -ci axon` over README, ISA, CONTRIBUTING, ARCHITECTURE | historical record and pinned names only | rg | F3 |
| ISC-21 | log | fixes to the family deployment over seven days | zero by Lars | git, overlay log | F5 |
| ISC-39 | command | `sjel capability mail` from an agent session, then the operator reads any code-like c1 row | no Secret value | sjel | F8 |
| ISC-45 | command | a `secrets/**` read from an agent session, and the tailnet shell through Tailscale Serve | refused; `/health` 200 | Claude Code, curl | F9 |
| ISC-55 | browser | the Systems page's updates panel against `sjel update --json` | every row matches | browser | F11 |
| ISC-22 | log | a timed real trip plan; seven days of app openings | under 30 min; no entry | operator | F6 |
| ISC-25 | log | approvals ÷ agent calls for one month, under `secrets/agent/` | not set | operator | F6 |

## Anti-claims

- [x] A1 — no new tracking surface replaces the tracker. Falsifier: a `TODO.md`,
  `PLAN.md`, `HANDOFF.md` or `ROADMAP.md` appears anywhere in the repo, gitignored ones
  included. Probe 2026-10-15: `git ls-files | grep -iE '(TODO|HANDOFF|ROADMAP|PLAN)\.md$'`
  returns nothing; the root `PLAN.md` was deleted that day and its open items folded into
  "Open work folded in".
- [x] A2 — no issue is closed whose content exists nowhere else. Falsifier: ISC-2 fails
  for any closed issue.
- [x] A3 — the public repo keeps a path for outside bug reports. Falsifier:
  `.github/ISSUE_TEMPLATE/` is deleted or issues are disabled repo-wide.

## Decisions

- **2026-10-15 — step 0 runs on the Longbridge gpui-mobile fork** (principal's call). The fork,
  `gpui-pre-mobile` (`upstreams.toml`, `gpui-mobile`), moved to `gpui-pre =0.3.9` on
  2026-10-13. That is the snapshot gpui-kit 0.7.3 uses, so step 0 does not force a pin. Step 0
  bumps `apps/gpui-prototype` from `gpui-pre 0.3.6` and gpui-kit 0.6.0 to that snapshot. Both
  clients use one split: the OS draws the chrome (AppKit or UIKit sidebar, toolbar, menus,
  glass), and GPUI draws the content. gpui-kit's `website/docs/mobile.md` describes the iOS
  side, and `apps/gpui-prototype/src/macos_glass.rs` tries the Mac side. Step 2 ports `context`,
  `calendar` and `inbox` first, and finishes the native `capabilities` page as the reference.
- **2026-10-11 — GPUI is the goal for every Sjel client, and Tauri and Svelte go when a port
  proves it** (principal's call, all recommendations accepted). This widens the 2026-10-09 GPUI
  entry below. The palette stays the first surface. The reason is the stack, not speed, because
  DSH-8 still shows that rendering is not where Sjel waits. One language and one build (`cargo`)
  replace Svelte, TypeScript, Bun and the Tauri CLI. The plan has four steps, and the switch
  waits for the last one:
  0. A toolchain test. One Cargo workspace builds `gpui-pre`, `gpui-kit` and `gpui-mobile` on
     one GPUI snapshot, and shows one gpui-kit list, fed over loopback HTTP, on the Mac and in
     the iOS simulator. If it fails, the iPhone half of this decision is reopened.
  1. The command palette, with start time and resident memory measured against the Tauri
     webview.
  2. A port of the dashboard's 20 routes (`dashboard/src/routes`) into one isolated app under
     `apps/`, not used day to day. Svelte and Tauri stay live, the iOS build included. Charts
     are drawn in GPUI. The map comes last, and gpui-kit's `gpui-webview` crate is its fallback.
  3. A test of the port and a review of what to rethink. Then a decision: switch, keep both, or
     stop.

  The base is the upstream Zed snapshot that gpui-kit follows. gpui-kit 0.7.1 depends on
  `gpui-pre` 0.3.8, which crates.io describes as a snapshot of `zed@279fe07`, published weekly.
  gpui-mobile pins Zed `5688167d` and `apps/gpui-prototype` pins `5f8a7413`. Both move to the
  same snapshot, because two GPUI copies in one build are two sets of types. `gpui-ce` is not a
  base, because neither gpui-kit nor gpui-mobile builds on it. Two risks are reported, not
  measured here: gpui-mobile has one maintainer and no release, and gpui-kit's author reports
  40–60% CPU while a complex GPUI view scrolls (`upstreams.toml`, `gpui-mobile` and `gpui-kit`).
- **2026-10-09 — the vault keeps knowledge, and Sjel takes projects, tasks and habits**
  (principal's call, all recommendations accepted). Obsidian keeps Knowledge, Atlas, Journal and
  Reflections. Projects, tasks and habits get a Sjel owner, because their value is the join with
  calendar, trips and finance, and a vault note cannot run a join. This **reverses PRD Q48**
  (2026-08-27, vault `Projects/Sjel/PRD Axon.md`, "Q48"), which returned the Action kind to
  `Projects/**/Tasks/` and deleted `capabilities/tasks` in `73a47ba1`. Principle 8 holds: the new
  owner writes markdown with frontmatter into a Sjel-owned root outside the vault, and its SQLite
  table is an index that can be rebuilt from those files. That is the shape vault
  `Domains/Planning.md` recommended on 2026-08-07: uniqueness becomes a parse-time check that
  names both paths, not a database constraint. Habits become `calendar` rhythms plus a daily
  metric record. The 72 vault notes typed `task` or `project` are imported once, then retired.
  Until the import runs, Q48 and the read-only `GET /api/tasks` in `capabilities/vault` stay in
  force. The owner itself is under Not yet specified, "A planning owner".
- **2026-10-09 — GPUI takes the resident keyboard surfaces, and Svelte keeps the rest**
  (principal's call). Measured first, on this Mac: the four calls behind `/context` answer in
  1–5 ms, a day switch settles in 26 ms in Chrome, and
  `GET /sjel-status/api/sjel-status/capabilities` takes 1.0–1.4 s (DSH-8). Rendering is not where
  Sjel waits, so this split is a placement rule, not a speed claim. GPUI gets surfaces that stay
  resident and are driven from the keyboard, where start time and memory count. The first is a
  command palette. Svelte keeps maps (`maplibre-gl-js`), charts (`vega`), forms, and every view
  the iPhone needs through Tauri, because GPUI has no iOS target. The Swift menu bar keeps
  Keychain, CloudKit and the global hotkey. `gpui` stays `watch` in `upstreams.toml` until the
  palette ships. The memory gap between a GPUI window and a webview is not measured. Widened
  on 2026-10-11 (above): Svelte and the Tauri iOS build stay only until a GPUI port replaces them.
- **2026-10-05 — clothing is a row, and a link is an intake** (principal's call, overruling the
  recommendation twice). Two questions, both answered against the smaller option. First: the
  three fields a garment needs (`category`, `groesse`, `farbe`, `saison`) go **on the same
  `interior_item` table** rather than into a wardrobe capability — the recommendation was to ship
  only `kind = gear` + `category = kleidung` and see whether the fields were missed, and the
  principal's answer was to add them now. Second: **both** intake paths, the link verb and the
  form — the recommendation was the link alone, and the form was argued for as the correction
  path, which is why it carries the fields in the *edit* form and not only the create form.
  Q58 was not reopened: it already said *"a tent and a wardrobe are the same row shape … do not
  fork it"*, so the only open question was which fields and which door. What the price reader
  refuses is part of the decision and not an implementation detail: `preis_cent` sums into the
  wishlist total that joins to `finance` (B29), so a guessed price moves a purchase decision
  while a missing one is counted and said out loud. A non-EUR price is refused rather than
  converted, because no exchange rate exists in this database.
- **2026-10-05 — the umbrella keeps its name, and the branch is read from the data**
  (principal's call, all three recommendations accepted). Asked because the principal reached for
  *"gear as an over-category"*: `gear` is not a taxonomy in this codebase but a **role** — `kind`
  is `piece` / `slot` / `gear`, and `gear` means *what travels instead of standing*, which is why
  the seven pack-list columns hang off it. So the umbrella stays the table Q58 already decided on,
  `interior_item`, and the name stays `interior` even though it is a misnomer the file header has
  admitted since B51: renaming it changes a table, a route and every bookmark and gains no
  behaviour, which is the one thing a rename has to earn. A garment is `kind = piece` — a thing
  that belongs to me — and **not** `gear`, because the first is true of a shirt in a wardrobe and
  the second would silently make every shirt a packing candidate the day `trips` builds *select
  gear for a trip*. The branch is the free-text `category`, and what it needed was not a schema
  but a **known list**: the words are read from the rows (`store::kategorien`), the form's
  suggestions are built from the same rows rather than a list written in Svelte, and two spellings
  of one branch (`kleidung` / `Kleidung`) are **named** by `store::kollisionen` and printed by
  `interior inventory` rather than silently merged. That is `budget::BEKANNTE_PRIORITAETEN`'s rule
  applied to a second free field: a list declared in code is a second truth about the same thing,
  and the first one somebody forgets. The fetch stays in the CLI and the form stays the correction
  path, so no route was added and the server still never fetches a URL a client chose.

- **2026-10-01 — the software nothing owns gets a report and a front door** (principal's call).
  `sjel update` reports every class of installed software with its owner, and `apply` moves what
  nothing else moves while delegating the rest to its owner. Recorded as F11. The UI was decided
  in a second round and overruled the read-only recommendation: the Systems panel gets an apply
  button, which is a mutation path from a web page to host package installs. What makes that
  acceptable is narrower than it sounds — sjel-status is the one capability that admits no agent
  token at all (ISC-47), the route inherits the origin guard and the session verifier from
  `build_router`'s layers, and `APPLY_CLASSES` refuses every class another tool already owns.
  cargo and npm stay unscheduled (decided the same day): they remain a report you run, and
  nothing compiles in the background at 3am.
- **2026-09-30 — the gate pseudonymizes agent reads; pseudonymized c2 may reach an agent**
  (principal's call). Recorded as F9. An own agent token, reversible per-session tokens, c3
  never. Amends Q27 for agent reads.
- **2026-09-30 — agents get read access first, through one authenticated path** (principal's
  call). Recorded as F8. Read-only by default, writes only by explicit grant per capability;
  secrets come from the user's own secret store per user, with Vaultwarden no longer the
  default. The order is discovery, then authentication, then the read-only scope, then MCP.
  Planning only that day: no gate code changed.
- **2026-09-30 — the rename's remaining surfaces are reopened deliberately** (principal's
  call), against what ISC-13 recorded on 2026-09-27. Three things it had deliberately kept are
  in scope again: the `X-Axon-*` signed-request headers (a protocol change, so the old name is
  accepted until the phone ships, ISC-28), `axon-fda-launcher` (renaming drops its Full Disk
  Access grant, so re-granting is part of the work, ISC-29), and "Axon" in prose (ISC-30). The
  `axon` symlink goes as well, superseding ISC-11 (ISC-27). The `AXON_*` environment fallback
  had already been retired earlier the same day, before this decision, and ISC-13 is amended
  to match.
- **2026-09-30 — the disk's own view belongs in the dashboard** (principal's call).
  `tools/storage` measures it and enforces R6; nothing showed it to a person. Recorded as F7.

## Log

Entries before 2026-10-08 are in [`ISA-archive.md`](ISA-archive.md#log).

- 2026-10-15 · The travel design snapshots retired. `Packs/travel/design/` held three dated
documents (2026-08-11/12); their still-live content — backend fragility, the do-not-build list, the
four open questions and the missing parquet-reader test — moved to `docs/capabilities/travel.md`
§4–6, and their open items were already in the Packs/travel list above. 1,216 lines of dated prose
that nothing read are gone.

- 2026-10-15 · Media F3 closed: the ledger has a read-only HTTP surface and a page that reads it.
  `media serve` answers `GET /api/status`, `/api/ingests` and `/api/ingests/{id}` on port 8102
  behind the shared inbound credential; `capabilities/media/service.toml` declares `proxy_api_only`,
  and `dashboard/src/routes/media/` reads it over the shell proxy. Verified through a fresh
  dev-server proxy (`/media/api/status` → 200 against a seeded ledger, a refusal returned named with
  its reason) and at the router (`every_read_route_answers_with_the_ledger_shape`). Contract:
  `docs/capabilities/media.md`. `media` is enabled on this machine; the manifest's arrival also
  retires the README's "a server manifest may come later" note.

- 2026-10-15 · Media's per-file pre-classification claim closed. A symlink, non-regular file,
  non-UTF-8 path or unreadable file is refused and recorded as a `refused` item instead of
  aborting the whole run (`files_with_refusals`, `staged_hashes`), so every path has a durable
  disposition. Test: `a_refused_path_does_not_deny_the_rest_its_disposition`.

- 2026-10-15 · Planning folded into one surface. Eleven per-unit `ISA.md` files and the root
  `PLAN.md` retired; open work is the section above, closed history is in `ISA-archive.md`, and the
  typed-id / `/api/links` contract moved to `docs/capabilities/links.md`. The vault PRD `Axon`
  (already archived) had its still-real §13 rows folded in. The route-schema ratchet shipped
  (`tools/check-route-schemas.sh`, wired into the repo gates): **92** undeclared write bodies
  baselined down-only — not the "fourteen" the travel ISA recorded on 2026-08-12.

- 2026-10-11 · GPUI becomes the goal for every client, iPhone included through gpui-mobile.
  The first step is a toolchain test, and the switch waits for a full port and a review.
  `upstreams.toml` updates `gpui` and `gpui-kit` and adds `gpui-mobile`.

- 2026-10-09 · Two decisions: projects, tasks and habits leave the vault for a Sjel owner,
  reversing Q48, and GPUI gets resident keyboard surfaces while Svelte keeps the rest. The
  measurement behind the second is DSH-8 in [`ISA-archive.md`](ISA-archive.md). The vault side is updated in
  `Domains/Planning.md`, the vault contract's kind table, and the Q48 callout.

- 2026-10-08 · Review of every ISA against the tree. Three claims changed state. ISC-65 is done:
  its routes left in b10e5260, and the box was not ticked. ISC-22 and ISC-25 are reopened,
  because their evidence measured a proxy (see each claim). Eight closed features, their test
  rows and the log moved to `ISA-archive.md`. "Operator checks" lists the one action each
  operator-bound claim needs. Problem, Vision and Goal now describe the repo-wide record that
  this file became, not the tracker migration (F0) that this file was created for.
