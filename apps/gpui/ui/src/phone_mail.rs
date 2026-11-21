//! The phone Mail cache and triage operations through the paired Mac.

use std::sync::{Mutex, OnceLock};

use sjel_gpui_core::{
    device::Source,
    mail::{category_label, BulkResult, MailAction, MailOffline, MailProposal, StatusFilter},
    shell::{self, FeedContentDetail},
};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

pub(super) enum Update {
    Status(String),
    Rows(Vec<MailProposal>),
    Detail {
        id: String,
        detail: Box<FeedContentDetail>,
    },
    DetailError {
        id: String,
        error: String,
    },
    ActionDone {
        succeeded: Vec<String>,
        notice: String,
    },
}

#[derive(Clone, Default)]
pub(super) struct Snapshot {
    pub(super) rows: Vec<MailProposal>,
    pub(super) status: String,
}

struct Feed {
    tx: Mutex<Option<UnboundedSender<Update>>>,
    snapshot: Mutex<Snapshot>,
    source: Mutex<Option<Source>>,
    offline: Mutex<Option<MailOffline>>,
    close: Mutex<Option<Box<dyn Fn() + Send>>>,
}

fn feed() -> &'static Feed {
    static FEED: OnceLock<Feed> = OnceLock::new();
    FEED.get_or_init(|| Feed {
        tx: Mutex::new(None),
        snapshot: Mutex::new(Snapshot {
            rows: Vec::new(),
            status: String::from("Waiting for mail triage…"),
        }),
        source: Mutex::new(None),
        offline: Mutex::new(None),
        close: Mutex::new(None),
    })
}

fn locked<T: Clone>(slot: &Mutex<Option<T>>) -> Option<T> {
    slot.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

fn send(update: Update) {
    {
        let mut snap = feed().snapshot.lock().unwrap_or_else(|e| e.into_inner());
        match &update {
            Update::Status(text) => snap.status = text.clone(),
            Update::Rows(rows) => snap.rows = rows.clone(),
            Update::ActionDone { succeeded, notice } => {
                snap.rows.retain(|row| !succeeded.contains(&row.id));
                snap.status = notice.clone();
            }
            Update::Detail { .. } | Update::DetailError { .. } => {}
        }
    }
    if let Some(tx) = feed().tx.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        let _ = tx.send(update);
    }
}

/// What "Back" does at the top of the screen. Without it, the header omits "Back".
pub fn set_close(close: impl Fn() + Send + 'static) {
    *feed().close.lock().unwrap_or_else(|e| e.into_inner()) = Some(Box::new(close));
}

pub(super) fn close() {
    if let Some(close) = feed()
        .close
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
    {
        close();
    }
}

/// Where opened mail threads and offline outbox actions live on this device.
pub fn set_offline(offline: MailOffline) {
    let cached = offline.cached_threads();
    let outbox = offline.outbox_entries();
    let count = cached.len();
    let outbox_count = outbox.len();
    *feed().offline.lock().unwrap_or_else(|e| e.into_inner()) = Some(offline);
    if count > 0 || outbox_count > 0 {
        let mut status = format!("{count} mail thread(s) kept on this phone");
        if outbox_count > 0 {
            status.push_str(&format!(" · {outbox_count} action(s) waiting in outbox"));
        }
        send(Update::Status(status));
        if !cached.is_empty() {
            send(Update::Rows(cached));
        }
    }
}

/// Connects the Mail screen to `source` (the Mac's shell or the paired phone's client) and
/// fetches the current triage proposals, replaying any queued outbox actions first.
pub fn connect(source: Source) {
    *feed().source.lock().unwrap_or_else(|e| e.into_inner()) = Some(source.clone());
    if let Some(offline) = locked(&feed().offline) {
        if !offline.outbox_entries().is_empty() {
            shell::spawn(async move {
                match offline.replay_outbox(&source).await {
                    Ok(n) if n > 0 => {
                        send(Update::Status(format!(
                            "Replayed {n} queued outbox action(s)."
                        )));
                    }
                    Ok(_) => {}
                    Err(err) => {
                        send(Update::Status(format!("Outbox replay note: {err}")));
                    }
                }
                refresh();
            });
            return;
        }
    }
    refresh();
}

/// Reloads the mail proposals from the connected [`Source`], falling back to kept offline threads.
pub fn refresh() {
    let Some(source) = locked(&feed().source) else {
        if let Some(offline) = locked(&feed().offline) {
            let cached = offline.cached_threads();
            let count = cached.len();
            let outbox = offline.outbox_entries().len();
            let mut status = format!("{count} cached mail thread(s)");
            if outbox > 0 {
                status.push_str(&format!(" · {outbox} waiting in outbox"));
            }
            send(Update::Rows(cached));
            send(Update::Status(status));
        }
        return;
    };
    send(Update::Status("Reading mail triage on the Mac…".into()));
    shell::spawn(async move {
        match source.mail_proposals().await {
            Ok(rows) => {
                let status = summary_status(&rows);
                send(Update::Rows(rows));
                send(Update::Status(status));
            }
            Err(reason) => {
                let mut status = format!("The Mac did not answer: {reason}");
                if let Some(offline) = locked(&feed().offline) {
                    let cached = offline.cached_threads();
                    if !cached.is_empty() {
                        status.push_str(&format!(". Showing {} cached offline.", cached.len()));
                        send(Update::Rows(cached));
                    }
                }
                send(Update::Status(status));
            }
        }
    });
}

pub(super) fn open_detail(id: String, proposal: Option<MailProposal>) {
    if let Some(offline) = locked(&feed().offline) {
        if let Some(cached) = offline.cached_detail_of(&id) {
            send(Update::Detail {
                id: id.clone(),
                detail: Box::new(cached),
            });
            if locked(&feed().source).is_none() {
                return;
            }
        }
    }
    let Some(source) = locked(&feed().source) else {
        return;
    };
    shell::spawn(async move {
        match source.mail_detail(&id).await {
            Ok(detail) => {
                if let Some(offline) = locked(&feed().offline) {
                    if let Some(p) = proposal.as_ref() {
                        let _ = offline.cache_thread(p, Some(&detail));
                    }
                }
                send(Update::Detail {
                    id,
                    detail: Box::new(detail),
                });
            }
            Err(error) => send(Update::DetailError { id, error }),
        }
    });
}

pub(super) fn apply_action(id: String, action: MailAction) {
    let source = locked(&feed().source);
    let offline = locked(&feed().offline);
    let label = action_progress_label(&action);
    send(Update::Status(label));
    let act_clone = action.clone();
    shell::spawn(async move {
        let ids = vec![id.clone()];
        let outcome = if let Some(source) = source {
            source.mail_action(&ids, &act_clone).await
        } else {
            Err("The Mac is not reachable.".into())
        };
        match outcome {
            Ok(result) => {
                let notice = format_bulk_notice(&act_clone, &result);
                send(Update::ActionDone {
                    succeeded: result.succeeded,
                    notice,
                });
                refresh();
            }
            Err(error) => {
                if let Some(offline) = offline {
                    match offline.queue_action(vec![id.clone()], act_clone.clone()) {
                        Ok(_) => {
                            send(Update::ActionDone {
                                succeeded: vec![id],
                                notice: "Action queued in offline outbox. Will apply on reconnect."
                                    .into(),
                            });
                            return;
                        }
                        Err(err) => {
                            send(Update::Status(format!("Could not queue in outbox: {err}")));
                            return;
                        }
                    }
                }
                send(Update::Status(format!("Not applied: {error}")));
            }
        }
    });
}

pub(super) fn apply_gmail_job(id: String, decision: &'static str) {
    let Some(source) = locked(&feed().source) else {
        return;
    };
    send(Update::Status(format!("Sending Gmail {decision}…")));
    shell::spawn(async move {
        match source.mail_gmail_job(&id, decision).await {
            Ok(()) => {
                let msg = if decision == "retry" {
                    "Gmail action retried."
                } else {
                    "Gmail action canceled."
                };
                send(Update::Status(msg.into()));
                refresh();
            }
            Err(error) => send(Update::Status(format!("Not applied: {error}"))),
        }
    });
}

fn summary_status(rows: &[MailProposal]) -> String {
    let pending = rows
        .iter()
        .filter(|m| StatusFilter::Pending.matches(&m.status))
        .count();
    let total = rows.len();
    format!("{pending} pending · {total} threads")
}

fn action_progress_label(action: &MailAction) -> String {
    match action {
        MailAction::Dismiss => "Dismissing thread…".into(),
        MailAction::Archive => "Archiving in Gmail…".into(),
        MailAction::Trash => "Moving to Gmail Trash…".into(),
        MailAction::Waiting => "Marking waiting…".into(),
        MailAction::ClearWaiting => "Clearing waiting…".into(),
        MailAction::Categorize(stream) => format!("Moving to {}…", category_label(stream)),
        MailAction::SetDataClass(class) => format!("Setting data class {class}…"),
    }
}

pub(super) fn format_bulk_notice(action: &MailAction, result: &BulkResult) -> String {
    if !result.failures.is_empty() {
        return format!(
            "{} updated; {} failed: {}",
            result.succeeded.len(),
            result.failures.len(),
            result.failures[0].error
        );
    }
    if result.narrowed > 0 {
        return format!(
            "{} stored subject(s) and preview(s) were redacted.",
            result.narrowed
        );
    }
    match action {
        MailAction::Dismiss => "Thread dismissed.".into(),
        MailAction::Archive => "Archived in Gmail.".into(),
        MailAction::Trash => "Moved to Gmail Trash.".into(),
        MailAction::Waiting => "Marked waiting.".into(),
        MailAction::ClearWaiting => "Cleared waiting.".into(),
        MailAction::Categorize(stream) => format!("Categorized as {}.", category_label(stream)),
        MailAction::SetDataClass(class) => format!("Marked {class}."),
    }
}

pub(super) fn subject(m: &MailProposal) -> String {
    m.subject
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("(No subject)")
        .to_owned()
}

pub(super) fn row_subtitle(m: &MailProposal) -> String {
    let mut parts = vec![
        category_label(&m.stream).to_owned(),
        m.from_addr
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or("Unknown sender")
            .to_owned(),
    ];
    if let Some(day) = m.internal_date.as_deref().and_then(|d| d.get(..10)) {
        parts.push(day.to_owned());
    }
    if let Some(top) = m.relevance.first() {
        parts.push(format!("{} {:.2}", top.profile_label, top.score));
    }
    if let Some(badge) = m.authenticity_badge() {
        parts.push(badge);
    }
    if m.waiting {
        parts.push("Waiting".into());
    }
    if let Some(sync) = m.gmail_sync_status.as_deref().filter(|s| *s != "synced") {
        parts.push(format!("Gmail: {sync}"));
    }
    parts.join(" · ")
}

pub(super) fn model_summary(m: &MailProposal) -> Option<(String, Option<String>)> {
    let verdict = m.model.as_ref()?;
    let proposed = verdict.proposes_change(&m.stream).map(str::to_owned);
    let mut line = match &proposed {
        Some(stream) => format!("Model proposes {}", category_label(stream)),
        None if verdict.state == "generated" => "Model agrees".to_owned(),
        None => verdict.state_text().to_owned(),
    };
    if let Some(bp) = verdict.confidence_bp.filter(|_| proposed.is_some()) {
        line.push_str(&format!(" · {}%", bp / 100));
    }
    if let Some(why) = verdict.rationale.as_deref().filter(|_| proposed.is_some()) {
        line.push_str(&format!(" · {why}"));
    }
    if let Some(held) = verdict
        .held_reason
        .as_deref()
        .filter(|_| verdict.mode == "held")
    {
        line.push_str(&format!(" · Held: {held}"));
    }
    Some((line, proposed))
}

pub(super) fn subscribe() -> (Snapshot, UnboundedReceiver<Update>) {
    let mut sender = feed().tx.lock().unwrap_or_else(|e| e.into_inner());
    let snapshot = feed()
        .snapshot
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let (tx, rx) = unbounded_channel();
    *sender = Some(tx);
    (snapshot, rx)
}

pub(super) fn closable() -> bool {
    feed()
        .close
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .is_some()
}

mod view;
pub use view::PhoneMail;
