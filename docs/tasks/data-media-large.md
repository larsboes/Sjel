---
id: "data-media-large"
title: "Stream large Media originals"
state: "backlog"
area: "data"
kind: "feature"
priority: 2
spec: "docs/architecture/data-intake.md"
---

# Stream large Media originals

## Outcome

Stream large Media originals through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Attach a video over 1 GiB through CLI and owner API with streamed SHA-256 and bounded memory.
- Verify digest, failure cleanup and existing deduplication; do not solve the current 128 MiB/buffering limit by unbounded global request growth.
