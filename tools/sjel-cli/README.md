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
| `audit` (no arguments) | `tools/audit` | bash, 2026-10-04 |
| `host-watch` (check, `--dry-run`, `--json`) | `tools/host-watch` | TypeScript (`host-watch.ts` + `host-watch.test.ts`), 2026-10-04 |
| `inference-keys` (key gateway, unlock, migration) | `tools/inference-keys`, `tools/bw-unlock`, Pack migration launcher | `bw-key.mjs`, `bw-unlock`, and `keychain-write.exp`, 2026-10-04 |
| `discover-ui-packages` | `tools/discover-ui-packages` | TypeScript implementation and tests, 2026-10-04 |
| `feed-sweep` | `tools/feed-sweep` | TypeScript (`feed-sweep.ts`), 2026-10-04 |
| `sparpreis-watch` | `tools/sparpreis-watch` | TypeScript (`sparpreis-watch.ts` + `sparpreis-watch.test.ts`), 2026-10-04 |
| `model-check` | `tools/model-check` | TypeScript (`model-check.ts`), 2026-10-04 |

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

`updates` grew an npm half on 2026-10-04, after the audit's installed pass showed the gap: all
five global npm owners reported current while 29 findings sat in their subtrees, so the report was
not wrong about any row and was wrong about the machine, and the audit's advice named a command
that could not reach any of them. `npm outdated -g` asks about the top level; `--all` asks about
every nested node, and each entry carries the `location` that says whose tree it is in. A row per
owner now names what is behind it — reported as CURRENT and never as stale, which is the judgement
in the change: every nested node of every global tree is behind someone's latest, because a parent
pins what it was published against, so marking owners stale would make the report red forever on
every machine with a global install. What was missing was not a flag but the command, and
`apply --only npm --re-resolve <package>` is it: npm has no lock to drop, so reinstalling the owner
is what makes it resolve its ranges again. The row's note carries that command, and the entry is
filtered by the crate's own `version_newer` so a package installed AHEAD of the registry's `latest`
tag — a major line the publisher never tagged, an alpha ahead of the release — is not reported as
being behind.

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

`tools/audit` followed on 2026-10-04, and closes the loop the `updates` port opened: `sjel update
apply` runs it as its final step (`updates/report.rs`'s `run_audit`) and `tools/doctor` reads the
verdict its exit code put into the host-patch receipt, so the exit contract 0/1/2 already had two
Rust readers — which is why it was the natural next one. `tools/host-patch.sh` is the other
interpreted tool `sjel update apply` runs, delegated to because it owns brew, uv and rustup, and
it stays bash.

Verified against the script before it was replaced, on this Mac. The live run was identical — 345
installed packages, 5 crate lockfiles, and the same 36 findings across 19 globally installed
packages, exit 1 in both — as were `-h`, no argument, and an unknown argument, after normalizing
the timestamp, the per-run temporary directory's name, osv-scanner's own inode and elapsed
counters, and the table width osv-scanner derives from that temporary name. `tools/audit.test.sh`
keeps its assertions and its fixture shape, and still drives every branch through the launcher: a
linked worktree, a non-repository overlay, an unreachable one, an unconfigured one, a leak, a
gitleaks error, the SBOM's two ecosystems and its exclusion of a third, a finding in the installed
half, an unreadable inventory, a missing scanner, and a finding beside one. Five `cargo test -p
sjel-cli` cases cover what the script proved by running it: purl encoding, which rows reach the
SBOM, what counts as an inventory, and the exit precedence.

Three differences are deliberate.

- The SBOM handed to the second osv-scanner pass is built here instead of by `jq`, so this tool no
  longer needs `jq` — and a missing `jq` can no longer read as an unscanned surface. `jq` keeps
  its toolchain row: `tools/graphify.sh`, `tools/restore.sh`, the secret setup, `tools/agentbox.test.sh`
  and `.github/workflows/security.yml` still pipe JSON through it.
- JSON object keys are sorted, where `jq` wrote them in the order the filter named them. The
  `components` array keeps the inventory's own order, which is what osv-scanner reads.
- `tools/audit -h` prints the whole header comment with its `#` markers stripped, where the script
  printed only its first nineteen lines and cut off mid-sentence. Same deliberate fix the
  `toolchain-check` port recorded.

`tools/host-watch` followed on 2026-10-04, and is the first tool in this crate to open the
shared store. It was the last `bun run` job whose READERS are already Rust: `sjel-status` serves
its rows at `/api/sjel-status/host-watch` and the dashboard ranks them at band 900, while
`tools/storage report --json` and `host-net check --json` stay invoked rather than reimplemented,
because each owns a policy file and a parsing rule that a second copy here would fork.

`mod.rs` is the verb surface, the policy read, the three probes and the store write; `pure.rs`
is the pure half — the two `ps` parsers, the runaway rule, the storage and net folds, and the emission
and resolution decisions — with `tools/host-watch.test.ts`'s 34 cases as Rust unit tests (31 of them;
the storage and net fixtures are built from JSON now, which exercises the deserialization too).
The store is `sjel-store`'s, opened the way every capability opens it: `sjel_config::database_path`
for the file, `sjel_store::open_pool` for the pragmas and the once-per-database migration, and
`sjel_store::write_transaction` for the one transaction a run writes. No second declaration of
where the database is or how it is opened.

Verified against the TypeScript before it was deleted, with a fake `ps` first on PATH so both
implementations saw one frozen process list and the comparison was deterministic. `-h`, `--dry-run`
and the three write verbs were byte-identical in stdout, stderr and exit code — the create,
the refresh and the clear, each printing its own line — and the `host_watch_findings` rows left
behind were identical after all three. The `--json` payloads are equal once parsed, with the key
order difference below. A run with no policy exits 2 from both, naming
`schemas/host-watch-policy.toml.example`. Then the real paths: `tools/service-runner.sh start
host-watch` — what launchd invokes hourly — rebuilt the binary through the new `build` line and
reported `808 processes, disk ok — nothing to report`, and `sjel-status`'s
`/api/sjel-status/host-watch` served `{"findings":[]}` from the table this writer maintains.

The capability's `service.toml` changed shape, not just argv: it named `bun run tools/host-watch.ts`
and now names `target/release/sjel-cli host-watch` with a `build` line, the same shape
`sjel-status` and `punctuality` use. That is deliberate and load-bearing — `service-runner.sh`
derives the unit's PATH from `command[0]` and from each `build` word, so naming `cargo` is what
puts it on the job's PATH and lets an hourly run rebuild a stale binary. A launcher would have
left that job unable to build anything. The scheduled argv itself did not change: launchd runs
`tools/service-runner.sh start host-watch` and the runner reads the manifest, so no installed unit
needs reinstalling. `tools/host-watch` stays as the human and test interface.

Three differences are deliberate.

- `--json` object keys come out sorted, where the TypeScript wrote the object literal's order —
  the same `serde_json` map behaviour the `harnesses`, `updates` and `packs-*` ports recorded. The
  parsed payloads are equal. The one camelCase field (`cpuSeconds`) is kept, so the payload's
  field names are the ones a reader of the old output already had.
- The overlay is resolved by `sjel-config`, as it is for every capability, rather than by
  `tools/lib/overlay.ts`. That reader takes `SJEL_PERSONAL_ROOT`; the TypeScript preferred
  `SJEL_OVERLAY_ROOT`. Both are the same value in every supported invocation — `tools/lib/paths.sh`
  sets the second from the first, which is what keeps the documented per-invocation override
  working — but a shell that exports only `SJEL_OVERLAY_ROOT` and runs the binary directly would
  now resolve nothing.
- `host-net-cli` is looked for under `CARGO_TARGET_DIR` when that is set, and under
  `<root>/target/release` otherwise, which is how `tools/lib/sjel-cli.sh` already resolves the
  same directory. The TypeScript always looked in `<root>/target/release`, so with a custom target
  directory it reported "not built" while the binary existed.

The Cargo.toml records the one cost this port has that no earlier one did: `sjel-cli` links
bundled SQLite now, 4.9 MB to 6.9 MB measured on this Mac, so the binary every launcher builds on
demand is no longer pure Rust. `libs/sjel-store` and `libs/sjel-config` were both already in the
workspace lock, so this resolves no new crate.

`osv-scanner-installed.toml` followed on 2026-10-04, and is the second osv-scanner pass's config
rather than a widening of `osv-scanner.toml`. The two hold opposite policies on purpose: a known
vulnerability stays blocking for this checkout, and CI reads that file, so accepting one there
would loosen CI as well. Installed software is a different problem — `cargo install --locked`
resolves the lock the crate published, a global npm tree is resolved by whoever published the
package, an exact pin cannot be re-resolved at all, and one package on this machine is on no
registry — so the second file accepts 22 findings with their reach, their fixed version if one
exists, and the date the acceptance ends. Its header states the cost that comes with the split:
the pass runs with `--verbosity error`, so an entry that stops applying is not announced, and the
header carries the command that lists which still apply.

The same day, 36 findings became 22: reinstalling `@mariozechner/snap-happy` and
`@modelcontextprotocol/server-github` re-resolved their subtrees (14 findings, including both
9.8 `simple-git` entries), even though every crate and every top-level npm package involved was
already at its latest release. Splitting the config then exposed two informational advisories the
installed pass had been inheriting from the shared file (`paste`, `ttf-parser`), which are
repeated in the new one so the split did not become a new finding. `tools/audit` exits 0.

`inference-keys` moved the secret-bearing process work out of Node and shell: the pi extension
stays TypeScript, while key lookup, status, unlock, and keychain migration use the Rust CLI.
`tools/bw-unlock` and the Pack's `keychain-migrate.sh` keep their existing entry points. The
Node gateway, the expect writer, and the old unlock implementation are deleted; the writer now
sends hex-encoded key material to `security -i` over stdin, never in argv.

Compared with the old scripts in a scratch environment using synthetic keys and stub `bw` and
Keychain commands: keychain and vault reads, duplicate-name selection, `--check`, `--manifest`,
`bwu --status`, and unlock/session caching matched. The new migration was also exercised with
stubs: the written key verified and its plaintext cache was removed. JSON object key order differs,
and the new status payload omits the old `unlockHelper` and `failure` diagnostics; the extension
reads neither field. The API-key timeout is now one total lookup budget rather than a fresh
timeout for every fallback call, keeping the helper below pi's deadline.

`discover-ui-packages` moves the CI package-tree walk and classifier into `src/ui_packages.rs`;
CI and `tools/doctor` keep calling the same launcher, and the test-only root override remains
`SJEL_UI_DISCOVERY_ROOT`. The 12 planted-tree TypeScript cases are covered by 12 Rust tests,
including a repository-root package path case. On this checkout, the default report and `--dirs` output,
stderr and exit codes matched; scratch trees
with a UI missing its check script, and with no checkable package, also matched in both modes.
Malformed `package.json` still fails closed, but the parser's diagnostic text differs between
`serde_json` and `JSON.parse`. `serde_json` was already a direct dependency, so no crate was added.

The two remaining scheduled jobs followed on 2026-10-04, and are the last two `bun run` jobs in
the repository: `tools/feed-sweep.ts` (comms, every 6 h) and `tools/sparpreis-watch.ts` (trips and
transit, every 12 h). They came together because they are one class — a timer that starts an
interpreter to make a handful of HTTP calls and exit — and because each one's manifest needed the
same shape change `host-watch`'s got. `src/feed_sweep.rs` is the first; `src/sparpreis_watch/` is
the second, `mod.rs` for the HTTP calls and `pure.rs` for the identity, discovery and history
helpers with `tools/sparpreis-watch.test.ts`'s cases as Rust unit tests.

Verified against the TypeScript before deletion, both implementations pointed at one stub server
so the comparison was deterministic: nothing tracked was touched — the stub's ports live in a
scratch copy of the three `service.toml` files, and the TypeScript read that tree the same way the
Rust binary did through `SJEL_ROOT`. `feed-sweep` was run in four scenarios (a rich response, a
two-source payload with an unreachable source; an empty body with no `sources` key; a 500; and a
200 whose body is not JSON), and stdout, stderr, exit code and the sequence of requests were
identical in all four. `sparpreis-watch` was run over one plan carrying a stage, a legacy per-day
item to fold and a rail `option_set` to match — identical stdout, stderr, exit code, and a request
sequence equal once the two runs' wall-clock `observed_at` timestamps are normalized, including
the JSON payloads of the two item writes and the one delete.

Four differences are deliberate.

- Every request carries a timeout here, where the TypeScript used `fetch`'s default, which is to
  say none: 300 s for the scan and 600 s for the relevance page, matching the `AbortSignal`
  budgets the TypeScript already set for those two, and 300 s for each of `sparpreis-watch`'s
  calls, which had none at all. `sjel_http::client` refuses to build a client without one, and
  that is where the user-agent and the redirect policy come from as well.
- The deployment credential is put on a loopback request by
  `sjel_server::InboundAuth::with_loopback_auth` — the same helper `capabilities/calendar`,
  `transit`, `comms`, `places` and `scouting` use — rather than by the TypeScript's own
  `authorizedLoopbackRequest`, so the "never off loopback" rule has one implementation. An
  unconfigured credential is one refusal line rather than an unhandled rejection, and a failed item
  write or delete is reported and the run continues where a network-level throw ended the
  TypeScript's.
- JSON object keys are sorted, `serde_json`'s default, where the TypeScript wrote insertion order
  — the difference every port since `harnesses` has recorded. Every payload here is read by name.
- `-h`/`--help` prints the usage. `feed-sweep` did not read its arguments at all, and
  `sparpreis-watch` read only `import.meta.main`; every other ported tool answers `-h`, and a job
  invoked by hand deserves the same.

The Cargo.toml records the cost: `sjel-cli` gains `sjel-http` and `sjel-server`, and the binary
nearly doubles — 7.3 MB to 14.1 MB measured on this Mac — because `sjel-server` brings axum,
rustls, rcgen and hyper-util into the link. All of them are already in the workspace lock and
already built for the capabilities, and most of that tree (rustls, hyper, tokio, h2) is shared
with the reqwest `sjel-http` needs anyway, so a warm rebuild pays a link rather than a compile.
The alternative was a second implementation of the deployment-token read and the loopback rule in
this crate, which is the duplication this crate exists to remove.

`tools/model-check` followed on 2026-10-04, and is the last place a Rust tool started an
interpreter to do work it could do itself: `tools/doctor` ran `bun tools/model-check.ts --local
--json` unconditionally, and that leg measured 6.9 s of the doctor's 20.6 s. The doctor now calls
`src/model_check/` in-process and reads the payload it read before, field for field. `mod.rs` is
the config read, the two HTTP calls and the loop; `pure.rs` is the family and version comparison,
the catalogue and refusal parsing and the status decisions, with the cases the TypeScript never had
as Rust unit tests — it had no test file at all.

Verified against the TypeScript before deletion, both implementations pointed at one stub provider
through a scratch overlay written for the comparison, so nothing real was touched. Five runs
(`--local`, `--local --json`, `--json`, `--probe --json`, `--probe`) matched byte for byte in
stdout, stderr and exit code for every text output, and the JSON payloads are equal once parsed —
entry for entry, in the same order. Then the real path: `sjel doctor` reports
`✓ 3/3 local role(s) answering` from the payload this module builds, and went from 20.6 s to
11.8 s.

Three differences are deliberate.

- JSON object keys come out sorted, where the TypeScript wrote insertion order — the difference
  every port since `harnesses` has recorded. The `entries` ARRAY keeps the file's own order, which
  needed `pure::OrderedMap`: `serde_json`'s map is a `BTreeMap` here, and turning on
  `preserve_order` would change it for every consumer of this binary, which is the same reason
  `updates/parse.rs` carries its own order-preserving map.
- Two rules come from `libs/inference` now instead of from a second copy: `is_loopback_url` decides
  which backends `--local` sweeps — broader than the TypeScript's three-name list, since it also
  reads `127.*` and `0.0.0.0` and strips userinfo, so `http://127.0.0.1:1@evil.example` is not
  loopback — and `resolve_key_file` plus `api_key_from_file` read the credential. That second pair
  is a real behaviour change for a backend whose `api_key_file` names a `~/` path or a JSON
  settings file: the TypeScript joined `~/.omlx/settings.json` onto the config directory, found
  nothing, and reported the role `credential unavailable` without ever dialling it. No loopback
  role on this machine declares one, so nothing it reports changed.
- A role's declaration is read leniently rather than through `InferenceConfig`, which requires
  `backend` and `model` on every role and degrades the WHOLE config to empty when one is missing —
  which would turn a broken declaration into doctor's `ok`. The one thing that must never happen
  here is a config fault reading as health.

The Cargo.toml records the cost: `sjel-inference` takes the binary from 14.1 MB to 14.3 MB, and
resolves no crate — its own dependencies were already in the link.

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
   sets `SJEL_CLI_BIN` to a prebuilt binary (`sjel_cli_prebuilt` in `tools/lib/test-support.sh`, as
   the audit, persistence and service-runner suites do).
5. When only part of a tool moves, name the split in both files and route the rest through
   this binary's exec (the harnesses read verbs did this until the write verbs followed on
   2026-10-04). Delete the moved code from the interpreted original — a second reader of one
   ledger is the duplication this crate exists to remove.
