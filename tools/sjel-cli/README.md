# sjel-cli

Sjel's operator tooling as one Rust binary. Each subcommand replaces one interpreted `tools/`
script. The script keeps its path and becomes a launcher, so `tools/install.sh`, `tools/doctor`
and the tests that call it do not change.

**Verdict:** build. CONTRIBUTING.md#cargo-and-bun-are-the-build-path orders operator tooling
Rust first, and the one-crate shape was decided on 2026-10-02.

## Ported so far

| Subcommand | Launcher | Replaced |
|---|---|---|
| `toolchain-check` | `tools/toolchain-check` | bash, 2026-10-02 |

The bash version was compared with the Rust version on this Mac before replacement. Text
output, JSON (sorted keys) and exit codes were identical for 10 flag combinations, and
`tools/toolchain-scope.test.sh` passes. One run took 0.31 s against 1.6 s for bash, measured
3 times each.

## Porting a script

1. Add `src/<name>.rs` and a match arm in `src/main.rs`.
2. Read locations from `paths::Paths`. Do not re-derive the overlay order:
   `tools/lib/paths.sh` owns it, and the launcher sources it first.
3. Replace the script body with a launcher:

   ```bash
   _here="$(cd "$(dirname "$0")" && pwd)"
   . "$_here/lib/paths.sh"
   . "$_here/lib/sjel-cli.sh"
   sjel_cli_exec <name> "$@"
   ```

4. Before you delete the old script, compare its output with the port's for every flag
   combination, and keep its tests green. A test that copies the launcher into a scratch root
   sets `SJEL_CLI_BIN` to a prebuilt binary (see `tools/toolchain-scope.test.sh`).
