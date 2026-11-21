//! Read-only adapter mapping Sjel [`MailProposal`] and [`FeedContentDetail`] records onto
//! `ely_gpui_component::mail` primitives (`Mail`, `MailItem`, `MailList`, `MailReader`,
//! `Message`, `Contact`) while preserving Sjel triage invariants.
//!
//! Composer adoption is intentionally excluded: all writes remain routed through Sjel's
//! capability confirmation flow (`MailAction::confirmation` and `POST /comms/triage/bulk`).

use crate::ely::calendar::jiff::Timestamp;
use gpui_kit::{div, prelude::*, AnyElement, ElementId, SharedString};
use sjel_gpui_core::mail::{category_label, MailProposal};
use sjel_gpui_core::shell::FeedContentDetail;

use crate::ely::{
    data_display::{Badge, Tone},
    mail::{Contact, Mail as ElyMail, MailItem, MailList, MailReader, Message},
};

/// Structured read-only projection of a [`MailProposal`] preserving every triage invariant
/// alongside its [`ElyMail`] representation.
#[derive(Clone, Debug, PartialEq)]
pub struct AdaptedMailProposal {
    pub id: SharedString,
    pub ely_mail: ElyMail,
    pub stream: SharedString,
    pub stream_label: SharedString,
    pub status: SharedString,
    pub selected: bool,
    pub cursor: bool,
    pub opened: bool,
    pub actionable: bool,
    pub waiting: bool,
    pub relevance_badge: Option<SharedString>,
    pub authenticity_badge: Option<(SharedString, Tone)>,
    pub sync_status: Option<(SharedString, Tone)>,
    pub sync_needs_attention: bool,
    pub held_reason: Option<SharedString>,
    pub model_summary: Option<SharedString>,
    pub model_proposed_stream: Option<SharedString>,
}

impl AdaptedMailProposal {
    /// Project a [`MailProposal`] into a read-only [`AdaptedMailProposal`].
    pub fn from_proposal(
        proposal: &MailProposal,
        selected_ids: &[String],
        cursor_id: Option<&str>,
        opened_id: Option<&str>,
    ) -> Self {
        let id = SharedString::from(proposal.id.clone());
        let selected = selected_ids.iter().any(|s| s == &proposal.id);
        let cursor = cursor_id == Some(proposal.id.as_str());
        let opened = opened_id == Some(proposal.id.as_str());
        let actionable = proposal.actionable();
        let waiting = proposal.waiting;

        let from = proposal
            .from_addr
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("Unknown sender");
        let subject = proposal
            .subject
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("(No subject)");
        let at = parse_mail_timestamp(proposal.internal_date.as_deref());

        let relevance_badge = proposal
            .relevance
            .first()
            .map(|top| SharedString::from(format!("{} {:.2}", top.profile_label, top.score)));

        let authenticity_badge = proposal.authenticity_badge().map(|label| {
            let verdict = proposal
                .authenticity
                .as_ref()
                .map(|a| a.verdict.as_str())
                .unwrap_or("suspicious");
            let tone = match verdict {
                "spoofed" | "failed" | "danger" => Tone::Danger,
                _ => Tone::Warning,
            };
            (SharedString::from(label), tone)
        });

        let sync_needs_attention = proposal.gmail_sync_status.as_deref() == Some("attention");
        let sync_status = proposal
            .gmail_sync_status
            .as_deref()
            .filter(|s| *s != "synced")
            .map(|sync| {
                let tone = match sync {
                    "attention" | "failed" => Tone::Danger,
                    "queued" | "running" => Tone::Warning,
                    _ => Tone::Neutral,
                };
                (SharedString::from(format!("Gmail sync: {sync}")), tone)
            });

        let mut held_reason = None;
        let mut model_summary = None;
        let mut model_proposed_stream = None;

        if let Some(verdict) = proposal.model.as_ref() {
            let proposed = verdict.proposes_change(&proposal.stream).map(str::to_owned);
            let mut text = match &proposed {
                Some(stream) => format!("Model proposes {}", category_label(stream)),
                None if verdict.state == "generated" => "Model agrees".to_owned(),
                None => verdict.state_text().to_owned(),
            };
            if let Some(bp) = verdict.confidence_bp.filter(|_| proposed.is_some()) {
                text.push_str(&format!(" · {}% (self-reported)", bp / 100));
            }
            if let Some(why) = verdict.rationale.as_deref().filter(|_| proposed.is_some()) {
                text.push_str(&format!(" · {why}"));
            }
            if let Some(held) = verdict
                .held_reason
                .as_deref()
                .filter(|_| verdict.mode == "held")
            {
                text.push_str(&format!(" · Held for you: {held}"));
                held_reason = Some(SharedString::from(held.to_owned()));
            }
            model_summary = Some(SharedString::from(text));
            model_proposed_stream = proposed.map(SharedString::from);
        }

        let mut ely_mail = ElyMail::new(id.clone(), from.to_owned(), subject.to_owned(), at)
            .unread(proposal.status == "proposed")
            .starred(waiting);
        if let Some(snippet) = proposal
            .snippet
            .as_deref()
            .map(sanitize_preview_body)
            .filter(|s| !s.is_empty())
        {
            ely_mail = ely_mail.snippet(snippet);
        }

        Self {
            id,
            ely_mail,
            stream: SharedString::from(proposal.stream.clone()),
            stream_label: SharedString::from(category_label(&proposal.stream).to_owned()),
            status: SharedString::from(proposal.status.clone()),
            selected,
            cursor,
            opened,
            actionable,
            waiting,
            relevance_badge,
            authenticity_badge,
            sync_status,
            sync_needs_attention,
            held_reason,
            model_summary,
            model_proposed_stream,
        }
    }

    /// Render the semantic status badges for this proposal using Ely [`Badge`].
    pub fn badges(&self) -> Vec<AnyElement> {
        let mut out: Vec<AnyElement> = Vec::new();
        if let Some(rel) = &self.relevance_badge {
            out.push(
                Badge::new(rel.clone())
                    .tone(Tone::Accent)
                    .into_any_element(),
            );
        }
        if let Some((auth, tone)) = &self.authenticity_badge {
            out.push(Badge::new(auth.clone()).tone(*tone).into_any_element());
        }
        if self.waiting {
            out.push(Badge::new("Waiting").tone(Tone::Warning).into_any_element());
        }
        if let Some((sync, tone)) = &self.sync_status {
            out.push(Badge::new(sync.clone()).tone(*tone).into_any_element());
        }
        if let Some(held) = &self.held_reason {
            out.push(
                Badge::new(format!("Held: {held}"))
                    .tone(Tone::Warning)
                    .into_any_element(),
            );
        }
        if self.status.as_ref() != "proposed" {
            out.push(
                Badge::new(self.status.clone())
                    .tone(Tone::Neutral)
                    .into_any_element(),
            );
        }
        out
    }

    /// Build a read-only Ely [`MailItem`] for this proposal.
    pub fn to_mail_item(&self, id: impl Into<ElementId>) -> MailItem {
        MailItem::new(id, self.ely_mail.clone())
    }
}

/// Convert a slice of proposals in caller order into [`AdaptedMailProposal`] values and an
/// Ely [`MailList`] preserving caller order and selection.
pub fn adapt_mail_list<'a>(
    id: impl Into<ElementId>,
    proposals: impl IntoIterator<Item = &'a MailProposal>,
    selected_ids: &[String],
    cursor_id: Option<&str>,
    opened_id: Option<&str>,
) -> (Vec<AdaptedMailProposal>, MailList) {
    let adapted: Vec<AdaptedMailProposal> = proposals
        .into_iter()
        .map(|p| AdaptedMailProposal::from_proposal(p, selected_ids, cursor_id, opened_id))
        .collect();
    let selected: Vec<SharedString> = adapted
        .iter()
        .filter(|a| a.selected)
        .map(|a| a.id.clone())
        .collect();
    let list = MailList::new(id, adapted.iter().map(|a| a.ely_mail.clone())).selected(selected);
    (adapted, list)
}

/// Safe preview-only projection of a mail thread into an Ely [`MailReader`].
#[derive(Clone, Debug, PartialEq)]
pub struct AdaptedMailReader {
    pub id: SharedString,
    pub subject: SharedString,
    pub message: Message,
    pub safe_body: SharedString,
    pub is_fallback_empty: bool,
}

impl AdaptedMailReader {
    /// Construct a read-only preview reader from a [`MailProposal`] and optional
    /// [`FeedContentDetail`], stripping unsafe HTML/script tags and refusing active payloads.
    pub fn from_proposal_and_detail(
        proposal: &MailProposal,
        detail: Option<&FeedContentDetail>,
    ) -> Self {
        let id = SharedString::from(proposal.id.clone());
        let subject = SharedString::from(
            detail
                .and_then(|d| d.title.as_deref())
                .or(proposal.subject.as_deref())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("(No subject)")
                .to_owned(),
        );
        let from_raw = detail
            .and_then(|d| d.author.as_deref())
            .or(proposal.from_addr.as_deref())
            .unwrap_or("Unknown sender");
        let contact = parse_contact(from_raw);
        let at = parse_mail_timestamp(
            proposal
                .internal_date
                .as_deref()
                .or_else(|| detail.map(|d| d.day.as_str())),
        );
        let raw_preview = detail
            .and_then(|d| d.content.as_deref())
            .or_else(|| detail.and_then(|d| d.summary.as_deref()))
            .or(proposal.snippet.as_deref())
            .unwrap_or_default();
        let cleaned = sanitize_preview_body(raw_preview);
        let (safe_body, is_fallback_empty) = if cleaned.is_empty() {
            (
                SharedString::from("Sjel holds no mail text for this item. Open it in Gmail."),
                true,
            )
        } else {
            (SharedString::from(cleaned), false)
        };
        let snippet = proposal
            .snippet
            .as_deref()
            .map(sanitize_preview_body)
            .unwrap_or_default();
        let message = Message::new(id.clone(), contact, at).snippet(snippet);
        Self {
            id,
            subject,
            message,
            safe_body,
            is_fallback_empty,
        }
    }

    /// Render as an Ely [`MailReader`] with no reply/forward/composer handlers attached.
    pub fn into_mail_reader(self, element_id: impl Into<ElementId>) -> MailReader {
        MailReader::new(element_id, self.subject, self.message).child(div().child(self.safe_body))
    }
}

/// Parse an RFC3339, YYYY-MM-DD, or unix-millisecond date string into a [`Timestamp`].
pub fn parse_mail_timestamp(raw: Option<&str>) -> Timestamp {
    let Some(raw) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Timestamp::UNIX_EPOCH;
    };
    if let Ok(ts) = raw.parse::<Timestamp>() {
        return ts;
    }
    if let Some(date_prefix) = raw.get(..10) {
        if let Ok(ts) = format!("{date_prefix}T00:00:00Z").parse::<Timestamp>() {
            return ts;
        }
    }
    if let Ok(ms) = raw.parse::<i64>() {
        if let Ok(ts) = Timestamp::from_millisecond(ms) {
            return ts;
        }
    }
    Timestamp::UNIX_EPOCH
}

/// Split `"Name <addr@example.com>"` or `"addr@example.com"` into an Ely [`Contact`].
pub fn parse_contact(raw: &str) -> Contact {
    let trimmed = raw.trim();
    if let Some((name_part, rest)) = trimmed.split_once('<') {
        if let Some((addr_part, _)) = rest.split_once('>') {
            let name = name_part.trim().trim_matches('"').trim();
            let address = addr_part.trim();
            let display_name = if name.is_empty() { address } else { name };
            return Contact::new(display_name.to_owned(), address.to_owned());
        }
    }
    Contact::new(trimmed.to_owned(), trimmed.to_owned())
}

/// Sanitize a mail snippet or body into safe preview-only plain text.
///
/// Removes `<script>...</script>` and `<style>...</style>` blocks completely, strips all HTML
/// tags, collapses control characters, and caps the preview length so untrusted markup never
/// reaches a native view.
pub fn sanitize_preview_body(raw: &str) -> String {
    let mut working = raw.to_owned();
    for tag in ["script", "style"] {
        loop {
            let lower = working.to_ascii_lowercase();
            let open_pat = format!("<{tag}");
            let close_pat = format!("</{tag}>");
            let Some(start) = lower.find(&open_pat) else {
                break;
            };
            if let Some(rel_end) = lower[start..].find(&close_pat) {
                let end = start + rel_end + close_pat.len();
                working.replace_range(start..end, " ");
            } else {
                working.truncate(start);
                break;
            }
        }
    }
    let mut out = String::with_capacity(working.len());
    let mut in_tag = false;
    for ch in working.chars() {
        match ch {
            '<' => in_tag = true,
            '>' if in_tag => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;
    use sjel_gpui_core::mail::{
        in_view, AuthenticityAssessment, AuthenticityFinding, MailAuthenticity, ModelVerdict,
        StatusFilter,
    };
    use sjel_gpui_core::shell::FeedRelevance;

    fn sample_proposal(id: &str, stream: &str, score: f64, date: &str) -> MailProposal {
        MailProposal {
            id: id.into(),
            from_addr: Some("Ada Lovelace <ada@analytical.engine>".into()),
            subject: Some(format!("Subject {id}")),
            snippet: Some(format!("Preview for {id}")),
            internal_date: Some(date.into()),
            stream: stream.into(),
            status: "proposed".into(),
            gmail_sync_status: Some("synced".into()),
            gmail_sync_action: None,
            waiting: false,
            relevance: vec![FeedRelevance {
                profile_label: "ops".into(),
                score,
                rationale: String::new(),
            }],
            authenticity: None,
            model: None,
        }
    }

    #[test]
    fn adapter_preserves_caller_order_ids_selection_metadata_authenticity_held_and_sync_failure() {
        let mut p1 = sample_proposal("m-1", "issue", 0.92, "2026-11-20T09:15:00Z");
        p1.waiting = true;
        p1.authenticity = Some(MailAuthenticity {
            verdict: "suspicious".into(),
            assessments: vec![AuthenticityAssessment {
                source: "headers".into(),
                verdict: "suspicious".into(),
                findings: vec![AuthenticityFinding {
                    code: "reply_to_mismatch".into(),
                    verdict: "suspicious".into(),
                    detail: "Replies go to external domain.".into(),
                }],
            }],
        });
        p1.model = Some(ModelVerdict {
            mode: "held".into(),
            state: "generated".into(),
            model_stream: Some("aktiv".into()),
            confidence_bp: Some(8700),
            rationale: Some("Actionable billing thread".into()),
            held_reason: Some("awaiting operator confirmation".into()),
            classification_version: "v1".into(),
        });

        let mut p2 = sample_proposal("m-2", "issue", 0.45, "2026-11-19T18:00:00Z");
        p2.gmail_sync_status = Some("attention".into());

        let mut p3 = sample_proposal("m-3", "werbung", 0.80, "2026-11-18T12:00:00Z");
        p3.gmail_sync_status = Some("queued".into());

        let raw = [p3, p2, p1];
        let ordered = in_view(raw.iter(), StatusFilter::Pending, None);
        assert_eq!(
            ordered.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
            ["m-1", "m-2", "m-3"]
        );

        let selected = vec!["m-1".to_owned(), "m-2".to_owned()];
        let (adapted, _list) = adapt_mail_list(
            "test-mail-list",
            ordered,
            &selected,
            Some("m-2"),
            Some("m-1"),
        );

        assert_eq!(
            adapted.iter().map(|a| a.id.as_ref()).collect::<Vec<_>>(),
            ["m-1", "m-2", "m-3"],
            "caller order from in_view is preserved"
        );

        let first = &adapted[0];
        assert!(first.selected);
        assert!(!first.cursor);
        assert!(first.opened);
        assert!(first.waiting);
        assert!(first.ely_mail.starred);
        assert!(first.ely_mail.unread);
        assert_eq!(first.relevance_badge.as_deref(), Some("ops 0.92"));
        assert_eq!(
            first
                .authenticity_badge
                .as_ref()
                .map(|(s, t)| (s.as_ref(), *t)),
            Some(("Suspicious: Replies go to external domain.", Tone::Warning))
        );
        assert_eq!(
            first.held_reason.as_deref(),
            Some("awaiting operator confirmation")
        );
        assert_eq!(first.model_proposed_stream.as_deref(), Some("aktiv"));
        assert!(first
            .model_summary
            .as_deref()
            .unwrap_or_default()
            .contains("Held for you: awaiting operator confirmation"));

        let second = &adapted[1];
        assert!(second.selected);
        assert!(second.cursor);
        assert!(!second.opened);
        assert!(second.sync_needs_attention);
        assert_eq!(
            second.sync_status.as_ref().map(|(s, t)| (s.as_ref(), *t)),
            Some(("Gmail sync: attention", Tone::Danger))
        );

        let third = &adapted[2];
        assert!(
            !third.actionable,
            "queued Gmail write holds further actions"
        );
        assert_eq!(
            third.sync_status.as_ref().map(|(s, t)| (s.as_ref(), *t)),
            Some(("Gmail sync: queued", Tone::Warning))
        );
    }

    #[test]
    fn preview_reader_strips_html_and_scripts_and_falls_back_when_empty() {
        let proposal = sample_proposal("m-html", "aktiv", 0.5, "2026-11-20");
        let detail = FeedContentDetail {
            id: "m-html".into(),
            source: "mail".into(),
            title: Some("Security alert".into()),
            author: Some("\"Security Team\" <sec@example.org>".into()),
            content: Some(
                "<html><style>body{display:none}</style><p>Hello <b>world</b>!</p><script>alert('xss')</script></html>"
                    .into(),
            ),
            ..Default::default()
        };
        let reader = AdaptedMailReader::from_proposal_and_detail(&proposal, Some(&detail));
        assert!(!reader.is_fallback_empty);
        assert_eq!(reader.safe_body.as_ref(), "Hello world !");
        assert_eq!(reader.message.from.name.as_ref(), "Security Team");
        assert_eq!(reader.message.from.address.as_ref(), "sec@example.org");

        let mut empty_proposal = sample_proposal("m-empty", "aktiv", 0.5, "2026-11-20");
        empty_proposal.snippet = None;
        let empty_reader = AdaptedMailReader::from_proposal_and_detail(&empty_proposal, None);
        assert!(empty_reader.is_fallback_empty);
        assert_eq!(
            empty_reader.safe_body.as_ref(),
            "Sjel holds no mail text for this item. Open it in Gmail."
        );
    }

    #[gpui::test]
    fn adapted_mail_list_and_reader_render_headlessly(cx: &mut TestAppContext) {
        crate::test_support::init(cx);
        let p1 = sample_proposal("m-1", "aktiv", 0.9, "2026-11-20T10:00:00Z");
        let p2 = sample_proposal("m-2", "issue", 0.6, "2026-11-19T10:00:00Z");
        let proposals = [&p1, &p2];
        let selected = vec!["m-1".to_owned()];

        let (_view, cx) = cx.add_window_view(move |_, _| {
            struct Host {
                p1: MailProposal,
                p2: MailProposal,
                selected: Vec<String>,
            }
            impl gpui_kit::Render for Host {
                fn render(
                    &mut self,
                    _: &mut gpui_kit::Window,
                    _: &mut gpui_kit::Context<Self>,
                ) -> impl IntoElement {
                    let (_, list) = adapt_mail_list(
                        "headless-mail-list",
                        [&self.p1, &self.p2],
                        &self.selected,
                        Some("m-1"),
                        Some("m-1"),
                    );
                    let reader = AdaptedMailReader::from_proposal_and_detail(&self.p1, None)
                        .into_mail_reader("headless-mail-reader");
                    div().flex().flex_col().child(list).child(reader)
                }
            }
            Host {
                p1: proposals[0].clone(),
                p2: proposals[1].clone(),
                selected,
            }
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("mail-list").is_some());
        assert!(cx.debug_bounds("mail-reader").is_some());
    }
}
