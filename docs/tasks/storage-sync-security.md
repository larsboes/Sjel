---
id: storage-sync-security
title: "Specify encrypted sync admission"
state: backlog
area: storage
kind: investigation
priority: 2
spec: docs/plans/storage.md
---

# Specify encrypted sync admission

## Outcome

Specify encrypted sync admission through the existing owner, without treating implementation as live acceptance.

## Acceptance

- Before a sync service, agree per-database identity, authorization, conflicts, revocation, freshness and class admission.
- Verify whether the selected engine supports device-side encryption or specify encrypted change logs.
- Relay keys remain in Secure Enclave/keychain; plaintext admission applies even when the relay sees only ciphertext.
