//! GPUI map over the travel tile cache and its shared camera.

use std::{collections::BTreeSet, path::PathBuf};

use gpui::{
    canvas, div, img, point, prelude::*, px, rgb, rgba, Context, MouseButton, PathBuilder,
    SharedString, Task, Window,
};

use super::{
    emit, feed, fit, load_tiles, publish, resize, select, state, travel, zoom, Fingers, MapEvent,
    PlaceCategory, Snapshot, TravelRoute,
};
use crate::{
    ely::buttons::{Button, ButtonVariant},
    theme::ThemeContext,
};
use sjel_gpui_core::travel::TripSection;

pub fn pinch(delta: f64, point: (f32, f32)) {
    if !delta.is_finite() || delta == 0. {
        return;
    }
    {
        let mut map = state();
        let camera = map.snap.camera;
        let point = (point.0 - map.origin.0, point.1 - map.origin.1);
        if !(0. ..camera.size.0).contains(&point.0) || !(0. ..camera.size.1).contains(&point.1) {
            return;
        }
        let (center, zoom) = travel::zoom_about(
            camera.center,
            camera.zoom,
            point,
            camera.size,
            delta,
            travel::MIN_ZOOM,
        );
        map.snap.camera.center = center;
        map.snap.camera.zoom = zoom;
        map.snap.fitted = true;
        publish(&map);
    }
    load_tiles();
}

pub struct PhoneMap {
    snap: Snapshot,
    drag: Option<(f32, f32)>,
    _updates: Task<()>,
}

impl PhoneMap {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let mut source = feed().tx.subscribe();
        let snap = source.borrow_and_update().clone();
        let updates = cx.spawn(async move |view, cx| {
            while source.changed().await.is_ok() {
                let snapshot = source.borrow_and_update().clone();
                if view
                    .update(cx, |view, cx| {
                        view.snap = snapshot;
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
            drag: None,
            _updates: updates,
        }
    }
}

impl Render for PhoneMap {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = ThemeContext::of(window, cx).palette;
        let camera = self.snap.camera;
        let mut tiles = BTreeSet::new();
        for tile in camera.wanted() {
            for level in (1..=4).rev() {
                if let Some(parent) = tile
                    .ancestor(level)
                    .filter(|parent| self.snap.tiles.contains_key(parent))
                {
                    tiles.insert(parent);
                }
            }
            if self.snap.tiles.contains_key(&tile) {
                tiles.insert(tile);
            }
        }
        let lines = self.snap.scene.connections.clone();
        let map = div()
            .id("travel-map")
            .relative()
            .flex_1()
            .min_h(px(160.))
            .overflow_hidden()
            .bg(rgba(palette.raised))
            .children(tiles.into_iter().map(|tile| {
                let placed = tile.place(camera.center, camera.zoom, camera.size.0, camera.size.1);
                img(PathBuf::from(&self.snap.tiles[&tile]))
                    .absolute()
                    .left(px(placed.x))
                    .top(px(placed.y))
                    .size(px(placed.size))
            }))
            .child(
                canvas(
                    |bounds, _, _| {
                        state().origin = (bounds.origin.x.into(), bounds.origin.y.into());
                        resize((f32::from(bounds.size.width), f32::from(bounds.size.height)));
                    },
                    move |bounds, (), window, _| {
                        let world = (256. * 2_f64.powf(camera.zoom)) as f32;
                        for &(from, to) in &lines {
                            let a = camera.point(from);
                            let b = camera.point(to);
                            let dx = (b.0 - a.0 + world / 2.).rem_euclid(world) - world / 2.;
                            let mut line = PathBuilder::stroke(px(2.));
                            line.move_to(bounds.origin + point(px(a.0), px(a.1)));
                            line.line_to(bounds.origin + point(px(a.0 + dx), px(b.1)));
                            if let Ok(path) = line.build() {
                                window.paint_path(path, rgb(palette.accent));
                            }
                        }
                    },
                )
                .absolute()
                .size_full(),
            )
            .children(self.snap.markers().filter_map(|(index, marker)| {
                let (x, y) = camera.point(marker.place.coordinate()?);
                Some(
                    div()
                        .id(("map-marker", index))
                        .absolute()
                        .left(px(x - 14.))
                        .top(px(y - 14.))
                        .size(px(28.))
                        .rounded_full()
                        .bg(rgb(palette.accent))
                        .text_color(gpui::white())
                        .flex()
                        .items_center()
                        .justify_center()
                        .child((index + 1).to_string())
                        .on_click(move |_, _, _| select(index)),
                )
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, event: &gpui::MouseDownEvent, _, _| {
                    view.drag = Some((event.position.x.into(), event.position.y.into()));
                }),
            )
            .on_mouse_move(cx.listener(|view, event: &gpui::MouseMoveEvent, _, _| {
                if !event.dragging() {
                    return;
                }
                let at = (event.position.x.into(), event.position.y.into());
                if let Some(before) = view.drag.replace(at) {
                    pan(before, at);
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|view, _, _, _| {
                    if view.drag.take().is_some() {
                        load_tiles();
                    }
                }),
            )
            .on_scroll(|event, _, cx| {
                let delta = event.delta.pixel_delta(px(28.));
                pan((0., 0.), (delta.x.into(), delta.y.into()));
                load_tiles();
                cx.stop_propagation();
            });

        let scene = &self.snap.scene;
        let status = self
            .snap
            .tile_error
            .as_deref()
            .or(scene.error.as_deref())
            .unwrap_or(if self.snap.loading || scene.loading {
                "Loading map…"
            } else {
                &scene.status
            });
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_1()
            .p_3()
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_1()
                    .child(
                        button("Trips")
                            .on_click(|_, _, _| emit(MapEvent::Open(TravelRoute::Trips))),
                    )
                    .child(div().flex_1().child(scene.title.clone()))
                    .child(button("Refresh").on_click(|_, _, _| emit(MapEvent::Refresh))),
            )
            .child(
                div().flex().flex_wrap().children(
                    [
                        None,
                        Some(PlaceCategory::Stay),
                        Some(PlaceCategory::Station),
                        Some(PlaceCategory::Airport),
                        Some(PlaceCategory::Place),
                    ]
                    .map(|category| {
                        button(category.map_or("All places", |category| category.label()))
                            .variant(if self.snap.category == category {
                                ButtonVariant::Primary
                            } else {
                                ButtonVariant::Ghost
                            })
                            .on_click(move |_, _, _| {
                                let mut map = state();
                                map.snap.category = category;
                                map.snap.selected = None;
                                publish(&map);
                            })
                    }),
                ),
            )
            .child(map)
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .child(button("Zoom out").on_click(|_, _, _| zoom(-1.)))
                    .child(button("Zoom in").on_click(|_, _, _| zoom(1.)))
                    .child(button("Fit").on_click(|_, _, _| fit()))
                    .child(
                        button(if self.snap.online {
                            "Online tiles"
                        } else {
                            "Cached only"
                        })
                        .on_click(|_, _, _| {
                            {
                                let mut map = state();
                                map.snap.online = !map.snap.online;
                                publish(&map);
                            }
                            load_tiles();
                        }),
                    ),
            )
            .child(
                button("© OpenStreetMap contributors")
                    .on_click(|_, _, cx| cx.open_url("https://www.openstreetmap.org/copyright")),
            )
            .child(div().text_sm().child(status.to_owned()))
            .when(self.snap.tile_error.is_some(), |screen| {
                screen.child(button("Retry map").on_click(|_, _, _| load_tiles()))
            })
            .child(
                div()
                    .id("map-places")
                    .max_h(px(136.))
                    .overflow_y_scroll()
                    .children(self.snap.markers().map(|(index, marker)| {
                        let id = marker.plan_id.clone();
                        let resolve = id.clone();
                        let at = marker.place.coordinate();
                        let name = marker.place.name.clone();
                        div()
                            .child(
                                Button::new(
                                    ("map-place", index),
                                    format!("{}. {}", index + 1, name),
                                )
                                .variant(ButtonVariant::Ghost)
                                .on_click(move |_, _, _| select(index)),
                            )
                            .when(self.snap.selected == Some(index), |row| {
                                row.child(div().text_sm().child(format!(
                                    "{} · {} · {}–{}",
                                    marker.label,
                                    marker.category.label(),
                                    marker.from,
                                    marker.to
                                )))
                                .children(marker.place.address.clone())
                                .when_some(at, |row, at| {
                                    let url = format!(
                                        "https://maps.apple.com/?q={}&ll={:.6},{:.6}",
                                        travel::urlencode(&name),
                                        at.latitude,
                                        at.longitude
                                    );
                                    row.child(
                                        button("Open in Apple Maps")
                                            .on_click(move |_, _, cx| cx.open_url(&url)),
                                    )
                                })
                                .when(at.is_none(), |row| {
                                    row.child(button("Resolve coordinates").on_click(
                                        move |_, _, _| emit(MapEvent::Resolve(resolve.clone())),
                                    ))
                                })
                                .child(
                                    button("Open trip").on_click(move |_, _, _| {
                                        emit(MapEvent::Open(TravelRoute::Trip {
                                            id: id.clone(),
                                            section: TripSection::Overview,
                                        }))
                                    }),
                                )
                            })
                    })),
            )
    }
}

fn pan(before: (f32, f32), after: (f32, f32)) {
    let mut state = state();
    state.snap.camera = state.snap.camera.gesture(
        Fingers {
            ids: vec![0],
            center: before,
            distance: 0.,
        },
        Fingers {
            ids: vec![0],
            center: after,
            distance: 0.,
        },
    );
    state.snap.fitted = true;
    state.snap.selected = None;
    publish(&state);
}

fn button(label: &'static str) -> Button {
    Button::new(SharedString::from(label), label).variant(ButtonVariant::Ghost)
}
