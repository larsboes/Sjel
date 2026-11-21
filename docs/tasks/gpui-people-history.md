---
id: "gpui-people-history"
title: "Show sourced person interaction history"
state: "backlog"
area: "gpui"
kind: "feature"
priority: 3
spec: "docs/architecture/native-ui-boundaries.md"
---

# Show sourced person interaction history

## Outcome

Show sourced person interaction history through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- List dated notes mentioning a person through existing owner links and classification boundaries.
- Do not infer identities from lexical similarity or duplicate private note content.

## Evidence

- Added `SourcedInteraction`, `SourcedGraphInteraction`, `CandidateNoteMention`, and `Person::sourced_interactions` in [`apps/gpui/core/src/people.rs`](../../apps/gpui/core/src/people.rs), admitting only explicit owner links (`note_ref`, dated `last_contact` and `connected_on` with source provenance, dated `EntityFact` entries, explicit graph links, and notes linked by entity ID or `note_ref` below `c3`) without copying private note bodies or matching by lexical similarity.
- Rendered the `"SOURCED INTERACTION HISTORY"` table and Ely `Badge` provenance pills in [`apps/gpui/ui/src/people/detail.rs`](../../apps/gpui/ui/src/people/detail.rs).
- Verified with unit test `sourced_interactions_list_owner_links_and_reject_lexical_similarity_or_c3_notes` in [`apps/gpui/core/src/people.rs`](../../apps/gpui/core/src/people.rs).

## Unrun checks

- Live inspection of a person record with linked project notes in the running desktop app.
