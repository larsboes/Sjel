---
id: tool-vault-cutover
title: "Cut over vault paths together"
state: backlog
area: tooling
kind: maintenance
priority: 2
spec: docs/plans/tooling.md
---

# Cut over vault paths together

## Outcome

One command verifies the destination before changing pointers.

## Acceptance

- Validate the new contract, backup and isolated restore.
- Update vault pointer, state mount and backup source atomically; verify service reload.
- Preserve the original until acceptance.
