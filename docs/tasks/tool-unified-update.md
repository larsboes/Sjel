---
id: tool-unified-update
title: "Unify update discovery"
state: backlog
area: tooling
kind: maintenance
priority: 3
spec: docs/plans/tooling.md
---

# Unify update discovery

## Outcome

`sjel update` reports global skill revisions, Pack drift and existing package classes without
installing anything or changing focus.

## Acceptance

- Report installed and available skill revisions through each owner's read-only metadata.
- Check Interceptor metadata without opening its native updater. Report unavailable revisions
  as unknown; do not invoke an update verb without documented dry-run semantics.
- Combine skill, Pack and package status in the existing report. Keep installation and update
  verbs with their owners.

## Current coverage

[`sjel update`](../../crates/sjel-cli/src/updates/report.rs) reports Interceptor integration age.
That date does not establish installed or available skill revisions.
