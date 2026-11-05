//! RoomPlan revisions timeline, draft comparison, and decision reviews.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::{div, prelude::*, rgb, rgba, AnyElement, Context};
use sjel_gpui_core::interior::{
    RoomPlanDecision, RoomPlanDraft, RoomPlanReconciliationResponse, RoomPlanSpaceEntry,
};
use sjel_gpui_core::shell::{self, ShellError};

use crate::data::Remote;
use crate::theme::{tint, Palette};

use super::RoomPlanHistory;

impl RoomPlanHistory {
    pub(super) fn contains_revision(&self, id: &str) -> bool {
        self.revisions
            .value
            .as_ref()
            .is_some_and(|set| set.revisions.iter().any(|draft| draft.draft_id == id))
            || self.local.iter().any(|record| record.draft.draft_id == id)
    }

    pub(super) fn space_for_revision(&self, id: &str) -> Option<&RoomPlanSpaceEntry> {
        self.spaces.value.as_ref()?.spaces.iter().find(|entry| {
            entry
                .space
                .revision_ids
                .iter()
                .any(|revision_id| revision_id == id)
        })
    }

    pub(super) fn select(&mut self, id: String, cx: &mut Context<Self>) {
        self.selected_id = Some(id);
        self.review_error = None;
        self.preview_error = None;
        self.comparison = Remote::default();
        cx.notify();
    }

    pub(super) fn preview_remote(&mut self, revision_id: String, cx: &mut Context<Self>) {
        if self.preview_busy.is_some() {
            return;
        }
        let (Some(source), Some(handler), Some(draft)) = (
            self.source.clone(),
            self.cache_preview_handler,
            self.revisions
                .value
                .as_ref()
                .and_then(|set| {
                    set.revisions
                        .iter()
                        .find(|draft| draft.draft_id == revision_id)
                })
                .cloned(),
        ) else {
            return;
        };
        self.preview_busy = Some(revision_id.clone());
        self.preview_error = None;
        cx.notify();
        let requested_id = revision_id;
        let asset_revision_id = requested_id.clone();
        let work = shell::run(async move {
            let bytes = source.revision_asset(&asset_revision_id).await?;
            let draft_json = serde_json::to_string(&draft).map_err(|_| ShellError::Unreadable)?;
            Ok::<_, ShellError>((draft_json, bytes))
        });
        cx.spawn(async move |weak, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = weak.update(cx, |page, cx| {
                page.preview_busy = None;
                match result {
                    Ok((draft_json, bytes))
                        if page
                            .selected_draft()
                            .is_some_and(|draft| draft.draft_id == requested_id) =>
                    {
                        handler(&requested_id, &draft_json, bytes);
                    }
                    Ok(_) => {}
                    Err(error) => page.preview_error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn previous_remote_draft(&self, current: &RoomPlanDraft) -> Option<&RoomPlanDraft> {
        let revisions = &self.revisions.value.as_ref()?.revisions;
        if let Some(space) = self.space_for_revision(&current.draft_id) {
            let index = space
                .space
                .revision_ids
                .iter()
                .position(|revision_id| revision_id == &current.draft_id)?;
            let previous_id = space.space.revision_ids.get(index.checked_sub(1)?)?;
            return revisions
                .iter()
                .find(|draft| &draft.draft_id == previous_id);
        }
        revisions
            .iter()
            .filter(|draft| {
                draft.draft_id != current.draft_id && draft.created_at < current.created_at
            })
            .max_by(|left, right| left.created_at.cmp(&right.created_at))
    }

    pub(super) fn compare_previous(&mut self, cx: &mut Context<Self>) {
        let Some(source) = self.source.clone() else {
            return;
        };
        let Some(current) = self.selected_draft().cloned() else {
            return;
        };
        let Some(previous) = self.previous_remote_draft(&current).cloned() else {
            return;
        };
        if !self.revisions.value.as_ref().is_some_and(|set| {
            set.revisions
                .iter()
                .any(|draft| draft.draft_id == current.draft_id)
        }) || !self.comparison.begin()
        {
            return;
        }
        self.comparison.value = None;
        cx.notify();
        let work = shell::run(async move {
            source
                .reconcile(&previous.draft_id, &current.draft_id)
                .await
        });
        cx.spawn(async move |weak, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = weak.update(cx, |page, cx| {
                page.comparison.settle(result);
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn review(&mut self, decision: RoomPlanDecision, cx: &mut Context<Self>) {
        let Some(source) = self.source.clone() else {
            return;
        };
        let Some(draft) = self.selected_draft().cloned() else {
            return;
        };
        if draft.review.is_some() || self.review_busy.is_some() {
            return;
        }
        self.review_busy = Some(draft.draft_id.clone());
        self.review_error = None;
        cx.notify();
        let id = draft.draft_id;
        let work = shell::run(async move {
            let review = source.review(&id, decision).await?;
            let revisions = source.revisions().await?;
            Ok::<_, ShellError>((review, revisions))
        });
        cx.spawn(async move |weak, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = weak.update(cx, |page, cx| {
                page.review_busy = None;
                match result {
                    Ok((_, revisions)) => page.revisions.settle(Ok(revisions)),
                    Err(error) => page.review_error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn selected_draft(&self) -> Option<&RoomPlanDraft> {
        let remote = self
            .revisions
            .value
            .iter()
            .flat_map(|set| set.revisions.iter());
        let local = self.local.iter().map(|record| &record.draft);
        if let Some(id) = &self.selected_id {
            return remote
                .clone()
                .find(|draft| &draft.draft_id == id)
                .or_else(|| local.clone().find(|draft| &draft.draft_id == id));
        }
        remote
            .chain(local)
            .max_by(|left, right| left.created_at.cmp(&right.created_at))
    }

    pub(super) fn render_timeline(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let mut drafts = Vec::new();
        if let Some(remote) = &self.revisions.value {
            drafts.extend(remote.revisions.iter());
        }
        let mut merged = drafts
            .into_iter()
            .map(|draft| (draft, "on this Mac"))
            .collect::<Vec<_>>();
        for local in &self.local {
            if !merged
                .iter()
                .any(|(draft, _)| draft.draft_id == local.draft.draft_id)
            {
                merged.push((&local.draft, local.sync_state.as_str()));
            }
        }
        merged.sort_by(|left, right| left.0.created_at.cmp(&right.0.created_at));
        merged.reverse();
        if merged.is_empty() {
            return div()
                .p_4()
                .text_sm()
                .text_color(rgb(palette.muted))
                .child(if self.revisions.loading {
                    "Loading saved scans…"
                } else if let Some(error) = &self.revisions.error {
                    return div()
                        .p_4()
                        .text_sm()
                        .text_color(rgb(palette.red))
                        .child(error.to_string())
                        .child(
                            Button::new("roomplan-history-retry")
                                .small()
                                .ghost()
                                .label("Retry")
                                .on_click(cx.listener(|page, _, _, cx| page.refresh(cx))),
                        )
                        .into_any_element();
                } else {
                    "No scans yet. Capture a room to start its revision history."
                })
                .into_any_element();
        }
        let mut rows = Vec::with_capacity(merged.len());
        for (draft, sync_state) in merged {
            let id = draft.draft_id.clone();
            let active = self.selected_id.as_deref() == Some(&draft.draft_id)
                || (self.selected_id.is_none()
                    && self
                        .selected_draft()
                        .is_some_and(|s| s.draft_id == draft.draft_id));
            let decision = draft
                .review
                .as_ref()
                .map(|review| match review.decision.as_str() {
                    "accept" => "Reviewed",
                    "reject" => "Rejected",
                    _ => "Reviewed",
                });
            let status = decision.unwrap_or(sync_state);
            rows.push(
                div()
                    .id(gpui_kit::SharedString::from(format!(
                        "roomplan-scan-{}",
                        id
                    )))
                    .p_3()
                    .rounded_md()
                    .cursor_pointer()
                    .bg(if active {
                        rgba(tint(palette.accent, 0x18))
                    } else {
                        rgba(tint(palette.raised, 0x32))
                    })
                    .on_click(cx.listener(move |page, _, _, cx| page.select(id.clone(), cx)))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .child(draft.room.id.clone()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(palette.muted))
                                    .child(status.to_owned()),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(draft.created_at.clone()),
                    ),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap_2()
            .children(rows)
            .into_any_element()
    }

    pub(super) fn render_selected(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let Some(draft) = self.selected_draft() else {
            return div().into_any_element();
        };
        let id = &draft.draft_id;
        let has_local_asset = self.local.iter().any(|record| record.draft.draft_id == *id);
        let preview_id = draft.draft_id.clone();
        let remote_available = self.revisions.value.as_ref().is_some_and(|set| {
            set.revisions
                .iter()
                .any(|revision| revision.draft_id == *id)
        });
        let preview_action = if has_local_asset {
            self.preview_handler.map(|_| {
                Button::new("roomplan-preview-usdz")
                    .small()
                    .ghost()
                    .label("View 3D scan")
                    .on_click(cx.listener(move |page, _, _, _| {
                        if let Some(preview) = page.preview_handler {
                            preview(&preview_id);
                        }
                    }))
                    .into_any_element()
            })
        } else if remote_available {
            self.cache_preview_handler.map(|_| {
                Button::new("roomplan-download-preview-usdz")
                    .small()
                    .ghost()
                    .label(if self.preview_busy.as_deref() == Some(id) {
                        "Downloading scan…"
                    } else {
                        "Download and view 3D scan"
                    })
                    .disabled(self.preview_busy.is_some())
                    .on_click(cx.listener(move |page, _, _, cx| {
                        page.preview_remote(preview_id.clone(), cx);
                    }))
                    .into_any_element()
            })
        } else {
            None
        };
        let status = draft.review.as_ref().map(|review| review.decision.as_str());
        let waiting_for_sync = !self
            .revisions
            .value
            .as_ref()
            .is_some_and(|set| set.revisions.iter().any(|remote| remote.draft_id == *id))
            && self
                .local
                .iter()
                .any(|record| record.draft.draft_id == *id && record.sync_state == "pending");
        let actions = if status.is_some() {
            div()
                .text_sm()
                .text_color(rgb(palette.muted))
                .child(format!("Review recorded: {}", status.unwrap_or_default()))
                .into_any_element()
        } else if waiting_for_sync {
            div()
                .text_sm()
                .text_color(rgb(palette.muted))
                .child("This scan will be reviewable after it syncs to the Mac.")
                .into_any_element()
        } else {
            div()
                .flex()
                .gap_2()
                .child(
                    Button::new("roomplan-accept")
                        .small()
                        .label(if self.review_busy.as_deref() == Some(id) {
                            "Saving…"
                        } else {
                            "Accept scan"
                        })
                        .disabled(self.review_busy.is_some())
                        .on_click(
                            cx.listener(|page, _, _, cx| page.review(RoomPlanDecision::Accept, cx)),
                        ),
                )
                .child(
                    Button::new("roomplan-reject")
                        .small()
                        .ghost()
                        .label("Reject")
                        .disabled(self.review_busy.is_some())
                        .on_click(
                            cx.listener(|page, _, _, cx| page.review(RoomPlanDecision::Reject, cx)),
                        ),
                )
                .into_any_element()
        };
        let comparison_action = if let Some(previous) = self.previous_remote_draft(draft) {
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    Button::new("roomplan-compare-prior")
                        .small()
                        .ghost()
                        .label(if self.comparison.loading {
                            "Comparing scans…"
                        } else {
                            "Compare with previous scan"
                        })
                        .disabled(self.comparison.loading)
                        .on_click(cx.listener(|page, _, _, cx| page.compare_previous(cx))),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(format!("Previous scan · {}", previous.created_at)),
                )
                .into_any_element()
        } else {
            div().into_any_element()
        };
        let comparison_detail = if let Some(error) = &self.comparison.error {
            div()
                .text_sm()
                .text_color(rgb(palette.red))
                .child(error.to_string())
                .into_any_element()
        } else if let Some(result) = &self.comparison.value {
            self.render_comparison(result, palette, cx)
        } else {
            div().into_any_element()
        };
        let in_space = self.space_for_revision(id).cloned();
        let synced = self.revisions.value.as_ref().is_some_and(|set| {
            set.revisions
                .iter()
                .any(|revision| revision.draft_id == *id)
        });
        let space_action = if let Some(entry) = in_space {
            let alignment = entry
                .space
                .alignments
                .iter()
                .find(|alignment| alignment.revision_id == *id);
            let alignment_status = alignment.map(|alignment| {
                let details = alignment.report.pointer("/reconciliation/alignment");
                let status = details
                    .and_then(|value| value.get("status"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("unavailable");
                let error = details
                    .and_then(|value| value.get("error_m"))
                    .and_then(serde_json::Value::as_f64)
                    .map(|error| format!(" · {error:.2} m error"))
                    .unwrap_or_default();
                format!("Saved alignment · {status}{error}")
            });
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(palette.muted))
                        .child(format!(
                            "Room · {} · {} scans",
                            entry.space.label,
                            entry.space.revision_ids.len()
                        )),
                )
                .children(
                    alignment_status
                        .map(|status| div().text_xs().text_color(rgb(palette.muted)).child(status)),
                )
                .into_any_element()
        } else if !synced {
            div()
                .text_xs()
                .text_color(rgb(palette.muted))
                .child("Sync this scan before adding it to a room history.")
                .into_any_element()
        } else {
            let mut actions = self
                .spaces
                .value
                .as_ref()
                .map(|set| {
                    set.spaces
                        .iter()
                        .map(|entry| {
                            let space_id = entry.space.space_id.clone();
                            div()
                                .child(
                                    Button::new(gpui_kit::SharedString::from(format!(
                                        "roomplan-add-space-{}",
                                        space_id
                                    )))
                                    .small()
                                    .ghost()
                                    .label(format!("Add to {}", entry.space.label))
                                    .disabled(self.space_busy.is_some())
                                    .on_click(cx.listener(move |page, _, _, cx| {
                                        page.assign_selected_to_space(Some(space_id.clone()), cx)
                                    })),
                                )
                                .into_any_element()
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            actions.push(
                Button::new("roomplan-create-space")
                    .small()
                    .label(if self.space_busy.as_deref() == Some(id) {
                        "Saving room…"
                    } else {
                        "Start room history"
                    })
                    .disabled(self.space_busy.is_some() || self.spaces.loading)
                    .on_click(cx.listener(|page, _, _, cx| page.assign_selected_to_space(None, cx)))
                    .into_any_element(),
            );
            div()
                .flex()
                .flex_col()
                .gap_2()
                .children(actions)
                .children(self.space_feedback.as_ref().map(|feedback| {
                    div()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(feedback.clone())
                }))
                .into_any_element()
        };
        div()
            .p_4()
            .rounded_lg()
            .bg(rgba(tint(palette.raised, 0x42)))
            .border_1()
            .border_color(rgba(tint(palette.border, 0x30)))
            .flex()
            .flex_col()
            .gap_3()
            .child(div().text_xl().font_weight(gpui::FontWeight::SEMIBOLD).child(draft.room.id.clone()))
            .child(div().text_sm().text_color(rgb(palette.muted)).child(draft.created_at.clone()))
            .child(
                div()
                    .flex()
                    .gap_4()
                    .child(metric("Surfaces", draft.room.surfaces.len(), palette))
                    .child(metric("Openings", draft.room.openings.len(), palette))
                    .child(metric("Objects", draft.room.objects.len(), palette)),
            )
            .children(draft.provenance.capture_mode.as_ref().map(|mode| {
                div().text_xs().text_color(rgb(palette.muted)).child(format!("Capture mode · {mode}"))
            }))
            .child(div().text_xs().text_color(rgb(palette.muted)).child(format!("Scan ID · {id}")))
            .children(preview_action)
            .children(self.preview_error.as_ref().map(|error| {
                div()
                    .text_sm()
                    .text_color(rgb(palette.red))
                    .child(error.clone())
            }))
            .child(div().text_xs().text_color(rgb(palette.amber)).child("Review records the scan decision. It does not change room measurements or furniture placements."))
            .children(self.review_error.as_ref().map(|error| {
                div().text_sm().text_color(rgb(palette.red)).child(error.clone())
            }))
            .child(actions)
            .child(space_action)
            .child(comparison_action)
            .child(comparison_detail)
            .into_any_element()
    }

    pub(super) fn render_comparison(
        &self,
        response: &RoomPlanReconciliationResponse,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let reconciliation = &response.reconciliation;
        let alignment = &reconciliation.alignment;
        let error = alignment
            .error_m
            .map(|error| format!("{error:.2} m alignment error"))
            .unwrap_or_else(|| "no alignment estimate".to_owned());
        let space = self.space_for_revision(&response.after_revision_id);
        let space_id = space.map(|entry| entry.space.space_id.clone());
        let mut matches = Vec::new();
        for candidate in &reconciliation.matches {
            let mut row = div()
                .flex()
                .flex_col()
                .gap_2()
                .child(div().text_sm().child(format!(
                    "{} · {} · {:.2} m{}",
                    candidate.category,
                    candidate.suggestion.replace('_', " "),
                    candidate.distance_m,
                    if candidate.competing_candidates > 0 {
                        format!(" · {} similar candidates", candidate.competing_candidates)
                    } else {
                        String::new()
                    }
                )));
            let existing_link = space.and_then(|entry| {
                entry.object_links.iter().find(|link| {
                    link.revision_id == response.after_revision_id
                        && link.observation_id == candidate.after_observation_id
                })
            });
            let previous_link = space.and_then(|entry| {
                entry.object_links.iter().find(|link| {
                    link.revision_id == response.before_revision_id
                        && link.observation_id == candidate.before_observation_id
                })
            });
            if existing_link.is_none() {
                if let Some(previous_link) = previous_link {
                    let prior_item_id = previous_link.item_id.clone();
                    let label = self
                        .inventory
                        .value
                        .as_ref()
                        .and_then(|rows| rows.iter().find(|row| row.item.id == prior_item_id))
                        .map(|row| row.item.label.clone())
                        .unwrap_or_else(|| prior_item_id.clone());
                    row = row.child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(format!("Previous scan's confirmed item · {label}")),
                    );
                    if let Some(space_id) = space_id.clone() {
                        let revision_id = response.after_revision_id.clone();
                        let observation_id = candidate.after_observation_id.clone();
                        row = row.child(
                            Button::new(gpui_kit::SharedString::from(format!(
                                "roomplan-carry-link-{}",
                                observation_id
                            )))
                            .small()
                            .ghost()
                            .label("Confirm this match")
                            .disabled(self.link_busy.is_some())
                            .on_click(cx.listener(
                                move |page, _, _, cx| {
                                    page.link_observation(
                                        space_id.clone(),
                                        revision_id.clone(),
                                        observation_id.clone(),
                                        prior_item_id.clone(),
                                        cx,
                                    )
                                },
                            )),
                        );
                    }
                }
            }
            if let Some(link) = existing_link {
                let linked_item_id = link.item_id.clone();
                let label = self
                    .inventory
                    .value
                    .as_ref()
                    .and_then(|rows| rows.iter().find(|row| row.item.id == link.item_id))
                    .map(|row| row.item.label.as_str())
                    .unwrap_or(link.item_id.as_str());
                row = row.child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette.green))
                        .child(format!("Confirmed inventory match · {label}")),
                );
                if let Some(placement) = self.placements.value.as_ref().and_then(|rows| {
                    rows.iter()
                        .find(|placement| placement.item_id == link.item_id)
                }) {
                    row = row.child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(format!(
                                "Stored position · x {}, y {}, {}° · revision {}",
                                placement.x, placement.y, placement.rot, placement.revision
                            )),
                    );
                } else if self.placements.loading {
                    row = row.child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child("Loading stored position…"),
                    );
                } else if let Some(error) = &self.placements.error {
                    row = row.child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.red))
                            .child(format!("Could not read stored position: {error}")),
                    );
                }
                let editor_open = self.placement_editor.as_ref().is_some_and(|editor| {
                    editor.item_id == linked_item_id
                        && editor.observation_id == candidate.after_observation_id
                });
                let placement_space_id = space_id.clone();
                let placement_revision_id = response.after_revision_id.clone();
                let placement_observation_id = candidate.after_observation_id.clone();
                let placement_item_id = linked_item_id.clone();
                if candidate.suggestion == "possible_movement" {
                    row = row.child(
                        Button::new(gpui_kit::SharedString::from(format!(
                            "roomplan-edit-placement-{}",
                            candidate.after_observation_id
                        )))
                        .small()
                        .ghost()
                        .label(if editor_open {
                            "Close position editor"
                        } else {
                            "Review placement"
                        })
                        .disabled(
                            self.placements.loading
                                || self.placements.value.is_none()
                                || self.placement_busy.is_some(),
                        )
                        .on_click(cx.listener(
                            move |page, _, window, cx| {
                                if let Some(space_id) = placement_space_id.clone() {
                                    page.edit_placement(
                                        space_id,
                                        placement_revision_id.clone(),
                                        placement_observation_id.clone(),
                                        placement_item_id.clone(),
                                        window,
                                        cx,
                                    );
                                }
                            },
                        )),
                    );
                }
                if editor_open {
                    if let Some(editor) = self.placement_editor.as_ref() {
                        let proposal_detail = if self.placement_proposal.loading {
                            div()
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child("Aligning this scan to the measured flat…")
                                .into_any_element()
                        } else if let Some(error) = &self.placement_proposal.error {
                            div()
                                .text_xs()
                                .text_color(rgb(palette.amber))
                                .child(format!(
                                    "No scan-derived position: {error}. You can still enter the flat-plan position manually."
                                ))
                                .into_any_element()
                        } else if let Some(response) = &self.placement_proposal.value {
                            let proposal = &response.proposal;
                            div()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(palette.green))
                                        .child(format!(
                                            "Scan estimate · x {}, y {}, {}° · centre {}, {} cm · {:.2} m fit RMS · scan size {} × {} cm",
                                            proposal.x,
                                            proposal.y,
                                            proposal.rot,
                                            proposal.center_x_cm,
                                            proposal.center_y_cm,
                                            proposal.alignment_error_m,
                                            proposal.observed_width_cm,
                                            proposal.observed_depth_cm,
                                        )),
                                )
                                .child(
                                    Button::new("roomplan-use-scan-position")
                                        .small()
                                        .ghost()
                                        .label("Use scan estimate in review")
                                        .disabled(self.placement_busy.is_some())
                                        .on_click(cx.listener(|page, _, window, cx| {
                                            page.use_scan_position(window, cx)
                                        })),
                                )
                                .into_any_element()
                        } else {
                            div().into_any_element()
                        };
                        row = row.child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_2()
                                .p_2()
                                .rounded_md()
                                .bg(rgba(tint(palette.raised, 0x48)))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(palette.muted))
                                        .child(format!(
                                            "Current position · {} · revision {}. Enter the reviewed flat-plan position in centimetres.",
                                            editor.current_position.map(|(x, y, rot)| format!("x {x}, y {y}, {rot}°")).unwrap_or_else(|| "not placed".into()),
                                            editor.expected_revision,
                                        )),
                                )
                                .child(proposal_detail)
                                .child(
                                    div()
                                        .flex()
                                        .gap_2()
                                        .child(div().w(gpui::px(100.)).child(Input::new(&editor.x).small()))
                                        .child(div().w(gpui::px(100.)).child(Input::new(&editor.y).small()))
                                        .child(div().w(gpui::px(140.)).child(Input::new(&editor.rot).small())),
                                )
                                .child(
                                    Button::new("roomplan-commit-placement")
                                        .small()
                                        .label(if self.placement_busy.as_deref() == Some(&editor.item_id) {
                                            "Checking…"
                                        } else {
                                            "Save reviewed position"
                                        })
                                        .disabled(self.placement_busy.is_some())
                                        .on_click(cx.listener(|page, _, _, cx| {
                                            page.commit_reviewed_placement(cx)
                                        })),
                                ),
                        );
                    }
                }
            }
            if let Some(space_id) = space_id.clone() {
                let observation_id = candidate.after_observation_id.clone();
                let is_open = self.linking_observation.as_deref() == Some(&observation_id);
                row = row.child(
                    Button::new(gpui_kit::SharedString::from(format!(
                        "roomplan-link-observation-{}",
                        observation_id
                    )))
                    .small()
                    .ghost()
                    .label(if existing_link.is_some() {
                        "Change inventory match"
                    } else {
                        "Confirm inventory match"
                    })
                    .disabled(self.link_busy.is_some())
                    .on_click(cx.listener(move |page, _, _, cx| {
                        page.toggle_link_options(observation_id.clone(), cx)
                    })),
                );
                if is_open {
                    let options = self
                        .inventory
                        .value
                        .as_ref()
                        .map(|rows| {
                            rows.iter()
                                .filter(|row| {
                                    row.item.kind != "slot"
                                        && !matches!(row.state.as_deref(), Some("wanted" | "gone"))
                                })
                                .map(|row| {
                                    let item_id = row.item.id.clone();
                                    let label = row.item.label.clone();
                                    let selected_space = space_id.clone();
                                    let revision_id = response.after_revision_id.clone();
                                    let observation_id = candidate.after_observation_id.clone();
                                    div()
                                        .child(
                                            Button::new(gpui_kit::SharedString::from(format!(
                                                "roomplan-link-item-{}-{}",
                                                observation_id, item_id
                                            )))
                                            .small()
                                            .ghost()
                                            .label(label)
                                            .disabled(self.link_busy.is_some())
                                            .on_click(cx.listener(move |page, _, _, cx| {
                                                page.link_observation(
                                                    selected_space.clone(),
                                                    revision_id.clone(),
                                                    observation_id.clone(),
                                                    item_id.clone(),
                                                    cx,
                                                )
                                            })),
                                        )
                                        .into_any_element()
                                })
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    let options_panel = if self.inventory.loading {
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child("Loading owned furniture…")
                            .into_any_element()
                    } else if let Some(error) = &self.inventory.error {
                        div()
                            .text_xs()
                            .text_color(rgb(palette.red))
                            .child(error.to_string())
                            .into_any_element()
                    } else if options.is_empty() {
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child("No owned inventory items are available to link.")
                            .into_any_element()
                    } else {
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .children(options)
                            .into_any_element()
                    };
                    row = row.child(options_panel);
                }
            } else {
                row = row.child(div().text_xs().text_color(rgb(palette.muted)).child(
                    "Add this scan to a room history before confirming an inventory match.",
                ));
            }
            matches.push(row.into_any_element());
        }
        let mut content = vec![
            div()
                .text_sm()
                .font_weight(gpui::FontWeight::MEDIUM)
                .child(format!(
                    "{} · {} matching walls · {}",
                    alignment.status, alignment.matching_walls, error
                ))
                .into_any_element(),
            div()
                .text_xs()
                .text_color(rgb(palette.muted))
                .child(if alignment.reason.is_empty() {
                    "Furniture matches are suggestions. Confirm each identity before changing a placement."
                        .to_owned()
                } else {
                    alignment.reason.clone()
                })
                .into_any_element(),
        ];
        content.extend(matches);
        content.push(
            div()
                .text_xs()
                .text_color(rgb(palette.muted))
                .child(format!(
                    "Unmatched observations · {} before · {} after",
                    reconciliation.unmatched_before.len(),
                    reconciliation.unmatched_after.len()
                ))
                .into_any_element(),
        );
        if let Some(feedback) = &self.space_feedback {
            content.push(
                div()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child(feedback.clone())
                    .into_any_element(),
            );
        }
        div()
            .p_3()
            .rounded_md()
            .bg(rgba(tint(palette.raised, 0x32)))
            .flex()
            .flex_col()
            .gap_2()
            .children(content)
            .into_any_element()
    }
}

fn metric(label: &str, value: usize, palette: Palette) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .text_xs()
                .text_color(rgb(palette.muted))
                .child(label.to_owned()),
        )
        .child(
            div()
                .text_lg()
                .font_weight(gpui::FontWeight::MEDIUM)
                .child(value.to_string()),
        )
        .into_any_element()
}
