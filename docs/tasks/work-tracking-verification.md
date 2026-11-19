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

The focused work-model, roadmap, document-link and CI-selection run passed 135 tests with
zero failures under an explicitly approved command-scoped On-the-go override. Site generation,
internal links and payload checks passed after duplicate-task consolidation.

The work checker validates 262 tasks, fourteen outcomes and no release records or publication
receipts. Relative links and compatibility anchors pass across 705 Markdown documents.
Fresh Rust CLI dispatch passes `sjel work check` and `bash tools/sjel.test.sh`.
The harness suite passes 71 tests; only the Sjel unit was refreshed on maintained materialized
harnesses. These checks do not establish product acceptance or public delivery.

Local browser checks render the roadmap, task metadata, Delivered and Releases pages. Light
and dark backgrounds switch correctly. Empty release history does not claim delivery.
The measured viewport remained 960 pixels despite window resizing, so narrow-screen acceptance
is unproven. Trusted keyboard input was unavailable; synthetic input did not navigate, and
keyboard acceptance remains open. The temporary browser group and preview server were closed.

Remaining acceptance: rendered keyboard and narrow-screen checks; publication reconciliation
against an actual approved release; full repository gates. The initial doctor failures remain
unresolved. No public issue, tag, release or installed app was changed.
