---
id: task-ff5205bc-f2b4-4e3b-bd55-f3ca5b905b5f
title: "Model a lost phone and laptop"
state: backlog
area: containment
kind: feature
priority: 2
---

# Model a lost phone and laptop

## Outcome

The threat model covers device loss, which `docs/architecture/security.md` ("Not covered")
states is missing.

## Acceptance

- Run the `threat-model` skill for the phone and for the laptop. Each run writes a scenario and
  register entries under `<overlay>/data/security/threat-model/`.
- `docs/architecture/security.md` "Not covered" names what a lost device exposes, or links the
  accepted residual risk, with no instance facts in the public text.
