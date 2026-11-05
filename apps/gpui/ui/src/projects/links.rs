//! Inventory item linking and verification for project records.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::{div, prelude::*, rgb, rgba, AnyElement, Context};

use crate::data::Remote;
use crate::theme::Palette;
use sjel_gpui_core::shell::{self, ProjectRecord, ShellError};

use super::{LinkReview, Projects};

impl Projects {
    pub(super) fn select_link_record(&mut self, path: String, cx: &mut Context<Self>) {
        if self.link_busy {
            return;
        }
        self.link_generation = self.link_generation.wrapping_add(1);
        let generation = self.link_generation;
        self.link_path = Some(path.clone());
        self.link_review = None;
        self.link_message = None;
        self.links = Remote::new_loading();
        self.link_items = Remote::new_loading();
        let shell = match self.repo.shell() {
            Ok(shell) => shell,
            Err(error) => {
                let error = if self.repo.is_replica() {
                    ShellError::Store("Inventory linking needs an owner-authenticated local shell, unavailable on this device.".into())
                } else {
                    error
                };
                self.links.settle(Err(error.clone()));
                self.link_items.settle(Err(error));
                cx.notify();
                return;
            }
        };
        let work = shell::run(async move {
            let (links, items) =
                futures::join!(shell.project_inventory_links(&path), shell.inventory_rows());
            (links, items)
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |view, cx| {
                if view.link_generation != generation {
                    return;
                }
                let (links, items) =
                    result.unwrap_or((Err(ShellError::Unreachable), Err(ShellError::Unreachable)));
                view.links.settle(links);
                view.link_items.settle(items);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn review_inventory_link(
        &mut self,
        item_ref: String,
        label: String,
        raw_id: Option<String>,
        remove: bool,
        cx: &mut Context<Self>,
    ) {
        if self.link_busy {
            return;
        }
        let Some(path) = self.link_path.clone() else {
            return;
        };
        let shell = match self.repo.shell() {
            Ok(shell) => shell,
            Err(error) => {
                self.link_message = Some(error.to_string());
                cx.notify();
                return;
            }
        };
        self.link_generation = self.link_generation.wrapping_add(1);
        let generation = self.link_generation;
        self.link_busy = true;
        self.link_review = None;
        self.link_message = None;
        let request_ref = item_ref.clone();
        let work = shell::run(async move {
            let record = shell.project_link_record(&path).await?;
            if record.path != path
                || record.kind != "project"
                || record.archived
                || record.updated_at.is_empty()
            {
                return Err(ShellError::Refused(
                    409,
                    Some("The project record is not a live project or has no revision.".into()),
                ));
            }
            if !remove {
                let identity = shell.inventory_identity(&request_ref).await?;
                if identity.typed_ref != request_ref
                    || Some(identity.raw_id) != raw_id
                    || identity.revision < 1
                {
                    return Err(ShellError::Refused(
                        409,
                        Some("Inventory identity did not match the selected item.".into()),
                    ));
                }
                if identity.state.as_deref() != Some("owned") {
                    return Err(ShellError::Refused(
                        409,
                        Some("The selected Inventory item is not owned.".into()),
                    ));
                }
            }
            Ok(record)
        });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                if view.link_generation != generation {
                    return;
                }
                view.link_busy = false;
                match result {
                    Ok(record) => {
                        view.link_review = Some(LinkReview {
                            record,
                            item_ref,
                            item_label: label,
                            remove,
                        })
                    }
                    Err(error) => {
                        view.link_message = Some(format!("Could not prepare review: {error}"))
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn confirm_inventory_link(&mut self, cx: &mut Context<Self>) {
        if self.link_busy {
            return;
        }
        let (Some((project, _)), Some(review)) = (&self.project, &self.link_review) else {
            return;
        };
        let shell = match self.repo.shell() {
            Ok(shell) => shell,
            Err(error) => {
                self.link_message = Some(error.to_string());
                cx.notify();
                return;
            }
        };
        let project = project.clone();
        let record = review.record.clone();
        let path = record.path.clone();
        let item_ref = review.item_ref.clone();
        let remove = review.remove;
        let generation = self.link_generation;
        self.link_busy = true;
        self.link_message = None;
        let work = shell::run(async move {
            shell
                .project_inventory_link_decision(remove, &project, &record, &item_ref)
                .await
        });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                if view.link_generation != generation { return; }
                view.link_busy = false;
                view.link_review = None;
                match result {
                    Ok(changed) => {
                        view.select_link_record(path, cx);
                        view.link_message = Some(if changed { if remove { "Link removed." } else { "Link added after Inventory verification." } } else { "No change; the link was already in that state." }.into());
                    }
                    Err(error @ ShellError::Refused(_, _)) => {
                        view.link_message = Some(format!("Link decision refused: {error}. Review again before retrying."));
                    }
                    Err(error) => {
                        view.select_link_record(path, cx);
                        view.link_message = Some(format!("Link result unavailable: {error}. The server may have applied it; check the refreshed links before retrying."));
                    },
                }
                cx.notify();
            });
        }).detach();
        cx.notify();
    }

    pub(super) fn render_inventory_links(
        &self,
        records: &Remote<Vec<ProjectRecord>>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut body = div()
            .flex()
            .flex_col()
            .gap_3()
            .p_3()
            .child(div().text_lg().child("Project inventory links"))
            .child("Choose a project record. Links are never inferred or created automatically.");
        if self.repo.is_replica() && self.repo.shell().is_err() {
            body = body.child("Inventory linking needs an owner-authenticated local shell, unavailable on this device.");
        }
        if records.loading {
            body = body.child("Loading project records…");
        }
        if let Some(error) = &records.error {
            body = body.child(format!("Could not load project records: {error}"));
        }
        if let Some(rows) = &records.value {
            let hubs: Vec<_> = rows.iter().filter(|r| r.kind == "project").collect();
            if hubs.is_empty() {
                body = body.child("No project record is available to link.");
            }
            for row in hubs {
                let path = row.path.clone();
                body = body.child(
                    Button::new(format!("link-record-{path}"))
                        .small()
                        .label(format!("{} · {}", row.title, row.path))
                        .disabled(self.link_busy)
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.select_link_record(path.clone(), cx)
                        })),
                );
            }
        }
        if let Some(path) = &self.link_path {
            body = body.child(format!("Selected project record: {path}"));
            if self.links.loading || self.link_items.loading {
                body = body.child("Loading links and Inventory…");
            }
            if let Some(error) = &self.links.error {
                body = body.child(
                    div()
                        .text_color(rgb(palette.red))
                        .child(format!("Could not load links: {error}")),
                );
            }
            if let Some(error) = &self.link_items.error {
                body = body.child(
                    div()
                        .text_color(rgb(palette.red))
                        .child(format!("Could not load Inventory: {error}")),
                );
            }
            let refresh_path = path.clone();
            body = body.child(
                Button::new("link-refresh")
                    .small()
                    .label("Refresh links and Inventory")
                    .disabled(self.link_busy)
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.select_link_record(refresh_path.clone(), cx)
                    })),
            );
            if let Some(message) = &self.link_message {
                body = body.child(div().text_sm().child(message.clone()));
            }
            if let Some(links) = &self.links.value {
                body = body.child("Existing links (current Inventory state not checked):");
                if links.is_empty() {
                    body = body.child("No links for this record.");
                }
                for link in links {
                    let item_ref = link.item_ref.clone();
                    let label = self
                        .link_items
                        .value
                        .as_ref()
                        .and_then(|items| {
                            items.iter().find(|row| {
                                shell::inventory_reference(&row.item.id).as_deref()
                                    == Some(item_ref.as_str())
                            })
                        })
                        .map(|row| row.item.label.clone())
                        .unwrap_or_else(|| "Inventory item unavailable".into());
                    let label_review = label.clone();
                    body = body.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(format!(
                                "{label} · {} · last verified revision {}",
                                link.item_ref, link.verified_revision
                            ))
                            .child(
                                Button::new(format!("link-remove-{}", link.item_ref))
                                    .small()
                                    .label("Review removal")
                                    .disabled(self.link_busy)
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        view.review_inventory_link(
                                            item_ref.clone(),
                                            label_review.clone(),
                                            None,
                                            true,
                                            cx,
                                        )
                                    })),
                            ),
                    );
                }
            }
            if let Some(items) = &self.link_items.value {
                body = body.child("Choose an owned Inventory item:");
                let owned: Vec<_> = items
                    .iter()
                    .filter(|row| row.state.as_deref() == Some("owned"))
                    .collect();
                if owned.is_empty() {
                    body = body.child("No owned Inventory items are available.");
                }
                for row in owned {
                    let Some(item_ref) = shell::inventory_reference(&row.item.id) else {
                        continue;
                    };
                    let id = row.item.id.clone();
                    let label = row.item.label.clone();
                    body = body.child(
                        Button::new(format!("link-item-{item_ref}"))
                            .small()
                            .label(format!("{} · {}", label, id))
                            .disabled(self.link_busy)
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.review_inventory_link(
                                    item_ref.clone(),
                                    format!("{} · {}", label, id),
                                    Some(id.clone()),
                                    false,
                                    cx,
                                )
                            })),
                    );
                }
            }
            if self.link_busy {
                body = body.child("Checking or saving…");
            }
            if let Some(review) = &self.link_review {
                body = body.child(div().flex().flex_col().gap_2().p_3().border_1().border_color(rgba(palette.border))
                    .child(if review.remove { "Confirm removal" } else { "Confirm link" })
                    .child(format!("Project record: {} · {}", review.record.title, review.record.path))
                    .child(format!("Inventory: {} · {}", review.item_label, review.item_ref))
                    .child(if review.remove { "Removal does not check whether the Inventory item still exists." } else { "Inventory was checked as owned. The server checks again when you confirm." })
                    .child(format!("Project revision: {}", review.record.updated_at))
                    .child(Button::new("link-confirm").small().primary().label(if review.remove { "Confirm removal" } else { "Confirm link" }).disabled(self.link_busy)
                        .on_click(cx.listener(|view, _, _, cx| view.confirm_inventory_link(cx))))
                    .child(Button::new("link-cancel").small().ghost().label("Cancel").disabled(self.link_busy)
                        .on_click(cx.listener(|view, _, _, cx| { view.link_review = None; cx.notify(); }))));
            }
        }
        div()
            .id("project-links-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .child(body)
            .into_any_element()
    }
}
