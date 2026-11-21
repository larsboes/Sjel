//! The phone Mail register and its thread reader.

use std::{rc::Rc, sync::Arc};

use gpui::{
    div, prelude::*, px, rgb, rgba, uniform_list, Context, Div, FontWeight, SharedString, Task,
    Window,
};
use gpui_kit::component::text::TextView;
use sjel_gpui_core::{
    mail::{self, category_label, MailAction, MailProposal, StatusFilter, CATEGORIES},
    shell::FeedContentDetail,
};

use super::{
    apply_action, apply_gmail_job, closable, close, model_summary, open_detail, refresh,
    row_subtitle, subject, subscribe, Snapshot, Update,
};
use crate::{
    doc_view::text_view_style,
    ely::{
        buttons::{Button, ButtonVariant},
        feedback::EmptyState,
        primitives::IconName,
        theme::ControlSize,
    },
    primitives::sub_tabs,
    theme::{Palette, ThemeContext},
};

pub struct PhoneMail {
    snapshot: Snapshot,
    visible: Rc<Vec<MailProposal>>,
    status: StatusFilter,
    category: Option<&'static str>,
    thread: Option<Thread>,
    _updates: Task<()>,
}

struct Thread {
    id: String,
    detail: Option<FeedContentDetail>,
    error: Option<String>,
    confirm: Option<MailAction>,
    more: bool,
}

impl PhoneMail {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let (snapshot, mut updates) = subscribe();
        let task = cx.spawn(async move |view, cx| {
            while let Some(update) = updates.recv().await {
                if view.update(cx, |view, cx| view.update(update, cx)).is_err() {
                    break;
                }
            }
        });
        let mut view = Self {
            snapshot,
            visible: Rc::default(),
            status: StatusFilter::Pending,
            category: None,
            thread: None,
            _updates: task,
        };
        view.refilter();
        view
    }

    fn refilter(&mut self) {
        self.visible = Rc::new(
            mail::in_view(self.snapshot.rows.iter(), self.status, self.category)
                .into_iter()
                .cloned()
                .collect(),
        );
    }

    fn update(&mut self, update: Update, cx: &mut Context<Self>) {
        match update {
            Update::Status(status) => self.snapshot.status = status,
            Update::Rows(rows) => {
                if self
                    .thread
                    .as_ref()
                    .is_some_and(|thread| !rows.iter().any(|row| row.id == thread.id))
                {
                    self.thread = None;
                }
                self.snapshot.rows = rows;
                self.refilter();
            }
            Update::Detail { id, detail } => {
                if let Some(thread) = self.thread.as_mut().filter(|thread| thread.id == id) {
                    thread.detail = Some(*detail);
                    thread.error = None;
                }
            }
            Update::DetailError { id, error } => {
                if let Some(thread) = self.thread.as_mut().filter(|thread| thread.id == id) {
                    thread.error = Some(error);
                }
            }
            Update::ActionDone { succeeded, notice } => {
                self.snapshot
                    .rows
                    .retain(|row| !succeeded.contains(&row.id));
                self.snapshot.status = notice;
                if self
                    .thread
                    .as_ref()
                    .is_some_and(|thread| succeeded.contains(&thread.id))
                {
                    self.thread = None;
                }
                self.refilter();
            }
        }
        cx.notify();
    }

    fn open(&mut self, row: MailProposal, cx: &mut Context<Self>) {
        self.thread = Some(Thread {
            id: row.id.clone(),
            detail: None,
            error: None,
            confirm: None,
            more: false,
        });
        open_detail(row.id.clone(), Some(row));
        cx.notify();
    }

    fn list(&self, palette: Palette, cx: &Context<Self>) -> Div {
        let visible = self.visible.clone();
        column()
            .flex_1()
            .min_h_0()
            .child({
                let on_select = cx.listener(|view, idx: &usize, _, cx| {
                    if let Some(&status) = StatusFilter::ALL.get(*idx) {
                        view.status = status;
                        view.refilter();
                        cx.notify();
                    }
                });
                sub_tabs(
                    "phone-mail-status",
                    StatusFilter::ALL
                        .iter()
                        .map(|status| {
                            let count = self
                                .snapshot
                                .rows
                                .iter()
                                .filter(|row| status.matches(&row.status))
                                .count();
                            (status.label(), Some(count), *status == self.status)
                        })
                        .collect(),
                    move |idx, window, cx| on_select(&idx, window, cx),
                )
            })
            .child(
                div().id("mail-categories").overflow_x_scroll().child(
                    div().flex().gap_1().children(
                        std::iter::once((None, "All"))
                            .chain(CATEGORIES.iter().map(|&(key, title)| (Some(key), title)))
                            .map(|(category, title)| {
                                let count = self
                                    .snapshot
                                    .rows
                                    .iter()
                                    .filter(|row| {
                                        self.status.matches(&row.status)
                                            && category.is_none_or(|key| row.stream == key)
                                    })
                                    .count();
                                div().flex_none().child(
                                    control(
                                        title,
                                        format!("{title} {count}"),
                                        self.category == category,
                                    )
                                    .on_click(cx.listener(
                                        move |view, _, _, cx| {
                                            view.category = category;
                                            view.refilter();
                                            cx.notify();
                                        },
                                    )),
                                )
                            }),
                    ),
                ),
            )
            .when(visible.is_empty(), |body| {
                body.child(EmptyState::new(
                    "phone-mail-empty",
                    IconName::Mail,
                    "No threads match these filters",
                ))
            })
            .child(
                uniform_list(
                    "phone-mail-threads",
                    visible.len(),
                    cx.processor(move |_, range: std::ops::Range<usize>, _, cx| {
                        range
                            .map(|index| {
                                let row = &visible[index];
                                let summary = model_summary(row)
                                    .map(|(label, _)| label)
                                    .or_else(|| row.snippet.clone());
                                let chosen = row.clone();
                                div()
                                    .id(("mail-thread", index))
                                    .h(px(78.))
                                    .border_b_1()
                                    .border_color(rgba(palette.border))
                                    .flex()
                                    .flex_col()
                                    .justify_center()
                                    .gap_1()
                                    .px_2()
                                    .child(
                                        div()
                                            .truncate()
                                            .font_weight(FontWeight::MEDIUM)
                                            .child(subject(row)),
                                    )
                                    .child(caption(palette, row_subtitle(row)).truncate())
                                    .when_some(summary, |card, line| {
                                        card.child(caption(palette, line).truncate())
                                    })
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        view.open(chosen.clone(), cx)
                                    }))
                            })
                            .collect()
                    }),
                )
                .flex_1()
                .min_h_0(),
            )
    }

    fn triage(
        &self,
        label: impl Into<SharedString>,
        action: MailAction,
        cx: &Context<Self>,
    ) -> Button {
        let label = label.into();
        control(label.clone(), label, false).on_click(cx.listener(move |view, _, _, cx| {
            if let Some(thread) = &mut view.thread {
                if action.confirmation(1).is_some() {
                    thread.confirm = Some(action.clone());
                } else {
                    thread.confirm = None;
                    apply_action(thread.id.clone(), action.clone());
                }
                cx.notify();
            }
        }))
    }

    fn reader(
        &self,
        thread: &Thread,
        row: &MailProposal,
        theme: ThemeContext,
        cx: &Context<Self>,
    ) -> Div {
        let palette = theme.palette;
        let subject = subject(row);
        let snippet = row
            .snippet
            .as_deref()
            .filter(|text| !text.trim().is_empty())
            .unwrap_or("No preview text stored for this thread.");
        let body = thread
            .detail
            .as_ref()
            .and_then(|detail| {
                detail
                    .content
                    .as_deref()
                    .filter(|text| !text.trim().is_empty())
                    .or_else(|| {
                        detail
                            .digest
                            .as_ref()
                            .and_then(|digest| digest.text.as_deref())
                            .filter(|text| !text.trim().is_empty())
                    })
            })
            .unwrap_or(snippet);
        let waiting = if row.waiting {
            ("Clear waiting", MailAction::ClearWaiting)
        } else {
            ("Waiting", MailAction::Waiting)
        };
        column()
            .flex_1()
            .min_h_0()
            .child(
                div()
                    .flex()
                    .justify_between()
                    .gap_2()
                    .child(
                        control("mail-back-to-list", "‹ Mail", false).on_click(cx.listener(
                            |view, _, _, cx| {
                                view.thread = None;
                                cx.notify();
                            },
                        )),
                    )
                    .child(
                        control(
                            "mail-more-actions",
                            if thread.more {
                                "Hide actions"
                            } else {
                                "Category & More"
                            },
                            thread.more,
                        )
                        .on_click(cx.listener(|view, _, _, cx| {
                            if let Some(thread) = &mut view.thread {
                                thread.more = !thread.more;
                                cx.notify();
                            }
                        })),
                    ),
            )
            .child(
                div()
                    .id("phone-mail-reader")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(
                        column()
                            .pb_3()
                            .when_some(
                                thread
                                    .confirm
                                    .as_ref()
                                    .and_then(|action| action.confirmation(1)),
                                |body, prompt| {
                                    body.child(
                                        column()
                                            .border_1()
                                            .border_color(rgba(palette.border))
                                            .rounded_md()
                                            .p_2()
                                            .child(
                                                div().font_weight(FontWeight::MEDIUM).child(prompt),
                                            )
                                            .child(
                                                div()
                                                    .flex()
                                                    .gap_2()
                                                    .child(
                                                        control(
                                                            "confirm-mail-action",
                                                            "Confirm",
                                                            true,
                                                        )
                                                        .on_click(cx.listener(|view, _, _, cx| {
                                                            if let Some(thread) = &mut view.thread {
                                                                if let Some(action) =
                                                                    thread.confirm.take()
                                                                {
                                                                    apply_action(
                                                                        thread.id.clone(),
                                                                        action,
                                                                    );
                                                                    cx.notify();
                                                                }
                                                            }
                                                        })),
                                                    )
                                                    .child(
                                                        control(
                                                            "cancel-mail-action",
                                                            "Cancel",
                                                            false,
                                                        )
                                                        .on_click(cx.listener(|view, _, _, cx| {
                                                            if let Some(thread) = &mut view.thread {
                                                                thread.confirm = None;
                                                                cx.notify();
                                                            }
                                                        })),
                                                    ),
                                            ),
                                    )
                                },
                            )
                            .when(thread.more, |body| {
                                body.child(
                                    column()
                                        .border_b_1()
                                        .border_color(rgba(palette.border))
                                        .pb_2()
                                        .child(caption(palette, "Move to stream"))
                                        .child(div().flex().flex_wrap().gap_1().children(
                                            CATEGORIES.iter().map(|&(key, label)| {
                                                self.triage(
                                                    label,
                                                    MailAction::Categorize(key.into()),
                                                    cx,
                                                )
                                                .variant(if row.stream == key {
                                                    ButtonVariant::Primary
                                                } else {
                                                    ButtonVariant::Ghost
                                                })
                                            }),
                                        ))
                                        .child(
                                            div()
                                                .flex()
                                                .gap_2()
                                                .child(self.triage("Trash", MailAction::Trash, cx))
                                                .child(self.triage(
                                                    "Secret (c3)",
                                                    MailAction::SetDataClass("c3".into()),
                                                    cx,
                                                )),
                                        ),
                                )
                            })
                            .when_some(model_summary(row), |body, (line, stream)| {
                                body.child(column().child(caption(palette, line)).when_some(
                                    stream,
                                    |panel, stream| {
                                        panel.child(self.triage(
                                            format!("Accept {}", category_label(&stream)),
                                            MailAction::Categorize(stream),
                                            cx,
                                        ))
                                    },
                                ))
                            })
                            .when(
                                row.gmail_sync_status.as_deref() == Some("attention"),
                                |body| {
                                    body.child(
                                        column()
                                            .child(caption(
                                                palette,
                                                "Automatic Gmail retries stopped.",
                                            ))
                                            .child(
                                                div().flex().gap_2().children(
                                                    [("retry", "Retry"), ("cancel", "Cancel sync")]
                                                        .map(|(decision, label)| {
                                                            let id = thread.id.clone();
                                                            control(decision, label, false)
                                                                .on_click(move |_, _, _| {
                                                                    apply_gmail_job(
                                                                        id.clone(),
                                                                        decision,
                                                                    )
                                                                })
                                                        }),
                                                ),
                                            ),
                                    )
                                },
                            )
                            .child(
                                div()
                                    .text_xl()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(subject),
                            )
                            .child(caption(palette, row_subtitle(row)))
                            .when_some(thread.error.as_ref(), |body, error| {
                                body.child(caption(palette, error.clone()))
                            })
                            .child(
                                TextView::markdown("phone-mail-body", body.to_owned())
                                    .style(text_view_style(palette, theme.density))
                                    .text_color(rgb(palette.text))
                                    .selectable(true)
                                    .image_source(|_| {
                                        gpui::ImageSource::Custom(Arc::new(|_, _| None))
                                    }),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_1()
                    .child(self.triage("Archive", MailAction::Archive, cx))
                    .child(self.triage(waiting.0, waiting.1, cx))
                    .child(self.triage("Dismiss", MailAction::Dismiss, cx)),
            )
    }
}

impl Render for PhoneMail {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = ThemeContext::of(window, cx);
        let palette = theme.palette;
        let reader = self.thread.as_ref().and_then(|thread| {
            self.snapshot
                .rows
                .iter()
                .find(|row| row.id == thread.id)
                .map(|row| (thread, row))
        });
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
                    .child(
                        div()
                            .text_2xl()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Mail"),
                    )
                    .child(
                        control("phone-mail-refresh", "Refresh", false)
                            .on_click(|_, _, _| refresh()),
                    ),
            )
            .when(closable() && reader.is_none(), |screen| {
                screen
                    .child(control("phone-mail-close", "‹ Back", false).on_click(|_, _, _| close()))
            })
            .child(caption(palette, self.snapshot.status.clone()))
            .child(match reader {
                Some((thread, row)) => self.reader(thread, row, theme, cx),
                None => self.list(palette, cx),
            })
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

fn caption(palette: Palette, label: impl Into<SharedString>) -> Div {
    div()
        .text_sm()
        .text_color(rgb(palette.muted))
        .child(label.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn phone_mail_mounts_at_phone_viewport(cx: &mut gpui::TestAppContext) {
        let (view, cx) = crate::test_support::mount_root(
            cx,
            Some(gpui::size(gpui::px(390.), gpui::px(844.))),
            |_, cx| PhoneMail::new(cx),
        );
        view.read_with(cx, |view, _| assert!(view.visible.is_empty()));
    }
}
