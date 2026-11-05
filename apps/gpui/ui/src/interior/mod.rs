//! Shared scan history and review for the Mac capability and paired iPhone.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::{div, prelude::*, rgb, rgba, Context, Render, Window};

use sjel_gpui_core::interior::{
    RoomPlanJob, RoomPlanLiveUpdate, RoomPlanLocalRecord, RoomPlanModel, RoomPlanPlacement,
    RoomPlanPlacementProposalResponse, RoomPlanReconciliationResponse, RoomPlanRevisionSet,
    RoomPlanSavedLayoutPreview, RoomPlanSource, RoomPlanSpaceSet,
};
use sjel_gpui_core::inventory::{InteriorLayoutSummary, InventoryRow};
use sjel_gpui_core::shell::{self, ShellError};

use crate::data::Remote;
use crate::theme::{tint, Palette, ThemeContext};

mod placements;
mod planning;
mod revisions;

use placements::PlacementEditor;
use planning::PlanningCandidate;

pub struct RoomPlanHistory {
    pub(super) source: Option<RoomPlanSource>,
    pub(super) revisions: Remote<RoomPlanRevisionSet>,
    pub(super) model: Remote<RoomPlanModel>,
    pub(super) layouts: Remote<Vec<InteriorLayoutSummary>>,
    pub(super) layout_preview: Remote<RoomPlanSavedLayoutPreview>,
    pub(super) selected_layout_id: Option<String>,
    pub(super) layout_preview_id: Option<String>,
    pub(super) spaces: Remote<RoomPlanSpaceSet>,
    pub(super) inventory: Remote<Vec<InventoryRow>>,
    pub(super) placements: Remote<Vec<RoomPlanPlacement>>,
    pub(super) placement_proposal: Remote<RoomPlanPlacementProposalResponse>,
    pub(super) comparison: Remote<RoomPlanReconciliationResponse>,
    pub(super) local: Vec<RoomPlanLocalRecord>,
    pub(super) live_snapshot: Option<RoomPlanLiveUpdate>,
    pub(super) planning_job: Option<RoomPlanJob>,
    pub(super) planning_candidates: Vec<PlanningCandidate>,
    pub(super) planning_error: Option<String>,
    pub(super) planning_message: Option<String>,
    pub(super) planning_starting: bool,
    pub(super) planning_analysis_pending: bool,
    pub(super) compose_refs: std::collections::BTreeSet<String>,
    pub(super) saving_candidate: Option<usize>,
    pub(super) import_handler: Option<fn()>,
    pub(super) preview_handler: Option<fn(&str)>,
    pub(super) cache_preview_handler: Option<fn(&str, &str, Vec<u8>)>,
    pub(super) preview_busy: Option<String>,
    pub(super) preview_error: Option<String>,
    pub(super) selected_id: Option<String>,
    pub(super) review_busy: Option<String>,
    pub(super) review_error: Option<String>,
    pub(super) space_busy: Option<String>,
    pub(super) space_feedback: Option<String>,
    pub(super) linking_observation: Option<String>,
    pub(super) link_busy: Option<String>,
    pub(super) placement_editor: Option<PlacementEditor>,
    pub(super) placement_busy: Option<String>,
    pub(super) palette: Option<Palette>,
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

}

impl Render for RoomPlanHistory {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self
            .palette
            .unwrap_or_else(|| ThemeContext::of(window, cx).palette);
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
