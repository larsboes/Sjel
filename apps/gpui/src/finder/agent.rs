//! Agent panel and session streaming for the finder.
use std::time::Duration;

use futures::StreamExt;
use gpui_kit::{div, prelude::*, px, rgb, rgba, Context, SharedString, Window};
use sjel_gpui_core::agent::{Pending, Row};
use sjel_gpui_core::search::Mode;
use sjel_gpui_ui::theme::{tint, Palette};

use crate::shell;

use super::search::{first_lines, tier_chip};
use super::types::FinderEvent;
use super::Finder;

impl Finder {
    /// Starts a run, or continues the session this panel is attached to.
    ///
    /// The whole chain — create the session if there is none, send the prompt, attach the stream
    /// — runs in one future on the HTTP runtime, so a slow capability cannot leave the panel with
    /// a session and no stream.
    pub(super) fn agent_send(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
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

    pub(super) fn agent_refresh_sessions(&mut self, cx: &mut Context<Self>) {
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

    pub(super) fn agent_follow(
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

    pub(super) fn agent_open_session(
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
    pub(super) fn agent_steer(
        &mut self,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
    pub(super) fn agent_decide(&mut self, allow: bool, for_session: bool, cx: &mut Context<Self>) {
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

    pub(super) fn agent_stop(&mut self, cx: &mut Context<Self>) {
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
    pub(super) fn agent_row(row: Row, palette: Palette) -> gpui::AnyElement {
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
    pub(super) fn agent_card(
        pending: Pending,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
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
}
