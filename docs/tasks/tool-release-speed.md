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
- Let one release command run the gate, the tag, both artifact builds, the draft and TestFlight,
  and resume after an interruption.
- Decide whether `tools/release` should accept a full CI run on the tagged commit from any trigger,
  and whether the fast gate's `gpui fmt` check runs before commit instead of at push.
- Fix or quarantine the flaky tests seen on 2026-11-21.
- Make `tools/demo-shots up` start from cold without manual repair.

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
- Flaky: `soundscape::slideover::tests::clicking_the_dark_bedside_surface_reveals_controls`
  failed 1 of 3 local runs; `ext::mcp::tests::stdio_mcp_times_out_when_call_exceeds_timeout`
  failed once in CI on PR #292.
- `tools/demo-shots up` failed on a cold start three times before it ran: Turso owners without a
  database path, a rustup install race between parallel builds, and leftover seeded notes. All
  three are fixed in `02e0ee21`; the cold Linux build of the capabilities still takes several
  minutes at three CPUs.
- `tools/release` notes list upstream and toolchain changes only, not app changes.
- Notarization no longer blocks: `prepare.ts macos --notary-profile` submits and records the
  submission in `release.json`, and `tools/distribution/notarize.ts --dir OUTPUT` resumes to staple
  (exit 75 while pending). Covered by `tools/distribution/notarize.test.ts`. A cached signed DMG
  rebuild from a clean tag worktree took 95 seconds.
