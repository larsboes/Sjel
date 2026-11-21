//! Room scan review, placements and layout planning in the phone GPUI window.

use std::collections::BTreeSet;

use gpui::{
    canvas, div, fill, point, prelude::*, px, rgb, size, Bounds, Context, Div, Entity, Focusable,
    FontWeight, PathBuilder, SharedString, Subscription, Task, Window,
};

use super::{
    emit, feed, load, perform, plan, publish, state, Action, InteriorEvent, PlanningKind,
    RoomPlanDecision, RoomPlanDraft, RoomPlanSavedLayoutItem, Snapshot,
};
use crate::{
    ely::{
        buttons::{Button, ButtonVariant},
        forms::{Input, TextInput},
        theme::ControlSize,
    },
    theme::{Palette, ThemeContext},
};

pub struct PhoneInterior {
    snap: Snapshot,
    layouts: bool,
    space: String,
    observation: String,
    item: String,
    expected: i64,
    coordinates: [Entity<TextInput>; 3],
    pieces: BTreeSet<String>,
    _inputs: Vec<Subscription>,
    _updates: Task<()>,
}

impl PhoneInterior {
    pub fn new(window: &mut Window, keyboard: fn(bool), cx: &mut Context<Self>) -> Self {
        let mut source = feed().tx.subscribe();
        let snap = source.borrow_and_update().clone();
        let coordinates = ["X (cm)", "Y (cm)", "Rotation (°)"]
            .map(|name| cx.new(|cx| TextInput::new(window, cx).placeholder(name)));
        let mut inputs = Vec::new();
        for field in &coordinates {
            let focus = field.read(cx).focus_handle(cx);
            inputs.push(cx.on_focus(&focus, window, move |_, _, _| keyboard(true)));
            inputs.push(cx.on_blur(&focus, window, move |_, _, _| keyboard(false)));
        }
        let updates = cx.spawn(async move |view, cx| {
            while source.changed().await.is_ok() {
                let snap = source.borrow_and_update().clone();
                if view
                    .update(cx, |view, cx| {
                        if snap.selected != view.snap.selected {
                            view.space = snap
                                .selected
                                .as_ref()
                                .and_then(|id| {
                                    snap.spaces
                                        .spaces
                                        .iter()
                                        .find(|entry| entry.space.revision_ids.contains(id))
                                })
                                .map(|entry| entry.space.space_id.clone())
                                .unwrap_or_default();
                            view.observation.clear();
                            view.item.clear();
                            view.expected = 0;
                        }
                        view.snap = snap;
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            snap,
            coordinates,
            layouts: false,
            space: String::new(),
            observation: String::new(),
            item: String::new(),
            expected: 0,
            pieces: BTreeSet::new(),
            _inputs: inputs,
            _updates: updates,
        }
    }

    fn coordinates(&self, values: [i32; 3], cx: &mut Context<Self>) {
        for (field, value) in self.coordinates.iter().zip(values) {
            field.update(cx, |field, cx| field.set_text(value.to_string(), cx));
        }
    }

    fn scans(&self) -> Div {
        column()
            .when_some(self.snap.live.as_ref(), |body, live| {
                body.child(title(live.state.clone()))
                    .child(note(live.guidance.clone()))
            })
            .children(self.snap.local.iter().map(|record| {
                select_scan(
                    record.draft.draft_id.clone(),
                    format!("{} · {}", record.draft.created_at, record.sync_state),
                )
            }))
            .children(self.snap.revisions.revisions.iter().map(|draft| {
                select_scan(
                    draft.draft_id.clone(),
                    format!(
                        "{} · {}",
                        draft.created_at,
                        draft
                            .review
                            .as_ref()
                            .map_or("Unreviewed", |review| review.decision.as_str())
                    ),
                )
            }))
    }

    fn revision(&self, id: &str, cx: &Context<Self>) -> Div {
        let local = self
            .snap
            .local
            .iter()
            .find(|record| record.draft.draft_id == id);
        let remote = self
            .snap
            .revisions
            .revisions
            .iter()
            .find(|draft| draft.draft_id == id);
        let Some(draft) = remote.or_else(|| local.map(|record| &record.draft)) else {
            return column().child("This scan is no longer in the archive.");
        };
        let local_id = id.to_owned();
        let review_id = id.to_owned();
        let space_id = id.to_owned();
        let mut body = column()
            .child(button("Back to scans").on_click(|_, _, _| choose_scan(None)))
            .child(title(draft.draft_id.clone()))
            .child(note(draft.created_at.clone()))
            .when(local.is_some(), |body| {
                body.child(
                    button("Preview local model")
                        .on_click(move |_, _, _| emit(InteriorEvent::Preview(local_id.clone()))),
                )
            });
        if let Some(remote) = remote {
            let preview = remote.clone();
            body = body
                .child(
                    button("Preview model from Mac").on_click(move |_, _, _| {
                        perform(Action::Preview(Box::new(preview.clone())))
                    }),
                )
                .child(
                    div().flex().children(
                        [
                            ("Accept", RoomPlanDecision::Accept),
                            ("Reject", RoomPlanDecision::Reject),
                        ]
                        .map(|(name, decision)| {
                            let id = review_id.clone();
                            button(name)
                                .disabled(self.snap.busy)
                                .on_click(move |_, _, _| {
                                    perform(Action::Review(id.clone(), decision))
                                })
                        }),
                    ),
                )
                .when_some(remote.review.as_ref(), |body, review| {
                    body.child(note(format!("Review: {}", review.decision)))
                })
                .child(
                    button("Start room history")
                        .disabled(self.snap.busy)
                        .on_click(move |_, _, _| perform(Action::CreateSpace(space_id.clone()))),
                );
        }
        body.child(title("Observations & room history"))
            .children(
                draft
                    .room
                    .surfaces
                    .iter()
                    .chain(&draft.room.openings)
                    .chain(&draft.room.objects)
                    .map(|element| {
                        let chosen = element.id.clone();
                        column()
                            .child(
                                choice(
                                    format!("{} · {}", element.category, element.id),
                                    self.observation == chosen,
                                )
                                .on_click(cx.listener(
                                    move |view, _, _, cx| {
                                        view.observation = chosen.clone();
                                        cx.notify();
                                    },
                                )),
                            )
                            .when_some(
                                element
                                    .extra
                                    .get("dimensions_m")
                                    .and_then(|value| value.as_array()),
                                |body, dimensions| {
                                    body.child(note(format!(
                                        "{} m",
                                        dimensions
                                            .iter()
                                            .map(ToString::to_string)
                                            .collect::<Vec<_>>()
                                            .join(" × ")
                                    )))
                                },
                            )
                    }),
            )
            .children(self.snap.spaces.spaces.iter().map(|entry| {
                let space = entry.space.space_id.clone();
                let chosen = space.clone();
                let revision = id.to_owned();
                div()
                    .flex()
                    .flex_wrap()
                    .child(
                        choice(
                            format!("Use room · {}", entry.space.label),
                            self.space == space,
                        )
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.space = chosen.clone();
                            cx.notify();
                        })),
                    )
                    .child(
                        button(format!("Add scan to {}", entry.space.label))
                            .disabled(self.snap.busy)
                            .on_click(move |_, _, _| {
                                perform(Action::AddSpace(space.clone(), revision.clone()))
                            }),
                    )
            }))
            .children(
                self.snap
                    .revisions
                    .revisions
                    .iter()
                    .filter(|previous| previous.draft_id != id)
                    .map(|previous| {
                        let before = previous.draft_id.clone();
                        let after = id.to_owned();
                        button(format!("Compare with {}", previous.created_at))
                            .disabled(self.snap.busy)
                            .on_click(move |_, _, _| {
                                perform(Action::Compare(before.clone(), after.clone()))
                            })
                    }),
            )
            .children(self.snap.comparison.iter().flat_map(|comparison| {
                comparison.reconciliation.matches.iter().map(|pair| {
                    note(format!(
                        "{} → {} · {:.2} m · {}",
                        pair.before_observation_id,
                        pair.after_observation_id,
                        pair.distance_m,
                        pair.suggestion
                    ))
                })
            }))
            .child(self.placement(draft, cx))
    }

    fn placement(&self, draft: &RoomPlanDraft, cx: &Context<Self>) -> Div {
        let revision = draft.draft_id.clone();
        let proposal = self.snap.proposal.as_ref().filter(|p| {
            p.space_id == self.space
                && p.item_id == self.item
                && p.revision_id == revision
                && p.observation_id == self.observation
        });
        let disabled = self.snap.busy
            || self.space.is_empty()
            || self.observation.is_empty()
            || self.item.is_empty();
        let link = revision.clone();
        let propose = revision.clone();
        column()
            .child(title("Inventory placement"))
            .child(note(format!(
                "Room: {} · observation: {} · item: {}",
                self.space, self.observation, self.item
            )))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .children(self.snap.inventory.iter().map(|row| {
                        let id = row.item.id.clone();
                        choice(row.item.label.clone(), self.item == id).on_click(cx.listener(
                            move |view, _, _, cx| {
                                let placement =
                                    view.snap.placements.iter().find(|p| p.item_id == id);
                                view.coordinates(
                                    placement.map_or([0, 0, 0], |p| [p.x, p.y, p.rot]),
                                    cx,
                                );
                                view.expected = placement.map_or(0, |p| p.revision);
                                view.item = id.clone();
                                cx.notify();
                            },
                        ))
                    })),
            )
            .child(
                button("Link observation to inventory item")
                    .disabled(disabled)
                    .on_click(cx.listener(move |view, _, _, _| {
                        perform(Action::Link {
                            space: view.space.clone(),
                            revision: link.clone(),
                            observation: view.observation.clone(),
                            item: view.item.clone(),
                        })
                    })),
            )
            .child(
                button("Propose placement")
                    .disabled(disabled)
                    .on_click(cx.listener(move |view, _, _, _| {
                        perform(Action::Propose {
                            space: view.space.clone(),
                            revision: propose.clone(),
                            observation: view.observation.clone(),
                            item: view.item.clone(),
                        })
                    })),
            )
            .when_some(proposal, |body, proposal| {
                let p = &proposal.proposal;
                let values = [p.x, p.y, p.rot];
                body.child(note(format!(
                    "Proposal: {}, {} cm · {}° · alignment error {:.2} m",
                    p.x, p.y, p.rot, p.alignment_error_m
                )))
                .child(
                    button("Use proposed position")
                        .on_click(cx.listener(move |view, _, _, cx| view.coordinates(values, cx))),
                )
            })
            .children(self.coordinates.iter().map(|field| Input::new(field)))
            .child(
                button("Apply reviewed placement")
                    .disabled(disabled)
                    .on_click(cx.listener(move |view, _, window, cx| {
                        let [Ok(x), Ok(y), Ok(rot)] = view
                            .coordinates
                            .each_ref()
                            .map(|field| field.read(cx).text().trim().parse())
                        else {
                            let mut current = state();
                            current.snap.error =
                                Some("Use whole numbers for position and rotation.".into());
                            publish(&current);
                            return;
                        };
                        window.blur(cx);
                        perform(Action::Apply {
                            space: view.space.clone(),
                            revision: revision.clone(),
                            observation: view.observation.clone(),
                            item: view.item.clone(),
                            expected: view.expected,
                            x,
                            y,
                            rot,
                        });
                    })),
            )
    }

    fn layout(&self, palette: Palette, cx: &Context<Self>) -> Div {
        let toggle = |id: String, label: String| {
            choice(label, self.pieces.contains(&id)).on_click(cx.listener(move |view, _, _, cx| {
                if !view.pieces.remove(&id) {
                    view.pieces.insert(id.clone());
                }
                cx.notify();
            }))
        };
        column()
            .child(title("Layouts"))
            .children(self.snap.layouts.iter().map(|layout| {
                let id = layout.id.clone();
                button(format!(
                    "{} · {} hard, {} soft",
                    layout.name, layout.hard, layout.soft
                ))
                .disabled(self.snap.busy)
                .on_click(move |_, _, _| perform(Action::Layout(id.clone())))
            }))
            .when(self.snap.model.is_some(), |body| {
                body.child(layout_diagram(self.snap.clone(), palette))
            })
            .when_some(self.snap.model.as_ref(), |body, model| {
                body.child(note(format!(
                    "{} × {} cm · {:.2} m²",
                    model.dimensions.width, model.dimensions.depth, model.area_m2
                )))
                .children(model.unmeasured.iter().map(|entry| {
                    note(format!(
                        "Unmeasured: {} · {}",
                        entry.label,
                        entry.fields.join(", ")
                    ))
                }))
            })
            .when_some(self.snap.layout.as_ref(), |body, layout| {
                body.child(title(layout.layout.name.clone()))
                    .child(note(format!("Checks: {}", layout.check)))
                    .children(layout.layout.items.iter().map(|piece| {
                        toggle(
                            piece.reference.clone(),
                            format!(
                                "{} · {}, {} cm · {}°",
                                piece.reference, piece.x, piece.y, piece.rot
                            ),
                        )
                    }))
            })
            .child(
                div().flex().flex_wrap().children(
                    self.snap
                        .inventory
                        .iter()
                        .map(|row| toggle(row.item.id.clone(), row.item.label.clone())),
                ),
            )
            .child(note(format!(
                "Selected items: {}",
                self.pieces.iter().cloned().collect::<Vec<_>>().join(", ")
            )))
            .child(
                div().flex().flex_wrap().children(
                    [
                        ("Search positions in saved layout", PlanningKind::Search),
                        ("Compose selected inventory items", PlanningKind::Compose),
                    ]
                    .map(|(label, kind)| {
                        button(label).disabled(self.snap.busy).on_click(cx.listener(
                            move |view, _, _, _| plan(kind, view.pieces.iter().cloned().collect()),
                        ))
                    }),
                ),
            )
            .children(
                self.snap
                    .candidates
                    .iter()
                    .enumerate()
                    .map(|(index, candidate)| {
                        column()
                            .py_2()
                            .child(title(candidate.title.clone()))
                            .child(note(candidate.details.clone()))
                            .child(
                                button("Save reviewed proposal")
                                    .disabled(self.snap.busy)
                                    .on_click(move |_, _, _| perform(Action::Save(index))),
                            )
                    }),
            )
    }
}

impl Render for PhoneInterior {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = ThemeContext::of(window, cx).palette;
        let body = if self.layouts {
            self.layout(palette, cx)
        } else if let Some(id) = self.snap.selected.as_deref() {
            self.revision(id, cx)
        } else {
            self.scans()
        };
        column()
            .size_full()
            .p_3()
            .child(title("Interior").text_xl())
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .child(button("Capture room").on_click(|_, _, _| emit(InteriorEvent::Capture)))
                    .child(button("Sync scans").on_click(|_, _, _| emit(InteriorEvent::Sync)))
                    .child(
                        button("Import legacy scans")
                            .on_click(|_, _, _| emit(InteriorEvent::Import)),
                    )
                    .child(button("Refresh").on_click(|_, _, _| load())),
            )
            .child(
                div()
                    .flex()
                    .children(
                        [(false, "Scans"), (true, "Layouts")].map(|(layouts, label)| {
                            choice(label, self.layouts == layouts).on_click(cx.listener(
                                move |view, _, _, cx| {
                                    view.layouts = layouts;
                                    cx.notify();
                                },
                            ))
                        }),
                    ),
            )
            .child(note(self.snap.error.clone().unwrap_or_else(|| {
                if self.snap.busy {
                    "Working…".into()
                } else if self.snap.loading {
                    "Reading room history…".into()
                } else {
                    self.snap.status.clone()
                }
            })))
            .child(
                div()
                    .id("interior-content")
                    .min_h_0()
                    .flex_1()
                    .overflow_y_scroll()
                    .child(body),
            )
    }
}

fn layout_diagram(snapshot: Snapshot, palette: Palette) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, (), window, _| {
            let Some(model) = &snapshot.model else {
                return;
            };
            let Some(first) = model.polygon.first() else {
                return;
            };
            let (min, max) = model
                .polygon
                .iter()
                .fold((*first, *first), |(min, max), p| {
                    (
                        [min[0].min(p[0]), min[1].min(p[1])],
                        [max[0].max(p[0]), max[1].max(p[1])],
                    )
                });
            let scale = ((f32::from(bounds.size.width) - 24.).max(1.)
                / (max[0] - min[0]).max(1) as f32)
                .min(
                    (f32::from(bounds.size.height) - 24.).max(1.) / (max[1] - min[1]).max(1) as f32,
                );
            let at = |p: [i32; 2]| {
                bounds.origin
                    + point(
                        px(12. + (p[0] - min[0]) as f32 * scale),
                        px(12. + (p[1] - min[1]) as f32 * scale),
                    )
            };
            let mut polygon = PathBuilder::stroke(px(2.));
            polygon.move_to(at(*first));
            for &corner in model.polygon.iter().skip(1) {
                polygon.line_to(at(corner));
            }
            polygon.close();
            if let Ok(path) = polygon.build() {
                window.paint_path(path, rgb(palette.text));
            }
            let mut segment = |from, to, width, color| {
                let mut path = PathBuilder::stroke(px(width));
                path.move_to(at(from));
                path.line_to(at(to));
                if let Ok(path) = path.build() {
                    window.paint_path(path, rgb(color));
                }
            };
            for wall in model.walls.values() {
                segment(wall.from, wall.to, 3., palette.text);
            }
            for opening in &model.openings {
                if let Some(wall) = model.walls.get(&opening.wall) {
                    let (from, to) =
                        if (wall.to[0] - wall.from[0]).abs() >= (wall.to[1] - wall.from[1]).abs() {
                            ([opening.from, wall.from[1]], [opening.to, wall.from[1]])
                        } else {
                            ([wall.from[0], opening.from], [wall.from[0], opening.to])
                        };
                    segment(from, to, 5., palette.accent);
                }
            }
            for fixed in &model.fixed_furniture {
                let area = Bounds::new(
                    at([fixed.x[0], fixed.y[0]]),
                    size(
                        px((fixed.x[1] - fixed.x[0]) as f32 * scale),
                        px((fixed.y[1] - fixed.y[0]) as f32 * scale),
                    ),
                );
                window.paint_quad(fill(area, rgb(palette.muted)));
            }
            let pieces = snapshot
                .layout
                .as_ref()
                .map(|layout| layout.layout.items.clone())
                .unwrap_or_else(|| {
                    snapshot
                        .placements
                        .iter()
                        .map(|piece| RoomPlanSavedLayoutItem {
                            reference: piece.item_id.clone(),
                            x: piece.x,
                            y: piece.y,
                            rot: piece.rot,
                            size: None,
                            kind: None,
                        })
                        .collect()
                });
            for piece in pieces {
                if let Some([mut w, mut d]) = piece.size.or_else(|| {
                    snapshot
                        .inventory
                        .iter()
                        .find(|row| row.item.id == piece.reference)
                        .and_then(|row| row.item.width.zip(row.item.depth).map(|(w, d)| [w, d]))
                }) {
                    if (piece.rot.rem_euclid(180) - 90).abs() < 45 {
                        std::mem::swap(&mut w, &mut d);
                    }
                    let area = Bounds::new(
                        at([piece.x, piece.y]),
                        size(px(w as f32 * scale), px(d as f32 * scale)),
                    );
                    window.paint_quad(fill(area, rgb(palette.accent)));
                }
            }
        },
    )
    .w_full()
    .h(px(260.))
}

fn choose_scan(id: Option<String>) {
    let mut current = state();
    current.snap.selected = id;
    current.snap.proposal = None;
    current.snap.comparison = None;
    publish(&current);
}
fn select_scan(id: String, label: String) -> Div {
    column()
        .child(button(label).on_click({
            let id = id.clone();
            move |_, _, _| choose_scan(Some(id.clone()))
        }))
        .child(note(id))
}
fn column() -> Div {
    div().flex().flex_col().gap_2()
}
fn title(text: impl Into<SharedString>) -> Div {
    div().font_weight(FontWeight::SEMIBOLD).child(text.into())
}
fn note(text: impl Into<SharedString>) -> Div {
    div().text_sm().child(text.into())
}
fn button(label: impl Into<SharedString>) -> Button {
    choice(label, false)
}
fn choice(label: impl Into<SharedString>, active: bool) -> Button {
    let label = label.into();
    Button::new(label.clone(), label)
        .size(ControlSize::Sm)
        .variant(if active {
            ButtonVariant::Primary
        } else {
            ButtonVariant::Ghost
        })
}
