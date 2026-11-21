//! The phone's agent transcript, prompts and approval controls.

use std::collections::BTreeSet;

use gpui::{
    div, prelude::*, rgb, rgba, Context, Div, Entity, Focusable, FontWeight, SharedString,
    Subscription, Task, Window,
};
use sjel_gpui_core::agent::{tool_summary, Pending, Row};

use super::{attach, decide, feed, new_session, refresh, stop, submit, Snapshot};
use crate::{
    doc_view::render_markdown,
    ely::{
        buttons::{Button, ButtonVariant},
        feedback::InlineMessage,
        forms::{Input, InputEvent, TextInput},
        primitives::Severity,
        theme::ControlSize,
    },
    theme::{Palette, ThemeContext},
};

pub struct PhoneAgent {
    snapshot: Snapshot,
    prompt: Entity<TextInput>,
    expanded: BTreeSet<usize>,
    _input: [Subscription; 3],
    _updates: Task<()>,
}

impl PhoneAgent {
    pub fn new(window: &mut Window, keyboard: fn(bool), cx: &mut Context<Self>) -> Self {
        let mut source = feed().tx.subscribe();
        let snapshot = source.borrow_and_update().clone();
        let prompt = cx.new(|cx| TextInput::new(window, cx).placeholder("Ask or guide the agent"));
        let send = cx.subscribe_in(&prompt, window, |view, _, event, _, cx| {
            if matches!(event, InputEvent::Submit) {
                submit(view.prompt.read(cx).text().to_owned());
            }
        });
        let focus = prompt.read(cx).focus_handle(cx);
        let on_focus = cx.on_focus(&focus, window, move |_, _, _| keyboard(true));
        let on_blur = cx.on_blur(&focus, window, move |_, _, _| keyboard(false));
        let updates = cx.spawn(async move |view, cx| {
            while source.changed().await.is_ok() {
                let snapshot = source.borrow_and_update().clone();
                if view
                    .update(cx, |view, cx| {
                        if snapshot.sent != view.snapshot.sent
                            && snapshot.accepted.as_deref() == Some(view.prompt.read(cx).text())
                        {
                            view.prompt.update(cx, |prompt, cx| prompt.set_text("", cx));
                        }
                        if snapshot.rows.len() < view.snapshot.rows.len() {
                            view.expanded.clear();
                        }
                        view.snapshot = snapshot;
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            snapshot,
            prompt,
            expanded: BTreeSet::new(),
            _input: [send, on_focus, on_blur],
            _updates: updates,
        }
    }

    fn approval(&self, pending: &Pending, theme: ThemeContext) -> Div {
        let blocked = pending.is_block();
        let palette = theme.palette;
        let mut card = column()
            .p_3()
            .rounded_md()
            .border_1()
            .border_color(rgba(palette.border))
            .child(caption(
                palette,
                if blocked {
                    "Blocked"
                } else {
                    "Approval required"
                },
            ))
            .child(title(pending.name.clone()))
            .child(div().child(pending.preview.clone()))
            .when_some(pending.diff.as_ref(), |card, diff| {
                card.child(render_markdown(
                    &format!("```diff\n{diff}\n```"),
                    palette,
                    theme.density,
                ))
            });
        if blocked {
            card = card.child(caption(
                palette,
                pending
                    .rule
                    .clone()
                    .unwrap_or_else(|| "This call is blocked by policy.".into()),
            ));
        }
        let choice = |id, label, allow, for_session| {
            let approval = pending.approval.clone();
            control(id, label, allow)
                .disabled(self.snapshot.busy)
                .on_click(move |_, _, _| decide(approval.clone(), allow, for_session))
        };
        card.child(
            div()
                .flex()
                .flex_wrap()
                .gap_1()
                .when(!blocked, |buttons| {
                    buttons
                        .child(choice("approve-once", "Allow once", true, false))
                        .child(choice(
                            "approve-session",
                            "Allow for this session",
                            true,
                            true,
                        ))
                })
                .child(choice("deny-approval", "Deny", false, false)),
        )
    }

    fn row(&self, index: usize, row: &Row, theme: ThemeContext, cx: &Context<Self>) -> Div {
        let palette = theme.palette;
        match row {
            Row::Prompt(prompt) => column().child(title("You")).child(prompt.clone()),
            Row::Answer(answer) => div().child(render_markdown(answer, palette, theme.density)),
            Row::Tool {
                name,
                arguments,
                result,
                ..
            } => column()
                .child(
                    control(
                        format!("agent-tool-{index}"),
                        tool_summary(name, arguments),
                        false,
                    )
                    .on_click(cx.listener(move |view, _, _, cx| {
                        if !view.expanded.remove(&index) {
                            view.expanded.insert(index);
                        }
                        cx.notify();
                    })),
                )
                .when(self.expanded.contains(&index), |row| {
                    row.child(div().text_sm().child(arguments.clone())).child(
                        div()
                            .text_sm()
                            .child(result.clone().unwrap_or_else(|| "Running…".into())),
                    )
                }),
            Row::Decided {
                name,
                allowed,
                for_session,
            } => {
                let decision = if *allowed { "allowed" } else { "denied" };
                let scope = if *for_session {
                    " for this session"
                } else {
                    ""
                };
                caption(palette, format!("{name}: {decision}{scope}"))
            }
            Row::Notice(message) => caption(palette, message.clone()),
        }
    }
}

impl Render for PhoneAgent {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = ThemeContext::of(window, cx);
        let palette = theme.palette;
        let snapshot = &self.snapshot;
        let (severity, status) = if let Some(error) = &snapshot.error {
            (Severity::Danger, error.clone())
        } else if !snapshot.connected {
            (
                Severity::Warning,
                "Pair this phone with the Mac to use Agent.".into(),
            )
        } else if snapshot.busy {
            (Severity::Info, "Sending…".into())
        } else {
            (Severity::Info, snapshot.status.clone())
        };
        column()
            .size_full()
            .p_3()
            .bg(rgba(palette.window_tint))
            .text_color(rgb(palette.text))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(title("Agent").text_xl())
                    .child(
                        control("agent-refresh", "Refresh sessions", false)
                            .on_click(|_, _, _| refresh()),
                    ),
            )
            .when(!status.is_empty(), |col| {
                col.child(InlineMessage::new(severity, status))
            })
            .child(
                div()
                    .id("phone-agent-transcript")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(
                        column()
                            .pb_2()
                            .child(
                                div().flex().flex_wrap().gap_1().children(
                                    [
                                        ("project", "New project session"),
                                        ("sjel", "New Sjel session"),
                                        ("account", "New account session"),
                                    ]
                                    .map(|(tier, label)| {
                                        control(tier, label, false)
                                            .disabled(!snapshot.connected)
                                            .on_click(cx.listener(move |view, _, _, cx| {
                                                view.expanded.clear();
                                                new_session(tier);
                                                cx.notify();
                                            }))
                                    }),
                                ),
                            )
                            .when_some(snapshot.pending.as_ref(), |view, pending| {
                                view.child(self.approval(pending, theme))
                            })
                            .children(
                                snapshot
                                    .rows
                                    .iter()
                                    .enumerate()
                                    .map(|(index, row)| self.row(index, row, theme, cx)),
                            )
                            .children(snapshot.sessions.iter().cloned().map(|session| {
                                let id = SharedString::from(session.id.clone());
                                control(id, session.label(), false).on_click(cx.listener(
                                    move |view, _, _, cx| {
                                        view.expanded.clear();
                                        attach(session.clone());
                                        cx.notify();
                                    },
                                ))
                            })),
                    ),
            )
            .when(snapshot.running, |view| {
                view.child(control("agent-stop", "Stop run", false).on_click(|_, _, _| stop()))
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().flex_1().min_w_0().child(Input::new(&self.prompt)))
                    .child(
                        control("agent-send", "Send", true)
                            .disabled(!snapshot.connected || snapshot.busy)
                            .on_click(cx.listener(|view, _, _, cx| {
                                submit(view.prompt.read(cx).text().to_owned());
                            })),
                    ),
            )
    }
}

fn control(id: impl Into<gpui::ElementId>, label: impl Into<SharedString>, active: bool) -> Button {
    Button::new(id, label)
        .size(ControlSize::Md)
        .variant(if active {
            ButtonVariant::Primary
        } else {
            ButtonVariant::Ghost
        })
}

fn column() -> Div {
    div().flex().flex_col().gap_2()
}

fn title(text: impl Into<SharedString>) -> Div {
    div().font_weight(FontWeight::SEMIBOLD).child(text.into())
}

fn caption(palette: Palette, text: impl Into<SharedString>) -> Div {
    div()
        .text_sm()
        .text_color(rgb(palette.muted))
        .child(text.into())
}
