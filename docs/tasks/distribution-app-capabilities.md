---
id: "distribution-app-capabilities"
title: "Install a capability from the app"
state: "backlog"
area: "tooling"
kind: "feature"
priority: 2
depends_on: ["distribution-clean-mac"]
issue: "https://github.com/larsboes/Sjel/issues/235"
---

# Install a capability from the app

## Outcome

Install a capability from the app with repeatable evidence and no implicit deployment or publication approval.

## Acceptance

- Use a trusted catalogue and verified archive digest; installation, approval, activation and persistence remain distinct.
- On the clean-user node install Calendar through the app, record a synthetic event, restart and disable without losing the event.
- Handle prerequisites and reviewed upgrades without installing unapproved components.
