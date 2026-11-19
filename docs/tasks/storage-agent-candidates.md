---
id: storage-agent-candidates
title: "Evaluate isolated agent storage"
state: backlog
area: storage
kind: investigation
priority: 3
spec: docs/plans/storage.md
---

# Evaluate isolated agent storage

## Outcome

Evaluate isolated agent storage through the existing owner, without treating implementation as live acceptance.

## Acceptance

- For one concrete job at a time, measure AgentFS maturity/licence/isolation, memory correction, indexing or caching against current sources.
- Keep authoritative records with capabilities and indexes rebuildable; caches never bypass freshness/security checks.
- Secrets remain in the OS keychain; any database secret store requires a separate security decision.
