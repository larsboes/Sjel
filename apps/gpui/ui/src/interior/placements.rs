//! Spatial 3D room plan placement review, space assignment, and layout geometry.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::InputState;
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::{
    canvas, div, point, prelude::*, px, rgb, rgba, AnyElement, Context, Entity, PathBuilder, Window,
};
use sjel_gpui_core::interior::PlacementUpdate;
use sjel_gpui_core::shell::{self, ShellError};

use crate::data::Remote;
use crate::theme::{tint, Palette};

use super::RoomPlanHistory;

#[derive(Clone)]
pub(crate) struct PlacementEditor {
    pub(super) space_id: String,
    pub(super) revision_id: String,
    pub(super) observation_id: String,
    pub(super) item_id: String,
    pub(super) expected_revision: i64,
    pub(super) current_position: Option<(i32, i32, i32)>,
    pub(super) x: Entity<InputState>,
    pub(super) y: Entity<InputState>,
    pub(super) rot: Entity<InputState>,
}

#[derive(Clone)]
pub(super) struct PlanPiece {
    reference: String,
    pub(super) x: i32,
    pub(super) y: i32,
    pub(super) rot: i32,
    size: Option<[i32; 2]>,
}

impl RoomPlanHistory {
    pub(super) fn assign_selected_to_space(
        &mut self,
        space_id: Option<String>,
        cx: &mut Context<Self>,
    ) {
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

    pub(super) fn toggle_link_options(&mut self, observation_id: String, cx: &mut Context<Self>) {
        if self.linking_observation.as_deref() == Some(&observation_id) {
            self.linking_observation = None;
        } else {
            self.linking_observation = Some(observation_id);
        }
        cx.notify();
    }

    pub(super) fn link_observation(
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

    pub(super) fn edit_placement(
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

    pub(super) fn use_scan_position(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    pub(super) fn commit_reviewed_placement(&mut self, cx: &mut Context<Self>) {
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
                .apply_placement(PlacementUpdate {
                    space_id: &editor.space_id,
                    item_id: &editor.item_id,
                    revision_id: &editor.revision_id,
                    observation_id: &editor.observation_id,
                    expected_revision: editor.expected_revision,
                    x,
                    y,
                    rotation: rot,
                })
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

    pub(super) fn render_measured_plan(
        &self,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
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
            .and(self.layout_preview.value.as_ref());
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
}
