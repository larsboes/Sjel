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

### F9 · One pseudonymizer, applied by the gate to every agent read

- [ ] ISC-45 — an agent cannot go around the gate. Falsifier: an agent session accesses unredacted data or secrets without going through the gate.
  Open operator action: in a new agent session, verify `Read` of `secrets/**` is refused by the
  user-level configuration floor.

## Unit work

Each bullet is open work, including bullets without a checkbox. Unit-local IDs retain their
owner heading; `F4` under media is not the root feature F4.

### From `PLAN.md`

- **Capture from the phone.** The phone reaches Sjel only through the LAN listener, which admits
  only requests signed with a paired device's key. An iOS Shortcut cannot sign, so an iOS share
  extension in the phone app is the path; a shared-token endpoint was rejected because it opens
  the signed-only listener.
- **Axon names.** Decided 2026-09-30: rename `axon` in names, headers, prose and the FDA
  launcher. The emitted headers are `X-Sjel-*` since 2026-10-01, and `X-Axon-*` stays *accepted* —
  the gate rewrites a legacy header before anything reads it — until the phone app ships. FDA is
  restored after the rename. The `AXON_*` environment fallback is retired. The config files are
  renamed: `sjel.toml`, `sjel.local.toml` and `sjel.local.toml.example` are the names on disk, and
  the pre-rename `axon.toml` / `axon.local.toml` are still *read* after them, so an older checkout
  resolves its overlay without an edit (`tools/manifest-resolution.test.sh` covers it). Persisted
  paths and compatibility names need their own migration; replacing prose alone does not close the
  rename. Probe: `git grep -n -i axon`.

### From the vault PRD `Axon`

Migrated 2026-10-15 from §13 of the frozen PRD (now the `projects` record `Sjel/PRD Axon.md`).

- **B49 — the mail-classification corpus labels.** The harness shipped 2026-09-07 and exported
  **121 fixtures, 0 labelled** (`<overlay>/config/comms-mail-stream-shadow.json`). The labels are
  the operator's hand judgement, written before any model output is read. The stream flip needs
  model agreement ≥ control + 10 points and a false-eviction rate ≤ 2%; urgency has its own gate.
  Falsifier: a label file with zero empty labels and a scored confusion.
- **B47 — the finance investment layer.** `instruments` holds all 13 records and `targets`
  carries EUR and a null `risk_free_rate_bp`, but `allocations` is empty. Open: decide the target
  cohort (active allocations sum to 10,000 bp ±50), fill a ticker where one is known (a wrong
  ticker prices the wrong company silently), and install the `finance-prices` launchd unit.
- **B48 — the trip roll-up numbers.** The budget field, wire and clearing shipped; `by_companion`
  was cut. Open: a budget on a plan and a trip id on a transaction (both data entry). The finance
  per-trip route still answers 200 with five zeros for an untagged trip.

### media · one open item

- **No structure draft for `To Sort/`.** The two drafts in the overlay target removed Inbox
  trees, so the 229 files / 53 GB in `Library/To Sort/` have no reviewed destinations and
  `organize` cannot act. Naming categories is the principal's judgement. (`F4` perceptual
  advisory layer shipped in `capabilities/media/src/perceptual.rs` with `media_phashes` and
  `media_reviews`.)

### projects · one open item

- **The rest of Atlas, Journal and Reflections leave the vault.** Ruled 2026-10-20
  ([projects README](capabilities/projects/README.md#why-this-shape-sjel-owns-projects-tasks-and-habits)).
  Done: Finance, Items, Interior, People, Events, Media, Reflections and `Journal/` (originals
  in the vault's `Archive/`). Open: Documents (`documents`), Focus and Personal. Then three
  follow-ups. `Knowledge/` cites the Media sources with 140 wikilinks that Obsidian no longer
  resolves; the store paths are unchanged (`Atlas/Media/X.md`). `comms` still writes saved
  items as source notes into the vault's `Resources/Sources/` (`comms/src/projection.rs`), the
  kind of note Media's sources were. And the vault bases and MOCs that showed the moved folders
  go. Then the vault bases and MOCs that
  showed those folders go, and the vault rules shrink into the `obsidian` skill. Falsifier: the
  vault holds a non-knowledge note outside `Archive/`.

### host-patch · one open item

- **The daily run exits 1 on three webpki advisories.** RUSTSEC-2026-0381, -0382 and -0383
  (`webpki-roots`, `rustls-webpki`, `webpki-root-certs`) are in the Cargo.lock published inside
  `tauri-cli 2.12.3` and `xberg-cli 1.3.10`, so no install on this Mac moves them. Open: accept
  each in `osv-scanner-installed.toml` with its reach and an end date, or upgrade when a
  maintainer republishes. Falsifier: `launchctl list com.sjel.host-patch` shows exit 0.

### backup · one claim

- **BKP-5 — per-kind verification.** Each target kind counts as verified only when its own check
  passes. Only the local kind is rehearsed; iCloud and ssh are unproven. Falsifier: a restore
  from each kind is observed passing.

### traveler · one open item

- **F4 — the offered-versus-chosen half.** Twenty-eight `option_set` items exist on one plan; one
  plan is one observation, and a preference inferred from a single choice is a guess. (`F3` vault
  proposals shipped on `GET /api/profile/proposals` with section gating; `F6` pseudonymous
  companion patterns shipped in `GET /api/profile/derived` keyed by `ent:` register IDs.)

### Packs/travel · three open items

- **`is_regional` disagrees between backends.** (Resolved: unified `is_regional_leg` checks both
  the `9G` attribute and Nahverkehr categories/names, so dbweb and dbnav agree on `is_regional: true`
  for RE5 and regional legs across routes.)
- **Recovering a type for dbweb's regional legs.** (Resolved: `resolve_train_label` bridges both
  backends on `train_number` via pre-seeded lines like RE5/RB26 and `{prefix}_train_labels` in
  transit's store; dbweb bare-number legs recover rider-facing labels and punctuality types and
  are scored.)
- **An open segment has no home in the plan model.** A stage names a destination and date, an
  item a day, and status is draft/saved/archived — none says "this part is undecided, here are
  the branches". What would settle it: a second trip planned this way.

### dashboard · one claim

- **DSH-3** — `/finance` reads right with live data (three `DataTable`s, subscription editor,
  `SidePeek`). A live-data judgement, not a build.

### places · three items

- **Photos layer.** (Resolved: `media_geotags` EXIF indexer respects PRD N4; `places` serves `GET /api/layers/photos` with bounding box filtering and cached coordinates.)
- **Self-hosted tiles.** The basemap still egresses to `tiles.openfreemap.org`; the seam is
  declared swappable to PMTiles.
- **Register proposals from calendar.** (Resolved: `attendees` array on calendar events with `places backfill calendar` extracting participants to registered contacts.)

### distribution

- **DIST-1 to DIST-4 — a fresh-user Mac and iPhone install.** Tracked in
  [#235](https://github.com/larsboes/Sjel/issues/235) since 2026-10-20, for another contributor.

### libs/agent · two items

- **Wave 2.** (Resolved: `ext::subagents` delegation with isolated child loop and tool turns, interactive line editing with history recall (`/history`), screen clearing (`/clear`), command help (`/help`), and termination handling).
- **Measure the specialist claim.** (Resolved: canonical 5-task Rust refactoring benchmark suite in `libs/agent/tests/specialist_tasks.json` with metric calculation evaluating clean clippy pass rates and delta in `libs/agent/tests/specialist_claim.rs`).

### harness · two items

- **`OPERATIONAL_RULES.md`, sorted.** (Resolved: old LifeOS rules sorted to durable principles in
  `<overlay>/config/OPERATIONAL_RULES.md` and `<overlay>/config/operator-profile.json`; stored in
  `operator-profile` and projected cleanly into `~/.claude/CLAUDE.md` via `operator-profile export --apply`.)
- **A prior-art skill that searches.** (Resolved: `Packs/coding/skills/prior-art` implements the 7-rung
  build ladder with executable search script `scripts/search-prior-art` across symbols, modules, crates,
  and upstreams; synced and verified.)
