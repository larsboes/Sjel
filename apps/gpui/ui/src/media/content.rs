//! Local preview evidence and explicit metadata review, separate from ingest operations.

use std::sync::Arc;

use gpui_kit::component::{
    button::{Button, ButtonVariants},
    input::{Input, InputEvent, InputState},
    Disableable, Sizable,
};
use gpui_kit::{
    div, img, prelude::*, px, rgb, AnyElement, Context, Entity, Image, ImageFormat, ObjectFit,
    SharedString, Subscription, Window,
};
use sjel_gpui_core::{
    media::{self, ContentAction, ContentDecision, ContentDetail, ContentPage, ContentState},
    shell::{self, ShellError},
};

#[cfg(test)]
mod tests;

use super::MediaPage;
use crate::{
    data::Remote,
    primitives::{card, note},
    theme::Palette,
};

pub(crate) struct ContentReview {
    pub(super) queue: Remote<ContentPage>,
    pub(super) detail: Remote<ContentDetail>,
    pub(super) state: String,
    pub(super) after: i64,
    pub(super) form: Option<ReviewForm>,
    pub(super) previews: Vec<Arc<Image>>,
    pub(super) busy: bool,
    pub(super) message: Option<(bool, String)>,
    generation: u64,
    selection_generation: u64,
}

pub(super) struct ReviewForm {
    id: i64,
    tags: Entity<InputState>,
    collections: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

impl Default for ContentReview {
    fn default() -> Self {
        Self {
            queue: Remote::default(),
            detail: Remote::default(),
            state: "pending".into(),
            after: 0,
            form: None,
            previews: Vec::new(),
            busy: false,
            message: None,
            generation: 0,
            selection_generation: 0,
        }
    }
}

impl MediaPage {
    pub(super) fn load_content(&mut self, cx: &mut Context<Self>) {
        if self.content.busy {
            return;
        }
        let shell = match self.repo.shell() {
            Ok(shell) => shell,
            Err(error) => {
                self.content.queue.settle(Err(error));
                cx.notify();
                return;
            }
        };
        self.content.generation += 1;
        let generation = self.content.generation;
        let state = self.content.state.clone();
        let after = self.content.after;
        self.content.queue.loading = true;
        let work = shell::run(async move { shell.media_content_proposals(&state, after).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                if page.content.generation != generation {
                    return;
                }
                page.content.queue.settle(result);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn select_content(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        if self.content.busy {
            return;
        }
        let shell = match self.repo.shell() {
            Ok(shell) => shell,
            Err(error) => {
                self.content.message = Some((true, error.to_string()));
                cx.notify();
                return;
            }
        };
        let tags =
            cx.new(|cx| InputState::new(window, cx).placeholder("Chosen tags, comma-separated"));
        let collections = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Chosen collections, comma-separated")
        });
        let subscriptions = [&tags, &collections]
            .iter()
            .map(|input| {
                cx.subscribe_in(input, window, |_, _, event, _, cx| {
                    if matches!(event, InputEvent::Change) {
                        cx.notify();
                    }
                })
            })
            .collect();
        self.content.selection_generation += 1;
        let generation = self.content.selection_generation;
        self.content.form = Some(ReviewForm {
            id,
            tags,
            collections,
            _subscriptions: subscriptions,
        });
        self.content.detail = Remote::default();
        self.content.detail.loading = true;
        self.content.previews.clear();
        self.content.message = None;
        let work = shell::run(async move {
            let detail = shell.media_content_detail(id).await?;
            let previews = media::content_previews(&detail).map_err(|_| ShellError::Unreadable)?;
            Ok::<_, ShellError>((detail, previews))
        });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                if page.content.selection_generation != generation
                    || page.content.form.as_ref().map(|f| f.id) != Some(id)
                {
                    return;
                }
                match result {
                    Ok((detail, previews)) => {
                        page.content.previews = previews
                            .into_iter()
                            .map(|bytes| Arc::new(Image::from_bytes(ImageFormat::Png, bytes)))
                            .collect();
                        page.content.detail.settle(Ok(detail));
                    }
                    Err(error) => page.content.detail.settle(Err(error)),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn decide_content(&mut self, action: ContentAction, cx: &mut Context<Self>) {
        if self.content.busy
            || self.content.queue.loading
            || self.content.queue.error.is_some()
            || self.content.detail.loading
            || self.content.detail.error.is_some()
        {
            return;
        }
        let (Some(form), Some(detail)) = (&self.content.form, &self.content.detail.value) else {
            return;
        };
        if form.id != detail.proposal.id || detail.proposal.state != ContentState::Pending {
            return;
        }
        let id = form.id;
        let decision = ContentDecision {
            fingerprint: detail.proposal.fingerprint.clone(),
            action,
            tags: if action == ContentAction::Accept {
                media::content_names(form.tags.read(cx).value().as_str())
            } else {
                Vec::new()
            },
            collections: if action == ContentAction::Accept {
                media::content_names(form.collections.read(cx).value().as_str())
            } else {
                Vec::new()
            },
        };
        if action == ContentAction::Accept
            && decision.tags.is_empty()
            && decision.collections.is_empty()
        {
            return;
        }
        let shell = match self.repo.shell() {
            Ok(shell) => shell,
            Err(error) => {
                self.content.message = Some((true, error.to_string()));
                cx.notify();
                return;
            }
        };
        self.content.busy = true;
        self.content.message = None;
        let work = shell::run(async move { shell.media_content_decide(id, &decision).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                page.content.busy = false;
                match result {
                    Ok(_) => {
                        page.content.message = Some((false, "Decision saved. Original bytes are unchanged.".into()));
                        page.content.form = None;
                        page.content.detail = Remote::default();
                        page.content.previews.clear();
                        page.load_content(cx);
                    }
                    Err(error) => { page.content.message = Some((true, format!("Decision not confirmed: {error}. Refresh and inspect before retrying."))); }
                }
                cx.notify();
            });
        }).detach();
        cx.notify();
    }

    pub(super) fn render_content(
        &mut self,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut panel = div()
            .flex()
            .flex_col()
            .gap_3()
            .child("Content review")
            .child(div().text_sm().text_color(rgb(palette.muted)).child(
                "Local model candidates, not accepted metadata. No file moves or deletion.",
            ));
        let mut filters = div().flex().gap_2();
        for (state, label) in [
            ("pending", "Pending"),
            ("accepted", "Accepted"),
            ("rejected", "Rejected"),
        ] {
            filters = filters.child(
                Button::new(SharedString::from(format!("content-filter-{state}")))
                    .small()
                    .label(label)
                    .when(self.content.state != state, |b| b.ghost())
                    .disabled(self.content.busy)
                    .on_click(cx.listener(move |page, _, _, cx| {
                        page.content.state = state.into();
                        page.content.after = 0;
                        page.content.form = None;
                        page.content.previews.clear();
                        page.load_content(cx);
                    })),
            );
        }
        panel = panel.child(
            filters.child(
                Button::new("content-refresh")
                    .small()
                    .label("Refresh review")
                    .disabled(self.content.busy || self.content.queue.loading)
                    .on_click(cx.listener(|page, _, _, cx| {
                        page.content.form = None;
                        page.content.previews.clear();
                        page.load_content(cx);
                    })),
            ),
        );
        if let Some((error, message)) = &self.content.message {
            panel = panel.child(note(
                message.clone(),
                if *error { palette.red } else { palette.green },
            ));
        }
        if self.content.queue.loading {
            panel = panel.child("Loading review queue…");
        }
        if let Some(error) = &self.content.queue.error {
            panel = panel.child(note(
                format!("Review queue failed: {error}. Refresh to retry; decisions are disabled."),
                palette.red,
            ));
        }
        if let Some(queue) = &self.content.queue.value {
            if queue.items.is_empty() {
                panel = panel.child("No proposals in this view. Stage a native evidence report to add pending items.");
            }
            for proposal in &queue.items {
                let id = proposal.id;
                panel = panel.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_3()
                        .child(div().text_sm().child(format!(
                            "{} · {}… · {}",
                            proposal.kind,
                            &proposal.sha256[..12.min(proposal.sha256.len())],
                            proposal.created_at
                        )))
                        .child(
                            Button::new(SharedString::from(format!("content-inspect-{id}")))
                                .small()
                                .label("Inspect evidence")
                                .disabled(
                                    self.content.busy
                                        || self.content.queue.loading
                                        || self.content.queue.error.is_some(),
                                )
                                .on_click(cx.listener(move |page, _, window, cx| {
                                    page.select_content(id, window, cx)
                                })),
                        ),
                );
            }
            let next = queue.next_after;
            panel = panel.child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("content-first")
                            .small()
                            .label("First page")
                            .disabled(
                                self.content.busy
                                    || self.content.after == 0
                                    || self.content.queue.loading,
                            )
                            .on_click(cx.listener(|page, _, _, cx| {
                                page.content.after = 0;
                                page.content.form = None;
                                page.content.previews.clear();
                                page.load_content(cx);
                            })),
                    )
                    .child(
                        Button::new("content-next")
                            .small()
                            .label("Next page")
                            .disabled(
                                self.content.busy || next.is_none() || self.content.queue.loading,
                            )
                            .on_click(cx.listener(move |page, _, _, cx| {
                                if let Some(after) = next {
                                    page.content.after = after;
                                    page.content.form = None;
                                    page.content.previews.clear();
                                    page.load_content(cx);
                                }
                            })),
                    ),
            );
        }
        if self.content.detail.loading && self.content.form.is_some() {
            panel = panel.child("Loading selected evidence…");
        }
        if let Some(error) = &self.content.detail.error {
            panel = panel.child(note(
                format!("Evidence failed: {error}. Inspect the item again to retry."),
                palette.red,
            ));
        }
        if let (Some(form), Some(detail)) = (&self.content.form, &self.content.detail.value) {
            panel = panel.child(div().text_sm().child(format!("Original names: {}", detail.filenames.join(", "))))
                .child(div().text_sm().text_color(rgb(palette.muted)).child(format!("{} · revision {} · {}", detail.proposal.engine, detail.proposal.revision, detail.os)))
                .child("Raw scores are not probabilities of correctness. Video evidence covers only the displayed samples.");
            for (frame_index, frame) in detail.frames.iter().enumerate() {
                let mut frame_view = div().flex().flex_col().gap_2();
                if let Some(image) = self.content.previews.get(frame_index) {
                    frame_view = frame_view.child(
                        img(image.clone())
                            .w(px(256.))
                            .h(px(256.))
                            .object_fit(ObjectFit::Contain),
                    );
                }
                if let (Some(requested), Some(actual)) = (frame.requested_seconds, frame.seconds) {
                    frame_view = frame_view
                        .child(format!("Requested {requested:.3}s · actual {actual:.3}s"));
                }
                for (label_index, label) in frame.labels.iter().enumerate() {
                    let name = label.identifier.clone();
                    frame_view = frame_view.child(
                        Button::new(SharedString::from(format!(
                            "content-label-{frame_index}-{label_index}"
                        )))
                        .small()
                        .ghost()
                        .label(format!(
                            "Add tag: {} · score {:.3}",
                            label.identifier, label.score
                        ))
                        .disabled(
                            self.content.busy || detail.proposal.state != ContentState::Pending,
                        )
                        .on_click(cx.listener(
                            move |page, _, window, cx| {
                                if let Some(form) = &page.content.form {
                                    let mut names =
                                        media::content_names(form.tags.read(cx).value().as_str());
                                    if !names.contains(&name) {
                                        names.push(name.clone());
                                    }
                                    form.tags.update(cx, |input, cx| {
                                        input.set_value(names.join(", "), window, cx)
                                    });
                                }
                            },
                        )),
                    );
                }
                panel = panel.child(frame_view);
            }
            if detail.proposal.state == ContentState::Pending {
                let empty = media::content_names(form.tags.read(cx).value().as_str()).is_empty()
                    && media::content_names(form.collections.read(cx).value().as_str()).is_empty();
                let stale = self.content.queue.loading
                    || self.content.queue.error.is_some()
                    || self.content.detail.error.is_some();
                panel = panel
                    .child("Chosen tags (edit or remove candidate names before saving)")
                    .child(Input::new(&form.tags).small())
                    .child("Chosen collections")
                    .child(Input::new(&form.collections).small())
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                Button::new("content-accept")
                                    .small()
                                    .primary()
                                    .label("Save chosen metadata")
                                    .disabled(self.content.busy || stale || empty)
                                    .on_click(cx.listener(|page, _, _, cx| {
                                        page.decide_content(ContentAction::Accept, cx)
                                    })),
                            )
                            .child(
                                Button::new("content-reject")
                                    .small()
                                    .label("Reject proposal")
                                    .disabled(self.content.busy || stale)
                                    .on_click(cx.listener(|page, _, _, cx| {
                                        page.decide_content(ContentAction::Reject, cx)
                                    })),
                            ),
                    );
            } else {
                panel = panel.child(format!(
                    "Decision: {:?} · Tags: {} · Collections: {}",
                    detail.proposal.state,
                    detail.proposal.tags.join(", "),
                    detail.proposal.collections.join(", ")
                ));
            }
            panel = panel.child(
                Button::new("content-close")
                    .small()
                    .ghost()
                    .label("Leave review")
                    .disabled(self.content.busy)
                    .on_click(cx.listener(|page, _, _, cx| {
                        page.content.form = None;
                        page.content.detail = Remote::default();
                        page.content.previews.clear();
                        cx.notify();
                    })),
            );
        }
        card(panel.into_any_element(), palette)
    }
}
