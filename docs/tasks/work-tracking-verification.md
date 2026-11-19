---
id: "work-tracking-verification"
title: "Accept repository work tracking"
state: "verify"
area: "tooling"
kind: "maintenance"
priority: 1
spec: "docs/architecture/work-tracking.md"
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

- The isolated repository snapshot validates 261 tasks and 14 outcomes. Relative links resolve
  across 654 source documents. Website generation, internal links and payload checks pass
  across 454 generated files.
- The installed CLI contract passes, including filtered listing by area and state. The focused
  work model, link and CI suites pass 52 tests on Bun 1.4.0. The CLI packaging and lifecycle
  suite passes another 41 tests on that version.
- Earlier browser checks rendered the roadmap, task metadata and empty Delivered and Releases
  views, and exercised light and dark themes at desktop width.

Keyboard activation and narrow-screen layout on the committed site remain unverified. The local
browser automation engines aborted at startup, and no managed browser session was available.
Publication reconciliation requires an approved release record and published artifacts. The
repository currently has no release records or publication receipts. Full repository gates
remain open.
