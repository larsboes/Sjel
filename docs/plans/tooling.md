# Tooling gaps

Gaps agents hit where Sjel tooling did not cover the task. The rule is in
[`AGENTS.md`](../../AGENTS.md): add an entry when you wrote a script or a workaround instead of
using a `sjel` command. When someone picks an entry up, build it or move it to a GitHub Issue,
then move it to Done here.

Entry format: what was needed, what the agent did instead, and the smallest tooling change that
covers it.

## Open

- **A launchd unit whose capability manifest is gone.** `com.sjel.tailscaled` runs
  `/opt/homebrew/opt/tailscale/bin/tailscaled` with `--statedir=<overlay>/secrets/tailscale-sjel`,
  and that userspace daemon is the tailnet node `sjel` that reaches `homepi`, the remote backup
  host. No `capabilities/tailscaled` manifest exists in either root, so `doctor` reports an orphan
  unit for a capability this machine does not enable and suggests `remove-persistence tailscaled` —
  which would drop the remote backup path at the next boot. Acting on that advice needs a check
  that asks what the unit serves, not only whether the enabled set names it. Smallest change: have
  the boot-persistence check report a unit with no manifest as `unmanaged` with its command line,
  and refuse the removal suggestion while another capability's declared remote depends on it.
- **CI cache inventory.** Inspecting Actions cache sizes and removing a merged pull request's
  unreachable cache required GitHub API calls. Add a read-only `sjel` cache report with a
  separately invoked cleanup for caches tied to closed pull requests and a dry run.
- **Stopping an orphaned process service.** `service-runner.sh stop sqld` placed a hold but
  refused to report success because an orphan still held the declared port. `lsof` identified
  the listener; the sandbox refused a direct signal. Smallest change: let the runner identify
  the listener against the declared executable and stop it through its existing orphan guard,
  while refusing an unrelated executable.
- **Package state of other hosts.** `tools/host-patch.sh` and `tools/doctor` cover only the
  machine they run on. Checking a Linux host on the tailnet for pending updates took a manual
  `ssh <host> apt list --upgradable`. Smallest change: a doctor line that runs that read over SSH
  for each host the overlay names, and reports the count and whether a reboot is required.
- **Daemons that outlive their upgrade.** `tools/host-patch.sh` upgrades a binary but does not
  restart the process that runs it, so a long-running daemon keeps the old code. An uninstalled
  Homebrew formula also left its launchd job running a deleted binary. `host-net tailnet` now
  catches this for tailscaled only. Smallest change: after the upgrade step, report each
  launchd job whose executable changed or was deleted since the job started.
- **Vault archive intake.** `tools/vault-archive-intake` copies into a new directory, verifies
  file bytes and reports excluded entries and symlinks as counts. It does not preserve file
  metadata, empty directories or excluded paths; the full vault backup remains the recovery
  source for them. Its read-only verification matched the filtered live vault and archive intake
  by normalized path and checksum. The copy operation has fixture tests but has not been run
  against the live vault. Open:
  an intake-specific backup target that avoids an extra staging copy and records exclusions in
  its receipt, plus reconciliation of excluded material.
- **Vault cutover.** The vault, state-mount and backup paths were edited separately in the
  machine overlay. Smallest change: a cutover command that validates the new vault contract,
  backup and restore, updates all three paths together, and verifies affected services reload.
- **Knowledge-vault migration comparison.** The migration used checksum comparisons and an ad hoc
  Unicode-normalized link comparison to distinguish filename normalization from deleted notes and
  identify newly unresolved links. The normalized comparison hid five filenames the copy had moved
  from NFC to NFD, and the agent found them with a second script. Smallest change: a read-only
  vault comparison that reports normalized path and content differences, names whose raw bytes
  differ, and links that resolve in the old root and not the new one, without exposing note text.
- **Runtime status for agents.** `sjel runtime status` answers
  `invalid or missing authentication token` from an agent session. An agent cannot see whether
  On-the-go mode deferred a model call. Workaround: read the defer reason from
  `/tmp/sjel-<cap>.err`. Smallest change: send the agent token, or add a read-only agent route
  for the runtime mode.
- **Timestamps in script service logs.** A Rust service stamps its own stderr from
  `sjel_server::serve` on (`libs/sjel-server/src/stderr_stamp.rs`). A `process` service that is a
  script or container still writes lines without a time to `/tmp/sjel-<cap>.err`. Smallest
  change: the runner pipes those through the same stamp.
- **Mail event outcomes by sender.** To choose the next ticket reader, the agent read
  `/triage/events/status` 20 ids at a time over all 892 mails and grouped the receipts by sender
  domain with an ad hoc Python script. Only 19 mails had a receipt, because the sweep scans new
  mail only. Smallest change: `sjel capability mail events report`, which groups receipts and
  candidate metadata by sender domain and state, and prints counts only.
- **Installed-software route for the dashboard and `ask`.** `sjel storage installed` walks every
  app bundle and its `~/Library` data and takes about 30 s on this machine, so it stays CLI-only,
  the same reason `report --where` is not served. Smallest change: sample it daily into the store
  `capabilities/sjel-status/src/storage_history.rs` already owns, and serve the last snapshot from
  `/api/sjel-status/storage-installed`, so the panel and an agent read a number instead of waiting
  for a walk.
- **Local inference server admin.** To cap oMLX's SSD cache at 4 GB, lower `max_concurrent_requests`
  and clear 26 GB of stale cache, the agent posted to `/admin/api/global-settings` and
  `/admin/api/ssd-cache/clear` through an ad hoc Python script that read the key out of
  `~/.omlx/settings.json`. `sjel model-check --local` reads that backend but changes nothing, and
  a hand-edit of the JSON loses to the app rewriting it on exit. Smallest change:
  `sjel inference admin get|set <key>=<value>` and `sjel inference admin clear-cache`, with the
  same key handling as the probe above, so the settings a deployment depends on are readable and
  durable from a `sjel` command.

- **Real names in tracked files.** A test fixture carried a real contact's full name, and no gate
  noticed. The agent compared tracked files against the people registry with an ad hoc Python
  script. Smallest change: the publication hygiene check in `tools/doctor` flags two adjacent
  registry tokens in a tracked file, and prints only the file and a count.
- **Broken relative links in docs.** When the plans moved to `docs/plans/`, the agent checked
  every relative Markdown link with a shell loop. No gate checks that a link target or its
  `#anchor` exists. Smallest change: a `tools/check-doc-links.sh` beside
  `check-doc-self-references.sh`, in the same CI repo gates.
- **Selective verification and build resources.** `tools/ci-impact.ts` plans the root Cargo
  workspace's packages for a change: the owner of each changed file, every package whose sources
  name the file's path (runtime reads, modeled from path literals), and their reverse
  dependencies. Workspace-wide inputs select every package. A file that no package owns or names
  selects none (decided 2026-10-29). `fastPlan` in the same file drives
  `sjel check` and the pre-push hook. The `cargo test` job compares the plan with the packages
  that failed on every pull request and writes misses to the run summary; it skips nothing.
  `tools/ci-impact-backtest.ts` replays CI history the same way. `bun-tests`, `ui-check` and
  `repo-gates` share a `sjel-cli-release` cache, and CI runs `tools/*.test.sh` in parallel.
  Open work:
  1. Enforce on pull requests only after the shadow step reports no miss over a period of real
     pull requests. Main keeps the complete suite. The ruleset requires the five existing checks;
     test the `verification result` aggregate before making it required.
  2. Extend the planner to the GPUI workspace, Bun and shell tests, UI packages and repository
     gates.
  3. Measure local target-directory growth and cache transfer, and benchmark bounded local
     parallelism. Set defaults only from measurements.
  Done when local and CI use the same validated planner, full main checks remain green, and
  measurements show the cost and savings.
- **Backup runs from the CLI.** To find why six backups were overdue, the agent read receipt
  files in `<overlay>/backup/receipts/` and grepped `sjel-status`'s stderr for `[backup]` lines.
  `GET /api/sjel-status/backup/runs` has the answer, but no `sjel` verb reads it. Smallest change:
  `sjel backup status|runs [--limit N]` over the existing routes.

## Done

- `tools/model-check --role <role> --probe` sends one request to one role with its configured key
  and never prints the key. A prompt file is not supported. The probe uses a fixed prompt.
- `sjel capability mail events [replay] <id>...` reads safe per-mail event scan states and
  replays them.
- `tools/service-runner.sh stop|restart` stops an orphaned copy of the capability's own binary
  that holds its port. Before, it refused and asked for a manual `kill`.
- `tools/harnesses` and `tools/doctor` read a `[unit_hold]` table from the overlay's machine.toml
  and merge it with `profiles.toml`'s, so one machine can hold a unit back without changing the
  shared file.
