---
id: "data-media-review"
title: "Accept content-aware Media review"
state: "done"
area: "data"
kind: "maintenance"
priority: 2
spec: "docs/architecture/data-intake.md"
evidence:
  - "Live acceptance confirmed by the maintainer on 2026-11-21."
---

# Accept content-aware Media review

## Outcome

Accept content-aware Media review through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Review actual local media labels as useful/misleading/absent without sending private pixels or inferred labels into agent/cloud prompts.
- Exercise live empty/loading/error/stale/busy states and explicit editable tag/collection decisions; similarity never authorizes deletion and digest originals never move.
- Run mixed image/video decisions, interruption/retry, unsupported codecs, changed sources, proposal backup/restore and unchanged original-byte checks.
- Verify dataless refusal and concurrent-source-change rejection beyond inspection-only evidence; terminal decisions survive replay and changed evidence starts pending.
