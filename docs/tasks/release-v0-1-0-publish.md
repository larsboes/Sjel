---
id: "release-v0-1-0-publish"
title: "Publish the v0.1.0 artifacts"
state: "ready"
area: "tooling"
kind: "maintenance"
priority: 1
---

# Publish the v0.1.0 artifacts

## Outcome

The v0.1.0 tag has a published GitHub release with every asset that
[`docs/releases/v0.1.0.md`](../releases/v0.1.0.md) requires, a committed receipt, and a TestFlight
build from the same commit plus only the changes App Store Connect requires.

## Acceptance

- Staple a notarized `Sjel-v0.1.0-aarch64-apple-darwin.dmg` built from the clean tag.
- Add the stapled DMG to the published release with `publish-draft.ts --update-release`.
- Commit `docs/releases/receipts/v0.1.0.json` from `tools/work receipt v0.1.0`.

## Evidence

- Tag `v0.1.0` points at `2e06c4ee` and is on origin. `tools/work release-check v0.1.0` passed, and
  full CI passed at that commit (run 44881977378).
- CLI workflow run 44882362512 built `sjel-cli-v0.1.0-*` for `aarch64-apple-darwin`,
  `x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu`, each with `release.json` recording
  a clean, rebuilt checkout at `2e06c4ee`. The run keeps its artifacts for 14 days.
- The first Mac DMG had no `release.json` because its blocking notary wait was stopped, so it
  cannot be published. A second DMG was rebuilt and signed from a clean `v0.1.0` worktree
  (`release.json` records `2e06c4ee`, `dirty: false`, `rebuilt: true`) and submitted with
  `tools/distribution/notarize.ts`.
- TestFlight build 2363 is processed and on the "Team" group. App Store Connect refused the tag
  itself for a missing `NSHealthUpdateUsageDescription`, so the build comes from
  `release/v0.1.0-ios` at `7c4f1af9`: `v0.1.0` plus only that plist key (`f875e59e` on main).
  `sjel testflight ship` took 384 seconds.

- Published at 2026-11-21 11:14 UTC with the three CLI builds and their source archives, before
  the DMG, because notarization of both submissions stayed `In Progress` for over three hours.
  The release notes say the DMG follows.
- The hold is on the notary account, not the DMG: a signed 16 KB probe binary
  (`7e64caa1-7334-41e4-823f-49984f228791`, 12:05 UTC) also stayed `In Progress` past a 10-minute
  wait. All four submissions of 2026-11-21 under team `52Q4FQ5XN8` were `In Progress` at 12:16 UTC.
  If they are still pending on 2026-11-22, contact Apple Developer Support with
  `f2ae86a2-3ce6-4fc2-959f-8c2f969dce69`.

## Unrun

- The DMG. Outputs are in `~/Library/Caches/Sjel/releases/v0.1.0/` (`macos/`, `cli/`). Run
  `bun tools/distribution/notarize.ts --dir ~/Library/Caches/Sjel/releases/v0.1.0/macos` until it
  exits 0, then `bun tools/distribution/publish-draft.ts --dir …/macos --include …/cli/sjel-cli-macos-15
  --include …/cli/sjel-cli-ubuntu-24.04 --include …/cli/sjel-cli-ubuntu-24.04-arm --update-release`,
  remove the DMG sentence from the release notes, and commit `tools/work receipt v0.1.0`.
