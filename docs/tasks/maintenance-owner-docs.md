---
id: "maintenance-owner-docs"
title: "Shorten owner documentation"
state: "backlog"
area: "tooling"
kind: "investigation"
priority: 1
spec: "docs/plans/restructure.md"
---

# Shorten owner documentation

## Outcome

Current manuals explain interfaces without duplicating task or dependency records.

## Acceptance

- Touched files follow the documentation rules and their links resolve.
- Open work has a canonical task; dependency verdicts stay in upstreams.toml with revisit conditions.

## Verification

Contributor onboarding links the engineering, Pack and release-preparation owners. The
operations entry point routes installation and release work separately. Existing section
anchors resolve. The link checker covers relative HTML links and ignores nested worktrees.
