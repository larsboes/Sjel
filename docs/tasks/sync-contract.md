---
id: sync-contract
title: "Share domain sync contracts"
state: backlog
area: storage
kind: feature
priority: 2
spec: docs/plans/storage.md
---

# Share domain sync contracts

## Outcome

Share domain sync contracts through the existing owner, without treating implementation as live acceptance.

## Acceptance

- Add Changes<T>, Tombstone and the idempotency header name in crates/sync-contract.
- Calendar serializes its existing feed through the shared type and its feed test passes unchanged.
