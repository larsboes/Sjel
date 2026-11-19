---
id: storage-data-root
title: "Separate personal data from configuration"
state: backlog
area: storage
kind: feature
priority: 1
spec: docs/plans/storage.md
---

# Separate personal data from configuration

## Outcome

Separate personal data from configuration through the existing owner, without treating implementation as live acceptance.

## Acceptance

- Add a configurable data root outside Git repositories.
- Migrate each owner with verified database/file copies and a backup; preserve records and private configuration.
- Audit tracked overlay data privately; untracking does not remove prior history.
