//! Progress controls for the operator's large-PDF queue.

use gpui::{div, prelude::*, rgb, Context, Task};
use sjel_gpui_core::document_queue::SplitQueue;

use crate::ely::buttons::{Button, ButtonVariant};
use crate::ely::theme::ControlSize;
use crate::theme::Palette;

use super::{compare_documents, DocSort, Documents, Filter, SortOrder};

#[derive(Default)]
pub(super) struct Queue {
    pub(super) state: SplitQueue,
    pub(super) show_deferred: bool,
    error: Option<String>,
    save: Option<Task<()>>,
}

impl Queue {
    pub(super) fn load() -> Self {
        #[cfg(test)]
        let stored: std::io::Result<SplitQueue> = Ok(SplitQueue::default());
        #[cfg(not(test))]
        let stored = SplitQueue::load();
        match stored {
            Ok(state) => Self {
                state,
                ..Self::default()
            },
            Err(error) => Self {
                error: Some(format!("Could not read split queue: {error}")),
                ..Self::default()
            },
        }
    }
}

impl Documents {
    pub(super) fn save_split_queue(&mut self, cx: &mut Context<Self>) {
        let state = self.split_queue.state.clone();
        let previous = self.split_queue.save.take();
        let executor = cx.background_executor().clone();
        self.split_queue.save = Some(cx.spawn(async move |this, cx| {
            if let Some(previous) = previous {
                previous.await;
            }
            let result = executor.spawn(async move { state.save() }).await;
            let _ = this.update(cx, |view, cx| {
                view.split_queue.error = result
                    .err()
                    .map(|error| format!("Could not save split queue: {error}"));
                cx.notify();
            });
        }));
    }

    pub(super) fn advance_split_queue(&mut self, current: &str, cx: &mut Context<Self>) {
        let rows = self.rows.value.as_deref().unwrap_or_default();
        let current_row = rows.iter().find(|row| row.id == current);
        let compare = |left: &_, right: &_| {
            compare_documents(left, right, DocSort::Pages, SortOrder::Descending)
                .then_with(|| left.id.cmp(&right.id))
        };
        let eligible = || {
            rows.iter().filter(|row| {
                row.id != current
                    && Filter::BigDocuments.matches(row)
                    && !self.split_queue.state.deferred.contains(&row.id)
                    && (self.search_applied
                        || self.search_query.is_empty()
                        || super::document_matches(row, &self.search_query))
            })
        };
        let next = eligible()
            .filter(|row| current_row.is_none_or(|current_row| compare(row, current_row).is_gt()))
            .min_by(|left, right| compare(left, right))
            .or_else(|| eligible().min_by(|left, right| compare(left, right)))
            .map(|row| row.id.clone());
        self.split_queue.show_deferred = false;
        if let Some(id) = next {
            self.open(id, cx);
        } else {
            self.close(cx);
        }
        self.order_rev = self.order_rev.wrapping_add(1);
        cx.notify();
    }

    pub(super) fn queue_controls(&self, palette: Palette, cx: &mut Context<Self>) -> gpui::Div {
        let counts = self.rows.value.as_deref().unwrap_or_default().iter().fold(
            (0, 0),
            |(waiting, deferred), row| {
                let waiting = waiting
                    + usize::from(super::split_picker::can_split(row) && row.page_count == 0);
                let deferred = deferred
                    + usize::from(
                        Filter::BigDocuments.matches(row)
                            && self.split_queue.state.deferred.contains(&row.id),
                    );
                (waiting, deferred)
            },
        );
        let resume = self.split_queue.state.resume.as_ref().and_then(|id| {
            self.rows
                .value
                .as_ref()?
                .iter()
                .find(|row| &row.id == id && Filter::BigDocuments.matches(row))
        });
        let mut bar = div()
            .flex()
            .items_center()
            .flex_wrap()
            .gap_2()
            .text_sm()
            .child(
                div()
                    .text_color(rgb(palette.muted))
                    .child(format!("{} PDFs awaiting page analysis", counts.0)),
            );
        if counts.1 > 0 || self.split_queue.show_deferred {
            bar = bar.child(
                Button::new(
                    "documents-deferred",
                    if self.split_queue.show_deferred {
                        "Back to queue".into()
                    } else {
                        format!("Deferred ({})", counts.1)
                    },
                )
                .size(ControlSize::Sm)
                .variant(ButtonVariant::Ghost)
                .on_click(cx.listener(|view, _, _, cx| {
                    view.split_queue.show_deferred = !view.split_queue.show_deferred;
                    view.page_offset = 0;
                    view.order_rev = view.order_rev.wrapping_add(1);
                    cx.notify();
                })),
            );
        }
        if let Some(row) = resume {
            let id = row.id.clone();
            bar = bar.child(
                Button::new("documents-resume-split", "Resume last bundle")
                    .size(ControlSize::Sm)
                    .on_click(cx.listener(move |view, _, _, cx| {
                        if view.selected.as_ref() == Some(&id) {
                            view.begin_split_picker(cx);
                        } else {
                            view.open(id.clone(), cx);
                        }
                    })),
            );
        }
        bar.children(
            self.split_queue
                .error
                .as_ref()
                .map(|error| div().text_color(rgb(palette.red)).child(error.clone())),
        )
    }
}
