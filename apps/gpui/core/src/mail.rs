//! Mail triage (comms `/triage`): the proposals a Gmail sweep made, and the rules a client
//! needs to show and act on them.
//!
//! The labels, order and status groups were the dashboard's (`dashboard/src/routes/feed/+page.svelte`);
//! this module owns them now (AGENTS.md, "The dashboard is the second consumer").

use serde::Deserialize;

use crate::shell::FeedRelevance;

/// One row of `GET /comms/triage`, comms' `TriageOut` (`capabilities/comms/src/server/contracts.rs`).
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct MailProposal {
    pub id: String,
    pub from_addr: Option<String>,
    pub subject: Option<String>,
    pub snippet: Option<String>,
    pub internal_date: Option<String>,
    /// The category key, one of [`CATEGORIES`].
    pub stream: String,
    pub status: String,
    pub gmail_sync_status: Option<String>,
    pub gmail_sync_action: Option<String>,
    #[serde(default)]
    pub waiting: bool,
    #[serde(default)]
    pub relevance: Vec<FeedRelevance>,
    /// `None` for a thread no authenticity rung has assessed yet.
    #[serde(default)]
    pub authenticity: Option<MailAuthenticity>,
    /// The local model rung's verdict; `None` for a thread it never looked at.
    #[serde(default)]
    pub model: Option<ModelVerdict>,
}

/// comms' `TriageModelOut`: what the local model rung said about one thread.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct ModelVerdict {
    /// `shadow`, `applied` or `held`.
    pub mode: String,
    /// `generated` when the model answered; otherwise why it did not.
    pub state: String,
    pub model_stream: Option<String>,
    pub confidence_bp: Option<i64>,
    pub rationale: Option<String>,
    /// Set when apply refused the verdict because it would raise the data class.
    pub held_reason: Option<String>,
    pub classification_version: String,
}

impl ModelVerdict {
    /// The category worth an Accept button: a generated verdict that differs from the
    /// category the row holds now. Compared with the held category, not the frozen
    /// `rule_stream`, so an accepted proposal stops offering itself.
    pub fn proposes_change(&self, current: &str) -> Option<&str> {
        (self.state == "generated")
            .then_some(self.model_stream.as_deref())
            .flatten()
            .filter(|stream| *stream != current)
    }

    /// The line shown when the model gave no category: a state, not an error.
    pub fn state_text(&self) -> &str {
        match self.state.as_str() {
            "local_refused" => {
                "Refused: a Secret mail never enters a prompt, local models included."
            }
            "skipped_over_window" => "Skipped: too long for the local model's window.",
            "unconfigured" => "No local model is configured for this rung on this machine.",
            "invalid_stream" => {
                "The model named a category that does not exist. Nothing was stored."
            }
            "unparseable" => {
                "The model's answer could not be read as JSON. It will be asked again."
            }
            "remote_refused" => "Refused: the resolved model was not on this machine.",
            other => other,
        }
    }
}

/// comms' `AuthenticityOut`: the worst verdict over every rung, and each rung's findings.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct MailAuthenticity {
    /// `verified`, `unverified`, `suspicious` or `failed`.
    pub verdict: String,
    #[serde(default)]
    pub assessments: Vec<AuthenticityAssessment>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct AuthenticityAssessment {
    pub source: String,
    pub verdict: String,
    #[serde(default)]
    pub findings: Vec<AuthenticityFinding>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct AuthenticityFinding {
    pub code: String,
    pub verdict: String,
    pub detail: String,
}

impl MailProposal {
    /// The row badge: the verdict and the reason behind it. `None` for a verified or
    /// unassessed mail, so a clean inbox shows no badge at all.
    pub fn authenticity_badge(&self) -> Option<String> {
        let auth = self.authenticity.as_ref()?;
        let label = match auth.verdict.as_str() {
            "failed" => "Failed check",
            "suspicious" => "Suspicious",
            "unverified" => "Unverified",
            _ => return None,
        };
        let reason = auth
            .assessments
            .iter()
            .flat_map(|a| &a.findings)
            .find(|f| f.verdict == auth.verdict)
            .map(|f| f.detail.as_str());
        Some(match reason {
            Some(reason) => format!("{label}: {reason}"),
            None => label.to_owned(),
        })
    }
}

/// Category keys as comms stores them, in the order a reader works through them, with labels.
pub const CATEGORIES: [(&str, &str); 7] = [
    ("issue", "Action"),
    ("aktiv", "Active"),
    ("steuern", "Tax"),
    ("belege", "Receipts"),
    ("feed", "Feed"),
    ("sonstiges", "Other"),
    ("werbung", "Advertising"),
];

/// Setting one of these raises the data class, and comms then redacts the stored subject and
/// preview for good (`content_item::mail_others_reason` in comms). A client confirms first.
pub const CLASS_RAISING: [&str; 2] = ["belege", "steuern"];

pub fn category_label(key: &str) -> &str {
    CATEGORIES
        .iter()
        .find(|(k, _)| *k == key)
        .map_or(key, |(_, label)| label)
}

/// The status groups a reader filters by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusFilter {
    /// Proposed or approved: still waiting on a decision.
    Pending,
    Archived,
    Trashed,
    Dismissed,
    All,
}

impl StatusFilter {
    pub const ALL: [StatusFilter; 5] = [
        Self::Pending,
        Self::Archived,
        Self::Trashed,
        Self::Dismissed,
        Self::All,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Pending => "Pending",
            Self::Archived => "Archived",
            Self::Trashed => "Trash",
            Self::Dismissed => "Dismissed",
            Self::All => "All",
        }
    }

    pub fn matches(self, status: &str) -> bool {
        match self {
            Self::Pending => matches!(status, "proposed" | "approved"),
            Self::Archived => status == "archived",
            Self::Trashed => status == "trashed",
            Self::Dismissed => status == "dismissed",
            Self::All => true,
        }
    }
}

/// A bulk action (`POST /comms/triage/bulk`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MailAction {
    Dismiss,
    Categorize(String),
    Archive,
    Trash,
    Waiting,
    ClearWaiting,
    /// A hand-set data class. Comms refuses a lowering without a rationale, so clients
    /// offer only the raise to Secret.
    SetDataClass(String),
}

impl MailAction {
    pub fn key(&self) -> &'static str {
        match self {
            Self::Dismiss => "dismiss",
            Self::Categorize(_) => "categorize",
            Self::Archive => "archive",
            Self::Trash => "trash",
            Self::Waiting => "waiting",
            Self::ClearWaiting => "clear-waiting",
            Self::SetDataClass(_) => "set-data-class",
        }
    }

    /// What to ask before the write, or `None` when it needs no confirmation. Archive and trash
    /// move threads in Gmail; a class-raising category destroys stored text. The Waiting label is
    /// one Gmail label that the next press removes, so it does not ask.
    pub fn confirmation(&self, count: usize) -> Option<String> {
        let threads = if count == 1 {
            "1 thread".to_owned()
        } else {
            format!("{count} threads")
        };
        match self {
            Self::Archive => Some(format!("Archive {threads} in Sjel and Gmail?")),
            Self::Trash => Some(format!("Move {threads} to Gmail Trash?")),
            Self::SetDataClass(class) => Some(format!(
                "Set data class {} on {threads}? A raise redacts the stored subject and preview for good.",
                class.to_uppercase()
            )),
            Self::Categorize(key) if CLASS_RAISING.contains(&key.as_str()) => Some(format!(
                "Set {} on {threads}? The stored subject and preview are redacted for good.",
                category_label(key)
            )),
            _ => None,
        }
    }
}

/// What `POST /comms/triage/bulk` answers.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct BulkResult {
    pub succeeded: Vec<String>,
    #[serde(default)]
    pub failures: Vec<BulkFailure>,
    /// Rows whose stored subject and preview this batch redacted.
    #[serde(default)]
    pub narrowed: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct BulkFailure {
    pub id: String,
    pub error: String,
}

impl MailProposal {
    /// A thread with a Gmail write in flight; a second action waits for it.
    pub fn actionable(&self) -> bool {
        StatusFilter::Pending.matches(&self.status)
            && !matches!(
                self.gmail_sync_status.as_deref(),
                Some("queued" | "retrying")
            )
    }

    pub fn gmail_url(&self) -> String {
        format!("https://mail.google.com/mail/u/0/#all/{}", self.id)
    }

    fn top_score(&self) -> f64 {
        self.relevance
            .first()
            .map_or(f64::NEG_INFINITY, |r| r.score)
    }
}

/// The proposals in view, in reading order: category by [`CATEGORIES`], then best relevance,
/// then newest. Categories comms adds later sort last rather than vanish.
pub fn in_view<'a>(
    rows: impl Iterator<Item = &'a MailProposal>,
    status: StatusFilter,
    category: Option<&str>,
) -> Vec<&'a MailProposal> {
    let rank = |key: &str| {
        CATEGORIES
            .iter()
            .position(|(k, _)| *k == key)
            .unwrap_or(CATEGORIES.len())
    };
    let mut rows: Vec<&MailProposal> = rows
        .filter(|m| status.matches(&m.status))
        .filter(|m| category.is_none_or(|c| m.stream == c))
        .collect();
    rows.sort_by(|a, b| {
        rank(&a.stream)
            .cmp(&rank(&b.stream))
            .then(b.top_score().total_cmp(&a.top_score()))
            .then(b.internal_date.cmp(&a.internal_date))
    });
    rows
}

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
