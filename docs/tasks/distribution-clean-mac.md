---
id: "distribution-clean-mac"
title: "Accept clean-user Mac installation"
state: "backlog"
area: "tooling"
kind: "feature"
priority: 2
issue: "https://github.com/larsboes/Sjel/issues/235"
---

# Accept clean-user Mac installation

## Outcome

Accept clean-user Mac installation with repeatable evidence and no implicit deployment or publication approval.

## Acceptance

- On a separate clean Mac user with no developer tools, install the notarized DMG and create or connect to a node entirely through the app.
- Use existing versioned runtime placement and launchd persistence; create credentials only on explicit user action and authenticate browser/native entry points.
- Verify signed copied binaries, quarantine, first launch/restart, third-party notices/source and provenance against exact artifacts.
- Rehearse one compatible update and rollback with a tested backup before database migration; measure download/installed/peak setup sizes without using the existing household node.
