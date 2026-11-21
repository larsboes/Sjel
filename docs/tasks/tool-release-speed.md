---
id: "tool-release-speed"
title: "Shorten the path from commit to installed apps"
state: "backlog"
area: "tooling"
kind: "investigation"
priority: 1
---

# Shorten the path from commit to installed apps

## Outcome

Measure each step from a commit on main to a published Mac DMG and a TestFlight build, then remove
the waits and rebuilds that do not buy a check.

## Acceptance

- Record wall time per step for CI, the CLI workflow, iOS smoke, the Mac DMG with notarization,
  and the TestFlight upload, with the run or command that produced each number.
- Run `tools/distribution/ship.ts` end to end on the next tag and record its wall time.

## Evidence

Measured on 2026-11-21:

- CI on push (`57101cee`) and full CI by dispatch (`2e06c4ee`): about 4.5 minutes each.
- CLI-only distribution on the tag: 10.5 minutes for three platforms.
- iOS screen smoke tests: 21 to 29 minutes over the last three runs.
- Mac DMG: the release build reused a 3.6 GB cache. Notarization of the first submission under a
  new Developer ID certificate waited over 36 minutes in `notarytool submit --wait`.
- `tools/release` refused the tag twice: once for a missing release record, once for missing full
  CI at the commit. The second was overridden with `--force`; full CI then passed.
- The pre-push hook rejected a push for `gpui fmt` after six commits were already made.
- Flaky `ext::mcp::tests::stdio_mcp_times_out_when_call_exceeds_timeout` (PR #292): its 1-second
  timeout also bounded the mock server's `initialize`, so a slow Python start failed the
  handshake. The test now gives 3 seconds against a 10-second call; 3 of 3 local runs passed.
- Flaky `soundscape::slideover::tests::clicking_the_dark_bedside_surface_reveals_controls`: GPUI's
  test scheduler panics ("Your test is not deterministic") when a tokio worker wakes one of its
  tasks, and every view that reads through `data::read_repo` hops through `shell::run` onto the
  HTTP runtime. On `HEAD` the full `sjel-gpui-ui` suite failed 1 of 5 runs. With core's
  `test-support` feature, `shell::run` now finishes the work on the calling thread: 0 of 15 runs
  failed, and the GPUI workspace tests pass.
- `tools/demo-shots up` failed on a cold start three times before it ran: Turso owners without a
  database path, a rustup install race between parallel builds, and leftover seeded notes. All
  three are fixed in `02e0ee21`; the cold Linux build of the capabilities still takes several
  minutes at three CPUs.
- `tools/release` notes list upstream and toolchain changes only, not app changes.
- Notarization no longer blocks: `prepare.ts macos --notary-profile` submits and records the
  submission in `release.json`, and `tools/distribution/notarize.ts --dir OUTPUT` resumes to staple
  (exit 75 while pending). Covered by `tools/distribution/notarize.test.ts`. A cached signed DMG
  rebuild from a clean tag worktree took 95 seconds.
- iOS smoke run 44892173162 took 15.5 minutes: 6.3 for cargo despite a full rust-cache hit,
  6 for the Xcode build, about 1.5 for the tests. `run-sim` builds through `tools/cargo-hermetic`,
  which writes under `CARGO_TARGET_DIR` or `$TMPDIR`, while the cache held `apps/gpui/target`.
  The workflow now sets `CARGO_TARGET_DIR` and caches that directory. Not yet measured on CI.
- CLI distribution run 44891247558: `Build CLI-only archive` took 599 seconds on `macos-15` with
  no Cargo cache. The workflow now caches the release target per runner.
- `sjel testflight ship` for build 2363 took 384 seconds, 150 of them a cold release build in a
  temporary target. Ship now reuses `~/Library/Caches/Sjel/ios-archive-target`. Not yet measured.
- `tools/distribution/ship.ts vX.Y.Z` runs every step after a pushed tag and resumes from state in
  `~/Library/Caches/Sjel/releases/<version>/`. The gate and the tag stay in `tools/release`, and
  publication needs `--publish`. Covered by `tools/distribution/ship.test.ts`; not yet run on a
  real tag.
- The iOS smoke's Xcode app build took 30 seconds; the first `xcodebuild test` took 4.4 minutes
  before its result bundle, which includes the simulator boot. Booting the simulator before the Rust build
  helped locally (ready at 150 seconds, when the builds finished) but slowed CI: run
  44897910641 (cold cache) took 39 minutes with 31.7 for cargo, and run 44904259833 (full cache
  hit, 37 workspace crates rebuilt) took 22.5 minutes with 14.1 for cargo. `run-sim` now boots
  early only outside CI. rust-cache drops workspace crates, so those 37 always rebuild.
  With the boot after the builds and a warm cache, run 44907167760 took 13.4 minutes: 1.8 for
  cargo, 220 seconds for both builds, 177 seconds for the simulator boot, then the tests.
- CLI distribution with the release-target cache: 12 minutes cold (run 44897383236), then 9 and
  5 (runs 44897910615, 44898513558).
- `tools/ci-plan.ts --require-full` already accepts a full run from any trigger; the refusal came
  from push CI being affected-only. `tools/release --run-ci` now dispatches full CI on main when
  it is missing and waits (`tools/ci-plan.ts --ensure-full`, covered in `tools/ci-run.test.ts`).
  A live run for `fe44e22a` dispatched run 44917093903 and passed after 4.9 minutes.
- The pre-commit hook now runs `tools/githooks/staged-rustfmt.sh` on the staged Rust files only, so a
  format error stops the commit instead of the push. It reads staged blobs, so unstaged edits from
  other sessions do not block it. All 1094 tracked files outside `research/` pass; about 60 ms per
  file. Covered by `tools/staged-rustfmt.test.sh`.
- `tools/demo-shots up` started from cold without manual repair on 2026-11-21: with the target
  and registry volumes removed, it built, seeded and answered `/health` with 200 after 864 seconds
  at the default 3 CPUs.
- `sjel testflight ship` for build 2384 (`ef53db5f`) took 342 seconds with the persistent archive
  target: 81 seconds for the Rust release build, which still recompiled the Ely bump and the
  changed UI crates, against 150 cold for build 2363.
