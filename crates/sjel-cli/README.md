# sjel-cli

Unified command interface, maintenance engine, and operator tooling for Sjel as one Rust binary.
The repository-root `sjel` launcher delegates to this binary with arguments preserved. Launcher scripts
in `tools/` (such as `tools/doctor`, `tools/install.sh`, `tools/capability.sh`, `tools/harnesses`) exec
their respective subcommands through `sjel-cli`.

## Surface

### Core CLI (`sjel`)

```bash
sjel help                      # command overview and usage
sjel capability list           # list active and available capabilities
sjel capability health         # query capability health status
sjel projects projects         # run the projects CLI
sjel operator-profile show     # run the operator-profile CLI
sjel search <query>            # search across skills, tools, and docs
sjel doctor                    # diagnostic health and contract checks
sjel setup plan                # inspect required host and capability configuration
sjel setup apply --confirm     # apply host and capability configuration steps
sjel harnesses sync --all      # synchronize skill packs across agent harnesses
sjel update report             # check dependencies and toolchain freshness
sjel audit                     # security vulnerability audit across locks and packages
```

`sjel <capability> <args...>` resolves the capability through the shared service-manifest
lookup and runs its declared `<capability>` or `<capability>-cli` Cargo binary. Built-in
commands take precedence. Cargo checks source and dependency freshness, builds in debug
mode with the lockfile, and forwards arguments after `--`. Debug builds leave deployed
release binaries intact. The CLI inherits the caller's working directory and receives the
resolved overlay. The local-build power policy also applies. Server and helper binaries are not
selected as CLI fallbacks.

### Launchers in `tools/`

Dedicated shell launchers under `tools/` maintain stable entry points for automation, git hooks,
and supervisor services (`tools/service-runner.sh`). Each sources `tools/lib/paths.sh` and
executes `sjel-cli` with the launcher's verb name.

### Process restarts

`tools/service-runner.sh restart <capability>` holds and stops a host process before building
its executable. A failed stop or build leaves a non-expiring restart hold in the deployment's
`data/service-runner/` directory. A successful retry clears that hold; an existing operator
maintenance hold remains in force. Ordinary maintenance holds retain their 30-minute expiry.

`tools/service-runner.sh resume <capability>` explicitly clears both holds and permits startup
again. Process lifecycle operations use an exclusive lock, so concurrent starts and resumes
cannot interrupt a restart build. The lock is released before a launchd handoff because the job
re-enters the runner. The launcher refuses restart through an older CLI that lacks this safety
contract, even when its automatic rebuild fails. Startup failures after a successful build keep
the existing supervisor retry behavior.

## Why this shape: Unified operator binary

Operator tooling is consolidated into a single Rust crate rather than independent scripts:

1. **Shared runtime state.** Commands like `sjel doctor`, `sjel setup`, and `sjel capability`
   inspect manifests, system configs, overlay state, and database tables. Compiling them in one binary
   avoids subprocess spawning overhead and eliminates duplicate schema parsers.
2. **In-process ledger inspection.** Harness synchronization, pack drift hooks, and self-model queries
   execute in-process without invoking secondary runtimes (`node`, `bun`, `bash`), keeping tool execution
   latencies under human perception thresholds.
3. **Hermetic path and overlay resolution.** Centralizing filesystem logic in `src/paths.rs` guarantees
   that overlay hierarchies (`$SJEL_PERSONAL_ROOT`), cargo target locations, and state mounts are
   interpreted consistently across all maintenance tasks.

## Why this shape: Setup planning and execution

Host and service setup via `sjel setup` (`src/setup/mod.rs`) decouples inspection from mutation:

1. **`plan` is the default and writes nothing.** Querying a host's readiness inspects system properties
   without altering state.
2. **`apply` requires explicit confirmation.** Every applied step is idempotent; running setup on a
   configured system produces a no-op verification report.
3. **Direct execution.** Setup plans contain only machine-executable steps executed directly by the binary,
   avoiding instructions that require manual operator execution.
4. **Credential isolation.** Secrets (such as replication tokens) are accepted exclusively via standard
   input (`--token-stdin`) and written directly to overlay deployment environments, preventing exposure
   in process arguments, logs, or command history.
5. **Decoupled API representation.** All setup commands support `--json` output, enabling web or native
   clients (such as `sjel-status` and desktop interfaces) to render status without running as privileged
   host agents.

## Why this shape: Host watch and store integration

`sjel-cli host-watch` monitors system metrics and writes telemetry directly into the shared store:

1. **Direct SQLite transactions.** Metrics are committed using `sjel-store` write transactions,
   ensuring atomic updates for findings and clearing stale alerts without out-of-process brokers.
2. **Deterministic process inspection.** Process evaluation rules (`src/host_watch/pure.rs`) decouple
   data collection from analysis, allowing hermetic unit testing of threshold logic.
3. **Structured supervisor integration.** Scheduled execution under `launchd` executes through
   `tools/service-runner.sh`, which provisions environment variables, PATH configuration, and automatic
   binary rebuilds when dependencies change.

## Why this shape: Harness synchronization and drift hooks

Agent pack deployments (`src/harnesses/mod.rs`) maintain agent configurations across installed harnesses:

1. **Deterministic serialization.** Manifests and settings files are written with sorted map keys,
   ensuring diffs reflect intentional configuration changes rather than map iteration order.
2. **Lightweight drift hooks.** The pre-session drift hook runs synchronously during harness startup.
   In-process evaluation checks file digests against recorded ledger state without starting an interpreter.

## Why this shape: Dependency and security auditing

Vulnerability auditing (`src/audit.rs` and `src/updates/mod.rs`) enforces repository hygiene:

1. **Split policy evaluation.** Repository lockfiles are audited against strict blocking advisories
   (`osv-scanner.toml`), while host-installed packages are evaluated against a dedicated acceptance ledger
   (`osv-scanner-installed.toml`) with bounded expiration horizons.
2. **Self-contained SBOM generation.** Lockfiles and package manifests are parsed directly into package
   URL inventories, removing external dependencies on tools like `jq`.
3. **Order-preserving inventory mappings.** Ecosystem inventories with strict ordering requirements (such
   as nested npm trees) use dedicated order-preserving structures to match upstream CLI outputs.

## Considered and declined

- **Preserving interpreted sidecars alongside ports.** Declined: maintaining dual implementations of
  ledger or manifest parsers introduces semantic drift; script originals are retired once Rust ports
  verify behavioral parity.
- **Dynamic plugin loading for CLI subcommands.** Declined: compile-time dispatch provides static
  dependency checks, faster startup times, and simpler distribution without dynamic library linking.
- **Global order-preserving JSON serialization.** Declined: enabling order preservation across `serde_json`
  globally impacts performance across all capability crates; order-preserving maps are used only in
  modules where external format parity requires it.
- **Accepting secrets via command-line arguments.** Declined: passing tokens or private keys in command
  arguments exposes credentials to process table queries (`ps`) and shell history.
