//! One travel surface for desktop and phone. Network and disk work stay on the shared runtime.
use std::collections::HashMap;
use std::path::PathBuf;

use futures::future::AbortHandle;
use gpui::{point, prelude::*, px, AnyElement, Context, Point, Render, ScrollHandle, Window};
use gpui_kit::component::button::Button;
use sjel_gpui_core::{
    calendar::{TripDraft, TripDrafts, TripMaterialization},
    shell::{self, ShellError},
    travel::{self, Coordinate, Place, PlaceCoords, Plan, Source, Tile, TravelRoute, TripSection},
};

use crate::{Density, Palette};

mod connections;
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
    show_all_locations: bool,
    map_options_open: bool,
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
    /// The connection search, built the first time its page draws.
    connections: Option<connections::Connections>,
    /// The row whose Remove was pressed once.
    confirm_remove: Option<String>,
    /// A remove that the Mac refused, when no form is open to say so.
    write_error: Option<String>,
    trip_drafts: Option<TripDrafts>,
    trip_drafts_window: Option<(String, String)>,
    trip_drafts_loading: bool,
    trip_drafts_error: Option<String>,
    trip_draft_action: Option<String>,
    trip_draft_results: HashMap<String, TripMaterialization>,
    presence: HashMap<String, travel::places::Presence>,
    presence_loading: bool,
    presence_loaded: bool,
    presence_again: bool,
    compact_map: bool,
    #[cfg(all(feature = "native-maps", target_os = "macos"))]
    vector: Option<vector::VectorMap>,
    #[cfg(all(feature = "native-maps", target_os = "macos"))]
    vector_enabled: bool,
}

#[derive(Clone)]
struct LocationMarker {
    plan_id: String,
    place: Place,
    from: String,
    to: String,
    label: String,
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
            show_all_locations: false,
            map_options_open: false,
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
            connections: None,
            confirm_remove: None,
            write_error: None,
            trip_drafts: None,
            trip_drafts_window: None,
            trip_drafts_loading: false,
            trip_drafts_error: None,
            trip_draft_action: None,
            trip_draft_results: HashMap::new(),
            presence: HashMap::new(),
            presence_loading: false,
            presence_loaded: false,
            presence_again: false,
            compact_map: false,
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
        } else {
            if matches!(self.route, TravelRoute::Trips) {
                self.load_trip_drafts(cx, false);
            }
            if self.map_enabled {
                self.load_map(cx);
            }
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
                        view.load_people_presence(cx, false);
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
                    if matches!(view.route, TravelRoute::Map) {
                        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
                        let ids: Vec<_> = view
                            .plans
                            .iter()
                            .filter(|plan| plan.date_end >= today)
                            .take(8)
                            .map(|plan| plan.id.clone())
                            .collect();
                        for id in ids {
                            view.read_details(id, true, cx);
                        }
                    } else if let Some(id) = view.selected.clone() {
                        view.read_details(id, true, cx);
                    }
                } else {
                    view.load_trip_drafts(cx, true);
                }
                view.fill_known_coordinates();
                view.presence_loaded = false;
                view.load_people_presence(cx, false);
                view.resolve_coordinates(cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn load_trip_drafts(&mut self, cx: &mut Context<Self>, force: bool) {
        if self.trip_drafts_loading {
            return;
        }
        let from = chrono::Local::now().date_naive();
        let to = from + chrono::Duration::days(180);
        let window = (
            from.format("%Y-%m-%d").to_string(),
            to.format("%Y-%m-%d").to_string(),
        );
        if !force
            && self.trip_drafts.as_ref().is_some()
            && self.trip_drafts_window.as_ref() == Some(&window)
        {
            return;
        }
        let Some(source) = self.source.clone() else {
            self.trip_drafts_error =
                Some("Connect to your Mac to check Calendar suggestions.".into());
            return;
        };
        self.trip_drafts_window = Some(window.clone());
        self.trip_drafts_loading = true;
        self.trip_drafts_error = None;
        let (from, to) = window.clone();
        let work = shell::run(async move { source.calendar_trip_drafts(&from, &to).await });
        cx.spawn(async move |this, cx| {
            let result = work
                .await
                .unwrap_or_else(|| Err("Loading trip suggestions stopped.".into()));
            let _ = this.update(cx, |view, cx| {
                if view.trip_drafts_window.as_ref() == Some(&window) {
                    view.trip_drafts_loading = false;
                    match result {
                        Ok(drafts) => {
                            view.trip_drafts = Some(drafts);
                            view.trip_drafts_error = None;
                        }
                        Err(error) => view.trip_drafts_error = Some(error),
                    }
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn load_people_presence(&mut self, cx: &mut Context<Self>, force: bool) {
        if !matches!(
            self.route,
            TravelRoute::Map
                | TravelRoute::Trip {
                    section: TripSection::Map,
                    ..
                }
        ) {
            return;
        }
        if self.presence_loading {
            self.presence_again = true;
            return;
        }
        if self.presence_loaded && !force {
            return;
        }
        let Some(shell) = self.writer() else { return };
        let markers = self.location_markers();
        let mut seen = std::collections::HashSet::new();
        let requests: Vec<_> = markers
            .into_iter()
            .filter_map(|marker| {
                let at = marker.place.coordinate()?;
                let key = presence_key(at, &marker.from, &marker.to);
                seen.insert(key.clone())
                    .then_some((key, at, marker.from, marker.to))
            })
            .take(8)
            .collect();
        if requests.is_empty() {
            self.presence_loaded = true;
            return;
        }
        self.presence_loading = true;
        self.presence_again = false;
        let work = shell::run(async move {
            futures::future::join_all(requests.into_iter().map(|(key, at, from, to)| {
                let shell = shell.clone();
                async move {
                    let result = shell
                        .people_presence(at.latitude, at.longitude, &from, &to)
                        .await;
                    (key, result)
                }
            }))
            .await
        });
        cx.spawn(async move |this, cx| {
            let results = work.await.unwrap_or_default();
            let _ = this.update(cx, |view, cx| {
                view.presence_loading = false;
                view.presence_loaded = true;
                for (key, result) in results {
                    if let Ok(value) = result {
                        view.presence.insert(key, value);
                    }
                }
                if std::mem::take(&mut view.presence_again) {
                    view.presence_loaded = false;
                    view.load_people_presence(cx, true);
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn set_compact_map(&mut self, compact: bool) {
        self.compact_map = compact;
    }

    pub fn home_map(&mut self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        self.set_palette(palette, cx);
        if self.route != TravelRoute::Map {
            self.open_route(TravelRoute::Map, cx);
        }
        self.compact_map = true;
        self.fit_pending = true;
        if !self.loaded && !self.loading {
            self.refresh(cx);
        }
        self.load_people_presence(cx, false);
        self.map(cx)
    }

    pub(super) fn materialize_trip_draft(&mut self, draft: TripDraft, cx: &mut Context<Self>) {
        if self.trip_draft_action.is_some() || self.writer().is_none() {
            return;
        }
        let key = trip_draft_key(&draft);
        if self.trip_draft_results.contains_key(&key) {
            return;
        }
        let title = format!("{} — {}", draft.place, draft.starts_on);
        let ids = draft.entry_ids.clone();
        let Some(shell) = self.writer() else { return };
        self.trip_draft_action = Some(key.clone());
        self.trip_drafts_error = None;
        let work = shell::run(async move { shell.calendar_materialize_trip(&ids, &title).await });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |view, cx| {
                view.trip_draft_action = None;
                match result {
                    Ok(result) => {
                        view.trip_draft_results.insert(key, result);
                        view.loaded = false;
                        view.refresh(cx);
                    }
                    Err(error) => view.trip_drafts_error = Some(error.to_string()),
                }
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

    fn location_markers(&self) -> Vec<LocationMarker> {
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let horizon = (chrono::Local::now().date_naive() + chrono::Duration::days(180))
            .format("%Y-%m-%d")
            .to_string();
        let mut markers = Vec::new();
        for plan in self
            .plans
            .iter()
            .filter(|plan| plan.date_end >= today && plan.date_start <= horizon)
            .take(8)
        {
            let from = plan.date_start.clone();
            let to = plan.date_end.clone();
            for place in plan.places() {
                markers.push(LocationMarker {
                    plan_id: plan.id.clone(),
                    place: place.clone(),
                    from: from.clone(),
                    to: to.clone(),
                    label: format!("{} · {}–{}", plan.title, from, to),
                });
            }
            if let Some(details) = self
                .details
                .get(&plan.id)
                .and_then(|slot| slot.value.as_ref())
            {
                for item in &details.details.items {
                    if item.item_type != "stay" {
                        continue;
                    }
                    let Some(latitude) = item
                        .payload
                        .get("latitude")
                        .and_then(serde_json::Value::as_f64)
                    else {
                        continue;
                    };
                    let Some(longitude) = item
                        .payload
                        .get("longitude")
                        .and_then(serde_json::Value::as_f64)
                    else {
                        continue;
                    };
                    let at = Coordinate {
                        latitude,
                        longitude,
                    };
                    let Some(day) = item
                        .payload
                        .get("check_in")
                        .and_then(serde_json::Value::as_str)
                        .or(item.day.as_deref())
                    else {
                        continue;
                    };
                    let to = item
                        .payload
                        .get("check_out")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or(day)
                        .to_owned();
                    let place = Place {
                        name: item.title.clone(),
                        address: item
                            .payload
                            .get("address")
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_owned),
                        latitude: Some(at.latitude),
                        longitude: Some(at.longitude),
                    };
                    markers.push(LocationMarker {
                        plan_id: plan.id.clone(),
                        place,
                        from: day.to_owned(),
                        to,
                        label: format!("{} · stay", plan.title),
                    });
                }
            }
        }
        markers
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
                if matches!(view.route, TravelRoute::Map) {
                    view.presence_loaded = false;
                    view.load_people_presence(cx, false);
                }
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
        self.compact_map = false;
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
        } else if matches!(self.route, TravelRoute::Map) && self.loaded {
            let today = chrono::Local::now().format("%Y-%m-%d").to_string();
            let ids: Vec<_> = self
                .plans
                .iter()
                .filter(|plan| plan.date_end >= today)
                .take(8)
                .map(|plan| plan.id.clone())
                .collect();
            for id in ids {
                self.read_details(id, false, cx);
            }
        }
        self.activate(cx);
        cx.notify();
    }

    fn section(&self) -> TripSection {
        match self.route {
            TravelRoute::Trip { section, .. } => section,
            TravelRoute::Map => TripSection::Map,
            TravelRoute::Trips | TravelRoute::Connections => TripSection::Overview,
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
        let points: Vec<_> = if matches!(self.route, TravelRoute::Map) {
            self.location_markers()
                .iter()
                .filter_map(|marker| marker.place.coordinate())
                .collect()
        } else {
            self.current()
                .map(|plan| plan.places().filter_map(|p| p.coordinate()).collect())
                .unwrap_or_default()
        };
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.route == TravelRoute::Connections {
            self.ensure_connections(window, cx);
        }
        self.render_page(cx)
    }
}

pub(super) fn trip_draft_key(draft: &TripDraft) -> String {
    draft.entry_ids.join("|")
}

fn presence_key(at: Coordinate, from: &str, to: &str) -> String {
    format!("{:.4}:{:.4}:{from}:{to}", at.latitude, at.longitude)
}
