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
- **Sjel names.** Decided 2026-09-30: rename `sjel` in names, headers, prose and the FDA
  launcher. `X-Sjel-*` headers stay accepted until the phone app ships, and FDA is restored after
  the rename. The `AXON_*` fallback is retired. The config files are renamed: `sjel.toml`,
  `sjel.local.toml` and `sjel.local.toml.example` are the names on disk, and the pre-rename
  `axon.toml` / `axon.local.toml` are still *read* after them, so an older checkout resolves its
  overlay without an edit (`tools/manifest-resolution.test.sh` covers it). Persisted paths and
  compatibility names need their own migration; replacing prose alone does not close the rename.
  Probe: `git grep -n -i sjel`.

### From the vault PRD `Sjel`

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

- **Photos layer.** Blocked on an indexer that respects PRD N4 (index and link, never copy).
  EXIF geotags would also be the best travel-history source.
- **Self-hosted tiles.** The basemap still egresses to `tiles.openfreemap.org`; the seam is
  declared swappable to PMTiles.
- **Register proposals from calendar.** Calendar has no attendee field yet to derive from.

### distribution

- **DIST-1 to DIST-4 — a fresh-user Mac and iPhone install.** Tracked in
  [#235](https://github.com/larsboes/Sjel/issues/235) since 2026-10-20, for another contributor.

### libs/agent · two items

- **Wave 2.** Subagents (`ext::subagents`), a full-screen TUI, and line editing (`capabilities/assistant`
  now runs the loop on `POST /api/ask` with read-only Sjel tools and no `bash`).
- **Measure the specialist claim.** A fixed set of Rust tasks in this workspace runs twice on the
  same model, with and without the `skills` and `cargo` extensions; the measure is the share that
  pass `cargo clippy --all-targets` with no new warnings. The task set does not exist yet.

### harness · two items

- **`OPERATIONAL_RULES.md`, sorted.** (Resolved: old LifeOS rules sorted to durable principles in
  `<overlay>/config/OPERATIONAL_RULES.md` and `<overlay>/config/operator-profile.json`; stored in
  `operator-profile` and projected cleanly into `~/.claude/CLAUDE.md` via `operator-profile export --apply`.)
- **A prior-art skill that searches.** (Resolved: `Packs/coding/skills/prior-art` implements the 7-rung
  build ladder with executable search script `scripts/search-prior-art` across symbols, modules, crates,
  and upstreams; synced and verified.)
