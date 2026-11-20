---
id: task-bd7107c0-a1a4-4bbf-8206-19acd4ffc6a6
title: "Decide the health data owner"
state: done
area: data
kind: feature
priority: 3
spec: docs/plans/data.md
evidence:
  - "crates/health exists with sleep, activity, heart and body metrics; tools/cargo-hermetic test -p sjel-health --locked passes 7 tests."
  - "docs/plans/data.md (Health) links the owner crate and the two follow-up tasks."
---

# Decide the health data owner

## Outcome

Sjel has one owner for the operator's health data (sleep, activity, heart rate, body
measurements), with a data class and an import path, before any importer is built.

## Decided

- Data class `c1` (Mine). The operator allows Claude and OpenAI models to read it (ruling
  2026-11-19). Third-party medical documents stay in `documents` under its sensitive-name rule.
- Source: Apple Health on the iPhone. The first export showed an Apple Watch (heart rate since
  2019-06-28, sleep stages since 2023-07-01) and the Sleep Cycle app as sleep sources.

## Plan

1. `crates/health` owns daily values per metric in the shared store, with agent read routes.
   Built.
2. `health import` folds the Health app's "Export All Health Data" archive for the history
   backfill. Built; the first real import is
   [its own task](task-c30b5f1e-8a96-4643-b3a4-f6c54dc93c7e.md).
3. The Sjel iOS app reads HealthKit and sends daily values to the paired Mac. A Mac has no
   HealthKit store, so the phone is the only live source. Open:
   [task](task-fbaa58b2-0aff-47e4-a482-0aa248dbedab.md).

Rejected: Health Auto Export (a paid third-party app that only duplicates step 3) and the Oura
and Eight Sleep connectors from the retired LifeOS importer (no device).

## Acceptance

- The operator confirmed the metric list (sleep, activity, heart, body) and the build order
  (capability and export import first) on 2026-11-19.
- `docs/plans/data.md` ("Health") names the owner crate, `crates/health`, and the two follow-up
  tasks. Step 2 uses `health import` directly: the `import` router hands over one file at a
  time over HTTP, and an export folder is skipped there with a pointer to `health import`.
