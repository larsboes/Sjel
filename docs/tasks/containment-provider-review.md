---
id: containment-provider-review
title: "Review exact provider variants"
state: backlog
area: containment
kind: investigation
priority: 3
spec: docs/plans/containment.md
---

# Review exact provider variants

## Outcome

Each provider variant has an explicit reviewed ring, model set and class ceiling.

## Acceptance

- Human-review API/subscription variants separately, including training toggles for text, voice and video.
- Aggregators list admitted models; unknown models/providers refuse.
- Verify Gmail Limited Use terms for training-disabled subscriptions before admitting derived mail.
- Check Claude bug/feedback transcript behavior before re-enabling either; credentials require separate owner action.
