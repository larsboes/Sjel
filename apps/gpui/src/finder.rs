//! The finder: one input for a search and a prompt, over whatever page is open (⌘K).
//!
//! It replaces the command palette and keeps everything the palette offered -- pages, actions,
//! dashboard destinations -- as search results, next to the projects, tasks and notes the Mac's
//! database already holds. The same input also asks: in *Auto* the words decide
//! (`sjel_gpui_core::search::intent`), the decision is printed beside the input, and ⌘↩ goes the
//! other way. A question is answered in place through the app's one `Ask` session, and the whole
//! conversation shows above the results while the text is a question. There is no Ask page.
//!
//! Decided 2026-10-23 (`PORTING_PLAN.md`, section 3): an overlay first; the docked right panel
//! comes with the adaptive UI. Server search (`/api/search`) is the next step and appends below
//! the local results.
//!
//! The server half (`GET /api/sjel-status/search`) runs [`SERVER_DEBOUNCE`] after the last
//! keystroke and appends below the local results, which never wait for it.

use std::time::{Duration, Instant};

use futures::StreamExt;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::{
    actions, div, prelude::*, px, rgb, rgba, Action, App, Context, Entity, EventEmitter,
    FocusHandle, Focusable, IntoElement, MouseButton, Render, SharedString, Subscription, Window,
};
use sjel_gpui_core::search::{self, Mode};

use crate::agent_run::{AgentRun, Pending, Row};
use crate::ask::{Ask, Model};
use crate::data::Repo;
use crate::route::Page;
use crate::shell::{self, GlobalNote, GlobalTask, ProjectSummary, ServerHit};
use crate::theme::{tint, Palette};

actions!(finder, [SelectUp, SelectDown, NextMode, Close]);

/// How many results the list shows. More than a screenful is a list to scroll, not an answer.
const SHOWN: usize = 12;
/// Records older than this are read again when the finder opens.
const RECORDS_FRESH: Duration = Duration::from_secs(60);
const WIDTH: f32 = 640.;
/// How long typing has to pause before the server is asked: one request per word, not per key.
const SERVER_DEBOUNCE: Duration = Duration::from_millis(180);
/// Server results shown at most, below the local ones.
const SERVER_SHOWN: usize = 15;

/// Where a result goes. The workspace carries it out, so the finder never touches a page.
#[derive(Clone)]
pub enum Open {
    Page(Page),
    /// An app action, built fresh each time: an action is not `Clone`.
    Action(fn() -> Box<dyn Action>),
    Dashboard(&'static str),
    Project(String),
    Record {
        project: String,
        path: String,
    },
    /// A server result's `sjel://` link, opened the way a deep link is.
    Link(String),
}

/// One result row.
#[derive(Clone)]
pub struct Hit {
    pub kind: &'static str,
    pub title: String,
    pub detail: String,
    pub open: Open,
}

pub enum FinderEvent {
    Open(Open),
    AgentStarted,
    Closed,
}

pub struct Finder {
    repo: Repo,
    ask: Entity<Ask>,
    /// Pages, actions and dashboard destinations: what the palette offered.
    commands: Vec<Hit>,
    /// Projects, open tasks and notes, read when the finder opens.
    records: Vec<Hit>,
    records_at: Option<Instant>,
    /// The server's answer, and the text it answered: shown only while that is still the text.
    server: Vec<Hit>,
    server_query: String,
    /// The text a server request is running for, if one is.
    searching: Option<String>,
    input: Option<(Entity<InputState>, Subscription)>,
    query: String,
    mode: Mode,
    cursor: usize,
    /// Set when the arrow keys moved the cursor: a result picked by hand is what Enter opens,
    /// whatever the words look like.
    picked: bool,
    /// True once a question was sent from here: the conversation shows until it is cleared.
    asked: bool,
    /// The agent panel: its session, its rows, and the call waiting on an answer.
    agent: AgentRun,
    agent_scroll: gpui::ScrollHandle,
    thread_scroll: gpui::ScrollHandle,
    open: bool,
    focus_next: bool,
    focus: FocusHandle,
    _ask: Subscription,
}

impl EventEmitter<FinderEvent> for Finder {}

impl Finder {
    pub fn new(repo: Repo, ask: Entity<Ask>, commands: Vec<Hit>, cx: &mut Context<Self>) -> Self {
        // The answer streams into the Ask session; the finder redraws with it and follows it down.
        let observe = cx.observe(&ask, |view: &mut Self, ask, cx| {
            if ask.read(cx).loading {
                view.thread_scroll.scroll_to_bottom();
            }
            cx.notify();
        });
        Self {
            repo,
            ask,
            commands,
            records: Vec::new(),
            records_at: None,
            server: Vec::new(),
            server_query: String::new(),
            searching: None,
            input: None,
            query: String::new(),
            mode: Mode::Auto,
            cursor: 0,
            picked: false,
            asked: false,
            agent: AgentRun::default(),
            agent_scroll: gpui::ScrollHandle::new(),
            thread_scroll: gpui::ScrollHandle::new(),
            open: false,
            focus_next: false,
            focus: cx.focus_handle(),
            _ask: observe,
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn open(&mut self, cx: &mut Context<Self>) {
        self.open = true;
        self.focus_next = true;
        self.cursor = 0;
        self.picked = false;
        // A finder opens empty: the last text was for the last errand. The last answer stays,
        // so reopening shows what came back while it was closed.
        self.query.clear();
        self.ask.update(cx, |ask, cx| ask.discover_providers(cx));
        if self
            .records_at
            .is_none_or(|at| at.elapsed() > RECORDS_FRESH)
        {
            self.load_records(cx);
        }
        cx.notify();
    }

    pub fn close(&mut self, cx: &mut Context<Self>) {
        if !self.open {
            return;
        }
        self.open = false;
        cx.emit(FinderEvent::Closed);
        cx.notify();
    }

    /// Open the finder and ask `question` in the session. An empty question only opens it.
    pub fn ask(&mut self, question: String, cx: &mut Context<Self>) {
        if !self.open {
            self.open(cx);
        }
        if question.trim().is_empty() {
            return;
        }
        self.ask.update(cx, |ask, cx| ask.submit(question, cx));
        self.asked = true;
        self.thread_scroll.scroll_to_bottom();
        cx.notify();
    }

    pub fn toggle(&mut self, cx: &mut Context<Self>) {
        if self.open {
            self.close(cx);
        } else {
            self.open(cx);
        }
    }

    /// Projects, open tasks and notes. Each read lands on its own, so a slow one does not hold
    /// back the others, and a failed one leaves the finder with what it has.
    fn load_records(&mut self, cx: &mut Context<Self>) {
        self.records_at = Some(Instant::now());
        let repo = self.repo.clone();
        let reads = shell::run(async move {
            let projects = repo.fetch_projects().await.unwrap_or_default();
            let tasks = repo.fetch_tasks(None).await.unwrap_or_default();
            let notes = repo.fetch_all_notes().await.unwrap_or_default();
            (projects, tasks, notes)
        });
        cx.spawn(async move |this, cx| {
            let Some((projects, tasks, notes)) = reads.await else {
                return;
            };
            let _ = this.update(cx, |view, cx| {
                view.records = record_hits(projects, tasks, notes);
                cx.notify();
            });
        })
        .detach();
    }

    /// What the list shows for the current text: the pages when nothing is typed, the best
    /// matches otherwise.
    pub fn hits(&self) -> Vec<&Hit> {
        if self.query.trim().is_empty() {
            return self
                .commands
                .iter()
                .filter(|hit| matches!(hit.open, Open::Page(_)))
                .take(SHOWN)
                .collect();
        }
        let all: Vec<&Hit> = self.commands.iter().chain(self.records.iter()).collect();
        let local = search::rank(&self.query, &all, |hit| {
            (hit.title.as_str(), vec![hit.detail.as_str(), hit.kind])
        })
        .into_iter()
        .take(SHOWN)
        .map(|index| all[index]);
        // The server's rows come after, in its own source order, and only for the text they
        // answer: a late answer to an older text is dropped rather than shown under a new one.
        let server = self
            .server
            .iter()
            .filter(|_| self.server_query == self.query.trim())
            .take(SERVER_SHOWN);
        local.chain(server).collect()
    }

    /// Ask the server for the current text once typing pauses. Ask mode asks; it does not search.
    fn search_server_soon(&mut self, cx: &mut Context<Self>) {
        let text = self.query.trim().to_owned();
        if text.is_empty() || self.mode == Mode::Ask || text == self.server_query {
            return;
        }
        let Ok(shell) = self.repo.shell() else {
            return;
        };
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SERVER_DEBOUNCE).await;
            // Still the text on screen? A later keystroke has its own request coming.
            let current = this
                .update(cx, |view, cx| {
                    let current = view.query.trim() == text;
                    if current {
                        view.searching = Some(text.clone());
                        cx.notify();
                    }
                    current
                })
                .unwrap_or(false);
            if !current {
                return;
            }
            let asked = text.clone();
            let answer = shell::run(async move { shell.search(&asked).await }).await;
            let _ = this.update(cx, |view, cx| {
                if view.searching.as_deref() == Some(text.as_str()) {
                    view.searching = None;
                }
                // A failed search leaves the local results alone.
                if let Some(Ok(hits)) = answer {
                    view.server = hits.into_iter().map(server_hit).collect();
                    view.server_query = text;
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Which way Enter goes for the current text.
    pub fn resolved(&self) -> Mode {
        self.mode.resolve(&self.query)
    }

    fn ensure_input(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<InputState> {
        let input = match &self.input {
            Some((input, _)) => input.clone(),
            None => {
                let input = cx.new(|cx| {
                    InputState::new(window, cx)
                        .placeholder("Search Sjel, ask it something, or start a task…")
                });
                let subscription = cx.subscribe_in(
                    &input,
                    window,
                    |view, input, event, window, cx| match event {
                        InputEvent::Change => {
                            view.query = input.read(cx).value().to_string();
                            view.cursor = 0;
                            view.picked = false;
                            view.search_server_soon(cx);
                            cx.notify();
                        }
                        InputEvent::PressEnter { secondary, .. } => {
                            view.confirm(*secondary, window, cx);
                        }
                        _ => {}
                    },
                );
                self.input = Some((input.clone(), subscription));
                input
            }
        };
        if std::mem::take(&mut self.focus_next) {
            let empty = self.query.is_empty();
            input.update(cx, |input, cx| {
                if empty && !input.value().is_empty() {
                    input.set_value("", window, cx);
                }
                input.focus(window, cx);
            });
        }
        input
    }

    /// Enter: open the selected result, or ask. `other_way` is ⌘↩, which flips the decision.
    ///
    /// In *Auto* a search that finds nothing asks instead: an empty list is never the answer to a
    /// text someone pressed Enter on.
    fn confirm(&mut self, other_way: bool, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.query.trim().to_owned();
        // Under an answer an empty Enter is not a request to open the first page.
        if text.is_empty() && self.asked {
            return;
        }
        // Agent is never what Enter does in Auto: it starts a run that edits files.
        if self.resolved() == Mode::Agent && !self.picked {
            if text.is_empty() {
                return;
            }
            if self.agent.running {
                self.agent_steer(text, window, cx);
            } else {
                self.agent_send(text, window, cx);
            }
            return;
        }
        let mut ask = self.resolved() == Mode::Ask && !self.picked;
        if other_way {
            ask = !ask;
        }
        let hit = self.hits().get(self.cursor).map(|hit| hit.open.clone());
        if !ask && hit.is_none() && self.mode == Mode::Auto && !text.is_empty() {
            ask = true;
        }
        if ask {
            if text.is_empty() {
                return;
            }
            self.ask.update(cx, |ask, cx| ask.submit(text, cx));
            self.asked = true;
            self.thread_scroll.scroll_to_bottom();
            self.query.clear();
            if let Some((input, _)) = &self.input {
                input.update(cx, |input, cx| input.set_value("", window, cx));
            }
            cx.notify();
            return;
        }
        if let Some(open) = hit {
            self.close(cx);
            cx.emit(FinderEvent::Open(open));
        }
    }

    /// Starts a run, or continues the session this panel is attached to.
    ///
    /// The whole chain — create the session if there is none, send the prompt, attach the stream
    /// — runs in one future on the HTTP runtime, so a slow capability cannot leave the panel with
    /// a session and no stream.
    fn agent_send(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        let repo = self.repo.clone();
        let existing = self.agent.id().map(str::to_owned);
        let draft_tier = self.agent.draft_tier;
        self.query.clear();
        if let Some((input, _)) = &self.input {
            input.update(cx, |input, cx| input.set_value("", window, cx));
        }
        self.agent.error = None;
        self.agent.running = true;
        cx.emit(FinderEvent::AgentStarted);
        self.agent_scroll.scroll_to_bottom();
        cx.notify();

        let work = shell::run(async move {
            let shell = repo.shell()?;
            let session = match existing {
                Some(id) => shell
                    .agent_sessions()
                    .await
                    .unwrap_or_default()
                    .into_iter()
                    .find(|session| session.id == id)
                    .unwrap_or(sjel_gpui_core::agent::Session {
                        id,
                        ..Default::default()
                    }),
                None => shell.agent_create(None, Some(draft_tier)).await?,
            };
            shell.agent_prompt(&session.id, &text).await?;
            let stream = shell.agent_stream(&session.id, 0).await?;
            Ok::<_, shell::ShellError>((session, stream))
        });

        cx.spawn(async move |this, cx| {
            let Some(Ok((session, stream))) = work.await else {
                let _ = this.update(cx, |view, cx| {
                    view.agent.running = false;
                    view.agent.error = Some(
                        "Sjel is not reachable, or the agent capability is not running.".into(),
                    );
                    cx.notify();
                });
                return;
            };
            let follow_session = session.clone();
            let _ = this.update(cx, |view, cx| {
                let first = view.agent.session.is_none();
                if first {
                    view.agent.session = Some(session.clone());
                }
                view.agent.sessions.retain(|listed| listed.id != session.id);
                view.agent.sessions.insert(0, session);
                view.agent.running = true;
                view.agent_follow(follow_session, stream, 0, cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn agent_refresh_sessions(&mut self, cx: &mut Context<Self>) {
        if self.agent.sessions_loading {
            return;
        }
        self.agent.sessions_loading = true;
        let repo = self.repo.clone();
        let work = shell::run(async move { repo.shell()?.agent_sessions().await });
        cx.spawn(async move |this, cx| {
            let Some(result) = work.await else { return };
            let _ = this.update(cx, |view, cx| {
                view.agent.sessions_loading = false;
                if let Ok(sessions) = result {
                    view.agent.sessions = sessions;
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn agent_follow(
        &mut self,
        session: sjel_gpui_core::agent::Session,
        mut stream: futures::channel::mpsc::UnboundedReceiver<sjel_gpui_core::agent::Event>,
        mut cursor: u64,
        cx: &mut Context<Self>,
    ) {
        let repo = self.repo.clone();
        cx.spawn(async move |this, cx| {
            let mut reconnects = 0;
            loop {
                let mut truncated = false;
                while let Some(event) = stream.next().await {
                    if event.kind == "truncated" {
                        cursor = event.first_seq.saturating_sub(1);
                        truncated = true;
                        break;
                    }
                    cursor = cursor.max(event.seq);
                    let terminal = event.terminal();
                    let applied = this
                        .update(cx, |view, cx| {
                            if view.agent.id() != Some(session.id.as_str()) {
                                return false;
                            }
                            view.agent.apply(event);
                            view.agent_scroll.scroll_to_bottom();
                            if terminal {
                                view.agent.running = false;
                            }
                            cx.notify();
                            !terminal
                        })
                        .unwrap_or(false);
                    if !applied {
                        return;
                    }
                }

                if truncated {
                    let repo = repo.clone();
                    let session_id = session.id.clone();
                    let work = shell::run(async move {
                        let shell = repo.shell()?;
                        let turns = shell.agent_transcript(&session_id).await?;
                        let stream = shell.agent_stream(&session_id, cursor).await?;
                        Ok::<_, shell::ShellError>((turns, stream))
                    });
                    let Some(Ok((turns, resumed))) = work.await else {
                        let _ = this.update(cx, |view, cx| {
                            view.agent.running = false;
                            view.agent.error = Some(
                                "Could not reload the agent transcript after stream truncation."
                                    .into(),
                            );
                            cx.notify();
                        });
                        return;
                    };
                    let mut turns = turns;
                    if cursor == 0
                        && session.active
                        && turns
                            .last()
                            .is_some_and(|turn| turn.role.as_str() == "user")
                    {
                        turns.pop();
                    }
                    let restored = this
                        .update(cx, |view, cx| {
                            if view.agent.id() != Some(session.id.as_str()) {
                                return false;
                            }
                            view.agent.restore(session.clone(), turns);
                            view.agent.last_seq = cursor;
                            view.agent
                                .rows
                                .push(Row::Notice("Transcript reloaded; stream resumed.".into()));
                            view.agent_scroll.scroll_to_bottom();
                            cx.notify();
                            true
                        })
                        .unwrap_or(false);
                    if !restored {
                        return;
                    }
                    stream = resumed;
                    reconnects = 0;
                    continue;
                }

                reconnects += 1;
                if reconnects >= 5 {
                    let _ = this.update(cx, |view, cx| {
                        if view.agent.id() == Some(session.id.as_str()) {
                            view.agent.running = false;
                            view.agent.error = Some(
                                "Agent stream disconnected. Reopen the session to reconnect."
                                    .into(),
                            );
                            cx.notify();
                        }
                    });
                    return;
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
                let repo = repo.clone();
                let session_id = session.id.clone();
                let work =
                    shell::run(
                        async move { repo.shell()?.agent_stream(&session_id, cursor).await },
                    );
                match work.await {
                    Some(Ok(resumed)) => stream = resumed,
                    _ => continue,
                }
            }
        })
        .detach();
    }

    fn agent_open_session(
        &mut self,
        session: sjel_gpui_core::agent::Session,
        cx: &mut Context<Self>,
    ) {
        let repo = self.repo.clone();
        let session_for_read = session.clone();
        let already_attached = self.agent.id() == Some(session.id.as_str());
        let cursor = if already_attached {
            self.agent.last_seq
        } else {
            0
        };
        let work = shell::run(async move {
            let shell = repo.shell()?;
            let mut turns = if already_attached {
                Vec::new()
            } else {
                shell.agent_transcript(&session.id).await?
            };
            if session.active
                && turns
                    .last()
                    .is_some_and(|turn| turn.role.as_str() == "user")
            {
                turns.pop();
            }
            let stream = if session.active {
                Some(shell.agent_stream(&session.id, cursor).await?)
            } else {
                None
            };
            Ok::<_, shell::ShellError>((turns, stream))
        });
        cx.spawn(async move |this, cx| {
            let Some(Ok((turns, stream))) = work.await else {
                return;
            };
            let _ = this.update(cx, |view, cx| {
                if !already_attached {
                    view.agent.restore(session_for_read.clone(), turns);
                }
                cx.notify();
            });
            if let Some(stream) = stream {
                let _ = this.update(cx, |view, cx| {
                    view.agent_follow(session_for_read.clone(), stream, cursor, cx);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// A message while a run is active steers it instead of starting a second one.
    fn agent_steer(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.agent.id().map(str::to_owned) else {
            return;
        };
        let repo = self.repo.clone();
        self.query.clear();
        if let Some((input, _)) = &self.input {
            input.update(cx, |input, cx| input.set_value("", window, cx));
        }
        self.agent.rows.push(Row::Prompt(text.clone()));
        cx.notify();
        cx.spawn(async move |this, cx| {
            let answer = shell::run(async move {
                let shell = repo.shell()?;
                shell.agent_steer(&id, &text).await
            })
            .await;
            if !matches!(answer, Some(Ok(()))) {
                let _ = this.update(cx, |view, cx| {
                    view.agent.error = Some("The message was not queued.".into());
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// The owner's answer to the call that is waiting.
    fn agent_decide(&mut self, allow: bool, for_session: bool, cx: &mut Context<Self>) {
        let (Some(pending), Some(id)) = (
            self.agent.pending.clone(),
            self.agent.id().map(str::to_owned),
        ) else {
            return;
        };
        let repo = self.repo.clone();
        cx.spawn(async move |this, cx| {
            let answer = shell::run(async move {
                let shell = repo.shell()?;
                shell
                    .agent_approve(&id, &pending.approval, allow, for_session)
                    .await
            })
            .await;
            let _ = this.update(cx, |view, cx| {
                match answer {
                    Some(Ok(())) => {
                        // The `approved` or `denied` event clears the card; this only makes the
                        // panel respond now rather than on the next frame of the stream.
                        view.agent.pending = None;
                        if allow {
                            view.agent.rows.push(Row::Decided {
                                name: pending.name,
                                allowed: true,
                                for_session,
                            });
                        }
                    }
                    _ => {
                        view.agent.error = Some("The decision did not reach the agent.".into());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn agent_stop(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.agent.id().map(str::to_owned) else {
            return;
        };
        let repo = self.repo.clone();
        cx.spawn(async move |this, cx| {
            let _ = shell::run(async move {
                let shell = repo.shell()?;
                shell.agent_stop(&id).await
            })
            .await;
            let _ = this.update(cx, |view, cx| {
                view.agent.running = false;
                cx.notify();
            });
        })
        .detach();
    }

    /// One transcript row. No listeners, so it can be built while the panel borrows the view.
    fn agent_row(row: Row, palette: Palette) -> gpui::AnyElement {
        match row {
            Row::Prompt(text) => div()
                .px_4()
                .pt_2()
                .text_xs()
                .text_color(rgb(palette.muted))
                .child(text)
                .into_any_element(),
            Row::Answer(text) => div().px_4().py_1().text_sm().child(text).into_any_element(),
            Row::Tool {
                name,
                arguments,
                result,
                ..
            } => {
                let detail = sjel_gpui_core::agent::tool_summary(&name, &arguments);
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .px_4()
                    .py_1()
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .flex_none()
                                    .text_xs()
                                    .text_color(rgb(palette.accent))
                                    .child("▸"),
                            )
                            .child(div().flex_none().text_xs().child(name))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_xs()
                                    .text_color(rgb(palette.muted))
                                    .child(detail),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .text_xs()
                                    .text_color(rgb(match &result {
                                        None => palette.muted,
                                        Some(_) => palette.green,
                                    }))
                                    .child(if result.is_some() { "done" } else { "running" }),
                            ),
                    )
                    .when_some(result, |card, result| {
                        card.child(
                            div()
                                .ml_4()
                                .pl_3()
                                .border_l_1()
                                .border_color(rgba(palette.border))
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child(first_lines(&result, 6)),
                        )
                    })
                    .into_any_element()
            }
            Row::Decided {
                name,
                allowed,
                for_session,
            } => div()
                .px_4()
                .py_1()
                .text_xs()
                .text_color(rgb(if allowed { palette.green } else { palette.red }))
                .child(format!(
                    "{} {name}{}",
                    if allowed { "allowed" } else { "denied" },
                    if for_session { " for this session" } else { "" }
                ))
                .into_any_element(),
            Row::Notice(text) => div()
                .px_4()
                .py_1()
                .text_xs()
                .text_color(rgb(palette.muted))
                .child(text)
                .into_any_element(),
        }
    }

    /// The call the run is waiting on: what it is, what it changes, and the three answers.
    fn agent_card(pending: Pending, palette: Palette, cx: &mut Context<Self>) -> gpui::AnyElement {
        let mono = ".AppleSystemUIFontMonospaced";
        let diff = pending.diff.clone().map(|text| {
            div()
                .id("finder-agent-diff")
                .flex()
                .flex_col()
                .max_h(px(180.))
                .overflow_y_scroll()
                .children(text.lines().take(60).map(|line| {
                    // The sign stays in the line: colour alone is not a diff a reader can parse.
                    let colour = if line.starts_with("+ ") {
                        palette.green
                    } else if line.starts_with("- ") {
                        palette.red
                    } else {
                        palette.muted
                    };
                    div()
                        .font_family(mono)
                        .text_xs()
                        .text_color(rgb(colour))
                        .child(line.to_owned())
                }))
        });
        let is_block = pending.is_block();
        let border_color = if is_block {
            palette.red
        } else {
            palette.accent
        };
        let tag_text = if is_block {
            match &pending.rule {
                Some(rule) => format!("blocked by policy · rule {rule}"),
                None => "blocked by policy".to_owned(),
            }
        } else {
            match &pending.rule {
                Some(rule) => format!("needs your answer · rule {rule}"),
                None => "needs your answer".to_owned(),
            }
        };
        let tag_color = if is_block {
            palette.red
        } else {
            palette.accent
        };

        let allow_button = div()
            .id("agent-allow")
            .cursor_pointer()
            .text_color(rgb(if is_block {
                palette.amber
            } else {
                palette.green
            }))
            .on_click(cx.listener(|view, _, _, cx| {
                view.agent_decide(true, false, cx);
            }))
            .child(if is_block {
                "allow once (override)"
            } else {
                "allow once"
            });

        let allow_session_button = (!is_block).then(|| {
            div()
                .id("agent-allow-session")
                .cursor_pointer()
                .text_color(rgb(palette.green))
                .on_click(cx.listener(|view, _, _, cx| {
                    view.agent_decide(true, true, cx);
                }))
                .child("allow for session")
        });

        let deny_button = div()
            .id("agent-deny")
            .cursor_pointer()
            .text_color(rgb(palette.red))
            .on_click(cx.listener(|view, _, _, cx| {
                view.agent_decide(false, false, cx);
            }))
            .child("deny");

        div()
            .flex()
            .flex_col()
            .gap_2()
            .mx_3()
            .my_2()
            .p_3()
            .rounded(px(8.))
            .bg(rgba(palette.raised))
            .border_1()
            .border_color(rgba(border_color))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .text_xs()
                    .child(div().text_color(rgb(tag_color)).child(tag_text))
                    .child(
                        div()
                            .text_color(rgb(palette.muted))
                            .child(pending.name.clone()),
                    ),
            )
            .child(
                div()
                    .font_family(mono)
                    .text_xs()
                    .child(pending.preview.clone()),
            )
            .children(diff)
            .child(
                div()
                    .flex()
                    .gap_3()
                    .pt_1()
                    .text_xs()
                    .child(allow_button)
                    .children(allow_session_button)
                    .child(deny_button),
            )
            .into_any_element()
    }

    /// The agent's half of the panel: what it is attached to, the transcript, and the answer.
    pub(crate) fn agent_panel_body(
        &mut self,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let sessions = self.agent.sessions.clone();
        let sessions_loading = self.agent.sessions_loading;
        let session_shelf = div()
            .flex()
            .flex_col()
            .border_b_1()
            .border_color(rgba(palette.border))
            .when(sessions_loading, |shelf| {
                shelf.child(
                    div()
                        .px_4()
                        .py_2()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child("Loading sessions…"),
                )
            })
            .children(sessions.into_iter().take(5).map(|session| {
                let selected = self.agent.id() == Some(session.id.as_str());
                let label = session
                    .title
                    .clone()
                    .filter(|title| !title.trim().is_empty())
                    .unwrap_or_else(|| session.root_name());
                let detail = format!(
                    "{} · {} turn(s){}",
                    session.root_name(),
                    session.turns,
                    if session.active { " · running" } else { "" }
                );
                div()
                    .id(SharedString::from(format!("agent-session-{}", session.id)))
                    .px_4()
                    .py_2()
                    .cursor_pointer()
                    .when(selected, |row| row.bg(rgba(palette.raised)))
                    .hover(move |row| row.bg(rgba(palette.raised)))
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.agent_open_session(session.clone(), cx);
                    }))
                    .child(div().text_sm().truncate().child(label))
                    .child(div().text_xs().text_color(rgb(palette.muted)).child(detail))
            }));
        let rows: Vec<gpui::AnyElement> = self
            .agent
            .rows
            .iter()
            .cloned()
            .map(|row| Self::agent_row(row, palette))
            .collect();
        let card = self
            .agent
            .pending
            .clone()
            .map(|pending| Self::agent_card(pending, palette, cx));
        let error = self.agent.error.clone();
        let status = self.agent.status();
        let running = self.agent.running;
        let attached = self.agent.session.is_some();
        let empty = rows.is_empty() && self.agent.pending.is_none();

        let stop = running.then(|| {
            div()
                .id("agent-stop")
                .cursor_pointer()
                .hover(move |link| link.text_color(rgb(palette.text)))
                .on_click(cx.listener(|view, _, _, cx| view.agent_stop(cx)))
                .child("stop")
                .into_any_element()
        });
        let fresh = (attached && !self.agent.busy()).then(|| {
            div()
                .id("agent-new")
                .cursor_pointer()
                .hover(move |link| link.text_color(rgb(palette.text)))
                .on_click(cx.listener(|view, _, _, cx| {
                    view.agent.clear();
                    view.mode = Mode::Agent;
                    view.open(cx);
                    view.agent_refresh_sessions(cx);
                    cx.notify();
                }))
                .child("new session")
                .into_any_element()
        });

        div()
            .flex()
            .flex_col()
            .child(session_shelf)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_4()
                    .py(px(6.))
                    .border_b_1()
                    .border_color(rgba(palette.border))
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child(match &self.agent.session {
                        Some(session) => {
                            let tier = session.tier.as_deref().unwrap_or("project");
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .px_2()
                                        .py(px(1.5))
                                        .rounded(px(4.))
                                        .bg(rgba(crate::theme::tint(palette.accent, 0x18)))
                                        .border_1()
                                        .border_color(rgba(crate::theme::tint(
                                            palette.accent,
                                            0x35,
                                        )))
                                        .text_color(rgb(palette.accent))
                                        .child(format!("tier: {tier}")),
                                )
                                .child(div().text_color(rgb(palette.muted)).child(format!(
                                    "{} · {}",
                                    session.root,
                                    session.plugins.join(", ")
                                )))
                                .into_any_element()
                        }
                        None => {
                            let draft_tier = self.agent.draft_tier;
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(div().text_color(rgb(palette.muted)).child("tier:"))
                                .child(tier_chip(
                                    "sjel",
                                    draft_tier == "sjel",
                                    palette,
                                    cx.listener(|view, _, _, cx| {
                                        view.agent.draft_tier = "sjel";
                                        cx.notify();
                                    }),
                                ))
                                .child(tier_chip(
                                    "project",
                                    draft_tier == "project",
                                    palette,
                                    cx.listener(|view, _, _, cx| {
                                        view.agent.draft_tier = "project";
                                        cx.notify();
                                    }),
                                ))
                                .child(tier_chip(
                                    "account",
                                    draft_tier == "account",
                                    palette,
                                    cx.listener(|view, _, _, cx| {
                                        view.agent.draft_tier = "account";
                                        cx.notify();
                                    }),
                                ))
                                .child(
                                    div()
                                        .text_color(rgb(palette.muted))
                                        .child("· in Sjel checkout"),
                                )
                                .into_any_element()
                        }
                    }),
            )
            .child(
                div()
                    .id("finder-agent")
                    .flex()
                    .flex_col()
                    .max_h(px(360.))
                    .overflow_y_scroll()
                    .track_scroll(&self.agent_scroll)
                    .when(empty, |panel| {
                        panel.child(
                            div()
                                .px_4()
                                .py_3()
                                .text_sm()
                                .text_color(rgb(palette.muted))
                                .child(
                                    "Describe a task. The agent reads, runs and edits in this \
                                     checkout, and every command and write asks you first.",
                                ),
                        )
                    })
                    .children(rows),
            )
            .children(card)
            .when_some(error, |panel, error| {
                panel.child(
                    div()
                        .px_4()
                        .py_2()
                        .text_sm()
                        .text_color(rgb(palette.red))
                        .child(error),
                )
            })
            .child(
                div()
                    .flex()
                    .gap_3()
                    .px_4()
                    .py(px(6.))
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child(status)
                    .child(div().flex_1())
                    .children(stop)
                    .children(fresh),
            )
            .into_any_element()
    }

    fn step(&mut self, by: isize, cx: &mut Context<Self>) {
        let count = self.hits().len();
        if count == 0 {
            return;
        }
        // The first arrow press picks the row the cursor is on; later ones move it.
        if self.picked {
            self.cursor = (self.cursor as isize + by).rem_euclid(count as isize) as usize;
        }
        self.picked = true;
        cx.notify();
    }
}

fn tier_chip(
    label: &'static str,
    active: bool,
    palette: Palette,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> gpui::AnyElement {
    let (bg, text_color, border_color) = if active {
        (
            rgba(crate::theme::tint(palette.accent, 0x24)),
            rgb(palette.accent),
            rgba(crate::theme::tint(palette.accent, 0x50)),
        )
    } else {
        (
            rgba(palette.raised),
            rgb(palette.muted),
            rgba(palette.border),
        )
    };
    div()
        .id(SharedString::from(format!("tier-{label}")))
        .flex()
        .items_center()
        .px_2()
        .py(px(1.5))
        .rounded(px(4.))
        .cursor_pointer()
        .bg(bg)
        .border_1()
        .border_color(border_color)
        .text_xs()
        .font_weight(if active {
            gpui::FontWeight::SEMIBOLD
        } else {
            gpui::FontWeight::NORMAL
        })
        .text_color(text_color)
        .on_click(on_click)
        .child(label)
        .into_any_element()
}

/// The first `count` lines of a tool result, with a count when there is more.
fn first_lines(text: &str, count: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut shown: String = lines
        .iter()
        .take(count)
        .copied()
        .collect::<Vec<_>>()
        .join("\n");
    if lines.len() > count {
        shown.push_str(&format!("\n… {} more line(s)", lines.len() - count));
    }
    shown
}

/// A server row as a result. Its link is opened through the deep-link parser, so the server can
/// add a source without the app learning a new kind of place.
fn server_hit(hit: ServerHit) -> Hit {
    Hit {
        kind: match hit.kind.as_str() {
            "event" => "event",
            "person" => "person",
            "document" => "document",
            "item" => "item",
            "subscription" => "subscription",
            "trip" => "trip",
            "mail" => "mail",
            _ => "result",
        },
        title: hit.title,
        detail: hit.detail,
        open: Open::Link(hit.link),
    }
}

/// The records as results: projects first, then open tasks, then notes, which is also the order
/// ties keep.
fn record_hits(
    projects: Vec<ProjectSummary>,
    tasks: Vec<GlobalTask>,
    notes: Vec<GlobalNote>,
) -> Vec<Hit> {
    let projects = projects
        .into_iter()
        .filter(|project| !project.archived)
        .map(|project| Hit {
            kind: "project",
            detail: format!("{} open · {} notes", project.open, project.notes),
            open: Open::Project(project.name.clone()),
            title: project.name,
        });
    let tasks = tasks.into_iter().filter(|task| !task.done).map(|task| Hit {
        kind: "task",
        detail: match &task.due {
            Some(due) => format!("{} · due {due}", task.project),
            None => task.project.clone(),
        },
        open: Open::Record {
            project: task.project.clone(),
            path: task.id.clone(),
        },
        title: task.title,
    });
    let notes = notes.into_iter().map(|note| Hit {
        kind: "note",
        detail: note.project.clone(),
        open: Open::Record {
            project: note.project.clone(),
            path: note.path.clone(),
        },
        title: note.title,
    });
    projects.chain(tasks).chain(notes).collect()
}

impl Focusable for Finder {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Finder {
    /// Bindings the finder carries. Up and down reach it because a one-line input lets them
    /// through, the same way the kit's command palette receives them. Escape is the
    /// workspace's window-wide binding, which closes an open finder first.
    pub fn keys() -> [gpui_kit::KeyBinding; 5] {
        [
            gpui_kit::KeyBinding::new("up", SelectUp, Some("Finder")),
            gpui_kit::KeyBinding::new("down", SelectDown, Some("Finder")),
            gpui_kit::KeyBinding::new("ctrl-p", SelectUp, Some("Finder")),
            gpui_kit::KeyBinding::new("ctrl-n", SelectDown, Some("Finder")),
            gpui_kit::KeyBinding::new("cmd-w", Close, Some("Finder")),
        ]
    }

    /// Tab is the input's own key (it indents), so the mode cycles on ⌘/ instead.
    pub fn mode_key() -> gpui_kit::KeyBinding {
        gpui_kit::KeyBinding::new("cmd-/", NextMode, Some("Finder"))
    }
}

impl Render for Finder {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.open {
            return div().into_any_element();
        }
        let palette = Palette::for_appearance(window.appearance());
        let input = self.ensure_input(window, cx);
        let resolved = self.resolved();
        let typing = !self.query.trim().is_empty();
        let agent_mode = resolved == Mode::Agent;
        // The conversation shows while the text is empty or is itself a question; a search
        // typed under it gets the panel to itself.
        let thread = if self.asked && !agent_mode && (!typing || resolved == Mode::Ask) {
            self.ask.read(cx).exchanges()
        } else {
            Vec::new()
        };
        let threaded = !thread.is_empty();
        // Under a conversation the page list is noise; typing brings results back.
        let hits: Vec<Hit> = if agent_mode || (threaded && !typing) {
            Vec::new()
        } else {
            self.hits().into_iter().cloned().collect()
        };
        let nothing = hits.is_empty();

        let modes = div().flex().gap_1().children(Mode::ALL.map(|mode| {
            let chosen = mode == self.mode;
            div()
                .id(SharedString::from(format!("finder-mode-{}", mode.label())))
                .px_2()
                .py(px(2.))
                .rounded(px(5.))
                .text_xs()
                .cursor_pointer()
                .bg(rgba(if chosen {
                    tint(palette.accent, 0x2a)
                } else {
                    palette.raised
                }))
                .text_color(rgb(if chosen {
                    palette.accent
                } else {
                    palette.muted
                }))
                .on_click(cx.listener(move |view, _, _, cx| {
                    view.mode = mode;
                    view.focus_next = true;
                    view.search_server_soon(cx);
                    if mode == Mode::Agent {
                        view.agent_refresh_sessions(cx);
                    }
                    cx.notify();
                }))
                .child(mode.label())
        }));

        // The decision Auto made, where the eye already is.
        let decision = (self.mode == Mode::Auto && !self.query.trim().is_empty()).then(|| {
            div()
                .text_xs()
                .text_color(rgb(palette.muted))
                .child(format!("↩ {}", resolved.label().to_lowercase()))
        });

        let list = div()
            .flex()
            .flex_col()
            .py_1()
            .children(hits.into_iter().enumerate().map(|(index, hit)| {
                // While Enter would ask, no row looks chosen; an arrow key chooses one.
                let selected = index == self.cursor && (resolved == Mode::Search || self.picked);
                let open = hit.open.clone();
                div()
                    .id(SharedString::from(format!("finder-hit-{index}")))
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_3()
                    .py(px(6.))
                    .mx_1()
                    .rounded(px(6.))
                    .cursor_pointer()
                    .when(selected, |row| row.bg(rgba(palette.raised)))
                    .hover(move |row| row.bg(rgba(palette.raised)))
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.close(cx);
                        cx.emit(FinderEvent::Open(open.clone()));
                    }))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(div().text_sm().truncate().child(hit.title))
                            .when(!hit.detail.is_empty(), |cell| {
                                cell.child(
                                    div()
                                        .text_xs()
                                        .truncate()
                                        .text_color(rgb(palette.muted))
                                        .child(hit.detail),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(hit.kind),
                    )
            }));

        // Nothing found says so, and says what Enter does instead of leaving an empty box.
        let empty = (nothing && !threaded && typing && !agent_mode).then(|| {
            let next = if self.searching.is_some() {
                "Searching Sjel…"
            } else if self.mode == Mode::Search {
                "Nothing found. ⌘↩ asks Sjel instead."
            } else {
                "Nothing found. ↩ asks Sjel instead."
            };
            div()
                .px_4()
                .py_3()
                .text_sm()
                .text_color(rgb(palette.muted))
                .child(next)
        });

        let (can_stop, loading, model) = {
            let ask = self.ask.read(cx);
            (ask.can_stop(), ask.loading, ask.model())
        };
        let link = move |id: &'static str| {
            div()
                .id(id)
                .cursor_pointer()
                .hover(move |link| link.text_color(rgb(palette.text)))
        };
        let thread = threaded.then(|| {
            div()
                .flex()
                .flex_col()
                .border_b_1()
                .border_color(rgba(palette.border))
                .child(
                    div()
                        .id("finder-thread")
                        .flex()
                        .flex_col()
                        .max_h(px(360.))
                        .overflow_y_scroll()
                        .track_scroll(&self.thread_scroll)
                        .children(thread.into_iter().map(|exchange| {
                            let status = match (exchange.loading, exchange.tools.is_empty()) {
                                (true, true) => "working…".to_owned(),
                                (true, false) => format!("working · {}", exchange.tools.join(", ")),
                                (false, true) => String::new(),
                                (false, false) => format!("used {}", exchange.tools.join(", ")),
                            };
                            div()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .px_4()
                                .py_2()
                                .when(!exchange.prompt.is_empty(), |card| {
                                    card.child(
                                        div()
                                            .text_xs()
                                            .text_color(rgb(palette.muted))
                                            .child(exchange.prompt),
                                    )
                                })
                                .when(!exchange.answer.is_empty(), |card| {
                                    card.child(div().text_sm().child(exchange.answer))
                                })
                                .when_some(exchange.error, |card, error| {
                                    card.child(
                                        div().text_sm().text_color(rgb(palette.red)).child(error),
                                    )
                                })
                                .when(!status.is_empty() || exchange.meta.is_some(), |card| {
                                    card.child(
                                        div()
                                            .flex()
                                            .gap_2()
                                            .text_xs()
                                            .text_color(rgb(palette.muted))
                                            .child(status)
                                            .child(div().flex_1())
                                            .children(exchange.meta),
                                    )
                                })
                        })),
                )
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap_3()
                        .px_4()
                        .py(px(6.))
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .when(can_stop, |row| {
                            row.child(
                                link("finder-stop")
                                    .on_click(cx.listener(|view, _, _, cx| {
                                        view.ask.update(cx, |ask, cx| ask.stop(cx));
                                    }))
                                    .child("stop"),
                            )
                        })
                        .when(!loading, |row| {
                            row.child(
                                link("finder-new")
                                    .on_click(cx.listener(|view, _, _, cx| {
                                        view.ask.update(cx, |ask, cx| ask.clear(cx));
                                        view.asked = false;
                                        view.focus_next = true;
                                        cx.notify();
                                    }))
                                    .child("new conversation"),
                            )
                        }),
                )
        });

        // Where the next question goes. Cloud gets the current prompt only, once it is marked
        // public (c0); the session refuses to send it before that.
        let model = match model {
            Model::Local { error } => link("finder-model")
                .on_click(cx.listener(move |view, _, _, cx| {
                    let retry = error.is_some();
                    view.ask.update(cx, |ask, cx| {
                        if retry {
                            ask.retry_providers(cx);
                        } else {
                            ask.next_model(cx);
                        }
                    });
                }))
                .child("local model")
                .into_any_element(),
            Model::Cloud { label, confirmed } => div()
                .flex()
                .gap_2()
                .child(
                    link("finder-model")
                        .text_color(rgb(palette.accent))
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.ask.update(cx, |ask, cx| ask.next_model(cx));
                        }))
                        .child(label),
                )
                .child(
                    link("finder-public")
                        .text_color(rgb(if confirmed {
                            palette.green
                        } else {
                            palette.red
                        }))
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.ask.update(cx, |ask, cx| ask.toggle_public(cx));
                        }))
                        .child(if confirmed {
                            "public c0 ✓"
                        } else {
                            "mark public c0"
                        }),
                )
                .into_any_element(),
        };

        let hint = if agent_mode {
            "↩ start or steer · run stays in the right panel"
        } else {
            "↩ open or ask"
        };

        let panel = div()
            .key_context("Finder")
            .track_focus(&self.focus)
            .on_action(cx.listener(|view, _: &SelectUp, _, cx| view.step(-1, cx)))
            .on_action(cx.listener(|view, _: &SelectDown, _, cx| view.step(1, cx)))
            .on_action(cx.listener(|view, _: &NextMode, _, cx| {
                view.mode = view.mode.next();
                view.search_server_soon(cx);
                if view.mode == Mode::Agent {
                    view.agent_refresh_sessions(cx);
                }
                cx.notify();
            }))
            .on_action(cx.listener(|view, _: &Close, _, cx| view.close(cx)))
            // A click inside the panel is not a click on the backdrop.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .w(px(WIDTH))
            .max_h(px(600.))
            .flex()
            .flex_col()
            .rounded(px(12.))
            .overflow_hidden()
            .bg(rgba(palette.panel()))
            .border_1()
            .border_color(rgba(palette.border))
            .shadow(vec![gpui_kit::BoxShadow::new(
                px(0.),
                px(12.),
                gpui_kit::hsla(0., 0., 0., 0.22),
            )
            .blur_radius(px(40.))])
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(rgba(palette.border))
                    .child(div().flex_1().child(Input::new(&input).appearance(false)))
                    .children(decision)
                    .child(modes),
            )
            .children(thread)
            .children(empty)
            .when(!agent_mode, |panel| {
                panel.child(
                    // Sized to its rows: two results make a short panel, not a tall empty one.
                    div()
                        .id("finder-list")
                        .max_h(px(420.))
                        .overflow_y_scroll()
                        .child(list),
                )
            })
            .child(
                div()
                    .flex()
                    .gap_3()
                    .px_3()
                    .py(px(6.))
                    .border_t_1()
                    .border_color(rgba(palette.border))
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .when_some(self.searching.as_ref(), |footer, _| {
                        footer.child(
                            div()
                                .text_color(rgb(palette.accent))
                                .child("searching Sjel…"),
                        )
                    })
                    .child(hint)
                    .child("⌘↩ the other way")
                    .child("⌘/ mode")
                    .child("esc close")
                    .child(div().flex_1())
                    .child(model),
            );

        div()
            .id("finder-backdrop")
            .absolute()
            .inset_0()
            .flex()
            .justify_center()
            // The panel is as tall as its rows: a stretched one is an empty box under them.
            .items_start()
            .pt(px(72.))
            .bg(gpui_kit::hsla(0., 0., 0., 0.18))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, _, _, cx| view.close(cx)),
            )
            .child(panel)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every link the server sends (`sjel-status/src/status/search.rs`) opens a native page.
    #[test]
    fn every_server_link_opens_a_native_page() {
        use crate::route::{parse_deep_link, Target};
        for link in [
            "sjel://calendar",
            "sjel://people",
            "sjel://documents",
            "sjel://inventory",
            "sjel://finance",
            "sjel://inbox",
        ] {
            assert!(
                matches!(parse_deep_link(link), Some(Target::Native(_))),
                "{link}"
            );
        }
        assert!(matches!(
            parse_deep_link("sjel://travel/t1/overview"),
            Some(Target::Travel(_))
        ));
        let hit = server_hit(ServerHit {
            kind: "mystery".into(),
            title: "x".into(),
            detail: String::new(),
            link: "sjel://inbox".into(),
        });
        assert_eq!(hit.kind, "result");
    }

    #[test]
    fn records_become_results_in_kind_order() {
        let projects =
            vec![
                serde_json::from_str::<ProjectSummary>(r#"{"name":"Atlas","open":2,"notes":3}"#)
                    .unwrap(),
            ];
        let tasks = vec![
            serde_json::from_str::<GlobalTask>(
                r#"{"id":"Atlas/Tasks/a.md","title":"Call Anna","project":"Atlas","due":"2026-10-24"}"#,
            )
            .unwrap(),
            serde_json::from_str::<GlobalTask>(
                r#"{"id":"Atlas/Tasks/b.md","title":"Done thing","project":"Atlas","done":true}"#,
            )
            .unwrap(),
        ];
        let notes = vec![serde_json::from_str::<GlobalNote>(
            r#"{"path":"Atlas/Notes/n.md","title":"Anna notes","project":"Atlas"}"#,
        )
        .unwrap()];
        let hits = record_hits(projects, tasks, notes);
        let kinds: Vec<&str> = hits.iter().map(|hit| hit.kind).collect();
        assert_eq!(
            kinds,
            ["project", "task", "note"],
            "a done task is not a result"
        );
        assert_eq!(hits[1].detail, "Atlas · due 2026-10-24");
        let refs: Vec<&Hit> = hits.iter().collect();
        let ranked = search::rank("anna", &refs, |hit| (hit.title.as_str(), vec![]));
        let found: Vec<&str> = ranked
            .into_iter()
            .map(|index| refs[index].title.as_str())
            .collect();
        assert_eq!(found, ["Anna notes", "Call Anna"]);
    }
}
