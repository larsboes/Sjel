//! Shared scan history and review for the Mac capability and paired iPhone.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::Entity;
use gpui_kit::{
    canvas, div, point, prelude::*, px, rgb, rgba, AnyElement, Context, PathBuilder, Render,
    Window, WindowAppearance,
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sjel_gpui_core::interior::{
    RoomPlanDecision, RoomPlanDraft, RoomPlanJob, RoomPlanJobState, RoomPlanLiveUpdate,
    RoomPlanLocalRecord, RoomPlanModel, RoomPlanPlacement, RoomPlanPlacementProposalResponse,
    RoomPlanReconciliationResponse, RoomPlanRevisionSet, RoomPlanSavedLayoutPreview,
    RoomPlanSource, RoomPlanSpaceEntry, RoomPlanSpaceSet,
};
use sjel_gpui_core::inventory::{InteriorLayoutSummary, InventoryRow};
use sjel_gpui_core::shell::{self, ShellError};

use crate::data::Remote;
use crate::theme::{tint, Palette};

pub struct RoomPlanHistory {
    source: Option<RoomPlanSource>,
    revisions: Remote<RoomPlanRevisionSet>,
    model: Remote<RoomPlanModel>,
    layouts: Remote<Vec<InteriorLayoutSummary>>,
    layout_preview: Remote<RoomPlanSavedLayoutPreview>,
    selected_layout_id: Option<String>,
    layout_preview_id: Option<String>,
    spaces: Remote<RoomPlanSpaceSet>,
    inventory: Remote<Vec<InventoryRow>>,
    placements: Remote<Vec<RoomPlanPlacement>>,
    placement_proposal: Remote<RoomPlanPlacementProposalResponse>,
    comparison: Remote<RoomPlanReconciliationResponse>,
    local: Vec<RoomPlanLocalRecord>,
    live_snapshot: Option<RoomPlanLiveUpdate>,
    planning_job: Option<RoomPlanJob>,
    planning_candidates: Vec<PlanningCandidate>,
    planning_error: Option<String>,
    planning_message: Option<String>,
    planning_starting: bool,
    planning_analysis_pending: bool,
    compose_refs: std::collections::BTreeSet<String>,
    saving_candidate: Option<usize>,
    import_handler: Option<fn()>,
    preview_handler: Option<fn(&str)>,
    cache_preview_handler: Option<fn(&str, &str, Vec<u8>)>,
    preview_busy: Option<String>,
    preview_error: Option<String>,
    selected_id: Option<String>,
    review_busy: Option<String>,
    review_error: Option<String>,
    space_busy: Option<String>,
    space_feedback: Option<String>,
    linking_observation: Option<String>,
    link_busy: Option<String>,
    placement_editor: Option<PlacementEditor>,
    placement_busy: Option<String>,
    palette: Option<Palette>,
}

#[derive(Clone)]
struct PlacementEditor {
    space_id: String,
    revision_id: String,
    observation_id: String,
    item_id: String,
    expected_revision: i64,
    current_position: Option<(i32, i32, i32)>,
    x: Entity<InputState>,
    y: Entity<InputState>,
    rot: Entity<InputState>,
}

#[derive(Clone)]
struct PlanPiece {
    reference: String,
    x: i32,
    y: i32,
    rot: i32,
    size: Option<[i32; 2]>,
}

#[derive(Clone, Copy)]
enum PlanningKind {
    Search,
    Compose,
}

#[derive(Clone)]
struct PlanningCandidate {
    title: String,
    details: String,
    items: Vec<serde_json::Value>,
}

fn planning_candidates(
    kind: PlanningKind,
    outcome: &serde_json::Value,
    base: &[sjel_gpui_core::interior::RoomPlanSavedLayoutItem],
) -> Vec<PlanningCandidate> {
    match kind {
        PlanningKind::Search => outcome
            .get("hits")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .enumerate()
            .filter_map(|(index, hit)| {
                let places = hit.get("places")?.as_object()?;
                let mut items = base
                    .iter()
                    .filter_map(|item| serde_json::to_value(item).ok())
                    .collect::<Vec<_>>();
                for item in &mut items {
                    let reference = item.get("ref")?.as_str()?.to_owned();
                    if let Some(position) =
                        places.get(&reference).and_then(serde_json::Value::as_array)
                    {
                        if position.len() == 2 {
                            item["x"] = position[0].clone();
                            item["y"] = position[1].clone();
                        }
                    }
                }
                let reserve = hit
                    .get("tightest_reserve_cm")
                    .and_then(serde_json::Value::as_i64)
                    .map_or_else(
                        || "reserve not reported".to_owned(),
                        |cm| format!("{cm} cm reserve"),
                    );
                let details = format!(
                    "{} soft findings · {} cm narrowest route · {} cm wall contact · {}{}",
                    hit.get("soft")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0),
                    hit.get("bottleneck_cm")
                        .and_then(serde_json::Value::as_i64)
                        .unwrap_or(0),
                    hit.get("wall_contact_cm")
                        .and_then(serde_json::Value::as_i64)
                        .unwrap_or(0),
                    reserve,
                    if hit
                        .get("pareto")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false)
                    {
                        " · Pareto choice"
                    } else {
                        ""
                    }
                );
                Some(PlanningCandidate {
                    title: format!("RoomPlan search proposal {}", index + 1),
                    details,
                    items,
                })
            })
            .collect(),
        PlanningKind::Compose => outcome
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
            .filter_map(|(index, result)| {
                let places = result.get("places")?.as_array()?;
                let items = places
                    .iter()
                    .filter_map(|place| {
                        let tuple = place.as_array()?;
                        Some(serde_json::json!({
                            "ref": tuple.first()?.as_str()?,
                            "x": tuple.get(1)?.get(0)?.as_i64()?,
                            "y": tuple.get(1)?.get(1)?.as_i64()?,
                            "rot": tuple.get(2)?.as_i64()?,
                        }))
                    })
                    .collect::<Vec<_>>();
                if items.is_empty() {
                    return None;
                }
                let hard = result
                    .get("hard")
                    .and_then(serde_json::Value::as_array)
                    .map_or(0, Vec::len);
                let soft = result
                    .get("soft")
                    .and_then(serde_json::Value::as_array)
                    .map_or(0, Vec::len);
                let hard_rules = result
                    .get("hard")
                    .and_then(serde_json::Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(serde_json::Value::as_str)
                    .take(3)
                    .collect::<Vec<_>>()
                    .join(", ");
                let soft_rules = result
                    .get("soft")
                    .and_then(serde_json::Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(serde_json::Value::as_str)
                    .take(3)
                    .collect::<Vec<_>>()
                    .join(", ");
                let details = format!(
                    "{} · {} hard ({}) · {} soft ({}) · {} cm narrowest route · {:.1} m² free",
                    if result
                        .get("pass")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false)
                    {
                        "Passes checks"
                    } else {
                        "Needs review"
                    },
                    hard,
                    if hard_rules.is_empty() {
                        "none"
                    } else {
                        hard_rules.as_str()
                    },
                    soft,
                    if soft_rules.is_empty() {
                        "none"
                    } else {
                        soft_rules.as_str()
                    },
                    result
                        .get("bottleneck_cm")
                        .and_then(serde_json::Value::as_i64)
                        .unwrap_or(0),
                    result
                        .get("free_m2")
                        .and_then(serde_json::Value::as_f64)
                        .unwrap_or(0.0),
                );
                Some(PlanningCandidate {
                    title: format!("RoomPlan composition {}", index + 1),
                    details,
                    items,
                })
            })
            .collect(),
    }
}

fn add_planning_analysis(candidate: &mut PlanningCandidate, review: &serde_json::Value) {
    let Some(check) = review.get("check") else {
        candidate.details.push_str(" · Detailed review unavailable");
        return;
    };
    let violations = |severity: &str| {
        check
            .get(severity)
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|finding| {
                let rule = finding.get("rule")?.as_str()?;
                let message = finding
                    .get("message")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("details unavailable");
                Some(format!("{rule}: {message}"))
            })
            .collect::<Vec<_>>()
    };
    let hard = violations("hard");
    let soft = violations("soft");
    let review_gaps = check
        .get("unchecked")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|gap| {
            Some(format!(
                "{}: {}",
                gap.get("rule")?.as_str()?,
                gap.get("reason")?.as_str()?
            ))
        })
        .collect::<Vec<_>>();
    let uncertainties = check
        .get("uncertainties")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|uncertainty| {
            Some(format!(
                "{}: {}",
                uncertainty.get("label")?.as_str()?,
                uncertainty
                    .get("fields")?
                    .as_array()?
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        })
        .collect::<Vec<_>>();
    let mut corridors = check
        .get("metrics")
        .and_then(|metrics| metrics.get("corridors"))
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|route| {
            let from = route.get("from")?.as_str()?;
            let to = route.get("to")?.as_str()?;
            let width = route.get("width_cm").and_then(serde_json::Value::as_i64);
            Some((
                width.unwrap_or(i64::MAX),
                format!(
                    "{from}–{to} {}",
                    width.map_or_else(|| "unmeasured".into(), |cm| format!("{cm} cm"))
                ),
            ))
        })
        .collect::<Vec<_>>();
    corridors.sort_by_key(|(width, _)| *width);
    let routes = corridors
        .iter()
        .take(3)
        .map(|(_, route)| route.as_str())
        .collect::<Vec<_>>();
    let sun = review
        .get("sun")
        .and_then(|sun| sun.get("hits_per_piece"))
        .and_then(serde_json::Value::as_object)
        .map(|hits| {
            let checked = review
                .get("sun")
                .and_then(|sun| sun.get("hours"))
                .and_then(serde_json::Value::as_array)
                .map_or(0, Vec::len);
            let mut pieces = hits
                .iter()
                .filter_map(|(reference, count)| Some((reference.as_str(), count.as_u64()?)))
                .collect::<Vec<_>>();
            pieces.sort_by(|left, right| right.1.cmp(&left.1));
            if pieces.is_empty() {
                format!("no direct sun across {checked} checked hours")
            } else {
                format!(
                    "direct sun in {checked} checked hours: {}",
                    pieces
                        .iter()
                        .take(3)
                        .map(|(reference, count)| format!("{reference} {count}h"))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
        })
        .or_else(|| {
            review
                .get("sun_note")
                .and_then(serde_json::Value::as_str)
                .map(|note| format!("sunlight unavailable: {note}"))
        })
        .unwrap_or_else(|| "sunlight unavailable".into());
    candidate.details = format!(
        "{} · {} hard findings{} · {} soft findings{} · unchecked: {} · uncertain: {} · access paths: {} · {}",
        candidate.details,
        hard.len(),
        if hard.is_empty() {
            String::new()
        } else {
            format!(" [{}]", hard.join("; "))
        },
        soft.len(),
        if soft.is_empty() {
            String::new()
        } else {
            format!(" [{}]", soft.join("; "))
        },
        if review_gaps.is_empty() {
            "none".into()
        } else {
            review_gaps.join("; ")
        },
        if uncertainties.is_empty() {
            "none".into()
        } else {
            uncertainties.join("; ")
        },
        if routes.is_empty() {
            "none reported".into()
        } else {
            routes.join(", ")
        },
        sun,
    );
}

impl RoomPlanHistory {
    pub fn new(source: Option<RoomPlanSource>, cx: &mut Context<Self>) -> Self {
        let page = Self {
            source,
            revisions: Remote::default(),
            model: Remote::default(),
            layouts: Remote::default(),
            layout_preview: Remote::default(),
            selected_layout_id: None,
            layout_preview_id: None,
            spaces: Remote::default(),
            inventory: Remote::default(),
            placements: Remote::default(),
            placement_proposal: Remote::default(),
            comparison: Remote::default(),
            local: Vec::new(),
            live_snapshot: None,
            planning_job: None,
            planning_candidates: Vec::new(),
            planning_error: None,
            planning_message: None,
            planning_starting: false,
            planning_analysis_pending: false,
            compose_refs: Default::default(),
            saving_candidate: None,
            import_handler: None,
            preview_handler: None,
            cache_preview_handler: None,
            preview_busy: None,
            preview_error: None,
            selected_id: None,
            review_busy: None,
            review_error: None,
            space_busy: None,
            space_feedback: None,
            linking_observation: None,
            link_busy: None,
            placement_editor: None,
            placement_busy: None,
            palette: None,
        };
        if page.source.is_some() {
            cx.spawn(async move |weak, cx| {
                let _ = weak.update(cx, |page, cx| {
                    page.refresh(cx);
                    page.refresh_model(cx);
                    page.refresh_layouts(cx);
                    page.refresh_spaces(cx);
                    page.refresh_inventory(cx);
                });
            })
            .detach();
        }
        page
    }

    pub fn set_source(&mut self, source: Option<RoomPlanSource>, cx: &mut Context<Self>) {
        self.source = source;
        if self.source.is_some() {
            self.refresh(cx);
            self.refresh_model(cx);
            self.refresh_layouts(cx);
            self.refresh_spaces(cx);
            self.refresh_inventory(cx);
        } else {
            self.revisions = Remote::default();
            self.model = Remote::default();
            self.layouts = Remote::default();
            self.layout_preview = Remote::default();
            self.selected_layout_id = None;
            self.layout_preview_id = None;
            self.spaces = Remote::default();
            self.inventory = Remote::default();
            self.placements = Remote::default();
            self.placement_proposal = Remote::default();
            self.placement_editor = None;
            self.planning_job = None;
            self.planning_candidates.clear();
            self.planning_error = None;
            self.planning_message = None;
            self.planning_starting = false;
            self.compose_refs.clear();
            self.saving_candidate = None;
            cx.notify();
        }
    }

    pub fn set_palette(&mut self, palette: Palette, cx: &mut Context<Self>) {
        if self.palette != Some(palette) {
            self.palette = Some(palette);
            cx.notify();
        }
    }

    pub fn set_local_archive(&mut self, local: Vec<RoomPlanLocalRecord>, cx: &mut Context<Self>) {
        self.local = local;
        cx.notify();
    }

    pub fn set_live_snapshot(&mut self, snapshot: RoomPlanLiveUpdate, cx: &mut Context<Self>) {
        self.live_snapshot = Some(snapshot);
        cx.notify();
    }

    pub fn set_preview_handler(&mut self, handler: Option<fn(&str)>) {
        self.preview_handler = handler;
    }

    pub fn set_import_handler(&mut self, handler: Option<fn()>) {
        self.import_handler = handler;
    }

    pub fn set_cache_preview_handler(&mut self, handler: Option<fn(&str, &str, Vec<u8>)>) {
        self.cache_preview_handler = handler;
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(source) = self.source.clone() else {
            self.revisions.settle(Err(ShellError::Unreachable));
            cx.notify();
            return;
        };
        self.refresh_layouts(cx);
        if !self.revisions.begin() {
            return;
        }
        cx.notify();
        let work = shell::run(async move { source.revisions().await });
        cx.spawn(async move |weak, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = weak.update(cx, |page, cx| {
                page.revisions.settle(result);
                if let Some(selected) = &page.selected_id {
                    if !page.contains_revision(selected) {
                        page.selected_id = None;
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn refresh_model(&mut self, cx: &mut Context<Self>) {
        let Some(source) = self.source.clone() else {
            self.model.settle(Err(ShellError::Unreachable));
            cx.notify();
            return;
        };
        if !self.model.begin() {
            return;
        }
        cx.notify();
        let work = shell::run(async move { source.model().await });
        cx.spawn(async move |weak, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = weak.update(cx, |page, cx| {
                page.model.settle(result);
                page.refresh_placements(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn refresh_layouts(&mut self, cx: &mut Context<Self>) {
        let Some(source) = self.source.clone() else {
            return;
        };
        if !self.layouts.begin() {
            return;
        }
        cx.notify();
        let work = shell::run(async move { source.layouts().await });
        cx.spawn(async move |weak, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = weak.update(cx, |page, cx| {
                page.layouts.settle(result);
                if let Some(id) = page.selected_layout_id.clone() {
                    if let Some(layouts) = page.layouts.value.as_ref() {
                        if layouts.iter().any(|layout| layout.id == id) {
                            page.load_layout_preview(id, cx);
                        } else {
                            page.selected_layout_id = None;
                            page.layout_preview_id = None;
                            page.layout_preview = Remote::default();
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn load_layout_preview(&mut self, id: String, cx: &mut Context<Self>) {
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

    fn select_layout(&mut self, id: Option<String>, cx: &mut Context<Self>) {
        self.selected_layout_id = id.clone();
        if let Some(id) = id {
            self.load_layout_preview(id, cx);
        } else {
            self.layout_preview = Remote::default();
            self.layout_preview_id = None;
            cx.notify();
        }
    }

    fn toggle_compose_ref(&mut self, id: String, cx: &mut Context<Self>) {
        if !self.compose_refs.remove(&id) {
            self.compose_refs.insert(id);
        }
        cx.notify();
    }

    fn start_planning(&mut self, kind: PlanningKind, cx: &mut Context<Self>) {
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

    fn watch_planning_job(
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

    fn save_planning_candidate(&mut self, index: usize, cx: &mut Context<Self>) {
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

    fn refresh_spaces(&mut self, cx: &mut Context<Self>) {
        let Some(source) = self.source.clone() else {
            return;
        };
        if !self.spaces.begin() {
            return;
        }
        cx.notify();
        let work = shell::run(async move { source.spaces().await });
        cx.spawn(async move |weak, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = weak.update(cx, |page, cx| {
                page.spaces.settle(result);
                page.refresh_placements(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn refresh_inventory(&mut self, cx: &mut Context<Self>) {
        let Some(source) = self.source.clone() else {
            return;
        };
        if !self.inventory.begin() {
            return;
        }
        cx.notify();
        let work = shell::run(async move { source.inventory_rows().await });
        cx.spawn(async move |weak, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = weak.update(cx, |page, cx| {
                page.inventory.settle(result);
                cx.notify();
            });
        })
        .detach();
    }

    fn refresh_placements(&mut self, cx: &mut Context<Self>) {
        let Some(source) = self.source.clone() else {
            return;
        };
        let flat = self
            .spaces
            .value
            .as_ref()
            .map(|spaces| spaces.flat.clone())
            .filter(|flat| !flat.is_empty())
            .or_else(|| self.model.value.as_ref().map(|model| model.flat.clone()));
        let Some(flat) = flat else {
            return;
        };
        if flat.is_empty() || !self.placements.begin() {
            return;
        }
        cx.notify();
        let work = shell::run(async move { source.placements(&flat).await });
        cx.spawn(async move |weak, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = weak.update(cx, |page, cx| {
                page.placements.settle(result);
                cx.notify();
            });
        })
        .detach();
    }

    fn assign_selected_to_space(&mut self, space_id: Option<String>, cx: &mut Context<Self>) {
        let Some(source) = self.source.clone() else {
            return;
        };
        let Some(draft) = self.selected_draft().cloned() else {
            return;
        };
        let revision_id = draft.draft_id;
        if !self.revisions.value.as_ref().is_some_and(|set| {
            set.revisions
                .iter()
                .any(|scan| scan.draft_id == revision_id)
        }) || self.space_busy.is_some()
        {
            return;
        }
        self.space_busy = Some(revision_id.clone());
        self.space_feedback = None;
        cx.notify();
        let work = shell::run(async move {
            let feedback = if let Some(space_id) = space_id {
                source
                    .add_revision_to_space(&space_id, &revision_id)
                    .await?;
                "Scan added to room history".to_owned()
            } else {
                let receipt = source.create_space(&revision_id).await?;
                format!("Room history started · {}", receipt.label)
            };
            let spaces = source.spaces().await?;
            Ok::<_, ShellError>((feedback, spaces))
        });
        cx.spawn(async move |weak, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = weak.update(cx, |page, cx| {
                page.space_busy = None;
                match result {
                    Ok((feedback, spaces)) => {
                        page.spaces.settle(Ok(spaces));
                        page.refresh_placements(cx);
                        page.space_feedback = Some(feedback);
                    }
                    Err(error) => page.space_feedback = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn toggle_link_options(&mut self, observation_id: String, cx: &mut Context<Self>) {
        if self.linking_observation.as_deref() == Some(&observation_id) {
            self.linking_observation = None;
        } else {
            self.linking_observation = Some(observation_id);
        }
        cx.notify();
    }

    fn link_observation(
        &mut self,
        space_id: String,
        revision_id: String,
        observation_id: String,
        item_id: String,
        cx: &mut Context<Self>,
    ) {
        let Some(source) = self.source.clone() else {
            return;
        };
        if self.link_busy.is_some() {
            return;
        }
        self.link_busy = Some(observation_id.clone());
        self.space_feedback = None;
        cx.notify();
        let work = shell::run(async move {
            source
                .link_object(&space_id, &revision_id, &observation_id, &item_id)
                .await?;
            let spaces = source.spaces().await?;
            Ok::<_, ShellError>(spaces)
        });
        cx.spawn(async move |weak, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = weak.update(cx, |page, cx| {
                page.link_busy = None;
                match result {
                    Ok(spaces) => {
                        page.spaces.settle(Ok(spaces));
                        page.refresh_placements(cx);
                        page.linking_observation = None;
                        page.space_feedback = Some("Furniture match confirmed".into());
                    }
                    Err(error) => page.space_feedback = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn edit_placement(
        &mut self,
        space_id: String,
        revision_id: String,
        observation_id: String,
        item_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.placement_editor.as_ref().is_some_and(|editor| {
            editor.item_id == item_id && editor.observation_id == observation_id
        }) {
            self.placement_editor = None;
            self.placement_proposal = Remote::default();
            cx.notify();
            return;
        }
        let placement = self
            .placements
            .value
            .as_ref()
            .and_then(|rows| rows.iter().find(|placement| placement.item_id == item_id));
        let (expected_revision, x, y, rot) = placement
            .map(|placement| (placement.revision, placement.x, placement.y, placement.rot))
            .unwrap_or((0, 0, 0, 0));
        let current_position = placement.map(|placement| (placement.x, placement.y, placement.rot));
        let x_input = cx.new(|cx| InputState::new(window, cx).placeholder("x · cm"));
        let y_input = cx.new(|cx| InputState::new(window, cx).placeholder("y · cm"));
        let rot_input = cx.new(|cx| InputState::new(window, cx).placeholder("rotation · degrees"));
        x_input.update(cx, |input, cx| input.set_value(x.to_string(), window, cx));
        y_input.update(cx, |input, cx| input.set_value(y.to_string(), window, cx));
        rot_input.update(cx, |input, cx| input.set_value(rot.to_string(), window, cx));
        self.placement_editor = Some(PlacementEditor {
            space_id: space_id.clone(),
            revision_id: revision_id.clone(),
            observation_id: observation_id.clone(),
            item_id: item_id.clone(),
            expected_revision,
            current_position,
            x: x_input,
            y: y_input,
            rot: rot_input,
        });
        self.placement_proposal = Remote::new_loading();
        self.space_feedback = None;
        cx.notify();
        if let Some(source) = self.source.clone() {
            let proposal_space_id = space_id;
            let proposal_revision_id = revision_id;
            let proposal_observation_id = observation_id;
            let proposal_item_id = item_id;
            let target = (
                proposal_space_id.clone(),
                proposal_revision_id.clone(),
                proposal_observation_id.clone(),
                proposal_item_id.clone(),
            );
            let work = shell::run(async move {
                source
                    .propose_placement(
                        &proposal_space_id,
                        &proposal_revision_id,
                        &proposal_observation_id,
                        &proposal_item_id,
                    )
                    .await
            });
            cx.spawn(async move |weak, cx| {
                let result = work.await.unwrap_or(Err(ShellError::Unreachable));
                let _ = weak.update(cx, |page, cx| {
                    let current = page.placement_editor.as_ref().is_some_and(|editor| {
                        editor.space_id == target.0
                            && editor.revision_id == target.1
                            && editor.observation_id == target.2
                            && editor.item_id == target.3
                    });
                    if current {
                        page.placement_proposal.settle(result);
                        cx.notify();
                    }
                });
            })
            .detach();
        }
    }

    fn use_scan_position(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(editor), Some(response)) = (
            self.placement_editor.as_ref(),
            self.placement_proposal.value.as_ref(),
        ) else {
            return;
        };
        let proposal = &response.proposal;
        if response.space_id != editor.space_id
            || response.revision_id != editor.revision_id
            || response.observation_id != editor.observation_id
            || response.item_id != editor.item_id
        {
            return;
        }
        editor.x.update(cx, |input, cx| {
            input.set_value(proposal.x.to_string(), window, cx)
        });
        editor.y.update(cx, |input, cx| {
            input.set_value(proposal.y.to_string(), window, cx)
        });
        editor.rot.update(cx, |input, cx| {
            input.set_value(proposal.rot.to_string(), window, cx)
        });
        self.space_feedback = Some(
            "Scan-derived position copied into the review fields. Check it before saving.".into(),
        );
        cx.notify();
    }

    fn commit_reviewed_placement(&mut self, cx: &mut Context<Self>) {
        let Some(source) = self.source.clone() else {
            return;
        };
        let Some(editor) = self.placement_editor.clone() else {
            return;
        };
        if self.placement_busy.is_some() {
            return;
        }
        let parse = |field: &Entity<InputState>, name: &str, cx: &Context<Self>| {
            field
                .read(cx)
                .value()
                .trim()
                .parse::<i32>()
                .map_err(|_| format!("Enter a whole-number {name} value."))
        };
        let x = match parse(&editor.x, "x", cx) {
            Ok(value) => value,
            Err(error) => {
                self.space_feedback = Some(error);
                cx.notify();
                return;
            }
        };
        let y = match parse(&editor.y, "y", cx) {
            Ok(value) => value,
            Err(error) => {
                self.space_feedback = Some(error);
                cx.notify();
                return;
            }
        };
        let rot = match parse(&editor.rot, "rotation", cx) {
            Ok(value) => value,
            Err(error) => {
                self.space_feedback = Some(error);
                cx.notify();
                return;
            }
        };
        self.placement_busy = Some(editor.item_id.clone());
        self.space_feedback = Some("Checking the proposed placement…".into());
        cx.notify();
        let work = shell::run(async move {
            source
                .apply_placement(
                    &editor.space_id,
                    &editor.item_id,
                    &editor.revision_id,
                    &editor.observation_id,
                    editor.expected_revision,
                    x,
                    y,
                    rot,
                )
                .await
        });
        cx.spawn(async move |weak, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = weak.update(cx, |page, cx| {
                page.placement_busy = None;
                match result {
                    Ok(_) => {
                        page.placement_editor = None;
                        page.placement_proposal = Remote::default();
                        page.space_feedback = Some("Reviewed placement saved.".into());
                        page.placements = Remote::default();
                        page.spaces = Remote::default();
                        page.refresh_spaces(cx);
                    }
                    Err(error) => {
                        let message = error.to_string();
                        page.space_feedback = Some(message.clone());
                        if message.contains("placement revision conflict") {
                            page.placement_editor = None;
                            page.placements = Remote::default();
                            page.refresh_placements(cx);
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn contains_revision(&self, id: &str) -> bool {
        self.revisions
            .value
            .as_ref()
            .is_some_and(|set| set.revisions.iter().any(|draft| draft.draft_id == id))
            || self.local.iter().any(|record| record.draft.draft_id == id)
    }

    fn space_for_revision(&self, id: &str) -> Option<&RoomPlanSpaceEntry> {
        self.spaces.value.as_ref()?.spaces.iter().find(|entry| {
            entry
                .space
                .revision_ids
                .iter()
                .any(|revision_id| revision_id == id)
        })
    }

    fn select(&mut self, id: String, cx: &mut Context<Self>) {
        self.selected_id = Some(id);
        self.review_error = None;
        self.preview_error = None;
        self.comparison = Remote::default();
        cx.notify();
    }

    fn preview_remote(&mut self, revision_id: String, cx: &mut Context<Self>) {
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

    fn previous_remote_draft(&self, current: &RoomPlanDraft) -> Option<&RoomPlanDraft> {
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

    fn compare_previous(&mut self, cx: &mut Context<Self>) {
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

    fn review(&mut self, decision: RoomPlanDecision, cx: &mut Context<Self>) {
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

    fn selected_draft(&self) -> Option<&RoomPlanDraft> {
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

    fn render_timeline(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
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

    fn render_layout_choices(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
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

    fn render_planning(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
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

    fn render_measured_plan(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        if self.source.is_none() {
            return div()
                .p_4()
                .rounded_lg()
                .bg(rgba(tint(palette.raised, 0x36)))
                .text_sm()
                .text_color(rgb(palette.muted))
                .child("Connect to the Mac to load this room’s measured plan.")
                .into_any_element();
        }
        let Some(model) = self.model.value.as_ref() else {
            let message = self
                .model
                .error
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_else(|| "Loading measured room…".to_owned());
            return div()
                .p_4()
                .rounded_lg()
                .bg(rgba(tint(palette.raised, 0x36)))
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(palette.muted))
                        .child(message),
                )
                .child(
                    Button::new("roomplan-model-refresh")
                        .small()
                        .ghost()
                        .label(if self.model.loading {
                            "Loading…"
                        } else {
                            "Retry"
                        })
                        .disabled(self.model.loading)
                        .on_click(cx.listener(|page, _, _, cx| page.refresh_model(cx))),
                )
                .into_any_element();
        };
        if model.polygon.len() < 3 {
            return div()
                .p_4()
                .text_sm()
                .text_color(rgb(palette.amber))
                .child("The measured model has no room polygon to draw.")
                .into_any_element();
        }

        let placements = self.placements.value.clone().unwrap_or_default();
        let inventory = self.inventory.value.clone().unwrap_or_default();
        let preview = self
            .selected_layout_id
            .as_ref()
            .filter(|id| self.layout_preview_id.as_ref() == Some(id))
            .and_then(|_| self.layout_preview.value.as_ref());
        let pieces = if self.selected_layout_id.is_some() {
            preview
                .map(|preview| {
                    preview
                        .layout
                        .items
                        .iter()
                        .map(|item| PlanPiece {
                            reference: item.reference.clone(),
                            x: item.x,
                            y: item.y,
                            rot: item.rot,
                            size: item.size,
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        } else {
            placements
                .iter()
                .map(|placement| PlanPiece {
                    reference: placement.item_id.clone(),
                    x: placement.x,
                    y: placement.y,
                    rot: placement.rot,
                    size: None,
                })
                .collect()
        };
        let missing_dimensions = pieces
            .iter()
            .filter(|piece| {
                piece.size.is_none()
                    && !inventory.iter().any(|row| {
                        row.item.id == piece.reference
                            && row.item.width.zip(row.item.depth).is_some()
                    })
            })
            .count();
        let displayed_count = pieces.len().saturating_sub(missing_dimensions);
        let piece_labels = pieces
            .iter()
            .map(|piece| {
                let label = inventory
                    .iter()
                    .find(|row| row.item.id == piece.reference)
                    .map(|row| row.item.label.clone())
                    .unwrap_or_else(|| piece.reference.clone());
                div()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .bg(rgba(tint(palette.raised, 0x50)))
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child(format!(
                        "{label} · x {}, y {} cm · {}°",
                        piece.x, piece.y, piece.rot
                    ))
                    .into_any_element()
            })
            .collect::<Vec<_>>();
        let draw_model = model.clone();
        let draw_palette = palette;
        let canvas = canvas(
            |bounds, _, _| bounds,
            move |bounds, _, window, _| {
                let points = &draw_model.polygon;
                let min_x = points.iter().map(|point| point[0]).min().unwrap_or(0) as f32;
                let max_x = points.iter().map(|point| point[0]).max().unwrap_or(1) as f32;
                let min_y = points.iter().map(|point| point[1]).min().unwrap_or(0) as f32;
                let max_y = points.iter().map(|point| point[1]).max().unwrap_or(1) as f32;
                let world_w = (max_x - min_x).max(1.);
                let world_h = (max_y - min_y).max(1.);
                let view_w = f32::from(bounds.size.width).max(1.);
                let view_h = f32::from(bounds.size.height).max(1.);
                let margin = 24.;
                let scale = ((view_w - margin * 2.).max(1.) / world_w)
                    .min((view_h - margin * 2.).max(1.) / world_h);
                let left = (view_w - world_w * scale) / 2.;
                let top = (view_h - world_h * scale) / 2.;
                let at = |x: f32, y: f32| {
                    bounds.origin
                        + point(
                            px(left + (x - min_x) * scale),
                            px(top + (y - min_y) * scale),
                        )
                };
                let mut floor = PathBuilder::fill();
                for (index, point) in points.iter().enumerate() {
                    let point = at(point[0] as f32, point[1] as f32);
                    if index == 0 {
                        floor.move_to(point);
                    } else {
                        floor.line_to(point);
                    }
                }
                floor.close();
                if let Ok(path) = floor.build() {
                    window.paint_path(path, rgba(tint(draw_palette.surface, 0x90)));
                }
                let mut outline = PathBuilder::stroke(px(2.));
                for (index, point) in points.iter().enumerate() {
                    let point = at(point[0] as f32, point[1] as f32);
                    if index == 0 {
                        outline.move_to(point);
                    } else {
                        outline.line_to(point);
                    }
                }
                outline.close();
                if let Ok(path) = outline.build() {
                    window.paint_path(path, rgb(draw_palette.text));
                }
                for wall in draw_model.walls.values() {
                    let mut path = PathBuilder::stroke(px(4.));
                    path.move_to(at(wall.from[0] as f32, wall.from[1] as f32));
                    path.line_to(at(wall.to[0] as f32, wall.to[1] as f32));
                    if let Ok(path) = path.build() {
                        window.paint_path(path, rgb(draw_palette.border));
                    }
                }
                for opening in &draw_model.openings {
                    let Some(wall) = draw_model.walls.get(&opening.wall) else {
                        continue;
                    };
                    let mut path = PathBuilder::stroke(px(5.));
                    if (wall.to[0] - wall.from[0]).abs() >= (wall.to[1] - wall.from[1]).abs() {
                        path.move_to(at(opening.from as f32, wall.from[1] as f32));
                        path.line_to(at(opening.to as f32, wall.from[1] as f32));
                    } else {
                        path.move_to(at(wall.from[0] as f32, opening.from as f32));
                        path.line_to(at(wall.from[0] as f32, opening.to as f32));
                    }
                    if let Ok(path) = path.build() {
                        window.paint_path(path, rgb(draw_palette.accent));
                    }
                }
                let mut paint_rect = |x0: f32, y0: f32, x1: f32, y1: f32, colour: u32| {
                    if x1 <= x0 || y1 <= y0 {
                        return;
                    }
                    let mut path = PathBuilder::fill();
                    path.move_to(at(x0, y0));
                    path.line_to(at(x1, y0));
                    path.line_to(at(x1, y1));
                    path.line_to(at(x0, y1));
                    path.close();
                    if let Ok(path) = path.build() {
                        window.paint_path(path, rgba(colour));
                    }
                };
                for fixture in &draw_model.fixed_furniture {
                    paint_rect(
                        fixture.x[0] as f32,
                        fixture.y[0] as f32,
                        fixture.x[1] as f32,
                        fixture.y[1] as f32,
                        tint(draw_palette.muted, 0x70),
                    );
                }
                for piece in &pieces {
                    let dimensions = piece.size.or_else(|| {
                        inventory
                            .iter()
                            .find(|row| row.item.id == piece.reference)
                            .and_then(|row| {
                                row.item
                                    .width
                                    .zip(row.item.depth)
                                    .map(|(width, depth)| [width, depth])
                            })
                    });
                    let Some([mut width, mut depth]) = dimensions else {
                        continue;
                    };
                    if (piece.rot.rem_euclid(180) - 90).abs() < 45 {
                        std::mem::swap(&mut width, &mut depth);
                    }
                    paint_rect(
                        piece.x as f32,
                        piece.y as f32,
                        (piece.x + width) as f32,
                        (piece.y + depth) as f32,
                        tint(draw_palette.green, 0x70),
                    );
                }
            },
        );
        let selected_summary = self.selected_layout_id.as_ref().and_then(|id| {
            self.layouts
                .value
                .as_ref()?
                .iter()
                .find(|layout| &layout.id == id)
        });
        let title = selected_summary
            .map(|layout| format!("Saved layout · {}", layout.name))
            .unwrap_or_else(|| format!("Measured plan · {}", model.flat));
        let layout_verdict = if let Some(summary) = selected_summary {
            let (label, colour) = if summary.pass {
                ("Solver passed", palette.green)
            } else if summary.hard > 0 {
                ("Hard violations", palette.red)
            } else {
                ("Warnings", palette.amber)
            };
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(div().text_xs().text_color(rgb(colour)).child(label))
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(format!("{} hard · {} soft", summary.hard, summary.soft)),
                )
                .into_any_element()
        } else {
            div().into_any_element()
        };
        let layout_load_state = if self.selected_layout_id.is_some() && preview.is_none() {
            if self.layout_preview.loading {
                div()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child("Loading layout positions…")
                    .into_any_element()
            } else if let Some(error) = &self.layout_preview.error {
                let id = self.selected_layout_id.clone();
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.red))
                            .child(error.to_string()),
                    )
                    .children(id.map(|id| {
                        Button::new("roomplan-layout-retry")
                            .small()
                            .ghost()
                            .label("Retry")
                            .on_click(cx.listener(move |page, _, _, cx| {
                                page.load_layout_preview(id.clone(), cx)
                            }))
                    }))
                    .into_any_element()
            } else {
                div().into_any_element()
            }
        } else {
            div().into_any_element()
        };
        let dimensions = format!(
            "{} × {} cm · {:.1} m²",
            model.dimensions.width, model.dimensions.depth, model.area_m2
        );
        let uncertainties = if model.unmeasured.is_empty() {
            "All model fields have recorded measurements.".to_owned()
        } else {
            format!("{} measurements still need review", model.unmeasured.len())
        };
        let legend = [
            ("Opening", palette.accent),
            ("Fixed fixture", palette.muted),
            ("Saved placement", palette.green),
        ]
        .into_iter()
        .map(|(label, colour)| {
            div()
                .flex()
                .items_center()
                .gap_1()
                .child(div().w(px(7.)).h(px(7.)).rounded_full().bg(rgb(colour)))
                .child(div().text_xs().text_color(rgb(palette.muted)).child(label))
        })
        .collect::<Vec<_>>();
        div()
            .p_4()
            .rounded_lg()
            .bg(rgba(tint(palette.raised, 0x36)))
            .border_1()
            .border_color(rgba(tint(palette.border, 0x30)))
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_base()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(title),
                    )
                    .child(div().text_xs().text_color(rgb(palette.muted)).child(dimensions)),
            )
            .child(self.render_layout_choices(palette, cx))
            .child(layout_verdict)
            .child(layout_load_state)
            .child(div().flex().items_center().gap_4().children(legend))
            .child(
                div()
                    .h(px(300.))
                    .w_full()
                    .rounded_md()
                    .overflow_hidden()
                    .bg(rgba(tint(palette.surface, 0x60)))
                    .child(canvas),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_1()
                    .children(piece_labels),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child(format!(
                        "Measured outline · {} vertices · {} openings · {} fixed fixtures · {} pieces shown · {} without dimensions · {uncertainties}",
                        model.polygon.len(), model.openings.len(), model.fixed_furniture.len(), displayed_count, missing_dimensions
                    )),
            )
            .into_any_element()
    }

    fn render_selected(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
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

    fn render_comparison(
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

impl Render for RoomPlanHistory {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self
            .palette
            .unwrap_or_else(|| Palette::for_appearance(WindowAppearance::Dark));
        let count = self
            .revisions
            .value
            .as_ref()
            .map(|set| set.revisions.len())
            .unwrap_or(0)
            + self
                .local
                .iter()
                .filter(|local| {
                    !self.revisions.value.as_ref().is_some_and(|set| {
                        set.revisions
                            .iter()
                            .any(|remote| remote.draft_id == local.draft.draft_id)
                    })
                })
                .count();
        let live_snapshot = self.live_snapshot.as_ref().map(|snapshot| {
            let state = match snapshot.state.as_str() {
                "scanning" => "Scanning now",
                "building" => "Preparing scan",
                "saved" => "Latest capture",
                "cancelled" => "Capture cancelled",
                "failed" => "Capture failed",
                _ => "RoomPlan snapshot",
            };
            div()
                .p_3()
                .rounded_md()
                .bg(rgba(tint(palette.accent, 0x14)))
                .flex()
                .flex_col()
                .gap_1()
                .child(div().text_sm().font_weight(gpui::FontWeight::MEDIUM).child(state))
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(format!(
                            "{} surfaces · {} openings · {} objects · +{} added · {} changed · {} removed",
                            snapshot.room.surfaces.len(),
                            snapshot.room.openings.len(),
                            snapshot.room.objects.len(),
                            snapshot.added_count,
                            snapshot.changed_count,
                            snapshot.removed_count,
                        )),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(snapshot.guidance.clone()),
                )
        });
        div()
            .id("roomplan-history")
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
                            .text_lg()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child("Room scans"),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .children(self.import_handler.map(|_| {
                                Button::new("roomplan-import-legacy")
                                    .small()
                                    .ghost()
                                    .label("Import from Files")
                                    .on_click(cx.listener(|page, _, _, _| {
                                        if let Some(import) = page.import_handler {
                                            import();
                                        }
                                    }))
                                    .into_any_element()
                            }))
                            .child(
                                Button::new("roomplan-history-refresh")
                                    .small()
                                    .ghost()
                                    .label(if self.revisions.loading {
                                        "Loading…"
                                    } else {
                                        "Refresh"
                                    })
                                    .disabled(self.revisions.loading)
                                    .on_click(cx.listener(|page, _, _, cx| page.refresh(cx))),
                            ),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child(format!("{count} saved revisions · chronological history")),
            )
            .children(self.import_handler.map(|_| {
                div()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child("For legacy scans, open the old Sjel app once, then choose On My iPhone > Sjel > RoomPlan-GPUI-Migration here.")
                    .into_any_element()
            }))
            .children(live_snapshot)
            .child(self.render_planning(palette, cx))
            .child(self.render_measured_plan(palette, cx))
            .child(self.render_timeline(palette, cx))
            .child(self.render_selected(palette, cx))
    }
}
