---
project: axon
type: isa
phase: climbing
progress: 75
principal_stated_goal: "I want no new issues, I wanna get rid of all issues for axon and axon personal and only carry through normal ISAs etc."
---

# ISA · Axon

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

- Disabling the issue tracker for the outside world. Axon is public; an external bug
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
- **C2** — `tools/self check` fails locally and that is pre-existing: it compares per-unit
  code counts only when a code graph is present, and this machine's graph is behind main.
  Verify in a `git worktree` of origin/main, not in place.
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

- [ ] ISC-8 — every entry a Dependabot pull request or alert names as behind or vulnerable
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
  network-backed and therefore an operational dependency, not a replacement for the hold.)
- [x] ISC-9 — the postgres 17.9 → 17.10 image decision is made on its own, not ridden
  along with another change. Falsifier: the bump appears in a commit about something else.
  Closed 2026-08-27 by PRD Q45, which retired the image rather than bumping it: the running
  17.9 instance is read once by `tools/migrate-pg-to-sqlite` and then stopped, so a 17.10
  decision has no subject. The constraint held either way — the retirement is its own
  commit, and the version that ran is in `upstreams.toml`'s git history at that date (Q77
  deleted the field on 2026-09-02).

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
- [ ] ISC-12 — the iPhone app has a Sjel bundle identifier and the phone is paired again.
  Changing the identifier makes iOS treat it as a new app: its offline copy and its Keychain
  key are gone. Falsifier: `com.lifeos.mobile` remains in `tauri.conf.json`, or the new app
  fails `devices/api/devices/me`. Done in code (725e70f: `com.larsboes.sjel`, the IPA signs).
  Open: install the Sjel app and pair it.
- [ ] ISC-13 — internal names move with a fallback: crates, `AXON_*` variables (the old name
  still read), launchd labels and the overlay. Falsifier: a service that ran before the change
  does not start after it, or a variable is renamed with no fallback reader. Progress,
  2026-09-26: the skill is `sjel` (ec52c11c); launchd labels are `com.sjel.*` with the old units
  removed (0479a731, 15 of 15 moved); settings are `SJEL_*`, read with the `AXON_*` fallback in
  Rust, shell and TypeScript. Open: the overlay's keys, the crates, the overlay repository.

### F4 · The documents a stranger reads

Why: the top of the README is the product definition, and the sources behind the design go into one
curated place. The README is still 870 lines of doctrine below 230 of product.

- [ ] ISC-14 — the README opens like a large open-source project (Graphify, Ollama): logo,
  badges, a screenshot of phone and dashboard showing travel and people first (the areas a
  stranger meets first), what it does, a three-command start, and a
  table of measured results (pseudonymizer 48/48, redaction recall 100% on the frozen corpus,
  Feed ranking 0.941 pairwise, the 1.5 s Same Wi-Fi fallback). Falsifier: a number in that
  table without a command or file that reproduces it.
- [ ] ISC-15 — `research/` states why the project exists: the problem it answers and the
  sources that show the problem is real. It grows over time; the first version holds at least
  one sourced entry, and the demo site shows it. Falsifier: an entry without a source, or a
  claim its source does not support.
- [ ] ISC-16 — the engineering doctrine lives in `CONTRIBUTING.md`, and no link points at a
  README anchor that no longer exists. About 208 links point into it today. Falsifier:
  `git grep "README.md#"` finds an anchor missing from `README.md`.

### F5 · The session's work, proven on real devices

Why: the transports and the model ladder shipped on 2026-09-25 with tests, but nothing ran on
the phone. Each item below is built and unverified, or ruled and unbuilt.

- [ ] ISC-17 — a phone on the home Wi-Fi reaches the Mac's `:8443` listener, pins it after the
  code comparison, and reads data with the tailnet off. Blocked by an operator act: the macOS
  firewall must admit the signed `axon-status` once. Falsifier: `curl -k
  https://<LAN address>:8443/health` from another device does not answer 200.
- [ ] ISC-18 — the assistant drawer calls the model ladder (`dashboard/src/lib/intelligence`)
  for at least one task and shows which rung answered. Falsifier: `rg "intelligence/backends"
  dashboard/src` finds no caller outside the module and its test.
- [ ] ISC-19 — `machNotch` moves into this repository and grows into the Mac app. It hosts
  the CloudKit relay, and a test reads a record back as Apple stores it and finds ciphertext
  (README, rule 4). Falsifier: a field name or value of a C2 record readable in the stored record.
- [ ] ISC-20 — the comms review queue sends pseudonymized jobs through `prepare_pseudonymized`
  (README, rule 4). Falsifier: a queued job whose payload carries a raw C2 entity.
- [ ] ISC-21 — the family deployment runs for a week without Lars touching it.
  Falsifier: any fix to it made by Lars in that week.

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
- **The remaining three database-URL call sites.** calendar, finance and trips each
  hand-roll the same `std::env::var("SJEL_<CAP>_DATABASE_URL")` two-liner that
  `axon_config::database_url_override` now owns. Four until 2026-08-27, when PRD Q48
  retired `tasks` and deleted its copy — the entry below still says four because that is
  what was true when it was written. They work, so this is deduplication rather
  than a defect, and the repo's rule is that shared logic moves into the lib. Not swept in
  the run that added the helper, deliberately: that run was about the demo.

- **Open source or open core.** AGPL-3.0 keeps both possible. Undecided whether parts stay
  closed later.
- **Funding.** Support and sponsorship with a build-in-public video series (decisions, papers,
  measurements, in the style of James Simo's city-builder devlog), hosting, or a company.
  Undecided.
- **The pseudonymizer as its own library.** Grow the data classes and the reversible
  pseudonymizer into a standalone Rust crate with its own README. Performance work (unsafe Rust included)
  only after a benchmark says where the time goes.
- **Private Cloud Compute as a ladder rung**, for pseudonymized prompts only (README, rule 4). The plugin
  reports its availability today and never calls it.
- **A fast structured-decision model as a rung**, the kind Jev is (typesafe.ai, 2026). Candidate,
  not measured.
- **Generative interface from the typed core** (README, rule 6). No design yet.
- **The on-device model path is untested on an eligible device.** The iPhone 14 Pro reports
  `deviceNotEligible`; a 15 Pro or later, or a Simulator, is needed.
- **`self.json` cannot regenerate.** graphify's semantic step calls
  `deepseek-ai/deepseek-v4-flash`, retired on 2026-08-07, so `tools/self generate` refuses. Commit
  `2f0feb6` says it regenerated `self.json`; only `ARCHITECTURE.md` changed.
- **A scheduled job cannot build a capability it requires.** A launchd unit's PATH holds the
  directories of its own command and build tool only. feed-sweep requires comms and starts it
  when it is down; after the 2026-09-26 checkout move comms needed a rebuild, and feed-sweep's
  run failed with "cargo: command not found" while comms' own watchdog, whose PATH has cargo,
  rebuilt it. Transient, and only after a clean or a move. The fix is to add the build tools of
  `requires` to persistence_path_dirs in tools/service-runner.sh.
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

## Anti-claims

- [x] A1 — no new tracking surface replaces the tracker. Falsifier: a `TODO.md`,
  `PLAN.md`, `HANDOFF.md` or `ROADMAP.md` appears anywhere in the repo, gitignored ones
  included.
- [x] A2 — no issue is closed whose content exists nowhere else. Falsifier: ISC-2 fails
  for any closed issue.
- [x] A3 — the public repo keeps a path for outside bug reports. Falsifier:
  `.github/ISSUE_TEMPLATE/` is deleted or issues are disabled repo-wide.

## Decisions

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
  `axon_config::database_url_override`, used by those three. The four that hand-roll the
  same two lines (calendar, finance, tasks, trips) are left alone and recorded below.
- **2026-08-20 — `upstream-checker` published the checkout's absolute path.** Its `--json`
  `manifest` field was `$SJEL_ROOT/upstreams.toml`, which axon-status serves and the demo
  records. `tools/check-site-payload` refused to publish over it, which is the job that
  gate has. Now repo-relative.
- **2026-08-19 — `.github/ISSUE_TEMPLATE/` stays.** Axon is public and an external
  report still needs somewhere to land; what changed is that our own backlog is not there.

## Log

- 2026-08-19 · Scaffolded. Carries Axon issues #172, #174, #180 and the tracker
  retirement itself; #185 and #186 went to `Packs/travel/ISA.md`.
- 2026-09-26 · F3 to F5 and five Not-yet-specified entries added from the session that named
  Sjel, switched the license and moved the product document into the README.
