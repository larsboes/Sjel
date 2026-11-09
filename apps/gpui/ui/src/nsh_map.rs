//! OpenStreetMap travel maps drawn by NSH, using the shared tile cache and projection.

use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard, OnceLock};

use futures::future::{AbortHandle, Abortable};
use futures::{stream, StreamExt};
use nsh_base::button;
use nsh_core::{
    element::TouchEvent,
    geometry::{point, size},
    prelude::*,
    Bounds, Points, TouchPhase,
};
use sjel_gpui_core::{
    shell,
    travel::{self, Coordinate, Tile, TravelRoute, TripSection},
};
use tokio::sync::watch;

use sjel_gpui_core::travel::markers::{LocationMarker, PlaceCategory};

/// A map action handled by the phone shell.
#[derive(Clone, Debug)]
pub enum MapEvent {
    Show,
    Refresh,
    Resolve(String),
    Open(TravelRoute),
}

/// The travel data shared with the map. The owning Travel view keeps reads and writes.
#[derive(Clone, Debug, PartialEq)]
pub struct MapScene {
    pub route: TravelRoute,
    pub title: String,
    pub markers: Vec<LocationMarker>,
    pub connections: Vec<(Coordinate, Coordinate)>,
    pub status: String,
    pub error: Option<String>,
    pub loading: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Camera {
    center: Coordinate,
    zoom: f64,
    size: (f32, f32),
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            center: Coordinate {
                latitude: 0.,
                longitude: 0.,
            },
            zoom: 2.,
            size: (0., 0.),
        }
    }
}

impl Camera {
    fn point(self, at: Coordinate) -> (f32, f32) {
        let (x, y) = at.world_at(self.zoom);
        let (cx, cy) = self.center.world_at(self.zoom);
        let world = 256. * 2_f64.powf(self.zoom);
        (
            ((x - cx + world / 2.).rem_euclid(world) - world / 2.) as f32 + self.size.0 / 2.,
            (y - cy) as f32 + self.size.1 / 2.,
        )
    }

    fn gesture(self, before: Fingers, after: Fingers) -> Self {
        if before.ids != after.ids || before.ids.is_empty() {
            return self;
        }
        let delta = if before.distance > 1. && after.distance > 1. {
            (after.distance / before.distance).log2() as f64
        } else {
            0.
        };
        let (center, zoom) = travel::zoom_about(
            self.center,
            self.zoom,
            before.center,
            self.size,
            delta,
            travel::MIN_ZOOM,
        );
        let (x, y) = center.world_at(zoom);
        Self {
            center: Coordinate::from_world_at(
                x - f64::from(after.center.0 - before.center.0),
                y - f64::from(after.center.1 - before.center.1),
                zoom,
            ),
            zoom,
            ..self
        }
    }

    fn wanted(self) -> Vec<Tile> {
        let level = self.zoom.round().clamp(travel::MIN_ZOOM, travel::MAX_ZOOM) as u8;
        let scale = 2_f64.powf(self.zoom - f64::from(level)) as f32;
        let mut tiles: Vec<_> =
            travel::visible_tiles(self.center, level, self.size.0 / scale, self.size.1 / scale)
                .into_iter()
                .map(|(tile, _, _)| tile)
                .collect();
        tiles.sort();
        tiles.dedup();
        tiles
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Fingers {
    ids: Vec<u32>,
    center: (f32, f32),
    distance: f32,
}

impl Fingers {
    fn from_touch(event: &TouchEvent) -> Self {
        let mut points: Vec<_> = event
            .points
            .iter()
            .filter(|p| !matches!(event.phase, TouchPhase::End | TouchPhase::Cancel) || !p.changed)
            .collect();
        points.sort_by_key(|p| p.id);
        let n = points.len().max(1) as f32;
        Self {
            ids: points.iter().map(|p| p.id).collect(),
            center: (
                points.iter().map(|p| p.location.x.0).sum::<f32>() / n,
                points.iter().map(|p| p.location.y.0).sum::<f32>() / n,
            ),
            distance: if points.len() == 2 {
                (points[0].location.x.0 - points[1].location.x.0)
                    .hypot(points[0].location.y.0 - points[1].location.y.0)
            } else {
                0.
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Snapshot {
    scene: MapScene,
    camera: Camera,
    fitted: bool,
    selected: Option<usize>,
    category: Option<PlaceCategory>,
    online: bool,
    tiles: BTreeMap<Tile, String>,
    loading: bool,
    tile_error: Option<String>,
}

impl Snapshot {
    fn markers(&self) -> impl Iterator<Item = (usize, &LocationMarker)> {
        self.scene
            .markers
            .iter()
            .enumerate()
            .filter(|(_, m)| self.category.is_none_or(|c| c == m.category))
    }

    fn fit(&mut self) {
        let points: Vec<_> = self
            .markers()
            .filter_map(|(_, m)| m.place.coordinate())
            .collect();
        if self.camera.size.0 < 1. || self.camera.size.1 < 1. {
            return;
        }
        if let Some((center, zoom)) = travel::fit(&points, self.camera.size.0, self.camera.size.1) {
            self.camera.center = center;
            self.camera.zoom = f64::from(zoom);
            self.fitted = true;
        }
    }
}

struct State {
    snap: Snapshot,
    generation: u64,
    request: Option<AbortHandle>,
    fingers: Fingers,
    active: bool,
    tap: Option<(f32, f32)>,
}

type Host = Box<dyn Fn(MapEvent) + Send>;

struct Feed {
    state: Mutex<State>,
    tx: watch::Sender<Snapshot>,
    host: Mutex<Option<Host>>,
}

fn feed() -> &'static Feed {
    static FEED: OnceLock<Feed> = OnceLock::new();
    FEED.get_or_init(|| {
        let snap = Snapshot {
            scene: MapScene {
                route: TravelRoute::Map,
                title: "Map & Places".into(),
                markers: Vec::new(),
                connections: Vec::new(),
                status: String::new(),
                error: None,
                loading: false,
            },
            camera: Camera::default(),
            fitted: false,
            selected: None,
            category: None,
            online: true,
            tiles: BTreeMap::new(),
            loading: false,
            tile_error: None,
        };
        let (tx, _) = watch::channel(snap.clone());
        Feed {
            state: Mutex::new(State {
                snap,
                generation: 0,
                request: None,
                fingers: Fingers::default(),
                active: false,
                tap: None,
            }),
            tx,
            host: Mutex::new(None),
        }
    })
}

fn state() -> MutexGuard<'static, State> {
    feed().state.lock().unwrap_or_else(|e| e.into_inner())
}

fn publish(state: &State) {
    feed().tx.send_replace(state.snap.clone());
}

fn emit(event: MapEvent) {
    if let Some(host) = feed()
        .host
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
    {
        host(event);
    }
}

/// Registers the phone shell's map navigation and refresh handler.
pub fn set_host(host: impl Fn(MapEvent) + Send + 'static) {
    *feed().host.lock().unwrap_or_else(|e| e.into_inner()) = Some(Box::new(host));
}

/// Updates the map without taking navigation away from the current screen.
pub fn set_scene(scene: MapScene) {
    let reload = {
        let mut state = state();
        if state.snap.scene == scene {
            return;
        }
        let changed_route = state.snap.scene.route != scene.route;
        if changed_route {
            state.snap.fitted = false;
            state.snap.selected = None;
            state.snap.category = None;
            state.fingers = Fingers::default();
        }
        let selected = state
            .snap
            .selected
            .and_then(|i| state.snap.scene.markers.get(i))
            .cloned();
        state.snap.scene = scene;
        state.snap.selected = selected.and_then(|before| {
            state.snap.scene.markers.iter().position(|after| {
                before.plan_id == after.plan_id
                    && before.place.name == after.place.name
                    && before.from == after.from
            })
        });
        if !state.snap.fitted {
            state.snap.fit();
        }
        publish(&state);
        changed_route || state.snap.fitted
    };
    if reload {
        load_tiles();
    }
}

/// Presents the phone's map. A requested stop takes priority over fitting the trip.
pub fn show(focus: Option<Coordinate>) {
    {
        let mut state = state();
        state.active = true;
        if let Some(center) = focus {
            state.snap.camera.center = center;
            state.snap.camera.zoom = 13.;
            state.snap.fitted = true;
            state.snap.selected = state
                .snap
                .scene
                .markers
                .iter()
                .position(|m| m.place.coordinate() == Some(center));
        }
        publish(&state);
    }
    load_tiles();
    emit(MapEvent::Show);
}

/// Cancels map work when another NSH page or a GPUI screen becomes visible.
pub fn hide() {
    let mut state = state();
    state.active = false;
    state.generation += 1;
    state.snap.loading = false;
    state.fingers = Fingers::default();
    state.tap = None;
    if let Some(request) = state.request.take() {
        request.abort();
    }
    publish(&state);
}

fn load_tiles() {
    let (missing, generation, online, registration) = {
        let mut state = state();
        if !state.active {
            return;
        }
        if state.snap.camera.size.0 < 1. || state.snap.camera.size.1 < 1. {
            return;
        }
        if let Some(request) = state.request.take() {
            request.abort();
        }
        state.generation += 1;
        let wanted = state.snap.camera.wanted();
        let level = state.snap.camera.zoom.round() as u8;
        state
            .snap
            .tiles
            .retain(|tile, _| tile.zoom <= level && level - tile.zoom <= 4);
        while state.snap.tiles.len() > 400 {
            state.snap.tiles.pop_first();
        }
        let missing: Vec<_> = wanted
            .into_iter()
            .filter(|t| !state.snap.tiles.contains_key(t))
            .collect();
        state.snap.loading = !missing.is_empty();
        state.snap.tile_error = None;
        let (abort, registration) = AbortHandle::new_pair();
        state.request = Some(abort);
        publish(&state);
        (missing, state.generation, state.snap.online, registration)
    };
    shell::spawn(async move {
        let _ = Abortable::new(
            async move {
                let mut reads = stream::iter(missing)
                    .map(|tile| async move { (tile, travel::load_tile(tile, online).await) })
                    .buffer_unordered(4);
                while let Some((tile, result)) = reads.next().await {
                    let mut state = state();
                    if state.generation != generation {
                        return;
                    }
                    match result {
                        Ok(path) => {
                            state
                                .snap
                                .tiles
                                .insert(tile, path.to_string_lossy().into_owned());
                            while state.snap.tiles.len() > 400 {
                                state.snap.tiles.pop_first();
                            }
                        }
                        Err(error) => state.snap.tile_error = Some(error),
                    }
                    publish(&state);
                }
                let mut state = state();
                if state.generation == generation {
                    state.snap.loading = false;
                    state.request = None;
                    publish(&state);
                }
            },
            registration,
        )
        .await;
    });
}

fn resize(size: (f32, f32)) {
    {
        let mut state = state();
        if state.snap.camera.size == size {
            return;
        }
        state.snap.camera.size = size;
        if !state.snap.fitted {
            state.snap.fit();
        }
        publish(&state);
    }
    load_tiles();
}

fn touch(event: &TouchEvent) {
    let finished = matches!(event.phase, TouchPhase::End | TouchPhase::Cancel);
    {
        let mut state = state();
        let next = Fingers::from_touch(event);
        if event.phase == TouchPhase::Start {
            state.tap = (next.ids.len() == 1).then_some(next.center);
        }
        if let Some(start) = state.tap {
            if next.ids.len() > 1
                || (event.phase == TouchPhase::Move
                    && (next.center.0 - start.0).hypot(next.center.1 - start.1) > 8.)
            {
                state.tap = None;
            }
        }
        if event.phase == TouchPhase::Move {
            state.snap.camera = state
                .snap
                .camera
                .gesture(state.fingers.clone(), next.clone());
            state.snap.fitted = true;
            state.snap.selected = None;
        }
        if event.phase == TouchPhase::End && next.ids.is_empty() {
            if let Some(at) = state.tap.take() {
                let selected = state
                    .snap
                    .markers()
                    .filter_map(|(index, m)| {
                        let p = state.snap.camera.point(m.place.coordinate()?);
                        ((p.0 - at.0).hypot(p.1 - at.1) <= 22.).then_some(index)
                    })
                    .next();
                state.snap.selected = selected;
            }
        }
        if event.phase == TouchPhase::Cancel {
            state.tap = None;
        }
        state.fingers = next;
        publish(&state);
    }
    if finished {
        load_tiles();
    }
}

fn zoom(delta: f64) {
    {
        let mut state = state();
        state.snap.camera.zoom =
            (state.snap.camera.zoom + delta).clamp(travel::MIN_ZOOM, travel::MAX_ZOOM);
        state.snap.fitted = true;
        publish(&state);
    }
    load_tiles();
}

fn select(index: usize) {
    {
        let mut state = state();
        let Some(marker) = state.snap.scene.markers.get(index) else {
            return;
        };
        let at = marker.place.coordinate();
        state.snap.selected = Some(index);
        if let Some(at) = at {
            state.snap.camera.center = at;
            state.snap.camera.zoom = 13.;
            state.snap.fitted = true;
        }
        publish(&state);
    }
    load_tiles();
}

fn fit() {
    {
        let mut state = state();
        state.snap.fit();
        publish(&state);
    }
    load_tiles();
}

fn rect(x: f32, y: f32, w: f32, h: f32) -> Bounds<Points> {
    Bounds {
        origin: point(px(x), px(y)),
        size: size(px(w), px(h)),
    }
}

fn draw(snap: &Snapshot) -> Painter {
    let camera = snap.camera;
    let mut p = Painter::new();
    let wanted = camera.wanted();
    let mut tiles = std::collections::BTreeSet::new();
    for tile in wanted {
        for levels in (1..=4).rev() {
            if let Some(parent) = tile.ancestor(levels) {
                if snap.tiles.contains_key(&parent) {
                    tiles.insert(parent);
                }
            }
        }
        if snap.tiles.contains_key(&tile) {
            tiles.insert(tile);
        }
    }
    for tile in tiles {
        let placement = tile.place(camera.center, camera.zoom, camera.size.0, camera.size.1);
        p = p.draw_image(
            &snap.tiles[&tile],
            rect(placement.x, placement.y, placement.size, placement.size),
        );
    }
    p = p
        .stroke_color(color(nsh_base::theme().primary))
        .line_width(2.);
    let world = (256. * 2_f64.powf(camera.zoom)) as f32;
    for &(from, to) in &snap.scene.connections {
        let a = camera.point(from);
        let b = camera.point(to);
        let dx = (b.0 - a.0 + world / 2.).rem_euclid(world) - world / 2.;
        p = p
            .begin_path()
            .move_to(a.0, a.1)
            .line_to(a.0 + dx, b.1)
            .stroke();
    }
    for (index, marker) in snap.markers() {
        let Some(at) = marker.place.coordinate() else {
            continue;
        };
        let (x, y) = camera.point(at);
        let radius = if snap.selected == Some(index) {
            16.
        } else {
            12.
        };
        p = p
            .fill_color(color(nsh_base::theme().primary))
            .begin_path()
            .arc(x, y, radius, 0., std::f32::consts::TAU)
            .fill()
            .fill_color(color(nsh_base::theme().primary_foreground))
            .font("12px sans-serif")
            .fill_text(&(index + 1).to_string(), x - 4., y + 4.);
    }
    p
}

fn action(label: impl Into<String>, press: impl Fn() + 'static) -> impl IntoElement {
    button()
        .px_3()
        .py_2()
        .child(text(label.into()).text_sm().number_of_lines(1u32))
        .on_press(press)
}

/// The phone's global or per-trip travel map.
#[component]
pub fn MapScreen() -> impl IntoElement {
    let mut rx = feed().tx.subscribe();
    let snap = signal(rx.borrow().clone());
    spawn(async move {
        while rx.changed().await.is_ok() {
            let next = rx.borrow_and_update().clone();
            snap.set(next);
        }
    });
    let category = computed(move || snap.with(|s| s.category));
    let online = computed(move || snap.with(|s| s.online));
    let places = computed(move || snap.with(|s| (s.scene.markers.clone(), s.category, s.selected)));
    view()
        .flex_1()
        .gap_2()
        .child(
            view()
                .flex_row()
                .items_center()
                .px_4()
                .child(action("Trips", || emit(MapEvent::Open(TravelRoute::Trips))))
                .child(
                    text(move || snap.with(|s| s.scene.title.clone()))
                        .text_lg()
                        .number_of_lines(1u32)
                        .flex_1(),
                )
                .child(action("Refresh", || emit(MapEvent::Refresh))),
        )
        .child(dynamic(move || {
            let current_category = category.get();
            let mut row = view().flex_row().flex_wrap().px_4();
            for (category, label) in [
                (None, "All places"),
                (Some(PlaceCategory::Stay), "Stay"),
                (Some(PlaceCategory::Station), "Station"),
                (Some(PlaceCategory::Airport), "Airport"),
                (Some(PlaceCategory::Place), "Place"),
            ] {
                let label = if current_category == category {
                    format!("{label} •")
                } else {
                    label.into()
                };
                row = row.child(action(label, move || {
                    let mut state = state();
                    state.snap.category = category;
                    state.snap.selected = None;
                    publish(&state);
                }));
            }
            row
        }))
        .child(
            canvas(move || snap.with(draw))
                .flex_1()
                .min_h(px(200.))
                .overflow_hidden()
                .bg(move || nsh_base::theme().muted)
                .test_id("travel-map")
                .claim_touches(TouchClaim::on_start())
                .on_touch(touch)
                .on_layout(|frame: &Bounds<Points>| {
                    resize((frame.size.width.0, frame.size.height.0))
                }),
        )
        .child(
            view()
                .flex_row()
                .items_center()
                .px_4()
                .child(action("Zoom out", || zoom(-1.)))
                .child(action("Zoom in", || zoom(1.)))
                .child(action("Fit", fit))
                .child(dynamic(move || {
                    action(
                        if online.get() {
                            "Online tiles"
                        } else {
                            "Cached only"
                        },
                        || {
                            {
                                let mut state = state();
                                state.snap.online = !state.snap.online;
                                publish(&state);
                            }
                            load_tiles();
                        },
                    )
                })),
        )
        .child(
            text("© OpenStreetMap contributors · Lines join saved stops, not road or rail routes.")
                .px_4()
                .text_xs()
                .text_color(move || nsh_base::theme().muted_foreground)
                .on_press(|| drop(open_url("https://www.openstreetmap.org/copyright"))),
        )
        .child(
            text(move || {
                snap.with(|s| {
                    s.tile_error
                        .clone()
                        .or_else(|| s.scene.error.clone())
                        .unwrap_or_else(|| {
                            if s.loading || s.scene.loading {
                                "Loading map…".into()
                            } else {
                                s.scene.status.clone()
                            }
                        })
                })
            })
            .px_4()
            .text_xs(),
        )
        .child(dynamic(move || {
            snap.with(|s| s.tile_error.is_some())
                .then(|| action("Retry map", load_tiles))
        }))
        .child(scroll().h(px(160.)).child(dynamic(move || {
            let (markers, category, selected) = places.get();
            let mut list = view().px_4();
            if markers.is_empty() {
                list = list
                    .child(text("No saved places to show. Open a trip to add stops.").text_sm());
            }
            for (index, marker) in markers
                .iter()
                .enumerate()
                .filter(|(_, m)| category.is_none_or(|c| c == m.category))
            {
                let suffix = if marker.place.coordinate().is_none() {
                    " · Coordinates not available"
                } else {
                    ""
                };
                list = list.child(action(
                    format!("{}. {}{suffix}", index + 1, marker.place.name),
                    move || select(index),
                ));
                if selected == Some(index) {
                    list = list.child(text(marker.place.name.clone()).text_sm());
                    list = list.child(text(marker.label.clone()).text_xs());
                    if let Some(address) = &marker.place.address {
                        list = list.child(text(address.clone()).text_sm());
                    }
                    list = list.child(
                        text(format!(
                            "{} · {}–{}",
                            marker.category.label(),
                            marker.from,
                            marker.to
                        ))
                        .text_xs(),
                    );
                    if let Some(at) = marker.place.coordinate() {
                        let url = format!(
                            "https://maps.apple.com/?q={}&ll={:.6},{:.6}",
                            travel::urlencode(&marker.place.name),
                            at.latitude,
                            at.longitude
                        );
                        list = list.child(action("Open in Apple Maps", move || {
                            drop(open_url(url.clone()));
                        }));
                    } else {
                        let id = marker.plan_id.clone();
                        list = list.child(action("Resolve coordinates", move || {
                            emit(MapEvent::Resolve(id.clone()))
                        }));
                    }
                    let id = marker.plan_id.clone();
                    list = list.child(action("Open trip", move || {
                        emit(MapEvent::Open(TravelRoute::Trip {
                            id: id.clone(),
                            section: TripSection::Overview,
                        }))
                    }));
                }
            }
            list
        })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sjel_gpui_core::travel::Place;

    fn camera() -> Camera {
        Camera {
            center: Coordinate::new(52.52, 13.405).unwrap(),
            zoom: 10.4,
            size: (390., 400.),
        }
    }

    fn fingers(ids: &[u32], center: (f32, f32), distance: f32) -> Fingers {
        Fingers {
            ids: ids.to_vec(),
            center,
            distance,
        }
    }

    #[test]
    fn a_pinch_and_pan_keep_the_ground_under_the_fingers() {
        let camera = camera();
        let before = fingers(&[1, 2], (100., 120.), 80.);
        let after = fingers(&[1, 2], (150., 180.), 160.);
        let (cx, cy) = camera.center.world_at(camera.zoom);
        let ground = Coordinate::from_world_at(cx + 100. - 195., cy + 120. - 200., camera.zoom);
        let next = camera.gesture(before, after);
        let point = next.point(ground);
        assert!((point.0 - 150.).abs() < 0.01);
        assert!((point.1 - 180.).abs() < 0.01);
        assert!((next.zoom - 11.4).abs() < 0.0001);
    }

    #[test]
    fn adding_or_lifting_a_finger_does_not_move_the_map() {
        let camera = camera();
        let one = fingers(&[1], (30., 50.), 0.);
        let two = fingers(&[1, 2], (110., 150.), 200.);
        assert_eq!(camera.gesture(one.clone(), two.clone()), camera);
        assert_eq!(camera.gesture(two, one), camera);
        assert_eq!(
            camera.gesture(
                fingers(&[1], (30., 50.), 0.),
                fingers(&[2], (130., 50.), 0.)
            ),
            camera
        );
    }

    #[test]
    fn fractional_zoom_requests_every_visible_tile() {
        let camera = camera();
        let wanted = camera.wanted();
        for (x, y) in [(1., 1.), (389., 1.), (1., 399.), (389., 399.)] {
            assert!(
                wanted.iter().any(|tile| {
                    let p = tile.place(camera.center, camera.zoom, 390., 400.);
                    x >= p.x && x < p.x + p.size && y >= p.y && y < p.y + p.size
                }),
                "the viewport corner {x},{y} must have a tile"
            );
        }
        assert!(wanted.len() < 20, "only the viewport is requested");
    }

    #[test]
    fn panning_wraps_the_dateline_and_stops_at_the_poles() {
        let camera = Camera {
            center: Coordinate::new(0., 179.9).unwrap(),
            zoom: 5.,
            ..camera()
        };
        let next = camera.gesture(
            fingers(&[1], (200., 200.), 0.),
            fingers(&[1], (100., 1_000_000.), 0.),
        );
        assert!(next.center.longitude < 0.);
        assert!(next.center.latitude.abs() <= 85.052);
        let at = camera.point(Coordinate::new(0., -179.9).unwrap());
        assert!((at.0 - 195.).abs() < 10.);
    }

    #[test]
    fn map_renders_filters_places_selects_a_stop_and_changes_connection_mode() {
        use nsh_core::element::Root;
        use nsh_engine::EngineConfig;
        use nsh_testing::Headless;
        hide();
        let (events, received) = std::sync::mpsc::channel();
        set_host(move |event| {
            events.send(event).unwrap();
        });
        set_scene(MapScene {
            route: TravelRoute::Map,
            title: "Trip map".into(),
            markers: vec![
                LocationMarker {
                    plan_id: "test-trip".into(),
                    place: Place {
                        name: "Central Station".into(),
                        latitude: Some(52.52),
                        longitude: Some(13.405),
                        ..Default::default()
                    },
                    from: "2026-11-10".into(),
                    to: "2026-11-11".into(),
                    label: "Trip · 10–11 November".into(),
                    category: PlaceCategory::Station,
                },
                LocationMarker {
                    plan_id: "other-trip".into(),
                    place: Place {
                        name: "Hotel".into(),
                        ..Default::default()
                    },
                    from: "2026-11-10".into(),
                    to: "2026-11-11".into(),
                    label: "Stay".into(),
                    category: PlaceCategory::Stay,
                },
            ],
            connections: Vec::new(),
            status: "Saved trips".into(),
            error: None,
            loading: false,
        });
        let mut app = Headless::start(EngineConfig {
            ui_sets: vec![nsh_base::registry()],
            ..EngineConfig::new(Root::of::<MapScreen>())
        })
        .unwrap();
        app.wait_for("text:Trip map").unwrap();
        app.press("text:1. Central Station").unwrap();
        assert_eq!(
            state().snap.selected,
            Some(0),
            "the stop button selects its marker"
        );
        app.wait_for("text:Trip · 10–11 November").unwrap();
        app.wait_for("text:Open trip").unwrap();
        app.press("text:Station").unwrap();
        app.wait_for("text:Station •").unwrap();
        assert!(!app
            .screen()
            .texts()
            .iter()
            .any(|text| text.contains("Coordinates not available")));
        app.press("text:Stay").unwrap();
        app.press("text:2. Hotel · Coordinates not available")
            .unwrap();
        app.press("text:Resolve coordinates").unwrap();
        assert!(
            matches!(received.try_recv().unwrap(), MapEvent::Resolve(id) if id == "other-trip")
        );
        app.press("text:Online tiles").unwrap();
        app.wait_for("text:Cached only").unwrap();
        assert!(!state().snap.online);
        app.press("text:Zoom in").unwrap();
        app.press("text:Fit").unwrap();
        assert!(state().snap.camera.zoom <= travel::MAX_ZOOM);
        let before = state().snap.camera;
        app.drag("id:travel-map", point(px(40.), px(0.))).unwrap();
        let after = state().snap.camera;
        assert_ne!(
            before.center, after.center,
            "the canvas receives a real touch drag"
        );
        assert_eq!(before.zoom, after.zoom);
        for (width, height) in [(375., 600.), (390., 750.)] {
            app.set_viewport(size(px(width), px(height))).unwrap();
            let nodes = app.screen().layout_nodes();
            let canvas = nodes
                .iter()
                .find(|n| n.test_id.as_deref() == Some("travel-map"))
                .unwrap();
            assert!(canvas.bounds.size.height.0 >= 200.);
            assert!(canvas.bounds.origin.y.0 + canvas.bounds.size.height.0 <= height);
        }
    }
}
