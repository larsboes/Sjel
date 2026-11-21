//! Phone travel map state, using the shared tile cache and projection.

use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard, OnceLock};

use futures::future::{AbortHandle, Abortable};
use futures::{stream, StreamExt};
use sjel_gpui_core::{
    shell,
    travel::{self, Coordinate, Tile, TravelRoute},
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
    #[cfg(target_os = "ios")]
    origin: (f32, f32),
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
                #[cfg(target_os = "ios")]
                origin: (0., 0.),
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

#[cfg(test)]
mod tests {
    use super::*;

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
}

#[cfg(target_os = "ios")]
mod phone;
#[cfg(target_os = "ios")]
pub use phone::{pinch, PhoneMap};
