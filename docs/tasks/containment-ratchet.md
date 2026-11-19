---
id: containment-ratchet
title: "Prevent session class downgrade"
state: backlog
area: containment
kind: investigation
priority: 3
spec: docs/plans/containment.md
---

# Prevent session class downgrade

## Outcome

A transcript cannot continue on a provider below the data it has read.

## Acceptance

- Track the highest data class read by a session.
- After c2 mail, switching or handing off to DeepSeek refuses and logs the refusal.
- Cover trusted Pi sessions without a proxy; no provider-switch bypass.
