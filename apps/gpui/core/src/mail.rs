//! Mail triage (comms `/triage`): the proposals a Gmail sweep made, and the rules a client
//! needs to show and act on them.
//!
//! The labels, order and status groups were the dashboard's (`dashboard/src/routes/feed/+page.svelte`);
//! this module owns them now (AGENTS.md, "The dashboard is the second consumer").

#[cfg(test)]
use crate::shell::FeedRelevance;

pub use feed_api::{
    category_label, in_view, AuthenticityAssessment, AuthenticityFinding, BulkFailure, BulkResult,
    MailAction, MailAuthenticity, MailProposal, ModelVerdict, StatusFilter, CATEGORIES,
    CLASS_RAISING,
};

#[cfg(test)]
mod tests {
    use super::*;

    fn mail(id: &str, stream: &str, status: &str, score: Option<f64>, date: &str) -> MailProposal {
        MailProposal {
            id: id.into(),
            from_addr: None,
            subject: Some(id.into()),
            snippet: None,
            internal_date: Some(date.into()),
            stream: stream.into(),
            status: status.into(),
            gmail_sync_status: None,
            gmail_sync_action: None,
            waiting: false,
            authenticity: None,
            model: None,
            relevance: score
                .map(|score| FeedRelevance {
                    profile_label: "p".into(),
                    score,
                    rationale: String::new(),
                })
                .into_iter()
                .collect(),
        }
    }

    #[test]
    fn a_proposal_is_offered_only_against_the_held_category() {
        let verdict = |state: &str, stream: Option<&str>| ModelVerdict {
            mode: "shadow".into(),
            state: state.into(),
            model_stream: stream.map(str::to_owned),
            confidence_bp: None,
            rationale: None,
            held_reason: None,
            classification_version: "v1".into(),
        };
        assert_eq!(
            verdict("generated", Some("advertising")).proposes_change("active"),
            Some("advertising")
        );
        assert_eq!(
            verdict("generated", Some("advertising")).proposes_change("advertising"),
            None
        );
        assert_eq!(
            verdict("unparseable", Some("advertising")).proposes_change("active"),
            None
        );
        assert_eq!(verdict("generated", None).proposes_change("active"), None);
    }

    #[test]
    fn reading_order_is_category_then_relevance_then_date() {
        let rows = [
            mail("ad", "werbung", "proposed", Some(0.9), "3"),
            mail("old", "issue", "proposed", None, "1"),
            mail("new", "issue", "approved", None, "2"),
            mail("best", "issue", "proposed", Some(0.5), "0"),
            mail("gone", "issue", "archived", Some(1.0), "9"),
            mail("later", "unknown", "proposed", None, "9"),
        ];
        let ids = |status, category| -> Vec<&str> {
            in_view(rows.iter(), status, category)
                .iter()
                .map(|m| m.id.as_str())
                .collect()
        };
        assert_eq!(
            ids(StatusFilter::Pending, None),
            ["best", "new", "old", "ad", "later"]
        );
        assert_eq!(ids(StatusFilter::Archived, None), ["gone"]);
        assert_eq!(ids(StatusFilter::Pending, Some("werbung")), ["ad"]);
    }

    #[test]
    fn only_moves_and_redactions_ask_first() {
        assert!(MailAction::Archive.confirmation(1).is_some());
        assert!(MailAction::Trash
            .confirmation(3)
            .unwrap()
            .contains("3 threads"));
        assert!(MailAction::Categorize("steuern".into())
            .confirmation(1)
            .is_some());
        assert!(MailAction::Categorize("aktiv".into())
            .confirmation(1)
            .is_none());
        assert!(MailAction::Dismiss.confirmation(1).is_none());
        assert!(MailAction::Waiting.confirmation(1).is_none());
    }

    #[test]
    fn a_thread_with_a_gmail_write_in_flight_waits() {
        let mut m = mail("a", "issue", "proposed", None, "0");
        assert!(m.actionable());
        m.gmail_sync_status = Some("queued".into());
        assert!(!m.actionable());
        m.gmail_sync_status = Some("synced".into());
        m.status = "archived".into();
        assert!(!m.actionable());
    }

    #[test]
    fn the_badge_names_the_worst_verdict_and_its_reason() {
        let mut m = mail("a", "issue", "proposed", None, "0");
        assert_eq!(m.authenticity_badge(), None);
        let finding = |code: &str, verdict: &str, detail: &str| AuthenticityFinding {
            code: code.into(),
            verdict: verdict.into(),
            detail: detail.into(),
        };
        m.authenticity = Some(MailAuthenticity {
            verdict: "suspicious".into(),
            assessments: vec![AuthenticityAssessment {
                source: "headers".into(),
                verdict: "suspicious".into(),
                findings: vec![
                    finding("dmarc_pass", "verified", "DMARC passed."),
                    finding(
                        "reply_to_mismatch",
                        "suspicious",
                        "Replies go to gmail.com.",
                    ),
                ],
            }],
        });
        assert_eq!(
            m.authenticity_badge().as_deref(),
            Some("Suspicious: Replies go to gmail.com.")
        );
        m.authenticity.as_mut().unwrap().verdict = "verified".into();
        assert_eq!(m.authenticity_badge(), None);
    }
}
