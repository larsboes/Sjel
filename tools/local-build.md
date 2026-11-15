# Battery-aware local builds

`tools/local-build run <command> [args...]` prepares an explicitly requested build and executes
it without starting a shell. `sjel cargo` uses the same preparation before verification.
Neither path changes the saved runtime profile or starts a background watcher.

```sh
sjel cargo check --locked -p sjel-runtime
sjel cargo test --locked -p sjel-runtime
tools/local-build run cargo build --locked -p sjel-cli
```

## Resource defaults

Normal preserves the caller's environment. On the go sets `CARGO_BUILD_JOBS` and
`RUST_TEST_THREADS` to two, preserving an existing value of one. The child receives a
per-command build allowance so nested guarded tools do not refuse that requested work.
An unreadable admission decision fails before the command starts. A legacy CLI that can
answer admission remains compatible; no compiler runs to discover the power profile.

These are Rust concurrency defaults, not an OS resource quota. Explicit Cargo `--jobs` and test
harness arguments can override environment defaults. Native dependency build scripts and
non-Rust commands can manage their own workers. Lower concurrency limits contention; it does
not establish a measured energy saving or guarantee a shorter build.

The wrapper preserves the child's exit status and does not clean or relocate artifacts.
`sjel cargo` retains its checkout-scoped verification cache and release-build refusal.
Development, verification and installed release outputs remain separate. Keeping incremental
state helps repeated builds; changing compiler flags, features or profiles can create another
artifact set. Cache reclamation remains owned by `sjel storage`.

## Admission-only callers

`tools/local-build` without arguments keeps its admission-only contract: zero admits, three
defers and two means the profile could not be determined. Automatic CLI bootstrap and agent
raw-command guards retain that conservative policy. An explicit `SJEL_ALLOW_LOCAL_BUILD=1`
bypasses admission without applying battery defaults.

## Checks

`bash tools/local-build.test.sh` exercises admission, battery defaults, existing lower limits,
Normal-mode preservation, invalid inputs, unreadable profiles and child exit status with a stub
CLI. No compiler or profile mutation is needed.
