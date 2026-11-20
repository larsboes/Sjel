---
id: sync-contract
title: "Share domain sync contracts"
state: done
area: storage
kind: feature
priority: 2
spec: docs/plans/storage.md
evidence:
  - "The existing Calendar change-feed regression test passes unchanged with the shared envelope."
---

# Share domain sync contracts

## Outcome

Share domain sync contracts through the existing owner, without treating implementation as live acceptance.

## Acceptance

- Add Changes<T>, Tombstone and the idempotency header name in crates/sync-contract.
- Calendar serializes its existing feed through the shared type and its feed test passes unchanged.
