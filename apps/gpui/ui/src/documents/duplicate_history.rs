//! Duplicate decision history and reopening.

use gpui::{div, prelude::*, rgb, AnyElement, Context, FontWeight};
use sjel_gpui_core::shell::{
    self, DuplicateDecisionAction, DuplicateHistoryEntry, DuplicateReopenRequest, ShellError,
};

use crate::data::Remote;
use crate::ely::buttons::{Button, ButtonVariant};
use crate::ely::theme::ControlSize;
use crate::theme::Palette;

use super::Documents;

const PAGE: usize = 50;

#[derive(Default)]
pub(super) struct History {
    pub(super) open: bool,
    offset: usize,
    entries: Remote<Vec<DuplicateHistoryEntry>>,
    pending: Option<DuplicateReopenRequest>,
    busy: bool,
    error: Option<String>,
}

impl Documents {
    fn load_duplicate_history(&mut self, offset: usize, cx: &mut Context<Self>) {
        let Ok(shell) = self.repo.shell() else { return };
        self.history.open = true;
        self.history.offset = offset;
        self.history.pending = None;
        self.history.entries = Remote::new_loading();
        let work = shell::run(async move { shell.duplicate_history(offset).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                if view.history.open && view.history.offset == offset {
                    view.history.entries.settle(result);
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn reopen_history_decision(&mut self, cx: &mut Context<Self>) {
        if self.history.busy {
            return;
        }
        let (Some(request), Ok(shell)) = (self.history.pending.clone(), self.repo.shell()) else {
            return;
        };
        self.history.busy = true;
        self.history.error = None;
        let work = shell::run(async move { shell.reopen_duplicate(&request).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                view.history.busy = false;
                match result {
                    Ok(restored) => {
                        if let Some(rows) = &mut view.rows.value {
                            for document in restored {
                                if let Some(row) = rows.iter_mut().find(|row| row.id == document.id)
                                {
                                    *row = document;
                                }
                            }
                        }
                        view.order_rev = view.order_rev.wrapping_add(1);
                        view.load_duplicate_history(view.history.offset, cx);
                    }
                    Err(error) => view.history.error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn history_button(&self, cx: &mut Context<Self>) -> Button {
        Button::new("duplicate-history", "Decision history")
            .size(ControlSize::Sm)
            .on_click(cx.listener(|view, _, _, cx| view.load_duplicate_history(0, cx)))
    }

    pub(super) fn history_workspace(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let state = &self.history;
        let rows = state.entries.value.as_deref().unwrap_or_default();
        let mut list = div().flex().flex_col().gap_3();
        for (index, row) in rows.iter().enumerate() {
            let title = match row.decision.action {
                DuplicateDecisionAction::ConfirmGroup { .. } => "Merge",
                DuplicateDecisionAction::Dismiss => "Kept separate",
                DuplicateDecisionAction::Split { .. } => "Split decision",
            };
            let mut card = div()
                .flex()
                .flex_col()
                .gap_1()
                .pb_3()
                .border_b_1()
                .border_color(gpui::rgba(palette.surface))
                .child(div().font_weight(FontWeight::SEMIBOLD).child(format!(
                    "{title} · {}{}",
                    row.decided_at.get(..10).unwrap_or(&row.decided_at),
                    if row.reopened_at.is_some() {
                        " · Reopened"
                    } else {
                        ""
                    },
                )));
            for member in &row.members {
                let role = match &row.decision.action {
                    DuplicateDecisionAction::ConfirmGroup { survivor_id }
                        if survivor_id == &member.id =>
                    {
                        "Kept"
                    }
                    DuplicateDecisionAction::ConfirmGroup { .. } if row.reopened_at.is_none() => {
                        "Archived by merge"
                    }
                    _ => "Member",
                };
                card = card.child(format!("{role} · {}", member.filename));
                for source in row
                    .locations
                    .iter()
                    .filter(|source| source.document_id == member.id)
                {
                    card = card.child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(source.path.clone()),
                    );
                }
            }
            if row.can_reopen {
                let request = DuplicateReopenRequest {
                    candidate_key: row.decision.candidate_key.clone(),
                    decided_at: row.decided_at.clone(),
                };
                let confirming = state.pending.as_ref().is_some_and(|pending| {
                    pending.decided_at == row.decided_at
                        && pending.candidate_key == row.decision.candidate_key
                });
                card = card.child(
                    Button::new(
                        ("reopen-duplicate", index),
                        if confirming {
                            "Confirm reopening"
                        } else {
                            "Reopen decision"
                        },
                    )
                    .size(ControlSize::Sm)
                    .disabled(state.busy)
                    .on_click(cx.listener(move |view, _, _, cx| {
                        if confirming {
                            view.reopen_history_decision(cx);
                        } else {
                            view.history.pending = Some(request.clone());
                            cx.notify();
                        }
                    })),
                );
            }
            list = list.child(card);
        }
        if rows.is_empty() {
            list = list.child(if state.entries.loading {
                "Loading decision history…"
            } else {
                "No decisions on this page."
            });
        }
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new("duplicate-history-back", "Back to candidates")
                            .size(ControlSize::Sm)
                            .disabled(state.busy)
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.history.open = false;
                                view.load_duplicate_candidates(cx);
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("duplicate-history-prev", "Previous")
                            .size(ControlSize::Sm)
                            .disabled(state.offset == 0 || state.busy)
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.load_duplicate_history(
                                    view.history.offset.saturating_sub(PAGE),
                                    cx,
                                );
                            })),
                    )
                    .child(
                        Button::new("duplicate-history-next", "Next")
                            .size(ControlSize::Sm)
                            .disabled(rows.len() < PAGE || state.busy)
                            .variant(ButtonVariant::Ghost)
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.load_duplicate_history(view.history.offset + PAGE, cx)
                            })),
                    ),
            )
            .children(
                state
                    .entries
                    .error
                    .as_ref()
                    .map(|error| div().text_color(rgb(palette.red)).child(error.to_string())),
            )
            .children(
                state
                    .error
                    .as_ref()
                    .map(|error| div().text_color(rgb(palette.red)).child(error.clone())),
            )
            .child(
                div()
                    .id("duplicate-decisions")
                    .min_h_0()
                    .flex_1()
                    .overflow_y_scroll()
                    .child(list),
            )
            .into_any_element()
    }
}
