//! Interior layout planning, search, composition, and candidate review.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::{div, prelude::*, rgb, rgba, AnyElement, Context};
use sjel_gpui_core::interior::{RoomPlanJob, RoomPlanJobState};
use sjel_gpui_core::shell::{self, ShellError};

use crate::data::Remote;
use crate::theme::{tint, Palette};

use super::RoomPlanHistory;

use sjel_gpui_core::interior::planning::{add_planning_analysis, planning_candidates};
pub(crate) use sjel_gpui_core::interior::planning::{PlanningCandidate, PlanningKind};

impl RoomPlanHistory {
    pub(super) fn load_layout_preview(&mut self, id: String, cx: &mut Context<Self>) {
        let Some(source) = self.source.clone() else {
            return;
        };
        self.layout_preview_id = Some(id.clone());
        self.layout_preview = Remote::new_loading();
        cx.notify();
        let requested_id = id.clone();
        let work = shell::run(async move { source.layout(&id).await });
        cx.spawn(async move |weak, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = weak.update(cx, |page, cx| {
                if page.selected_layout_id.as_deref() == Some(requested_id.as_str()) {
                    page.layout_preview.settle(result);
                    page.layout_preview_id = Some(requested_id);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(super) fn select_layout(&mut self, id: Option<String>, cx: &mut Context<Self>) {
        self.selected_layout_id = id.clone();
        if let Some(id) = id {
            self.load_layout_preview(id, cx);
        } else {
            self.layout_preview = Remote::default();
            self.layout_preview_id = None;
            cx.notify();
        }
    }

    pub(super) fn toggle_compose_ref(&mut self, id: String, cx: &mut Context<Self>) {
        if !self.compose_refs.remove(&id) {
            self.compose_refs.insert(id);
        }
        cx.notify();
    }

    pub(super) fn start_planning(&mut self, kind: PlanningKind, cx: &mut Context<Self>) {
        if self.planning_starting
            || matches!(
                self.planning_job.as_ref().map(|job| &job.state),
                Some(RoomPlanJobState::Running { .. })
            )
        {
            return;
        }
        let Some(source) = self.source.clone() else {
            self.planning_error =
                Some("Connect to the Interior capability before planning.".into());
            cx.notify();
            return;
        };
        let base = if matches!(kind, PlanningKind::Search) {
            let Some(id) = self.selected_layout_id.clone() else {
                self.planning_error = Some("Choose a saved layout first.".into());
                cx.notify();
                return;
            };
            if self.layout_preview_id.as_deref() != Some(id.as_str()) {
                self.planning_error = Some("Wait for the selected layout to load.".into());
                cx.notify();
                return;
            }
            let Some(preview) = self.layout_preview.value.as_ref() else {
                self.planning_error = Some("The selected layout has no loaded positions.".into());
                cx.notify();
                return;
            };
            preview.layout.items.clone()
        } else {
            Vec::new()
        };
        let layout = self.selected_layout_id.clone().unwrap_or_default();
        let move_refs = base
            .iter()
            .map(|item| item.reference.clone())
            .collect::<Vec<_>>();
        let compose_refs = self.compose_refs.iter().cloned().collect::<Vec<_>>();
        if matches!(kind, PlanningKind::Compose) && compose_refs.is_empty() {
            self.planning_error = Some("Select owned items to compose a layout.".into());
            cx.notify();
            return;
        }
        let request = async move {
            match kind {
                PlanningKind::Search => source.search_layout(&layout, &move_refs).await,
                PlanningKind::Compose => source.compose_layout(&compose_refs).await,
            }
        };
        self.planning_starting = true;
        self.planning_analysis_pending = false;
        self.planning_candidates.clear();
        self.planning_error = None;
        self.planning_message = Some(match kind {
            PlanningKind::Search => "Starting a deterministic layout search…".into(),
            PlanningKind::Compose => "Starting a deterministic room composition…".into(),
        });
        cx.notify();
        let work = shell::run(request);
        cx.spawn(async move |weak, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = weak.update(cx, |page, cx| {
                page.planning_starting = false;
                match result {
                    Ok(receipt) => page.watch_planning_job(receipt.job, kind, base, cx),
                    Err(error) => {
                        page.planning_error = Some(error.to_string());
                        page.planning_message = None;
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn watch_planning_job(
        &mut self,
        id: u64,
        kind: PlanningKind,
        base: Vec<sjel_gpui_core::interior::RoomPlanSavedLayoutItem>,
        cx: &mut Context<Self>,
    ) {
        let Some(source) = self.source.clone() else {
            return;
        };
        self.planning_job = Some(RoomPlanJob {
            id,
            state: RoomPlanJobState::Running { since_ms: 0 },
        });
        let work_source = source.clone();
        cx.spawn(async move |weak, cx| loop {
            cx.background_executor().timer(Duration::from_secs(1)).await;
            let source = work_source.clone();
            let result = shell::run(async move { source.planning_job(id).await })
                .await
                .unwrap_or(Err(ShellError::Unreachable));
            let mut finished = false;
            let mut review_requests = None;
            let updated = weak.update(cx, |page, cx| {
                match result {
                    Ok(job) => {
                        match &job.state {
                            RoomPlanJobState::Running { since_ms } => {
                                page.planning_message =
                                    Some(format!("Planning… {}s", since_ms / 1000));
                            }
                            RoomPlanJobState::Done { outcome } => {
                                page.planning_candidates =
                                    planning_candidates(kind, outcome, &base);
                                if page.planning_candidates.is_empty() {
                                    page.planning_message = Some(
                                        "The solver found no proposals for these inputs.".into(),
                                    );
                                } else if let Some(source) = page.source.clone() {
                                    page.planning_analysis_pending = true;
                                    page.planning_message = Some(
                                        "Checking rule details, access paths and sunlight…".into(),
                                    );
                                    review_requests = Some((
                                        source,
                                        page.planning_candidates
                                            .iter()
                                            .map(|candidate| candidate.items.clone())
                                            .collect::<Vec<_>>(),
                                    ));
                                }
                                finished = true;
                            }
                            RoomPlanJobState::Failed { reason } => {
                                page.planning_error = Some(reason.clone());
                                page.planning_message = None;
                                finished = true;
                            }
                        }
                        page.planning_job = Some(job);
                    }
                    Err(error) => {
                        page.planning_job = None;
                        page.planning_error = Some(error.to_string());
                        page.planning_message = None;
                        finished = true;
                    }
                }
                cx.notify();
            });
            if updated.is_err() {
                break;
            }
            if let Some((source, candidates)) = review_requests {
                let mut reviews = Vec::with_capacity(candidates.len());
                for items in candidates {
                    let source = source.clone();
                    reviews.push(
                        shell::run(async move { source.preview_placements(&items).await })
                            .await
                            .unwrap_or(Err(ShellError::Unreachable)),
                    );
                }
                let _ = weak.update(cx, |page, cx| {
                    for (candidate, review) in page.planning_candidates.iter_mut().zip(reviews) {
                        match review {
                            Ok(review) => add_planning_analysis(candidate, &review),
                            Err(error) => candidate
                                .details
                                .push_str(&format!(" · Detailed review failed: {error}")),
                        }
                    }
                    page.planning_analysis_pending = false;
                    page.planning_message =
                        Some("Proposal checks are ready; review findings before saving.".into());
                    cx.notify();
                });
            }
            if finished {
                break;
            }
        })
        .detach();
    }

    pub(super) fn save_planning_candidate(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.saving_candidate.is_some() {
            return;
        }
        let Some(candidate) = self.planning_candidates.get(index).cloned() else {
            return;
        };
        let Some(source) = self.source.clone() else {
            return;
        };
        let id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let id = format!("roomplan-proposal-{id}");
        let note = format!(
            "Reviewed deterministic RoomPlan proposal: {}. {}",
            candidate.title, candidate.details
        );
        self.saving_candidate = Some(index);
        self.planning_error = None;
        self.planning_message = Some("Saving a new layout proposal…".into());
        cx.notify();
        let work = shell::run(async move {
            source
                .save_proposal(&id, &candidate.title, candidate.items, &note)
                .await
                .map(|_| id)
        });
        cx.spawn(async move |weak, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = weak.update(cx, |page, cx| {
                page.saving_candidate = None;
                match result {
                    Ok(id) => {
                        page.selected_layout_id = Some(id.clone());
                        page.planning_message = Some(
                            "Proposal saved as a new layout; current placements were not changed."
                                .into(),
                        );
                        page.refresh_layouts(cx);
                        page.load_layout_preview(id, cx);
                    }
                    Err(error) => {
                        page.planning_error = Some(error.to_string());
                        page.planning_message = None;
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn render_layout_choices(
        &self,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut choices = vec![Button::new("roomplan-current-layout")
            .small()
            .ghost()
            .label(if self.selected_layout_id.is_none() {
                "✓ Current placements"
            } else {
                "Current placements"
            })
            .on_click(cx.listener(|page, _, _, cx| page.select_layout(None, cx)))
            .into_any_element()];
        if let Some(layouts) = self.layouts.value.as_ref() {
            for layout in layouts {
                let id = layout.id.clone();
                choices.push(
                    Button::new(gpui_kit::SharedString::from(format!(
                        "roomplan-layout-{id}"
                    )))
                    .small()
                    .ghost()
                    .label(if self.selected_layout_id.as_deref() == Some(&id) {
                        format!("✓ {}", layout.name)
                    } else {
                        layout.name.clone()
                    })
                    .on_click(
                        cx.listener(move |page, _, _, cx| page.select_layout(Some(id.clone()), cx)),
                    )
                    .into_any_element(),
                );
            }
        }
        let status = if let Some(error) = &self.layouts.error {
            div()
                .text_xs()
                .text_color(rgb(palette.red))
                .child(format!("Saved layouts unavailable · {error}"))
                .into_any_element()
        } else if self.layouts.loading {
            div()
                .text_xs()
                .text_color(rgb(palette.muted))
                .child("Loading saved layouts…")
                .into_any_element()
        } else if self.layouts.value.as_ref().is_some_and(Vec::is_empty) {
            div()
                .text_xs()
                .text_color(rgb(palette.muted))
                .child("No named layouts saved yet.")
                .into_any_element()
        } else {
            div().into_any_element()
        };
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_1()
            .children(choices)
            .child(status)
            .into_any_element()
    }

    pub(super) fn render_planning(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let searching = self.planning_starting
            || matches!(
                self.planning_job.as_ref().map(|job| &job.state),
                Some(RoomPlanJobState::Running { .. })
            );
        let base_loaded = self.selected_layout_id.as_ref().is_some_and(|id| {
            self.layout_preview_id.as_deref() == Some(id.as_str())
                && self
                    .layout_preview
                    .value
                    .as_ref()
                    .is_some_and(|preview| !preview.layout.items.is_empty())
        });
        let compose_items = self
            .inventory
            .value
            .as_deref()
            .unwrap_or_default()
            .iter()
            .filter(|row| {
                row.state.as_deref() == Some("owned")
                    && row.item.width.is_some()
                    && row.item.depth.is_some()
            });
        let compose_buttons = compose_items.map(|row| {
            let id = row.item.id.clone();
            let selected = self.compose_refs.contains(&id);
            let label = if selected {
                format!("✓ {}", row.item.label)
            } else {
                row.item.label.clone()
            };
            Button::new(gpui_kit::SharedString::from(format!(
                "roomplan-compose-item-{id}"
            )))
            .small()
            .label(label)
            .on_click(cx.listener(move |page, _, _, cx| page.toggle_compose_ref(id.clone(), cx)))
        });
        let compose_state = if self.inventory.loading {
            Some("Loading owned items…".to_owned())
        } else if self.inventory.error.is_some() {
            None
        } else if self.inventory.value.as_ref().is_some_and(|rows| {
            !rows.iter().any(|row| {
                row.state.as_deref() == Some("owned")
                    && row.item.width.is_some()
                    && row.item.depth.is_some()
            })
        }) {
            Some("No owned items with measured dimensions are available to compose.".to_owned())
        } else {
            None
        };
        let job_status = self.planning_job.as_ref().map(|job| match &job.state {
            RoomPlanJobState::Running { since_ms } => {
                format!("Solver running · {}s", since_ms / 1000)
            }
            RoomPlanJobState::Done { .. } => "Solver finished".into(),
            RoomPlanJobState::Failed { reason } => format!("Solver failed · {reason}"),
        });
        let candidates = self
            .planning_candidates
            .iter()
            .enumerate()
            .map(|(index, candidate)| {
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .py_2()
                    .border_t_1()
                    .border_color(rgba(tint(palette.border, 0x22)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .child(candidate.title.clone()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(palette.muted))
                                    .child(candidate.details.clone()),
                            ),
                    )
                    .child(
                        Button::new(gpui_kit::SharedString::from(format!(
                            "roomplan-save-proposal-{index}"
                        )))
                        .small()
                        .label(if self.saving_candidate == Some(index) {
                            "Saving…"
                        } else {
                            "Save proposal"
                        })
                        .disabled(self.saving_candidate.is_some() || self.planning_analysis_pending)
                        .on_click(cx.listener(move |page, _, _, cx| {
                            page.save_planning_candidate(index, cx)
                        })),
                    )
            });
        div()
            .p_3()
            .rounded_md()
            .bg(rgba(tint(palette.raised, 0x24)))
            .flex()
            .flex_col()
            .gap_2()
            .child(div().text_sm().font_weight(gpui::FontWeight::SEMIBOLD).child("Plan a layout"))
            .child(div().text_xs().text_color(rgb(palette.muted)).child("Run the Interior solver, review its arrangements, then save a proposal as a new layout. Current placements stay unchanged."))
            .child(div().flex().items_center().gap_2()
                .child(Button::new("roomplan-search-layout")
                    .small()
                    .label(if let Some(id) = &self.selected_layout_id { format!("Search {} · move all items", id) } else { "Choose a saved layout to search".into() })
                    .disabled(searching || !base_loaded)
                    .on_click(cx.listener(|page, _, _, cx| page.start_planning(PlanningKind::Search, cx))))
                .child(Button::new("roomplan-compose-layout")
                    .small()
                    .label(format!("Compose from {} selected", self.compose_refs.len()))
                    .disabled(searching || self.compose_refs.is_empty())
                    .on_click(cx.listener(|page, _, _, cx| page.start_planning(PlanningKind::Compose, cx)))))
            .when(!base_loaded, |panel| panel.child(div().text_xs().text_color(rgb(palette.muted)).child("Choose a named layout above and wait for its item positions to load before searching.")))
            .child(div().flex().flex_wrap().items_center().gap_1().children(compose_buttons))
            .when_some(compose_state, |panel, status| panel.child(div().text_xs().text_color(rgb(palette.muted)).child(status)))
            .when_some(self.inventory.error.clone(), |panel, error| panel.child(
                div().flex().items_center().gap_2()
                    .child(div().text_xs().text_color(rgb(palette.red)).child(format!("Owned items unavailable · {error}")))
                    .child(Button::new("roomplan-compose-retry").small().ghost().label("Retry")
                        .on_click(cx.listener(|page, _, _, cx| page.refresh_inventory(cx))))
            ))
            .when_some(job_status, |panel, status| panel.child(div().text_xs().text_color(rgb(palette.muted)).child(status)))
            .when_some(self.planning_message.clone(), |panel, message| panel.child(div().text_xs().text_color(rgb(palette.muted)).child(message)))
            .when_some(self.planning_error.clone(), |panel, error| panel.child(div().text_xs().text_color(rgb(palette.red)).child(error)))
            .children(candidates)
            .into_any_element()
    }
}
