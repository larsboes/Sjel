//! One travel surface for desktop and phone. Network and disk work stay on the shared runtime.
use std::collections::HashMap;
use std::path::PathBuf;

use futures::future::AbortHandle;
use gpui::{point, prelude::*, px, Context, Point, Render, ScrollHandle, Window};
use gpui_kit::component::button::Button;
use sjel_gpui_core::{
    shell,
    travel::{self, Coordinate, Place, PlaceCoords, Plan, Source, Tile, TravelRoute, TripSection},
};

use crate::{Density, Palette};

mod details;
mod forms;
mod map;
#[cfg(all(feature = "native-maps", target_os = "macos"))]
mod native_renderer;
mod pages;
#[cfg(all(feature = "native-maps", target_os = "macos"))]
mod vector;

pub struct Travel {
    source: Option<Source>,
    plans: Vec<Plan>,
    selected: Option<String>,
    loading: bool,
    loaded: bool,
    status: String,
    error: Option<String>,
    center: Option<Coordinate>,
    /// Fractional on purpose: a wheel or a pinch moves it continuously, and the imagery on
    /// screen is drawn at `2^(zoom - level)` so nothing has to wait for a whole level.
    zoom: f64,
    viewport: (f32, f32),
    /// The map's top-left corner in window points, recorded by the canvas so a pointer
    /// position can be turned into a point on the map — which is what anchors a zoom.
    origin: (f32, f32),
    /// Bumped per gesture so only the last of a run requests anything.
    settle_seq: u64,
    /// Device pixels per layout point, read from the canvas. The vector map renders at this
    /// density so its labels are sharp instead of an upscaled 1x image.
    scale: u32,
    tiles: HashMap<Tile, PathBuf>,
    map_enabled: bool,
    map_loading: bool,
    map_generation: u64,
    map_request: Option<AbortHandle>,
    map_error: Option<String>,
    online: bool,
    drag: Option<Point<gpui::Pixels>>,
    palette: Palette,
    show_all: bool,
    saved_at: Option<u64>,
    fit_pending: bool,
    density: Density,
    route: TravelRoute,
    details: HashMap<String, details::DetailState>,
    /// Coordinates resolved for places the capability stored as typed, kept beside the
    /// plan snapshot so a second open needs no geocoder.
    coords: PlaceCoords,
    /// Keys already asked of the geocoder this session, so a name it cannot place is asked
    /// once rather than on every detail read.
    asked: std::collections::HashSet<String>,
    resolving: bool,
    /// A read arrived while a pass was running; the names it brought are resolved next.
    resolve_again: bool,
    selected_stop: Option<Place>,
    map_dragged: bool,
    scroll: ScrollHandle,
    /// The open trip, booking or stage form ([`forms`]).
    form: Option<forms::Form>,
    /// The row whose Remove was pressed once.
    confirm_remove: Option<String>,
    /// A remove that the Mac refused, when no form is open to say so.
    write_error: Option<String>,
    #[cfg(all(feature = "native-maps", target_os = "macos"))]
    vector: Option<vector::VectorMap>,
    #[cfg(all(feature = "native-maps", target_os = "macos"))]
    vector_enabled: bool,
}

impl Travel {
    pub fn new(source: Option<Source>, density: Density) -> Self {
        Self {
            source,
            plans: Vec::new(),
            selected: None,
            loading: false,
            loaded: false,
            status: "Refresh to load trips. Saved trips remain available without the Mac.".into(),
            error: None,
            center: None,
            zoom: 11.,
            viewport: (0., 0.),
            origin: (0., 0.),
            settle_seq: 0,
            scale: 2,
            tiles: HashMap::new(),
            map_enabled: true,
            map_loading: false,
            map_generation: 0,
            map_request: None,
            map_error: None,
            online: true,
            drag: None,
            palette: Palette::light(),
            show_all: false,
            saved_at: None,
            fit_pending: true,
            density,
            route: TravelRoute::Trips,
            details: HashMap::new(),
            coords: PlaceCoords::default(),
            asked: std::collections::HashSet::new(),
            resolving: false,
            resolve_again: false,
            selected_stop: None,
            map_dragged: false,
            scroll: ScrollHandle::new(),
            form: None,
            confirm_remove: None,
            write_error: None,
            #[cfg(all(feature = "native-maps", target_os = "macos"))]
            vector: None,
            #[cfg(all(feature = "native-maps", target_os = "macos"))]
            vector_enabled: true,
        }
    }

    pub fn set_source(&mut self, source: Source) {
        self.source = Some(source);
    }

    pub fn scroll_handle(&self) -> ScrollHandle {
        self.scroll.clone()
    }

    pub fn route(&self) -> &TravelRoute {
        &self.route
    }

    pub fn set_palette(&mut self, palette: Palette, cx: &mut Context<Self>) {
        if self.palette != palette {
            self.palette = palette;
            cx.notify();
        }
    }

    fn button(&self, id: impl Into<gpui::ElementId>) -> Button {
        Button::new(id).min_h(self.density.min_touch_target())
    }

    pub fn activate(&mut self, cx: &mut Context<Self>) {
        if !self.loaded && !self.loading {
            self.refresh(cx);
        } else if self.map_enabled {
            self.load_map(cx);
        }
    }

    pub fn deactivate(&mut self) {
        if let Some(request) = self.map_request.take() {
            request.abort();
        }
        self.map_generation += 1;
        self.map_loading = false;
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.loading {
            return;
        }
        self.loading = true;
        self.error = None;
        let work = shell::run(async {
            let saved = travel::saved_plans().await;
            // Read with the snapshot: the map can only be instant on a second open if the
            // answers it needs are already on disk before the first frame.
            let coords = travel::places::load().await;
            (saved, coords)
        });
        cx.spawn(async move |this, cx| {
            let loaded = work.await;
            let source = this.update(cx, |view, cx| {
                match loaded {
                    Some((saved, coords)) => {
                        match coords {
                            Ok(coords) => view.coords = coords,
                            Err(error) => view.error = Some(error),
                        }
                        if view.plans.is_empty() {
                            match saved {
                                Ok(Some(snapshot)) => {
                                    view.saved_at = Some(snapshot.saved_at);
                                    view.apply_plans(snapshot.plans);
                                    view.status = "Saved trips · checking for updates…".into();
                                }
                                Ok(None) => {}
                                Err(error) => view.error = Some(error),
                            }
                        }
                        view.fill_known_coordinates();
                        // Resolve while the network read is still in flight, so a trip opens
                        // with its stops whether or not the Mac answers.
                        view.resolve_coordinates(cx);
                    }
                    None => view.error = Some("Reading saved trips stopped.".into()),
                }
                cx.notify();
                view.source.clone()
            });
            let Ok(source) = source else {
                return;
            };
            let result = if let Some(source) = source {
                let work = shell::run(async move {
                    let plans = source.plans().await?;
                    let save_error = travel::save_plans(&plans).await.err();
                    Ok::<_, String>((plans, save_error))
                });
                work.await
                    .unwrap_or_else(|| Err("Loading trips stopped.".into()))
            } else {
                Err("Connect to your paired Mac and refresh to update trips.".into())
            };
            let _ = this.update(cx, |view, cx| {
                view.loading = false;
                view.loaded = true;
                match result {
                    Ok((plans, save_error)) => {
                        view.apply_plans(plans);
                        if save_error.is_none() {
                            view.saved_at = Some(chrono::Utc::now().timestamp().max(0) as u64);
                        }
                        view.status = "Trips updated · saved on this device".into();
                        view.error = save_error;
                        if view.error.is_some() {
                            view.status = "Trips updated · saving failed".into();
                        }
                    }
                    Err(error) => {
                        view.status = "Showing saved trips".into();
                        view.error = Some(error);
                    }
                }
                if !matches!(view.route, TravelRoute::Trips) {
                    if let Some(id) = view.selected.clone() {
                        view.read_details(id, true, cx);
                    }
                }
                view.fill_known_coordinates();
                view.resolve_coordinates(cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn apply_plans(&mut self, mut plans: Vec<Plan>) {
        plans.retain(|p| p.status != "archived");
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        plans.sort_by(|a, b| {
            let past_a = a.date_end < today;
            let past_b = b.date_end < today;
            past_a.cmp(&past_b).then_with(|| {
                if past_a {
                    b.date_start.cmp(&a.date_start)
                } else {
                    a.date_start.cmp(&b.date_start)
                }
            })
        });
        self.plans = plans;
        if !self
            .plans
            .iter()
            .any(|p| Some(&p.id) == self.selected.as_ref())
        {
            if let TravelRoute::Trip { id, .. } = &self.route {
                self.selected = Some(id.clone());
            } else {
                self.selected = self.plans.first().map(|p| p.id.clone());
            }
            self.fit_pending = true;
            self.center = self.current().and_then(|p| {
                p.destinations
                    .iter()
                    .chain(std::iter::once(&p.origin))
                    .find_map(|p| p.coordinate())
            });
        }
    }

    fn current(&self) -> Option<&Plan> {
        let id = self.selected.as_ref()?;
        self.details
            .get(id)
            .and_then(|slot| slot.value.as_ref())
            .map(|saved| &saved.details.plan)
            .or_else(|| self.plans.iter().find(|p| &p.id == id))
    }

    /// Fills every place a cached answer already knows, across the plan list and every
    /// cached detail read. Returns how many places gained a coordinate.
    fn fill_known_coordinates(&mut self) -> usize {
        let coords = &self.coords;
        let mut filled: usize = self
            .plans
            .iter_mut()
            .map(|plan| plan.fill_coordinates(coords))
            .sum();
        for slot in self.details.values_mut() {
            if let Some(saved) = slot.value.as_mut() {
                filled += saved.details.plan.fill_coordinates(coords);
            }
        }
        if filled > 0 && self.center.is_none() {
            self.center = self
                .current()
                .and_then(|plan| plan.places().find_map(|place| place.coordinate()));
        }
        filled
    }

    /// True while the selected trip has no coordinate to draw at all.
    fn has_no_known_stop(&self) -> bool {
        self.current()
            .is_none_or(|plan| plan.places().all(|place| place.coordinate().is_none()))
    }

    /// Resolves the selected trip's typed places through `places` and keeps the answer.
    ///
    /// Only names nothing has answered yet are asked for, so opening the same trip twice
    /// costs one file read and no network. One geocoder refusal is not an error state: the
    /// stop keeps reading "Coordinates not available", which is the truth about it.
    fn resolve_coordinates(&mut self, cx: &mut Context<Self>) {
        if self.resolving {
            self.resolve_again = true;
            return;
        }
        let Some(source) = self.source.clone() else {
            return;
        };
        let Some(plan) = self.current().cloned() else {
            return;
        };
        let mut seen = std::collections::HashSet::new();
        let pending: Vec<(String, String)> = plan
            .places()
            .filter(|place| place.coordinate().is_none())
            .filter(|place| self.coords.get(place).is_none())
            .filter_map(|place| {
                let key = travel::places::key(place);
                (!self.asked.contains(&key) && seen.insert(key.clone()))
                    .then(|| (key, travel::places::query(place)))
            })
            .collect();
        if pending.is_empty() {
            return;
        }
        self.resolving = true;
        cx.spawn(async move |this, cx| {
            let answers = travel::places::resolve(source, pending).await;
            let saved = this.update(cx, |view, cx| {
                view.resolving = false;
                let mut filled = false;
                let mut transport_failed = false;
                for (key, answer) in answers {
                    match answer {
                        Ok(Some(at)) => {
                            view.coords.insert_at(&key, at);
                            view.asked.insert(key);
                            filled = true;
                        }
                        // The geocoder answered and has nothing: asking again this session
                        // would only spend the provider's budget on the same name.
                        Ok(None) => {
                            view.asked.insert(key);
                        }
                        // The call failed, so the name is still unanswered and a later read
                        // retries it.
                        Err(_) => transport_failed = true,
                    }
                }
                if !filled {
                    if !transport_failed && view.section() == TripSection::Map {
                        // Nothing to draw: the world overview is the honest fallback, and
                        // load_map has been holding back for this answer.
                        view.load_map(cx);
                    }
                    cx.notify();
                    return None;
                }
                // A trip that had nothing to draw gets fitted once the first answer lands;
                // one that was already on screen keeps the view the operator chose.
                let first_frame = view.has_no_known_stop();
                view.fill_known_coordinates();
                if view.section() == TripSection::Map {
                    if first_frame {
                        view.fit_trip(cx);
                    } else {
                        view.load_map(cx);
                    }
                }
                cx.notify();
                Some(view.coords.clone())
            });
            if let Ok(Some(coords)) = saved {
                let _ = shell::run(travel::places::save(coords)).await;
            }
            // A detail read can arrive with stops the list did not carry (stages are details
            // only). It could not start its own pass, so this is where those names are asked.
            let _ = this.update(cx, |view, cx| {
                if std::mem::take(&mut view.resolve_again) {
                    view.resolve_coordinates(cx);
                }
            });
        })
        .detach();
        cx.notify();
    }
    /// Opens a native travel subpage; details are fetched only when a trip is opened.
    pub fn open_route(&mut self, route: TravelRoute, cx: &mut Context<Self>) {
        self.deactivate();
        self.scroll.set_offset(point(px(0.), px(0.)));
        let id = match &route {
            TravelRoute::Trip { id, .. } => Some(id.clone()),
            _ => None,
        };
        self.route = route;
        self.form = None;
        self.confirm_remove = None;
        self.write_error = None;
        if let Some(id) = id {
            self.selected = Some(id.clone());
            self.selected_stop = None;
            self.center = self
                .current()
                .and_then(|plan| plan.places().find_map(|p| p.coordinate()));
            self.fit_pending = true;
            self.read_details(id, false, cx);
        }
        self.activate(cx);
        cx.notify();
    }

    fn section(&self) -> TripSection {
        match self.route {
            TravelRoute::Trip { section, .. } => section,
            TravelRoute::Map => TripSection::Map,
            TravelRoute::Trips => TripSection::Overview,
        }
    }

    fn open_section(&mut self, section: TripSection, cx: &mut Context<Self>) {
        self.deactivate();
        self.scroll.set_offset(point(px(0.), px(0.)));
        if let Some(id) = self.selected.clone() {
            self.route = TravelRoute::Trip { id, section };
        }
        if section == TripSection::Map {
            self.load_map(cx);
        }
        cx.notify();
    }

    fn show_stop(&mut self, place: Place, cx: &mut Context<Self>) {
        self.center = place.coordinate();
        self.selected_stop = Some(place);
        self.fit_pending = false;
        self.zoom = 13.;
        self.open_section(TripSection::Map, cx);
    }

    fn fit_trip(&mut self, cx: &mut Context<Self>) {
        if self.viewport.0 < 1. {
            self.fit_pending = true;
            return;
        }
        self.fit_pending = false;
        let points: Vec<_> = self
            .current()
            .map(|plan| plan.places().filter_map(|p| p.coordinate()).collect())
            .unwrap_or_default();
        // Cached details can arrive after the initial world overview is shown.
        self.fit_pending = points.is_empty();
        if let Some((center, zoom)) = travel::fit(&points, self.viewport.0, self.viewport.1) {
            self.center = Some(center);
            self.zoom = f64::from(zoom);
        }
        self.load_map(cx);
        cx.notify();
    }

    fn select(&mut self, id: String, cx: &mut Context<Self>) {
        self.open_route(
            TravelRoute::Trip {
                id,
                section: TripSection::Overview,
            },
            cx,
        );
    }
}

impl Render for Travel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_page(cx)
    }
}
