//! The phone's agent seat, using the shared run model and signed capability stream.
use crate::nsh_ui::{action, caption, field, heading, snapshot};
use futures::{
    future::{AbortHandle, Abortable},
    StreamExt,
};
use nsh_core::prelude::*;
use sjel_gpui_core::{
    agent::{tool_summary, AgentRun, Host, Pending, Row, Session},
    shell,
};
use std::collections::BTreeSet;
use std::sync::{Mutex, OnceLock};
use tokio::sync::watch;

#[derive(Clone, Default)]
struct Snapshot {
    rows: Vec<Row>,
    pending: Option<Pending>,
    sessions: Vec<Session>,
    error: Option<String>,
    status: String,
    running: bool,
    connected: bool,
    busy: bool,
    sent: u64,
    accepted: Option<String>,
}
struct State {
    run: AgentRun,
    host: Option<Host>,
    generation: u64,
    request: Option<AbortHandle>,
    busy: bool,
    sent: u64,
    accepted: Option<String>,
}
struct Feed {
    state: Mutex<State>,
    tx: watch::Sender<Snapshot>,
}
fn feed() -> &'static Feed {
    static FEED: OnceLock<Feed> = OnceLock::new();
    FEED.get_or_init(|| {
        let (tx, _) = watch::channel(Snapshot::default());
        Feed {
            state: Mutex::new(State {
                run: AgentRun::default(),
                host: None,
                generation: 0,
                request: None,
                busy: false,
                sent: 0,
                accepted: None,
            }),
            tx,
        }
    })
}
fn state() -> std::sync::MutexGuard<'static, State> {
    feed().state.lock().unwrap_or_else(|e| e.into_inner())
}
fn publish(s: &State) {
    feed().tx.send_replace(Snapshot {
        rows: s.run.rows.clone(),
        pending: s.run.pending.clone(),
        sessions: s.run.sessions.clone(),
        error: s.run.error.clone(),
        status: s.run.status(),
        running: s.run.running,
        connected: s.host.is_some(),
        busy: s.busy,
        sent: s.sent,
        accepted: s.accepted.clone(),
    });
}
/// Supplies the paired agent host. Replacing it invalidates an older attachment.
pub fn connect(host: Host) {
    {
        let mut s = state();
        if let Some(old) = s.request.take() {
            old.abort();
        }
        s.host = Some(host);
        s.generation += 1;
        s.busy = false;
        publish(&s);
    }
    refresh();
}
/// Reads the session shelf without changing the attached run.
pub fn refresh() {
    let (host, generation) = {
        let s = state();
        (s.host.clone(), s.generation)
    };
    let Some(host) = host else {
        return;
    };
    shell::spawn(async move {
        let answer = host.agent_sessions().await;
        let mut s = state();
        if s.generation != generation {
            return;
        }
        match answer {
            Ok(sessions) => s.run.sessions = sessions,
            Err(e) => s.run.error = Some(e),
        }
        publish(&s);
    });
}
fn new_session(tier: &'static str) {
    let mut s = state();
    if let Some(old) = s.request.take() {
        old.abort();
    }
    s.generation += 1;
    s.busy = false;
    let sessions = std::mem::take(&mut s.run.sessions);
    s.run.clear();
    s.run.sessions = sessions;
    s.run.draft_tier = tier;
    publish(&s);
}
fn failure(generation: u64, error: String) {
    let mut s = state();
    if s.generation == generation {
        s.run.error = Some(error);
        s.run.running = false;
        s.busy = false;
        publish(&s);
    }
}

fn attach(session: Session) {
    let (host, generation, registration) = {
        let mut s = state();
        let Some(host) = s.host.clone() else {
            return;
        };
        if let Some(old) = s.request.take() {
            old.abort();
        }
        s.generation += 1;
        s.busy = true;
        let (abort, registration) = AbortHandle::new_pair();
        s.request = Some(abort);
        publish(&s);
        (host, s.generation, registration)
    };
    shell::spawn(async move {
        let _ = Abortable::new(
            async move {
                let turns = match host.agent_transcript(&session.id).await {
                    Ok(mut turns) => {
                        if session.active && turns.last().is_some_and(|t| t.role == "user") {
                            turns.pop();
                        }
                        turns
                    }
                    Err(e) => {
                        failure(generation, e);
                        return;
                    }
                };
                {
                    let mut s = state();
                    if s.generation != generation {
                        return;
                    }
                    s.run.restore(session.clone(), turns);
                    s.busy = false;
                    publish(&s);
                }
                if session.active {
                    follow(host, session, generation, 0).await;
                }
            },
            registration,
        )
        .await;
    });
}

fn submit(prompt: String) -> bool {
    if prompt.trim().is_empty() {
        return false;
    }
    let (host, generation, existing, running, cursor, tier, registration) = {
        let mut s = state();
        if s.busy {
            return false;
        }
        let Some(host) = s.host.clone() else {
            s.run.error =
                Some("Pair this phone with the Mac to start or follow an agent run.".into());
            publish(&s);
            return false;
        };
        let running = s.run.running;
        let existing = s.run.session.clone();
        if !running {
            if let Some(old) = s.request.take() {
                old.abort();
            }
            s.generation += 1;
        }
        s.busy = true;
        s.run.error = None;
        let (abort, registration) = AbortHandle::new_pair();
        if !running {
            s.request = Some(abort);
        }
        publish(&s);
        (
            host,
            s.generation,
            existing,
            running,
            if running { s.run.last_seq } else { 0 },
            s.run.draft_tier,
            registration,
        )
    };
    shell::spawn(async move {
        let work = async move {
            if running {
                let Some(session) = existing else {
                    failure(
                        generation,
                        "Wait for the session to open before guiding it.".into(),
                    );
                    return;
                };
                match host.agent_steer(&session.id, &prompt).await {
                    Ok(()) => {
                        let mut s = state();
                        if s.generation == generation {
                            s.sent += 1;
                            s.accepted = Some(prompt.clone());
                            s.run.rows.push(Row::Prompt(prompt));
                            s.busy = false;
                            publish(&s);
                        }
                    }
                    Err(e) => {
                        let mut s = state();
                        if s.generation == generation {
                            s.run.error = Some(e);
                            s.busy = false;
                            publish(&s);
                        }
                    }
                }
                return;
            }
            let session = match existing {
                Some(session) => session,
                None => match host.agent_create(tier).await {
                    Ok(session) => session,
                    Err(e) => {
                        failure(generation, e);
                        return;
                    }
                },
            };
            if let Err(e) = host.agent_prompt(&session.id, &prompt).await {
                failure(generation, e);
                return;
            }
            let session = {
                let mut s = state();
                if s.generation != generation {
                    return;
                }
                s.sent += 1;
                s.accepted = Some(prompt.clone());
                let session = s.run.started(session);
                s.busy = false;
                publish(&s);
                session
            };
            follow(host, session, generation, cursor).await;
        };
        if running {
            work.await;
        } else {
            let _ = Abortable::new(work, registration).await;
        }
    });
    true
}

async fn follow(host: Host, session: Session, generation: u64, mut cursor: u64) {
    for attempt in 0..5 {
        let stream = host.agent_stream(&session.id, cursor).await;
        if let Ok(mut stream) = stream {
            while let Some(event) = stream.next().await {
                if event.kind == "truncated" {
                    cursor = event.first_seq.saturating_sub(1);
                    match host.agent_transcript(&session.id).await {
                        Ok(mut turns) => {
                            if session.active && turns.last().is_some_and(|t| t.role == "user") {
                                turns.pop();
                            }
                            let mut s = state();
                            if s.generation != generation {
                                return;
                            }
                            s.run.restore(session.clone(), turns);
                            s.run.last_seq = cursor;
                            s.run.rows.push(Row::Notice(
                                "Transcript reloaded after stream truncation.".into(),
                            ));
                            publish(&s);
                        }
                        Err(e) => {
                            failure(generation, e);
                            return;
                        }
                    }
                    break;
                }
                let terminal = event.terminal();
                cursor = cursor.max(event.seq);
                {
                    let mut s = state();
                    if s.generation != generation || s.run.id() != Some(&session.id) {
                        return;
                    }
                    if event.seq > s.run.last_seq {
                        s.run.apply(event);
                    }
                    if terminal {
                        s.run.running = false;
                    }
                    publish(&s);
                }
                if terminal {
                    refresh();
                    return;
                }
            }
        }
        if attempt < 4 {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    }
    failure(
        generation,
        "The agent stream disconnected. Reopen the session to reconnect.".into(),
    );
}

fn decide(approval: String, allow: bool, session: bool) {
    let (host, id, generation) = {
        let mut s = state();
        let Some(pending) = s.run.pending.as_ref() else {
            return;
        };
        if pending.approval != approval || (allow && pending.is_block()) || s.busy {
            return;
        }
        let (Some(host), Some(id)) = (s.host.clone(), s.run.id().map(str::to_owned)) else {
            return;
        };
        s.busy = true;
        publish(&s);
        (host, id, s.generation)
    };
    shell::spawn(async move {
        let result = host.agent_approve(&id, &approval, allow, session).await;
        let mut s = state();
        if s.generation != generation {
            return;
        }
        s.busy = false;
        match result {
            Ok(()) => {
                if s.run
                    .pending
                    .as_ref()
                    .is_some_and(|p| p.approval == approval)
                {
                    s.run.pending = None;
                }
            }
            Err(e) => s.run.error = Some(e),
        }
        publish(&s);
    });
}
fn stop() {
    let (host, id, generation) = {
        let s = state();
        (s.host.clone(), s.run.id().map(str::to_owned), s.generation)
    };
    let (Some(host), Some(id)) = (host, id) else {
        return;
    };
    shell::spawn(async move {
        let answer = host.agent_stop(&id).await;
        let mut s = state();
        if s.generation != generation {
            return;
        }
        match answer {
            Ok(()) => s.run.running = false,
            Err(e) => s.run.error = Some(e),
        }
        publish(&s);
    });
}

/// Session shelf, streamed transcript, guidance and explicit per-call approvals.
#[component]
pub fn AgentScreen() -> impl IntoElement {
    let snap = snapshot(&feed().tx);
    let prompt = signal(String::new());
    let sent = stored(state().sent);
    effect(move || {
        let current = snap.get();
        if current.sent != sent.get() {
            sent.replace(current.sent);
            if current.accepted.as_deref() == Some(prompt.get().as_str()) {
                prompt.set(String::new());
            }
        }
    });
    let expanded = signal(BTreeSet::<usize>::new());
    view()
        .flex_1()
        .gap_2()
        .px_4()
        .child(heading("Agent"))
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .child(action("Refresh sessions", refresh))
                .child(action("New project session", || new_session("project")))
                .child(action("New Sjel session", || new_session("sjel")))
                .child(action("New account session", || new_session("account"))),
        )
        .child(
            view()
                .test_id("agent-prompt")
                .child(field("Ask or guide the agent", prompt)),
        )
        .child(action("Send", move || {
            submit(prompt.get());
        }))
        .child(dynamic(move || {
            snap.with(|s| s.running).then(|| action("Stop run", stop))
        }))
        .child(
            text(move || {
                snap.with(|s| {
                    s.error.clone().unwrap_or_else(|| {
                        if !s.connected {
                            "Pair this phone with the Mac to start or follow an agent run.".into()
                        } else if s.busy {
                            "Sending…".into()
                        } else {
                            s.status.clone()
                        }
                    })
                })
            })
            .text_sm(),
        )
        .child(scroll().flex_1().child(dynamic(move || {
            let current = snap.get();
            let mut out = view().gap_2();
            if let Some(pending) = current.pending {
                let blocked = pending.is_block();
                out = out
                    .child(heading(pending.name))
                    .child(text(pending.preview).text_sm());
                if let Some(diff) = pending.diff {
                    out =
                        out.child(nsh_base::text_view().markdown(format!("```diff\n{diff}\n```")));
                }
                let deny = pending.approval.clone();
                if !blocked {
                    let once = pending.approval.clone();
                    let session = pending.approval.clone();
                    out = out
                        .child(action("Allow once", move || {
                            decide(once.clone(), true, false)
                        }))
                        .child(action("Allow for this session", move || {
                            decide(session.clone(), true, true)
                        }));
                } else {
                    out = out.child(caption(
                        pending
                            .rule
                            .unwrap_or_else(|| "This call is blocked by policy.".into()),
                    ));
                }
                out = out.child(action("Deny", move || decide(deny.clone(), false, false)));
            }
            for (index, row) in current.rows.into_iter().enumerate() {
                match row {
                    Row::Prompt(prompt) => {
                        out = out.child(heading("You")).child(text(prompt).text_sm())
                    }
                    Row::Answer(answer) => out = out.child(nsh_base::text_view().markdown(answer)),
                    Row::Tool {
                        name,
                        arguments,
                        result,
                        ..
                    } => {
                        out = out.child(action(tool_summary(&name, &arguments), move || {
                            expanded.update(|e| {
                                if !e.remove(&index) {
                                    e.insert(index);
                                }
                            })
                        }));
                        if expanded.with(|e| e.contains(&index)) {
                            out = out
                                .child(text(arguments).text_sm())
                                .child(text(result.unwrap_or_else(|| "Running…".into())).text_sm());
                        }
                    }
                    Row::Decided {
                        name,
                        allowed,
                        for_session,
                    } => {
                        out = out.child(caption(format!(
                            "{name}: {}{}",
                            if allowed { "allowed" } else { "denied" },
                            if for_session { " for this session" } else { "" }
                        )))
                    }
                    Row::Notice(notice) => out = out.child(caption(notice)),
                }
            }
            for session in current.sessions {
                out = out.child(action(session.label(), move || attach(session.clone())));
            }
            out
        })))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn policy_blocks_never_offer_allow_and_stream_updates_keep_the_prompt() {
        use nsh_engine::EngineConfig;
        use nsh_testing::Headless;
        {
            let mut s = state();
            s.run.clear();
            s.run.pending = Some(Pending {
                approval: "a".into(),
                name: "Blocked command".into(),
                preview: "synthetic command".into(),
                diff: None,
                rule: Some("B-test".into()),
                class: Some("block".into()),
            });
            publish(&s);
        }
        let mut app = Headless::start(EngineConfig {
            ui_sets: vec![nsh_base::registry()],
            ..EngineConfig::new(nsh_core::element::Root::of::<AgentScreen>())
        })
        .unwrap();
        app.wait_for("text:Blocked command").unwrap();
        assert!(!app.screen().texts().contains(&"Allow once"));
        assert!(!app.screen().texts().contains(&"Allow for this session"));
        app.type_text("id:agent-prompt", "keep this draft").unwrap();
        {
            let mut s = state();
            s.run.rows.push(Row::Answer("A streaming answer".into()));
            publish(&s);
        }
        app.wait_for("text:A streaming answer").unwrap();
        assert_eq!(
            app.screen().input_text("id:agent-prompt"),
            Ok(Some("keep this draft"))
        );
        decide("a".into(), true, true);
        assert!(state().run.pending.is_some());
    }
}
