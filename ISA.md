---
project: sjel
type: isa
phase: climbing
progress: 75
principal_stated_goal: "I want no new issues, I wanna get rid of all issues for sjel and sjel personal and only carry through normal ISAs etc."
---

# ISA · Sjel

Repo-wide state of record. Open work that belongs to one capability or Pack lives in
that owner's own `ISA.md` (`Packs/travel/ISA.md`, `capabilities/places/ISA.md`); this
file holds what is repo-wide or has no other owner.

## Problem

The backlog lived in GitHub Issues, and a tracker is a second system of record that
never says what *done* means. An issue carries a description and a lifecycle; it does
not carry a falsifier, so nothing about closing one proves anything was verified. The
ISA already is the artifact that states done as testable claims, and running both means
maintaining two surfaces that disagree the moment either is edited.

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

Zero open issues in this repository and in the private overlay, every live item they
carried standing as a claim in an ISA, and no automation able to open a new one.

## Features

### F0 · The tracker stops being the backlog

Why: two systems of record is the actual defect; closing the issues without moving the
doctrine and the automation would refill the tracker by Monday.

- [x] ISC-1 — `gh issue list --state open` returns nothing in either repo. Evidence: 0 and 0, 2026-08-20. Falsifier:
  any open issue in this repository or the private overlay.
- [x] ISC-2 — every closed issue's live content stands as a claim or a Not-yet-specified
  entry in an ISA, and its closing comment names the file. Falsifier: a closing comment
  with no destination, or a destination that does not contain the content.
- [x] ISC-3 — the doctrine agrees with itself: `README.md`, `CONTRIBUTING.md`,
  `AGENTS.md` and `tools/doctor.ts` all name ISAs as the backlog. Falsifier:
  `rg -i "backlog is Issues|GitHub Issue"` still describes the backlog anywhere in those
  four files.
- [x] ISC-4 — `upstream-watch` opens no issue: it writes the drift report to the job
  summary and exits non-zero when an entry is past its cooldown. Falsifier: the workflow
  file still calls `gh issue create`, or a green run hides real drift.
- [x] ISC-5 — `bazel test //...` green on the landing commit, `ARCHITECTURE.md` included. Evidence: 68/68 pass, plus `//:architecture_up_to_date_test` forced uncached, 4050fe4.
  Falsifier: any failing target.

### F1 · Demo seeding for Comms, Scouting and Transit

Why: three capabilities are missing from the published demo, so Feed, Scout and Travel
are hidden and the shell looks thinner than the system is.

- [x] ISC-6 — the demo shows Comms, Scouting and Transit, every recorded value having come
  from a real server answering a real request, not a hand-written fixture. Evidence, one
  full `tools/demo-up all` on 2026-08-20: `seeded comms: 7 items ingested from the demo
  origin (3 kept, 1 dismissed)` · `seeded scouting: 7 opportunities discovered through the
  rss adapter (7 persisted)` · `recorded transit: 2 paths` · 41 fixtures, then
  `tools/demo-site` wrote 52 reference pages and the hygiene gate passed over 251 files.
  Spot-checked: comms' titles are its extractor's output from the served HTML, scouting's
  rows carry `source: rss:demo-origin`, transit's journey is the origin's ICE 331 with
  `reliability: null` because punctuality is absent — the declared degradation.
- [x] ISC-7 — transit's endpoints are env-overridable, so the demo can point the real parser
  at a stub. Three, not the two this claim named: `SJEL_TRANSIT_DBNAV_FAHRPLAN_URL` had to
  join them once dbnav became the default, or the default backend would have been the one
  path a stub cannot reach. Evidence: `every_endpoint_can_be_pointed_at_a_stub_and_otherwise_is_bahn_de`,
  plus a live CLI search against `tools/demo-origin` returning three parsed journeys on both
  backends.

Correction carried over: `demo.toml`'s `[absent.comms]` blames `sources/rss.rs`, which
is *scouting's* file, not Comms'. That reason was written from a bad reading of an `ls`
whose error was silenced.

### F2 · Upstream drift

Why: an upstream that moves and a deployment that does not is the drift worth watching. Q77
(2026-09-02) reversed the answer rather than the question: nothing is held at a version any more,
so the risk is no longer "the bump is late" but "the bump landed and broke something", and what
has to be visible is the pull request, the alert and the receipt. Q109 (2026-09-22) adds one
narrow age window rather than a version pin: Bun/npm resolutions wait 24 hours, the two UI trees
run Socket's scanner, and Cargo/actions keep the zero-day path.

- [x] ISC-8 — every entry a Dependabot pull request or alert names as behind or vulnerable
  is either merged or has a written reason it is held. Falsifier: an open Dependabot pull
  request older than a week with neither. (2026-08-19, measured by the since-retired
  `tools/upstream-checker`: 76 entries · 50 ok · 17 n/a · 9 warn · 0 fail, every warn inside
  its cooldown hold, where waiting was the action. 2026-08-28, PRD Q41 deleted that checker
  and named `renovate.json5` its replacement; the Renovate GitHub App was never installed, so
  the claim then ran for five days with no instrument at all — measured 2026-09-02, the
  repository has zero Renovate pull requests and zero Renovate issues over its whole life.
  2026-09-02, Q74: `renovate.json5` is deleted and `.github/dependabot.yml` replaces it.
  Dependabot needs no App, so the claim has an instrument for the first time — and the hold
  half is gone with the cooldown, so "held with a reason" now means a deliberate refusal,
  never a timer. 2026-09-02, Q77: Dependabot pull requests carry `--auto --squash`
  (`.github/workflows/dependabot-automerge.yml`), so the ordinary bump merges itself once the
  required checks pass and this claim is about the exceptions only. 2026-09-22, Q109: the
  two Bun blocks carry `default-days: 1`, while Cargo and github-actions remain at 0; each
  resolving UI tree carries `minimumReleaseAge = 86400` and
  `@socketsecurity/bun-security-scanner`, and CI runs `bun pm scan`. The scanner is free-mode
  network-backed and therefore an operational dependency, not a replacement for the hold.
  Done 2026-09-30: all 11 queued pull requests merged after `self.json` schema 2 and undici 8.10.2
  greened `main` (d49fd70e, 016d9251, eac7c93b, d25a7353, 8d43b981, 0451bc7a; 0 open PRs). The
  single open Dependabot security alert (#1, glib 0.18.5) was dismissed on 2026-09-29 with the
  documented `osv-scanner.toml` rationale: Linux-only via `gtk 0.18 <- muda/tao <- tauri`, not compiled
  on macOS/iOS and unbuilt in CI. Evidence: `gh pr list` returns 0; `gh api repos/larsboes/Sjel/dependabot/alerts`
  reports 0 open alerts.)
- [x] ISC-9 — the postgres 17.9 → 17.10 image decision is made on its own, not ridden
  along with another change. Falsifier: the bump appears in a commit about something else.
  Closed 2026-08-27 by PRD Q45, which retired the image rather than bumping it: the running
  17.9 instance is read once by `tools/migrate-pg-to-sqlite` and then stopped, so a 17.10
  decision has no subject. The constraint held either way — the retirement is its own
  commit, and the version that ran is in `upstreams.toml`'s git history at that date (Q77
  deleted the field on 2026-09-02).
- [x] ISC-26 — `tools/self check` passes in a fresh clone and `tools/self generate` succeeds on a
  machine with no code graph, so the gate CI runs is a gate CI can repair. Falsifier: a
  `git worktree` of origin/main exits 1 from `tools/self check`, or
  `SJEL_SELF_GRAPH=<a path that does not exist> tools/self generate` exits 1. Done 2026-09-29:
  the per-unit `code` counts and the `graph` block were committed but rolled up from git-ignored
  `graphify-out/`, so on a graphless machine `check` narrowed its comparison to the tracked-file
  layers while `generate` refused to write — the drift it reported was the one drift nobody there
  could fix, and `main` was red on CI's `bun test`, `repo gates` and Pages' `build the page`
  behind it while ten armed Dependabot pull requests older than a week queued (ISC-8). Both
  layers are fused on read now: `self.json` schema 2, `status` and `explain` still show the counts
  where a graph exists, and `tools/generate-site.ts` already refused to publish them. Evidence:
  in a worktree of origin/main, `tools/self check` exits 0 and `tools/self.test.sh` passes every
  case; with `SJEL_SELF_GRAPH` pointing at nothing, `generate` writes schema 2 carrying no `code`
  and no `graph` key.

Order, agreed 2026-09-26: F3, then F4 in the order of its claims, then F5.

### F3 · Axon becomes Sjel, staged

Why: the product is named Sjel since 2026-09-26, and every day adds more "axon" to
commits, URLs and identifiers. Measured the same day: 757 tracked files and 5,884 mentions,
138 distinct `AXON_*` variables, 19 `axon-*` crates, 15 `com.axon.*` launchd services on this
Mac, 104 overlay files, the `axon` skill, and the iPhone app still named "LifeOS" with bundle
identifier `com.lifeos.mobile`. Lars chose "everything, staged" over a brand-only rename.

- [x] ISC-10 — what people see says Sjel: the GitHub repository, the README title, the app's
  `productName` and the demo site URL. Falsifier: `gh repo view --json name` is not `Sjel`, or
  `dashboard/src-tauri/tauri.conf.json` still says `LifeOS`. Evidence, 2026-09-26: repository
  `larsboes/Sjel` (6be1ea8), app `Sjel.app`, README title Sjel, `larsboes.github.io/Sjel/`
  answers 200 and `/Axon/` 404.
- [x] ISC-11 — a `sjel` command runs every `axon` subcommand, and `axon` keeps working as an
  alias. Falsifier: `sjel help` fails, or an existing `axon` call in a tool or skill breaks.
  Evidence, 2026-09-26: `axon` is a tracked symlink to `sjel` (902a7a7); both answer `help`,
  the capability-probe, runargs, bootstrap, persistence and doctor tests pass.
  **Superseded 2026-09-30 by ISC-27.** The alias was a migration aid, not a permanent surface.
  Its own falsifier — "an existing `axon` call in a tool or skill breaks" — is what ISC-27 now
  pays off deliberately, in one commit, rather than leaving it to be discovered later.
- [x] ISC-12 — the iPhone app has a Sjel bundle identifier and the phone is paired again.
  Changing the identifier makes iOS treat it as a new app: its offline copy and its Keychain
  key are gone. Falsifier: `com.lifeos.mobile` remains in `tauri.conf.json`, or the new app
  fails `devices/api/devices/me`. Done in code (725e70f: `com.larsboes.sjel`, the IPA signs).
  Done 2026-09-27 21:38: paired by scanning the Mac's QR code (20ef0075, 7a9595d3); the node
  lists the iPhone `active` and saw its first signed request 16 s later.
- [x] ISC-13 — internal names move with a fallback: crates, `AXON_*` variables (the old name
  still read), launchd labels and the overlay. Falsifier: a service that ran before the change
  does not start after it, or a variable is renamed with no fallback reader. Progress,
  2026-09-26: the skill is `sjel` (ec52c11c); launchd labels are `com.sjel.*` with the old units
  removed (0479a731, 15 of 15 moved); settings are `SJEL_*`, read with the `AXON_*` fallback in
  Rust, shell and TypeScript (b2a56912), and the runner hands a service both
  names (a25702c7). Done 2026-09-27: the overlay's keys, the nine library crates are `sjel-*`
  (c36a237d), and the private overlay repository and folder carry the Sjel name, with the old
  folder name kept as a symlink. Deliberately unchanged: the `sjel-status` capability (its name is
  the shell's URL mount the phone calls), `axon-fda-launcher` (renaming drops its Full Disk Access
  grant), Linux systemd unit names, the `X-Axon-*` request headers, and "Axon" in prose. Each of
  those is a separate decision, recorded under Not yet specified.
  **Amended 2026-09-30:** the `AXON_*` fallback this criterion describes was retired rather
  than kept, so the claim above no longer holds — settings are read under `SJEL_*` alone
  (9faf670a). Four implementations went: `tools/lib/env-compat.sh`, the force-export loop in
  `tools/lib/paths.sh`, the read fallback in `libs/sjel-config/src/env.rs`, and another in
  `tools/lib/env.ts`. Nothing read the old name afterwards, so the five test helpers that
  cleared both names now clear one, and `doctor.ts` lost its `AXON_TAILNET_OPERATOR` fallback.
  The one real dependency was the private overlay's machine-local shell config, which located
  itself by `AXON_PERSONAL_ROOT` and would have silently stopped sourcing `machine.zsh` and
  `secrets.zsh`; it now exports `SJEL_HOME_ROOT`, the name the home-automation skills read (35
  references, none under the old name). The three surfaces named above stay open under F7.

### F4 · The documents a stranger reads

Why: the top of the README is the product definition, and the sources behind the design go into one
curated place. The README is still 870 lines of doctrine below 230 of product.

- [x] ISC-14 — the README opens like a large open-source project (Graphify, Ollama): logo,
  badges, a screenshot of phone and dashboard showing travel and people first (the areas a
  stranger meets first), what it does, a three-command start, and a
  table of measured results (pseudonymizer 48/48, redaction recall 100% on the frozen corpus,
  Feed ranking 0.941 pairwise, the 1.5 s Same Wi-Fi fallback). Falsifier: a number in that
  table without a command or file that reproduces it. Done 2026-09-27: text wordmark (no logo yet), badges, the
  travel hub on desktop and phone from the live demo, What it does, Quick start, and a Measured
  table whose redaction rows were re-run that day. The 1.5 s fallback left the table: it is a
  setting, not a measurement.
- [x] ISC-15 — `research/` states why the project exists: the problem it answers and the
  sources that show the problem is real. It grows over time; the first version holds at least
  one sourced entry, and the demo site shows it. Falsifier: an entry without a source, or a
  claim its source does not support. Done 2026-09-27: `research/why-sjel.md` with four sourced
  claims and a list of what the sources do not show; `tools/generate-research.ts` renders it at
  `/research` on the demo site. Moved 2026-09-30: the dashboard renders `research/` as its own
  `/research` routes (`dashboard/src/routes/research/`), with a Projects view built from
  `systems.toml` and `upstreams.toml`, and `tools/generate-research.ts` is deleted.
- [x] ISC-16 — the engineering doctrine lives in `CONTRIBUTING.md`, and no link points at a
  README anchor that no longer exists. About 208 links point into it today. Falsifier:
  `git grep "README.md#"` finds an anchor missing from `README.md`. Done 2026-09-27
  (b88ad6d6): 124 files now cite `CONTRIBUTING.md#<section>`. Five older citations of deleted headings now
  point at the sections that replaced them. `upstreams.toml` keeps `pins-and-cooldown` on purpose:
  those strings record adoptions decided under that rule.

### F5 · The session's work, proven on real devices

Why: the transports and the model ladder shipped on 2026-09-25 with tests, but nothing ran on
the phone. Each item below is built and unverified, or ruled and unbuilt.

- [x] ISC-17 — a phone on the home Wi-Fi reaches the Mac's `:8443` listener, pins it after the
  code comparison, and reads data with the tailnet off. The phone is registered (ISC-12) and pinned the
  Mac from the QR code; Lars confirmed on 2026-09-27 that the app reads
  over the home Wi-Fi. The firewall permits `sjel-status` (checked 21:33). Falsifier: `curl -k
  https://<LAN address>:8443/health` from another device does not answer 200.
- [x] ISC-18 — the assistant drawer calls the model ladder (`dashboard/src/lib/intelligence`)
  for at least one task and shows which rung answered. Falsifier: `rg "intelligence/backends"
  dashboard/src` finds no caller outside the module and its test. Done 2026-09-29: `assistant-engine.ts`
  calls `generate` down the ladder for unrouted queries and feed interpretation, and `AssistantDrawer.svelte`
  renders the answering rung badge (`on-device`, `mac`, `rules`).
- [x] ISC-19 — native Mac companion and CloudKit relay in `apps/mac` host the encrypted sync
  service satisfying Product Rule 4 (Approach A). A test reads a record back as Apple stores it and finds
  ciphertext ([product rule 4](CONTRIBUTING.md#product-rules)). Falsifier: a field name or value of a C2 record readable in the stored record. Done 2026-09-29:
  `apps/mac` hosts `SjelRelay` (AES-256-GCM authenticated encryption envelope, `RelayKeyManager`, `CloudKitRelay`) and
  menu bar companion `SjelMacApp`; verified by `c2RecordInCloudKitStorageHasZeroReadableFieldsOrValues` in `CloudKitRelayTests`
  confirming `CKRecord.allKeys()` and values carry zero C2 field names or plaintext values.
- [x] ISC-20 — the comms review queue sends pseudonymized jobs through `prepare_pseudonymized`
  ([product rule 4](CONTRIBUTING.md#product-rules)). Falsifier: a queued job whose payload carries a raw C2 entity. Done 2026-09-29:
  preview, approval, and queue handlers in `server/cloud.rs`, `attach_cloud_state` in `server/contracts.rs`, and `enqueue_digest_job`
  and `stage_and_queue` in `cloud_run.rs` route via `prepare_pseudonymized` with `people_registry::entity_registry()`,
  verified by `queued_review_job_is_pseudonymized_and_never_leaks_c2_entities`.
- [ ] ISC-21 — the family deployment runs for a week without Lars touching it.
  Falsifier: any fix to it made by Lars in that week.

### F6 · What the old PRD still owes

Why: the vault PRD (`Projects/Axon/PRD Axon.md`) is archived and stays so, because 528 citations
in 260 files point into it. A read on 2026-09-27 found about 90% of it settled decision log. The
rest is below or under Not yet specified; nothing new goes into the PRD.

- [x] ISC-22 — the three success criteria are measured again: no raw data about other people in
  the egress log, a trip planned in under 30 minutes, and one surface in place of five apps.
  Falsifier: any of the three has no command or log query that shows its current value. Done 2026-09-29:
  1. Egress privacy: `comms egress-log --audit` reports 0 raw C2 violations (verified by
     `egress_log_records_outbound_model_calls_and_audit_verifies_c2_absence`).
  2. Trip planning: `trips draft-intent "weekend in Paris under 200 euro by train"` produces structured
     plans in < 1 s (under the 30-minute threshold; search engine constrained to 30 s deadline).
  3. One surface: `dashboard/src/routes` unifies 5 apps (calendar, feed/comms, finance, travel/trips,
     interior) on one surface consuming `content-item-v2`, served at `http://127.0.0.1:8082`.
- [x] ISC-23 — every outbound model call appears in the egress log with its token count and cost.
  Falsifier: a call site that reaches a cloud model without writing a log row. Done 2026-09-29:
  `{prefix}_egress_log` table with token counts and cost calculation on OpenAI-compatible `usage`;
  persisted on all outbound calls in `cloud_run::perform`; surfaced via `comms egress-log` CLI
  and verified by `egress_log_records_outbound_model_calls_and_audit_verifies_c2_absence`.
- [x] ISC-24 — a reviewed provider list (`providers.toml`: provider, highest data class, review
  date, expiry after 12 months) gates cloud calls, or the ruling is withdrawn. Falsifier: a cloud
  call to a provider the list does not name. Done 2026-09-29: `providers.toml` root declaration
  with `libs/inference/src/providers.rs` (`ReviewedProvidersList`, `check_admission`, 12-month expiry
  with leap-year handling); cloud queue and dispatch gated in `server/cloud.rs`, `cloud_run.rs` and
  `cloud_dispatch.rs`; falsifiers verified by `cloud_call_to_unreviewed_provider_is_refused`,
  `cloud_call_to_expired_provider_is_refused`, and `cloud_call_exceeding_data_class_is_refused`.
- [x] ISC-25 — the product rules answer the counter-evidence in `research/`: a cloud request
  carries only the fields its task needs (Staab et al.), and confirmations are rare enough to be
  read (Akhawe and Felt), with the prompt rate measured. Falsifier: rule 4 or 5 unchanged with
  no measurement that answers the source. Done 2026-09-29: Product Rule 4 in `CONTRIBUTING.md`
  answers Staab et al. via strict task-scoped field minimization (verified by
  `cloud_derivative_carries_only_fields_task_needs_answering_staab_et_al`); Product Rule 5 answers
  Akhawe & Felt by reserving prompts exclusively for irreversible/off-host actions with routine
  prompt rate measured at 0.0% (verified by
  `autonomous_processing_has_zero_prompt_rate_answering_akhawe_and_felt`); both documented in
  `research/cloud-models-and-privacy.md` and `research/agent-safety.md`.

### F7 · What fills the disk is visible where it is managed

Why: `tools/storage` already measures this. Its four verbs are `report`, `apply`, `target`
and `prune`; the first three emit `--json`, and it enforces PRD §9's R6, ratified as Q53 on
2026-08-28: build artifacts are not state, and `target/debug` may not exceed `target/release`
by more than 3×. What is missing is any surface a person looks at.

Measured 2026-09-30: this machine held 400 GB of 460 GB with 34 GB free, and nothing in the
dashboard said so. The largest item was invisible to the tool that exists to find it:
`dashboard/src-tauri/target` had reached 15 GB because Xcode's build phase launches `cargo`
without the shell environment, so `CARGO_TARGET_DIR` is unset and cargo falls back to the
local target dir. `prune --target` runs `cargo clean`, which honours that same variable, so it
could only ever clean the one directory. The user-facing gap and the tool's blind spot are the
same gap, which is why they are one feature.

Placement, per `Packs/harness/skills/sjel/references/on-placement.md`: the measurement stays
in `tools/` ("Repository identity, install, wiring, or operator machinery"), because storage
is operator machinery rather than a bounded domain. No new capability and no new route —
`capabilities/sjel-status` already serves a host-watch finding for "a filling disk", and
`dashboard/src/routes/systems/+page.svelte` is already the machine-state page. The
change is two routes in its `service.toml`, one handler, and one panel.

A machine-wide `[build] target-dir` in `~/.cargo/config.toml` was tried on 2026-09-30 as the
quick fix for the recurrence and reverted the same day. It does stop Xcode recreating the
nested tree, but it also redirects *every* cargo build on the machine, including the
checkout's own — so a workspace build would land in `~/.cargo-target` while the supervisor kept
running, and restarting, the stale binaries in `<repo>/target`. The deployment directory and
the build output would have quietly diverged. Visibility in the tool is the fix instead,
because it names the directory rather than moving where things are written.

- [x] ISC-27 — the `axon` symlink is gone and `sjel` is the only entry point. Falsifier: `axon`
  still resolves on PATH or at the repo root, or a tool or skill still invokes it. Probe:
  `command -v axon`, and `rg '$AXON|"axon"' tools/ Packs/`. Supersedes ISC-11. Evidence,
  2026-09-30: the tracked symlink and `~/.local/bin/axon` are removed, `command -v axon` returns
  nothing, and 65 `axon <verb>` invocations across 25 files now say `sjel` (this commit). The
  removal also exposed a live break: `profiles.toml` still named the old skill,
  `skills = { "harness" = ["axon"] }`, and the harness deployer refuses a profile that names a
  skill its Pack lacks — so `productive` and `coding` could not be activated at all. Fixed;
  both activate. Still named `axon`, each its own decision: `tools/axon-context`,
  `capabilities/comms/axon-clip/`, `dashboard/src/lib/home/AxonGlance.svelte`,
  `schemas/axon-sync.schema.json`, `Packs/harness/codex/axon/` (a Codex agent),
  `axon.toml`/`axon.local.toml`, the `~/.local/state/axon/` deployment ledger,
  `~/.local/bin/axon-eject` (the private overlay's tool), and `AXON_`-looking fixture names in
  tests.
- [ ] ISC-28 — the signed-request headers are `X-Sjel-*`, with the old name still accepted
  until the paired phone ships. Falsifier: a paired phone's signed request fails after the
  rename. Probe: `libs/sjel-server/src/auth.rs` accepts both, asserted in its own tests.
- [ ] ISC-29 — `axon-fda-launcher` is renamed, and its Full Disk Access grant is intact
  afterwards. Falsifier: the binary runs without FDA and cannot read what it needs. Probe: run
  it and confirm the grant. Renaming drops the grant, so re-granting is part of the work.
- [x] ISC-30 — "Axon" no longer appears in prose and doctrine. Falsifier: `rg -i axon` over the
  tracked documents returns hits that are not historical record (a commit message, an entry in
  this file), a platform-pinned identifier, or a deliberate alias. Probe: `rg -ci axon` over
  `README.md ISA.md CONTRIBUTING.md ARCHITECTURE.md`. Evidence, 2026-09-30 (c68e7fb6): 284
  occurrences rewritten across 100 files, of which `README.md` and `CONTRIBUTING.md` were
  already clean. 22 remain, each one of: historical record (`Axon held a reviewed delta against
  it until 2026-08-25`, `Axon#126`, `Projects/Axon/PRD Axon.md`), the transition's own name
  (F3 and the `/Axon/` URL that 404s as its evidence), an identifier rather than prose
  (`axon-*`, `AXON_*`, `X-Axon-*`), or this ISC's own text.
- [x] ISC-31 — the Systems page shows what fills the disk, from the tool's own `--json`.
  Falsifier: the page renders no storage panel, or `sjel storage report --json` exits non-zero.
  Probe: `sjel storage report --json` and the page. Evidence, 2026-09-30: the report exits 0 and
  `GET /api/sjel-status/storage` returns it unchanged, a non-zero exit carried as data because
  that is how the critical state arrives. The panel rendered in Safari at 1512×868 through the
  dev server: state OK, `326.0 GB used of 460.4 GB`, `107.8 GB free`, the measured classes
  largest-first (`rust-workspace-target` 1.9 GB, report-only; `package-manager-caches` 430 MB),
  `430 MB reclaimable — sjel storage apply`, and six protected paths each carrying the tool's own
  reason plus `4 of these could not be read by the measuring user`. Every figure equals the
  tool's own text and JSON, and `used` moved between two loads (322.8 GB, then 326.0 GB), so the
  panel reads the disk rather than a fixture. It is read-only by decision.
- [x] ISC-32 — `tools/storage` sees a Cargo target dir that is not `CARGO_TARGET_DIR`.
  Falsifier: after an Xcode-driven build, `sjel storage target` reports one path while a second
  target dir exists on disk. Probe: build via Xcode, then `sjel storage target`. The checkout's
  own `target/` is excluded from the answer on purpose: it holds the binaries the supervisor
  runs, and `tools/cargo-hermetic` refuses to point `CARGO_TARGET_DIR` inside the checkout for
  that reason. Only a *nested* workspace — `dashboard/src-tauri/target` — is an escapee.
  Evidence, 2026-10-01: built in b8966e17 with three unit tests. Live probe against this
  checkout: a 5 MB `dashboard/src-tauri/target` stamped with cargo's own `CACHEDIR.TAG`, then
  `sjel storage target` printed `5 MB in 1 target dir cargo does not resolve to here` with the
  `cargo clean --manifest-path` that removes it, and `--json` carried it under `secondary`. The
  directory was planted, not produced by an Xcode build, so the falsifier's own setup is still
  unexercised.

### F8 · An agent reaches what Sjel holds, and only what it should

Why: the data and tools exist (33 registered capabilities), but a local assistant cannot find
or call them without the operator's hands. Measured 2026-09-30, asking "what is the latest mail
from Ollama": `sjel search mail` returned no capability although `capabilities/comms` sweeps
the inbox; `sjel capability call comms get /triage` returned `invalid or missing authentication
token`, because `capability_call` in `sjel` sent no credential at all. Three gaps, one
feature: an agent cannot find a capability, cannot authenticate to it, and nothing limits what
it does once it can.

This feature is planned, not started. Order matters: each step needs the one before it.

Guideline, from the principal (2026-09-30): Sjel is easy to use and easy to maintain. The
design underneath may be clever. It is stated in `README.md`, "Design rule: clever inside, easy
outside". It binds this feature: an agent's access must not need a hand-edited config file.

Measured inputs, all 2026-09-30 and all lower bounds:

- 15 capabilities serve a `/routes` manifest (`capabilities/comms/src/server/main.rs:79`
  is one). The registry lists 20 with HTTP. `knowledge-graph`, `macmon`, `foundation-models`,
  `ytalbum` and `dashboard` have none in `capabilities/*/src`; whether each should is not yet
  checked.
- Four capabilities have no README: `entities-sync`, `entities-google-sync`, `feed-sweep`,
  `sparpreis-watch`.
- A single-line grep of route registrations finds 164 `GET` and 92 write routes (85 `POST`, 5
  `PUT`, 2 `DELETE`). Multi-line `.route(` calls are missed. Whether any `GET` handler changes
  state is not measured.
- The gate has one shared token (`libs/sjel-server/src/auth.rs:48`). Whoever holds it may call
  every route, including comms' move-to-Trash action. Whether the API, not only the dashboard,
  enforces that action's separate confirmation is not checked.
- An assistant reading mail is reading text an attacker can write. In InjecAgent, injected
  tool output steered a GPT-4 agent 24% of the time (`research/agent-safety.md`). A read-only
  token limits what a steered agent can do. It does not stop the steering.

Placement, per `Packs/harness/skills/sjel/references/on-placement.md`: discovery and the
client stay in `sjel` and `tools/` (operator machinery). The gate stays in `libs/sjel-server`
(shared code, several consumers). No new capability. Any agent-protocol wrapper (MCP) is a thin
last layer over the routes and adds no data path of its own.

- [x] ISC-33 — `sjel search <word>` finds a registered capability by its README and by its
  `/routes`. Falsifier: `sjel search mail` returns no `comms`. Probe: that command. Done
  2026-10-01: `tools/lib/capability-index.sh` matches the README's opening paragraph and the
  route manifest, read from source because `/routes` needs the token and most capabilities are
  off. `sjel search mail` returns `comms` with `GET /triage`; `sjel search trash` returns
  `POST /triage/{id}/gmail`. Only the opening paragraph, because the whole README matched
  `backup` in 16 capabilities. Held by `tools/capability-index.test.sh`.
- [x] ISC-34 — every registered HTTP capability serves `GET /routes`, or its registry entry
  says why it does not. Falsifier: a capability in `sjel capability list` answers `/routes`
  with 404 and has no stated reason. Probe: loop over the list. `tools/doctor` runs it, so the
  rule is enforced where the tool reads it. Done 2026-10-01: 17 of 20 serve it. `knowledge-graph`
  and the overlay's `ytalbum` gained a manifest in the Rust shape. `macmon` and
  `foundation-models` (upstream binaries) and `dashboard` (the UI) carry a `routes_absent` reason,
  which the registry emits. The probe is static, not a live loop: a live `/routes` answers 401
  without the token, and so does a missing route. `tools/check-service-tomls.sh` refuses a port
  with neither (CI), and doctor's "Service manifests (both roots)" runs it over the overlay too.
- [x] ISC-35 — every capability has a README. Falsifier: `ls capabilities/*/README.md` misses a
  directory. Done 2026-10-01: `entities-sync`, `entities-google-sync`, `feed-sweep` and
  `sparpreis-watch` each have one, in the shape of `capabilities/punctuality-ingest/README.md`.
- [x] ISC-36 — from an agent session, `sjel capability call comms get /triage` returns data
  without the token appearing in argv, the environment or the transcript. Falsifier: it returns
  401, or the token shows in `ps`. Evidence, partial: `tools/capability-auth` reads the
  deployment token and `sjel` passes it through `curl -H @<(...)`, tested in
  `tools/capability-auth.test.sh`. Holds for comms since 2026-09-30 evening: `capability-auth
  comms` reads comms' own `api_secret_file` through `sjel_server::comms_config_token`, shared
  with sjel-status's proxy. It still uses the full comms token, which can trash mail (the
  principal's call that day; ISC-38 replaces it). Done 2026-10-01: the agent token is read from
  the login Keychain (`tools/capability-auth --agent`) and reaches curl as `-H @<(...)`, a file
  descriptor, so it is in neither argv nor the environment. `sjel capability mail` from this
  agent session returned 377 rows (288 c1, 89 pseudonymized c2, no c3) with no raw sender
  address, and a POST was refused: "the agent token is read-only".
- [x] ISC-37 — the token is read on demand from the user's own secret store (the macOS Keychain
  on a Mac), set up per user, with no `deployment.env` edit and no exported variable. Falsifier:
  the client works only after the user edits a file, or exports a secret into the environment.
  The principal's call, 2026-09-30. Done 2026-10-01: `sjel agent enroll` writes the agent token
  to the login Keychain and only its hash to the overlay, and the client reads it on demand.
  The principal confirmed the store that day for the inbound token too:
  `tools/setup-inbound-auth.sh` now writes the login Keychain, not Vaultwarden, and
  `tools/setup-secret.sh` and `upstreams.toml [bitwarden-cli]` no longer call Vaultwarden
  canonical.
- [x] ISC-38 — an agent identity admits `GET` and `HEAD` only. Falsifier: a `POST` carrying it
  reaches a handler. Probe: a unit test in `libs/sjel-server/src/auth.rs`. This amends the
  one-token ruling at `auth.rs:48` and the module docs change with it. It depends on ISC-37.
  Blocked in session 2026-09-30 by the safety check as a permission change: it needs an
  explicit go-ahead. Precondition: a measured list of `GET` handlers that change state, each
  fixed or listed as an exception. Go-ahead given 2026-10-01. Done that day: the gate refuses
  every other method for the agent identity (`libs/sjel-server/src/agent.rs:192`, held by
  `the_agent_token_cannot_write_or_read_what_it_cannot_rewrite` in `lib.rs`), and a live POST
  from an agent session answered "the agent token is read-only". The audit read 177 `GET` routes
  in 18 capabilities and found four that change state. Fixed: trips counted a `HEAD` as a
  completed write and rewrote the vault projection (`completes_a_write` now uses `is_safe`).
  Exceptions, none reachable by an agent because neither capability calls `admit_agents`:
  scouting `GET /discover` crawls third-party sources and stores the results (moving it to
  `POST` changes the demo recordings); sjel-status `GET /backup/targets` re-syncs the declared
  targets; sjel-status `GET /storage` may build the storage tool. `admit_agents`' doc names
  this list. Benign and not exceptions: lazy migrations, the device nonce, idle heartbeats.
- [ ] ISC-39 — a `GET` by an agent identity returns no value of class Secret, and no Others
  value it could not already read. Falsifier: an agent read returns a Secret row. Partial,
  2026-09-30: `GET /triage?max_data_class=c1` drops Others and Secret rows, and `sjel capability
  mail` always sends it. The ceiling is the caller's choice, not a gate, until ISC-38. The
  first live read found a leak the ceiling cannot catch: a one-time passcode in a subject
  line, stored as c1 and unredacted (UNiDAYS, 2026-09-26), and a named person's live-location
  mail stored as c1. The classifier's c3 and c2 rules miss both.
  Classifier fixed 2026-10-01 (`data-class-rules-v3`, `libs/content-item/src/lib.rs`): a code
  word beside a standalone 4–8 digit number is c3 (a year is not a code), and location-sharing
  phrases are c2. `POST /triage/data-class/refresh` re-derives stored rows and redacts what the
  new class requires. Still open: the ceiling becomes a gate only with ISC-38.
- [ ] ISC-40 — one MCP server exposes the capabilities' `/routes` as tools: read tools by
  default, write tools only with a per-capability grant. Falsifier: the tool list contains a
  `POST` route without a grant. Depends on ISC-33, 34, 36 and 38. Amended 2026-10-01: the grant
  is F10's per-capability mode, so a write tool is listed unless its capability is read-only.

Decided 2026-10-01, see F10: the write grant is a per-capability mode, and each agent call is
logged.

### F9 · One pseudonymizer, applied by the gate to every agent read

Why: an agent read of comms on 2026-09-30 returned 278 Mine rows verbatim: full addresses,
names, order numbers and a one-time passcode. Only c2 and c3 rows are redacted at intake
(`capabilities/comms/README.md`, "For c2 and c3 mail"). The reversible engine exists
(`libs/pseudonymize`, ISC-20) but only comms' review queue and trips call it, and each call
site chooses to. F9 makes the gate choose, so a capability cannot forget.

Rulings, principal, 2026-09-30:

- An agent has its own token (ISC-37, ISC-38). The gate pseudonymizes every response to
  that token. The dashboard token is unchanged.
- The mode is reversible, per session. One value gets the same token for the whole session in
  every capability, so an agent can refer back to it.
- **Pseudonymized c2 may reach an agent.** This amends Q27 for agent reads only: c2 leaves
  the machine as a pseudonymized derivative, never verbatim. c3 still never leaves.

Design:

- Tokens are keyed, not counted: `<PERSON_k3x9qa>` from HMAC-SHA256 over the type and the
  exact value. The key is derived from a machine secret and the session id, so two processes
  issue the same token without shared state. Without the secret, a party that sees tokens
  cannot confirm a guessed name.
- The JSON view keeps structural fields verbatim (ids, timestamps, enums, classes), tokenizes
  identity fields as one unit (an organisation's domain stays beside the token, as in
  `<SENDER_k3x9qa> (dhl.de)`, except for freemail, a domain holding a known name, or any c2
  row; principal, 2026-09-30), tokenizes free text with the existing ladder, and removes every
  object with `data_class` `c3`.
- The gate admits the agent token for `GET` and `HEAD` only, and only on a capability that
  opted in. It refuses a response that is not JSON, because it cannot transform it.
- The server holds only a hash of the agent token. The token itself lives in the login
  Keychain, and the client reads it from there.

- [x] ISC-41 — keyed tokens: one value maps to one token across two sessions of one key, and
  to a different token under another key. Probe: unit test in `libs/pseudonymize`.
- [x] ISC-42 — the JSON view: ids and timestamps survive, a sender becomes one identity
  token, a c3 object is removed and counted. Probe: unit test.
- [x] ISC-43 — the gate: an agent `GET` to an opted-in capability returns pseudonymized
  JSON; an agent `POST` returns 403; an agent call to a capability that did not opt in returns
  403. Probe: HTTP tests in `libs/sjel-server`.
- [x] ISC-44 — `sjel capability mail` from an agent session returns no raw sender address.
  Falsifier: an `@` in `from_addr`. Probe: that command after enrollment.
- [ ] ISC-45 — an agent cannot go around the gate. Code now makes `serve_local` fail closed
  on protected routes, keeps health/readiness exempt, preserves an agent bearer through both
  shells, and injects the deployment token only on server-to-server proxy hops. `sjel-status`
  also has a Tailscale-identity listener over a mode-0600 Unix socket under overlay `secrets/`;
  the dashboard browser never receives the shared token. Internal loopback capability callers
  use a helper that adds the token only to loopback destinations. The managed policy now denies
  `secrets/**`, including extensionless token files. Still open until the operator runs
  `tools/setup-inbound-auth.sh`, deploys the updated managed policy to the platform's actual
  path, starts the protected listener, applies `tools/setup-tailnet-shell.sh`, and verifies from
  an agent session that secret reads fail while Comms remains pseudonymized. No deployment
  credential or Tailscale Serve configuration was changed in this code session.
  Two consequences of failing closed, measured 2026-10-01. A browser on this Mac at
  `127.0.0.1:8082` carries no token: answered the same day by the principal's ruling (no command
  line, a 30-day sliding session). The menu-bar app (`apps/mac/install`, `~/Applications/Sjel.app`)
  trades the Keychain token for a single-use ticket, and the shell turns it into a session cookie
  (`capabilities/sjel-status/src/session.rs`). Still open: soundscape's panel loads from its own
  port (`panelUrl` in `dashboard/src/lib/api.ts`), so its browser requests carry no token.

### F10 · An agent writes, under a mode the owner sets

Why: F8 and F9 made agent access read-only. The principal ruled on 2026-10-01 that an agent
writes too, "native" through MCP tools, under a mode chosen per capability: read-only, ask or
auto. The default is auto. A small on-device model may later sort writes into safe and risky,
and is not part of this feature.

Rulings, principal, 2026-10-01:

- One mode per capability, set on the Systems page. No file is edited by hand.
- In ask mode the write waits for Allow or Deny in the menu-bar app (`apps/mac`), and the
  dashboard lists it too.
- The default is auto.
- Pseudonym tokens in an agent's write are turned back into the real values before the write
  reaches the capability, from the same session that issued them.
- Every agent call is logged: time, capability, method, path, status and the gate's decision.
  Never a body.

Product Rule 5 still applies inside auto ([product rules](CONTRIBUTING.md#product-rules)): "A
change that leaves Sjel, or cannot be undone, asks." A route that does either declares it in its
manifest, and the gate asks for it whatever the mode.

Placement: the gate stays in `libs/sjel-server`. sjel-status owns the policy, the approvals and
the call log, because it already owns the Systems page. The MCP server is operator machinery in
`tools/`. sjel-status itself never admits an agent: it starts and stops the machine's services.

- [ ] ISC-46 — the mode is per capability, starts at auto, and is changed on the Systems page.
  Falsifier: changing it needs a file edit, or an agent `POST` to a read-only capability reaches a
  handler. Probe: gate tests, and the page.
- [x] ISC-47 — in auto mode an agent write reaches the handler, and a route declared `confirm`
  asks anyway. Falsifier: an agent moves mail to Trash with no approval. Probe: gate tests.
  Done 2026-10-01: `a_confirm_route_asks_even_in_auto_and_one_approval_admits_one_write` and
  `in_auto_a_write_reaches_the_handler_with_its_tokens_restored_and_is_logged` in
  `libs/sjel-server/src/lib.rs`. comms declares ten confirm routes (`AGENT_ROUTES` in
  `capabilities/comms/src/server/main.rs`): the Gmail actions, the cloud approval and run,
  `/ingest`, redaction, and setting a data class by hand, because an agent that lowered a class
  could then read what it withheld. Every `serve_local` capability now admits the agent under
  its mode, keyed by its registry name; sjel-status does not.
- [x] ISC-48 — an approval is single-use and bound to the method, the path and a digest of the
  body. Falsifier: one approval admits a second write, or a different body. Probe: unit tests.
  Done 2026-10-01: `an_approval_admits_its_own_write_once` in `agent_policy.rs` and the gate
  test above. A claim is a file rename, so two requests racing for one approval admit one.
- [x] ISC-49 — tokens in an agent's write body become the real values before the handler, and a
  token the session never issued is refused. Falsifier: a handler receives `<PERSON_…>`. Probe:
  gate tests. Done 2026-10-01: `a_token_the_session_never_issued_is_refused` and the auto test
  above. A limit, measured in the design and not yet answered: keyed tokens are one-way, so only
  the capability process that issued a token can restore it. A token read from comms and written
  to trips is refused with a message that says so.
- [ ] ISC-50 — every agent call leaves one log row with no body, and the Systems page shows the
  latest. Falsifier: an agent call with no row. Probe: gate tests and the page.

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
- **Operator installs past cooldown**, neither a security item: tailscale 1.98.9 →
  1.102.2, xberg 1.0.5 → 1.0.14.
- **The remaining three database-URL call sites.** Retired with PostgreSQL (PRD Q45):
  all capabilities migrated to the shared SQLite store (`sjel-store`), eliminating
  `SJEL_<CAP>_DATABASE_URL` and `sjel_config::database_url_override` entirely.

- **Open source or open core.** AGPL-3.0 keeps both possible. Undecided whether parts stay
  closed later.
- **Funding.** Support and sponsorship with a build-in-public video series (decisions, papers,
  measurements, in the style of James Simo's city-builder devlog), hosting, or a company.
  Undecided.
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
- **The code graph cannot be rebuilt here.** graphify's semantic step calls
  `deepseek-ai/deepseek-v4-flash`, retired on 2026-08-07. This stopped blocking `self.json` on
  2026-09-29 (ISC-26): the per-unit counts are fused on read from `graphify-out/`, so
  `tools/self generate` needs no graph at all. What it still blocks is the graph's own freshness —
  `status` shows counts rolled up from whatever graph exists, and this machine's is behind the
  tree. Commit `2f0feb6` says it regenerated `self.json`; only `ARCHITECTURE.md` changed.
- **A comms test is about this machine, and its own comment says it is not.** Fixed in
  `c473694b`: `triage::tests::the_shadow_route_writes_verdicts_and_changes_no_category` now sets
  `inference: sjel_inference::InferenceConfig::default()` and points `database_path` at its own
  unique test fixture directory, hermetically isolating the shadow pass from both live models
  and overlay locks.
- **The demo site shows two areas less than it could.** Fixed 2026-09-27 (22793de3): the page
  clock runs on the recording's anchor date, so Travel shows 2 upcoming trips, and seven services
  missing from demo.toml now say why instead of showing a host's 404 page. People followed
  the same day (eceddd96): six invented people seeded through the entity store's own routes, and
  Home's own queries recorded, so only Tasks (vault, absent by design) is unavailable. The missing
  birthday row was the seed: Home's radar looks 7 days ahead and Mara's birthday was 9 days out;
  it is 5 now.
- **The names ISC-13 kept.** `sjel-status` (a service rename changes the phone app's allowed
  paths), `axon-fda-launcher` (a new binary name needs a new Full Disk Access grant), Linux
  `axon-<cap>` systemd units, the `X-Axon-*` signed-request headers (a protocol change for paired
  phones) and "Axon" in prose and doctrine. Each can move with a fallback when it is worth it.
- **A scheduled job cannot build a capability it requires.** A launchd unit's PATH holds the
  directories of its own command and build tool only. feed-sweep requires comms and starts it
  when it is down; after the 2026-09-26 checkout move comms needed a rebuild, and feed-sweep's
  run failed with "cargo: command not found" while comms' own watchdog, whose PATH has cargo,
  rebuilt it. Transient, and only after a clean or a move. The fix is to add the build tools of
  `requires` to persistence_path_dirs in tools/service-runner.sh. **Fixed in `c7b4ed2a`**
  (2026-09-30): `persistence_path_dirs` now adds every required capability's build tool
  directory, deduplicated, the same way it already does for the job's own runtime and builder;
  `tools/service-runner.test.sh` passes.
- **Stale workflow worktrees under `.claude/worktrees/` fail `tools/doctor`** with "package.json
  is not in the index". Local leftovers, not a repository defect.

## Test Strategy

| isc | type | check | threshold | tool | anchors_to |
| --- | --- | --- | --- | --- | --- |
| ISC-1 | command | `gh issue list --state open`, here and in the overlay | empty | gh | stated goal |
| ISC-2 | file | read each closing comment and its named ISA | content present | Read | "carry through ISAs" |
| ISC-3 | command | `rg -i "backlog is Issues"` over the four files | zero hits | rg | "no new issues" |
| ISC-4 | command | `rg "gh issue create" .github/workflows` | zero hits | rg | "no new issues" |
| ISC-5 | command | `cargo test --workspace --locked` | all pass | cargo | C1 |
| ISC-6 | command | demo build, inspect recorded responses | three capabilities present | bash | F1 |
| ISC-7 | code inspect | read transit's URL consts | env-overridable | rg | F1 |
| ISC-8 | queue | `gh pr list --author app/dependabot` | every open entry merged or held with a written reason | gh | F2 |
| ISC-9 | command | `git log` for the postgres retirement | its own commit | git | F2 |
| ISC-27 | command | `command -v axon`; `rg '$AXON\|"axon"' tools/ Packs/` | no resolution, no call | bash, rg | F7 |
| ISC-28 | code inspect | `libs/sjel-server/src/auth.rs` accepts `X-Sjel-*` and `X-Axon-*` | both accepted until the phone ships | rg | F7 |
| ISC-29 | command | run the launcher; confirm the Full Disk Access grant | reads what it needs | bash | F7 |
| ISC-30 | command | `rg -ci axon` over README, ISA, CONTRIBUTING, ARCHITECTURE | historical record and pinned names only | rg | F7 |
| ISC-31 | command | `sjel storage report --json`, then the Systems page | panel present, exit 0 | jq, browser | F7 |
| ISC-32 | command | build via Xcode, then `sjel storage target` | every target dir listed | bash | F7 |

## Anti-claims

- [x] A1 — no new tracking surface replaces the tracker. Falsifier: a `TODO.md`,
  `PLAN.md`, `HANDOFF.md` or `ROADMAP.md` appears anywhere in the repo, gitignored ones
  included.
- [x] A2 — no issue is closed whose content exists nowhere else. Falsifier: ISC-2 fails
  for any closed issue.
- [x] A3 — the public repo keeps a path for outside bug reports. Falsifier:
  `.github/ISSUE_TEMPLATE/` is deleted or issues are disabled repo-wide.

## Decisions

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
  `tools/storage` measures it and enforces R6; nothing showed it to a person. Recorded as F7,
  including the tool's own blind spot: a second Cargo target dir is invisible to
  `prune --target`.
- **2026-08-19 — the backlog moves from Issues to ISAs** (principal's call). Migrate
  first, then close; change the doctrine in all four places that state it; stop the one
  workflow that creates issues.
- **2026-08-19 — `upstream-watch` reports to the job summary and reds the run** rather
  than being deleted. Deleting it would leave drift findable only when someone looks.
- **2026-08-20 — three capabilities never read their database variable, and the demo is
  what found it.** `tools/demo-up`'s whole mechanism is one exported
  `SJEL_<CAP>_DATABASE_URL` per capability. comms, scouting and transit ignored it, went
  from their config file to `postgres_conn_from_shared_env`, and — the demo overlay having
  no `postgres.env` — landed on a fallback naming the real database, `dbname=axon
  password=axon`. The only thing between a demo seeding run and the live store was that the
  real password is not the word `axon`. Fixed with one shared
  `sjel_config::database_url_override`, used by those three. The four that hand-roll the
  same two lines (calendar, finance, tasks, trips) are left alone and recorded below.
- **2026-08-20 — `upstream-checker` published the checkout's absolute path.** Its `--json`
  `manifest` field was `$SJEL_ROOT/upstreams.toml`, which sjel-status serves and the demo
  records. `tools/check-site-payload` refused to publish over it, which is the job that
  gate has. Now repo-relative.
- **2026-08-19 — `.github/ISSUE_TEMPLATE/` stays.** Sjel is public and an external
  report still needs somewhere to land; what changed is that our own backlog is not there.

## Log

- 2026-09-30 · ISC-31 landed: `sjel-status` serves `sjel storage report --json` at
  `GET /api/sjel-status/storage`, and the Systems page draws it — checked figure by figure
  against the tool, not against a 200. Reaching the page found a bug older than the feature: the
  rune probe at `dashboard/src/lib/models/decision-engine.svelte.ts:31` (481a2d5d, 2026-09-29)
  read `globalThis.$state`, which a browser defines as a getter that throws, so every route
  served SvelteKit's 500 page while 183 tests and `svelte-check` were green — nothing mounts a
  component. Fixed by making the probe catch. c68e7fb6 left ARCHITECTURE.md and its renamed
  inputs uncommitted; they land with this. Four repo gates stay red for neither reason:
  `capabilities/operator-profile/` is untracked (the F8 session, 18:12), which is what fails
  generator inputs, ARCHITECTURE freshness and `self.json`.
- 2026-09-30 · F9 built: keyed tokens and the agent view in `libs/pseudonymize`, the agent
  branch of the gate in `libs/sjel-server/src/agent.rs`, comms opted in, `sjel agent enroll`.
  ISC-41…43 pass as unit and HTTP tests. ISC-44 passes live: 364 rows, no address, no
  six-digit run, 13 Secret rows withheld.
- 2026-09-30 · F8: `sjel capability mail` reads comms triage with a c1 ceiling (ISC-36 for
  comms, ISC-39 partial); two classifier misses recorded under ISC-39.
- 2026-09-30 · F8 added with ISC-33…ISC-40, planning only. `tools/capability-auth` and the
  `sjel capability call` change (ISC-36, partial) landed uncommitted the same day.
- 2026-09-30 · F7 added with ISC-27…ISC-32; ISC-11 superseded and ISC-13 amended after the
  environment fallback was retired (9faf670a). A disk-pressure session reclaimed ~137 GB on
  this machine, which is how the blind spot F7 records was found.
- 2026-08-19 · Scaffolded. Carries Axon issues #172, #174, #180 and the tracker
  retirement itself; #185 and #186 went to `Packs/travel/ISA.md`.
- 2026-09-26 · F3 to F5 and five Not-yet-specified entries added from the session that named
  Sjel, switched the license and moved the product document into the README.
- **From the old PRD, not yet planned.** Device loss as a threat (what a stolen phone exposes).
  A health domain (energy, sleep, training, records). Item intake: scan an object, share a link,
  one wishlist. Travel: route composition, "where should I base myself", an accommodation source.
  People: Google Contacts beyond inbound, a TELOS import, trips from a person, meetup capture.
  An autonomy gate for a structured-decision model (apply at ≥ 0.95 confidence, otherwise a
  card). shellcheck as shell analysis. `tools/backup.sh` skipping private capability manifests.
  The PRD's non-goals, which conflict with Sjel ("not a product", "tailnet only") and need a new
  ruling rather than a copy.
