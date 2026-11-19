---
id: storage-rotation-acceptance
title: "Accept live key rotation"
state: verify
area: storage
kind: fix
priority: 0
spec: docs/plans/storage.md
---

# Accept live key rotation

## Outcome

Accept live key rotation through the existing owner, without treating implementation as live acceptance.

## Acceptance

- Exercise live Keychain read/delete and rotation against a started primary under owner authorization.
- Test power loss at every durable write and direct-started sqld detection.
- Reprovision recorded clients separately; never treat disposable proof as live-primary acceptance.
