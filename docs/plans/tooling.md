# Tooling gaps

Gaps agents hit where Sjel tooling did not cover the task. The rule is in
[`AGENTS.md`](../../AGENTS.md): add an entry when you wrote a script or a workaround instead of
using a `sjel` command. When someone picks an entry up, build it or move it to a GitHub Issue,
then move it to Done here.

Entry format: what was needed, what the agent did instead, and the smallest tooling change that
covers it.

## Open

- **Archive a project.** The projects capability reads a project as archived when its folder
  sits under `Archive/`, but no route or `sjel` command moves it there. `POST /api/record/property`
  can only set `status: done` or `cancelled`, so a closed project stays in the live list. Smallest
  change: `POST /api/projects/archive` that moves the folder under `Archive/` and reindexes it.
- **Harness intake review, live.** `tools/intake review --reviewer claude` passed only against a
  fake `claude` in `tools/skills.test.sh`. A real `claude -p` run has not happened. Unverified:
  the CLI accepts `--setting-sources project` together with `--json-schema`, and the field that
  holds the answer (the parser reads `structured_output`, then JSON in `result`). Smallest
  change: run it once on a held stage and fix the flags it rejects. Also open: Codex as a second
  reviewer in `review::HARNESSES`, and a timeout on the harness run.
- **One update surface.** `sjel update` reports package classes but no skills,
  `tools/skills` has no update check, and `tools/harnesses` owns Packs. Smallest change: one
  `sjel update` entry point that delegates skills to `tools/skills check|update`, Packs to
  `tools/harnesses`, and the rest to its existing surfaces, with one report. Blocked by the
  read-only check below: `tools/skills check` installs today, so the delegate must not call it.
  The per-owner verbs stay the implementation; the skill `sjel` carries the route in a reference
  rather than a second skill.
- **Read-only global skill update checks.** `tools/skills list --global --json` inventories
  installations but has no update-check verb. The upstream CLI exposes `update` without a
  documented dry-run mode, and `sjel update` reports Interceptor integration state without
  checking the latest release. Workaround: compare Anatomy's global lock hash with upstream
  Git HEAD, and compare the installed Interceptor version with its public appcast; its normal
  check can open a native update window. Smallest change: `tools/skills check --global --json`
  reports installed and available revisions through each owner's read-only metadata, without
  installation or focus changes. Link this inventory to P7 in [`packs.md`](packs.md).
- **Clipper browser smoke checks.** `tools/clipper build` compiles the extension and `check`
  pings its native host; neither verifies a live page capture. YouTube verification required
  reloading the extension and inspecting its popup through native Chrome controls because
  `interceptor contexts` and `tab list` exited without output. Smallest change: a smoke-check
  command that reports the loaded extension revision and verifies transcript availability,
  description and optional comment variables on a controlled browser fixture.
- **Cited web answers for agents.** Decided 2026-11-03: no Sjel command for Gemini Search
  grounding. Google refused grounding on the free tier (HTTP 429 on `gemini-3.8-flash` and
  `gemini-3.6-flash`; `gemini-2.5-flash` is closed to new users), and the Gemini API terms
  forbid caching, analyzing or learning from grounded results even on the paid tier
  (https://aistudio.google.com/docs/terms, "Grounding with Google Search"). Driving the AI Studio
  chat with an agent is excluded too: its `robots.txt` disallows `/prompts/` and `/app`. The
  `research` skill (Packs/knowledge) routes instead: built-in web search for quick facts,
  `interceptor-research` for wide investigations, and a drafted prompt that the user runs in
  AI Studio and clips. Open: the prompt outbox (`data.md`, "Prompt outbox").
- **Reading the newest clip.** `sjel search clip` found nothing, and no `comms` binary was on
  `PATH`. Workaround: `cargo build --bin comms -p sjel-comms`, then
  `target/debug/comms clips --limit 5` and `clips show <key>`. Smallest change: install `comms`
  with the other user-facing binaries, and add a `sjel` command alias for `clips`.

- **`tools/packs-pi deploy` ignores the active profile.** Deploying `harness` and `deliberation`
  to refresh pi's agent files re-added every skill in both Packs, so pi went from the `lean`
  profile's 7 skills to 14. Workaround: re-run `tools/harnesses use lean --harness pi`, which also
  rewrites the agent files. Smallest change: `deploy` keeps the active profile's skill subset.
- **Installed crates whose source was collected.** `tools/audit` reads each installed crate's
  `Cargo.lock` from `~/.cargo/registry/src`, and a cache trim can take that whole tree while
  leaving `registry/cache`, so the globals pass scanned 0 crate lockfiles and reported clean.
  Smallest change: fetch a missing crate source before the scan, or report each installed crate
  without a lockfile as not scanned.
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
  `sjel check` and the pre-push hook. On a pull request, the `cargo test` job tests only the
  selected packages; main and any planner error test the whole workspace.
  `tools/ci-impact-backtest.ts` replays CI history and reports a failing package a plan skipped. `bun-tests`, `ui-check` and
  `repo-gates` share a `sjel-cli-release` cache, and CI runs `tools/*.test.sh` in parallel.
  Open work:
  1. Run the backtest after a few weeks of enforced pull requests. A main run that fails in a
     package the pull request's plan skipped is a miss: repair the planner, or widen.
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
- **`tools/doctor` in a worktree reports the repository as its own undeclared connection.** The
  undeclared-connections sweep derives the repos that count as "self" from the *basename* of the
  checkout (`tools/sjel-cli/src/doctor/checks.rs`, `undeclared_connections`: `self_roots` is
  `basename(ctx.root)`), which is `Sjel` in the primary checkout and `host-health-reclaim-retry` in
  a worktree. Every tracked path naming this repository then reads as a reference to a sibling
  repo with no `systems.toml` entry, so a worktree fails three checks that pass on `main` and the
  same three files are named regardless of what was edited. Smallest change: resolve self from
  `git rev-parse --show-toplevel` plus the *common* git dir (`git rev-parse --git-common-dir`), so a
  worktree inherits the primary's identity instead of its own directory name.

- **Pi TUI math probe.** Checking what Pi's terminal renders needed an ad-hoc `node -e` call
  to `renderLatex` in the installed `pi-tui/dist/latex.js`. A second ad-hoc call loaded the
  obsidian-access extension against a stub API to print its live `promptGuidelines`. Smallest
  change: a `tools/pi-render` command that prints the Unicode output of a LaTeX string (inline
  and display) and the prompt guidelines that the configured Packs register.

- **Obsidian plugin typecheck and live RPC check.** `tools/obsidian-plugin` builds with
  `bun build`, which does not typecheck, and Sjel has no `obsidian` type package. Checking
  `pi-learn` needed a throwaway directory with `obsidian`, `typescript` and `@types/node` and a
  strict `tsc` run against `src/`. Its live check was a `/tmp` script that started
  `pi --mode rpc` through the plugin's `PiRpc`, asked for one `quiz` and answered the
  `pi-questions/v1` dialog. Smallest change: `tools/obsidian-plugin <name> check` for the
  typecheck, and an opt-in `tools/pi-learn.test.ts` case for the round trip, gated on an
  environment variable because it spends model tokens.

- **Obsidian view smoke check.** Checking the Learn view needed Obsidian relaunched with
  `interceptor macos cdp launch Obsidian --port 9333 --confirm`, then ad-hoc `interceptor eval`
  calls that reloaded the plugin, captured `console.error`, opened notes and read the header.
  That found the `titleEl` defect that every unit test passed. Smallest change: a
  `tools/obsidian-plugin <name> smoke` verb that attaches to a running debug port, reloads the
  plugin, opens its view, and fails on any console error.

## Done

- Dependabot watches `/apps/gpui` (`.github/dependabot.yml`). It ignores the coupled
  `gpui-kit`, `gpui-pre` and `gpui-mobile` set, which moves by hand per
  [`gpui.md`](gpui.md#6-one-ui-crate-every-platform). The `verification result` check is
  required on `main`, so a GPUI bump that fails its GPUI check does not auto-merge.
- `sjel demo app [page]` runs the Mac app against the seeded demo node, to debug on
  synthetic data next to the live node. See [`gpui.md`](gpui.md#10-dev-and-prod).
- `tools/intake review <dir> --source <name> --reviewer claude` runs a fresh `claude -p` with
  Read, Glob and Grep in a directory that holds only the staged text files, and records a receipt
  with the harness, model, digest and findings. The gate uses that receipt in place of the model
  review for exactly that digest. A medium or high finding still holds until `approve`.
- `projects put <path> <file> [--base <file>]` creates a record from a file, also in a subfolder,
  and replaces one only while the stored content still equals the base file.
- `inventory price` records a dated price on a wish by hand, and the daily `wish-prices` job
  (`inventory prices refresh`) records each wish-group candidate's geizhals product page. See
  GPUI WP 28 in [`gpui.md`](gpui.md#4-work-packages).
- `tools/audit` reads an installed crate's published `Cargo.lock` from the `.crate` archive in
  `~/.cargo/registry/cache` when the extracted `registry/src` copy is gone, and names every crate
  whose lockfile survived nowhere as not scanned, which exits 2 rather than reporting clean.
- `tools/model-check --role <role> --probe` sends one request to one role with its configured key
  and never prints the key. A prompt file is not supported. The probe uses a fixed prompt.
- `sjel capability mail events [replay] <id>...` reads safe per-mail event scan states and
  replays them.
- `tools/service-runner.sh stop|restart` stops an orphaned copy of the capability's own binary
  that holds its port. Before, it refused and asked for a manual `kill`.
- `tools/harnesses` and `tools/doctor` read a `[unit_hold]` table from the overlay's machine.toml
  and merge it with `profiles.toml`'s, so one machine can hold a unit back without changing the
  shared file.
- `tools/doctor` reports a launchd unit with no capability manifest in either root as unmanaged,
  names the program it runs, and does not suggest `remove-persistence`. Before, it suggested
  removing `com.sjel.tailscaled`, which the remote backups use.
- `tools/doctor` warns about a command name with two owners across `tools/lib/tool-dirs`, which
  `host-patch.sh`, `backup-all.sh` and `container-refresh.sh` now share as their PATH prefix.
- `tools/audit` prints the exceptions in `osv-scanner-installed.toml` that match nothing installed.
- `sjel ci-cache` reports the Actions caches against the 10 GB limit and marks the dead ones:
  superseded by a more recently restored entry with the same key prefix, or tied to a closed pull
  request. `sjel ci-cache prune` previews the deletion and `--apply` performs it.
