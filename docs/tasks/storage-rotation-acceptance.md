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

## Verification

All 14 CLI rotation fixture tests pass, including replacement and rejection of the previous key,
journal recovery, and the persistence guard. A disposable instance of the pinned sqld binary
accepts the new token and rejects the old token after restart. The live primary is stopped and
the recovery command reports no pending rotation. Live rotation requires the owner at a terminal.
Live Keychain and primary checks, power-loss checks at each durable write, direct-started sqld
detection, and client reprovisioning remain open.
