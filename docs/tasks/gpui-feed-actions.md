---
id: "gpui-feed-actions"
title: "Accept Feed jobs and classification changes"
state: "verify"
area: "gpui"
kind: "maintenance"
priority: 2
spec: "docs/architecture/native-ui-boundaries.md"
---

# Accept Feed jobs and classification changes

## Outcome

Accept Feed jobs and classification changes through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Queue/run a cloud job through the existing class-gated capability mode and lower classification only with the required rationale.
- Verify Feed/Inbox capture review and selected-row state in the installed app; no task authorizes new egress or class lowering automatically.
