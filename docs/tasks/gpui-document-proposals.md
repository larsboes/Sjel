---
id: "gpui-document-proposals"
title: "Accept evidence-backed document proposals"
state: "backlog"
area: "gpui"
kind: "feature"
priority: 2
issue: "https://github.com/larsboes/Sjel/issues/247"
---

# Accept evidence-backed document proposals

## Outcome

Accept evidence-backed document proposals with repeatable evidence and no implicit deployment or publication approval.

## Acceptance

- Show summaries with engine/revision/evidence page/generated-at/class/review outcome; rule and model category proposals remain separate rows with visible agreement/conflict.
- Never overwrite owner values or file automatically; show absent summary/confidence honestly and require evidence spans.
- Verify the frozen synthetic German/English corpus; generation needs an approved loopback role, c2 never leaves the host and c3 never enters prompts.

## Evidence

- Implemented `ProposalAgreement`, `ProposalEvidenceView`, `can_leave_host`, `can_enter_model_prompt`, and `evaluate_proposal_run` in [`apps/gpui/core/src/document_queue.rs`](../../apps/gpui/core/src/document_queue.rs).
- Upgraded [`apps/gpui/ui/src/documents/proposals.rs`](../../apps/gpui/ui/src/documents/proposals.rs) to display rule and model proposals as separate rows with Ely `Badge` agreement/conflict indicators (`Tone::Success` when agreeing, `Tone::Warning` when conflicting), `DescriptionList` provenance (`Summary`, `Engine / Rev`, `Evidence Page`, `Generated`, `Class`, `Review`), exact-span badges, honest absent summary/confidence labels (`"No summary generated"`, `"No confidence score"`), and `c3` model-prompt blocking.
- Added unit test `frozen_german_and_english_corpus_verifies_evidence_pages_agreement_and_data_classes` in [`apps/gpui/core/src/document_queue.rs`](../../apps/gpui/core/src/document_queue.rs).

## Unrun checks

- Interactive operator review of a live local-model document proposal run in the running macOS app.
