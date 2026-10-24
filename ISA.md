---
project: sjel
type: isa
phase: climbing
progress: 89
principal_stated_goal: "Track concrete work in GitHub Issues; retire ISA entries and dated decision prose incrementally as code, tests, and owner docs become the source of truth."
---

# ISA · Sjel

Use GitHub Issues for concrete work a maintainer or contributor can test or complete. The
existing ISA is a transition backlog for unresolved claims with falsifiable probes, not ordinary
tasks. Retire entries incrementally as their truth moves into code, tests, or owner docs; Git
keeps history.

Decisions live with their owner ([rule](CONTRIBUTING.md#decisions-live-with-their-owner)), never
here. Verification rules live in the `sjel` skill's `on-verification.md`.

Issues remain open to outside reports. Temporary plans may guide active work, but do not create a
second standing backlog. See [planning rules](CONTRIBUTING.md#issues-and-the-isa-transition).

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
  kind of note Media's sources were. Then the vault bases and MOCs that showed those folders go, and the vault rules shrink into the `obsidian` skill. Falsifier: the
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

### documents · seven open claims

- **DOC-1 — external backup and restore.** `documents` stores originals locally and declares a backup path; metadata remains in the shared store's archive. Open until both archives are verified and a restore rehearsal proves a document's bytes and metadata return together from the selected external SSD. Falsifier: a document is readable after restoring only one half, or the selected target is not the same SSD used for media/photos.
- **DOC-2 — evaluated inbox enrichment and source receipts.** Folder intake, polling, and the operator UI/API exist; source identities, mail attachment intake, source-linked insights, and class-gated model enrichment do not. Follow the pipeline in `capabilities/documents/README.md`. Close with a frozen German/English corpus, declared acceptance criteria, recorded results, and an idempotent mail replay. Falsifiers: an insight has no supporting source span, a retry creates a duplicate source receipt, a `c2` document reaches cloud inference, or `c3` content enters any prompt.
- **DOC-3 — mixed-scan segmentation.** Evaluate proposed document boundaries separately from PDF page extraction. Preserve the parent original, child page ranges, and complete page coverage. Close with annotated German/English multi-document scans, declared boundary acceptance criteria, and a verified reconstruction. Falsifiers: a page is lost or duplicated, a boundary lacks inspectable evidence, or a child cannot be traced to its original pages.
- **DOC-4 — live operator reader.** The document API, dashboard collection, filing controls, and PDF/text reader compile and pass focused checks; browser rendering is unverified. Close with an authenticated browser exercising upload/replay, search/filter, filing, archive/restore, and original PDF viewing at desktop and phone widths. Verify original hashes after mutations. Falsifiers: the page shows counts that disagree with the library, an original cannot be viewed or downloaded, a narrow viewport clips controls, or an unauthenticated/agent request returns document contents.
- **DOC-5 — the native reader replaces the dashboard page.** GPUI renders a PDF's pages from the hash-verified original, compares two documents side by side, and files a record under an operator-typed category and project. Decisions and phase order are in `capabilities/documents/README.md` (2026-10-22). Close when a real multi-page PDF renders at fit width on the Mac and on the iPhone with the Mac unreachable, and filing from the app makes the dashboard page unnecessary. Falsifiers: a page renders blank or shows the wrong page, a count disagrees with `GET /api/documents`, or filing still requires the dashboard.
- **DOC-6 — duplicate and split decisions are recorded and durable.** `src/duplicates.rs` owns the report; this side records a survivor with absorbed records and source paths, a dismissal keyed so it does not return, and a split whose children hold the parent hash and page ranges. Close when a merge, a dismissal and a split each survive a restart and `restore-index`, every source path is retained, and every input page is accounted for. Falsifiers: a dismissed pair returns after restart, a merge loses a name or source path, or a split loses or duplicates a page.
- **DOC-7 — proposals and summaries are reviewable, not asserted.** Rule and model proposals are separate records with evidence page, engine identity, revision and confidence; a summary records what produced it. Nothing files automatically and no proposal overwrites an operator value. Close when the review UI shows agreement and conflict for a synthetic corpus and generation runs only against a loopback role. Falsifiers: a proposal renders without an evidence span, an operator-edited field is replaced by a later proposal, or any `c2` text leaves the host.

### import · two open claims

- **IMP-1 — every route has an owner that accepts it.** `capabilities/import` sorts a folder and hands documents over today. Media, finance and notes files are recorded `pending` because those capabilities have no import contract. Close when each route has a write contract and a run over a real mixed folder leaves no `pending` row. Falsifiers: a file is handed to two owners, or a rerun stores the same bytes twice.
- **IMP-2 — the operator imports from the native app.** The GPUI Import page runs plan, run and status against a chosen folder and shows the outcome per route. The page and API compile and pass focused checks; an authenticated run from the app is unverified. Close when an import of a real folder from the app matches `sjel-import status`. Falsifiers: the app and the CLI count the same folder differently.

### traveler · one open item

- **F4 — the offered-versus-chosen half.** Twenty-eight `option_set` items exist on one plan; one
  plan is one observation, and a preference inferred from a single choice is a guess. (`F3` vault
  proposals shipped on `GET /api/profile/proposals` with section gating; `F6` pseudonymous
  companion patterns shipped in `GET /api/profile/derived` keyed by `ent:` register IDs.)

### Pack retirement work · one open item

The travel Pack left this repository on 2026-10-24 (with home-automation, 3d-printing and cv); its
runbook now lives in `docs/capabilities/travel.md`. The open item it carried travels unchanged:

- **An open segment has no home in the plan model.** A stage names a destination and date, an
  item a day, and status is draft/saved/archived — none says "this part is undecided, here are
  the branches". What would settle it: a second trip planned this way.

### settings · one claim

- **SET-1 — every inference model and key this machine depends on is named by one surface that can
  prove it.** The truth is currently spread across `inference.json` (backends and roles), the
  overlay's `config/graphify.env` (the tool-level selection no role names), and key files that no
  verb reports at all — `sjel credentials` covers only the two Google OAuth grants in the keychain.
  Close when one authenticated read names every backend with its model and its key state, every
  role with the backend and model behind it, and the tool-level selection, and a probe proves each
  named model answers. Falsifier: a model or key file named by a tracked artifact that no surface
  reports, or the surface's verdict for one backend disagreeing with `tools/model-check`'s.

### distribution

- **DIST-1 to DIST-4 — a fresh-user Mac and iPhone install.** Tracked in
  [#235](https://github.com/larsboes/Sjel/issues/235) since 2026-10-20, for another contributor.
