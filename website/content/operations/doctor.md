# Doctor diagnostics suite

`tools/doctor` checks whether an already configured host matches its declarations. It reports
failures; it does not establish that every application workflow works.

```bash
tools/doctor            # full report without GitHub calls
tools/doctor --online   # also probe declared systems and fetch origin/main
tools/doctor --version  # version identity only
```

The full report exits `0` when all checks pass and `1` when one or more checks fail.
`--version` exits `0` and is not a health assertion. The current report sections are defined in
[`doctor/checks.rs`](../../tools/sjel-cli/src/doctor/checks.rs).

## Check categories

### 1. Host toolchain & runtime

Doctor delegates dependency checks to `tools/toolchain-check` and reports machine identity,
state mounts, enabled capabilities, manifest validity, port uniqueness, and shared server policy.
It also reports local inference roles and agent harness deployment.

### 2. Scheduled producers

Persistence checks compare installed units with their generated declarations. Producer checks
look for evidence that scheduled work ran. Read the named producer and its log or receipt when
an installed unit has stopped producing data; unit presence alone does not prove a successful run.

### 3. Store integrity

The shared-store check opens SQLite read-only and runs `PRAGMA integrity_check`. This checks the
database structure; domain correctness and application behavior need their own checks.

### 4. Freshness & backups

Freshness checks use declared arrival contracts. Backup checks inspect receipts and the archives
they name. Follow the owning capability's backup contract for restore verification; a fresh
receipt alone does not prove recovery.

## Remediation workflow

1. Record the failed section and its exact output.
2. Read the named tool or capability's diagnostics and inspect its referenced log or receipt.
3. Correct that cause, then rerun the relevant command and `tools/doctor`.

For lifecycle or installed-unit failures, use the [service runner](persistence.md#lifecycle-commands).
A manual stop creates a maintenance hold; use `resume` when you intend to lift it. Restarting a
job without inspecting the failure can hide the evidence needed to fix it.
