---
id: containment-phone-apps
title: "Review consumer-app MCP access"
state: backlog
area: containment
kind: investigation
priority: 3
spec: docs/plans/containment.md
---

# Review consumer-app MCP access

## Outcome

Phone consumer apps may read Sjel only after terms and client support are checked.

## Acceptance

- Review subscription permission and iOS remote-MCP support before exposing the endpoint.
- An approved Claude phone test answers from c2 mail through trusted_view with a logged trusted call and withheld c3.
- Scheduled/background subscription calls remain excluded; foreground user-started review is separate.
