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
| `capability.sh` (list, enable, disable, registry) | `tools/capability.sh` | bash, 2026-10-02 |
| `service-runner.sh` (lifecycle, holds, drift, persistence units) | `tools/service-runner.sh` | bash and `tools/lib/runargs.sh`, 2026-10-02 |
| `doctor` | `tools/doctor` | TypeScript (`doctor.ts`), 2026-10-02 |
| `harnesses` (list, status, drift) | `tools/harnesses` | TypeScript (`harnesses.ts`), 2026-10-02 |

`toolchain-check` was compared with its bash version on this Mac before replacement. Text
output, JSON (sorted keys) and exit codes were identical for 10 flag combinations, and
`tools/toolchain-scope.test.sh` passes. One run took 0.31 s against 1.6 s for bash, measured
3 times each.

`sjel` was compared with its bash version in the same way: 35 invocations of the read-only
verbs, including `capability health` against the live machine and five searches, with
identical stdout, stderr and exit codes. Two changes are deliberate. `sjel help search` prints
`<words...>` instead of `search search`, and a capability registry that fails is now an error
for `capability list`, which used to print nothing and exit 0. The verbs that read the
registry were still about 2.5 s at that point, because `tools/capability.sh registry` was bash.

`capability.sh` was compared across 15 invocations: `registry`, `registry --lines` and `list`
against the live overlay, and every `enable` and `disable` path (new, already enabled,
unknown, a dependency chain, a still-required capability, a leaf) against a scratch copy of
the overlay's manifests and machine.toml. Stdout, stderr, exit codes and the rewritten
machine.toml were identical. Only `-h` differs, because its old text claimed every read went
through tools/lib/toml.sh. `registry` takes 17 ms against 2.40 s (hyperfine, 5 runs), and
`sjel capability health` and `sjel search` now call it in-process: 0.16 s and 0.05 s.

`service-runner.sh` was checked three ways before the launcher replaced it. The suites that
drive it with stub runtimes and supervisors (service-runner, persistence, container-refresh,
backup, pipe) pass against the binary, and fail 17 and 52 checks against `/usr/bin/false`,
so they exercise it. On this Mac, `status` and `persistence-status` for all 32 registry rows
were identical to the bash version, including the 15 installed launchd units it re-renders
and compares byte for byte. One on-demand capability went through start, held stop, refused
start, resume and idle-stop, and ended as it began. This machine runs no container
capability, so the container verbs are verified by the stub suites only. One change is
deliberate: a scheduled job now stops the dependencies it started even when the job fails,
where `set -e` used to exit first.

`doctor` was compared with doctor.ts on this Mac, in both run orders: the same 264 lines and
exit code, differing only in ages that ticked between the two runs. Every case of
doctor.test.ts is a unit test in `src/doctor/pure.rs`. Pack deployment state is read
in-process by `src/harnesses/` since the same day — the `tools/doctor-packs.ts` sidecar
existed for one commit and was deleted with the read half of `tools/harnesses.ts`.
Independent sections run at once and print in order: 26.6 s became 15.6 s, and
`SJEL_DOCTOR_TIMING=1` shows that one `sjel-storage target` walk is now most of what remains.

`tools/harnesses` was ported read-verbs-first (decided 2026-10-02): `list`, `status` and
`drift` are Rust, and `sync`, `use`, `promote` and `accept` still exec `tools/harnesses.ts`,
which keeps the engine that owns the mutation lock and the atomic install. The read code was
deleted from the TypeScript file rather than left beside the port, so each ledger has one
reader. `list`, `status` and `drift` were compared against it across thirteen invocations on
the live machine and three against a scratch claude destination — every per-harness `status`,
`status <pack>`, `--all-harnesses`, `status --json`, `drift`, `drift <pack>` and
`drift --diff` — where the scratch destination was deployed through the TypeScript engine,
then edited to produce a drifted file, a missing file and an only-at-destination file. stdout,
stderr and exit codes were identical, and `status --json` is byte-identical after dropping
`measuredAt`. On this Mac `status` takes 33 ms against 63 ms for the TypeScript version (three
runs each), and `status --json` 30 ms against 40 ms — the smaller number matters less than
what it removes: the dashboard's Packs page, the doctor and the CLI now read one ledger without
bun in the path.

Three differences are deliberate, and each is the only one found:

- Discovered pi entries and extensions print in name order, where readdir order was
  filesystem-dependent. The set is identical; the order is now the same on every machine.
- The per-file lines inside one `drift` unit print in sorted order for the same reason.
- A pack.toml that will not parse reports the `toml` crate's message where `Bun.TOML`
  reported its own, and a ledger that is malformed JSON reports one fixed sentence instead of
  the parser's. Both are only reachable on a file that is already broken, and the doctor's
  "Pack state unreadable" line is what an operator sees either way.

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
5. When only part of a tool moves, name the split in both files and route the rest through
   this binary's exec (`src/harnesses/mod.rs` forwards four verbs to `tools/harnesses.ts`).
   Delete the moved code from the interpreted original — a second reader of one ledger is the
   duplication this crate exists to remove.
