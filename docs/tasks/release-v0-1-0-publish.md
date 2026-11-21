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
build from the same commit.

## Acceptance

- Staple a notarized `Sjel-v0.1.0-aarch64-apple-darwin.dmg` built from the clean tag.
- Run `tools/distribution/publish-draft.ts` with the Mac output and the three CLI outputs, review
  the draft, then publish it.
- Commit `docs/releases/receipts/v0.1.0.json` from `tools/work receipt v0.1.0`.
- Upload a TestFlight build from `v0.1.0` with `sjel testflight ship`.

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

## Unrun

- Stapling, the draft release, publication, the receipt and the TestFlight upload.
