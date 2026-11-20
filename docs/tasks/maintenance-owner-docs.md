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
operations entry point routes installation and release work separately. Architecture routes
readers to the active crate and service roots and distinguishes owner storage from the shared
file. The relative-link gate covers Markdown and HTML links, including compatibility anchors.
The remaining owner manuals still need review.
