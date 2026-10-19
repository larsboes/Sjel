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

Outside bug reports may use GitHub Issues; our backlog stays here. Nothing automatically
creates a work item. Move enduring content to its owner before closing a claim. Do not create
another planning file during cleanup. A possible later migration to one `PLAN.md` is deferred;
see [planning rules](CONTRIBUTING.md#the-backlog-is-isas). Keep external bug reporting available.

## Constraints

- C1: claim the gates pass only after `cargo test --workspace --locked`, `bun test`, the
  `tools/check-*.sh` gates and `tools/check-architecture-fresh.sh` pass. Bun reaches
  `tools/*.test.ts`, including the four dashboard build gates.
- C2: `tools/self check` compares tracked inputs and must pass without Graphify. Graph-derived
  counts are fused on read and may be stale. Verify origin/main gate claims in a worktree.
- C3: overlay sweeps use `rg --no-ignore --hidden --follow`; plain `rg` omits ignored config.
- C4: container status reports a declared channel, not the running version. Digests identify
  the running image and the backup writer. `container-refresh` owns pull/recreate; argument
  drift omits images. See [container image identity](docs/operations/persistence.md#container-image-identity).

## Features

### F5 · The session's work, proven on real devices

- [ ] ISC-21 — the family deployment runs for a week without Lars touching it.
  Falsifier: any fix to it made by Lars in that week.

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
  Open operator actions:
  1. In a new agent session, verify `Read` of `secrets/**` is refused by the user-level configuration floor.
  2. Find a socket path the Tailscale system extension may connect to, complete
     `tools/setup-tailnet-shell.sh`, and verify `/health` 200 through Tailscale Serve.

### F14 · Sjel owns projects and tasks

Built and imported 2026-10-18: `capabilities/projects`, contract in its README. The store holds
674 notes and 761 attachments; `projects verify` checks them against the rendered tree. The
vault's `Projects/` now holds only its law, `Projects/Sjel/Knowledge-Base/`. Everything else moved
to vault `Archive/Projects/` as a frozen backup, and the vault's task reader retired.

- [ ] ISC-75 — the vault backup is deleted once the operator calls the owner final.
  Falsifier: vault `Archive/Projects/` still exists after that call. Probe: `ls` it.

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
- **German identifiers to English.** About 700 identifiers in `interior` and `inventory` are
  German; many are JSON fields and database columns, so the move needs a store migration, a
  change to the phone outbox's queued payloads, and an import that still reads the old TOML keys.
- **Axon names.** Review remaining tracked references with `git grep -n -i axon`.
  Config file fallback reads shipped (`sjel.local.toml` / `axon.local.toml` and `sjel.toml` / `axon.toml`).
  Persisted paths and compatibility names need their own migration; replacing prose alone
  does not close the rename.


### From the vault PRD `Axon`

The vault `Projects/Sjel/PRD Axon.md` was already `status: archived` and frozen on 2026-09-26,
but its §13 open rows were never migrated. Verified against the tree on 2026-10-15; the rows
below are still real, the rest are recorded as superseded at the end.

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
- **B28 / B41 — the real flat's own data.** 38 of 42 rows declare nothing; `bett_bestand` is
  210 × 140 against a 100 cm door and does not say `zerlegbar`; R9 needs `[lage]` and glazing
  heights; 13 open needs carry no price. A second real flat is what B28 needs.
- **D15 — the reranking stage.** `bge-reranker-v2-m3-mlx` carries `verdict = "adopt"` and has
  never served a ranking; Ollama 0.33.3 answers 404 to `/api/rerank`. Waits for a local runtime
  that can serve it, or accept first-stage-only ranking and drop the `adopt` verdict.
- **B48 — the trip roll-up numbers.** The budget field, wire and clearing shipped; `by_companion`
  was cut. Open: a budget on a plan and a trip id on a transaction (both data entry). The finance
  per-trip route still answers 200 with five zeros for an untagged trip.
- **B5 — M3–M5.** The always-on agent, one trip end to end, and public. Partly built; undesigned
  in detail.


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

### dashboard · three claims and six design questions

- **DSH-2** — ⌃⌥Space opens Ask from any app and the question arrives in the drawer. Code exists
  (`+layout.svelte` reads `?ask=`, `sjel-status/session.rs` `landing`); only an operator restart
  is outstanding. Falsifier: the question does not arrive.
- **DSH-3** — `/finance` reads right with live data (three `DataTable`s, subscription editor,
  `SidePeek`). A live-data judgement, not a build.
- **DSH-9** — Calendar reads from a row replica on the device (decision 2026-10-19). The node
  needs a change feed per table, `?since=<cursor>`. Entries carry `updated_at`
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

### printing · one question

- **Does KlipperMCP replace the slicer probe?** Open since the capability shipped; never revisited.

### projects · two gaps

- **No way to add an attachment.** The routes cover records only, and the `projects` CLI is not
  on `PATH`. Adding 48 photos to `Waschmaschine/Assets/` on 2026-10-19 meant a `cp -n` into
  the overlay tree, which does what `Projects::attach` does without its path check.
  Falsifier: an agent adds a file through the API or an installed CLI, and a second add of
  different bytes is refused.
- **Attachment paths come back as identity tokens.** On 2026-10-19 `GET /api/files` answered
  809 paths, and 525 of them were one whole `<PHONE_…>` or `<TOKEN_…>` token. A file name such
  as `IMG_0700.jpeg` is not an identity, but the agent cannot read it. Falsifier: a path with
  no identity in it reaches the agent unchanged.


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
- **Private Cloud Compute as a ladder rung**, for pseudonymized prompts only ([product rule 4](CONTRIBUTING.md#product-rules)). The plugin
  reports its availability today and never calls it.
- **A fast structured-decision model as a rung**, the kind Jev is (typesafe.ai, 2026). Candidate,
  not measured.
- **Generative interface from the typed core** ([product rule 6](CONTRIBUTING.md#product-rules)). No design yet.
- **The on-device model path is untested on an eligible device.** The iPhone 14 Pro reports
  `deviceNotEligible`; a 15 Pro or later, or a Simulator, is needed.
- **Semantic graph coverage is unmeasured.** `tools/graphify.sh` defaults to authenticated
  oMLX, with a code-only fallback; the retired DeepSeek model is no longer the configured
  blocker. Probe: run `tools/graphify.sh --check`, then a rebuild, and inspect its reported
  non-AST elements/hyperedges before claiming semantic coverage. `tools/self generate` needs
  no graph. What the code-only fallback loses remains unmeasured.
- **From the old PRD, not yet planned.** Device loss as a threat (what a stolen phone exposes).
  A health domain (energy, sleep, training, records). Item intake: **the link half is built**
  (F12, 2026-10-05) — what remains is *scan an object*, which needs the Q63 ladder to end at
  something better than text, and a barcode path, which is `[tauri-plugin-barcode-scanner]`
  today and reads the Mac's pairing QR code. Travel: route composition, "where should I base
  myself", an accommodation source. People: Google Contacts beyond inbound, a TELOS import,
  trips from a person, meetup capture. An autonomy gate for a structured-decision model (apply
  at ≥ 0.95 confidence, otherwise a card). shellcheck as shell analysis. The PRD's non-goals, which conflict with Sjel ("not a
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
- **Habits in the planning owner.** Projects and tasks are specified in F14. Habits are not:
  which `calendar` rhythm fields carry a habit, and whether the daily metrics in
  `Journal/01. Daily Notes` (read by vault `Habits.base`) move or stay as journal entries.
  Vault `Domains/Journal.md` says nothing is written back into a journal entry.


## Decisions

These bind open work. Git retains earlier reasoning, measurements, overrides and superseded
alternatives.

- 2026-10-15 / 2026-10-11: GPUI is the goal for all clients. Step 0 uses the Longbridge
  `gpui-pre-mobile` fork and one GPUI snapshot (`gpui-pre =0.3.9`, gpui-kit 0.7.3). OS chrome
  belongs to AppKit/UIKit; content belongs to GPUI. See `upstreams.toml`, gpui-kit's
  `website/docs/mobile.md` and `apps/gpui-prototype/src/macos_glass.rs`.
  0. One Cargo workspace builds GPUI, gpui-kit and gpui-mobile; show one loopback-HTTP-fed list
     on Mac and iOS Simulator. Failure reopens the iPhone decision.
  1. Build the command palette; measure start time and resident memory against Tauri.
  2. Port the dashboard's routes into an isolated app: `context`, `calendar`, `inbox` first,
     native `capabilities` as reference. GPUI draws charts; map last, `gpui-webview` fallback.
     2026-10-18 editor spike (`1e0300ac`..`f7d4270b`): block editing, inline formatting, `/`
     menu, drag reorder and a task table all work without a fork. 1.6 ms CPU per keystroke on a
     1,740-line note. GPUI gaps had outside workarounds: no inline styling outside code-editor
     mode, fixed textarea padding, key-context precedence.
  3. Test and review; decide switch, both or stop. Svelte/Tauri stay live through that review.
  This replaces the narrower 2026-10-09 keyboard-only split. It is a stack decision; DSH-8
  did not identify rendering as the wait. Do not retire Pi before one household task and one
  coding task beat the existing path.
- 2026-10-19: A client reads from its own replica and never waits on the node to paint. The
  replica is the read source for the paths in `OFFLINE_PROJECTION_PATHS` and `C1_PREFIXES`
  (`dashboard/src-tauri/src/sync.rs:43`), online or not. Every other path stays network-first.
  The replica stores rows, not answers: today's snapshot keys each answer by its exact
  query, so a new calendar range misses it. Sync, not the page, talks to the node, so a node
  on another machine changes freshness and not paint time. Writes keep the outbox. The replica
  and sync belong in a crate that the Tauri bridge and `apps/gpui-prototype/core` both use, so
  the work survives the GPUI move. Calendar goes first (DSH-9).
- 2026-10-17 / 2026-10-09: Sjel owns projects, tasks and habits. Obsidian keeps Knowledge,
  Atlas, Journal, Reflections. This reverses Q48. Capability `projects` stores projects, tasks
  and project notes as tables in the shared store. Every write re-renders the record as
  markdown under `<overlay>/data/projects/`, which keeps Principle 8 the way finance decisions
  do. Nobody edits the rendered files; Sjel's CLI and API are the only writers. The import
  takes every markdown note and every attachment of a vault project folder, and nothing from the
  law folder `Sjel/Knowledge-Base/`. Attachments stay files in the rendered tree, not rows
  (2026-10-18: 986 MB would grow every backup of the shared database). The frozen vault notes
  moved to vault `Archive/Projects/` as the backup until the owner is final. The page that
  listed apps under `/projects` moved to `/apps`; `/projects` is this capability. The import ran
  2026-10-18, which ends Q48. The 2026-10-17 part replaces markdown-canonical storage after
  the operator compared both.
- 2026-10-05: garments share `interior_item`, with `category`, `groesse`, `farbe`, `saison`.
  Both URL intake and create/edit forms are required. A garment is `kind = piece`; `gear`
  means portable packing equipment. Keep the `interior` name. Read category suggestions from
  rows; report spelling collisions without merging. Fetch URLs only in the CLI. Refuse guessed
  and non-EUR prices; missing prices stay explicit because totals feed finance (B29).
- 2026-10-01: `sjel update` reports all software with its owner; apply delegates owned classes
  and moves only the rest. Systems gets apply, under operator/session/origin guards and
  `APPLY_CLASSES`; sjel-status admits no agent token (ISC-47). Cargo/npm stay unscheduled.
- 2026-09-30: agent access starts discovered, authenticated and read-only, with explicit
  per-capability write grants and the user's secret store. The gate pseudonymizes reads;
  reversible session tokens permit pseudonymized c2, never c3 (amends Q27).
- 2026-09-30: rename `axon`, headers, prose and the FDA launcher. Accept `X-Axon-*` until
  the phone ships; restore FDA after renaming (ISC-29). The `AXON_*` fallback is retired.
- 2026-09-30: the dashboard exposes the disk's own measurements from `tools/storage`, under R6.
