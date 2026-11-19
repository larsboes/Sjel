---
id: storage-engine
title: "Measure one database engine"
state: backlog
area: storage
kind: investigation
priority: 2
spec: docs/plans/storage.md
---

# Measure one database engine

## Outcome

Measure one database engine through the existing owner, without treating implementation as live acceptance.

## Acceptance

- Review libsql and the Turso candidate before choosing with the owner.
- On copies, test unchanged-file opening, recursive queries, json_each, vector queries, concurrent writers, crash recovery and an iOS build.
- Keep capability writes behind owner contracts; whole-file sync is not an accepted outbox replacement.

## Verification

The existing libSQL suite passes local open, pragmas, transaction rollback, concurrent writers,
and indexed vector search. [Turso's compatibility notes](https://github.com/tursodatabase/turso/blob/main/COMPAT.md)
document JSON and exact vector functions. The [Turso roadmap](https://github.com/tursodatabase/turso/blob/main/README.md#features-and-roadmap)
still lists approximate vector indexing, so a candidate must prove unchanged opening of a copy
with the libSQL index schema before an engine choice. Candidate execution, crash recovery, and an
iOS build remain unverified.
