//! Mail triage (comms `/triage`): the proposals a Gmail sweep made, and the rules a client
//! needs to show and act on them.
//!
//! The labels, order and status groups were the dashboard's (`dashboard/src/routes/feed/+page.svelte`);
//! this module owns them now (AGENTS.md, "The dashboard is the second consumer").

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

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
    #[serde(default)]
    pub failures: Vec<BulkFailure>,
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
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let now = chrono::Utc::now();
        let id = format!(
            "{}_{}_{}",
            now.timestamp_micros(),
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let entry = MailOutboxEntry {
            id: id.clone(),
            thread_ids,
            action,
            queued_at: now.to_rfc3339_opts(chrono::SecondsFormat::Nanos, false),
            failures: Vec::new(),
        };
        self.store_outbox_entry(&entry)?;
        Ok(id)
    }

    fn store_outbox_entry(&self, entry: &MailOutboxEntry) -> Result<(), String> {
        let dir = self.outbox_dir();
        let temporary = dir.join(format!("{}.tmp", entry.id));
        let target = dir.join(format!("{}.json", entry.id));
        let json = serde_json::to_vec(entry).map_err(|e| e.to_string())?;
        fs::create_dir_all(&dir)
            .and_then(|()| fs::write(&temporary, json))
            .and_then(|()| fs::rename(&temporary, target))
            .map_err(|e| format!("Could not save mail outbox entry: {e}"))
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
        list.sort_by(|a, b| a.queued_at.cmp(&b.queued_at).then_with(|| a.id.cmp(&b.id)));
        list
    }

    fn remove_outbox_entry(&self, id: &str) -> Result<(), String> {
        fs::remove_file(self.outbox_dir().join(format!("{id}.json")))
            .map_err(|e| format!("Could not remove applied mail outbox entry: {e}"))
    }

    fn record_result(
        &self,
        entry: &mut MailOutboxEntry,
        result: &BulkResult,
    ) -> Result<(), String> {
        result.retain_unapplied(&mut entry.thread_ids);
        if entry.thread_ids.is_empty() {
            return self.remove_outbox_entry(&entry.id);
        }
        entry.failures = entry
            .thread_ids
            .iter()
            .map(|id| {
                result
                    .failures
                    .iter()
                    .find(|failure| &failure.id == id)
                    .cloned()
                    .unwrap_or_else(|| BulkFailure {
                        id: id.clone(),
                        error: "The Mac did not acknowledge this action.".into(),
                    })
            })
            .collect();
        self.store_outbox_entry(entry)
    }

    /// Replays queued triage actions in order and retains work the owner did not acknowledge.
    pub async fn replay_outbox(&self, source: &Source) -> Result<usize, String> {
        static REPLAY: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
        let _replay = REPLAY.lock().await;
        let mut replayed = 0;
        let mut blocked_threads = HashSet::new();
        let mut first_failure = None;
        for mut entry in self.outbox_entries() {
            if entry
                .thread_ids
                .iter()
                .any(|id| blocked_threads.contains(id))
            {
                continue;
            }
            let result = source
                .mail_action(&entry.thread_ids, &entry.action)
                .await
                .map_err(|err| format!("Outbox replay stopped on {}. {err}", entry.id))?;
            self.record_result(&mut entry, &result)?;
            if entry.thread_ids.is_empty() {
                replayed += 1;
            } else {
                if first_failure.is_none() {
                    first_failure = entry
                        .failures
                        .into_iter()
                        .next()
                        .map(|failure| failure.error);
                }
                blocked_threads.extend(entry.thread_ids);
            }
        }
        match first_failure {
            Some(reason) => Err(format!(
                "{} thread action(s) still queued. {reason}",
                blocked_threads.len()
            )),
            None => Ok(replayed),
        }
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
    fn older_outbox_entries_load_without_failure_metadata() {
        let entry: MailOutboxEntry = serde_json::from_value(serde_json::json!({
            "id": "old",
            "thread_ids": ["thread"],
            "action": "Archive",
            "queued_at": "2026-11-08T12:00:00+00:00",
        }))
        .unwrap();
        assert_eq!(entry.action, MailAction::Archive);
        assert!(entry.failures.is_empty());
    }

    #[test]
    fn partial_outbox_replay_keeps_unacknowledged_ids_and_reasons() {
        let dir = std::env::temp_dir().join(format!(
            "sjel-mail-replay-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap()
        ));
        let offline = MailOffline::new(dir.clone());
        let id = offline
            .queue_action(
                vec!["applied".into(), "refused".into(), "omitted".into()],
                MailAction::Archive,
            )
            .unwrap();
        let mut entry = offline.outbox_entries().pop().unwrap();
        let result = BulkResult {
            succeeded: vec!["applied".into()],
            failures: vec![BulkFailure {
                id: "refused".into(),
                error: "Review required".into(),
            }],
            narrowed: 0,
        };
        offline.record_result(&mut entry, &result).unwrap();
        let offline = MailOffline::new(dir.clone());
        let mut remaining = offline.outbox_entries().pop().unwrap();
        assert_eq!(remaining.id, id);
        assert_eq!(remaining.thread_ids, ["refused", "omitted"]);
        assert_eq!(remaining.failures[0], result.failures[0]);
        assert_eq!(remaining.failures[1].id, "omitted");
        let result = BulkResult {
            succeeded: remaining.thread_ids.clone(),
            ..Default::default()
        };
        offline.record_result(&mut remaining, &result).unwrap();
        assert!(offline.outbox_entries().is_empty());
        fs::remove_dir_all(dir).unwrap();
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

        offline.remove_outbox_entry(&entry_id).unwrap();
        assert!(offline.outbox_entries().is_empty());
        let _ = std::fs::remove_dir_all(temp_dir);
    }
}
