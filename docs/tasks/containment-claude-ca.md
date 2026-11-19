---
id: containment-claude-ca
title: "Resolve native Claude TLS refusal"
state: backlog
area: containment
kind: investigation
priority: 3
spec: docs/plans/containment.md
---

# Resolve native Claude TLS refusal

## Outcome

Native Claude containment can retain the data wall while trusting required CA roots.

## Acceptance

- Find the rule denying /etc/ssl/cert.pem in the native sandbox.
- Prove repository, Cargo, loopback, Sjel and TLS work while overlay-data reads stay denied.
- Only after those checks may the owner enable the native sandbox; do not weaken the file wall as a workaround.
