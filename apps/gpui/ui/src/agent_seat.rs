//! The agent seat: one run's controller and panel, drawn by the Mac's finder and edge and by the
//! phone's `Seat::Agent`.
//!
//! The run lives in the agent capability, not here. This entity attaches to it through a
//! [`Host`], follows its stream, and sends the owner's prompts, guidance and decisions. The shell
//! owns the text field and calls [`AgentSeat::submit`].

use std::time::Duration;

use futures::StreamExt;
use gpui::{
    div, prelude::*, px, rgb, rgba, AnyElement, ClickEvent, Context, EventEmitter, FontWeight,
    MouseButton, MouseDownEvent, ScrollHandle, SharedString, Window,
};
use sjel_gpui_core::agent::Event;
pub use sjel_gpui_core::agent::{tool_summary, AgentRun, Host, Pending, Row, Session, Turn};
use sjel_gpui_core::shell;

use crate::theme::{tint, Palette, ThemeContext};

/// Reconnects a dropped stream this many times before the panel says so.
const RECONNECTS: u32 = 5;
const RECONNECT_DELAY: Duration = Duration::from_secs(2);

pub enum AgentSeatEvent {
    /// A new run started from this seat.
    Started,
    /// The owner asked for a fresh session; the shell focuses its text field.
    NewSession,
}

pub struct AgentSeat {
    /// `None` until the shell can reach the Mac: an unpaired phone, or a Mac without its shell.
    host: Option<Host>,
    pub run: AgentRun,
    scroll: ScrollHandle,
    /// Phone layout: the transcript fills the seat and every control is a touch target.
    touch: bool,
    /// What the seat says while `host` is `None`.
    offline: SharedString,
}

impl EventEmitter<AgentSeatEvent> for AgentSeat {}

impl AgentSeat {
    pub fn new(host: Option<Host>, touch: bool, offline: impl Into<SharedString>) -> Self {
        Self {
            host,
            run: AgentRun::default(),
            scroll: ScrollHandle::new(),
            touch,
            offline: offline.into(),
        }
    }

    pub fn set_host(&mut self, host: Option<Host>, cx: &mut Context<Self>) {
        self.host = host;
        cx.notify();
    }

    pub fn connected(&self) -> bool {
        self.host.is_some()
    }

    /// Guidance for the run that is working, or a prompt that starts or continues the session.
    pub fn submit(&mut self, text: String, cx: &mut Context<Self>) {
        if self.run.running {
            self.steer(text, cx);
        } else {
            self.send(text, cx);
        }
    }

    pub fn new_session(&mut self, cx: &mut Context<Self>) {
        let sessions = std::mem::take(&mut self.run.sessions);
        let draft_tier = self.run.draft_tier;
        self.run.clear();
        self.run.sessions = sessions;
        self.run.draft_tier = draft_tier;
        cx.emit(AgentSeatEvent::NewSession);
        self.refresh_sessions(cx);
        cx.notify();
    }

    /// Starts a run, or continues the session this seat is attached to.
    ///
    /// The whole chain — create the session if there is none, send the prompt, attach the stream
    /// — runs in one future on the HTTP runtime, so a slow capability cannot leave the seat with
    /// a session and no stream.
    fn send(&mut self, text: String, cx: &mut Context<Self>) {
        let Some(host) = self.host.clone() else {
            self.run.error = Some(self.offline.to_string());
            cx.notify();
            return;
        };
        let existing = self.run.id().map(str::to_owned);
        let draft_tier = self.run.draft_tier;
        self.run.error = None;
        self.run.running = true;
        cx.emit(AgentSeatEvent::Started);
        self.scroll.scroll_to_bottom();
        cx.notify();

        let work = shell::run(async move {
            let session = match existing {
                Some(id) => host
                    .agent_sessions()
                    .await
                    .unwrap_or_default()
                    .into_iter()
                    .find(|session| session.id == id)
                    .unwrap_or(Session {
                        id,
                        ..Default::default()
                    }),
                None => host.agent_create(draft_tier).await?,
            };
            host.agent_prompt(&session.id, &text).await?;
            let stream = host.agent_stream(&session.id, 0).await?;
            Ok::<_, String>((session, stream))
        });

        cx.spawn(async move |this, cx| {
            let (session, stream) = match work.await {
                Some(Ok(answer)) => answer,
                failed => {
                    let why = match failed {
                        Some(Err(why)) => why,
                        _ => "the request was dropped".to_owned(),
                    };
                    let _ = this.update(cx, |seat, cx| {
                        seat.run.running = false;
                        seat.run.error = Some(format!("The run did not start: {why}"));
                        cx.notify();
                    });
                    return;
                }
            };
            let _ = this.update(cx, |seat, cx| {
                if seat.run.session.is_none() {
                    seat.run.session = Some(session.clone());
                }
                seat.run.sessions.retain(|listed| listed.id != session.id);
                seat.run.sessions.insert(0, session.clone());
                seat.run.running = true;
                seat.follow(session, stream, 0, cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub fn refresh_sessions(&mut self, cx: &mut Context<Self>) {
        let Some(host) = self.host.clone() else {
            return;
        };
        if self.run.sessions_loading {
            return;
        }
        self.run.sessions_loading = true;
        let work = shell::run(async move { host.agent_sessions().await });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |seat, cx| {
                seat.run.sessions_loading = false;
                match result {
                    Some(Ok(sessions)) => seat.run.sessions = sessions,
                    Some(Err(why)) => {
                        seat.run.error = Some(format!("Sessions did not load: {why}"))
                    }
                    None => {}
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn follow(
        &mut self,
        session: Session,
        mut stream: futures::channel::mpsc::UnboundedReceiver<Event>,
        mut cursor: u64,
        cx: &mut Context<Self>,
    ) {
        let Some(host) = self.host.clone() else {
            return;
        };
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
                        .update(cx, |seat, cx| {
                            if seat.run.id() != Some(session.id.as_str()) {
                                return false;
                            }
                            seat.run.apply(event);
                            seat.scroll.scroll_to_bottom();
                            if terminal {
                                seat.run.running = false;
                                seat.refresh_sessions(cx);
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
                    let host = host.clone();
                    let id = session.id.clone();
                    let work = shell::run(async move {
                        let turns = host.agent_transcript(&id).await?;
                        let stream = host.agent_stream(&id, cursor).await?;
                        Ok::<_, String>((turns, stream))
                    });
                    let Some(Ok((mut turns, resumed))) = work.await else {
                        let _ = this.update(cx, |seat, cx| {
                            seat.run.running = false;
                            seat.run.error = Some(
                                "Could not reload the agent transcript after stream truncation."
                                    .into(),
                            );
                            cx.notify();
                        });
                        return;
                    };
                    if cursor == 0 && session.active && ends_with_prompt(&turns) {
                        turns.pop();
                    }
                    let restored = this
                        .update(cx, |seat, cx| {
                            if seat.run.id() != Some(session.id.as_str()) {
                                return false;
                            }
                            seat.run.restore(session.clone(), turns);
                            seat.run.last_seq = cursor;
                            seat.run
                                .rows
                                .push(Row::Notice("Transcript reloaded; stream resumed.".into()));
                            seat.scroll.scroll_to_bottom();
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
                if reconnects >= RECONNECTS {
                    let _ = this.update(cx, |seat, cx| {
                        if seat.run.id() == Some(session.id.as_str()) {
                            seat.run.running = false;
                            seat.run.error = Some(
                                "Agent stream disconnected. Reopen the session to reconnect."
                                    .into(),
                            );
                            cx.notify();
                        }
                    });
                    return;
                }
                cx.background_executor().timer(RECONNECT_DELAY).await;
                let host = host.clone();
                let id = session.id.clone();
                if let Some(Ok(resumed)) =
                    shell::run(async move { host.agent_stream(&id, cursor).await }).await
                {
                    stream = resumed;
                }
            }
        })
        .detach();
    }

    /// Attaches to a listed session: its stored turns, then its stream while it runs.
    pub fn open_session(&mut self, session: Session, cx: &mut Context<Self>) {
        let Some(host) = self.host.clone() else {
            return;
        };
        let attached = self.run.id() == Some(session.id.as_str());
        let cursor = if attached { self.run.last_seq } else { 0 };
        let read = session.clone();
        let work = shell::run(async move {
            let mut turns = if attached {
                Vec::new()
            } else {
                host.agent_transcript(&read.id).await?
            };
            if read.active && ends_with_prompt(&turns) {
                turns.pop();
            }
            let stream = if read.active {
                Some(host.agent_stream(&read.id, cursor).await?)
            } else {
                None
            };
            Ok::<_, String>((turns, stream))
        });
        cx.spawn(async move |this, cx| {
            let (turns, stream) = match work.await {
                Some(Ok(answer)) => answer,
                Some(Err(why)) => {
                    let _ = this.update(cx, |seat, cx| {
                        seat.run.error = Some(format!("The session did not open: {why}"));
                        cx.notify();
                    });
                    return;
                }
                None => return,
            };
            let _ = this.update(cx, |seat, cx| {
                if !attached {
                    seat.run.restore(session.clone(), turns);
                }
                if let Some(stream) = stream {
                    seat.follow(session, stream, cursor, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// A message while a run is active steers it instead of starting a second one.
    fn steer(&mut self, text: String, cx: &mut Context<Self>) {
        let (Some(host), Some(id)) = (self.host.clone(), self.run.id().map(str::to_owned)) else {
            return;
        };
        self.run.rows.push(Row::Prompt(text.clone()));
        cx.notify();
        let work = shell::run(async move { host.agent_steer(&id, &text).await });
        cx.spawn(async move |this, cx| {
            if !matches!(work.await, Some(Ok(()))) {
                let _ = this.update(cx, |seat, cx| {
                    seat.run.error = Some("The message was not queued.".into());
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// The owner's answer to the call that is waiting.
    pub fn decide(&mut self, allow: bool, for_session: bool, cx: &mut Context<Self>) {
        let (Some(host), Some(pending), Some(id)) = (
            self.host.clone(),
            self.run.pending.clone(),
            self.run.id().map(str::to_owned),
        ) else {
            return;
        };
        let approval = pending.approval.clone();
        let work =
            shell::run(async move { host.agent_approve(&id, &approval, allow, for_session).await });
        cx.spawn(async move |this, cx| {
            let answer = work.await;
            let _ = this.update(cx, |seat, cx| {
                match answer {
                    Some(Ok(())) => {
                        // The `approved` or `denied` event clears the card; this only makes the
                        // seat respond now rather than on the next frame of the stream.
                        seat.run.pending = None;
                        if allow {
                            seat.run.rows.push(Row::Decided {
                                name: pending.name,
                                allowed: true,
                                for_session,
                            });
                        }
                    }
                    _ => seat.run.error = Some("The decision did not reach the agent.".into()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn stop(&mut self, cx: &mut Context<Self>) {
        let (Some(host), Some(id)) = (self.host.clone(), self.run.id().map(str::to_owned)) else {
            return;
        };
        let work = shell::run(async move { host.agent_stop(&id).await });
        cx.spawn(async move |this, cx| {
            let answer = work.await;
            let _ = this.update(cx, |seat, cx| {
                seat.run.running = false;
                if !matches!(answer, Some(Ok(()))) {
                    seat.run.error = Some("The stop did not reach the agent.".into());
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// A control: a text link on the Mac, a padded 44-point target on the phone.
    fn control(
        &self,
        id: &'static str,
        label: &'static str,
        colour: u32,
    ) -> gpui::Stateful<gpui::Div> {
        div()
            .id(id)
            .cursor_pointer()
            .text_color(rgb(colour))
            .when(self.touch, |button| {
                button
                    .min_h(px(44.))
                    .flex()
                    .items_center()
                    .px_3()
                    .rounded(px(7.))
            })
            .child(label)
    }

    /// The call the run is waiting on: what it is, what it changes, and the three answers.
    fn card(&self, pending: Pending, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let mono = ".AppleSystemUIFontMonospaced";
        let diff = pending.diff.clone().map(|text| {
            div()
                .id("agent-diff")
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
        let accent = if is_block {
            palette.red
        } else {
            palette.accent
        };
        let tag = match (is_block, &pending.rule) {
            (true, Some(rule)) => format!("blocked by policy · rule {rule}"),
            (true, None) => "blocked by policy".to_owned(),
            (false, Some(rule)) => format!("needs your answer · rule {rule}"),
            (false, None) => "needs your answer".to_owned(),
        };
        let allow = self
            .control(
                "agent-allow",
                if is_block {
                    "allow once (override)"
                } else {
                    "allow once"
                },
                if is_block {
                    palette.amber
                } else {
                    palette.green
                },
            )
            .on_click(cx.listener(|seat, _, _, cx| seat.decide(true, false, cx)));
        let allow_session = (!is_block).then(|| {
            self.control("agent-allow-session", "allow for session", palette.green)
                .on_click(cx.listener(|seat, _, _, cx| seat.decide(true, true, cx)))
        });
        let deny = self
            .control("agent-deny", "deny", palette.red)
            .on_click(cx.listener(|seat, _, _, cx| seat.decide(false, false, cx)));

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
            .border_color(rgba(accent))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .text_xs()
                    .child(div().text_color(rgb(accent)).child(tag))
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
                    .flex_wrap()
                    .gap_3()
                    .pt_1()
                    .text_xs()
                    .child(allow)
                    .children(allow_session)
                    .child(deny),
            )
            .into_any_element()
    }

    fn tier_header(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        if let Some(session) = &self.run.session {
            let tier = session.tier.as_deref().unwrap_or("project");
            return div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .px_2()
                        .py(px(1.5))
                        .rounded(px(4.))
                        .bg(rgba(tint(palette.accent, 0x18)))
                        .border_1()
                        .border_color(rgba(tint(palette.accent, 0x35)))
                        .text_color(rgb(palette.accent))
                        .child(format!("tier: {tier}")),
                )
                .child(div().text_color(rgb(palette.muted)).child(format!(
                    "{} · {}",
                    session.root,
                    session.plugins.join(", ")
                )))
                .into_any_element();
        }
        let draft = self.run.draft_tier;
        let mut header = div()
            .flex()
            .items_center()
            .gap_2()
            .child(div().text_color(rgb(palette.muted)).child("tier:"));
        for tier in ["sjel", "project", "account"] {
            header = header.child(tier_pill(
                "agent",
                tier,
                draft == tier,
                palette,
                cx.listener(move |seat, _, _, cx| {
                    seat.run.draft_tier = tier;
                    cx.notify();
                }),
            ));
        }
        header.into_any_element()
    }
}

impl Render for AgentSeat {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = ThemeContext::of(window, cx).palette;
        if self.host.is_none() {
            return div()
                .p_4()
                .text_sm()
                .text_color(rgb(palette.muted))
                .child(self.offline.clone())
                .into_any_element();
        }
        let shelf = div()
            .flex()
            .flex_col()
            .border_b_1()
            .border_color(rgba(palette.border))
            .when(self.run.sessions_loading, |shelf| {
                shelf.child(
                    div()
                        .px_4()
                        .py_2()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child("Loading sessions…"),
                )
            })
            .children(self.run.sessions.iter().take(5).cloned().map(|session| {
                let selected = self.run.id() == Some(session.id.as_str());
                let detail = format!(
                    "{} · {} turn(s){}",
                    session.root_name(),
                    session.turns,
                    if session.active { " · running" } else { "" }
                );
                let label = session
                    .title
                    .clone()
                    .filter(|title| !title.trim().is_empty())
                    .unwrap_or_else(|| session.root_name());
                div()
                    .id(SharedString::from(format!("agent-session-{}", session.id)))
                    .px_4()
                    .py_2()
                    .when(self.touch, |row| row.min_h(px(44.)))
                    .cursor_pointer()
                    .when(selected, |row| row.bg(rgba(palette.raised)))
                    .hover(move |row| row.bg(rgba(palette.raised)))
                    .on_click(cx.listener(move |seat, _, _, cx| {
                        seat.open_session(session.clone(), cx);
                    }))
                    .child(div().text_sm().truncate().child(label))
                    .child(div().text_xs().text_color(rgb(palette.muted)).child(detail))
            }));
        let rows: Vec<AnyElement> = self
            .run
            .rows
            .iter()
            .cloned()
            .map(|row| transcript_row(row, palette))
            .collect();
        let empty = rows.is_empty() && self.run.pending.is_none();
        let card = self
            .run
            .pending
            .clone()
            .map(|pending| self.card(pending, palette, cx));
        let stop = self.run.running.then(|| {
            self.control("agent-stop", "stop", palette.muted)
                .hover(move |link| link.text_color(rgb(palette.text)))
                .on_click(cx.listener(|seat, _, _, cx| seat.stop(cx)))
        });
        let fresh = (self.run.session.is_some() && !self.run.busy()).then(|| {
            self.control("agent-new", "new session", palette.muted)
                .hover(move |link| link.text_color(rgb(palette.text)))
                .on_click(cx.listener(|seat, _, _, cx| seat.new_session(cx)))
        });
        let transcript = div()
            .id("agent-transcript")
            .flex()
            .flex_col()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .map(|panel| {
                if self.touch {
                    panel.flex_1().min_h_0()
                } else {
                    panel.max_h(px(360.))
                }
            })
            .when(empty, |panel| {
                panel.child(
                    div()
                        .px_4()
                        .py_3()
                        .text_sm()
                        .text_color(rgb(palette.muted))
                        .child(
                            "Describe a task. The agent reads, runs and edits in the Mac's \
                             checkout, and every command and write asks you first.",
                        ),
                )
            })
            .children(rows);

        div()
            .flex()
            .flex_col()
            .when(self.touch, |seat| seat.flex_1().min_h_0())
            .child(shelf)
            .child(
                div()
                    .px_4()
                    .py(px(6.))
                    .border_b_1()
                    .border_color(rgba(palette.border))
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child(self.tier_header(palette, cx)),
            )
            .child(transcript)
            .children(card)
            .when_some(self.run.error.clone(), |seat, error| {
                seat.child(
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
                    .items_center()
                    .gap_3()
                    .px_4()
                    .py(px(6.))
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child(self.run.status())
                    .child(div().flex_1())
                    .children(stop)
                    .children(fresh),
            )
            .into_any_element()
    }
}

fn ends_with_prompt(turns: &[Turn]) -> bool {
    turns
        .last()
        .is_some_and(|turn| turn.role.as_str() == "user")
}

/// The first `count` lines of a tool result, with a count when there is more.
fn first_lines(text: &str, count: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut shown = lines[..lines.len().min(count)].join("\n");
    if lines.len() > count {
        shown.push_str(&format!("\n… {} more line(s)", lines.len() - count));
    }
    shown
}

/// One transcript row. No listeners, so it can be built while the seat is borrowed.
fn transcript_row(row: Row, palette: Palette) -> AnyElement {
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
            let detail = tool_summary(&name, &arguments);
            let done = result.is_some();
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
                                .text_color(rgb(if done { palette.green } else { palette.muted }))
                                .child(if done { "done" } else { "running" }),
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

/// Renders a session tier selector pill (`sjel`, `project`, `account`).
pub fn tier_pill(
    id_prefix: &str,
    label: &'static str,
    active: bool,
    palette: Palette,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> AnyElement {
    let (bg, text_color, border_color) = if active {
        (
            rgba(tint(palette.accent, 0x24)),
            rgb(palette.accent),
            rgba(tint(palette.accent, 0x50)),
        )
    } else {
        (
            rgba(palette.raised),
            rgb(palette.muted),
            rgba(palette.border),
        )
    };
    div()
        .id(SharedString::from(format!("{id_prefix}-tier-{label}")))
        .flex()
        .items_center()
        .px_2()
        .py(px(3.))
        .rounded(px(5.))
        .cursor_pointer()
        .bg(bg)
        .border_1()
        .border_color(border_color)
        .text_xs()
        .font_weight(if active {
            FontWeight::SEMIBOLD
        } else {
            FontWeight::NORMAL
        })
        .text_color(text_color)
        .on_click(on_click)
        .child(label)
        .into_any_element()
}

/// Renders a session row in the session shelf/list.
pub fn session_row(
    id_prefix: &str,
    session: &Session,
    selected: bool,
    palette: Palette,
    on_select: impl Fn(&MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
) -> AnyElement {
    let id = &session.id;
    let label = session.title.clone().unwrap_or_else(|| session.root_name());
    div()
        .id(SharedString::from(format!("{id_prefix}-session-{id}")))
        .flex()
        .items_center()
        .gap_2()
        .px_3()
        .py_2()
        .rounded(px(7.))
        .cursor_pointer()
        .when(selected, |row| row.bg(rgba(palette.raised)))
        .text_color(rgb(palette.text))
        .child(div().flex_1().truncate().child(label))
        .child(
            div()
                .text_xs()
                .text_color(rgb(if session.active {
                    palette.green
                } else {
                    palette.muted
                }))
                .child(if session.active { "live" } else { "saved" }),
        )
        .on_mouse_down(MouseButton::Left, on_select)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn an_unpaired_seat_says_why_and_a_new_session_keeps_the_tier(cx: &mut gpui::TestAppContext) {
        let seat = cx.new(|_| AgentSeat::new(None, true, "Pair first."));
        seat.update(cx, |seat, cx| {
            seat.run.draft_tier = "sjel";
            seat.submit("hello".into(), cx);
            assert_eq!(seat.run.error.as_deref(), Some("Pair first."));
            assert!(!seat.run.running);
            seat.new_session(cx);
            assert_eq!(seat.run.draft_tier, "sjel");
            assert!(seat.run.error.is_none());
        });
    }
}
