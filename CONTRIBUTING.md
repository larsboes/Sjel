# Contributing to Axon

Axon accepts changes that improve the reusable public shell. Personal data and deployment state
stay in a private overlay. The same boundary covers credentials, private host details, and
operator-specific policy.

## Before writing code

Name the consumer and the outcome. An interesting technology without a concrete Axon consumer
remains an idea, not an implementation commitment.

No backlog entry is required to start. Add a claim to the owning `ISA.md` only when something
must outlive the change itself: a defect being left unfixed, or a decision that needs a record.
Write it as a claim with the probe that would falsify it, not as a description. The issue tracker
takes reports from outside the project; it is not where this project's work is planned.

Before external code or adopted design influence enters the tree, record its canonical source in
`upstreams.toml`. Record the license and verdict there too, then state precisely what Axon
adopts. No version: the register holds none since 2026-09-02, because every dependency tracks its
upstream's latest release (README.md#patch-first).

## Work on one change

Start from current `main` and create a branch named for the change: `<area>-<short-slug>`. Keep
the diff inside one coherent boundary. Put reusable code and doctrine in Axon; use synthetic fixtures for
data-shaped tests. Never copy an active overlay or secret value into public work. Workstation paths
and private logs must also stay out of commits and GitHub text, including screenshots and test
failures.

Run `tools/doctor` before editing and record unrelated or machine-only failures separately. A
fresh source checkout does not need a real private overlay for CI; repository tests use synthetic
configuration where a machine contract is required.

## Validate the changed boundary

Run the nearest tests and checks declared by the package you changed. Then inspect the focused
diff, `git diff --check`, and `git status --short` before committing. Common repository checks
are:

~~~sh
cargo test --workspace --locked
bun test
tools/check-publication-hygiene.sh
~~~

Rust packages are members of the root `Cargo.toml` workspace and share the
root `Cargo.lock`. Keep each package's direct dependencies in that package's
manifest; do not add a nested lockfile. For a Rust change, verify the workspace
view and the build:

~~~sh
cargo metadata --locked --no-deps --format-version 1
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo check --workspace --locked
cargo test --workspace --locked
~~~

Run the format and Clippy commands from the repository root. They use the channel and
components `rust-toolchain.toml` names -- `stable`, so the release rustup last fetched;
CI resolves the same file, not a literal held somewhere else. Do not replace a finding
with a workspace-wide allowance. A narrow allowance belongs beside the affected
item and must explain the invariant that makes the lint inapplicable.

That command needs no database service and no environment variable. The
database-backed suites are `db_tests::` — one module name across the workspace,
which is what makes them selectable — and each test opens a temp SQLite file of
its own. Run them alone with:

~~~sh
cargo test --workspace --locked -- db_tests::
~~~

Until PRD Q45 (2026-08-27) these suites needed a running Postgres and six
`*_TEST_DATABASE_URL` variables, so the hermetic command carried
`--skip postgres_tests::` and CI ran a second job with a service container.
Both are gone; a checkout with no overlay and no server runs everything.

Run `bun run check` in `dashboard/` when dashboard code changes. Manifest or
generated-architecture changes also require:

~~~sh
tools/generate-architecture.sh
tools/check-architecture-fresh.sh
~~~

Do not describe a skipped or unavailable check as passing.

**`SJEL_DB_PATH` isolates the database and nothing else.** It does not isolate a vault
projection. `capabilities/trips`' `project_after_write` runs as a router layer after any
successful non-GET request and takes its root from the overlay config, which that variable
never touches — so a live check on 2026-09-05 that redirected only the database passed every
assertion while re-exporting thirteen real plan notes into the operator's Obsidian vault and
creating a fourteenth. `finance` and `comms` project too. Before running a server against a
copy, export every projection root as well: `SJEL_TRIPS_OBSIDIAN_ROOT`,
`SJEL_FINANCE_OBSIDIAN_ROOT`, `SJEL_FINANCE_DECISIONS_ROOT`, and a scratch `SJEL_COMMS_CONFIG`
(comms resolves its config file from that variable, so a scratch file is what redirects it).
Overriding `SJEL_PERSONAL_ROOT` instead is not the fix: it redirects the config *read* too, so
the run tests a configuration nobody is operating.

**Do not `cargo build --release` in a worktree that shares the repository's `target/`.** That
build writes `target/release/<bin>`, which is the exact artifact the supervisor runs, and on
Apple Silicon a running process dies when its own binary changes underneath it. On 2026-09-05
a worktree release build therefore killed a live service, the supervisor restarted it from the
new artifact, and the branch's migration ran against the real database — leaving eight empty
tables no merged commit had put there. The build reported success and nothing anywhere
reported the restart. `tools/service-runner.sh`'s `maybe_build` is the function that makes the
artifact load-bearing. For a live check from a worktree, build a debug binary, run it on a free
port and point it at a copy; use a separate `CARGO_TARGET_DIR` if a release build is
unavoidable.

The same rule applies inside a test. An assertion that needs something only one platform has —
`/dev/full`, a container runtime, a specific filesystem — may be given up on a developer machine
and never in an automated run, where "it runs in CI" would otherwise be an assumption nobody can
see failing. Guard it with `skippable` from `tools/lib/test-support.sh`: outside CI it prints what
coverage was lost, and inside CI it fails.

## Open the pull request

Open a draft pull request first. State the outcome, then bound the exact scope. List every
completed validation command with its result and name the known limits. Keep separable follow-ups
out of the pull request rather than widening it. A merge should land one coherent change and
remain easy to review or revert.

Security findings follow [SECURITY.md](SECURITY.md); never report one in a public issue or pull
request.

## License and sign-off

The project is licensed under the GNU Affero General Public License, version 3 only
([LICENSE](LICENSE)). Anyone who runs a changed version as a service for other people must
offer those people its source. Code that `Packs/` vendors from other projects keeps its own
license; each such Pack reproduces the upstream notice in its `LICENSE` file.

Every commit carries a Developer Certificate of Origin sign-off
([developercertificate.org](https://developercertificate.org/)): a `Signed-off-by:` line that
states you have the right to submit the change under this license. `git commit -s` adds it.
A sign-off does not transfer copyright: a contributor keeps it in their change. A later license
change (README, open question O2) therefore needs the consent of every contributor whose code
remains, or a contributor license agreement introduced before outside contributions arrive.

Releases before 2026-09-26 were published under the MIT license and stay available under it.
