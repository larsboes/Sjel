//! Individual mail row rendering and mobile drawer actions.
use crate::ely::buttons::{Button, ButtonVariant};
use crate::ely::theme::ControlSize;
use gpui_kit::{div, prelude::*, px, rgb, rgba, AnyElement, Context, FontWeight, SharedString};
use sjel_gpui_core::mail::{category_label, MailAction, MailProposal, CATEGORIES};

use crate::theme::Palette;

use super::Mail;

impl Mail {
    pub(super) fn row(
        &self,
        i: usize,
        m: MailProposal,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let adapted = super::adapter::AdaptedMailProposal::from_proposal(
            &m,
            &self.selected,
            self.cursor.as_deref(),
            self.opened_thread.as_deref(),
        );
        let chosen = adapted.cursor;
        let picked = adapted.selected;
        let is_mobile = self.density.is_mobile();
        let is_open = adapted.opened;
        let meta = [
            m.from_addr
                .clone()
                .unwrap_or_else(|| "Unknown sender".into()),
            m.internal_date
                .as_deref()
                .and_then(|d| d.get(..10))
                .unwrap_or_default()
                .to_owned(),
        ]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
        let badges = adapted.badges();
        let model_line = adapted.model_summary.clone().map(|summary| {
            (
                summary.to_string(),
                adapted
                    .model_proposed_stream
                    .as_ref()
                    .map(|s| s.to_string()),
            )
        });
        let accept_id = m.id.clone();
        let attention = adapted.sync_needs_attention.then(|| m.id.clone());
        let select = m.id.clone();
        let toggle = m.id.clone();
        let id_for_triage = m.id.clone();
        let pad_y = self.density.pad_y();
        let pad_x = self.density.pad_x();

        let row_item = div()
            .id(("mail-row", i))
            .flex()
            .flex_col()
            .px(pad_x)
            .py(pad_y)
            .rounded(px(6.))
            .when(chosen, |el| el.bg(rgba(palette.raised)))
            .hover(move |style| style.bg(rgba(palette.raised)))
            .child(
                div()
                    .id(("mail-row-head", i))
                    .flex()
                    .items_center()
                    .gap_2()
                    .when(is_mobile, |el| el.min_h(self.density.min_touch_target()))
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.cursor = Some(select.clone());
                        if is_mobile {
                            if view.opened_thread.as_deref() == Some(select.as_str()) {
                                view.opened_thread = None;
                            } else {
                                view.opened_thread = Some(select.clone());
                            }
                        }
                        window.focus(&view.focus, cx);
                        cx.notify();
                    }))
                    .child(
                        div()
                            .id(("mail-pick", i))
                            .w(px(18.))
                            .text_sm()
                            .text_color(rgb(if picked { palette.text } else { palette.muted }))
                            .cursor_pointer()
                            .on_click(cx.listener(move |view, _, _, cx| {
                                cx.stop_propagation();
                                view.toggle(toggle.clone(), cx);
                            }))
                            .child(if picked { "☑" } else { "☐" }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .truncate()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(
                                        m.subject.clone().unwrap_or_else(|| "(No subject)".into()),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .items_center()
                                    .gap_1p5()
                                    .child(
                                        div()
                                            .truncate()
                                            .text_xs()
                                            .text_color(rgb(palette.muted))
                                            .child(meta),
                                    )
                                    .children(badges),
                            )
                            .when_some(model_line, |el, (text, proposed)| {
                                el.child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .truncate()
                                                .text_xs()
                                                .text_color(rgb(palette.muted))
                                                .child(text),
                                        )
                                        .when_some(proposed, |el, stream| {
                                            el.child(
                                                Button::new(
                                                    SharedString::from(format!(
                                                        "mail-model-accept-{i}"
                                                    )),
                                                    format!("Accept {}", category_label(&stream)),
                                                )
                                                .size(ControlSize::Sm)
                                                .variant(ButtonVariant::Ghost)
                                                .on_click(cx.listener(move |view, _, _, cx| {
                                                    cx.stop_propagation();
                                                    view.direct_action(
                                                        accept_id.clone(),
                                                        MailAction::Categorize(stream.clone()),
                                                        cx,
                                                    );
                                                })),
                                            )
                                        }),
                                )
                            })
                            .when_some(attention, |el, id| {
                                let retry = id.clone();
                                el.child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .child(
                                            div().text_xs().text_color(rgb(palette.muted)).child(
                                                "Automatic retries stopped after five attempts.",
                                            ),
                                        )
                                        .child(
                                            Button::new(
                                                SharedString::from(format!("mail-job-retry-{i}")),
                                                "Retry",
                                            )
                                            .size(ControlSize::Sm)
                                            .variant(ButtonVariant::Ghost)
                                            .on_click(
                                                cx.listener(move |view, _, _, cx| {
                                                    cx.stop_propagation();
                                                    view.gmail_job(retry.clone(), "retry", cx);
                                                }),
                                            ),
                                        )
                                        .child(
                                            Button::new(
                                                SharedString::from(format!("mail-job-cancel-{i}")),
                                                "Cancel action",
                                            )
                                            .size(ControlSize::Sm)
                                            .variant(ButtonVariant::Ghost)
                                            .on_click(
                                                cx.listener(move |view, _, _, cx| {
                                                    cx.stop_propagation();
                                                    view.gmail_job(id.clone(), "cancel", cx);
                                                }),
                                            ),
                                        ),
                                )
                            })
                            .when(!is_mobile && m.snippet.is_some(), |el| {
                                el.child(
                                    div()
                                        .truncate()
                                        .text_xs()
                                        .text_color(rgb(palette.muted))
                                        .child(m.snippet.clone().unwrap_or_default()),
                                )
                            }),
                    )
                    .when(is_mobile, |el| {
                        el.child(
                            div()
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child(if is_open { "▾" } else { "▸" }),
                        )
                    }),
            )
            .when(is_mobile && is_open, |el| {
                let id1 = id_for_triage.clone();
                let id2 = id_for_triage.clone();
                let id3 = id_for_triage.clone();
                let id4 = id_for_triage.clone();
                let id_cat = id_for_triage.clone();
                el.child(
                    div()
                        .pt_2()
                        .pb_1()
                        .pl_6()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .border_t_1()
                        .border_color(rgba(crate::theme::tint(palette.border, 0x40)))
                        .when_some(m.snippet.clone(), |drawer, snip| {
                            drawer.child(div().text_xs().text_color(rgb(palette.text)).child(snip))
                        })
                        .child(
                            div()
                                .flex()
                                .flex_wrap()
                                .gap_1()
                                .child(
                                    Button::new(
                                        SharedString::from(format!("mail-row-d-{i}")),
                                        "Dismiss",
                                    )
                                    .size(ControlSize::Sm)
                                    .variant(ButtonVariant::Ghost)
                                    .on_click(cx.listener(
                                        move |view, _, _, cx| {
                                            view.direct_action(
                                                id1.clone(),
                                                MailAction::Dismiss,
                                                cx,
                                            );
                                        },
                                    )),
                                )
                                .child(
                                    Button::new(
                                        SharedString::from(format!("mail-row-w-{i}")),
                                        "Waiting",
                                    )
                                    .size(ControlSize::Sm)
                                    .variant(ButtonVariant::Ghost)
                                    .on_click(cx.listener(
                                        move |view, _, _, cx| {
                                            view.direct_action(
                                                id2.clone(),
                                                MailAction::Waiting,
                                                cx,
                                            );
                                        },
                                    )),
                                )
                                .child(
                                    Button::new(
                                        SharedString::from(format!("mail-row-e-{i}")),
                                        "Archive",
                                    )
                                    .size(ControlSize::Sm)
                                    .variant(ButtonVariant::Ghost)
                                    .on_click(cx.listener(
                                        move |view, _, _, cx| {
                                            view.direct_action(
                                                id3.clone(),
                                                MailAction::Archive,
                                                cx,
                                            );
                                        },
                                    )),
                                )
                                .child(
                                    Button::new(
                                        SharedString::from(format!("mail-row-t-{i}")),
                                        "Trash",
                                    )
                                    .size(ControlSize::Sm)
                                    .variant(ButtonVariant::Ghost)
                                    .on_click(cx.listener(
                                        move |view, _, _, cx| {
                                            view.direct_action(id4.clone(), MailAction::Trash, cx);
                                        },
                                    )),
                                )
                                .children(CATEGORIES.iter().take(3).map(|(key, label)| {
                                    let this_id = id_cat.clone();
                                    Button::new(
                                        SharedString::from(format!("mail-row-cat-{key}-{i}")),
                                        format!("→ {label}"),
                                    )
                                    .size(ControlSize::Sm)
                                    .variant(ButtonVariant::Ghost)
                                    .on_click(cx.listener(
                                        move |view, _, _, cx| {
                                            view.direct_action(
                                                this_id.clone(),
                                                MailAction::Categorize((*key).to_owned()),
                                                cx,
                                            );
                                        },
                                    ))
                                })),
                        ),
                )
            });

        row_item.into_any_element()
    }
}
