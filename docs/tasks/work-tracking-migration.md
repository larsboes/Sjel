---
id: "work-tracking-migration"
title: "Reconcile remaining planning sections"
state: "done"
area: "tooling"
kind: "investigation"
priority: 1
spec: "docs/architecture/work-tracking.md"
evidence:
  - "Reviewed source-to-task reconciliation in this record; 262 tasks and 14 outcomes pass tools/work check."
  - "Fresh Rust CLI work dispatch and 71 harness tests pass; scoped Sjel unit sync is current on maintained harnesses."
  - "Focused model/site tests, site generation and 705-document relative-link checks pass; remaining product acceptance has separate owners."
---

# Reconcile remaining planning sections

## Outcome

Every original open acceptance has one canonical task, specification or explicit rejection.

## Acceptance

- Inventory every section in PLAN.md and docs/plans/ before removing its source list.
- Reconcile existing GitHub reports without synchronizing duplicate task bodies.
- Migrate reviewed batches only when concurrent file owners release their changes.
- Remove shipped breadcrumbs and redundant verification journals; preserve unresolved acceptance.
- Update planning instructions, CLI discovery and deployed harness references after migration.

## Reconciliation

| Original source | Canonical destination |
|---|---|
| Agent tiers, policy and proposal-only runs | [Session boundaries](../architecture/agent-sessions.md) and [Agent tasks](../plans/agent.md) |
| Agent terminal, plugins, live checks and Pi evaluation | [Agent tasks](../plans/agent.md); source implementation is not native acceptance |
| Data mail evidence, intake, Media, clips, settings and domain backlog | [Intake boundaries](../architecture/data-intake.md) and [Data tasks](../plans/data.md) |
| GPUI work packages, prototype, controls, performance, views and phone checks | [UI boundaries](../architecture/native-ui-boundaries.md) and [Native tasks](../plans/gpui.md) |
| GPUI Calendar | Existing Mac and phone Calendar tasks, linked from [Calendar](../plans/gpui-calendar.md) |
| Learning pilot, plugin, conventions and evidence-gated candidates | [Learning tasks](../plans/learning.md), with the Pack and vault contract retaining ownership |
| Open GitHub reports 235, 239–241, 245–247 | Canonical tasks carry intake URLs; no remote issue was closed or rewritten |
| Completed inventories, chronological test journals and superseded pre-plugin learning proposal | Removed from active plans; current interfaces remain with their implemented owners |

Media retains Instagram drafting and manual phone posting. Posts retains LinkedIn drafting;
network-neutral schema does not silently transfer ownership. Concurrent planning edits were
reconciled under explicit authorization; unrelated implementation changes remain untouched.

Dependencies now encode explicit source ordering, including reviewed Posts ingestion/publishing,
shared sync contracts, RoomPlan-before-scanner extraction and evidence-gated learning changes.
Shared contracts keep their own priority; phone-only implementation and acceptance remain P4.
The stale agent-over-daily-work priority statement was removed in favor of PLAN.md.

## Verification

`tools/work check` validates 262 tasks, fourteen outcomes and no release records or publication
receipts. Relative links and anchors pass across 705 Markdown documents. The focused suite
passes 135 tests; site generation, link and payload checks pass after task consolidation.

Fresh Rust CLI dispatch passes `sjel work check` and `bash tools/sjel.test.sh`.
The harness suite passes 71 tests, including unit-scoped refresh and sibling isolation.
Only the Sjel unit was refreshed in Claude Code and Codex; both report current. Pi and
Antigravity read the source registry. Unmaintained harnesses remain unchanged.

Planning instructions now route acceptance, handoff and unattended checkpoints to canonical
tasks. The local integration excludes unrelated concurrent changes and does not establish
publication on origin/main.

Rendered browser and actual-release checks remain owned by
[work-tracking verification](work-tracking-verification.md). No task is marked done merely
because its Markdown validates.
