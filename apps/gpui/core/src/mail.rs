//! Mail triage (comms `/triage`): the proposals a Gmail sweep made, and the rules a client
//! needs to show and act on them.
//!
//! The labels, order and status groups were the dashboard's (`dashboard/src/routes/feed/+page.svelte`);
//! this module owns them now (AGENTS.md, "The dashboard is the second consumer").

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::device::Source;
use crate::shell::FeedContentDetail;
#[cfg(test)]
use crate::shell::FeedRelevance;

pub use feed_api::{
    category_label, in_view, AuthenticityAssessment, AuthenticityFinding, BulkFailure, BulkResult,
    MailAction, MailAuthenticity, MailProposal, ModelVerdict, StatusFilter, CATEGORIES,
    CLASS_RAISING,
};

/// An opened mail thread cached on the phone (`docs/plans/gpui.md` §2 rule 2).
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct CachedMailThread {
    pub proposal: MailProposal,
    pub detail: Option<FeedContentDetail>,
    pub cached_at: String,
}

/// A triage action waiting for reconnection (`docs/plans/gpui.md` §2 rule 3).
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct MailOutboxEntry {
    pub id: String,
    pub thread_ids: Vec<String>,
    pub action: MailAction,
    pub queued_at: String,
}

/// Local storage for opened mail threads and offline triage actions.
#[derive(Clone, Debug)]
pub struct MailOffline {
    dir: PathBuf,
}

impl MailOffline {
    pub fn new(dir: PathBuf) -> Self {
        let _ = fs::create_dir_all(dir.join("threads"));
        let _ = fs::create_dir_all(dir.join("outbox"));
        Self { dir }
    }

    fn threads_dir(&self) -> PathBuf {
        self.dir.join("threads")
    }

    fn outbox_dir(&self) -> PathBuf {
        self.dir.join("outbox")
    }

    /// Stores an opened mail thread's proposal and detail on this device.
    pub fn cache_thread(
        &self,
        proposal: &MailProposal,
        detail: Option<&FeedContentDetail>,
    ) -> Result<(), String> {
        let _ = fs::create_dir_all(self.threads_dir());
        let cached = CachedMailThread {
            proposal: proposal.clone(),
            detail: detail.cloned(),
            cached_at: chrono::Utc::now().to_rfc3339(),
        };
        let json = serde_json::to_vec(&cached).map_err(|e| e.to_string())?;
        fs::write(
            self.threads_dir().join(format!("{}.json", proposal.id)),
            json,
        )
        .map_err(|e| format!("Could not cache mail thread: {e}"))
    }

    /// Returns the cached detail of `id` if opened previously.
    pub fn cached_detail_of(&self, id: &str) -> Option<FeedContentDetail> {
        let path = self.threads_dir().join(format!("{id}.json"));
        let bytes = fs::read(path).ok()?;
        let cached: CachedMailThread = serde_json::from_slice(&bytes).ok()?;
        cached.detail
    }

    /// Returns all cached thread proposals on this device, newest first.
    pub fn cached_threads(&self) -> Vec<MailProposal> {
        let Ok(entries) = fs::read_dir(self.threads_dir()) else {
            return Vec::new();
        };
        let mut threads: Vec<CachedMailThread> = entries
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "json"))
            .filter_map(|e| fs::read(e.path()).ok())
            .filter_map(|b| serde_json::from_slice(&b).ok())
            .collect();
        threads.sort_by(|a, b| b.proposal.internal_date.cmp(&a.proposal.internal_date));
        threads.into_iter().map(|c| c.proposal).collect()
    }

    /// Queues a triage action when the Mac is unreachable.
    pub fn queue_action(
        &self,
        thread_ids: Vec<String>,
        action: MailAction,
    ) -> Result<String, String> {
        let _ = fs::create_dir_all(self.outbox_dir());
        let id = format!(
            "{}_{}",
            chrono::Utc::now().timestamp_millis(),
            thread_ids.first().map(String::as_str).unwrap_or("batch")
        );
        let entry = MailOutboxEntry {
            id: id.clone(),
            thread_ids,
            action,
            queued_at: chrono::Utc::now().to_rfc3339(),
        };
        let json = serde_json::to_vec(&entry).map_err(|e| e.to_string())?;
        fs::write(self.outbox_dir().join(format!("{id}.json")), json)
            .map_err(|e| format!("Could not queue action in outbox: {e}"))?;
        Ok(id)
    }

    /// Returns all queued outbox entries, oldest first.
    pub fn outbox_entries(&self) -> Vec<MailOutboxEntry> {
        let Ok(entries) = fs::read_dir(self.outbox_dir()) else {
            return Vec::new();
        };
        let mut list: Vec<MailOutboxEntry> = entries
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "json"))
            .filter_map(|e| fs::read(e.path()).ok())
            .filter_map(|b| serde_json::from_slice(&b).ok())
            .collect();
        list.sort_by(|a, b| a.queued_at.cmp(&b.queued_at));
        list
    }

    /// Removes an outbox entry after it has been applied to the Mac.
    pub fn remove_outbox_entry(&self, id: &str) {
        let _ = fs::remove_file(self.outbox_dir().join(format!("{id}.json")));
    }

    /// Replays any queued outbox actions through `source`.
    pub async fn replay_outbox(&self, source: &Source) -> Result<usize, String> {
        let entries = self.outbox_entries();
        let mut replayed = 0;
        for entry in entries {
            match source.mail_action(&entry.thread_ids, &entry.action).await {
                Ok(_) => {
                    self.remove_outbox_entry(&entry.id);
                    replayed += 1;
                }
                Err(err) => {
                    return Err(format!("Outbox replay stopped on {}: {err}", entry.id));
                }
            }
        }
        Ok(replayed)
    }
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

    #[test]
    fn mail_offline_caches_threads_and_queues_outbox_actions() {
        let temp_dir = std::env::temp_dir().join(format!(
            "sjel-mail-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let offline = MailOffline::new(temp_dir.clone());
        let m = mail("thread_offline", "steuern", "proposed", None, "2026-11-08");
        let detail = FeedContentDetail {
            id: "thread_offline".into(),
            source: "mail".into(),
            content: Some("Tax statement".into()),
            ..Default::default()
        };

        assert!(offline.cache_thread(&m, Some(&detail)).is_ok());
        let cached_detail = offline.cached_detail_of("thread_offline");
        assert_eq!(
            cached_detail.and_then(|d| d.content).as_deref(),
            Some("Tax statement")
        );

        let cached_list = offline.cached_threads();
        assert_eq!(cached_list.len(), 1);
        assert_eq!(cached_list[0].id, "thread_offline");

        let entry_id = offline
            .queue_action(vec!["thread_offline".into()], MailAction::Archive)
            .expect("queues outbox action");
        let entries = offline.outbox_entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].action, MailAction::Archive);
        assert_eq!(entries[0].thread_ids, vec!["thread_offline"]);

        offline.remove_outbox_entry(&entry_id);
        assert!(offline.outbox_entries().is_empty());
        let _ = std::fs::remove_dir_all(temp_dir);
    }
}
