//! The Mail triage reader drawn by NSH (`docs/plans/gpui.md` §11): UIKit views on the phone,
//! GPUI Fast inside the Mac app.
//!
//! Reads and triage writes run on core's runtime (`shell::spawn`) through [`Source`]. Results
//! cross to NSH's engine thread over an unbounded channel drained by [`MailScreen`]. Archive,
//! trash, and class-raising categories ask for confirmation before sending (`MailAction::confirmation`).

use std::sync::{Mutex, OnceLock};

use nsh_base::{button, text_view as nsh_markdown_view, TextViewLinks};
use nsh_core::prelude::*;
use sjel_gpui_core::device::Source;
use sjel_gpui_core::mail::{
    self, category_label, BulkResult, MailAction, MailOffline, MailProposal, StatusFilter,
    CATEGORIES,
};
use sjel_gpui_core::shell::{self, FeedContentDetail};
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};

enum Update {
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
struct Snapshot {
    rows: Vec<MailProposal>,
    status: String,
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

fn close() {
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

/// Connects the NSH Mail screen to `source` (the Mac's shell or the paired phone's client) and
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

fn open_detail(id: String, proposal: Option<MailProposal>) {
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

fn apply_action(id: String, action: MailAction) {
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

fn apply_gmail_job(id: String, decision: &'static str) {
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

fn format_bulk_notice(action: &MailAction, result: &BulkResult) -> String {
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

fn subject(m: &MailProposal) -> String {
    m.subject
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("(No subject)")
        .to_owned()
}

fn row_subtitle(m: &MailProposal) -> String {
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

fn model_summary(m: &MailProposal) -> Option<(String, Option<String>)> {
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

#[derive(Clone, Copy)]
struct OpenThread {
    proposal: Signal<Option<MailProposal>>,
    detail: Signal<Option<FeedContentDetail>>,
    detail_error: Signal<Option<String>>,
    pending_confirm: Signal<Option<MailAction>>,
}

#[component]
pub fn MailScreen() -> impl IntoElement {
    let initial = feed()
        .snapshot
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let rows = signal(initial.rows);
    let status = signal(initial.status);
    let status_filter = signal(StatusFilter::Pending);
    let category_filter = signal(None::<&'static str>);
    let open_thread = OpenThread {
        proposal: signal(None::<MailProposal>),
        detail: signal(None::<FeedContentDetail>),
        detail_error: signal(None::<String>),
        pending_confirm: signal(None::<MailAction>),
    };

    let (tx, mut rx) = unbounded_channel();
    *feed().tx.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx);
    spawn(async move {
        while let Some(update) = rx.recv().await {
            match update {
                Update::Status(text) => status.set(text),
                Update::Rows(batch) => {
                    if let Some(current) = open_thread
                        .proposal
                        .with(|p| p.as_ref().map(|m| m.id.clone()))
                    {
                        let updated = batch.iter().find(|m| m.id == current).cloned();
                        open_thread.proposal.set(updated);
                    }
                    rows.set(batch);
                }
                Update::Detail { id, detail } => {
                    if open_thread
                        .proposal
                        .with(|p| p.as_ref().is_some_and(|m| m.id == id))
                    {
                        open_thread.detail_error.set(None);
                        open_thread.detail.set(Some(*detail));
                    }
                }
                Update::DetailError { id, error } => {
                    if open_thread
                        .proposal
                        .with(|p| p.as_ref().is_some_and(|m| m.id == id))
                    {
                        open_thread.detail_error.set(Some(error));
                    }
                }
                Update::ActionDone { succeeded, notice } => {
                    status.set(notice);
                    rows.update(|all| all.retain(|m| !succeeded.contains(&m.id)));
                    if open_thread
                        .proposal
                        .with(|p| p.as_ref().is_some_and(|m| succeeded.contains(&m.id)))
                    {
                        open_thread.proposal.set(None);
                        open_thread.detail.set(None);
                        open_thread.pending_confirm.set(None);
                    }
                }
            }
        }
    });

    let closable = feed()
        .close
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .is_some();

    view()
        .flex_1()
        .child(closable.then(|| {
            button()
                .px_4()
                .pt_2()
                .on_press(close)
                .child(text("‹ Back").text_base())
        }))
        .child(
            view()
                .px_4()
                .pt_4()
                .pb_2()
                .gap_1()
                .child(
                    view()
                        .flex_row()
                        .justify_between()
                        .items_center()
                        .child(text("Mail").text_2xl().font_weight(FontWeight::SEMIBOLD))
                        .child(
                            button()
                                .px_3()
                                .py_1()
                                .on_press(refresh)
                                .child(text("Refresh").text_sm()),
                        ),
                )
                .child(text(move || status.get()).text_sm()),
        )
        .child(dynamic(move || {
            match open_thread.proposal.with(|p| p.clone()) {
                Some(proposal) => thread_reader(proposal, open_thread).into_element(),
                None => {
                    thread_list(rows, status_filter, category_filter, open_thread).into_element()
                }
            }
        }))
}

fn thread_list(
    rows: Signal<Vec<MailProposal>>,
    status_filter: Signal<StatusFilter>,
    category_filter: Signal<Option<&'static str>>,
    open_thread: OpenThread,
) -> impl IntoElement {
    let status_chip = move |target: StatusFilter| {
        button()
            .px_3()
            .py_1()
            .on_press(move || status_filter.set(target))
            .child(
                text(move || {
                    let count =
                        rows.with(|all| all.iter().filter(|m| target.matches(&m.status)).count());
                    if status_filter.get() == target {
                        format!("{} ({count}) •", target.label())
                    } else {
                        format!("{} ({count})", target.label())
                    }
                })
                .text_xs(),
            )
    };

    let cat_chip = move |key: Option<&'static str>, label: &'static str| {
        button()
            .px_2()
            .py_1()
            .on_press(move || category_filter.set(key))
            .child(
                text(move || {
                    let active_status = status_filter.get();
                    let count = rows.with(|all| {
                        all.iter()
                            .filter(|m| {
                                active_status.matches(&m.status)
                                    && key.is_none_or(|k| m.stream == k)
                            })
                            .count()
                    });
                    if category_filter.get() == key {
                        format!("{label} {count} •")
                    } else {
                        format!("{label} {count}")
                    }
                })
                .text_xs(),
            )
    };

    let visible_rows = move || {
        let s = status_filter.get();
        let c = category_filter.get();
        rows.with(|all| {
            mail::in_view(all.iter(), s, c)
                .into_iter()
                .cloned()
                .collect::<Vec<_>>()
        })
    };

    let mut status_bar = view().flex_row().px_4().pb_1().gap_2();
    for s in StatusFilter::ALL {
        status_bar = status_bar.child(status_chip(s));
    }

    let mut category_bar = view().flex_row().gap_2().child(cat_chip(None, "All"));
    for (key, label) in CATEGORIES {
        category_bar = category_bar.child(cat_chip(Some(key), label));
    }

    view()
        .flex_1()
        .child(status_bar)
        .child(scroll().horizontal().px_4().pb_2().child(category_bar))
        .child(
            virtual_list(
                move || visible_rows().len(),
                move |at| {
                    let item = move || {
                        visible_rows()
                            .get(at.index())
                            .cloned()
                            .unwrap_or_else(|| MailProposal {
                                id: String::new(),
                                from_addr: None,
                                subject: None,
                                snippet: None,
                                internal_date: None,
                                stream: String::new(),
                                status: String::new(),
                                gmail_sync_status: None,
                                gmail_sync_action: None,
                                waiting: false,
                                relevance: Vec::new(),
                                authenticity: None,
                                model: None,
                            })
                    };
                    button()
                        .px_4()
                        .py_3()
                        .gap_1()
                        .on_press(move || {
                            let chosen = item();
                            if !chosen.id.is_empty() {
                                let id = chosen.id.clone();
                                open_thread.detail.set(None);
                                open_thread.detail_error.set(None);
                                open_thread.pending_confirm.set(None);
                                open_thread.proposal.set(Some(chosen.clone()));
                                open_detail(id, Some(chosen));
                            }
                        })
                        .child(
                            text(move || subject(&item()))
                                .text_base()
                                .font_weight(FontWeight::MEDIUM),
                        )
                        .child(text(move || row_subtitle(&item())).text_xs())
                        .child(
                            text(move || {
                                let current = item();
                                if let Some((model_line, _)) = model_summary(&current) {
                                    model_line
                                } else {
                                    current.snippet.unwrap_or_default()
                                }
                            })
                            .text_xs(),
                        )
                },
            )
            .estimated_item_size(px(76.0))
            .flex_1(),
        )
}

fn action_btn(
    label: impl IntoReactive<SharedString>,
    press: impl Fn() + 'static,
) -> impl IntoElement {
    button()
        .px_3()
        .py_2()
        .on_press(press)
        .child(text(label).text_sm())
}

fn request_or_run(id: String, action: MailAction, open_thread: OpenThread) {
    if action.confirmation(1).is_some() {
        open_thread.pending_confirm.set(Some(action));
    } else {
        open_thread.pending_confirm.set(None);
        apply_action(id, action);
    }
}

fn thread_reader(proposal: MailProposal, open_thread: OpenThread) -> impl IntoElement {
    let id = proposal.id.clone();
    let id_dismiss = id.clone();
    let id_wait = id.clone();
    let id_archive = id.clone();
    let id_trash = id.clone();
    let id_secret = id.clone();
    let waiting = proposal.waiting;
    let model = model_summary(&proposal);
    let attention_id =
        (proposal.gmail_sync_status.as_deref() == Some("attention")).then(|| id.clone());

    let top_actions = view()
        .flex_row()
        .px_2()
        .gap_1()
        .child(action_btn("‹ Mail", move || {
            open_thread.proposal.set(None);
            open_thread.detail.set(None);
            open_thread.detail_error.set(None);
            open_thread.pending_confirm.set(None);
        }))
        .child(action_btn("Dismiss", move || {
            request_or_run(id_dismiss.clone(), MailAction::Dismiss, open_thread);
        }))
        .child(action_btn(
            if waiting { "Clear waiting" } else { "Waiting" },
            move || {
                let act = if waiting {
                    MailAction::ClearWaiting
                } else {
                    MailAction::Waiting
                };
                request_or_run(id_wait.clone(), act, open_thread);
            },
        ))
        .child(action_btn("Archive", move || {
            request_or_run(id_archive.clone(), MailAction::Archive, open_thread);
        }))
        .child(action_btn("Trash", move || {
            request_or_run(id_trash.clone(), MailAction::Trash, open_thread);
        }))
        .child(action_btn("Secret (c3)", move || {
            request_or_run(
                id_secret.clone(),
                MailAction::SetDataClass("c3".into()),
                open_thread,
            );
        }));

    let id_for_confirm = id.clone();
    let confirm_strip = dynamic(move || match open_thread.pending_confirm.get() {
        Some(pending) => {
            let prompt = pending.confirmation(1).unwrap_or_default();
            let id_yes = id_for_confirm.clone();
            view()
                .px_4()
                .py_2()
                .gap_2()
                .child(text(prompt).text_sm().font_weight(FontWeight::MEDIUM))
                .child(
                    view()
                        .flex_row()
                        .gap_2()
                        .child(action_btn("Confirm", move || {
                            open_thread.pending_confirm.set(None);
                            apply_action(id_yes.clone(), pending.clone());
                        }))
                        .child(action_btn("Cancel", move || {
                            open_thread.pending_confirm.set(None);
                        })),
                )
                .into_element()
        }
        None => view().into_element(),
    });

    let id_for_cats = id.clone();
    let mut category_buttons = view().flex_row().gap_2();
    for (key, label) in CATEGORIES {
        let cat_id = id_for_cats.clone();
        let cat_key = (*key).to_owned();
        category_buttons = category_buttons.child(
            button()
                .px_2()
                .py_1()
                .on_press(move || {
                    request_or_run(
                        cat_id.clone(),
                        MailAction::Categorize(cat_key.clone()),
                        open_thread,
                    );
                })
                .child(text(format!("→ {label}")).text_xs()),
        );
    }

    let model_strip = model.map(|(line, proposed)| {
        let id_accept = id.clone();
        view()
            .px_4()
            .py_1()
            .gap_1()
            .child(text(line).text_xs())
            .child(proposed.map(|stream| {
                let label = format!("Accept {}", category_label(&stream));
                action_btn(label, move || {
                    request_or_run(
                        id_accept.clone(),
                        MailAction::Categorize(stream.clone()),
                        open_thread,
                    );
                })
            }))
    });

    let attention_strip = attention_id.map(|att_id| {
        let retry_id = att_id.clone();
        let cancel_id = att_id;
        view()
            .flex_row()
            .px_4()
            .py_1()
            .gap_2()
            .child(text("Automatic Gmail retries stopped.").text_xs())
            .child(action_btn("Retry", move || {
                apply_gmail_job(retry_id.clone(), "retry");
            }))
            .child(action_btn("Cancel sync", move || {
                apply_gmail_job(cancel_id.clone(), "cancel");
            }))
    });

    let fallback_snippet = proposal
        .snippet
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "No preview text stored for this thread.".into());
    let head_title = subject(&proposal);
    let head_sub = row_subtitle(&proposal);

    view()
        .flex_1()
        .child(top_actions)
        .child(confirm_strip)
        .child(scroll().horizontal().px_4().pb_2().child(category_buttons))
        .child(model_strip)
        .child(attention_strip)
        .child(
            scroll().flex_1().child(
                view()
                    .px_4()
                    .pb_4()
                    .gap_2()
                    .child(
                        text(head_title)
                            .text_xl()
                            .font_weight(FontWeight::SEMIBOLD)
                            .select_text(),
                    )
                    .child(text(head_sub).text_sm())
                    .child(dynamic(move || {
                        let body = match open_thread.detail.with(|d| d.clone()) {
                            Some(detail) => detail
                                .content
                                .filter(|t| !t.trim().is_empty())
                                .or_else(|| detail.digest.and_then(|d| d.text))
                                .filter(|t| !t.trim().is_empty())
                                .unwrap_or_else(|| fallback_snippet.clone()),
                            None => match open_thread.detail_error.get() {
                                Some(err) => format!("{fallback_snippet}\n\n({err})"),
                                None => fallback_snippet.clone(),
                            },
                        };
                        nsh_markdown_view()
                            .markdown(body)
                            .selectable(true)
                            .links(TextViewLinks::Open)
                            .into_element()
                    })),
            ),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use sjel_gpui_core::mail::{BulkFailure, ModelVerdict};

    fn sample_proposal() -> MailProposal {
        MailProposal {
            id: "thread_1".into(),
            from_addr: Some("Ada <ada@example.com>".into()),
            subject: Some(" Tax return 2026 ".into()),
            snippet: Some("Please review the attached return.".into()),
            internal_date: Some("2026-11-08T09:30:00Z".into()),
            stream: "aktiv".into(),
            status: "proposed".into(),
            gmail_sync_status: Some("synced".into()),
            gmail_sync_action: None,
            waiting: true,
            relevance: Vec::new(),
            authenticity: None,
            model: Some(ModelVerdict {
                mode: "shadow".into(),
                state: "generated".into(),
                model_stream: Some("steuern".into()),
                confidence_bp: Some(9200),
                rationale: Some("Mentions tax return.".into()),
                held_reason: None,
                classification_version: "v1".into(),
            }),
        }
    }

    #[test]
    fn row_helpers_format_subject_subtitle_and_model_verdict() {
        let m = sample_proposal();
        assert_eq!(subject(&m), "Tax return 2026");
        assert_eq!(
            row_subtitle(&m),
            "Active · Ada <ada@example.com> · 2026-11-08 · Waiting"
        );
        let (line, proposed) = model_summary(&m).expect("model summary present");
        assert_eq!(proposed.as_deref(), Some("steuern"));
        assert!(line.contains(&format!("Model proposes {}", category_label("steuern"))));
        assert!(line.contains("92%"));
        assert!(line.contains("Mentions tax return."));
    }

    #[test]
    fn bulk_notice_formats_success_redaction_and_failure() {
        let ok = BulkResult {
            succeeded: vec!["a".into()],
            failures: Vec::new(),
            narrowed: 0,
        };
        assert_eq!(
            format_bulk_notice(&MailAction::Archive, &ok),
            "Archived in Gmail."
        );
        let redacted = BulkResult {
            succeeded: vec!["a".into()],
            failures: Vec::new(),
            narrowed: 1,
        };
        assert!(
            format_bulk_notice(&MailAction::SetDataClass("c3".into()), &redacted)
                .contains("redacted")
        );
        let failed = BulkResult {
            succeeded: Vec::new(),
            failures: vec![BulkFailure {
                id: "a".into(),
                error: "gmail offline".into(),
            }],
            narrowed: 0,
        };
        assert!(format_bulk_notice(&MailAction::Trash, &failed).contains("gmail offline"));
    }
}
