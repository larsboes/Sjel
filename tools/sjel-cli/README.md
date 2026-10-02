# sjel-cli

Sjel's public command interface and operator tooling as one Rust binary. The top level is
`sjel`: the repository-root `sjel` launcher execs this binary with its arguments unchanged.
A ported `tools/` script keeps its path and execs it with its own name as the first argument,
so `tools/install.sh`, `tools/doctor` and the tests that call it do not change.

**Verdict:** build. CONTRIBUTING.md#cargo-and-bun-are-the-build-path orders operator tooling
Rust first, and the one-crate shape was decided on 2026-10-02.

## Ported so far

| Subcommand | Launcher | Replaced |
|---|---|---|
| `toolchain-check` | `tools/toolchain-check` | bash, 2026-10-02 |
| `sjel` (dispatch, help, `capability`, `search`, `pack`) | `sjel` | bash and three `tools/lib` files, 2026-10-02 |

`toolchain-check` was compared with its bash version on this Mac before replacement. Text
output, JSON (sorted keys) and exit codes were identical for 10 flag combinations, and
`tools/toolchain-scope.test.sh` passes. One run took 0.31 s against 1.6 s for bash, measured
3 times each.

`sjel` was compared with its bash version in the same way: 35 invocations of the read-only
verbs, including `capability health` against the live machine and five searches, with
identical stdout, stderr and exit codes. Two changes are deliberate. `sjel help search` prints
`<words...>` instead of `search search`, and a capability registry that fails is now an error
for `capability list`, which used to print nothing and exit 0. The verbs that read the
registry are still about 2.5 s, because `tools/capability.sh registry` is bash and takes most
of that.

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
