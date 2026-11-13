# Contributing to Sjel

Sjel accepts changes that improve the reusable public shell. Personal data and deployment state
stay in a private overlay. The same boundary covers credentials, private host details, and
operator-specific policy.

## Before writing code

Name the consumer and the outcome. An interesting technology without a concrete Sjel consumer
remains an idea, not an implementation commitment.

Use GitHub Issues for concrete work that a maintainer or contributor can test or complete. Name
the consumer, outcome, and acceptance check. Use `type:idea` for experiments that may conclude
not to build.

Open work that is not yet an issue lives in [`PLAN.md`](PLAN.md) and the area plans in
[`docs/plans/`](docs/plans/). Move enduring behavior into code, tests, or the owning docs. Comments
explain local invariants, not dated decisions. Git keeps the history.

Before external code or adopted design influence enters the tree, record its canonical source in
`upstreams.toml`. Record the license and verdict there too, then state precisely what Sjel
adopts. Record no version: every dependency tracks its upstream's latest release
([Patch first](#patch-first)).

## Work on one change

Until `PLAN.md` is done, every change goes straight to `main`. Branches, worktrees and pull
requests start when the plan is done, independently of release tags. Rules: `Packs/harness/skills/sjel/references/on-changes.md`.

Keep the diff inside one coherent boundary. Put reusable code and doctrine in Sjel, and use
synthetic fixtures for data-shaped tests. Never copy an active overlay or a secret value into
public work. Keep workstation paths and private logs out of commits and GitHub text, including
screenshots and test failures.

Run `tools/doctor` before editing and record unrelated or machine-only failures separately. A
fresh source checkout does not need a real private overlay for CI; repository tests use synthetic
configuration where a machine contract is required.

## Validate the changed boundary

Run `sjel check` (`tools/ci-local fast`) before pushing; the pre-push hook runs it too. It checks
formatting in affected Rust workspaces, publication hygiene and affected cheap repository gates.
Tests, Clippy and builds run in CI rather than being repeated in the push hook.
`SJEL_SKIP_PUSH_CHECKS=1` skips formatting and selected gates for one push, but does not skip
secret scanning or publication hygiene.
Inspect the diff, `git diff --check` and `git status --short` before committing. Do not describe
a skipped or unavailable check as passing.

### On the go

On battery, the runtime profile defers local compilation, Clippy and test suites. `tools/cargo-hermetic`,
the CLI rebuild in `tools/lib/sjel-cli.sh`, `tools/ci-local run` and the pi command-policy
extension refuse a start with the reason; nothing already running is killed. Profile and exceptions:
`crates/runtime/README.md`. One command may proceed:

~~~sh
SJEL_ALLOW_LOCAL_BUILD=1 tools/cargo-hermetic test -p sjel-runtime --locked
~~~

Fast editing checks still run: formatting, publication hygiene and cheap repository gates compile
nothing. A checkout with no built `sjel-cli` cannot consult the profile, says so, and admits the
work; a `cargo` command typed outside these tools is also outside the guard.

### Change-aware CI

`tools/ci-plan.ts` selects verification inputs for local checks and Actions. Rust selection
includes package owners, runtime-path readers and reverse dependencies. Bun and UI selection
follows imports and package dependencies; unresolved runtime inputs widen coverage. Repository
gates declare their input boundaries in `tools/lib/ci-inputs.ts`. Every plan retains publication
hygiene. `tools/ci-run.ts` executes selected tests, gates and package builds.

A main push compares against the last successful ancestor run of its workflow, not the previous
push. Failed or cancelled pushes therefore remain in the next comparison. A missing base,
planner change or selection failure selects full verification. Actions summaries record the
base, selected and skipped surfaces, reasons and recent full-verification history.

The CI workflow runs full verification daily at 03:23 UTC, on published releases and on manual
dispatch. Distribution preparation calls full CI at the caller revision before building
artifacts. `tools/release` requires a successful full-verification result for the exact commit;
an ordinary selective success is insufficient. Security and CodeQL retain weekly full scans
because advisory databases and query packs change independently of source files.

Successful local format checks can be reused when checkout contents, the command and modeled
toolchain inputs match. Publication checks, tests and builds are never reused as format receipts.
`tools/ci-local fast --full` bypasses reuse; it still defers tests and compilation to CI.
`bun tools/ci-plan.ts --base origin/main --local` prints the current selection without running it.

### Rust

Rust packages are members of the root `Cargo.toml` workspace and share the root `Cargo.lock`.
Keep each package's direct dependencies in its own manifest, and add no nested lockfile. The
full set, from the repository root:

~~~sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --locked
~~~

These use the channel and components `rust-toolchain.toml` names, as CI does. Do not answer a
lint with a workspace-wide allowance. A narrow allowance goes beside the affected item and
names the invariant that makes the lint inapplicable.

`cargo test` needs no database service and no environment variable. The database-backed suites
share the module name `db_tests::`, and each test opens its own temporary SQLite file:

~~~sh
cargo test --workspace --locked -- db_tests::
~~~

`tools/cargo-hermetic` isolates the local overlay, writable data and build output, and refuses
release builds. For a run with less memory and contention:

~~~sh
CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2 tools/cargo-hermetic test --workspace --locked
~~~

Temporary test files live inside the run's sandbox and are removed after Cargo exits, including
failed tests. Build output stays outside that sandbox in a reusable cache keyed by checkout.
Nested invocations retain the resolved cache rather than adding another checkout suffix.
An inherited cache that cannot create and remove artifact directories falls back to the shared
temporary cache; an explicit `--target-dir` is refused instead of silently redirected. Reuse the
default cache for ordinary verification rather than creating a target directory per command.

### Other checks

- `tools/maintenance --json` inventories tracked maintenance inputs offline. Add `--unused`,
  `--freshness` or `--licenses` for optional read-only analyzers. Missing requested coverage
  exits 2; license inventory covers the current Rust host target and is not legal approval.
- `sjel gates` and `sjel test` replay CI's repo gates and Bun job from the workflow file.
  `tools/ci-local list` shows which jobs can be replayed.
- A package that `tools/discover-ui-packages` reports: run `bun run check`, and `bun run build`
  where it declares one. CI runs both.
- A manifest change: run `tools/generate-architecture.sh`, then
  `tools/check-architecture-fresh.sh`.
- A test that needs a platform resource (`/dev/full`, a container runtime) uses `skippable` from
  `tools/lib/test-support.sh`. It reports omitted coverage locally and fails in CI.

A performance claim names a probe this repository can re-run: an ignored test beside the code it
measures, or a script under `tools/`.

### Pitfalls

- **`SJEL_DB_PATH` isolates the database only.** Vault projections (`comms`, `finance`, `trips`)
  resolve their roots from overlay configuration. When you test against a copy, also export
  `SJEL_TRIPS_EXPORT_ROOT`, `SJEL_FINANCE_DECISIONS_ROOT` and a scratch `SJEL_COMMS_CONFIG`.
- **Do not `cargo build --release` in a worktree that shares the root `target/`.** That build
  overwrites `target/release/<bin>`, the binary `tools/service-runner.sh` supervises. Build debug
  binaries there, or set an isolated `CARGO_TARGET_DIR`.

## Open the pull request

Open a draft pull request first. State the outcome, then bound the exact scope. List every
completed validation command with its result and name the known limits. Keep separable follow-ups
out of the pull request rather than widening it. A pull request lands as a squash merge: one coherent
change per commit on `main`, easy to review or revert.

Security findings follow [SECURITY.md](SECURITY.md); never report one in a public issue or pull
request.

## License and sign-off

The project is licensed under the GNU Affero General Public License, version 3 only
([LICENSE](LICENSE)). Anyone who runs a changed version as a service for other people must
offer those people its source. Code that `Packs/` vendors from other projects keeps its own
license; each such Pack reproduces the upstream notice in its `LICENSE` file.

A contributed commit carries a Developer Certificate of Origin sign-off
([developercertificate.org](https://developercertificate.org/)): a `Signed-off-by:` line that
states you have the right to submit the change under this license. `git commit -s` adds it. A
sign-off does not transfer copyright: a contributor keeps it in their change. A later license
change therefore needs the consent of every contributor whose code remains.

Releases before 2026-09-26 were published under the MIT license and stay available under it.

---

# Engineering doctrine

The rules below govern every change to this repository. Detailed procedures live in the linked
technical documents; each rule has one owner. Code comments and READMEs link to that owner.

## Product rules

Every change keeps to these. Rules 1, 4 and 5 describe the target behaviour; the README's
[State](README.md#state) table shows how much of it is built.

1. A task runs on the best model the device can reach: its own Apple model, then the Mac's, then
   deterministic rules. The result shows which one answered.
2. A paired device's key admits its requests. Every connection type carries the same signed
   requests.
3. Sjel detects what the devices can do and chooses the defaults. Other options are under
   Advanced.
4. C2 data leaves the owner's devices only end-to-end encrypted, with keys on those devices, or
   pseudonymized. A cloud request carries only the fields its task needs (Staab et al.); ambient
   metadata, protocol headers, and extraneous history are stripped. A test proves each path. A
   failing test closes the path.
5. Summaries and labels appear without a confirmation. Confirmations protect only while they are
   rare enough to be read (Akhawe and Felt); frequent prompts cause habituation and click-through.
   A reversible change applies by itself only after a frozen test set shows it is reliable. A change
   that leaves Sjel, or cannot be undone, asks. The prompt rate on routine flows is measured and
   kept at zero.
6. Screens and assistant tools come from the typed data.
7. A statement about the system cites the file, command or measurement that proves it.

## Start here

Sjel supports macOS and Linux. The guided installer detects the platform, creates or connects a
private deployment overlay, writes the local machine manifest, and leaves existing state alone.

Follow [the shell-only installation workflow](website/content/operations/index.md#install-a-shell-only-node)
for prerequisites, source setup, capability selection, authentication, build, startup and reboot
verification. The installer reports missing tools rather than installing them silently.
Contributors who only need source and tests can skip operating setup and start at
[Before writing code](#before-writing-code).

## Operating values

These are design tests, not aspirations. A value only counts when it changes a placement,
contract, install, or review decision.

1. **Lean runtime, growing monorepo.** A fresh install enables a small useful core. The repo
   may hold far more — optional capabilities, Packs, public reference projects — and none of
   that may become a runtime prerequisite.
2. **One feed, many views.** Capabilities keep domain ownership, but publish typed events with
   provenance, time, confidence and state. The app sorts and connects those events; it does not
   become another data silo.
3. **Data and mechanism never blur.** Sjel owns public code, schemas, renderers and explicitly
   public first-party datasets. The active private overlay owns private content, secrets, machine
   configuration and history. Both sides use the same bounded
   contracts: data may select an allow-listed behavior, never become executable code.
4. **Evidence before automation.** Every surfaced claim keeps its source, evidence boundary and
   decision state. Agents rank and explain; anything irreversible still goes through the
   capability contract, and leaves a trace.
5. **Security is continuous observation.** An audit gate is a start, never the property itself.
   Dependencies track their upstream's latest release ([Patch first](#patch-first)). Bun/npm
   packages maintain a 24-hour resolution window to mitigate rapid publish-compromise attacks.
   Egress, access and agent touches stay observable; telemetry lives in the private overlay.
6. **Replaceable edges, stable contracts.** Agent harnesses, model providers, visual renderers
   and deployment substrates are adapters. Adopt first and record why. Re-open the call when
   its stated flip condition comes true.
7. **Self-hosted by default, distributed when earned.** One home-server deployment should serve
   phone and desktop clients. Kubernetes, WASM or a native shell enter only for a measured
   portability, isolation or performance need.
8. **English is the shared surface.** Sjel-authored interfaces, documentation, prompts,
   summaries, explanations and errors default to English. Source material and explicitly
   locale-specific capabilities may remain multilingual; accepting multilingual input must not
   silently change Sjel's output language.

### Principles cited by number

Code and READMEs cite "Principle N" by the number the frozen PRD gave it. Those principles bind
like the values above:

- **Principle 1, cheapest mechanism first.** Logic, then algorithms, then ML, then AI. A rule
  beats a model. Each capability names the lowest rung that does its job and justifies a higher
  one.
- **Principle 2, privacy by data class.** What the data is decides its route, not whoever wrote
  the call site. See [Data classes](#data-classes).
- **Principle 3, one fact, one owner.** Each field is maintained by hand or derived, and has a
  declared owner. A fact stated in two places without one is drift.
- **Principle 5, evidence before automation.** Operating value 4.
- **Principle 8, human-readable at rest.** Markdown, JSON, YAML and plaintext journals are the
  ground truth. A database is a rebuildable index, never the only copy.
- **Principle 9, cut what stopped earning its place.** A folder, field, capability or rule that
  stopped earning its place is removed, not documented around. When it conflicts with another
  principle, this one wins.

## Feed, Scouting, and Obsidian boundaries

“One feed” means one general observation stream, not one opportunity engine. A feed item is
anything the operator may need to notice: a security advisory, system or package update,
change in a watched repository, news item, useful article, scholarship, hackathon, event, or a
future source that fits the same typed observation contract. Each item keeps its source,
observed time, evidence link, kind and decision state. The originating capability keeps any
domain-specific payload.

`comms` currently supplies the feed store, manual link ingest and mail triage. A new source
belongs in the general feed when its first job is awareness or reading.

Mail triage uses deterministic rules with an optional local model fallback over unresolved threads.
It reads the sender's domain, redacted subject, and preview, refusing Secret mail before prompt
assembly. Proposals that would raise a mail's data class are held for operator review.

Personal relevance is a Feed annotation, not a new ownership boundary. Comms scores items
against explicitly configured focus notes, storing matches separately and labeling whether the
comparison was semantic or lexical. Sorting combines focus fit, upcoming travel, freshness, and
operator keep/dismissal history. Ranking inputs re-evaluate from stored matches rather than
re-prompting models. `/feed/[id]` provides a unified dynamic reader across all item kinds.

The app shows passive intake and active discovery as two views of one `/feed` workspace.
That is a UI integration and never a database merge: each capability keeps its own contract and
provenance.

Narrower by design, `scouting` searches for and scores opportunities such as scholarships,
hackathons, events, calls for papers, or travel deals against an interest profile.

Obsidian holds learning notes only: what `teach` and `practice` write, and subject knowledge
(`PLAN.md`, Decisions, 2026-11-08). It is not a Sjel database. Operational records live in each
capability's store; notes that belong to a record live in `projects`. The markdown adapters in
`comms` and `scouting` read any configured directory and no longer point into the vault
([`storage.md`](docs/plans/storage.md#one-vault-for-prose)). Integrations never mutate
another capability's notes by implication.

Lineage: Sjel's public-core and private-overlay separation draws inspiration from upstream AI-operator
patterns (see `upstreams.toml` [lifeos]).

## Architecture and ownership

### Public core and private overlays

Sjel contains public code, schemas, renderers, and engine logic. Private data, vault contents,
backups, and deployment configuration live in the active overlay. See [website/content/architecture/security.md](website/content/architecture/security.md).

Mechanism stays in Sjel; instance inventory and values stay in the overlay.
`tools/check-publication-hygiene.sh` gates against committed workstation paths and deployment
markers.

### What the installer owns in an agent harness

Agent configuration directories (`~/.claude`, `~/.codex`, `~/.pi`) belong to the operator.
Installer authority is strictly additive:
- `tools/claude-code-config/claude-code-config` merges Sjel baseline settings into `settings.json` without
  removing or clobbering existing operator keys. Refuses unparseable files rather than overwriting.
- Drift is detected and reported via `sjel claude check`.
- Packs are offered rather than force-linked during installation.

### What survives a harness swap

A harness is an adapter (operating value 6), so a swap must keep everything below:

| Must survive | Where it lives |
|---|---|
| The operator's preferences and principles | `crates/operator-profile` |
| Long-term memory | The vault |
| Skills and know-how | `Packs/` |
| Tool contracts | `schemas/` and capability manifests |
| Conversation history | Agent state outside the harness |
| Scheduled jobs | Sjel, never the harness |

A swap may lose the chat UI, the exact prompt format and provider-specific caching.

### Integrate-first topology

Personal and self-authored components integrate into Sjel by default. Standalone repositories are
reserved for independent products with distinct lifecycles, external collaborators, or distinct
overlays. Integration occurs via declared contracts and state mounts.

### Three architectural nouns

Sjel organizes all code into three nouns:

- **Spine**: The core runtime and developer surface: root manifests, `schemas/`, `tools/` (scripts),
  `crates/` (shared crates, apps and tools), the `sjel-status` gateway and the apps in `apps/`.
- **Capability**: Autonomous domain units declared via `service.toml` (Rust crates under `crates/<name>/`,
  supervised third-party software and scripts under `services/<name>/`).
- **Pack**: Modular agent know-how and workflows under `Packs/<name>/`.

See [website/content/architecture/index.md](website/content/architecture/index.md) for full topology and boundary rules.
Shared Rust code belongs in `crates/`; third-party services in `services/`; contracts belong in `schemas/`; operator tools belong in `tools/`.
Do not introduce unstructured `utils/` catch-alls.

### Schemas and dependency direction

`schemas/` defines shared contracts: import them, never redefine them. Dependencies point downward:
Packs -> Spine -> Capabilities -> Libraries (`libs/`).
Capabilities interact with other capabilities via loopback HTTP contracts and declared schemas, never
via direct database mutation.

The apps are presentation: they read and write through capability contracts and own no domain
state.

## Dependencies and build

### Upstream-first

Choose in this order: adopt, contribute upstream, configure or adapt existing source, then
build. Decide explicitly whether Sjel consumes an upstream product or maintains an adaptation.
Use the upstream installer for a product Sjel only consumes. A locally maintained adaptation
belongs in Sjel when its behavior and integration are part of Sjel's work; preserve provenance,
licenses, the reviewed upstream revision and the reasons for local changes in its owning README.
Review upstream changes selectively and preserve local behavior during upgrades.

Discovery does not authorize adoption. Offer unselected extensions and skills as optional
upstream installs; do not copy or enable them merely because they were found. Conversely, an
explicit decision to maintain source locally is authorization to improve it here, not a demand
to replace it with a stock installation. The harness Pack records the current ownership choices.

### Cargo and bun are the build path

`cargo` builds and tests the Rust workspace: one root workspace, one `Cargo.lock`, with each member
manifest managing its own direct dependencies. A service manifest declares its `target/release` binary,
and `tools/service-runner.sh` compiles it on demand. `bun` powers TypeScript tooling and UI bundles.
Freshness of generated artifacts is enforced via `tools/check-architecture-fresh.sh`.

Operator machinery is unified in `tools/sjel-cli` and `tools/storage` in Rust for fast parallel execution,
with lightweight wrapper scripts maintaining stable paths.

Build artifacts are ephemeral. `sjel storage target` bounds debug-to-release ratio (`target/debug` within
3× of `target/release`), verified by `tools/doctor`. Space is reclaimed via `sjel storage reclaim --repo`.

### Implementation languages and intelligence

New backend logic defaults to Rust. Choose the lowest rung that can solve the problem and justify
every move upward: heuristic, algorithm, classical machine learning, local AI, cloud AI. A model is
not a substitute for a deterministic baseline it has not beaten.

### One manifest per concern

Configuration has one owner per concern (see [website/content/capabilities/manifest.md](website/content/capabilities/manifest.md)):

| Manifest | Owns |
| --- | --- |
| `sjel.toml` | Shared platform defaults and the shipped overlay fallback; never machine-specific state |
| `sjel.local.toml` | This checkout's active overlay location; gitignored and written by the installer |
| `upstreams.toml` | External code and adopted influence: url, verdict, license and why. No version — every entry tracks its upstream's latest |
| `toolchain.toml` | Host executables Sjel commands assume, with requiredness, scope and install hints |
| `systems.toml` | Systems, services and projects that have a role in the setup |
| `<overlay>/config/machine.toml` | OS, container runtime, enabled capabilities and state mounts for this machine. An overlay owning several machines uses `<overlay>/config/machines/<name>.toml` instead, selected by `sjel.local.toml` or the hostname |
| `<overlay>/config/deployment.env` | facts true of the whole deployment rather than one machine or one capability — the home timezone, and `SJEL_INBOUND_TOKEN_FILE`, the reference to the shared secret every capability server authenticates inbound requests against (`libs/sjel-server/README.md`). Declared once because several capabilities need it and independent copies drift silently (`schemas/deployment.env.example`) |

`tools/lib/toml.sh` is the parser for single-line scalar and array fields. More complex TOML goes
through the shared Bun parser; no caller grows another partial parser or duplicates manifest data.
Event sources follow the same pattern: declared configuration, never values spread through code.

Host requirements are scoped, not global. `toolchain.toml`'s `needed_by` says where an entry
applies: absent means every machine, `workflow:<name>` means only when that workflow is asked
about, and `capability-field:<field>` derives from the enabled set so the requirement follows the
capability when it moves host. A runtime node is therefore never told to install a scanner it has
no path to invoke, and a tool that is out of scope is reported as `n/a` naming what would pull it
in rather than omitted. Run the scoped check after changing what a machine is enabled to do, and
run it for a workflow before invoking one:

```sh
tools/toolchain-check                    # this machine: core + enabled capabilities + runtime
tools/toolchain-check --workflow restore # before a restore, not after it holds a service down
```

### State mounts record reality

Sjel does not relocate an adopted tool's data directory. The active machine manifest records where
the tool really persists data, its class, sync policy and direction. Backup and monitoring walk
that registry. Dotfiles and shell configuration are state mounts with injection direction, not
special cases.

## Security and data

### Dependency verdicts and provenance

Every consumed external dependency gets a verdict in `upstreams.toml` first. **No entry, no
entry.** The manifest records, per upstream, the URL, the verdict, the licence and the `why` that
argues for it — four fields and nothing else. The consuming README also records what Sjel
adopted: runtime, idea, architecture, algorithm, code or asset. A local clone or archive path is
never durable provenance.

**The register records no version.** Every dependency tracks its upstream's latest release:
`services/host-patch` moves brew, uv and rustup nightly, `services/container-refresh`
moves the images daily, Dependabot moves the Cargo and Bun manifests, and the workflows install
`latest`. An audit date belongs in the `why`.

| Question | Answered by |
|---|---|
| Is there a verdict, a licence and a reason? | `upstreams.toml` itself, read by a human at review time |
| Is a newer release out? | Dependabot version updates — `.github/dependabot.yml`, one grouped pull request per ecosystem per day; Cargo and actions use 0 days, Bun a one-day window |
| Is a locked dependency known-vulnerable? | Dependabot alerts and security updates, and `osv-scanner` in `.github/workflows/security.yml` and in `tools/audit` |
| Is software installed outside this checkout known-vulnerable? | `tools/audit`'s second `osv-scanner` pass: the full global npm tree as a CycloneDX SBOM, plus each installed crate's own published `Cargo.lock` (the resolution `cargo install --locked` uses). Nothing else covers it — Dependabot reads lockfiles in this repository, and `osv-scanner`'s `directory` plugin extracts nothing from installed software |
| Is there a CVE in a declared capability image? | `grype registry:<image>:<tag>` in `security.yml`, weekly. Findings are report-only and land in the run summary; the job goes red only if it discovered no image to scan. No container runtime and no local pull |
| Is there a secret in this repository? | GitHub secret scanning, with push protection — and `gitleaks` in `tools/audit`, which reads history the push protection never saw |
| Is there a secret in the private overlay? | `tools/audit` alone. GitHub charges for secret scanning on a private repository, and this repository's CI cannot reach the overlay, so this is the one scan that has to be local |
| Is there a flaw in the code as written? | CodeQL — `.github/workflows/codeql.yml`, `security-extended` over rust, javascript-typescript, python and actions |
| Is a host package behind? | `sjel update` — every class of software installed outside this checkout, with the owner of each named and the stale ones listed. `services/host-patch` moves `brew`, `uv` and `rustup` daily; `sjel update apply` moves the two classes nothing else owns (`cargo install`ed crates, global npm packages) |
| Is a running container behind its channel? | Nothing asks. `services/container-refresh` pulls every declared image every day and recreates what moved |

`upstreams.toml` is documentation. No script enforces or reads its fields, except `tools/self`,
which publishes each name and verdict into `self.json`. A human reads a verdict before adopting a
dependency.

**Two gaps.** No scanner covers shell: CodeQL has no shell extractor, and `tools/` is the largest
hand-written surface. A malicious release is covered unevenly: CI runs Socket's scanner over each
Bun package that `tools/discover-ui-packages` finds, and every other install path relies on its
ecosystem's own controls and on Bun's lifecycle-hook blocking. The scanner's free mode does not
replace the age window; see [Patch first](#patch-first).

### Patch first

Take upstream patches under the [patch policy](website/content/operations/index.md#patch-first), including
the Bun/npm age window, update ownership, receipts, and accepted risks.

### Secrets

Secret values live in Vaultwarden, never in this repository or an overlay note. A capability may
materialize a real value only into its gitignored runtime environment because the service consumes
plaintext environment variables. Creating or changing one requires the user to run
`tools/setup-secret.sh` interactively after explicit, specific approval. A general “continue” is
not authorization. A pre-Vaultwarden bootstrap exception must be argued when a real need exists;
none exists today.

### Data classes

Follow the [data-class and model-admission policy](website/content/architecture/security.md#data-classes).
The shared admission functions own the gate; labels and callers must agree with it.

### Backups before migrations

No data migration starts without a tested 3-2-1 backup. Follow the
[recovery rehearsal](website/content/architecture/storage.md#backups-before-migrations) before changing live
state. Delivery receipts alone do not prove recovery.

## Tooling conventions

### Portable shell

Shell scripts remain compatible with macOS Bash 3.2. Do not use associative arrays, `mapfile`,
`readarray` or another Bash 4-only feature unless the script owns and verifies a newer runtime.

### Language tooling

Use `uv` for Python and `bun` for TypeScript. Do not add `pip`, `npm` or bare `node` commands to
Sjel code, launchers or documentation. The runtime choice belongs in `toolchain.toml`; an external
package consumed as code also belongs in `upstreams.toml`.

### Capabilities are data

A container-backed capability is a `service.toml` consumed by the shared service runner and
watchdog, not a new lifecycle script. Before adding a command, ask whether the behavior belongs in
the manifest. A user-facing command shipped by one capability lives with and is named after that
capability; implementation helpers stay off `PATH`.

### Dynamic paths and current facts

Resolve repository, overlay and platform facts through `tools/lib/paths.sh` and
`tools/lib/platform.sh`. Do not duplicate absolute paths or personal directory names. The only
unavoidable bootstrap is the initial `SJEL_ROOT` shell setting before any shared resolver can be
found.

Ports, enabled capabilities, health, issue state, test counts, graph size and machine state come
from manifests and tools at runtime. Prose may keep historical measurements and constants that are
part of an argument, but it must not claim a changing count that no gate verifies.

### Public CLI

`sjel` is the public command interface for humans and agents. Run `sjel help` to discover
operations and `sjel search <task>` to narrow the current capability and Pack surface without an
installed agent skill. Repository policy lives in `AGENTS.md`; command help and capability or Pack
contracts own operational detail.

There is deliberately no separate CLI reference. `docs/sjel-cli.md` was one, and every row of it
restated something `sjel help` already prints — a command table, the harness names, the discovery
instruction `AGENTS.md` carries verbatim. A second copy of a generated surface is the kind of doc
that rots first and is believed longest. `tools/install.sh` owns the installation contract: it
links `~/.local/bin/sjel` to the tracked launcher, never overwrites a non-Sjel command at that
path, and reports the exact shell-path action when `~/.local/bin` is absent from `PATH`.

## Releases

### The release line

The public spine versions with SemVer, not CalVer. A tag is `vMAJOR.MINOR.PATCH` with all three
components present — `tools/release` refuses any other shape — and `tools/lib/version.sh` orders
tags with `sort -V`.

- **major** — a public contract or architectural shape changes such that an existing overlay needs
  edits to keep working.
- **minor** — a capability or operator-visible feature lands.
- **patch** — fixes only.

Judge a change against those three before opening a pull request. A contract change that ships as a
patch is what breaks an overlay on an update it was told was safe.

Deployment overlays stay untagged. An overlay is deployment state rather than a public release
line, and its commit identity stays independent of the Sjel version it runs against.

### Cutting and consuming a release

Tags are created through `tools/release`, never by hand. It gates on a clean tree, on `main`, not
behind `origin/main`, and a successful CI run on the exact commit, then generates notes from the manifest-aware
delta in `tools/lib/delta.sh` — the same view `tools/update.sh` shows a consumer, so release notes
and the incoming preview cannot drift apart.

A checkout moves along the line with `tools/update.sh`: fetch, fast-forward, re-run `tools/doctor`.
It never resets, rebases or force-pushes; a checkout that is both ahead and behind is left alone
with instructions rather than silently repaired.

### Getting onto the line

`bootstrap.sh` clones a release and hands off to `tools/install.sh`. The `usage` profile uses
`--depth 1 --branch <tag>`; `development` keeps full history. Fetch the bootstrap by a chosen
release tag rather than executing a moving branch. `tools/bootstrap.test.sh` exercises both
clone shapes, installer handoff, refusals and truncated downloads against a local test remote.

A usage checkout can become a development checkout without reinstalling. Restore the branch
fetch specification before deepening the tag-only clone:

~~~sh
git config remote.origin.fetch '+refs/heads/*:refs/remotes/origin/*'
git fetch --unshallow origin
~~~

Both profiles retain the source tree: tools resolve `SJEL_ROOT` from it. The bootstrap chooses
the clone shape; it does not provide prebuilt services or complete authentication and startup.

**A release tarball instead of a clone was rejected.** `tools/update.sh` updates by fetch and
fast-forward. A tarball would require a second update mechanism, including removal of files
deleted upstream. Historical clone measurements do not establish the current installation size;
use the [footprint method](website/content/operations/index.md#installation-footprint).

Source checkouts retain Git for release identity, updates and repository metadata. The consumer
distribution target is now prebuilt apps and a minimal node, with optional capability packages;
it must not require Git or build tooling on the user's machine. `distribution.json` and the
release catalogue carry the packaged runtime's identity. The remaining acceptance probes live
in [#235](https://github.com/larsboes/Sjel/issues/235), and [release preparation](website/content/operations/index.md#release-preparation-and-later-testing)
does not establish that the source-free installation is ready.

The agent-readable install page is the primary route and the one-liner is the terminal alternative.
Sjel's install contains real decisions — overlay location, container runtime, capability selection,
secrets — and an agent walking those with a permission gate per step beats a script asking the same
questions blind. Both land in the same `tools/install.sh` prompts.

## Packs and agent harnesses

### Harness-neutral Packs

`Packs/<name>/pack.toml` plus `skills/` is the neutral source for a togglable agent bundle. Harness
adapters translate that source into their own installation format; harness-specific metadata never
changes the neutral manifest or forks the canonical `SKILL.md`.

### Proportionate approvals and delegation

Repeated approvals for the same routine operation are a policy defect to investigate, not a
reason to keep asking. Inspect the matched rule and actual side effects. Prefer a narrow,
reviewed permission for the intended command and scope, with regression cases for harmful
variants. Do not disable a guard, broaden an entire tool, or weaken secret, destructive,
publication or data-class boundaries to reduce prompt count. Distinguish authorization cards
from product decisions that still require the user. The command-policy cases and the agent
plan own policy changes and their prompt-rate evidence.

Delegate only when an independent investigation, parallel implementation or verification
justifies its startup, context and reconciliation cost. Use direct tools for known targets;
do not repeat a delegated search in the parent. Before expanding delegation, and after failed
or redundant runs, compare usable findings and avoided work with elapsed time and reported
usage. Reduce agents or work inline when delegation did not help. Change canonical harness
instructions when repeated waste points to an instruction defect; retain independent review
where it provides evidence the author cannot supply. No fixed agent count substitutes for this
judgment, and no per-agent report is required for routine successful delegation.

### Owned agent components

Skills, agents, prompt workflows, hooks and harness plugins have two ownership forms:

- **Sjel-owned behavior:** components authored, forked, vendored or redistributed by Sjel are
  maintained locally, with a contract and checks. Adapt a justified behavior into its existing
  owner. Renaming, copying, screening or storing a package does not establish ownership.
- **Reviewed native dependencies:** unmodified upstream-managed components may be approved for
  their supported host or vendor integration, including community-maintained components. Their
  native installer and upstream maintain the package; Sjel owns only its local configuration or
  integration. Copying their behavior into Sjel or another harness requires an owned adaptation.

Native approval is component- and host-specific, after proportionate review of intended use,
licence, permissions, execution, data access, egress and cost. Record the decision and constraints
in `upstreams.toml`. Marketplace membership, installation or a familiar publisher is not approval;
activation requires a separate decision. Source-porting and native-use verdicts are distinct.

Delegate routine native updates to upstream within the approved trust boundary. Material changes
to publisher, permissions, execution, egress or licensing require re-review before accepting the
expanded boundary; not every prompt edit requires a revision gate. Upstream updates never
replace locally maintained Sjel sources automatically.

Preserve attribution and every applicable license obligation, including notices and source
requirements. Attribution alone is insufficient. Stop an adoption when permission or license
compatibility is unresolved. Runtime libraries and host applications remain dependencies; this
rule governs the agent components they supply, not ownership of every underlying tool.

The adoption procedure and migration boundaries live in
[`Packs/harness/README.md`](Packs/harness/README.md#upstream-adoption).

### Public skills

Skills stay public and contain no host, IP address, key, personal path or private preset. They
resolve instance values from the active overlay at runtime.

### Skills drive capabilities

A skill is a thin workflow over capability code and contracts. It may teach discovery and safe
operation; it does not embed a second implementation or a static copy of the capability registry.

### Adapter-owned deployment

Each harness adapter owns its wiring and may remove only installations it can prove it owns.
Codex deployment is materialized and drift-checked; destination edits are never overwritten
silently. Other adapters may use a different mechanism, but no installed copy becomes the editing
source.

Claude Code uses the same profile-aware deployment path as the other harnesses:
`tools/harnesses` and `tools/packs-claude`. There is no separate native marketplace catalogue.
The tracked `.claude/settings.json` contains project hooks and disables automatic memory;
it is not a Pack distribution channel.

### Pack documentation and attribution

Every Pack has a README and SPDX license field. For adaptations and idea-only references, the
README names the canonical upstream, reviewed revision or content digest, license, adopted
influence and material not adopted. Keep complete applicable license and NOTICE texts with the
owning Pack and link them from its README. Preserve lineage comments where changes would
otherwise obscure origin.

## Documentation

### Documentation stays owned and current

Keep one README per capability or Pack, the root README, manifests, schemas and code. A new document
earns its place only when a workflow or executable consumer needs it. Do not add hand-maintained
status, session or fleet files; `tools/doctor`, Git and the self-model already own that state.

Capability and Pack READMEs explain what the thing solves, its verdict, tradeoffs, provenance and
honest experience. A README may precede implementation when defining the capability is itself the
current work.

The [technical docs](website/content/index.md) route readers by task: understand the architecture, build a
capability, or operate an installation. Keep commands and interface contracts in their references;
explain entry points and invariants beside the owning code. This organization draws on
[TigerBeetle's documentation](https://docs.tigerbeetle.com/) and
[rust-analyzer's architecture guide](https://rust-analyzer.github.io/book/contributing/architecture.html),
recorded as documentation influences in `upstreams.toml`.

User docs live in `website/content/` and are published by `website/build` (see
[website/README.md](website/README.md)). `docs/` holds contributor material: `docs/plans/`.

Report vulnerabilities through the private route in [SECURITY.md](SECURITY.md), never through
a public issue.

### Document size and modularity

Aim for fewer than 500 lines per hand-maintained document. At 500–800 lines, review whether it
still serves one reader task. Keep new documents below 800 lines. Existing longer documents
may stay while they are cleaned up; avoid further growth by removing duplication or extracting
a cohesive section when that section changes. Apply this through review before introducing a
CI limit. Generated files and vendored upstream documentation are outside this limit.

Split by subject and ownership, with a short entry point linking to the details. Move a rule
to one canonical location and update its references; do not copy it into several documents.
Use existing READMEs and `website/content/` sections before adding another guide. Keep `AGENTS.md` a short
bootstrap that routes to the shared rules and relevant skills. A separate repository-rules
file earns its place only when it replaces an existing body of rules and simplifies navigation.

### Generated architecture

`ARCHITECTURE.md` is generated from manifests and the tracked tree. Never edit it directly or
maintain a second diagram that can drift. Change its source or generator, regenerate with
`tools/generate-architecture.sh`, and verify with `tools/check-architecture-fresh.sh`.

### Decisions live with their owner

Implemented decisions live in code; reasoning needed to change them lives beside it. Rejected
alternatives use `## Considered and declined` in the affected README; capability-wide choices
use `## Why this shape: <topic>`. Doctor validates that structure and any declared absent paths.
Repository-wide doctrine lives here. Do not create a detached decision-log directory.

### Scratch is not documentation

`to-integrate/` is ignored pre-git scratch and may never be cited by permanent files. Unjudged
external leads belong in the owning README or relevant `website/content/` section. When a lead becomes a
concrete experiment someone can run, track it in a `type:idea` issue. Adopted dependencies and
influences pass through `upstreams.toml` and the consuming owner.

### Quarries and one-way migration

Legacy tooling and LifeOS-mono archives are quarries, not dependencies. Migrate
material into Sjel deliberately, redact it, verify the new owner, and leave the source until the
user explicitly approves removal. Never bulk-import their history or treat an archive location as
permanent provenance.

### Issues and plans

Use GitHub Issues for discrete work that someone can test or complete. State the consumer,
outcome, and acceptance check. `type:idea` marks an experiment that may end without an
implementation. Issues remain open to outside reports as well.

[`PLAN.md`](PLAN.md) is the one ordered plan: it orders the work by priority and records the
decisions that span areas. Each area has one file in [`docs/plans/`](docs/plans/), with its state,
open work and done-when checks. A plan is not a source of truth.
Code and tests own implemented behavior. Shipped interfaces belong in the owner's technical manual and are verified by its doctor or
self-test.

## Placement guide

| Adding | Goes in | Boundary |
| --- | --- | --- |
| Bounded domain, external system or data store | `capabilities/<name>/` | Register an upstream verdict before consuming external code |
| User-facing command for one capability | `capabilities/<name>/<name>` | The shell discovers commands by capability name |
| UI serving one capability | `capabilities/<name>/ui/` | Serve it through that capability's HTTP surface |
| Shared code with no domain | `libs/<name>/` | Requires a second consumer and its own crate in the Cargo workspace |
| Shared contract | `schemas/` | Import it; do not redefine it |
| Agent workflow | `Packs/<pack>/skills/<name>/` | Public and runtime-configured through the overlay |
| System or project Sjel connects to | `systems.toml` | Private URLs go in the overlay extension |
| Host executable required by Sjel | `toolchain.toml` | Add `upstream = <id>` only when it is also consumed code |
| Machine fact or private state | Active overlay | Resolve it dynamically; never copy it into public prose |
| Shared operator logic | `tools/lib/` | Source it from every caller |
| Technical manual & specs | `docs/` | Bounded interface specifications and architecture diagrams |
| Interesting unjudged lead | Owning README or relevant `docs/` section | Track a runnable experiment in a `type:idea` issue; apply the provenance gate before adoption |
| Concrete work someone can test or do | GitHub Issue | State the consumer, outcome, and acceptance check |
| Open work that is not yet an issue | The area plan in `docs/plans/` | Give it a done-when check. Move lasting truth to code, tests, or owner docs once built |

## Control surface

The native app in `apps/gpui` is the visible form of the gluing layer, on the Mac and the iPhone.
It shows capability status and setup, and every capability's own pages. It is presentation only:
every action goes through the same Rust services and manifests (`sjel.toml`, `upstreams.toml`)
that the CLI and agents use. A rule the Mac and the phone share lives in a capability or in
`apps/gpui/core`.

## Layout

| Path | Holds |
|---|---|
| `sjel.toml` | Sjel manifest: platform name, the release-tag pattern, default overlay root. Tracked and shared, so nothing machine-specific lives here |
| `sjel.local.toml` | this machine's overlay root, and optionally which of that overlay's machines this is. Gitignored, one per machine, written by `tools/install.sh` (`sjel.local.toml.example` is the template) |
| `<overlay>/config/machine.toml` | this machine's identity: os, container runtime, enabled capabilities, state-mount registry. One file per machine once an overlay holds more than one, under `config/machines/` |
| `<overlay>/config/deployment.env` | this deployment's shared facts, resolved by `libs/sjel-config` and `libs/sjel-server` for every capability that needs one. A capability may still override, but may not silently disagree |
| `profiles.toml` | named Pack sets (`tools/packs-codex use <profile>`). Tracked and shared: a profile says which Packs belong together, while which machine deploys them stays in the overlay |
| `upstreams.toml` | every external project: url, verdict, license, why |
| `README.md` | what Sjel is, where it stands, and how to start |
| `CONTRIBUTING.md` | how to change Sjel, and the engineering doctrine |
| `PLAN.md`, `docs/plans/` | priority order, and one plan per area with its open work |
| `website/content/` | architecture, capability contracts and operations manuals, published as the public site |
| `AGENTS.md` | minimal cross-harness bootstrap that routes assistants into the `sjel` skill. Every harness, Claude Code included, reads it. There is no `CLAUDE.md`. |
| `ARCHITECTURE.md` | generated snapshot of capabilities/Packs/upstreams/systems — never hand-edited, see `tools/generate-architecture.sh`. State mounts are machine-local and deliberately absent; `tools/doctor` reports those |
| `capabilities/<name>/` | one dir per capability: curated README + its code as it lands |
| `apps/` | `gpui`, the native app for Mac and iPhone, and `mac`, the Swift menu bar and CloudKit relay. Both reach capabilities through the `sjel-status` shell; neither owns domain data |
| `libs/<name>/` | spine-owned shared code with no domain of its own — statically linked into capability binaries at compile time, own crate in the Cargo workspace from day one |
| `schemas/` | shared contracts; import, never redefine |
| `tools/` | operator tools: `install.sh`, `capability.sh` (enable, disable, resolve `requires`), `update.sh`, `doctor`, `audit` (gitleaks and osv-scanner), `storage` (disk classes from the overlay policy, `target` and `reclaim --repo`), `generate-architecture.sh`, `agent-integrations.sh`, and the `sjel-cli` behind `sjel` |

`ARCHITECTURE.md`'s tables and its Mermaid dependency graph (Packs → capabilities they drive →
the upstream image each declares) are derived straight from
`sjel.toml`/`upstreams.toml`/`systems.toml`/`Packs/*/pack.toml` — that's the live, generated
view. Don't hand-edit it, and don't hand-maintain a second diagram somewhere else that it could
drift from. `tools/check-architecture-fresh.sh` catches drift (fails if a manifest changed and
nobody regenerated); `tools/generate-architecture.sh` fixes it.

Every capability dir carries a **curated, opinionated README**: what this solves, the verdict
and why-not-the-alternatives, links worth having, and honest personal experience including
mistakes (`upstreams.toml`'s `[local-llm]` entry is the style bar this is measured against).
The README can exist long before any code does; a capability may BE its README.

The layering below is the fixed conceptual shape and won't change when a capability is added
or removed — for what's actually wired right now, see `ARCHITECTURE.md`'s generated graph
instead.

```mermaid
flowchart TB
    subgraph HARNESS["agent harnesses (swappable)"]
        H["Claude Code · Codex · opencode · pi"]
    end
    subgraph SJEL["Sjel (public shell)"]
        A["apps/gpui (Mac and iPhone)"]
        M["sjel.toml + upstreams.toml"]
        S["schemas/"]
        T["tools/ (install, doctor, audit, generate-architecture)"]
        CAP["capabilities/ (domain owners)"]
        SH["sjel-status (shell)"]
        L["libs/ (spine-owned shared code)"]
        A -->|HTTP| SH
        SH -.->|proxies + starts on demand| CAP
        L --> CAP
    end
    subgraph OVERLAY["selected deployment overlay (private)"]
        P["config values · vault · databases · memory · backups"]
    end
    H --> SJEL
    SJEL -.->|runtime injection only| OVERLAY
    SJEL -->|contracts + state mounts| EXT["separate projects & services"]
```
