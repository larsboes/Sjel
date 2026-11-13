# Tooling gaps

Gaps agents hit where Sjel tooling did not cover the task. The rule is in
[`AGENTS.md`](../../AGENTS.md): add an entry when you wrote a script or a workaround instead of
using a `sjel` command. When someone picks an entry up, build it or move it to a GitHub Issue,
then move it to Done here.

Entry format: what was needed, what the agent did instead, and the smallest tooling change that
covers it.

## Harness delivery

Claude Code Pack installation uses the profile-aware harness adapter only. The native marketplace
catalogue, per-Pack manifests, generator and freshness gate are retired; project hooks and the
automatic-memory setting remain in `.claude/settings.json`. Attribution references to upstream
plugin metadata remain intact. CI selection regressions cover the remaining gate registry.

## CLI installation and release delivery

Decided 2026-11-09: deliver a checkout-free CLI and coding-harness installation before desktop
release work. Direct installations notify about updates and apply them only on explicit request.
Preview releases use successive `v0.0.x` tags; existing tags remain immutable. A bounded release
checklist, not completion of `PLAN.md`, gates `v0.1.0`. This does not change the primary-checkout
and branch policy.

Delivered as draft `v0.0.2` at `fca5dd79`: coding-only resources, frozen production extension
dependencies, explicit updates, integrity checks and retained-version recovery (`7b219631`).
The root README and Operations guide document authenticated draft downloads, platform selection,
prerequisites and explicit profile activation; installer behavior and tagged assets are unchanged.
The distribution workflow targets Ubuntu 24.04 x86-64/ARM64 and macOS 15 ARM64 and runs
`cli-smoke.sh` in an isolated home. Commands, requirements and the HTTPS/checksum trust model are owned by
[Operations](../../website/content/operations/index.md#cli-only-coding-installation).

The existing Bun distribution owner was extended rather than introducing another release
framework. Bootstrap remains Bash so installation needs no developer runtime. No new release
framework is adopted; cargo-dist remains an option only if maintaining this path proves costly.
The CLI installation does not start services, generate credentials or deploy a harness profile.
Registered Pack paths remain stable across updates; materialized copies require explicit,
drift-checked reconciliation. Runtime recovery does not imply database rollback.

### Standalone installer for the next preview

The public entry point uses GitHub Releases directly: `releases/latest/download/install.sh`
and `releases/download/v0.0.3/install.sh`. No separate hosting or Pages routing is added.
`prepare.ts` renders a POSIX stdin entry point around the existing Bash lifecycle installer,
with the artifact version as its default. The versioned runtime retains latest-version discovery
for later explicit updates. Shared bootstrap assets are checksummed and merged across platforms.
The command appears directly below the README introduction and in the website hero, with an
adjacent publication warning and links to installation options. The hero's primary action leads
to the installer rather than source/demo setup. Browser layout checks place the command fully
inside desktop (1512×827) and narrow (375×844) viewports, with no page-level horizontal overflow.

Curl and GNU wget are supported without disabling TLS verification or allowing HTTP redirects.
Downloader configuration files are ignored. Prerequisites are checked before creating the install
root; PATH instructions do not edit shell configuration. Truncated script bodies never activate
an installation. The distribution workflow rehearses the generated entry point and defaults to
`v0.0.3`; `v0.0.2` assets remain unchanged. A new tag or public release has not been created.

Verification: 38 installer/packaging/publication/notice tests with 1045 assertions pass on macOS
and network-disabled Linux ARM64. Three-platform rebuilt bootstrap/install/update/rollback CI
passes at `b33ad149` (`42654301653`), as does full CI (`42654299933`). Installed Pi 1.5.0 loads
all four registered extensions and both packaged extension entry points, runs startup handlers,
and registers the packaged tools without a model request. Synthetic load and startup failures
are rejected. The check also passes with SDK peers omitted from a fresh dependency installation.
`cli-smoke.sh` enables this independent
SDK check when `SJEL_PI_PACKAGE` names an existing Pi installation; its child receives only an
isolated HOME, PATH and smoke settings, not host authentication or model configuration.

Public builds include complete Rust dependency notices under `THIRD_PARTY/rust` and a
`THIRD_PARTY.md` index. A separate platform-specific source archive carries the committed source
and complete cached source for the compiled Rust dependency graph. Existing JavaScript notices
remain inside the bundled packages; missing or unreviewed grants fail packaging. The missing
`boolbase@1.0.0` ISC notice is restored from its pinned upstream grant. SDK peers are omitted,
so an installed harness supplies its own API rather than receiving another bundled harness.
The inventory consumes the actual build's Cargo `compiler-artifact` records, including cached
artifacts and workspace-unified features. A selected-package tree omitted compiled dependencies;
`a3e6814d` artifacts are therefore not publication-eligible despite passing lifecycle tests and
full CI. Preflight against that ARM build's compiler log accounts for 374 compiled packages and
355 registry sources. Five Rust crate archives omit grants; version-scoped upstream supplements
restore them, including Xberg's native-library inventory. Source paths outside the checkout and
unrecognized registries fail the build. Fresh corrected three-platform artifacts and public-channel
acceptance remain pending. Actionlint is unavailable locally.

### Open acceptance before publication

1. Fresh three-platform CI must verify the final notice/source-bearing artifacts. Real-binary
   isolated-home installation, update and rollback pass on all supported platforms; real installed
   Pi loading passes on macOS. Older glibc and musl targets are not supported by the Ubuntu 24.04
   build claim. Check generated metadata against the candidate's committed inputs, isolated from
   concurrent working-tree edits.
2. Publishing remains explicit. Verify official-channel discovery and one real download/update
   after publication; mocked HTTP checks do not establish live-channel acceptance. No independent
   release-signature system is implemented; current authentication relies on GitHub HTTPS and
   trusted checksums.
3. Reassess third-party source/notice obligations for bundled production dependencies before
   public distribution. Keep `friend_ready: false` until the applicable clean-user checks pass.
   CLI delivery does not wait for desktop readiness; desktop first-run and recovery acceptance
   remain owned by [#235](https://github.com/larsboes/Sjel/issues/235).

The change-workflow reference, `AGENTS.md` and `CONTRIBUTING.md` now keep release tags independent
of the branch workflow. Primary-checkout work continues until `PLAN.md` is done.

Verification at `fca5dd79`: full CI (`42624847992`), Security (`42624849577`), CodeQL
(`42624849319`) and three-platform real-binary build/install/update/rollback smoke (`42624852266`)
pass. All three downloaded archive checksums match their clean, rebuilt source catalogues.
The annotated tag and seven uploaded draft assets are verified; public publication has not occurred.
The documented GitHub draft download and local installation commands pass in an isolated macOS home,
including version output, harness listing and installation status. Pi loading acceptance now passes
against the subsequent `v0.0.3` artifacts as described above.

Uninstall and retained-version pruning are not implemented. Lifecycle additions require a separate
specification and a new immutable preview version.

## CI integration

The shell-test job installs uv through the pinned official action. Permission-gate tests use a
remote write that still requires approval, not newly permitted local commits. Storage process-group
termination separates options from its negative group identifier for GNU and BSD kill. Focused
agent/status Rust suites pass (65 and 112 tests), and CI-selection regressions pass (113 tests).

Bundled SQLite is upgraded to the patched `libsqlite3-sys` through rusqlite. `sjel-store` keeps
r2d2 but owns the file-backed manager because the external adapter prevents the dependency upgrade.
Its 14 tests and strict Clippy pass; the root lockfile security scan reports no unfiltered issues.
Full CI and security scanning pass at `fca5dd79`, including the iterator-conversion lint fix,
complete shell-fixture listing consumption and removal of the marketplace-only CLI route.
Release and update previews drain the full history stream when capping
commit output. A 200-commit fixture checks fifteen displayed rows, the explicit remainder and
successful completion under errexit and pipefail.

## Signed macOS helper

`sjel fda-launcher` owns the stable Application Support installation, mandatory signing,
source/integrity receipt, launchd permission probe, and guarded migration out of the overlay.
The existing shell installer remains the macOS platform boundary; the CLI and persistence
renderer use it rather than duplicating its path or signing policy. Contract:
[`signed helper`](../../tools/fda-launcher/README.md).

Verification: isolated lifecycle tests, two Rust persistence tests, CLI build, and formatting
pass. Live migration and protected-folder access under launchd pass; the old executable is
removed, and the switched status capability answers its health endpoint. Synthetic regressions
cover non-Sjel consumers and inherited user-domain jobs. Obsolete privacy-panel rows require
manual cleanup without resetting unrelated grants. Broader Rust suites, packaged helper
installation, and alternate-platform execution remain unchecked. The node
archive includes the managed path resolver; its extracted-path regression passes.

## Open

- **Read web clips from the CLI and MCP.** Sjel Clip saves into comms over
  `data/comms/clip.sock` (`crates/comms/src/web_clip_socket.rs`), and `sjel search clip` finds no
  route to list or read them. Comms `/captures` and `/feed` do not hold clips. A session read one
  with a hand-written length-prefixed JSON client. Smallest change: a `sjel clips [list|show <key>]`
  command over the socket's `documents` and `document` requests, plus a read-only MCP route.
- **Stage only this session's hunks.** Other sessions leave uncommitted edits in shared files
  (`upstreams.toml`, this plan). Committing one session's change needed a hand-built reverse
  patch against a snapshot of the file. Smallest change: snapshot a file at a session's first
  edit and stage the difference from that snapshot.
- **Regenerate `ARCHITECTURE.md` from the index.** `tools/generate-architecture.sh` reads the
  working tree, so another session's uncommitted `upstreams.toml` rows enter the output.
  Smallest change: an option that reads each input with `git show :<path>`.
- **Show which session holds which work.** `PLAN.md` says to take work no other session holds,
  but nothing records a hold. On 2026-11-09 one session migrated the phone's remaining screens
  while another decided the phone's renderer. Smallest change: a hold record per area plan that
  `sjel` lists and `tools/doctor` reports when it is stale.

- **Overlay ignore-policy compatibility — deferred.** The shared floor excludes the machine
  manifest and all runtime-data paths, while existing private overlays deliberately version
  selected instance manifests and source assets there. Adopting it unchanged would conflict
  with those tracking contracts. Define the versioned-input/runtime-state boundary before
  migration; preserve reviewed source tracking and prove that secret and runtime exclusions
  still win. Do not silently weaken the floor or change an overlay's tracking set.

- **Orphan build roots outside the checkout — deferred.** Storage inventories the selected
  Cargo target and secondary targets inside the public checkout; a retired sibling-overlay
  cache has no route that selects only that orphan. A scoped manual removal checked tracked
  files, open files, launcher references, and the surviving active binary first. Add an
  explicit-root guarded inventory/reclaim route only if another such cleanup is needed;
  reject active targets, source/state trees, and symlink roots in its regression tests.

- **Native-plugin activation control.** Sjel's Pack holds do not cover native marketplace or
  account-managed plugins. The scoped cleanup used Claude's native disable command; Codex exposes
  removal but no persistent disable command, so its named plugin enablement was edited directly.
  Smallest change: inventory native activation alongside Packs, distinguish approved native
  dependencies from owned and unapproved components, and add guarded activation/disable routes.
  Bind approvals to component/host and the reviewed trust boundary; source-porting rejection is
  not a native-use verdict. Routine upstream updates are delegated, with material scope changes
  requiring re-review. No native enforcement is implemented by the policy change. Prove non-target
  settings remain unchanged; do not substitute uninstall for disable.

- **Unit-scoped harness sync.** `tools/harnesses sync` selects a whole Pack. A coding sync would
  also deploy unrelated concurrent Rust guidance changes. Pi already reads the changed owned
  source through its registry; materialized destinations are intentionally not refreshed in this
  change. Smallest change: add an explicit unit selector with existing profile and drift guards,
  and prove sibling destinations and ledgers remain unchanged.

- **Controlled owned-skill comparisons.** The existing eval runner launches native Claude
  processes with other installed plugins and compares project installation against no local copy,
  not one owned revision against another. This adoption used fresh owned Pi agents with immutable
  before/candidate guidance and synthetic prompts; common inherited context remains a limitation.
  Public responses and source identities live in the skills' eval fixtures. Smallest change:
  support explicit baseline/candidate snapshots and a controlled owned runner. The current
  native runner now records the actual UTC date, validates exact grading coverage, includes
  grading costs and supports opt-in descriptive repeats with separate sample identities.
  These changes do not isolate installed context or test runtime triggering. The security
  fixture probe also encountered a checkout-first review actor whose mandatory Git inspection
  conflicted with the requested no-tools scope; its semantic answers are not a tool-isolated
  result. Controlled evaluation needs explicit actor/context and tool boundaries, with an
  incompatible fixture-only invocation rejected before checkout access. Keep live workspace
  data out of inputs; public receipts omit the unrelated checkout diff.

- **Non-mutating command help.** `tools/harnesses apply --help` was needed to inspect options,
  but applied every bound profile instead. The observed side effect deployed the already-selected
  Sjel-owned `vendor-audit` skill to Claude, Codex, OpenCode and Copilot. Do not use nested help
  on this verb until fixed. Smallest change: handle help and reject unknown options before any
  write, with tests proving help leaves settings, destinations and ledgers unchanged.

- **Owned-component deployment eligibility.** Intake screening and store pins do not distinguish
  reference snapshots from intentionally maintained adaptations. Current workaround: explicit
  profile exclusions and global holdbacks for unchanged Matt and Anatomy store Packs, plus
  native plugin controls. Whole-Pack exclusions are needed because the Pi path-hold matcher
  does not apply the materialized adapter's trailing-slash prefix semantics.
  Smallest change: an ownership/adoption eligibility contract checked by install and deployment
  entry points, including wildcard profiles; reference-only or unknown ownership must not become
  active by being present in a store. Include application and account discovery in the audit.

- **On-demand disk explorer.** `sjel storage browse` and the Systems storage explorer share
  `status-api::StorageDirectorySnapshot`: one allocation walk, immediate-directory drilldown,
  explicit unlisted bytes, omitted counts and partial errors. The status route caches at most
  32 roots in memory, runs one scan at a time with a timeout, and starts discovery only on an
  explicit POST. No scheduled discovery is authorized. Systems reads cached state, retains old
  snapshots during refresh, and compares only complete readings. Policy classes are no longer
  presented as a summed reclaimable total; allocation outside the reported volume is shown
  separately. Verification: 124 storage tests, 5 shared-contract tests, 112 status tests,
  26 core Systems tests and 8 UI Systems tests pass; scoped backend Clippy passes with warnings
  denied. A live CLI volume scan returned a partial snapshot with its permission warning.
  Focused independent review passed. Installed CLI smoke and status cache-read checks pass;
  the signed desktop app builds, installs and restarts. Its installer now restarts with the
  existing no-activation flag and can reuse the prebuilt release binary. Pending: native visual
  and interaction acceptance; full repository gates did not run. App/tool/container ownership
  adapters remain a later slice.

- **Safe process restart ordering.** Host process restarts take a non-expiring restart hold,
  stop before building, and release that hold only after a successful build. Failed stops do not
  build. Successful retries preserve any ordinary operator hold; explicit resume releases both.
  Ordinary maintenance holds retain their 30-minute expiry. Restart holds live in deployment
  state, not boot-cleared temporary files. Process lifecycle calls serialize through a standard
  file lock, released before launchd re-enters the runner. The launcher refuses restart through
  stale CLIs that lack the safety protocol. Startup retry behavior remains unchanged after a
  successful build. Verification: 14 runner tests, both service-runner shell suites and strict
  CLI Clippy pass. The delayed-handoff regression fails against the old implementation and
  passes after the fix; focused review passes. Fresh-checkout profile exclusions now accept absent
  globally held Packs while retaining unknown-name errors; fresh Rust test execution remains in
  CI. Both shell suites and local repository gates pass. Verification did not restart live services.

- **Verified import-source disposal.** Import records successful intake but has no owner-aware
  source cleanup command. The bounded workaround reads only successful media ledger rows,
  verifies source and stored-original SHA-256 plus stable regular-file identity, records a private
  receipt, and sends only matching source files to the platform Trash after explicit approval.
  Excluded paths and directories are never disposal candidates. Smallest change: an Import
  cleanup plan and explicit apply command using the owner's verified lookup, with stale-source,
  missing-original, symlink and partial-Trash-failure cases. Permanent deletion remains excluded.

- **Verification build footprint.** `sjel` verification guidance requires checkout-scoped cache
  reuse, guarded retention for justified separate targets, and non-incremental builds only for
  genuinely one-shot checks. The deployment's existing two-day scratch policy also covers the
  two confirmed verification targets missed by its naming patterns; release locations remain
  unchanged. Acceptance: wrapper regressions and skill validation pass; a reclaim dry run
  measures both roots without selecting recent entries. Pending: observe expiry and volume-space
  recovery after the retention interval; no cleanup or service migration is part of this change.

- **Maintained scratch replacements.** Calendar exact-copy maintenance is owned by
  `crates/calendar/src/google_sync/dedupe.rs`, exposed through `sjel calendar dedupe`;
  installed Obsidian plugin inventory is `tools/vault-plugin-audit`, with deployment remaining
  in the existing plugin tools. The old one-off scripts used fixed private paths and historical
  cleanup assumptions. Their private reports and originals must not enter public source.
  Synthetic tests cover exact-content selection, stale revisions, repeat-apply refusal and plugin
  inventory. A configured-calendar audit fetched 2 events and found no exact groups; nothing was
  deleted. The live plugin inventory reported two installed, unmapped plugins as incomplete rather
  than a successful upstream check. Conditional deletion against explicitly approved synthetic
  remote events remains untested. No real calendar deletion or old plugin-settings migration is
  authorized by the replacements.

- **Approval and delegation proportionality.** `CONTRIBUTING.md#proportionate-approvals-and-delegation`
  owns the instruction, linked from `AGENTS.md` for every harness. Repeated routine approval
  cards require a matched-rule and side-effect review, then a narrow regression-tested change,
  not a blanket tool grant. Failed or redundant delegation is evaluated before more agents start.
  No permission rule has been loosened without evidence. Acceptance: identify a repeated routine
  rule from actual usage, reduce its unnecessary cards without admitting harmful fixture cases,
  and compare usable delegation output against its recorded cost.

- **Cross-root build-output comparison.** Directory totals do not establish byte identity across
  Cargo profiles or build configurations. Workaround: compare artifact names and sizes, then hash
  matching candidates locally without changing either tree. A standalone deduplication command is
  deferred while cache reuse addresses repeated verification builds. Smallest future change:
  extend `storage target` with a read-only cross-root comparison that reports exact identity
  separately from recoverable space; artifact deletion or relinking remains outside that report.

- **Standalone tool installation from isolated builds.** A restricted session could build storage
  in a writable scratch target but could not clean temporary archive directories in the shared
  Cargo target. Workaround: build through the existing launcher with an isolated target, check
  executable use, and atomically install hash-verified bytes at the CLI and scheduled artifact
  locations. Smallest change: a tooling install command that derives both locations from existing
  launch and service contracts, refuses active executables, and verifies the installed digest.
  Done when one command installs the verified tool without rebuilding in the shared cache.

- **Duplicate behavior discovery.** The maintenance report inventories contracts and unresolved
  path candidates, but does not estimate duplicate code. Finance and Inventory now reuse their
  API contracts. Run any future clone detector as an optional discovery tool with generated,
  vendored and fixture exclusions; inspect shared behavior before extracting another module.
  Knip and jscpd are candidates, not mandatory gates or established coverage.
  The [compiler trial](../../research/benchmarks/maintenance/README.md#typescript-cleanup)
  covers unused declarations; public exports and cross-file clones remain outside that check.

- **Native iOS screen validation.** Compile checks and screenshot-only touch tests did
  not detect an empty Documents host on a physical phone. `ios/run-sim --smoke`
  checks visible Documents and Mail headings through XCTest and exports screenshots
  even when a test fails. The `iOS screen smoke tests` workflow runs the same command
  on an Apple Silicon Mac with simulator services. The agent shell cannot connect
  to CoreSimulator; native Device Hub control can boot and inspect a simulator.
  The shared NSH host applies adaptive foreground and background colors
  (`0f638ba7`); native smoke tests select real light and dark simulator appearances
  (`bd0fe27d`). All [six iOS 27 checks](https://github.com/larsboes/Sjel/actions/runs/42519242500)
  pass, covering Documents, Mail and navigation after Today. The pinned headless
  driver also covers contrast, tab switching and a 500-row page and reader.
  Apple builds and checks target only iOS 27 and macOS 27 on the Xcode 27 image;
  app manifests own the minimum versions. Open: confirm the updated physical build
  and provide shell access to host simulator services without manual intervention.


- **Redacted outgoing secret-scan findings.** The pre-push hook reports a finding count
  without its location. Workaround: run `gitleaks git --redact=100` over the outgoing
  range, write a JSON report, and extract only rule, path, line and commit metadata.
  A synthetic redaction fixture was identified without displaying its value.
  Smallest change: `sjel audit --outgoing --json` returns this metadata without match
  or secret fields. Done when a fixture finding can be diagnosed through the normal
  audit command without exposing its matched value.

- **Full local CI replay remains unverified.** The isolated CLI suite, nested-build
  regression and hermetic-wrapper suite pass. Explicit targets reach nested Cargo
  consistently (`b2fc9ed1`). A later broad run completed the remaining Rust suites,
  but its wrapper exited during a concurrent edit; it is not a full-replay receipt.
  Four native macOS sandbox integration tests require an unrestricted host;
  their security assertions remain intact. The same revision's
  [full Actions run](https://github.com/larsboes/Sjel/actions/runs/42511020140)
  passes Rust tests, Bun/shell tests, UI checks and all five platform compile checks.
  Remaining acceptance: verify the native integrations and full replay on an unrestricted host.

- **Pre-push checks mix committed and unfinished changes.** `ci-local fast --committed`
  selects committed inputs but executes formatting and gates against the current
  worktree. Concurrent travel edits therefore blocked an unrelated push.
  Workaround: verify the exact outgoing revision in a clean temporary local clone
  using `ci-local fast --committed --base <remote-sha>`, then retain the hook's secret
  scan and publication gate while skipping its duplicate worktree checks.
  Smallest change: let `ci-local fast --revision <sha>` verify an immutable snapshot
  and bind a successful receipt to that revision.

- **Stale path check after a move.** After the move to `crates/` and `services/`, nothing
  reported that `cv` and the gitleaks allowlist still named `capabilities/` paths. Workaround: a
  one-off script that rewrote `capabilities/<name>` where a crate or service of that name exists,
  in Markdown and comment lines only. Smallest change: a doctor check that fails on a
  repository-relative path in tracked code or config that does not exist.

- **Unpaired and per-tab phone screenshots.** `ios/run-sim` installs on the first simulator and
  takes one screenshot of the launch tab. Workaround: run it with `SJEL_IOS_SIM` set to a never
  paired simulator, then loop `simctl launch` with `SIMCTL_CHILD_SJEL_TAB` and
  `SIMCTL_CHILD_SJEL_SIDEBAR=off` per tab. Smallest change: `ios/run-sim --tabs today,mail,…`
  writes one screenshot per tab, and `--unpaired` picks or creates a simulator with no pairing.

- **Sandboxed GPUI build target.** Cargo could not write the configured shared target or a
  target below the checkout. Workaround: pass `--target-dir /private/tmp/sjel-calendar-target`
  to native Cargo checks and tests. Smallest change: let the build tooling select and report
  a writable target directory for sandboxed agent sessions.

- **Scoped commits with a shared index.** Calendar changes shared plan files with unrelated
  edits while another session had staged a file. Workaround: build a temporary Git index from
  HEAD, stage the owned paths and plan hunks, commit it, then refresh only those paths in the
  primary index. Smallest change: a scoped commit command that preserves unrelated staged
  files and rejects a changed HEAD.

- **Harness intake review execution.** The real Claude review and receipt reuse passed on
  Anatomy on 2026-11-08. Open: Codex as a second reviewer in `review::HARNESSES`, and a timeout
  on the harness run. The accepted CLI flags and output parser no longer need a live probe.

- **Project reconciliation export and revision checks.** A reviewed cleanup used a local tar
  of the rendered project tree, checked record revisions, then wrote and removed records through
  Projects. Pseudonymized content could not round-trip as an exact `base`; the retained raw
  projection supplied that base without exposing it to the prompt. Smallest change: an owner
  export with a manifest and opaque revision-checked updates/removals, so reconciliation needs
  neither a shell archive nor a raw-content base. Attachments need an owner migration contract.

- **Restricted skill deployment preflight.** Directory renames under harness homes were denied
  after staging. `35d866ed` makes replacement errors explicit and verifies destination bytes
  before recording success. Workaround: finish the atomic installation in an unrestricted
  terminal, which completed all four Anatomy copies; do not publish partial copies. Smallest
  change: probe the required rename capability
  before materializing a large Pack and report the failing destination and retained stage.
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
  `web-research` skill (Packs/knowledge) routes instead: built-in web search for quick facts,
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
  `sjel_server::serve` on (`crates/sjel-server/src/stderr_stamp.rs`). A `process` service that is a
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
  `crates/sjel-status/src/storage_history.rs` already owns, and serve the last snapshot from
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
- **Selective verification and build resources.** The shared planner, selected runners,
  daily/release full verification and local format receipts are published; commands
  live in `CONTRIBUTING.md#change-aware-ci`. Ordinary pushes select affected work;
  full verification runs daily and before release artifacts.
  Open acceptance checks:
  1. Observe a selective Actions run, a scheduled daily full run and distribution
     preflight. A [push-triggered full run](https://github.com/larsboes/Sjel/actions/runs/42511020140)
     passes Rust tests, formatting/Clippy, Bun/shell tests, repository gates, UI
     checks and all five platform compile jobs. Native device acceptance remains
     separate from compilation.
  2. Continue `bun tools/ci-impact-backtest.ts` against history containing failing Rust tests.
     The current 20-run sample yielded 19 usable runs and no failing Rust job, so its zero misses
     do not establish failure-detection coverage.
  3. Measure target-directory growth, cache transfer and runner time before expanding successful
     result reuse beyond local formatting. Set resource defaults only from measurements.
  Done when local and CI share the same planner, live full verification passes, and measurements
  establish the cost and savings.
- **Backup runs from the CLI.** To find why six backups were overdue, the agent read receipt
  files in `<overlay>/backup/receipts/` and grepped `sjel-status`'s stderr for `[backup]` lines.
  `GET /api/sjel-status/backup/runs` has the answer, but no `sjel` verb reads it. Smallest change:
  `sjel backup status|runs [--limit N]` over the existing routes.
- **`tools/doctor` in a worktree reports the repository as its own undeclared connection.** The
  undeclared-connections sweep derives the repos that count as "self" from the *basename* of the
  checkout (`crates/sjel-cli/src/doctor/checks.rs`, `undeclared_connections`: `self_roots` is
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

- **Instrument ticker lookup.** Setting `ticker` on a finance instrument needed a symbol search
  and a check that the listing matches the holding. Workaround: a session script queried Yahoo's
  public symbol search by instrument label, then compared the reviewed broker price with the
  listing's one-year range converted to EUR. Smallest change: `finance-cli prices suggest`,
  which prints candidate symbols per instrument with exchange, currency and that range check,
  and writes nothing.

## Done

- `tools/check-doc-links.sh` fails a Markdown file whose relative link names a missing path or a
  heading the target lacks. It runs in the repo gates for every `*.md` change
  (`tools/lib/ci-inputs.ts`). `tools/doctor` does not run it yet.
- `tools/service-runner.sh stale [<capability>]` reports whether each running process serves the
  binary on disk, and `refresh [<capability>]` builds first and restarts only a process whose
  binary changed. A failed build keeps the old process running. `tools/doctor` warns about a
  stale process. `tools/codesign-binary.sh` skips a binary that the same identity already
  signed, so a no-op build does not look like a new binary.
- Storage's launcher accepts an explicitly selected, executable `SJEL_STORAGE_BIN` for isolated
  verification; arguments and child status are preserved, and invalid overrides refuse without
  compiling. The launcher regression passes. Pi's held-package fixture canonicalizes its root
  before comparing registry paths, preserving its exact unit/harness assertions.

- Places geocoding (`93b05a72`) accepts the existing `place_id` response through the shared contract;
  the desktop response copies and server request copy are removed. Software storage and clients
  share the complete app model. Focused tests, strict Clippy, GPUI core tests and the host
  workspace check pass; device acceptance remains separate.

- Hermetic Cargo owns per-run temporary files, cleans them after successful or failed runs, and
  reuses resolved or writable fallback caches (`8ee79415`). Runner regressions and a live Comms
  database probe passed; its sandbox was absent afterward. Legacy fixtures and interrupted
  sandboxes remain subject to deployment-owned age and process guards.
- Container inventory and age-filtered reclaim use Docker's native CLI, documented in the storage
  owner manual (`abf6e057`). Container and volume pruning remain separate operations; VM allocation
  is not treated as cache-only usage.

- `35d866ed`: upstream skill snapshots retain bounded binary assets, keep the text limit, and
  cap total package bytes. The skills launcher uses a session-writable npm cache; a real Anatomy
  harness review passed and its receipt admitted the exact stored digest.

- Maintenance (`7fa64333`) reports all three tracked Cargo workspaces, five locked Bun projects, API
  contracts, plan lengths and unresolved inline path candidates. Optional `--unused`,
  `--freshness` and `--licenses` distinguish missing coverage from completed reports, apply no
  fixes and leave git pins explicitly unchecked. License inventory covers the current Rust host;
  no legal allowlist or duplicate advisory queue was added. All requested analyzers completed
  in the local trial. See the [maintenance review](../../research/plan-maintenance-review.md).
- Cisco's pinned, source-reviewed local scanner (`f46337c0`) was evaluated in network-disabled containers
  against ten synthetic cases. It did not meet the existing gate's smoke cases; the decision is
  to keep intake and avoid another routine scanner. Source, lock patch, runner and results live
  in [the trial](../../research/benchmarks/skill-intake/README.md).
- Completed inference-role intake reviews (`fa7a7ba5`) persist content- and policy-bound receipts; source,
  baseline and model configuration changes invalidate reuse. Text-part completeness and receipt
  persistence have regression coverage. Existing installation tests pass; live inference and
  harness acceptance remain open above.
- The website launcher disables environment-file loading for every Bun generator and server
  (`6cbd5de2`); scratch generation, internal links and payload checks pass.

- Storage checks aged entries independently, supports name filters, protects descendants, reports
  volume free-space changes and exposes policy-managed directories with `--include-named`
  (`41589496`); filtered measurement and policy validation were corrected in `4052ed70`.
  All 119 storage tests and scoped Clippy passed in an isolated writable target.
  Live reclaim preserved active browser clones; the next scheduled firing and full workspace
  gates were not observed or run.

- GPUI formatting is selected through `tools/ci-run.ts` and covered by `tools/ci-local.test.ts`
  (`217a921b`); revision-isolated verification remains open above.

- iOS launchers check simulator discovery or bounded CoreDevice access before compilation and signing settings. Five planted-command regressions cover unavailable services, missing simulators, explicit selection and build ordering (`tools/ios-preflight.test.sh`). Live device acceptance still requires working host device services.

- Hermetic Cargo verification isolates XDG state; the skill-install rollback regression passes. Portable containment ancestors, backup receipt thresholds, service-runner admission fixtures and Bun environment isolation match their current contracts (`da81a3a1`).

- Capability CLIs run as `sjel <capability> <args...>` through Cargo, with the resolved overlay and local-build policy (`b7e2b832`).
- Trusted MCP text preserves known licence labels and compact ISO date intervals; pseudonymized free-text dates remain protected (`26f1313b`).

Verification for these fixes: 46 pseudonymization tests, two CLI unit tests, the CLI process
integration test and scoped Clippy with warnings denied passed. The rollback regression also passes with isolated XDG state. Live capability rebuilds and
restarts and a live MCP response check did not run. The shared build cache refused temporary archive
cleanup; verification used the isolated Cargo runner with a writable scratch target.

- `sjel-contain --credential <provider>|<entry>=<VAR>` resolves a provider key from the private
  catalogue and hands it to the contained child as its environment variable
  (`crates/containment/src/provider_key.rs`). The `<entry>=<VAR>` form serves a harness whose own
  variable no inference slug spells, for example `copilot-github=COPILOT_GITHUB_TOKEN`.

- `projects archive|unarchive <project>` and `POST /api/projects/archive|unarchive` move a
  project's records and attachments into or out of `Archive/`, after checking every target path.
- Dependabot watches `/apps/gpui` (`.github/dependabot.yml`). It ignores the coupled
  `gpui-kit`, `gpui-pre`, `gpui-fast` and `nsh` set, which moves by hand per
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
