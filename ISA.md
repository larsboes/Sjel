---
project: sjel
type: isa
phase: climbing
progress: 89
principal_stated_goal: "I want no new issues, I wanna get rid of all issues for sjel and sjel personal and only carry through normal ISAs etc."
---

# ISA · Sjel

Keep the current ISA organization while cleaning up open work incrementally. Claims stay
until their probes pass or the principal withdraws them with a reason. Explanations and ideas
belong in the owning README or `docs/`. Remove completed and withdrawn items after their
enduring contracts reach their owners; Git keeps the history.

Decisions live with their owner ([rule](CONTRIBUTING.md#decisions-live-with-their-owner)), never
here. Verification rules live in the `sjel` skill's `on-verification.md`.

Outside bug reports may use GitHub Issues; our backlog stays here. Nothing automatically
creates a work item. Move enduring content to its owner before closing a claim. Do not create
another planning file during cleanup. A possible later migration to one `PLAN.md` is deferred;
see [planning rules](CONTRIBUTING.md#the-backlog-is-isas). Keep external bug reporting available.

## Features

### F6 · What the old PRD still owes

- [ ] ISC-22 — the three success criteria are measured again: no raw data about other people in
  the egress log, a trip planned in under 30 minutes, and one surface in place of five apps.
  Falsifier: any of the three has no command or log query that shows its current value.
  Probes:
  - Value 1 (Egress privacy): `comms egress-log --audit` reports 0 raw C2 violations.
  - Value 2 (Trip planning): operator plans one real trip from request to bookable legs. Threshold: under 30 minutes.
  - Value 3 (One surface): operator records each time a replaced app is opened for a covered task over 7 days. Threshold: 0 entries.
- [ ] ISC-25 — the product rules answer the counter-evidence in `research/`: a cloud request
  carries only the fields its task needs (Staab et al.), and confirmations are rare enough to be
  read (Akhawe and Felt), with the prompt rate measured. Falsifier: rule 4 or 5 unchanged with
  no measurement that answers the source.
  Probes:
  - Field minimization: verified by `cloud_derivative_carries_only_fields_task_needs_answering_staab_et_al`.
  - Prompt rate: divide approvals in `secrets/agent/approvals/` by lines in `secrets/agent/calls/<YYYY-MM>.jsonl` over one calendar month. Threshold: set by principal after first month.
- [ ] ISC-76 — each of the PRD's five non-goals has a current ruling in root `README.md`
  `## Scope`. N1 "not a product to sell" and N3 "tailnet only, no inbound webhooks" conflict with
  the distribution work and the LAN listener. N4 "not a replacement for Obsidian" was re-opened
  for projects on 2026-10-09. N2 "not a full local LLM host" and N5 "not multi-user" are
  unconfirmed. Falsifier: `## Scope` has no ruling on one of the five.

### F9 · One pseudonymizer, applied by the gate to every agent read

- [ ] ISC-45 — an agent cannot go around the gate. Falsifier: an agent session accesses unredacted data or secrets without going through the gate.
  Open operator actions:
  1. In a new agent session, verify `Read` of `secrets/**` is refused by the user-level configuration floor.
  2. Find a socket path the Tailscale system extension may connect to, complete
     `tools/setup-tailnet-shell.sh`, and verify `/health` 200 through Tailscale Serve.

## Unit work

Each bullet is open work, including bullets without a checkbox. Unit-local IDs retain their
owner heading; `F4` under media is not the root feature F4.

### From `PLAN.md`

- **Capture from the phone.** The phone reaches Sjel only through the LAN listener, which admits
  only requests signed with a paired device's key. An iOS Shortcut cannot sign, so an iOS share
  extension in the phone app is the path; a shared-token endpoint was rejected because it opens
  the signed-only listener.
- **Model suggestions.** Domain rules suggest a tag; the model suggests nothing for what the rules
  leave.
- **Axon names.** Decided 2026-09-30: rename `axon` in names, headers, prose and the FDA
  launcher. `X-Axon-*` headers stay accepted until the phone app ships, and FDA is restored after
  the rename. The `AXON_*` fallback is retired. Config file fallback reads shipped
  (`sjel.local.toml` / `axon.local.toml` and `sjel.toml` / `axon.toml`). Persisted paths and
  compatibility names need their own migration; replacing prose alone does not close the rename.
  Probe: `git grep -n -i axon`.

### From the vault PRD `Axon`

Migrated 2026-10-15 from §13 of the frozen PRD (now the `projects` record `Sjel/PRD Axon.md`).

- **B49 — the mail-classification corpus labels.** The harness shipped 2026-09-07 and exported
  **121 fixtures, 0 labelled** (`<overlay>/config/comms-mail-stream-shadow.json`). The labels are
  the operator's hand judgement, written before any model output is read. The stream flip needs
  model agreement ≥ control + 10 points and a false-eviction rate ≤ 2%; urgency has its own gate.
  Falsifier: a label file with zero empty labels and a scored confusion.
- **B23 — the 4,096-token rung ladder.** `SystemLanguageModel.contextSize = 4096` and
  `libs/summarize::fits_context` encode it. When macOS 27 / AFM 3 lands (a 32k window on the PCC
  tier), the "Sectioned" rung may become local. A doctrine revision: the two documents move
  together.
- **B47 — the finance investment layer.** `instruments` holds all 13 records and `targets`
  carries EUR and a null `risk_free_rate_bp`, but `allocations` is empty. Open: decide the target
  cohort (active allocations sum to 10,000 bp ±50), fill a ticker where one is known (a wrong
  ticker prices the wrong company silently), and install the `finance-prices` launchd unit.
- **D15 — the reranking stage.** `bge-reranker-v2-m3-mlx` carries `verdict = "adopt"` and has
  never served a ranking; Ollama 0.33.3 answers 404 to `/api/rerank`. Waits for a local runtime
  that can serve it, or accept first-stage-only ranking and drop the `adopt` verdict.
- **B48 — the trip roll-up numbers.** The budget field, wire and clearing shipped; `by_companion`
  was cut. Open: a budget on a plan and a trip id on a transaction (both data entry). The finance
  per-trip route still answers 200 with five zeros for an untagged trip.

### media · F4 and one open item

- **F4 — perceptual layer, advisory.** No `media_phashes` or `media_reviews` table exists.
  Falsifier: a near-duplicate is proposed from a phash and reviewed.
- **No structure draft for `To Sort/`.** The two drafts in the overlay target removed Inbox
  trees, so the 229 files / 53 GB in `Library/To Sort/` have no reviewed destinations and
  `organize` cannot act. Naming categories is the principal's judgement.

### backup · one claim and one gap

- **BKP-5 — per-kind verification.** Each target kind counts as verified only when its own check
  passes. Only the local kind is rehearsed; iCloud and ssh are unproven. Falsifier: a restore
  from each kind is observed passing.
- **A `restic`/offsite target kind.** Named as future work, nothing built.

### traveler · three open items

- **F3 — the vault proposes.** `TELOS/Personal/Events Profile.md` is a draft; `scouting` already
  reads it. The route reads the TELOS notes, emits a per-field diff with the quoted span each
  change rests on, and writes nothing until accepted. Gated on which TELOS sections are
  admissible (`CURRENT_STATE` holds a different kind of fact from an interest list).
- **F4 — the offered-versus-chosen half.** Twenty-eight `option_set` items exist on one plan; one
  plan is one observation, and a preference inferred from a single choice is a guess.
- **F6 — companion patterns, pseudonymously.** `places_person_places` holds nineteen rows in
  state `proposed`, names in a plain column. The profile should learn company (solo, pair,
  group, family) through the register's person ids, never names.

### Packs/travel · three open items

- **`is_regional` disagrees between backends.** Same RE5 Bonn→Köln, 2026-08-20: dbnav `true`,
  dbweb `false`. The flag decides whether a leg is presented as Deutschlandticket-covered, so
  a false negative sells a ticket already held. Needs a spread of routes.
- **Recovering a type for dbweb's regional legs.** Both backends agree on `train_number`; a
  number→label resolution would make dbweb scorable. Nothing holds the mapping today.
- **An open segment has no home in the plan model.** A stage names a destination and date, an
  item a day, and status is draft/saved/archived — none says "this part is undecided, here are
  the branches". What would settle it: a second trip planned this way.

### dashboard · three claims and six design questions

- **DSH-2** — ⌃⌥Space opens Ask from any app and the question arrives in the drawer. Code exists
  (`+layout.svelte` reads `?ask=`, `sjel-status/session.rs` `landing`); only an operator restart
  is outstanding. Falsifier: the question does not arrive.
- **DSH-3** — `/finance` reads right with live data (three `DataTable`s, subscription editor,
  `SidePeek`). A live-data judgement, not a build.
- **DSH-9** — Calendar reads from a row replica on the device (`dashboard/README.md`, the
  replica). The node needs a change feed per table, `?since=<cursor>`. Entries carry `updated_at`
  (`capabilities/calendar/src/model.rs:131`), but `delete_entry` removes the row
  (`store.rs:521`), so the feed also needs tombstones. Until then the page keeps each loaded
  window in memory and dims it while it reloads. Falsifier: with the node stopped, a month
  never opened before still shows its entries once a sync has run.
- **Home: a ranked list, or a view per context.** Waited for declared joins (shipped 2026-10-06);
  now the next candidate.
- **A measured GPUI command palette.** `apps/gpui-prototype/src/workspace.rs` implements ⌘K
  search over pages and dashboard destinations. Open: measure start time and resident memory
  against Tauri, then decide whether it replaces the Swift ⌃⌥Space panel or opens beside it.
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

### distribution

- **DIST-1 — fresh-user Mac setup.** The preparation DMG bundles the native app and core
  runtime, but the app still assumes an existing node. Add create/connect-node first-run UI,
  per-user writable runtime placement, credential provisioning by explicit user action and
  authenticated browser/native entry. Probe: on a clean Mac user, install the notarized DMG and
  reach a working node without a source checkout, terminal or developer tools. Installation
  testing is deferred at the principal's request; see the operations workflow.
- **DIST-2 — release identity and recovery.** Preparation writes checksums and catalogues and
  can sign/notarize a DMG. Open: validate first launch and restart, signed package provenance,
  third-party notices/source availability, versioned runtime updates and rollback, and backup
  gates for database migrations. Probe: rehearse a compatible update and recovery on the same
  clean-user installation; record actual download, installed and peak setup sizes.
- **DIST-3 — capabilities installed through the app.** Calendar has a prebuilt archive format
  and a CLI verifier/explicit installer. Open: trusted catalogue download, app installation UI,
  prerequisite installation, activation, persistence and reviewed upgrades. Probe: a fresh user
  installs Calendar through the app, records a synthetic event, restarts and disables the
  capability without losing the event. No package installation is tested in this preparation.
- **DIST-4 — native iOS distribution.** Archive/export preparation is separate from the Tauri
  development installer. Open: signing/provisioning, physical-device validation, privacy and
  export declarations, review access and TestFlight acceptance. Probe: install a reviewed native
  build through TestFlight, pair with a synthetic test node and complete a supported flow.

### operator-profile · two items

- **An export that writes.** Export is dry-run only; a write needs its own reviewed contract.
- **Harness targets other than Claude Code.** Each needs its user-instruction path verified first.

### libs/agent · two items

- **Wave 2.** Subagents (`ext::subagents`), a full-screen TUI, line editing, and
  `capabilities/assistant` adopting the loop with read-only Sjel tools and no `bash`.
- **Measure the specialist claim.** A fixed set of Rust tasks in this workspace runs twice on the
  same model, with and without the `skills` and `cargo` extensions; the measure is the share that
  pass `cargo clippy --all-targets` with no new warnings. The task set does not exist yet.

### libs/links · known limits

- **Trips' own place namespace.** A destination's `obsidian-place:…` id does not resolve to the
  places registry, so a place cannot list the trips that went there. See `docs/capabilities/links.md`.

### comms · one open item

- **A `comms-server` outside the runner held the port.** On 2026-10-19 `service-runner.sh
  restart comms` refused because a `comms-server` started six days earlier still answered on
  8083 with no pid file, so the live service ran six-day-old code. Stopping that process and
  `start comms` fixed it; how it escaped the pid file is unknown. Falsifier: a `comms-server`
  answers on the port while the runner's pid file names no running process.

### projects · two gaps

- **No way to add an attachment.** The routes cover records only, and the `projects` CLI (on
  `PATH` since 2026-10-19 through `capabilities/projects/projects`) has `import` but no `attach`.
  Adding photos to a project on 2026-10-19 meant a `cp -n` into the overlay tree, which does
  what `Projects::attach` does without its path check. Falsifier: an agent adds a file through
  the API or the CLI, and a second add of different bytes is refused.
- **Attachment paths come back as identity tokens.** On 2026-10-19 about two thirds of the paths
  `GET /api/files` answered were one whole `<PHONE_…>` or `<TOKEN_…>` token. A camera file name
  such as `IMG_1234.jpeg` is not an identity, but the agent cannot read it. Falsifier: a path with
  no identity in it reaches the agent unchanged.

### harness · two items

- **`OPERATIONAL_RULES.md`, sorted.** From the 2026-08-22 skills audit (claim C7): sort the old
  LifeOS operational rules down to what is worth keeping, put them in the private overlay, and
  make them deployable to the user-level `CLAUDE.md`. The source is only in the overlay's LifeOS
  backup. Falsifier: a port that carries LifeOS-specific rules across, or edits made only in the
  live `~/.claude` tree.
- **A prior-art skill that searches.** Same audit, claim C8: rethink the prior-art and
  build-ladder skills from the start rather than wrap them. Falsifier: a skill that restates the
  ladder without a way to actually search.
