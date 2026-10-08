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
  references, none under the old name). The three surfaces named above stay open as ISC-28,
  ISC-29 and ISC-30 below.
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
- [x] ISC-28 — the signed-request headers are `X-Sjel-*`, with the old name still accepted
  until the paired phone ships. Falsifier: a paired phone's signed request fails after the
  rename. Probe: `libs/sjel-server/src/auth.rs` accepts both, asserted in its own tests. Done
  2026-10-01: the gate renames each `x-axon-*` header to its `x-sjel-*` name before anything
  reads it (`normalize_legacy_headers`), held by
  `a_legacy_header_is_renamed_and_the_current_one_wins` and
  `a_phone_sending_the_legacy_header_names_is_still_admitted`. The phone's signer
  (`dashboard/src-tauri/src/mac_bridge.rs`) still sends the old names on purpose, until every
  server has restarted on the renaming gate. The protocol string `axon-device-auth/v1` stays: it
  is part of what the device signs.
- [ ] ISC-29 — `axon-fda-launcher` is renamed, and its Full Disk Access grant is intact
  afterwards. Falsifier: the binary runs without FDA and cannot read what it needs. Probe: run
  it and confirm the grant. Renaming drops the grant, so re-granting is part of the work.
  Progress 2026-10-01 (cf3e888e): the binary and crate are `sjel-fda-launcher`, signed as
  `dev.sjel.fda-launcher`. No manifest declares `full_disk_access`, so nothing runs through it
  today. Open until the grant is given again and a run reads what it needs.
- [x] ISC-30 — "Axon" no longer appears in prose and doctrine. Falsifier: `rg -i axon` over the
  tracked documents returns hits that are not historical record (a commit message, an entry in
  this file), a platform-pinned identifier, or a deliberate alias. Probe: `rg -ci axon` over
  `README.md ISA.md CONTRIBUTING.md ARCHITECTURE.md`. Evidence, 2026-09-30 (c68e7fb6): 284
  occurrences rewritten across 100 files, of which `README.md` and `CONTRIBUTING.md` were
  already clean. 22 remain, each one of: historical record (`Axon held a reviewed delta against
  it until 2026-08-25`, `Axon#126`, `Projects/Axon/PRD Axon.md`), the transition's own name
  (F3 and the `/Axon/` URL that 404s as its evidence), an identifier rather than prose
  (`axon-*`, `AXON_*`, `X-Axon-*`), or this ISC's own text.

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

- [ ] ISC-22 — the three success criteria are measured again: no raw data about other people in
  the egress log, a trip planned in under 30 minutes, and one surface in place of five apps.
  Falsifier: any of the three has no command or log query that shows its current value. Done 2026-09-29:
  1. Egress privacy: `comms egress-log --audit` reports 0 raw C2 violations (verified by
     `egress_log_records_outbound_model_calls_and_audit_verifies_c2_absence`).
  2. Trip planning: `trips draft-intent "weekend in Paris under 200 euro by train"` produces structured
     plans in < 1 s (under the 30-minute threshold; search engine constrained to 30 s deadline).
  3. One surface: `dashboard/src/routes` unifies 5 apps (calendar, feed/comms, finance, travel/trips,
     interior) on one surface consuming `content-item-v2`, served at `http://127.0.0.1:8082`.
  **Reopened 2026-10-08.** Two of the three values are proxies. Value 2 times a draft, not a trip:
  `draft-intent` returns a plan nobody has submitted, and that is the first step of planning, not
  all of it. Value 3 lists routes, and a list of routes does not show that the five apps stopped
  being opened. Value 1 stands. New probes:
  - Value 2: the operator plans one real trip, from the first request to a plan whose every leg is
    booked or bookable, and records the start and end time. Threshold: under 30 minutes.
  - Value 3: for seven days, the operator writes down each time one of the five replaced apps is
    opened for a task its route covers. Threshold: no entry.
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
- [ ] ISC-25 — the product rules answer the counter-evidence in `research/`: a cloud request
  carries only the fields its task needs (Staab et al.), and confirmations are rare enough to be
  read (Akhawe and Felt), with the prompt rate measured. Falsifier: rule 4 or 5 unchanged with
  no measurement that answers the source. Done 2026-09-29: Product Rule 4 in `CONTRIBUTING.md`
  answers Staab et al. via strict task-scoped field minimization (verified by
  `cloud_derivative_carries_only_fields_task_needs_answering_staab_et_al`); Product Rule 5 answers
  Akhawe & Felt by reserving prompts exclusively for irreversible/off-host actions with routine
  prompt rate measured at 0.0% (verified by
  `autonomous_processing_has_zero_prompt_rate_answering_akhawe_and_felt`); both documented in
  `research/cloud-models-and-privacy.md` and `research/agent-safety.md`.
  **Reopened 2026-10-08.** The Staab et al. half stands: field minimization is a property of the
  code, and a test is a fair probe for it. The Akhawe and Felt half does not: the 0.0% comes from
  a unit test that runs autonomous processing, and the claim asks for the rate people see. New
  probe: for one calendar month, divide the approvals in `secrets/agent/approvals/` by the lines in
  `secrets/agent/calls/<YYYY-MM>.jsonl` (`libs/sjel-server/src/agent_policy.rs`, module doc). An
  agent may not read either path, so the operator runs the count. Threshold: not set. The
  principal sets it after the first month's value.

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
  `backup` in 16 capabilities. Held by `tools/capability-index.test.sh` until 2026-10-02, when
  the index moved into Rust; now held by `capability_index_finds_comms_for_mail` in
  `tools/sjel-cli/src/search.rs`.
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
  new class requires. The ceiling is a gate since ISC-38 and ISC-43: the gate removes every
  c3 object from an agent read. Probe 2026-10-01: `sjel capability mail` from an agent session
  returned 383 rows, 292 c1 and 91 c2, with no c3 and no raw sender address. A code word beside
  a 4–8 digit number matched 2 c1 rows. This session did not read their content, because the
  agent policy refused it, so whether they hold a Secret value is the operator's check. Open
  until that check is done.
- [x] ISC-40 — one MCP server exposes the capabilities' `/routes` as tools: read tools by
  default, write tools only with a per-capability grant. Falsifier: the tool list contains a
  `POST` route without a grant. Depends on ISC-33, 34, 36 and 38. Amended 2026-10-01: the grant
  is F10's per-capability mode, so a write tool is listed unless its capability is read-only.
  Done 2026-10-01: `tools/sjel-mcp.ts`, run as `sjel mcp`, register with `claude mcp add sjel --
  sjel mcp`. Tools come from each registered gate's `GET /routes`, so a new route is a new tool.
  Live, from this agent session: 43 comms tools (31 writes, 10 marked as waiting for the owner),
  and `comms__get_triage` returned 382 rows with no raw sender address. Held by
  `tools/sjel-mcp.test.ts`. Other capabilities register their gates on their next restart.
  Progress 2026-10-02: the registration above was never run and could not have been. `claude mcp
  list` was empty, and `claude mcp add sjel` now answers "not allowed by enterprise policy":
  the deployed managed policy sets `allowManagedMcpServersOnly: true` with `allowedMcpServers:
  []`, and `mergeFragment` refuses to let an overlay fragment extend an allowlist
  (`rejects expanding an allowlist even when the base declares the array`,
  `tools/claude-code-config.test.ts`). The Claude Code half of this row's own text is
  therefore unreachable until the principal decides whether `tools/templates/claude-code/
  managed-settings.json` should name `sjel`, which is a source change plus a root redeploy,
  not a config edit. Pi was registered instead, user-level at `~/.pi/agent/mcp.json`, exposure
  `codemode` so no tool schema is declared, and `timeout: 180` because an ask-mode write may
  wait 120 s for the owner while Pi's default is 60 s. `pi mcp list`: connected, 87 tools
  (comms 43, calendar 34, devices 10). Measured the same day through the server, not the shell:
  one `comms__get_triage` call returned 384 rows in 589 KB, about 147k tokens, and ignored a
  `limit` query parameter, so no capability read may be returned to a model unfiltered; 0 of the
  384 `from_addr` values held an `@`, all 384 matched `<TYPE_token>`, and no `c3` row was
  present, re-confirming ISC-43 and ISC-44 on a second day. Guidance and worked recipes are in
  `Packs/harness/skills/sjel/references/with-operations.md`, "Through MCP tools".
  Progress 2026-10-02, later: registration is no longer a command in a document. `sjel mcp
  register [<harness>]` and `unregister` (tools/sjel-mcp/sjel-mcp, crate `sjel-mcp`, held by its
  own tests) write pi's entry, drive `claude mcp add` for Claude Code, and then speak MCP to the
  server they just registered, reporting the tool count; 87 answered. pi is the one harness written
  directly rather than driven through its CLI, and the measurement is the reason: `pi mcp add`
  rejects `--timeout` ("Unknown option"), and pi's 60 s per-request default would cut off an
  ask-mode write that waits up to 120 s for the owner's Allow. Claude Code needed no such setting,
  and an inference on the way there was wrong and is recorded so it is not made twice:
  `MCP_TOOL_TIMEOUT` defaults to 100000000 ms, about 28 hours, so writing 180000 would have cut a
  28-hour budget to three minutes. Codex is reported and not touched: ~/.codex exists on this
  machine and no `codex` binary does, so a writer for it could not be verified even once, and
  tools/sjel-cli/src/harnesses/registry.rs states the rule that a row moves in only on a verified
  format.
  Superseded the same day, and recorded because it was acted on before it was: this note first
  described a narrower fix — pinning the server into the managed policy's `allowedMcpServers` as
  `{"serverCommand": ["${HOME}/.local/bin/sjel", "mcp"]}`, staged and awaiting a root deploy. The
  principal asked instead whether that layer earned its cost, and the ruling below retired it. The
  entry, the template that carried it and the test tying registration to it are deleted; MCP now
  has no allowlist at all, `claude mcp add` is the whole story, and `sjel mcp register claude` has
  no policy to satisfy. The vendor conflict this note left open — the settings schema saying
  `allowManagedMcpServersOnly: true` makes user-scope servers "ignored", against
  `/docs/en/managed-mcp`'s "Approved catalog" pattern saying the allowlist filters them — is moot
  with the key gone, and is recorded only because it was never settled by measurement.
  Progress 2026-10-04: the server is Rust. `tools/sjel-mcp.ts` and its test are deleted, and
  `src/server.rs` (the JSON-RPC loop, the gate-backed tool list, the approval polling) and
  `src/tools.rs` (mode to tool list, name and URL) hold both halves of the row in the `sjel-mcp`
  crate. Held by 20 `cargo test -p sjel-mcp` cases, the tool-list cases the TypeScript test had
  among them. Compared against the TypeScript before deletion on this machine across one request
  stream — `initialize`, `tools/list`, `ping`, a notification, an unknown method, a malformed
  line, a live `tools/call` and an unknown name — byte-identical reply for reply except the tool
  order, the sorted JSON object keys and the pseudonym tokens, all three named in
  `tools/sjel-mcp/README.md`. The live list was the same 92 tools as the TypeScript's, for the
  same gates.

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
  Progress 2026-10-01: the operator ran `tools/setup-inbound-auth.sh` and installed the managed
  policy at `/Library/Application Support/ClaudeCode/managed-settings.json`. Until then this Mac
  had none: the only copy was the Linux path `/etc/claude-code/`, which macOS ignores. After a
  restart on the new token, every running capability answers `/health` with 200 and a protected
  route with 401 when no token is sent, and `sjel capability mail` returns 383 rows with no
  address. Still open: `tools/setup-tailnet-shell.sh`, and a probe of the `secrets/**` deny from
  a session started after the policy was installed. The session that installed it was not
  refused, which fits a policy read at startup (not verified).
  Progress 2026-10-04: the local browser session is now a shared identity in the gate
  (`libs/sjel-server/src/session.rs`), verified by every capability. It does not widen the agent
  path: a session is minted only behind the deployment token (`POST
  /api/sjel-status/session/ticket` is gated, and the shell's listener calls `require_credential`),
  an agent bearer is not a session, and a `refuse_without_token` route still refuses one.
  Two consequences of failing closed, measured 2026-10-01. A browser on this Mac at
  `127.0.0.1:8082` carries no token: answered the same day by the principal's ruling (no command
  line, a 30-day sliding session). The menu-bar app (`apps/mac/install`, `~/Applications/Sjel.app`)
  trades the Keychain token for a single-use ticket, and the shell turns it into a session cookie
  (`capabilities/sjel-status/src/session.rs`). **Closed 2026-10-04:** soundscape's panel loads
  from its own port (`panelUrl` in `dashboard/src/lib/api.ts`), so its browser requests carry no
  token — and until this date nothing on that port could verify the shell's session, because the
  session was a table only the shell could read. The session is now a signed token
  (`libs/sjel-server/src/session.rs`) that `InboundAuth::resolve` verifies for every capability,
  so the panel and its API answer `200` to the browser's cookie and `401` without it.
  Progress 2026-10-01, later: a session started after the policy was installed refuses a
  `secrets/**` read. `Read` of a missing file under the overlay's `secrets/` returned "denied by
  your permission settings" instead of "not found", and `cat` of the same path was denied
  before it ran. The session's sandbox also lists the overlay `secrets/` paths as read-denied.
  This confirms the startup-read explanation above. Still open: `tools/setup-tailnet-shell.sh`.
  Ruling, principal, 2026-10-01: the Bash sandbox is chosen per launch and is off by default.
  It blocked `git push`, the interceptor daemon, local ports and the Tailscale CLI. The managed
  policy no longer sets `sandbox.enabled`, and `tools/claude-sandboxed` turns it on. Without it,
  the permission deny rules still refuse a `Read` of `secrets/**` and a Bash command that names
  `personal/secrets`. A Bash command that reaches the file without naming the path is not
  refused. ISC-45 therefore holds in full only for sessions started with `tools/claude-sandboxed`.
  Measured the same evening: Tailscale Serve proxies `/` to the protected socket and answers
  `/health` with 502, while the sjel-status log reports the socket listener ready. This Mac runs
  the Tailscale system-extension build (`io.tailscale.ipn.macsys`). Upstream says that extension
  "cannot serve arbitrary paths on macOS due to sandbox restrictions"
  (`cmd/tailscale/cli/serve_v2.go`). Verified later the same evening: it does. During one
  request, `tailscale debug daemon-logs` printed `http: proxy error: dial unix <overlay
  secrets/>sjel-status.sock: connect: operation not permitted`, while `lsof` showed sjel-status
  holding that socket. The extension's sandbox refuses the connect; the listener is not at fault.
  Ruling, principal, 2026-10-01: keep the installed Tailscale app, and install no other Tailscale
  build. The open-source `tailscaled` is therefore not a fix for this. The tailnet shell stays
  unreachable until a socket path the extension may connect to is found; everything else on the
  tailnet is unaffected.
  Ruling, principal, 2026-10-02: the managed policy layer is retired. Asked whether it earned its
  cost, the principal's answer was no — it is hard to edit and grasp, it needed sudo on every
  device for every change, and its MCP allowlist had silently blocked this machine's own graphify
  server since 2026-08-02 without anything reporting it. The floor now lives in
  `tools/templates/claude-code/settings.base.json` and lands in the user-level
  `~/.claude/settings.json` via `sjel claude` (crate `sjel-claude-config`, in Rust under
  `tools/claude-code-config/`), with no privileged step anywhere in the path. What this does to
  this row's claim is a weakening and is recorded as one: the same reads are still refused, and
  deny entries still merge across scopes so no other scope can lift one, but the file that holds
  them is now writable by the agent sessions that run as this user. "An agent cannot go around the
  gate" was true of Claude Code while the file was root-owned; it is now "an agent does not, and
  `sjel claude check` reports drift with exit 3 if it did". That check is the replacement for the
  guarantee, not a restoration of it. The row stays open, and its probe is unchanged.
  What could not come along, verified against the settings schema rather than assumed: two keys
  are managed-only and are now inert if written at user level. `disableSideloadFlags` is lost, so
  `--mcp-config`, `--plugin-dir`, `--plugin-url` and `--agents` are accepted again;
  `sandbox.enabledPlatforms` is lost too, which matters only where the sandbox is unsupported —
  `failIfUnavailable: true` is in the floor, so a device that is not macOS or Linux would now fail
  to start rather than ignore an inert config. `allowManagedPermissionRulesOnly`,
  `allowManagedMcpServersOnly`, `allowedMcpServers` and `sandbox.filesystem.allowManagedReadPathsOnly`
  went with the layer, and their removal is the point: user-level permission rules work again and
  MCP servers load without an allowlist.
  The deployment's own rules did not vanish with it. This machine's
  `<overlay>/config/claude-code/managed-settings.fragment.json` still supplies 8 deny rules, 3 read
  and 3 write sandbox paths, and 12 credential identifiers that are not this repository's to
  publish; the tool reads the same file under the name `settings.fragment.json` and still accepts
  the old name with a note, so a rename nobody made cannot drop a rule silently. A fragment that
  names an array appends to it rather than replacing it, so naming `permissions.deny` cannot drop
  the shared floor by omission. The guard that refused a fragment extending an allowlist is gone
  with the allowlist it guarded, and the guard that refused a fragment relaxing the base is gone
  too — it protected a policy the operator deployed, and the overlay owner can now edit the merged
  result directly, so keeping it would have been theatre.
  Deployment state on this Mac at the time of the ruling: the root-owned
  `/Library/Application Support/ClaudeCode/managed-settings.json` was still installed and still
  enforcing, so the move had not taken effect. It must be removed by the operator with sudo, and
  the order matters — while it is present, `allowManagedPermissionRulesOnly: true` makes user-level
  permission rules be ignored, so the new floor is inert and cannot be verified. Apply the user
  file first (done in the same session), then remove the deployed file, then verify both halves: a
  `secrets/**` read refused, and graphify loading again after `sjel mcp register claude`. The
  vendor conflict ISC-40 recorded — schema text saying user-scope servers are "ignored" under
  `allowManagedMcpServersOnly`, against the docs' "Approved catalog" pattern — is moot once the
  key is gone, and was never settled by measurement.
  Open, and deliberately not built: hooks. The principal's note on this ruling was that the
  security half might belong in hooks instead. They are worth naming precisely, because they do not
  restore what was given up: hooks run outside the sandbox, like MCP servers, and are referenced
  from the same user-level file, so they extend what can be enforced dynamically — the case the
  deny rules cannot express, a Bash command that reaches a protected file without naming the path —
  but not tamper-resistance, which is what the root-owned layer actually provided.
  Progress 2026-10-08: `/Library/Application Support/ClaudeCode/managed-settings.json` is absent,
  and graphify's MCP server loads in an agent session. The removal step above is done. The
  `secrets/**` refusal under the user-level floor is not probed yet.

### F11 · What nothing was moving

Why: `capabilities/host-patch` moves brew, uv and rustup nightly (Q77),
`capabilities/container-refresh` moves the images, and the agent harnesses move themselves.
Everything else was moved only when somebody remembered it existed. Measured 2026-10-01 on this
host: four `cargo install`ed crates, all four behind — including `macmon` 0.7.0 against 0.8.2,
whose only declared update path is `toolchain.toml` [macmon]'s own install hint — and eleven
stale global npm packages, none of which anything had ever asked about. The two upstream harness
integrations had gone 21 days since upstream's installer last wrote their files.

Placement, per `Packs/harness/skills/sjel/references/on-placement.md`: a new tool in `tools/`
("Repository identity, install, wiring, or operator machinery"), reached as `sjel update`. No new
capability and no new route on the CLI side — the scheduled jobs stay the owners of what they own
and this reports and delegates to them. The one thing it must never do is grow a second
`brew upgrade`: one binary, one owner is `tools/host-patch.sh`'s own rule, and PRD §13 is what
this deployment paid the last time it was broken.

- [x] ISC-51 — `sjel update` reports every class of installed software with its owner named, and
  exits 1 when anything is stale. Falsifier: a class of installed software that no row names, or
  a stale row with no owner. Probe: `sjel update`, and `sjel update --json` against the table.
  Evidence, 2026-10-01: ten classes, 15 stale rows on this host, every one carrying the command
  that moves it. The row set is `SURFACES` in `tools/sjel-cli/src/updates/report.rs`, and a unit
  test asserts every row groups under a known surface so the renderer cannot drop one silently.
- [x] ISC-52 — `apply` moves only what nothing else owns, and delegates the rest to its owner.
  Falsifier: `apply` runs a package manager whose binaries already have an owner. Probe: unit
  test over the plan, asserting no step's argv contains `brew upgrade`, `uv tool` or
  `rustup update`. Evidence, 2026-10-01: brew, uv and rustup go through `tools/host-patch.sh`;
  the integrations through `tools/agent-integrations.sh update`; only cargo and npm are run
  directly, because nothing else runs them.
- [x] ISC-53 — the report distinguishes "the job ran" from "something is newer", and never
  prints an unchecked class as current. Falsifier: a `--offline` run that reports a row current
  without having asked. Probe: `sjel update --offline`, and the unit test asserting no registry
  is consulted. Evidence, 2026-10-01: brew answers for itself via `brew outdated`, rustup via
  `rustup check`, and the job's receipt is reported as a note rather than folded into a status,
  because the audit's verdict is a fact about the machine and not about brew.
- [x] ISC-54 — a pre-release that merely sorts higher is named and not adopted. Falsifier:
  `apply` installs an `-alpha`/`-beta` over a stable release. Probe: `cargo search tauri-cli`
  answers `3.0.0-alpha.4` over the installed `2.12.0`; the row must say current. Evidence,
  2026-10-01: the row reads `newer pre-release 3.0.0-alpha.4 exists — not adopted` with no
  action, asserted in the updates module's own tests.
- [ ] ISC-55 — the Systems page shows what is stale and what nothing owns, from the tool's own
  `--json`, and can start one class moving. Falsifier: the page renders no updates panel, or
  `sjel update --json` exits non-zero without emitting it. Probe: `sjel update --json` and the
  page. Evidence, 2026-10-01: `GET /api/sjel-status/updates` serves the report and
  `POST /api/sjel-status/updates/apply` starts one class, both mounted above `build_router`'s
  origin-guard line and declared in `ROUTES` (the coverage test passes both ways). The apply
  route answers `202` before the work starts and the tool writes
  `<overlay>/data/updates/last-apply.json`, which every report carries as `lastApply` — because
  `apply --only cargo` was measured at 5m06s for three crates, and a held request would have
  timed out in the browser while the install succeeded. Verified: 62 Rust tests, 977 root bun
  tests, 215 dashboard tests, `svelte-check` 0 errors, 17/17 repo gates, and the panel present in
  the built bundle (`dashboard/dist/_app/immutable/nodes/23.*.js`). NOT verified: the panel
  rendering in a browser, because every route here needs the operator's credential and an agent
  session is refused one by design (403 `no inbound token is configured` on `/storage` too, which
  is pre-existing). That check is the operator's, and it is the one thing this ISC still owes.
  The apply button is narrower than the CLI on purpose: `APPLY_CLASSES` in
  `capabilities/sjel-status/src/status/updates.rs` omits `containers`, `checkout` and `vendor`,
  which already have owners, and the class is an allowlist because it becomes an argv element.

- [x] ISC-56 — the report never recommends an upgrade that would break a constraint, and never
  one the registry has deprecated. Falsifier: a row offering an action whose version another
  global's range does not admit, or whose package npm marks deprecated. Probe: `sjel update`
  against `npm ls -g --json --all` and `npm view <pkg> deprecated`. Evidence, 2026-10-01: the
  first version of this tool recommended three upgrades that were all wrong —
  `@mariozechner/pi-agent-core` and `@sinclair/typebox` are `^0.52.12` / `^0.34.48` requirements
  of `claude-agent-sdk-pi` and `@marckrenn/pi-sub-bar` (so 0.73.1 is not admitted, and npm's own
  `wanted` column cannot see this because the range belongs to a different package), and the
  whole `@mariozechner` scope is deprecated in favour of `@earendil-works`. Both are now rows
  with a reason and no action. The version-aware refinement came next: matching a parent's
  subtree by NAME conflated a hoisted pin with a leftover, so a parent that bundles its own copy
  now reads `unused duplicate` rather than `pinned by`.
- [x] ISC-57 — `tools/agent-integrations.sh update interceptor` updates the product and dates its
  marker. Falsifier: the verb exits non-zero, or the harnesses' marker keeps its old date after a
  successful run. Probe: `tools/agent-integrations.sh update interceptor` and the marker's
  contents. Evidence, 2026-10-01: the branch called `interceptor upgrade`, which on 1.0.19 is
  "Promote browser-only install to full computer-use mode (macOS)" and exits 1 without `--full`
  — so `interceptor skills`, the second half of the pair, had never once run, and the marker sat
  at 2026-09-10 through every "update". It is `interceptor update` (the product's own appcast),
  and the branch now writes the marker for each configured harness the way graphify's arm does.
  All four markers read 2026-10-01 after the fix. upstreams.toml [interceptor] said "its own
  update/upgrade verbs keep it current"; the half that was true is `update`.

- [x] ISC-58 — the newest STABLE release decides a crate's row, not the newest release. Falsifier:
  a crate whose newest stable is above the installed version while a pre-release sorts higher is
  reported current. Probe: `sjel update` against `https://crates.io/api/v1/crates/<name>/versions`.
  Evidence, 2026-10-01: `cargo search tauri-cli` answers only the maximum, which was
  `3.0.0-alpha.4`, so this tool called the crate current and hid the released patch `2.12.1`
  above the installed `2.12.0` — the fix is a crate that was already on this machine and had a
  one-line upgrade waiting. The full version list is now read (crates.io 403s without a
  User-Agent, measured), yanked releases are dropped, and a pre-release above the newest stable
  is still named on the row so the reader sees both facts. `cargo search` remains the fallback
  when crates.io does not answer.

- [x] ISC-59 — the audit reaches software installed outside this checkout, and every apply
  records what it found. Falsifier: an installed package no scan covers, or an `apply --only
  cargo` / `--only npm` whose receipt carries no verdict. Probe: plant a known-vulnerable global
  package and run `tools/audit`; then `sjel update apply --only npm --yes` and read
  `<overlay>/data/updates/last-apply.json`. Evidence, 2026-10-03: measured first that
  `osv-scanner` cannot do this itself — its `directory` plugin walked 1 dir and made 0 Extract
  calls on 2.6.0, against `/opt/homebrew/lib/node_modules`, against `/opt/homebrew/Cellar`, and
  against a hand-built `var/lib/dpkg/status` holding one package. A CycloneDX SBOM fed through
  `-L` works instead: a three-component fixture returned lodash 4.17.15's advisories and exit 1,
  the same fixture without it returned 0, and percent-encoded scoped names
  (`pkg:npm/%40sinclair/typebox@0.34.52`) resolve. The inventory is `tools/updates --json
  --offline --inventory`, whose two managers the report already reads — no second reader of
  `cargo install --list` or `npm ls -g --json --all` is added. `--inventory` had to be added to
  that tool, and not the default `rows`, because `rows` is the actionable view and stops at the
  top level: on this host 13 packages against 541 distinct name@version pairs (4 crates.io, 537
  npm), the nested 528 being where advisories mostly live. Coverage is therefore the full npm
  tree as a CycloneDX SBOM, plus, per installed crate, the `Cargo.lock` **published inside it**
  — copied from `$CARGO_HOME/registry/src` and matched by `name-version`, so an uninstalled
  cached version is not scanned. That lockfile is the resolution `cargo install --locked
  --force` uses, which is the command `tools/updates` plans; it is a claim about what a
  reinstall would produce, and may differ from what is on disk if the crate was installed
  without `--locked`. npm's half is the resolved tree actually present.

  **The first honest run found things.** 23 packages carrying 46 advisories — 3 Critical, 16
  High, 15 Medium, 7 Low, 5 Unknown, 36 fixable — none of which any scanner had seen, and
  `tools/audit` now exits 1 where the same run reported clean minutes earlier. `host-patch`
  therefore exits 1 too, which its own contract already means (`1 = the audit found something`,
  distinct from `2 = a step failed`). Nothing about this change blocks an install or an apply:
  the verdict is report-only, and this ISC is about reaching the surface, not about what to do
  with what it found — which is a decision this change deliberately leaves open.
  `apply` ends in `tools/audit` and writes the verdict to the receipt. Deliberate: the pass runs
  even when the plan already delegated to host-patch, because reading host-patch's receipt back
  would make the field mean "whatever ran last" and need a third branch for a missing one.
  `--verbosity error` silences osv-scanner listing `osv-scanner.toml`'s four entries as "unused
  ignores" on this pass, which otherwise appears directly under a repository pass that just used
  all four and reads as an invitation to delete them; the cost is that an exception applying to
  an installed package would be swallowed, and no current entry does. NOT verified: `--only
  cargo` or `--only npm` end to end through a real apply, because nothing on this host was stale
  at the time.

- [x] ISC-60 — `apply` can remove a leftover and can re-resolve a crate, and neither happens
  without the operator naming it. Falsifier: `--prune` removes a package a parent pins, or
  `--re-resolve` drops `--locked` for a crate not named. Probe: unit tests over `planPrune` and
  `planApply`; `sjel update report --prune` and `report --re-resolve x` refused with exit 2.
  Evidence, 2026-10-03: `Row.removable` is set on exactly two branches of `gatherNpm` — a
  registry-deprecated package and an unused duplicate — and never on a pinned copy, because that
  copy exists to satisfy its parent; the test asserts the pinned row is absent from `planPrune`
  and never appears in a removal argv. `apply --prune` prints the removals as their own block
  before the plan and before the existing `--yes` gate, removals are ordered first so a name is
  free before anything arrives under it, and `--re-resolve <crate,...>` drops `--locked` only for
  the crates named, printing the reason beside each and saying so out loud when a name matches
  nothing the plan moves. NOT verified: a live prune, because this host has nothing removable
  (`sjel update --json | jq '[.rows[]|select(.removable)]'` returns `[]` after the two add-ons
  came off), and a live re-resolve, because no crate was stale.

  The same change removed `@marckrenn/pi-sub-bar` and `claude-agent-sdk-pi`, and that was
  measured rather than assumed: both nested copies of `pi-coding-agent` were the bundled peer
  of one of them (`peerDependencies: "@mariozechner/pi-coding-agent": "*"` — the deprecated
  scope, with no successor package on npm — and `"@earendil-works/pi-coding-agent": "^0.74.0"`
  against an installed `1.0.0`), both parents were already at their latest release, and neither
  name appeared anywhere in this repository, the overlay, pi's `settings.json` or `mcp.json`,
  `~/.claude.json`, or `~/.codex/config.toml`. Removing them took 399 packages and 11 of the 46
  advisories with them: 541 installed packages became 439, 23 affected became 18. The other six
  globals nothing references were left alone on the operator's ruling, because `playwright/cli`
  and `bobshell` are plausibly invoked by hand and no configuration would show that.

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
- **One Rust agent for the Sjel app and local coding.** Recorded 2026-10-03, principal's
  decision: keep this as a candidate, no implementation yet. The intended destination is an
  agent replacement usable both inside the app and as a local coding agent; a household-only
  loop would not satisfy it. Pi is already adopted (`upstreams.toml [pi-coding-agent]`), so
  replacing it must earn its maintenance cost rather than follow from the language choice.
  Today `capabilities/assistant/src/main.rs` sends a system and user message to a local model
  and reads text back; it has no tool-calling conversation loop. Discovery, capability policy,
  approvals and pseudonymization already exist (F8–F10). MCP dispatch and its registration are
  both in the Rust `tools/sjel-mcp` crate since 2026-10-04; there is no TypeScript in the MCP path.

  Candidate design to evaluate: one headless engine for model exchanges, tool calls, bounded
  context and session events, with separate household and coding tool profiles. Household
  access must retain the declared capability contracts and their authentication, rewriting,
  approvals and audit; importing domain stores into the engine would bypass that boundary.
  The coding profile needs workspace file and command tools, with an explicit execution and
  data-egress boundary. Coding access must not silently give the app, or household tool output,
  unrestricted shell or filesystem access. Rust does not establish either boundary by itself.

  Before promotion: name the Pi features the replacement must retain, including a decision
  on sessions, context folding, skills, subagents, MCP, provider coverage and terminal UX.
  Prove one useful multi-step household task and one local coding task against those needs,
  with observed completion, latency, memory and operating cost compared with the existing
  path. The probe must also exercise malformed tool calls, oversized results, budget limits,
  injected tool text, cancellation and interrupted side effects; an uncertain write must not
  be reported as safely aborted or blindly replayed. No claim of a tiny line count, automatic
  process cancellation or zero disclosure is accepted from the proposal. The unresolved
  privacy probes in ISC-39 and ISC-45 remain unresolved. Failure to meet either use case, or
  replacement work that costs more than its measured benefit, is grounds to keep Pi and narrow
  or retire the candidate. No new UI, storage migration or Pi retirement is authorized here.
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
  included.
- [x] A2 — no issue is closed whose content exists nowhere else. Falsifier: ISC-2 fails
  for any closed issue.
- [x] A3 — the public repo keeps a path for outside bug reports. Falsifier:
  `.github/ISSUE_TEMPLATE/` is deleted or issues are disabled repo-wide.

## Decisions

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
  `tools/storage` measures it and enforces R6; nothing showed it to a person. Recorded as F7,
  including the tool's own blind spot: a second Cargo target dir is invisible to
  `prune --target`.
- **2026-08-20 — three capabilities never read their database variable, and the demo is
  what found it.** `tools/demo-up`'s whole mechanism is one exported
  `SJEL_<CAP>_DATABASE_URL` per capability. comms, scouting and transit ignored it, went
  from their config file to `postgres_conn_from_shared_env`, and — the demo overlay having
  no `postgres.env` — landed on a fallback naming the real database, `dbname=axon
  password=axon`. The only thing between a demo seeding run and the live store was that the
  real password is not the word `axon`. Fixed with one shared
  `sjel_config::database_url_override`, used by those three. The four that hand-roll the
  same two lines (calendar, finance, tasks, trips) went with PostgreSQL (PRD Q45).
- **2026-08-20 — `upstream-checker` published the checkout's absolute path.** Its `--json`
  `manifest` field was `$SJEL_ROOT/upstreams.toml`, which sjel-status serves and the demo
  records. `tools/check-site-payload` refused to publish over it, which is the job that
  gate has. Now repo-relative.
- **2026-08-19 — the backlog moves from Issues to ISAs** (principal's call). Migrate
  first, then close; change the doctrine in all four places that state it; stop the one
  workflow that creates issues.
- **2026-08-19 — `upstream-watch` reports to the job summary and reds the run** rather
  than being deleted. Deleting it would leave drift findable only when someone looks.
- **2026-08-19 — `.github/ISSUE_TEMPLATE/` stays.** Sjel is public and an external
  report still needs somewhere to land; what changed is that our own backlog is not there.

## Log

Entries before 2026-10-08 are in [`ISA-archive.md`](ISA-archive.md#log).

- 2026-10-08 · Review of every ISA against the tree. Three claims changed state. ISC-65 is done:
  its routes left in b10e5260, and the box was not ticked. ISC-22 and ISC-25 are reopened,
  because their evidence measured a proxy (see each claim). Eight closed features, their test
  rows and the log moved to `ISA-archive.md`. "Operator checks" lists the one action each
  operator-bound claim needs. Problem, Vision and Goal now describe the repo-wide record that
  this file became, not the tracker migration (F0) that this file was created for.
