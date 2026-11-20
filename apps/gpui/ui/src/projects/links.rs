//! Inventory item linking and verification for project records.

use std::rc::Rc;

use crate::ely::data_display::Tone;
use crate::ely::layout::Sheet;
use crate::ely::primitives::Place;
use crate::ely::tables::{Cell, Column, DataTable, Row, RowAction};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::{div, prelude::*, px, rems, rgb, rgba, AnyElement, Context};

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
                let columns = vec![
                    Column::new("label", "LINKED ITEM").min_width(rems(12.)),
                    Column::new("reference", "REFERENCE").width(rems(9.)),
                    Column::new("revision", "VERIFIED REV").width(rems(8.)),
                    Column::new("actions", "")
                        .width(rems(9.))
                        .actions([RowAction::new("remove", "Review removal")]),
                ];
                let rows: Rc<Vec<Row>> = Rc::new(
                    links
                        .iter()
                        .map(|link| {
                            let label = self
                                .link_items
                                .value
                                .as_ref()
                                .and_then(|items| {
                                    items.iter().find(|row| {
                                        shell::inventory_reference(&row.item.id).as_deref()
                                            == Some(link.item_ref.as_str())
                                    })
                                })
                                .map(|row| row.item.label.clone())
                                .unwrap_or_else(|| "Inventory item unavailable".into());
                            Row::new(
                                link.item_ref.clone(),
                                [
                                    Cell::Text(label.into()),
                                    Cell::Text(link.item_ref.clone().into()),
                                    Cell::Tag(
                                        format!("rev {}", link.verified_revision).into(),
                                        Tone::Neutral,
                                    ),
                                    Cell::Actions(vec!["remove".into()]),
                                ],
                            )
                        })
                        .collect(),
                );
                let labels_by_ref: std::collections::HashMap<String, String> = links
                    .iter()
                    .map(|link| {
                        let label = self
                            .link_items
                            .value
                            .as_ref()
                            .and_then(|items| {
                                items.iter().find(|row| {
                                    shell::inventory_reference(&row.item.id).as_deref()
                                        == Some(link.item_ref.as_str())
                                })
                            })
                            .map(|row| row.item.label.clone())
                            .unwrap_or_else(|| "Inventory item unavailable".into());
                        (link.item_ref.clone(), label)
                    })
                    .collect();
                let on_act = cx.entity();
                body = body
                    .child("Existing links (current Inventory state not checked):")
                    .child(
                        DataTable::new("project-existing-links-table", columns)
                            .label("Existing inventory links")
                            .rows(rows)
                            .on_action(move |key, action, _, cx| {
                                if action.as_ref() == "remove" {
                                    let item_ref = key.to_string();
                                    let label = labels_by_ref
                                        .get(&item_ref)
                                        .cloned()
                                        .unwrap_or_else(|| "Inventory item unavailable".into());
                                    on_act.update(cx, |view, cx| {
                                        view.review_inventory_link(item_ref, label, None, true, cx);
                                    });
                                }
                            })
                            .empty("No links for this record."),
                    );
            }
            if let Some(items) = &self.link_items.value {
                let owned: Vec<_> = items
                    .iter()
                    .filter(|row| row.state.as_deref() == Some("owned"))
                    .filter_map(|row| {
                        shell::inventory_reference(&row.item.id).map(|item_ref| (row, item_ref))
                    })
                    .collect();
                let columns = vec![
                    Column::new("label", "OWNED ASSET").min_width(rems(14.)),
                    Column::new("id", "ITEM ID").width(rems(9.)),
                    Column::new("ref", "TYPED REF").width(rems(9.)),
                    Column::new("actions", "")
                        .width(rems(8.))
                        .actions([RowAction::new("link", "Link item")]),
                ];
                let rows: Rc<Vec<Row>> = Rc::new(
                    owned
                        .iter()
                        .map(|(row, item_ref)| {
                            Row::new(
                                item_ref.clone(),
                                [
                                    Cell::Text(row.item.label.clone().into()),
                                    Cell::Text(row.item.id.clone().into()),
                                    Cell::Text(item_ref.clone().into()),
                                    Cell::Actions(vec!["link".into()]),
                                ],
                            )
                        })
                        .collect(),
                );
                let owned_map: std::collections::HashMap<String, (String, String)> = owned
                    .iter()
                    .map(|(row, item_ref)| {
                        (
                            item_ref.clone(),
                            (row.item.label.clone(), row.item.id.clone()),
                        )
                    })
                    .collect();
                let on_act = cx.entity();
                body = body.child("Choose an owned Inventory item:").child(
                    DataTable::new("project-owned-inventory-table", columns)
                        .label("Owned inventory items")
                        .rows(rows)
                        .on_action(move |key, action, _, cx| {
                            if action.as_ref() == "link" {
                                if let Some((label, id)) = owned_map.get(key.as_ref()).cloned() {
                                    let item_ref = key.to_string();
                                    on_act.update(cx, |view, cx| {
                                        view.review_inventory_link(
                                            item_ref,
                                            format!("{label} · {id}"),
                                            Some(id),
                                            false,
                                            cx,
                                        );
                                    });
                                }
                            }
                        })
                        .empty("No owned Inventory items are available."),
                );
            }
            if self.link_busy {
                body = body.child("Checking or saving…");
            }
        }
        let review_sheet = self.link_review.as_ref().map(|review| {
            let close_entity = cx.entity();
            let sheet_title = if review.remove {
                "Confirm Link Removal"
            } else {
                "Confirm Inventory Link"
            };
            Sheet::new(
                "project-inventory-link-sheet",
                Place::Right,
                sheet_title,
                move |_, cx| {
                    close_entity.update(cx, |view, cx| {
                        if !view.link_busy {
                            view.link_review = None;
                            cx.notify();
                        }
                    });
                },
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .p_3()
                    .border_1()
                    .border_color(rgba(palette.border))
                    .rounded(px(6.))
                    .child(format!(
                        "Project record: {} · {}",
                        review.record.title, review.record.path
                    ))
                    .child(format!(
                        "Inventory: {} · {}",
                        review.item_label, review.item_ref
                    ))
                    .child(format!("Project revision: {}", review.record.updated_at))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Button::new("link-confirm")
                                    .small()
                                    .primary()
                                    .label(if review.remove {
                                        "Confirm removal"
                                    } else {
                                        "Confirm link"
                                    })
                                    .disabled(self.link_busy)
                                    .on_click(cx.listener(|view, _, _, cx| {
                                        view.confirm_inventory_link(cx)
                                    })),
                            )
                            .child(
                                Button::new("link-cancel")
                                    .small()
                                    .ghost()
                                    .label("Cancel")
                                    .disabled(self.link_busy)
                                    .on_click(cx.listener(|view, _, _, cx| {
                                        view.link_review = None;
                                        cx.notify();
                                    })),
                            ),
                    ),
            )
        });
        div()
            .id("project-links-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .child(body)
            .children(review_sheet)
            .into_any_element()
    }
}
