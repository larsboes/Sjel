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
| `harnesses` (sync, use, promote, accept) | `tools/harnesses` | TypeScript (`harnesses.ts`), 2026-10-04 |
| `updates` (report and apply) | `tools/updates` | TypeScript (`updates.ts`), 2026-10-04 |
| `packs-claude`, `packs-codex`, `packs-opencode`, `packs-pi` | `tools/packs-*` | TypeScript (`packs-*.ts`), 2026-10-04 |
| `pack-drift-hook` | `tools/pack-drift-hook` | TypeScript (`pack-drift-hook.ts`), 2026-10-04 |
| `self` (generate, status, explain, coupling, check) | `tools/self` | TypeScript (`self.ts` + `lib/self-model.ts`), 2026-10-04 |

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
existed for one commit and was deleted with the read half of `harnesses.ts`.
Independent sections run at once and print in order: 26.6 s became 15.6 s, and
`SJEL_DOCTOR_TIMING=1` shows that one `sjel-storage target` walk is now most of what remains.

`tools/harnesses` was ported read-verbs-first (decided 2026-10-02): `list`, `status` and
`drift` are Rust, and the write verbs followed on 2026-10-04 (see below). The read code was
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

The write verbs followed on 2026-10-04, and with them `tools/lib/pack-deploy.ts`'s mutation half
(`src/harnesses/mutate.rs`: the state lock, the atomic install, the digest policy, `sync_pack`,
`adopt_pack`, `reconcile_unit`, `deploy_pack`, `remove_pack`, profiles) and pi's settings
registry (`src/harnesses/pi_settings.rs`), which `use --harness pi` needs. `tools/harnesses.ts`
and its test are deleted, so the whole tool is Rust. Every verb was compared against the
TypeScript on two identical scratch roots — a copy of two Packs, `profiles.toml` and `tools/`,
with every harness destination and state file pointed at the scratch tree — across `sync`
(undeployed, one pack, `--all`), `use` on claude and on pi, `promote` and `accept`: identical
stdout, stderr, exit codes, destination trees, rewritten `pack.toml` and ledgers.

Two differences are deliberate. The JSON ledgers and `settings.json` write map keys sorted
where TypeScript wrote them in insertion order — a ledger is read by name, and the semantic
comparison above is what the parity claim rests on. And the provenance comment in a materialized
pi agent file now reads `Generated by Sjel tools/sjel-cli` where the TypeScript wrote
`Generated by Axon tools/packs-pi`: the generator moved and the old name is retired, so a
deployed copy is rewritten once and its recorded digest changes with it.

`tools/updates` was ported whole — the report half and the apply half together, because `apply`
re-reads the report after its steps and a split would have left two readers of the same rows.
`tools/updates.ts` and its 76-case `updates.test.ts` are deleted; the parsers, gatherers and plan
are 137 `cargo test -p sjel-cli` cases with the same fixtures. Compared against `updates.ts` at
HEAD across every flag combination — `-h`, `--offline`, `--json`, `--json --offline --inventory`,
five refusal paths, the live report and `--json`, and four `apply` plans refused on a non-TTY —
stdout, stderr and exit codes were identical after normalizing `generatedAt` and the ages that
tick between two runs.

One difference was found and fixed rather than recorded. `serde_json`'s default map sorts keys,
and npm's nested dependency order is not sorted, so the `inventory` array `tools/audit` scans came
out in a different order (the set was identical). `updates/parse.rs` carries an order-preserving
map for the npm parsers, which restores npm's order without turning on
`serde_json/preserve_order` — that feature is additive across the whole build and would change
`serde_json::Map` for every crate in the workspace.

The four `packs-*` adapters followed on 2026-10-04, and with them `tools/lib/pack-deploy.ts`,
`tools/lib/harness-registry.ts` and `tools/pack-drift-hook.ts`. This was the port the engine was
waiting for: `harnesses` had already moved `pack-deploy.ts`'s mutation half into
`src/harnesses/mutate.rs`, so the four adapters were the only thing keeping the 1,320-line
TypeScript engine alive as a second implementation of one ledger. `src/packs.rs` is their verb
surface — four dispatchers over the engine, because the four CLIs differ in small load-bearing
ways (claude heads a multi-pack write with the Pack name and codex does not; codex alone can
`migrate-generated`; pi is a settings registry rather than a copy). `src/pack_hook.rs` is the
drift hook, which now runs on every session start and file change without a bun start-up in the
path.

The three non-adapter readers moved in the same change, so one ledger has one reader:
`pack-drift-hook.ts` became `src/pack_hook.rs` (the engine in-process, not a shell-out);
`harness-registry.ts` was deleted and its one remaining consumer, `tools/pack-extensions.test.ts`,
keeps pi's marker inline; and `tools/generate-marketplace.ts` reads `sjel packs list`, a new read
verb over the same `available_packs`. `sjel search` reads it in-process too, where it used to
shell `packs-opencode list`.

Verified against the TypeScript before deletion, on two identical scratch roots (two crafted
Packs carrying a skill, an `agents/` tree, an extension, a codex overlay and a vendored pi
package; a `profiles.toml`; every destination and ledger pointed at the scratch tree). 114
comparisons — `status`, `list`, `deploy`, `sync`, `sync --all`, `remove`, `adopt`, `use`, `-h`,
an unknown verb, and `migrate-generated` in its refusal, positive and already-migrated paths —
were all identical in stdout, stderr and exit code after normalizing the scratch root's own
path. The deployed trees were identical except for one file, and the ledgers were semantically
identical except for the digests that file's change moves. Three differences are deliberate:

- The provenance comment in a materialized pi agent file reads `Generated by Sjel
  tools/sjel-cli` where the TypeScript wrote `Generated by Axon tools/packs-pi`. That is the
  same difference the `harnesses` port recorded, restated here because it is the one file the
  scratch comparison found changed: a deployed copy is rewritten once and its recorded digest
  moves with it.
- JSON object keys are sorted, because `serde_json`'s default map does that; the TypeScript
  wrote insertion order. `settings.json` is where this is visible, and pi reads it by key.
- `migrate-generated` removes a `__pycache__` directory's files in name order, and its messages
  follow sorted ledger keys, where the TypeScript took `readdirSync` and insertion order. The
  set removed is identical.

`tools/self` followed on 2026-10-04: `tools/self.ts` and `tools/lib/self-model.ts` are deleted,
and `src/self_model/mod.rs` (the I/O and the verb surface) and `src/self_model/model.rs` (the pure
half, with the cases `tools/self.test.ts` held as Rust unit tests) replace them. It moved for a
measured reason rather than a doctrinal one: `tools/doctor` runs `tools/self check` on every
invocation, so a bun start-up sat inside a Rust tool's path, and agents run `self explain` and
`self coupling` at orientation.

Verified against the TypeScript before deletion, on this checkout: `generate` produced a
byte-identical artifact except for the one deliberate line below; `status`, `status --json`,
`explain <unit>` (text and `--json`, a known unit and an unknown one), `coupling` (text and
`--json`), `check` on a current artifact and on a stale one — including the `diff -u` rendering —
and `-h` were identical in stdout, stderr and exit code. Three differences are deliberate:

- The artifact's `generator` field reads `tools/self` where the TypeScript wrote `tools/self.ts`.
  `self.json` was regenerated in the same change, which is the whole of the artifact diff.
- The help line and the status header say `Sjel's self-model` where the TypeScript still said
  `Axon's self-model`. That rename is repository-wide and older than this port (ISA F3); those two
  strings were the ones it had missed.
- `rollUp` no longer builds its `node id -> unit` map. The TypeScript returned it "for edge
  attribution" and nothing read it — the coupling layer comes from `#[path]` attributes and Cargo
  path dependencies, never from the graph. A graph node is no longer deserialized with an id.

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
   this binary's exec (the harnesses read verbs did this until the write verbs followed on
   2026-10-04). Delete the moved code from the interpreted original — a second reader of one
   ledger is the duplication this crate exists to remove.
