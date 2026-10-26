//! Viewport rendering and cancellable tile loading for the shared travel view.
//!
//! Two rules make the map feel continuous rather than drawn block by block:
//!
//! - **Imagery is kept and transformed, not thrown away.** A wheel or pinch changes a
//!   fractional camera; the tiles already on disk are drawn at `256 · 2^(zoom − tile.zoom)`,
//!   and a tile that is not there yet is covered by its nearest cached ancestor upscaled.
//!   Requests go out only when the gesture settles (`MAP_SETTLE`).
//! - **Positions are pure functions of the camera.** `Tile::place`, `zoom_about` and the
//!   vector-frame offset in `map` all read from the camera and nothing else, so a frame can be
//!   drawn mid-gesture without any extra state to keep in step.
use super::forms::FormKind;
use super::{LocationMarker, Travel};
use futures::future::{AbortHandle, Abortable};
use futures::{stream, SinkExt, StreamExt};
use gpui::{
    canvas, div, img, point, prelude::*, px, rgb, rgba, AnyElement, Context, MouseButton,
    PathBuilder, StyledImage,
};
use gpui_kit::component::button::{Button, ButtonVariants};
use sjel_gpui_core::{
    shell,
    travel::{self, Coordinate, Tile, TravelRoute, TripSection},
};
use std::time::Duration;

/// How long a gesture must be quiet before anything is requested. Long enough to swallow a
/// wheel run or a pinch, short enough that the sharp frame feels like the gesture's own end.
const MAP_SETTLE: Duration = Duration::from_millis(140);

/// How many tile levels above the current one stay usable as placeholders.
const TILE_LEVELS_KEPT: i32 = 4;

/// Tiles kept in memory at once. At 256 points a side this is a few screens' worth at each of
/// the four kept levels, which is what stops a pan from discarding the level it just had.
const TILE_BUDGET: usize = 400;

impl Travel {
    pub(super) fn map_page(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut page = div().flex().flex_col().gap_3();
        let mut actions = div()
            .w_full()
            .flex()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap_2();
        if matches!(self.route, TravelRoute::Map) {
            actions = actions
                .children(self.form_button(
                    "location-plan-trip",
                    "Plan a trip",
                    FormKind::Trip(None),
                    cx,
                ))
                .child(
                    self.button("map-options-toggle")
                        .ghost()
                        .label(if self.map_options_open {
                            "Hide map options"
                        } else {
                            "Map options"
                        })
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.map_options_open = !view.map_options_open;
                            cx.notify();
                        })),
                );
        }
        page = page.child(actions);
        if self.map_options_open && matches!(self.route, TravelRoute::Map) {
            let mut options = div().flex().flex_wrap().items_center().gap_2();
            #[cfg(all(feature = "native-maps", target_os = "macos"))]
            {
                options = options
                    .child(div().text_xs().text_color(rgb(self.palette.muted)).child(
                        if self.vector_enabled {
                            "Vector map · OpenFreeMap"
                        } else {
                            "Raster map · OpenStreetMap"
                        },
                    ))
                    .child(
                        self.button("map-renderer")
                            .ghost()
                            .label(if self.vector_enabled {
                                "Use raster map"
                            } else {
                                "Use vector map"
                            })
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.deactivate();
                                view.vector_enabled = !view.vector_enabled;
                                view.map_error = None;
                                view.load_map(cx);
                                cx.notify();
                            })),
                    );
            }
            if !self.uses_vector_map() {
                options = options.child(
                    self.button("map-connection")
                        .ghost()
                        .label(if self.online {
                            "Online tiles"
                        } else {
                            "Cached only"
                        })
                        .tooltip(if self.online {
                            "Switch to cached tiles only"
                        } else {
                            "Allow online map tiles"
                        })
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.online = !view.online;
                            view.load_map(cx);
                            cx.notify();
                        })),
                );
            }
            page = page.child(options);
        }
        let map = self.map(cx);
        let stops = self.map_stops(cx);
        if self.density.is_mobile() {
            page = page.child(map).child(stops);
        } else {
            page = page.child(
                div()
                    .w_full()
                    .flex()
                    .items_start()
                    .gap_3()
                    .child(div().flex_1().min_w_0().child(map))
                    .child(stops),
            );
        }
        if matches!(self.route, TravelRoute::Map) {
            page = page.children(self.active_trip_form_card(cx));
        }
        if self
            .current()
            .is_some_and(|plan| !plan.connections().is_empty())
        {
            page = page.child(div().text_xs().text_color(rgb(self.palette.muted)).child(
                "Lines connect saved stage endpoints; they do not show road or rail routing.",
            ));
        }
        if !self.map_enabled {
            page = page.child(
                self.button("load-basemap")
                    .label("Load map")
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.map_enabled = true;
                        view.load_map(cx);
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(self.palette.muted))
                    .child("Loads the visible area from OpenStreetMap. Viewed tiles are cached on this device."),
            );
        }
        page.into_any_element()
    }

    fn map_stops(&self, cx: &mut Context<Self>) -> AnyElement {
        let markers: Vec<LocationMarker> = if matches!(self.route, TravelRoute::Map) {
            self.location_markers()
        } else {
            self.current()
                .map(|plan| {
                    plan.places()
                        .map(|place| LocationMarker {
                            plan_id: plan.id.clone(),
                            place: place.clone(),
                            from: plan.date_start.clone(),
                            to: plan.date_end.clone(),
                            label: format!(
                                "{} · {}–{}",
                                plan.title, plan.date_start, plan.date_end
                            ),
                        })
                        .collect()
                })
                .unwrap_or_default()
        };
        let places: Vec<_> = markers.iter().map(|marker| marker.place.clone()).collect();
        let overview = matches!(self.route, TravelRoute::Map);
        let mut panel = div()
            .flex()
            .flex_col()
            .gap_2()
            .when(self.density.is_mobile(), |panel| panel.w_full())
            .when(!self.density.is_mobile(), |panel| {
                panel.w(px(252.)).flex_none()
            })
            .p_2()
            .rounded(px(8.))
            .border_1()
            .border_color(rgba(self.palette.border))
            .bg(rgb(self.palette.surface >> 8))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(div().text_sm().child(if overview {
                        if self.selected_stop.is_some() {
                            "Selected place"
                        } else {
                            "Up next"
                        }
                    } else {
                        "Stops"
                    }))
                    .child(div().text_xs().text_color(rgb(self.palette.muted)).child(
                        if overview {
                            format!("{} locations", places.len())
                        } else {
                            places.len().to_string()
                        },
                    )),
            );
        if overview && self.selected_stop.is_none() {
            if let Some(marker) = markers.first() {
                let id = marker.plan_id.clone();
                let place = marker.place.clone();
                let mut next = div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .py_2()
                    .border_b_1()
                    .border_color(rgba(self.palette.border))
                    .child(div().text_base().child(place.name.clone()))
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(self.palette.muted))
                            .child(marker.label.clone()),
                    )
                    .child(
                        self.button("location-open-next-trip")
                            .primary()
                            .label("Open trip")
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.open_route(
                                    TravelRoute::Trip {
                                        id: id.clone(),
                                        section: TripSection::Overview,
                                    },
                                    cx,
                                )
                            })),
                    );
                if place.coordinate().is_some() {
                    next = next.child(
                        self.button("location-show-next")
                            .ghost()
                            .label("Show on map")
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.show_location(place.clone(), cx)
                            })),
                    );
                }
                panel = panel.child(next);
            }
        }
        if let Some(stop) = &self.selected_stop {
            let mut detail = div()
                .flex()
                .flex_col()
                .gap_1()
                .py_1()
                .border_b_1()
                .border_color(rgba(self.palette.border))
                .child(div().text_sm().child(stop.name.clone()))
                .when_some(stop.address.clone(), |panel, address| {
                    panel.child(
                        div()
                            .text_xs()
                            .text_color(rgb(self.palette.muted))
                            .child(address),
                    )
                })
                .when_some(stop.coordinate(), |panel, at| {
                    panel
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(self.palette.muted))
                                .child(format!("{:.5}, {:.5}", at.latitude, at.longitude)),
                        )
                        .child(
                            self.button("open-stop-map")
                                .label("Open in OpenStreetMap")
                                .on_click(move |_, _, cx| {
                                    cx.open_url(&format!(
                                        "https://www.openstreetmap.org/?mlat={}&mlon={}#map=16/{}/{}",
                                        at.latitude, at.longitude, at.latitude, at.longitude
                                    ))
                                }),
                        )
                });
            let mut linked = std::collections::HashSet::new();
            for (index, marker) in markers
                .iter()
                .filter(|marker| {
                    marker.place.name == stop.name && marker.place.coordinate() == stop.coordinate()
                })
                .filter(|marker| linked.insert(marker.plan_id.as_str()))
                .enumerate()
            {
                let id = marker.plan_id.clone();
                detail = detail.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(self.palette.muted))
                                .child(marker.label.clone()),
                        )
                        .child(
                            self.button(gpui::SharedString::from(format!("map-open-trip-{index}")))
                                .primary()
                                .label("Open trip")
                                .on_click(cx.listener(move |view, _, _, cx| {
                                    view.open_route(
                                        TravelRoute::Trip {
                                            id: id.clone(),
                                            section: TripSection::Overview,
                                        },
                                        cx,
                                    )
                                })),
                        ),
                );
            }
            if matches!(self.route, TravelRoute::Map) {
                detail = detail.children(self.form_button(
                    "map-plan-trip-here",
                    "Plan a trip here",
                    FormKind::Trip(Some(stop.name.clone())),
                    cx,
                ));
            }
            panel = panel.child(detail);
        }
        if places.is_empty() {
            panel = panel.child(
                div()
                    .text_xs()
                    .text_color(rgb(self.palette.muted))
                    .child(if matches!(self.route, TravelRoute::Map) {
                        "No current or upcoming trip locations yet. Plan a trip or review a Calendar suggestion to add one."
                    } else {
                        "No saved stops for this trip."
                    }),
            );
        }
        if overview && markers.len() > 1 {
            panel = panel.child(
                self.button("toggle-all-locations")
                    .ghost()
                    .label(if self.show_all_locations {
                        "Hide all locations".into()
                    } else {
                        format!("Show all {} locations", markers.len())
                    })
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.show_all_locations = !view.show_all_locations;
                        cx.notify();
                    })),
            );
        }
        if !overview || self.show_all_locations {
            for (index, marker) in markers.iter().enumerate() {
                panel = if overview {
                    panel.child(self.location_row(index, marker, cx))
                } else {
                    panel.child(self.place_row(index, &marker.place, cx))
                };
            }
        }
        panel.into_any_element()
    }

    fn location_row(
        &self,
        index: usize,
        marker: &LocationMarker,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let place = marker.place.clone();
        let can_show_on_map = place.coordinate().is_some();
        let map_place = place.clone();
        let plan_id = marker.plan_id.clone();
        let open_trip_id = plan_id.clone();
        let row = div()
            .flex()
            .flex_col()
            .gap_0()
            .py_2()
            .border_b_1()
            .border_color(rgba(self.palette.border))
            .child(
                self.button(gpui::SharedString::from(format!("location-map-{index}")))
                    .ghost()
                    .label(place.name.clone())
                    .on_click(cx.listener(move |view, _, _, cx| {
                        if can_show_on_map {
                            view.show_location(map_place.clone(), cx);
                        } else {
                            view.open_route(
                                TravelRoute::Trip {
                                    id: plan_id.clone(),
                                    section: TripSection::Overview,
                                },
                                cx,
                            );
                        }
                    })),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(self.palette.muted))
                    .child(marker.label.clone()),
            )
            .child(
                self.button(gpui::SharedString::from(format!(
                    "location-open-trip-{index}"
                )))
                .ghost()
                .label("Open trip")
                .on_click(cx.listener(move |view, _, _, cx| {
                    view.open_route(
                        TravelRoute::Trip {
                            id: open_trip_id.clone(),
                            section: TripSection::Overview,
                        },
                        cx,
                    )
                })),
            );
        row.into_any_element()
    }

    fn show_location(&mut self, place: travel::Place, cx: &mut Context<Self>) {
        self.center = place.coordinate();
        self.selected_stop = Some(place);
        self.fit_pending = false;
        self.zoom = 13.;
        self.load_map(cx);
        cx.notify();
    }

    /// Requests what the current camera needs, now.
    pub(super) fn load_map(&mut self, cx: &mut Context<Self>) {
        if self.center.is_none() {
            // No inferred city or device location: start with a world overview.
            self.center = Some(Coordinate {
                latitude: 0.,
                longitude: 0.,
            });
            self.zoom = 2.;
        }
        if !self.map_enabled
            || self.viewport.0 < 1.
            || self.section() != TripSection::Map
            || matches!(self.route, TravelRoute::Trips)
        {
            return;
        }
        if self.resolving && self.has_no_known_stop() {
            // The first coordinate is a moment away. Loading the world overview now would
            // spend the tile budget on twelve tiles the fitted view throws away.
            return;
        }
        #[cfg(all(feature = "native-maps", target_os = "macos"))]
        if self.vector_enabled {
            self.zoom = self.zoom.max(1.);
            self.load_vector_map(cx);
            return;
        }
        self.load_tiles(cx);
    }

    /// Requests the camera the gesture settled on, once the gesture stops changing it.
    ///
    /// A wheel run is dozens of events and a pinch is hundreds. Until it stops, the imagery
    /// already on screen is transformed to follow it (`draw_tiles`, and the vector frame in
    /// `map`), so nothing is blank and no request is spent on a camera nobody asked for.
    pub(super) fn settle_map(&mut self, cx: &mut Context<Self>) {
        self.settle_seq += 1;
        let seq = self.settle_seq;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(MAP_SETTLE).await;
            let _ = this.update(cx, |view, cx| {
                if view.settle_seq == seq {
                    view.load_map(cx);
                }
            });
        })
        .detach();
    }

    /// Zooms about the pointer, which is what a wheel or a pinch does.
    pub(super) fn zoom_by(
        &mut self,
        delta: f64,
        cursor: Option<(f32, f32)>,
        cx: &mut Context<Self>,
    ) {
        let Some(center) = self.center else {
            return;
        };
        let cursor = cursor.unwrap_or((self.viewport.0 / 2., self.viewport.1 / 2.));
        let (center, zoom) = travel::zoom_about(
            center,
            self.zoom,
            cursor,
            self.viewport,
            delta,
            self.min_zoom(),
        );
        if zoom == self.zoom {
            // Already at a limit: there is nothing to animate and nothing to request.
            return;
        }
        self.center = Some(center);
        self.zoom = zoom;
        self.selected_stop = None;
        self.fit_pending = false;
        self.settle_map(cx);
        cx.notify();
    }

    /// A scroll wheel over the map zooms it; the page scrolls when the pointer is outside.
    ///
    /// The wheel deltas are per-event and unbounded: a trackpad flick sends a long tail of
    /// small ones and a notched mouse sends one large line delta, so both are turned into a
    /// zoom delta and capped per event.
    pub(super) fn on_wheel(&mut self, event: &gpui::ScrollWheelEvent, cx: &mut Context<Self>) {
        use gpui::ScrollDelta;
        let delta = match event.delta {
            ScrollDelta::Pixels(d) => -f64::from(d.y) * 0.0035,
            ScrollDelta::Lines(d) => -f64::from(d.y) * 0.4,
        };
        if delta == 0. {
            return;
        }
        let cursor = self.cursor_in_map(event.position);
        self.zoom_by(delta.clamp(-1.5, 1.5), cursor, cx);
    }

    /// A trackpad pinch arrives as a fractional zoom, so it is fed straight in.
    pub(super) fn on_pinch(&mut self, event: &gpui::PinchEvent, cx: &mut Context<Self>) {
        if event.delta == 0. {
            return;
        }
        let delta = (1. + f64::from(event.delta)).log2();
        let cursor = self.cursor_in_map(event.position);
        self.zoom_by(delta, cursor, cx);
        if matches!(event.phase, gpui::TouchPhase::Ended) {
            self.load_map(cx);
        }
    }

    /// Pointer position in map points. The event carries window coordinates, so the map's
    /// own top-left corner, recorded by the canvas, is subtracted and the result clamped to
    /// the map: a wheel event that lands on the border must not anchor outside the image.
    fn cursor_in_map(&self, position: gpui::Point<gpui::Pixels>) -> Option<(f32, f32)> {
        let (width, height) = self.viewport;
        if width < 1. || height < 1. {
            return None;
        }
        let x = (f32::from(position.x) - self.origin.0).clamp(0., width);
        let y = (f32::from(position.y) - self.origin.1).clamp(0., height);
        Some((x, y))
    }

    fn min_zoom(&self) -> f64 {
        #[cfg(all(feature = "native-maps", target_os = "macos"))]
        if self.vector_enabled {
            return 1.;
        }
        travel::MIN_ZOOM
    }

    /// The tile level drawn at the current zoom. Fractional zoom is carried by the draw
    /// transform, not by a new level, so a pinch does not request a level per event.
    pub(super) fn tile_level(&self) -> u8 {
        self.zoom.round().clamp(0., f64::from(u8::MAX)) as u8
    }

    fn load_tiles(&mut self, cx: &mut Context<Self>) {
        let Some(center) = self.center else {
            return;
        };
        let level = self.tile_level();
        let mut wanted: Vec<Tile> =
            travel::visible_tiles(center, level, self.viewport.0, self.viewport.1)
                .into_iter()
                .map(|(tile, _, _)| tile)
                .collect();
        wanted.sort_by_key(|tile| (tile.zoom, tile.x, tile.y));
        wanted.dedup();
        // Tiles are kept, not discarded, so the level just shown stays available as the
        // placeholder for the one being loaded. Only tiles far from the current level go.
        self.prune_tiles(level);
        let missing: Vec<Tile> = wanted
            .into_iter()
            .filter(|tile| !self.tiles.contains_key(tile))
            .collect();
        self.map_error = None;
        if missing.is_empty() {
            self.map_loading = false;
            self.map_request = None;
            return;
        }
        self.map_generation += 1;
        if let Some(request) = self.map_request.take() {
            request.abort();
        }
        let (abort, registration) = AbortHandle::new_pair();
        self.map_request = Some(abort);
        let generation = self.map_generation;
        let online = self.online;
        self.map_loading = true;
        let (mut sender, mut received) = futures::channel::mpsc::channel(4);
        shell::spawn(async move {
            let _ = Abortable::new(
                async move {
                    let mut reads =
                        stream::iter(missing)
                            .map(move |tile| async move {
                                (tile, travel::load_tile(tile, online).await)
                            })
                            .buffer_unordered(4);
                    while let Some(result) = reads.next().await {
                        if sender.send(result).await.is_err() {
                            break;
                        }
                    }
                },
                registration,
            )
            .await;
        });
        cx.spawn(async move |this, cx| {
            while let Some((tile, result)) = received.next().await {
                let applied = this.update(cx, |view, cx| {
                    if view.map_generation != generation {
                        return false;
                    }
                    match result {
                        Ok(path) => {
                            view.tiles.insert(tile, path);
                        }
                        Err(error) => view.map_error = Some(error),
                    }
                    cx.notify();
                    true
                });
                if !matches!(applied, Ok(true)) {
                    return;
                }
            }
            let _ = this.update(cx, |view, cx| {
                if view.map_generation == generation {
                    view.map_loading = false;
                    view.map_request = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Keeps the tile pyramid around the current level and a bounded number of tiles overall.
    fn prune_tiles(&mut self, level: u8) {
        let level = i32::from(level);
        self.tiles
            .retain(|tile, _| (i32::from(tile.zoom) - level).abs() <= TILE_LEVELS_KEPT);
        if self.tiles.len() <= TILE_BUDGET {
            return;
        }
        let mut keep: Vec<Tile> = self.tiles.keys().copied().collect();
        keep.sort_by_key(|tile| {
            (
                (i32::from(tile.zoom) - level).abs(),
                std::cmp::Reverse(tile.zoom),
                tile.x,
                tile.y,
            )
        });
        keep.truncate(TILE_BUDGET);
        let keep: std::collections::HashSet<Tile> = keep.into_iter().collect();
        self.tiles.retain(|tile, _| keep.contains(tile));
    }

    /// Every tile to draw, coarse placeholders first so the exact level paints over them.
    ///
    /// A missing tile is covered by the nearest cached ancestor, upscaled. That is the whole
    /// difference between tiles arriving one square at a time and the map looking continuous.
    pub(super) fn tile_layers(&self) -> Vec<travel::TilePlacement> {
        let (Some(center), (width, height)) = (self.center, self.viewport) else {
            return Vec::new();
        };
        let level = self.tile_level();
        let mut coarse = Vec::new();
        let mut exact = Vec::new();
        let mut covered = std::collections::HashSet::new();
        for (tile, _, _) in travel::visible_tiles(center, level, width, height) {
            if self.tiles.contains_key(&tile) {
                exact.push(tile.place(center, self.zoom, width, height));
                continue;
            }
            for distance in 1..=TILE_LEVELS_KEPT as u8 {
                let Some(ancestor) = tile.ancestor(distance) else {
                    break;
                };
                if !self.tiles.contains_key(&ancestor) {
                    continue;
                }
                if covered.insert(ancestor) {
                    coarse.push(ancestor.place(center, self.zoom, width, height));
                }
                break;
            }
        }
        coarse.extend(exact);
        coarse
    }

    pub(super) fn map(&self, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        let entity = cx.entity().downgrade();
        let center = self.center;
        let (width, height) = self.viewport;
        let no_locations = self
            .current()
            .is_some_and(|plan| plan.places().all(|place| place.coordinate().is_none()));
        let mut map = div()
            .id("trip-map")
            .relative()
            .w_full()
            .h(px(if self.compact_map {
                260.
            } else if matches!(self.route, TravelRoute::Map) {
                if self.density.is_mobile() {
                    440.
                } else {
                    600.
                }
            } else if self.density.is_mobile() {
                320.
            } else {
                440.
            }))
            .flex_none()
            .overflow_hidden()
            .rounded(px(8.))
            .border_1()
            .border_color(rgba(palette.border))
            .bg(rgb(palette.surface >> 8))
            .child(
                canvas(
                    move |bounds, window, cx| {
                        let size = (f32::from(bounds.size.width), f32::from(bounds.size.height));
                        // Two device pixels per point covers every Mac display in use; more
                        // would only buy MapLibre frame memory past its own size bound.
                        let scale = (window.scale_factor().round() as u32).clamp(1, 2);
                        let origin = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
                        let entity = entity.clone();
                        cx.defer(move |cx| {
                            let _ = entity.update(cx, |view, cx| {
                                view.origin = origin;
                                if view.viewport != size || view.scale != scale {
                                    view.viewport = size;
                                    view.scale = scale;
                                    if view.fit_pending {
                                        view.fit_trip(cx);
                                    } else {
                                        view.load_map(cx);
                                    }
                                    cx.notify();
                                }
                            });
                        });
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .on_scroll_wheel(cx.listener(|view, event: &gpui::ScrollWheelEvent, _, cx| {
                // Zooming here is the point of the map; the page scrolls everywhere else.
                cx.stop_propagation();
                view.on_wheel(event, cx);
            }))
            .on_pinch(cx.listener(|view, event: &gpui::PinchEvent, _, cx| {
                cx.stop_propagation();
                view.on_pinch(event, cx);
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, event: &gpui::MouseDownEvent, _, _| {
                    view.drag = Some(event.position);
                    view.map_dragged = false;
                }),
            )
            .on_mouse_move(cx.listener(|view, event: &gpui::MouseMoveEvent, _, cx| {
                if !event.dragging() {
                    view.drag = None;
                    return;
                }
                if let Some(previous) = view.drag.replace(event.position) {
                    if previous == event.position {
                        return;
                    }
                    view.map_dragged = true;
                    if let Some(center) = view.center {
                        let zoom = view.zoom;
                        let (x, y) = center.world_at(zoom);
                        view.center = Some(Coordinate::from_world_at(
                            x + f64::from(f32::from(previous.x - event.position.x)),
                            y + f64::from(f32::from(previous.y - event.position.y)),
                            zoom,
                        ));
                        cx.notify();
                    }
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|view, _, _, cx| {
                    if view.drag.take().is_some() && view.map_dragged {
                        view.settle_map(cx);
                        cx.notify();
                    }
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|view, _, _, cx| {
                    if view.drag.take().is_some() && view.map_dragged {
                        view.settle_map(cx);
                        cx.notify();
                    }
                }),
            );
        if let Some(center) = center {
            #[cfg(all(feature = "native-maps", target_os = "macos"))]
            if self.vector_enabled {
                if let Some((camera, image)) =
                    self.vector.as_ref().and_then(|map| map.frame.as_ref())
                {
                    // The last frame is drawn at the *current* camera: scaled by the zoom the
                    // gesture has moved since it was rendered, and centred where its own centre
                    // projects to now. That is what makes a pinch or a pan answer immediately
                    // instead of waiting for the next 0.5 s render.
                    let scale = 2_f64.powf(self.zoom - camera.zoom);
                    let frame_center = Coordinate {
                        latitude: camera.latitude,
                        longitude: camera.longitude,
                    };
                    let (ax, ay) = frame_center.world_at(self.zoom);
                    let (bx, by) = center.world_at(self.zoom);
                    let world = 256. * 2_f64.powf(self.zoom);
                    let dx = (ax - bx + world / 2.).rem_euclid(world) - world / 2.;
                    let (frame_w, frame_h) = (
                        f64::from(camera.width) * scale,
                        f64::from(camera.height) * scale,
                    );
                    map = map.child(
                        img(image.clone())
                            .absolute()
                            .left(px((dx + f64::from(width) / 2. - frame_w / 2.) as f32))
                            .top(px((ay - by + f64::from(height) / 2. - frame_h / 2.) as f32))
                            .w(px(frame_w as f32))
                            .h(px(frame_h as f32)),
                    );
                }
            }
            if !self.uses_vector_map() {
                for placement in self.tile_layers() {
                    if let Some(path) = self.tiles.get(&placement.tile) {
                        map = map.child(
                            img(path.clone())
                                .with_fallback(|| {
                                    div()
                                        .p_2()
                                        .text_xs()
                                        .child("Map tile cannot be displayed")
                                        .into_any_element()
                                })
                                .absolute()
                                .left(px(placement.x))
                                .top(px(placement.y))
                                .size(px(placement.size)),
                        );
                    }
                }
            }
            let connections = if matches!(self.route, TravelRoute::Map) {
                Vec::new()
            } else {
                self.current()
                    .map(|plan| plan.connections())
                    .unwrap_or_default()
            };
            let zoom = self.zoom;
            map = map.child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, _| {
                        let (cx, cy) = center.world_at(zoom);
                        let world = 256. * 2_f64.powf(zoom);
                        let shortest = |dx: f64| (dx + world / 2.).rem_euclid(world) - world / 2.;
                        for (from, to) in &connections {
                            let (ax, ay) = from.world_at(zoom);
                            let (bx, by) = to.world_at(zoom);
                            let x1 = shortest(ax - cx) as f32 + width / 2.;
                            let x2 = x1 + shortest(bx - ax) as f32;
                            let y1 = (ay - cy) as f32 + height / 2.;
                            let y2 = (by - cy) as f32 + height / 2.;
                            let mut path = PathBuilder::stroke(px(2.));
                            path.move_to(bounds.origin + point(px(x1), px(y1)));
                            path.line_to(bounds.origin + point(px(x2), px(y2)));
                            if let Ok(path) = path.build() {
                                window.paint_path(path, rgb(palette.accent));
                            }
                        }
                    },
                )
                .absolute()
                .size_full(),
            );
            let markers: Vec<_> = if matches!(self.route, TravelRoute::Map) {
                self.location_markers()
            } else {
                self.current()
                    .map(|plan| {
                        plan.places()
                            .map(|place| super::LocationMarker {
                                plan_id: plan.id.clone(),
                                place: place.clone(),
                                from: plan.date_start.clone(),
                                to: plan.date_end.clone(),
                                label: plan.title.clone(),
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            };
            if !markers.is_empty() {
                let (cx_world, cy_world) = center.world_at(self.zoom);
                let world_size = 256. * 2_f64.powf(self.zoom);
                for (index, marker) in markers.iter().enumerate() {
                    let place = &marker.place;
                    let Some(at) = place.coordinate() else {
                        continue;
                    };
                    let (x, y) = at.world_at(self.zoom);
                    let dx =
                        (x - cx_world + world_size / 2.).rem_euclid(world_size) - world_size / 2.;
                    let x = dx as f32 + width / 2.;
                    let y = (y - cy_world) as f32 + height / 2.;
                    let marker_place = place.clone();
                    let selected = self.selected_stop.as_ref().is_some_and(|stop| {
                        stop.name == place.name && stop.coordinate() == place.coordinate()
                    });
                    let companions = self
                        .presence
                        .get(&super::presence_key(at, &marker.from, &marker.to))
                        .map(|p| p.known_companions);
                    let label = if matches!(self.route, TravelRoute::Map) {
                        format!(
                            "{} · {}{}",
                            marker.label,
                            place.name,
                            companions
                                .map(|n| format!(" · {n} known companions in these dates"))
                                .unwrap_or_default()
                        )
                    } else {
                        place.name.clone()
                    };
                    map = map.child(
                        div()
                            .absolute()
                            .left(px(x - 5.))
                            .top(px(y - 5.))
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(
                                self.button(gpui::SharedString::from(format!("map-stop-{index}")))
                                    .label((index + 1).to_string())
                                    .tooltip(label.clone())
                                    .when(selected, |button| button.primary())
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        view.selected_stop = Some(marker_place.clone());
                                        cx.notify();
                                    })),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .px_1()
                                    .bg(rgb(0xffffff))
                                    .text_color(rgb(0x222222))
                                    .child(label),
                            ),
                    );
                }
            }
        }
        let controls = div()
            .absolute()
            .top(px(8.))
            .left(px(8.))
            .flex()
            .gap_1()
            .p_1()
            .rounded(px(6.))
            .border_1()
            .border_color(rgba(palette.border))
            .bg(rgb(palette.surface >> 8))
            .child(
                self.button("fit-trip")
                    .label("Fit trip")
                    .primary()
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.selected_stop = None;
                        view.fit_trip(cx);
                    })),
            )
            .child(
                self.button("zoom-in")
                    .label("Zoom in")
                    .on_click(cx.listener(|view, _, _, cx| view.zoom_by(1., None, cx))),
            )
            .child(
                self.button("zoom-out")
                    .label("Zoom out")
                    .on_click(cx.listener(|view, _, _, cx| view.zoom_by(-1., None, cx))),
            );
        map = map.child(controls);
        if no_locations && self.map_error.is_none() {
            map = map.child(
                div()
                    .absolute()
                    .top(px(56.))
                    .left(px(8.))
                    .px_2()
                    .py_1()
                    .rounded(px(6.))
                    .bg(rgb(palette.surface >> 8))
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child(if self.resolving {
                        "Locating stops…"
                    } else {
                        "World view · no trip stops located"
                    }),
            );
        }
        if (self.map_loading && !self.has_imagery()) || (!self.uses_vector_map() && !self.online) {
            let status = if self.map_loading && !self.has_imagery() {
                if self.uses_vector_map() || self.online {
                    "Loading map…"
                } else {
                    "Loading cached tiles…"
                }
            } else {
                "Cached tiles only"
            };
            map = map.child(
                div()
                    .absolute()
                    .top(px(8.))
                    .right(px(8.))
                    .px_2()
                    .py_1()
                    .rounded(px(6.))
                    .bg(rgb(palette.surface >> 8))
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child(status),
            );
        }
        if let Some(error) = self.map_error.clone() {
            map = map.child(
                div()
                    .absolute()
                    .top(px(56.))
                    .left(px(8.))
                    .right(px(8.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .p_2()
                    .rounded(px(6.))
                    .border_1()
                    .border_color(rgba(palette.border))
                    .bg(rgb(palette.surface >> 8))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_xs()
                            .text_color(rgb(palette.amber))
                            .child(error),
                    )
                    .child(
                        self.button("retry-map")
                            .label("Retry")
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.load_map(cx);
                                cx.notify();
                            })),
                    ),
            );
        }
        map.child(
            div()
                .absolute()
                .bottom_0()
                .right_0()
                .px_1()
                .py_0()
                .bg(rgb(palette.surface >> 8))
                .text_color(rgb(palette.text))
                .text_xs()
                .flex()
                .flex_wrap()
                .child(
                    Button::new("osm-attribution")
                        .label("© OpenStreetMap contributors")
                        .ghost()
                        .min_h(px(24.))
                        .px_1()
                        .py_0()
                        .on_click(|_, _, cx| {
                            cx.open_url("https://www.openstreetmap.org/copyright")
                        }),
                )
                .when(self.uses_vector_map(), |attribution| {
                    attribution
                        .child(
                            Button::new("openmaptiles-attribution")
                                .label("OpenMapTiles")
                                .ghost()
                                .min_h(px(24.))
                                .px_1()
                                .py_0()
                                .on_click(|_, _, cx| cx.open_url("https://www.openmaptiles.org/")),
                        )
                        .child(
                            Button::new("openfreemap-attribution")
                                .label("OpenFreeMap")
                                .ghost()
                                .min_h(px(24.))
                                .px_1()
                                .py_0()
                                .on_click(|_, _, cx| cx.open_url("https://openfreemap.org/")),
                        )
                }),
        )
        .into_any_element()
    }

    fn uses_vector_map(&self) -> bool {
        #[cfg(all(feature = "native-maps", target_os = "macos"))]
        {
            self.vector_enabled
        }
        #[cfg(not(all(feature = "native-maps", target_os = "macos")))]
        {
            false
        }
    }

    /// True once there is something to look at: a vector frame, or tiles covering the view.
    /// The loading chip is for the empty case only; during a gesture the previous imagery is
    /// on screen, so announcing "Loading map…" over it would be noise.
    fn has_imagery(&self) -> bool {
        #[cfg(all(feature = "native-maps", target_os = "macos"))]
        if self.vector_enabled && self.vector.as_ref().is_some_and(|map| map.frame.is_some()) {
            return true;
        }
        !self.tile_layers().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Density;
    use gpui::{
        point, Entity, Modifiers, ScrollDelta, ScrollWheelEvent, TestAppContext, TouchPhase,
        VisualTestContext,
    };
    use std::cell::RefCell;
    use std::rc::Rc;

    /// A Map-subpage view in a window, with tile requests switched off: this exercises the
    /// input wiring, not the network. `map_enabled` false is the same state as before the
    /// operator presses Load map, and it still draws the map element and its handlers.
    fn map_view(cx: &mut TestAppContext) -> (Entity<Travel>, &mut VisualTestContext) {
        cx.update(gpui_kit::init);
        let slot: Rc<RefCell<Option<Entity<Travel>>>> = Rc::new(RefCell::new(None));
        let found = slot.clone();
        let (_, cx) = cx.add_window_view(move |window, cx| {
            let view = cx.new(|_| Travel::new(None, Density::Desktop));
            view.update(cx, |view, _| {
                // A saved trip with two known stops: the Map subpage renders the map element
                // only when there is a plan to draw.
                let plan: sjel_gpui_core::travel::Plan =
                    serde_json::from_value(serde_json::json!({
                        "id": "trip-1", "title": "Hamburg", "status": "saved",
                        "date_start": "2026-10-15", "date_end": "2026-10-18",
                        "origin": {"name": "Bonn", "latitude": 50.735, "longitude": 7.102},
                        "destinations": [{"name": "Hamburg", "latitude": 53.55, "longitude": 10.0}],
                        "stages": [], "travelers": [], "budget_cents": null
                    }))
                    .expect("plan fixture");
                view.plans = vec![plan];
                view.selected = Some("trip-1".into());
                view.route = TravelRoute::Trip {
                    id: "trip-1".into(),
                    section: TripSection::Map,
                };
                view.center = Some(Coordinate::new(50.8, 8.0).expect("coordinate"));
                view.zoom = 6.;
                // Keep the fixture camera: a pending fit would recompute it from the stops.
                view.fit_pending = false;
                view.map_enabled = false;
            });
            found.replace(Some(view.clone()));
            gpui_kit::component::Root::new(view, window, cx)
        });
        let view = slot.take().expect("the map view was built");
        // One painted frame: the map records its own bounds from the canvas, and the hit
        // boxes the wheel is dispatched against only exist after a paint.
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.run_until_parked();
        (view, cx)
    }

    /// The ground under a point on the map, which a zoom anchored there must not move.
    fn ground_at(
        center: Coordinate,
        zoom: f64,
        viewport: (f32, f32),
        at: (f32, f32),
    ) -> Coordinate {
        let (cx, cy) = center.world_at(zoom);
        Coordinate::from_world_at(
            cx + f64::from(at.0) - f64::from(viewport.0) / 2.,
            cy + f64::from(at.1) - f64::from(viewport.1) / 2.,
            zoom,
        )
    }

    #[gpui::test]
    fn the_wheel_zooms_about_the_pointer(cx: &mut TestAppContext) {
        let (view, cx) = map_view(cx);
        let (origin, viewport) = view.read_with(cx, |view, _| (view.origin, view.viewport));
        assert!(
            viewport.0 > 100. && viewport.1 > 100.,
            "the map was laid out"
        );

        let at = (viewport.0 * 0.75, viewport.1 * 0.25);
        let cursor = point(px(origin.0 + at.0), px(origin.1 + at.1));
        cx.simulate_mouse_move(cursor, None, Modifiers::none());
        let (start, start_center) = view.read_with(cx, |view, _| (view.zoom, view.center.unwrap()));
        let under_pointer = ground_at(start_center, start, viewport, at);

        // A wheel notch away from the map zooms out; the point under the pointer stays put.
        cx.simulate_event(ScrollWheelEvent {
            position: cursor,
            delta: ScrollDelta::Lines(point(0., 1.)),
            modifiers: Modifiers::none(),
            touch_phase: TouchPhase::Moved,
        });
        let (zoom, center, settled) = view.read_with(cx, |view, _| {
            (view.zoom, view.center.unwrap(), view.settle_seq)
        });
        assert!(zoom < start, "one notch down zooms out: {zoom} < {start}");
        assert!(settled > 0, "the gesture scheduled a settled request");
        let still_under = ground_at(center, zoom, viewport, at);
        assert!((still_under.latitude - under_pointer.latitude).abs() < 1e-4);
        assert!((still_under.longitude - under_pointer.longitude).abs() < 1e-4);

        // And back the other way, from the same pointer.
        cx.simulate_event(ScrollWheelEvent {
            position: cursor,
            delta: ScrollDelta::Lines(point(0., -1.)),
            modifiers: Modifiers::none(),
            touch_phase: TouchPhase::Moved,
        });
        let back = view.read_with(cx, |view, _| (view.zoom, view.center.unwrap()));
        assert!(back.0 > zoom);
        let back_under = ground_at(back.1, back.0, viewport, at);
        assert!((back_under.latitude - under_pointer.latitude).abs() < 1e-4);
        assert!((back_under.longitude - under_pointer.longitude).abs() < 1e-4);
        // Finish the settle the gesture scheduled, so nothing outlives this test.
        cx.executor().advance_clock(MAP_SETTLE);
        cx.run_until_parked();
    }

    #[gpui::test]
    fn a_pinch_zooms_in_at_the_gesture_centre(cx: &mut TestAppContext) {
        let (view, cx) = map_view(cx);
        let (origin, viewport) = view.read_with(cx, |view, _| (view.origin, view.viewport));
        let cursor = point(
            px(origin.0 + viewport.0 / 2.),
            px(origin.1 + viewport.1 / 2.),
        );
        cx.simulate_mouse_move(cursor, None, Modifiers::none());
        let start = view.read_with(cx, |view, _| view.zoom);
        cx.simulate_event(gpui::PinchEvent {
            position: cursor,
            delta: 0.25,
            modifiers: Modifiers::none(),
            phase: TouchPhase::Moved,
        });
        let zoom = view.read_with(cx, |view, _| view.zoom);
        // A quarter more scale is log2(1.25) ≈ 0.32 zoom levels, not a quarter of one.
        assert!(
            (zoom - start - 0.3219).abs() < 0.001,
            "pinch zooms in: {zoom}"
        );
        // Pinched exactly at the centre: the map's own centre must not move.
        let center = view.read_with(cx, |view, _| view.center.unwrap());
        assert!(
            (center.latitude - 50.8).abs() < 1e-4 && (center.longitude - 8.0).abs() < 1e-4,
            "a centre pinch keeps the centre: {center:?}"
        );
        // Finish the settle the gesture scheduled, so nothing outlives this test.
        cx.executor().advance_clock(MAP_SETTLE);
        cx.run_until_parked();
    }

    #[gpui::test]
    fn a_missing_tile_is_covered_by_its_cached_ancestor(cx: &mut TestAppContext) {
        let (view, cx) = map_view(cx);
        view.update(cx, |view, _| {
            view.zoom = 8.;
            view.tiles.clear();
        });
        let layers = view.read_with(cx, |view, _| view.tile_layers());
        assert!(layers.is_empty(), "nothing is cached, so nothing is drawn");

        // Only the level-6 ancestors are on disk, which is what a zoom-in from level 6 leaves.
        let ancestors: Vec<Tile> = view.read_with(cx, |view, _| {
            let (center, (width, height)) = (view.center.unwrap(), view.viewport);
            let mut ancestors: Vec<Tile> = travel::visible_tiles(center, 8, width, height)
                .into_iter()
                .filter_map(|(tile, _, _)| tile.ancestor(2))
                .collect();
            ancestors.sort_by_key(|tile| (tile.x, tile.y));
            ancestors.dedup();
            ancestors
        });
        view.update(cx, |view, _| {
            for tile in &ancestors {
                view.tiles
                    .insert(*tile, std::path::PathBuf::from("/dev/null"));
            }
        });
        let layers = view.read_with(cx, |view, _| view.tile_layers());
        assert_eq!(
            layers.len(),
            ancestors.len(),
            "one placement per ancestor, never per missing child"
        );
        for placement in &layers {
            assert!(ancestors.contains(&placement.tile));
            assert!((placement.size - 1024.).abs() < 0.001, "upscaled by four");
        }
    }

    #[gpui::test]
    fn the_zoom_controls_are_clickable(cx: &mut TestAppContext) {
        let (view, cx) = map_view(cx);
        let (origin, viewport) = view.read_with(cx, |view, _| (view.origin, view.viewport));
        let start = view.read_with(cx, |view, _| view.zoom);
        // The controls sit eight points inside the map's top-left corner: Fit trip, then
        // Zoom in, then Zoom out. Aim at the middle of the second button.
        let at = point(px(origin.0 + 8. + 130.), px(origin.1 + 8. + 18.));
        cx.simulate_mouse_move(at, None, Modifiers::none());
        cx.simulate_click(at, Modifiers::none());
        let zoom = view.read_with(cx, |view, _| view.zoom);
        assert!(
            zoom > start,
            "Zoom in raises the zoom: {zoom} > {start} (map {viewport:?})"
        );
    }

    #[gpui::test]
    fn a_wheel_outside_the_map_leaves_it_alone(cx: &mut TestAppContext) {
        let (view, cx) = map_view(cx);
        let (origin, viewport) = view.read_with(cx, |view, _| (view.origin, view.viewport));
        let before = view.read_with(cx, |view, _| (view.zoom, view.center));
        // The header above the map is part of the same window but not part of the map.
        let above = point(px(origin.0 + viewport.0 / 2.), px(origin.1 - 40.));
        cx.simulate_mouse_move(above, None, Modifiers::none());
        cx.simulate_event(ScrollWheelEvent {
            position: above,
            delta: ScrollDelta::Lines(point(0., 1.)),
            modifiers: Modifiers::none(),
            touch_phase: TouchPhase::Moved,
        });
        assert_eq!(
            view.read_with(cx, |view, _| (view.zoom, view.center)),
            before
        );
    }
}
