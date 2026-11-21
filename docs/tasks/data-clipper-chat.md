---
id: "data-clipper-chat"
title: "Accept multi-turn clipping and citations"
state: "done"
area: "data"
kind: "maintenance"
priority: 2
depends_on: ["data-clipper-first"]
spec: "docs/architecture/data-intake.md"
evidence:
  - "Live acceptance confirmed by the maintainer on 2026-11-21."
---

# Accept multi-turn clipping and citations

## Outcome

Accept multi-turn clipping and citations through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- Check Chrome extraction and GPUI composition/read/export preserve turn order, thoughts, links, footnotes, tables and source turn indices without nested or repeated prose.
- Resolve conversation description being the site tagline, Google grounding redirect links and identical repeated create saves without conflating separate highlights/documents.
- Check large-clip pagination, visible-page citation auditing, parsing once, confirmation before document/version deletion and incremental cache scoping to the source conversation.
