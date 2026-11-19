---
id: "work-tracking-verification"
title: "Accept repository work tracking"
state: "verify"
area: "tooling"
kind: "maintenance"
priority: 1
spec: "docs/architecture/work-tracking.md"
blocked_by: "No public release is available for live reconciliation"
---

# Accept repository work tracking

## Outcome

Tasks, roadmap views and release preparation share one Markdown authority.

## Acceptance

- Model tests reject invalid state, duplicate IDs, missing references and dependency cycles.
- Completed tasks disappear from active views; draft releases never count as delivered.
- Release acceptance checks the exact source commit and required artifact set.
- Static site link, payload, keyboard, narrow-screen and light/dark checks pass.

## Verification

- The static site builds from a clean repository snapshot. The generated links, publication
  payload and all 19 registered repository gates pass.
- The installed CLI contract passes, including filtered listing by area and state. The focused
  work model, link and CI suites pass 52 tests on Bun 1.4.0. The CLI packaging and lifecycle
  suite passes another 41 tests on that version.
- Chromium verifies Tab and Enter navigation, skip-to-content focus, light and dark themes,
  and no horizontal overflow at 360, 390, 768, 800 and 1280 CSS pixels. The check covers the
  roadmap, task index, task detail, Delivered, and documentation navigation.
- The release fixture writes a receipt for a matching tag and artifact set, reconciles it,
  and refuses changed remote assets without modifying the saved receipt.

Live reconciliation remains open. The repository has no release record or publication receipt,
and GitHub lists no public release.
