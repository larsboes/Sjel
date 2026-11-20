//! Browse collections and edit the membership of the open document.

use gpui::{div, prelude::*, px, rgb, Context, FontWeight};
use sjel_gpui_core::shell::{self, DocumentCollection, DocumentSummary, ShellError};

use crate::data::Remote;
use crate::ely::buttons::{Button, ButtonVariant};
use crate::ely::theme::ControlSize;
use crate::theme::{Density, Palette};

use super::Documents;

#[derive(Default)]
pub(super) struct Collections {
    pub(super) open: bool,
    selected: Option<String>,
    entries: Remote<Vec<DocumentCollection>>,
    members: Remote<Vec<DocumentSummary>>,
    busy: bool,
    error: Option<String>,
}

impl Documents {
    pub(super) fn open_collections(&mut self, cx: &mut Context<Self>) {
        self.collections.open = true;
        let Ok(shell) = self.repo.shell() else {
            self.collections.entries.settle(Err(ShellError::NotSetUp));
            cx.notify();
            return;
        };
        self.collections.entries.loading = true;
        let work = shell::run(async move { shell.document_collections().await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                view.collections.entries.settle(result);
                if let Some(id) = view.selected.as_ref() {
                    let collection_id = view
                        .collections
                        .entries
                        .value
                        .as_ref()
                        .and_then(|entries| {
                            entries.iter().find(|entry| entry.member_ids.contains(id))
                        })
                        .map(|entry| entry.id.clone());
                    if let Some(id) = collection_id {
                        view.open_collection(id, cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn open_collection(&mut self, id: String, cx: &mut Context<Self>) {
        let Ok(shell) = self.repo.shell() else { return };
        self.collections.selected = Some(id.clone());
        self.collections.members = Remote::new_loading();
        let expected = id.clone();
        let work = shell::run(async move { shell.document_collection_members(&id).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                if view.collections.selected.as_ref() == Some(&expected) {
                    view.collections.members.settle(result);
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn change_membership(
        &mut self,
        collection: String,
        id: String,
        add: bool,
        cx: &mut Context<Self>,
    ) {
        if self.collections.busy {
            return;
        }
        let Ok(shell) = self.repo.shell() else { return };
        self.collections.busy = true;
        self.collections.error = None;
        let work =
            shell::run(async move { shell.change_document_collection(&collection, id, add).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                view.collections.busy = false;
                match result {
                    Ok(collection) => {
                        if let Some(entries) = &mut view.collections.entries.value {
                            if let Some(entry) =
                                entries.iter_mut().find(|entry| entry.id == collection.id)
                            {
                                entry.clone_from(&collection);
                            }
                        }
                        if view.collections.selected.as_ref() == Some(&collection.id) {
                            view.open_collection(collection.id, cx);
                        }
                    }
                    Err(error) => view.collections.error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn collection_workspace(
        &self,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let state = &self.collections;
        let mobile = self.density == Density::Mobile;
        let entries = state.entries.value.as_deref().unwrap_or_default();
        let selected = entries
            .iter()
            .find(|entry| Some(&entry.id) == state.selected.as_ref());
        let mut list = div()
            .flex()
            .flex_col()
            .gap_1()
            .flex_none()
            .when(!mobile, |list| list.w(px(240.)));
        for (index, entry) in entries.iter().enumerate() {
            let id = entry.id.clone();
            list = list.child(
                Button::new(
                    ("document-collection", index),
                    format!(
                        "{} · {}{}",
                        entry.name,
                        entry.member_ids.len(),
                        if self
                            .selected
                            .as_ref()
                            .is_some_and(|id| entry.member_ids.contains(id))
                        {
                            " · Contains open document"
                        } else {
                            ""
                        }
                    ),
                )
                .size(ControlSize::Sm)
                .full_width()
                .variant(if state.selected.as_ref() == Some(&entry.id) {
                    ButtonVariant::Secondary
                } else {
                    ButtonVariant::Ghost
                })
                .on_click(cx.listener(move |view, _, _, cx| view.open_collection(id.clone(), cx))),
            );
        }
        if entries.is_empty() {
            list = list.child(if state.entries.loading {
                "Loading collections…"
            } else {
                "No collections"
            });
        }
        let mut members = div().flex_1().min_w_0().flex().flex_col().gap_2();
        if let Some(collection) = selected {
            members = members.child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(collection.name.clone()),
            );
            if let Some(id) = &self.selected {
                if !collection.member_ids.contains(id) {
                    let document = id.clone();
                    let collection = collection.id.clone();
                    members = members.child(
                        Button::new("collection-add-current", "Add open document")
                            .size(ControlSize::Sm)
                            .disabled(state.busy)
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.change_membership(
                                    collection.clone(),
                                    document.clone(),
                                    true,
                                    cx,
                                );
                            })),
                    );
                }
            }
            if state.members.loading {
                members = members.child("Loading documents…");
            } else if let Some(error) = &state.members.error {
                members = members.child(error.to_string());
            } else if let Some(rows) = &state.members.value {
                if rows.is_empty() {
                    members = members.child("This collection is empty.");
                }
                for (index, row) in rows.iter().enumerate() {
                    let open = row.id.clone();
                    let remove = row.id.clone();
                    let collection = collection.id.clone();
                    members = members.child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .child(
                                Button::new(
                                    ("collection-open-record", index),
                                    row.extracted_title
                                        .as_deref()
                                        .filter(|title| !title.trim().is_empty())
                                        .unwrap_or(&row.filename)
                                        .to_owned(),
                                )
                                .size(ControlSize::Sm)
                                .variant(ButtonVariant::Ghost)
                                .on_click(cx.listener(
                                    move |view, _, _, cx| {
                                        view.collections.open = false;
                                        view.open(open.clone(), cx);
                                    },
                                )),
                            )
                            .child(
                                Button::new(("collection-remove-record", index), "Remove")
                                    .size(ControlSize::Sm)
                                    .variant(ButtonVariant::Ghost)
                                    .disabled(state.busy)
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        view.change_membership(
                                            collection.clone(),
                                            remove.clone(),
                                            false,
                                            cx,
                                        )
                                    })),
                            ),
                    );
                }
            }
        } else {
            members = members.child("Choose a collection to see its documents.");
        }
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_xl()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Collections"),
                    )
                    .child(
                        Button::new("collections-back", "Back to documents")
                            .size(ControlSize::Sm)
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.collections.open = false;
                                cx.notify();
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
                    .id("document-collections")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(
                        div()
                            .flex()
                            .gap_4()
                            .when(mobile, |layout| layout.flex_col())
                            .child(list)
                            .child(members),
                    ),
            )
    }
}
