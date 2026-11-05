//! Mail-detected event proposal review panel and actions.
use chrono::{Duration, Local};
use gpui::{div, prelude::*, px, rgb, rgba, AnyElement, Context, FontWeight, SharedString};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Disableable, Sizable};
use sjel_gpui_core::shell::{self, ShellError};

use crate::Palette;

use super::page::CalendarPage;

impl CalendarPage {
    pub(super) fn load_proposals(&mut self, cx: &mut Context<Self>, force: bool) {
        if self.proposals.loading {
            return;
        }
        let from = Local::now().date_naive();
        let to = from + Duration::days(90);
        let window = (
            from.format("%Y-%m-%d").to_string(),
            to.format("%Y-%m-%d").to_string(),
        );
        if !force && self.proposals_window.as_ref() == Some(&window) {
            return;
        }
        let shell = match self.repo.shell() {
            Ok(shell) => shell,
            Err(error) => {
                self.proposals.settle(Err(error));
                cx.notify();
                return;
            }
        };
        self.proposals_window = Some(window.clone());
        self.proposals.loading = true;
        let (from, to) = window.clone();
        let work = shell::run(async move { shell.calendar_proposals(&from, &to).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                if view.proposals_window.as_ref() == Some(&window) {
                    view.proposals.settle(result);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(super) fn review_proposal(&mut self, id: String, remove: bool, cx: &mut Context<Self>) {
        if remove && self.proposal_confirm_remove.as_deref() != Some(id.as_str()) {
            self.proposal_confirm_remove = Some(id);
            cx.notify();
            return;
        }
        let Ok(shell) = self.repo.shell() else { return };
        self.proposal_action = Some(id.clone());
        self.proposal_confirm_remove = None;
        self.write_error = None;
        let entry_id = id.clone();
        let work = shell::run(async move {
            if remove {
                shell.delete_calendar_entry(&entry_id).await.map(|()| None)
            } else {
                shell
                    .update_calendar_entry(
                        &entry_id,
                        serde_json::json!({ "kind": "event", "commitment": "planned" }),
                    )
                    .await
                    .map(Some)
            }
        });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                view.proposal_action = None;
                match result {
                    Ok(_) => {
                        if let Some(proposals) = view.proposals.value.as_mut() {
                            proposals.retain(|entry| entry.id != id);
                        }
                        view.proposals_window = None;
                        view.load_proposals(cx, true);
                        view.refresh(cx);
                    }
                    Err(error) => view.write_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn toggle_proposals(&mut self, cx: &mut Context<Self>) {
        self.reviewing_proposals = !self.reviewing_proposals;
        self.proposal_confirm_remove = None;
        if self.reviewing_proposals {
            self.load_proposals(cx, false);
        }
        cx.notify();
    }

    pub(super) fn proposal_panel(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let proposals = self.proposals.value.as_deref().unwrap_or_default();
        let mut panel = div()
            .flex()
            .flex_col()
            .gap_3()
            .p_4()
            .rounded(px(8.))
            .bg(rgba(palette.surface))
            .border_1()
            .border_color(rgba(palette.border))
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Detected event proposals"),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child("Mail event dates appear here as possible entries. Add a proposal to put it on your Calendar."),
            );

        if let Some(error) = &self.proposals.error {
            panel = panel.child(
                div()
                    .text_sm()
                    .text_color(rgb(palette.red))
                    .child(error.to_string()),
            );
        }
        if self.proposals.loading && self.proposals.value.is_none() {
            return panel
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(palette.muted))
                        .child("Loading proposals…"),
                )
                .into_any_element();
        }
        if proposals.is_empty() && self.proposals.error.is_none() {
            return panel
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(palette.muted))
                        .child("Nothing pending in the next 90 days."),
                )
                .into_any_element();
        }

        for entry in proposals {
            let id = entry.id.clone();
            let remove_id = id.clone();
            let confirming = self.proposal_confirm_remove.as_deref() == Some(id.as_str());
            let busy = self.proposal_action.as_deref() == Some(id.as_str());
            let when = if entry.all_day {
                format!(
                    "{} · all day",
                    entry.starts_at.get(..10).unwrap_or(&entry.starts_at)
                )
            } else {
                format!(
                    "{}–{}",
                    entry.starts_at.get(..16).unwrap_or(&entry.starts_at),
                    entry.ends_at.get(11..16).unwrap_or("")
                )
            };
            panel =
                panel.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .py_3()
                        .border_t_1()
                        .border_color(rgba(palette.border))
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .child(entry.title.clone()),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child(format!(
                                    "{}{} · {}",
                                    when,
                                    entry
                                        .location
                                        .as_ref()
                                        .map(|place| format!(" · {place}"))
                                        .unwrap_or_default(),
                                    entry.source
                                )),
                        )
                        .child(
                            div()
                                .flex()
                                .gap_2()
                                .child(
                                    Button::new(SharedString::from(format!("proposal-add-{id}")))
                                        .small()
                                        .label(if busy { "Adding…" } else { "Add to Calendar" })
                                        .disabled(self.proposal_action.is_some())
                                        .on_click(cx.listener(move |page, _, _, cx| {
                                            page.review_proposal(id.clone(), false, cx)
                                        })),
                                )
                                .child(
                                    Button::new(SharedString::from(format!(
                                        "proposal-remove-{remove_id}"
                                    )))
                                    .small()
                                    .ghost()
                                    .label(if confirming {
                                        "Confirm remove"
                                    } else {
                                        "Remove"
                                    })
                                    .disabled(self.proposal_action.is_some())
                                    .on_click(cx.listener(move |page, _, _, cx| {
                                        page.review_proposal(remove_id.clone(), true, cx)
                                    })),
                                ),
                        ),
                );
        }
        panel.into_any_element()
    }
}
