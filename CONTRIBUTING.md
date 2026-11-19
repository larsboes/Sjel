# Contributing to Sjel

Sjel accepts changes that improve the reusable public shell. Personal data and deployment state
stay in a private overlay. The same boundary covers credentials, private host details, and
operator-specific policy.

## Before writing code

Name the consumer and the outcome. An interesting technology without a concrete Sjel consumer
remains an idea, not an implementation commitment.

GitHub Issues provide reports and discussion. Canonical tasks live in [`docs/tasks/`](docs/tasks/)
with an outcome and acceptance checks. `tools/work list` shows open work; area indexes link the
records rather than duplicating them.

The [work-tracking contract](docs/architecture/work-tracking.md) owns lifecycle, roadmap and
release rules. Move enduring behavior into code, tests or owner manuals. Git keeps history.

Before external code or adopted design influence enters the tree, record its canonical source in
`upstreams.toml`. Record the license and verdict there too, then state precisely what Sjel
adopts. Record no version: every dependency tracks its upstream's latest release
([Patch first](docs/operations/index.md#patch-first)).

## Work on one change

The [branch and commit workflow](Packs/harness/skills/sjel/references/on-changes.md#choose-the-path)
defines which branch to use, including for cloud sessions.

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
The hook checks the pushed commits. When the working tree has other changes, it runs the checks
in a temporary clone of each pushed commit, so another session's uncommitted edits cannot fail
the push. `SJEL_SKIP_PUSH_CHECKS=1` skips formatting and selected gates for one push, but does not skip
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
dispatch. CI and distribution use [the repository Bun pin](.bun-version). The
[Bun compatibility workflow](.github/workflows/bun-latest.yml) tests CLI packaging against the
latest Bun weekly. Distribution preparation calls full CI at the caller revision before building
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

- `tools/maintenance --json` inventories tracked maintenance inputs offline. Add `--overlay`
  to include the configured private repository in a separate local report. Bun runs freshness
  checks for tracked Bun and npm lockfile trees, including workspace packages. Add `--unused`,
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

Follow the [pull request workflow](Packs/harness/skills/sjel/references/on-changes.md#prepare-a-pull-request)
when the branch path calls for one.

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

## Engineering doctrine

The [engineering guide](docs/architecture/engineering.md) owns repository-wide rules.
The [release preparation guide](docs/operations/release-preparation.md) owns release mechanics.

<details>
<summary>Section links</summary>

- <a id="product-rules"></a>[Product rules](docs/architecture/engineering.md#product-rules)
- <a id="start-here"></a>[Start here](docs/architecture/engineering.md#start-here)
- <a id="operating-values"></a>[Operating values](docs/architecture/engineering.md#operating-values)
- <a id="principles-cited-by-number"></a>[Principles cited by number](docs/architecture/engineering.md#principles-cited-by-number)
- <a id="feed-scouting-and-obsidian-boundaries"></a>[Feed, Scouting, and Obsidian boundaries](docs/architecture/engineering.md#feed-scouting-and-obsidian-boundaries)
- <a id="architecture-and-ownership"></a>[Architecture and ownership](docs/architecture/engineering.md#architecture-and-ownership)
- <a id="public-core-and-private-overlays"></a>[Public core and private overlays](docs/architecture/engineering.md#public-core-and-private-overlays)
- <a id="what-the-installer-owns-in-an-agent-harness"></a>[What the installer owns in an agent harness](docs/architecture/engineering.md#what-the-installer-owns-in-an-agent-harness)
- <a id="what-survives-a-harness-swap"></a>[What survives a harness swap](docs/architecture/engineering.md#what-survives-a-harness-swap)
- <a id="integrate-first-topology"></a>[Integrate-first topology](docs/architecture/engineering.md#integrate-first-topology)
- <a id="three-architectural-nouns"></a>[Three architectural nouns](docs/architecture/engineering.md#three-architectural-nouns)
- <a id="schemas-and-dependency-direction"></a>[Schemas and dependency direction](docs/architecture/engineering.md#schemas-and-dependency-direction)
- <a id="dependencies-and-build"></a>[Dependencies and build](docs/architecture/engineering.md#dependencies-and-build)
- <a id="upstream-first"></a>[Upstream-first](docs/architecture/engineering.md#upstream-first)
- <a id="cargo-and-bun-are-the-build-path"></a>[Cargo and bun are the build path](docs/architecture/engineering.md#cargo-and-bun-are-the-build-path)
- <a id="implementation-languages-and-intelligence"></a>[Implementation languages and intelligence](docs/architecture/engineering.md#implementation-languages-and-intelligence)
- <a id="one-manifest-per-concern"></a>[One manifest per concern](docs/architecture/engineering.md#one-manifest-per-concern)
- <a id="state-mounts-record-reality"></a>[State mounts record reality](docs/architecture/engineering.md#state-mounts-record-reality)
- <a id="security-and-data"></a>[Security and data](docs/architecture/engineering.md#security-and-data)
- <a id="dependency-verdicts-and-provenance"></a>[Dependency verdicts and provenance](docs/architecture/engineering.md#dependency-verdicts-and-provenance)
- <a id="patch-first"></a>[Patch first](docs/architecture/engineering.md#patch-first)
- <a id="secrets"></a>[Secrets](docs/architecture/engineering.md#secrets)
- <a id="data-classes"></a>[Data classes](docs/architecture/engineering.md#data-classes)
- <a id="backups-before-migrations"></a>[Backups before migrations](docs/architecture/engineering.md#backups-before-migrations)
- <a id="tooling-conventions"></a>[Tooling conventions](docs/architecture/engineering.md#tooling-conventions)
- <a id="portable-shell"></a>[Portable shell](docs/architecture/engineering.md#portable-shell)
- <a id="language-tooling"></a>[Language tooling](docs/architecture/engineering.md#language-tooling)
- <a id="capabilities-are-data"></a>[Capabilities are data](docs/architecture/engineering.md#capabilities-are-data)
- <a id="dynamic-paths-and-current-facts"></a>[Dynamic paths and current facts](docs/architecture/engineering.md#dynamic-paths-and-current-facts)
- <a id="public-cli"></a>[Public CLI](docs/architecture/engineering.md#public-cli)
- <a id="releases"></a>[Releases](docs/operations/release-preparation.md)
- <a id="the-release-line"></a>[The release line](docs/operations/release-preparation.md#the-release-line)
- <a id="cutting-and-consuming-a-release"></a>[Cutting and consuming a release](docs/operations/release-preparation.md#cutting-and-consuming-a-release)
- <a id="getting-onto-the-line"></a>[Getting onto the line](docs/operations/release-preparation.md#getting-onto-the-line)
- <a id="packs-and-agent-harnesses"></a>[Packs and agent harnesses](docs/architecture/engineering.md#packs-and-agent-harnesses)
- <a id="harness-neutral-packs"></a>[Harness-neutral Packs](Packs/harness/README.md#pack-source-and-attribution)
- <a id="proportionate-approvals-and-delegation"></a>[Proportionate approvals and delegation](docs/architecture/engineering.md#proportionate-approvals-and-delegation)
- <a id="owned-agent-components"></a>[Owned agent components](docs/architecture/engineering.md#owned-agent-components)
- <a id="public-skills"></a>[Public skills](Packs/harness/README.md#pack-source-and-attribution)
- <a id="skills-drive-capabilities"></a>[Skills drive capabilities](Packs/harness/README.md#pack-source-and-attribution)
- <a id="adapter-owned-deployment"></a>[Adapter-owned deployment](Packs/harness/README.md#pack-source-and-attribution)
- <a id="pack-documentation-and-attribution"></a>[Pack documentation and attribution](Packs/harness/README.md#pack-source-and-attribution)
- <a id="documentation"></a>[Documentation](docs/architecture/engineering.md#documentation)
- <a id="documentation-stays-owned-and-current"></a>[Documentation stays owned and current](docs/architecture/engineering.md#documentation-stays-owned-and-current)
- <a id="document-size-and-modularity"></a>[Document size and modularity](docs/architecture/engineering.md#document-size-and-modularity)
- <a id="generated-architecture"></a>[Generated architecture](docs/architecture/engineering.md#generated-architecture)
- <a id="decisions-live-with-their-owner"></a>[Decisions live with their owner](docs/architecture/engineering.md#decisions-live-with-their-owner)
- <a id="scratch-is-not-documentation"></a>[Scratch is not documentation](docs/architecture/engineering.md#scratch-is-not-documentation)
- <a id="quarries-and-one-way-migration"></a>[Quarries and one-way migration](docs/architecture/engineering.md#quarries-and-one-way-migration)
- <a id="issues-and-plans"></a>[Issues and plans](docs/architecture/engineering.md#issues-and-plans)
- <a id="placement-guide"></a>[Placement guide](docs/architecture/engineering.md#placement-guide)
- <a id="control-surface"></a>[Control surface](docs/architecture/engineering.md#control-surface)
- <a id="layout"></a>[Layout](docs/architecture/engineering.md#layout)

</details>
