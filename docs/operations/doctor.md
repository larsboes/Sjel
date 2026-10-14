# Doctor Diagnostics Suite

`tools/doctor` is the primary health assertion tool for a Sjel host. It is non-destructive and evaluates whether current machine state conforms to declared architectural invariants.

## Check Categories

### 1. Host Toolchain & Runtime
- Verifies that all CLI utilities required by the host profile are installed (e.g. `bun`, `cargo`, `sqlite3`, `launchctl`).
- Enforces Bun/npm install policies (`minimumReleaseAge = 86400`).

### 2. Scheduled Producers
- Queries `launchctl list` (or `systemctl`) to verify periodic background workers.
- Asserts that every scheduled job's last run exited with status code `0`.
- Verifies that output logs (`/tmp/axon-*-schedule.log`) are actively written within their declared intervals.

### 3. Store Integrity
- Executes `PRAGMA integrity_check;` on the shared SQLite database.
- Checks WAL file size and permissions.

### 4. Freshness & Backups
- Evaluates declared contracts across `comms`, `finance`, and `places`.
- Confirms that backup archives exist at destinations with valid checksums and timestamps.

## Remediation Workflow

When a check fails:
1. Note the specific sub-check and error output.
2. Inspect the log referenced by the unit (e.g. `/tmp/axon-<unit>-schedule.err`).
3. For launchd units reporting non-zero exits, kickstart the unit:
   ```bash
   launchctl kickstart -k gui/$(id -u)/com.sjel.<unit>
   ```
4. Re-run `tools/doctor` to confirm resolution.
