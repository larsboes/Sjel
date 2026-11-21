---
id: "gpui-mail-adapter"
title: "Prove read-only Ely Mail parity"
state: "backlog"
area: "gpui"
kind: "investigation"
priority: 2
depends_on: ["gpui-mail-mac"]
spec: "docs/architecture/native-ui-boundaries.md"
---

# Prove read-only Ely Mail parity

## Outcome

Prove read-only Ely Mail parity through the existing owner, with implementation and live acceptance recorded separately.

## Acceptance

- First compare a read-only adapter preserving proposal IDs, selected/opened state, metadata, authenticity, held writes, sync failure and caller message order.
- Verify safe preview-only bodies before adopting a reader; routing remains Sjel-owned.
- Composer adoption is not approved by this task and requires a reviewed draft adapter and explicit confirmation through the existing capability write mode.

## Evidence

- `apps/gpui/ui/src/mail/adapter.rs` implements `AdaptedMailProposal`, `adapt_mail_list`, `AdaptedMailReader`, and `sanitize_preview_body`, mapping `sjel_gpui_core::mail::MailProposal` and `FeedContentDetail` onto `ely_gpui_component::mail::{Mail, MailItem, MailList, MailReader, Message, Contact}` without mounting `MailComposer`.
- `apps/gpui/ui/src/mail/row.rs` and `apps/gpui/ui/src/mail/mod.rs` route row badges (`Badge`), filter toggles (`filter_pill`), keyboard hints (`key_bar`), and notices (`InlineMessage`) through `AdaptedMailProposal` while keeping all writes routed through `MailAction::confirmation` and `POST /comms/triage/bulk`.
- Unit and headless tests (`sjel cargo test --manifest-path apps/gpui/Cargo.toml -p sjel-gpui-ui --lib -- mail`) verify caller order from `sjel_gpui_core::mail::in_view`, proposal IDs, selection/cursor/opened state, sender/date/relevance/authenticity metadata, held writes (`mode == "held"` and in-flight `gmail_sync_status == "queued"`), sync failure (`gmail_sync_status == "attention"`), and HTML/script stripping in preview-only `MailReader` bodies.
