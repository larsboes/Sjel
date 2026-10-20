//! Native multi-turn Ask session (`sjel://ask?q=…` and `Page::Ask`).
//!
//! Runs multi-turn capability queries in-session through `POST /assistant/api/ask` (which
//! drives `libs/agent` with read-only Sjel tools and no `bash`), keeping the conversation
//! inside the native GPUI window instead of handing off to a browser tab.

use std::sync::Arc;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme, Disableable, Sizable};
use gpui_kit::{
    div, prelude::*, px, rgb, rgba, AnyElement, Context, Entity, FontWeight, SharedString,
    Subscription, Window,
};

use crate::shell::{self, AskTurn, Shell, ShellError};
use crate::theme::Palette;
use crate::ui::{list, note, row, section};

const STARTERS: &[&str] = &[
    "What is on my calendar today and tomorrow?",
    "Which capabilities are up right now?",
    "List my active projects and open tasks",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    User,
    Assistant,
    Error,
}

#[derive(Clone, Debug)]
struct Turn {
    role: Role,
    content: String,
    meta: Option<String>,
}

pub struct Ask {
    shell: Result<Arc<Shell>, ShellError>,
    turns: Vec<Turn>,
    pub loading: bool,
    input: Option<(Entity<InputState>, Subscription)>,
    focus_next: bool,
}

impl Ask {
    pub fn new(shell: Result<Arc<Shell>, ShellError>) -> Self {
        Self {
            shell,
            turns: Vec::new(),
            loading: false,
            input: None,
            focus_next: true,
        }
    }

    pub fn focus_input(&mut self, cx: &mut Context<Self>) {
        self.focus_next = true;
        cx.notify();
    }

    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.turns.clear();
        self.loading = false;
        self.focus_next = true;
        cx.notify();
    }

    pub fn submit(&mut self, raw: String, cx: &mut Context<Self>) {
        let prompt = raw.trim().to_owned();
        if prompt.is_empty() || self.loading {
            return;
        }
        let shell = match &self.shell {
            Ok(shell) => shell.clone(),
            Err(error) => {
                self.turns.push(Turn {
                    role: Role::Error,
                    content: error.to_string(),
                    meta: None,
                });
                cx.notify();
                return;
            }
        };
        let history: Vec<AskTurn> = self
            .turns
            .iter()
            .filter_map(|t| match t.role {
                Role::User => Some(AskTurn {
                    role: "user".into(),
                    content: t.content.clone(),
                }),
                Role::Assistant => Some(AskTurn {
                    role: "assistant".into(),
                    content: t.content.clone(),
                }),
                Role::Error => None,
            })
            .collect();
        self.turns.push(Turn {
            role: Role::User,
            content: prompt.clone(),
            meta: None,
        });
        self.loading = true;
        self.focus_next = true;
        cx.notify();

        let work = shell::run(async move { shell.ask(&prompt, &history).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                view.loading = false;
                match result {
                    Ok(reply) => {
                        let meta = if reply.tools.is_empty() {
                            format!("{} · {}", reply.model, reply.source)
                        } else {
                            format!(
                                "{} · tools: {}",
                                reply.model,
                                reply.tools.join(", ")
                            )
                        };
                        view.turns.push(Turn {
                            role: Role::Assistant,
                            content: reply.text,
                            meta: Some(meta),
                        });
                    }
                    Err(error) => {
                        view.turns.push(Turn {
                            role: Role::Error,
                            content: error.to_string(),
                            meta: None,
                        });
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn ensure_input(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<InputState> {
        let input = if let Some((input, _)) = &self.input {
            input.clone()
        } else {
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("Ask Sjel anything about your calendar, projects, trips or capabilities…")
            });
            let subscription =
                cx.subscribe_in(&input, window, |view, input, event, window, cx| {
                    if let InputEvent::PressEnter { .. } = event {
                        let text = input.read(cx).value().trim().to_owned();
                        if text.is_empty() || view.loading {
                            return;
                        }
                        input.update(cx, |input, cx| input.set_value("", window, cx));
                        view.submit(text, cx);
                    }
                });
            self.input = Some((input.clone(), subscription));
            input
        };
        if self.focus_next {
            self.focus_next = false;
            input.update(cx, |input, cx| input.focus(window, cx));
        }
        input
    }
}

impl Render for Ask {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = Palette::for_appearance(window.appearance());
        let mono = cx.theme().mono_font_family.clone();
        let input = self.ensure_input(window, cx);
        let input_for_send = input.clone();

        let composer = div()
            .flex()
            .items_center()
            .gap_2()
            .p_3()
            .rounded(px(12.))
            .bg(rgba(palette.surface))
            .border_1()
            .border_color(rgba(palette.border))
            .child(div().flex_1().child(Input::new(&input)))
            .child(
                Button::new("ask-send")
                    .primary()
                    .small()
                    .label("Send")
                    .tooltip("Send in session (↩)")
                    .loading(self.loading)
                    .on_click(cx.listener(move |view, _, window, cx| {
                        let text = input_for_send.read(cx).value().trim().to_owned();
                        if text.is_empty() || view.loading {
                            return;
                        }
                        input_for_send
                            .update(cx, |state, cx| state.set_value("", window, cx));
                        view.submit(text, cx);
                    })),
            )
            .when(!self.turns.is_empty(), |bar| {
                bar.child(
                    Button::new("ask-clear")
                        .ghost()
                        .small()
                        .label("Clear")
                        .disabled(self.loading)
                        .on_click(cx.listener(|view, _, _, cx| view.clear(cx))),
                )
            });

        let starters = section(
            "Suggested prompts",
            list(
                STARTERS
                    .iter()
                    .enumerate()
                    .map(|(idx, &prompt)| {
                        row()
                            .id(("starter", idx))
                            .cursor_pointer()
                            .hover(move |style| style.bg(rgba(palette.raised)))
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.submit(prompt.to_owned(), cx);
                            }))
                            .child(
                                div()
                                    .flex_1()
                                    .text_sm()
                                    .text_color(rgb(palette.text))
                                    .child(prompt),
                            )
                            .child(
                                div()
                                    .font_family(mono.clone())
                                    .text_xs()
                                    .text_color(rgb(palette.muted))
                                    .child("↩"),
                            )
                            .into_any_element()
                    })
                    .collect(),
                palette,
            ),
            palette,
        );

        let transcript: AnyElement = if self.turns.is_empty() {
            note(
                "Ask a question above or press ⌥Space from any app. Follow-up questions stay in this native session.",
                palette.muted,
            )
        } else {
            let mut rows: Vec<AnyElement> = self
                .turns
                .iter()
                .map(|turn| {
                    let (label, label_color) = match turn.role {
                        Role::User => ("You", palette.accent),
                        Role::Assistant => ("Sjel", palette.green),
                        Role::Error => ("Error", palette.red),
                    };
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .px_4()
                        .py_3()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .child(
                                    div()
                                        .text_xs()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(rgb(label_color))
                                        .child(label),
                                )
                                .children(turn.meta.as_ref().map(|meta| {
                                    div()
                                        .font_family(mono.clone())
                                        .text_xs()
                                        .text_color(rgb(palette.muted))
                                        .child(SharedString::from(meta.clone()))
                                })),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(rgb(if turn.role == Role::Error {
                                    palette.red
                                } else {
                                    palette.text
                                }))
                                .child(SharedString::from(turn.content.clone())),
                        )
                        .into_any_element()
                })
                .collect();
            if self.loading {
                rows.push(
                    row()
                        .text_sm()
                        .text_color(rgb(palette.muted))
                        .child("Reading capabilities and thinking…")
                        .into_any_element(),
                );
            }
            section("Session", list(rows, palette), palette).into_any_element()
        };

        div()
            .flex()
            .flex_col()
            .gap_6()
            .child(composer)
            .child(transcript)
            .when(self.turns.is_empty(), |page| page.child(starters))
    }
}
