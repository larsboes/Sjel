//! Native clients consume the trips HTTP contract and retain the last successful read locally.
use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::{device::DeviceClient, record::CapabilityLinks, shell::Shell};

pub mod connections;
mod details;
pub mod edit;
mod navigation;
pub mod places;
pub use details::{save_details, saved_details, Details, Item, SavedDetails};
pub use navigation::{TravelRoute, TripSection};
pub use places::PlaceCoords;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Place {
    pub name: String,
    #[serde(default)]
    pub address: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
}

impl Place {
    pub fn coordinate(&self) -> Option<Coordinate> {
        Coordinate::new(self.latitude?, self.longitude?)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Stage {
    /// `stage:<n>`; empty in a snapshot saved before the app could edit stages.
    #[serde(default)]
    pub id: String,
    pub sequence: usize,
    pub origin: Place,
    pub destination: Place,
    pub date: Option<String>,
    pub status: String,
    #[serde(default)]
    pub transport_modes: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Plan {
    pub id: String,
    pub title: String,
    pub origin: Place,
    pub destinations: Vec<Place>,
    pub date_start: String,
    pub date_end: String,
    pub status: String,
    #[serde(default)]
    pub stages: Vec<Stage>,
}

impl Plan {
    /// Connections follow explicitly saved stages; they are not turn-by-turn routes.
    pub fn connections(&self) -> Vec<(Coordinate, Coordinate)> {
        self.stages
            .iter()
            .filter_map(|stage| Some((stage.origin.coordinate()?, stage.destination.coordinate()?)))
            .collect()
    }

    pub fn places(&self) -> impl Iterator<Item = &Place> {
        let mut seen = std::collections::HashSet::new();
        std::iter::once(&self.origin)
            .chain(self.destinations.iter())
            .chain(self.stages.iter().flat_map(|s| [&s.origin, &s.destination]))
            .filter(move |place| {
                seen.insert((
                    place.name.as_str(),
                    place.latitude.map(f64::to_bits),
                    place.longitude.map(f64::to_bits),
                ))
            })
    }

    /// Fills every place that has no coordinate from `coords`, leaving a saved one alone.
    ///
    /// A coordinate that arrives with the record always wins: this is the fallback for a plan
    /// whose places were stored as typed, not a correction of one that already knows where it
    /// is going. Returns how many places were filled, so a caller can tell whether the map has
    /// anything new to draw.
    pub fn fill_coordinates(&mut self, coords: &PlaceCoords) -> usize {
        let mut filled = 0;
        let mut fill = |place: &mut Place| {
            if place.coordinate().is_some() {
                return;
            }
            if let Some(at) = coords.get(place) {
                place.latitude = Some(at.latitude);
                place.longitude = Some(at.longitude);
                filled += 1;
            }
        };
        fill(&mut self.origin);
        for place in &mut self.destinations {
            fill(place);
        }
        for stage in &mut self.stages {
            fill(&mut stage.origin);
            fill(&mut stage.destination);
        }
        filled
    }
}

#[derive(Deserialize, Serialize)]
pub struct Snapshot {
    pub saved_at: u64,
    pub plans: Vec<Plan>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct FareAlert {
    pub id: String,
    pub title: String,
}

pub fn upcoming(plan: &Plan, today: &str) -> bool {
    let Ok(today) = chrono::NaiveDate::parse_from_str(today, "%Y-%m-%d") else {
        return false;
    };
    chrono::NaiveDate::parse_from_str(&plan.date_end, "%Y-%m-%d")
        .or_else(|_| chrono::NaiveDate::parse_from_str(&plan.date_start, "%Y-%m-%d"))
        .is_ok_and(|last_day| last_day >= today)
}

/// New low-fare notes in upcoming trip details that this device has not announced.
pub fn unseen_fare_lows(
    details: &[Details],
    seen: &std::collections::HashSet<String>,
    today: &str,
) -> Vec<FareAlert> {
    let mut emitted = std::collections::HashSet::new();
    details
        .iter()
        .filter(|trip| upcoming(&trip.plan, today))
        .flat_map(|trip| trip.items.iter())
        .filter_map(|item| {
            if item.item_type != "note"
                || !item.external_id.starts_with("sparpreis-low:")
                || seen.contains(&item.external_id)
                || !emitted.insert(item.external_id.clone())
            {
                return None;
            }
            Some(FareAlert {
                id: item.external_id.clone(),
                title: if item.title.is_empty() {
                    "A fare fell to a new low".to_owned()
                } else {
                    item.title.clone()
                },
            })
        })
        .collect()
}

#[derive(Clone)]
pub enum Source {
    Shell(Arc<Shell>),
    Device(Arc<DeviceClient>),
}

impl Source {
    pub async fn plans(&self) -> Result<Vec<Plan>, String> {
        match self {
            Self::Shell(shell) => shell.travel_plans().await.map_err(|e| e.to_string()),
            Self::Device(client) => client.get("/trips/api/plans").await,
        }
    }

    /// Loads one plan, including the items omitted by the trip list.
    pub async fn details(&self, id: &str) -> Result<Details, String> {
        let details: Details = match self {
            Self::Shell(shell) => shell.travel_details(id).await.map_err(|e| e.to_string())?,
            Self::Device(client) => client.get(&details_path(id)?).await?,
        };
        if details.plan.id != id {
            return Err("The server returned a different trip.".into());
        }
        Ok(details)
    }

    /// Calendar entries owned by Calendar and visible during the trip window.
    pub async fn calendar(
        &self,
        from: &str,
        to: &str,
    ) -> Result<Vec<crate::shell::CalendarEntry>, String> {
        match self {
            Self::Shell(shell) => shell
                .calendar(from, to)
                .await
                .map_err(|error| error.to_string()),
            Self::Device(client) => client.get(&calendar_entries_path(from, to)?).await,
        }
    }

    /// Calendar events grouped with this trip's destinations and date window.
    pub async fn related_calendar_events(
        &self,
        plan: &Plan,
    ) -> Result<Vec<crate::shell::CalendarEntry>, String> {
        let (from, to) = calendar_context_window(plan)
            .ok_or("This trip has invalid dates, so Calendar events cannot be matched.")?;
        let drafts = self.calendar_trip_drafts(&from, &to).await?;
        let places = destination_names(plan);
        let linked_ids: std::collections::HashSet<String> = drafts
            .drafts
            .into_iter()
            .filter(|draft| {
                let draft_place = draft.place.to_lowercase();
                places.iter().any(|place| {
                    draft_place == *place
                        || draft_place.contains(place)
                        || place.contains(&draft_place)
                })
            })
            .flat_map(|draft| draft.entry_ids)
            .collect();
        if linked_ids.is_empty() {
            return Ok(Vec::new());
        }
        let entries = self.calendar(&from, &to).await?;
        Ok(related_calendar_events(&linked_ids, entries))
    }

    /// Reads Calendar's exact references to one trip's entries.
    ///
    /// # Errors
    ///
    /// Returns an error when the plan id is invalid or Calendar cannot answer.
    pub async fn calendar_plan_links(&self, plan_id: &str) -> Result<CapabilityLinks, String> {
        details_path(plan_id)?;
        match self {
            Self::Shell(shell) => shell
                .capability_links("calendar", plan_id)
                .await
                .map_err(|error| error.to_string()),
            Self::Device(client) => client.get(&calendar_plan_links_path(plan_id)?).await,
        }
    }

    pub async fn calendar_trip_drafts(
        &self,
        from: &str,
        to: &str,
    ) -> Result<crate::calendar::TripDrafts, String> {
        match self {
            Self::Shell(shell) => shell
                .calendar_trip_drafts(from, to)
                .await
                .map_err(|error| error.to_string()),
            Self::Device(client) => client.get(&trip_drafts_path(from, to)?).await,
        }
    }

    pub async fn materialize_calendar_trip(
        &self,
        entry_ids: &[String],
        title: &str,
    ) -> Result<crate::calendar::TripMaterialization, String> {
        match self {
            Self::Shell(shell) => shell
                .calendar_materialize_trip(entry_ids, title)
                .await
                .map_err(|error| error.to_string()),
            Self::Device(_) => Err("Trip suggestions can only be adopted on the Mac.".into()),
        }
    }
}

fn calendar_entries_path(from: &str, to: &str) -> Result<String, String> {
    let mut url = reqwest::Url::parse("http://localhost/calendar/api/entries")
        .map_err(|error| error.to_string())?;
    url.query_pairs_mut()
        .append_pair("from", from)
        .append_pair("to", to);
    let mut path = url.path().to_owned();
    if let Some(query) = url.query() {
        path.push('?');
        path.push_str(query);
    }
    Ok(path)
}

fn calendar_plan_links_path(plan_id: &str) -> Result<String, String> {
    let mut url = reqwest::Url::parse("http://localhost/calendar/api/links")
        .map_err(|error| error.to_string())?;
    url.query_pairs_mut().append_pair("to", plan_id);
    let mut path = url.path().to_owned();
    if let Some(query) = url.query() {
        path.push('?');
        path.push_str(query);
    }
    Ok(path)
}

fn destination_names(plan: &Plan) -> Vec<String> {
    let mut places: Vec<String> = plan
        .destinations
        .iter()
        .chain(plan.stages.iter().map(|stage| &stage.destination))
        .flat_map(|place| place.name.split(',').map(str::trim))
        .filter(|name| name.chars().count() >= 3)
        .map(str::to_lowercase)
        .collect();
    places.sort();
    places.dedup();
    places
}

fn calendar_context_window(plan: &Plan) -> Option<(String, String)> {
    let start = chrono::NaiveDate::parse_from_str(&plan.date_start, "%Y-%m-%d").ok()?;
    let last = chrono::NaiveDate::parse_from_str(&plan.date_end, "%Y-%m-%d").ok()?;
    if start > last {
        return None;
    }
    Some((
        (start - chrono::Duration::days(5))
            .format("%Y-%m-%d")
            .to_string(),
        (last + chrono::Duration::days(6))
            .format("%Y-%m-%d")
            .to_string(),
    ))
}

/// Calendar stays the event owner. Only events in Calendar's own destination/time clusters join.
pub fn related_calendar_events(
    linked_ids: &std::collections::HashSet<String>,
    entries: Vec<crate::shell::CalendarEntry>,
) -> Vec<crate::shell::CalendarEntry> {
    let mut events: Vec<_> = entries
        .into_iter()
        .filter(|entry| entry.kind == "event" && linked_ids.contains(&entry.id))
        .collect();
    events.sort_by(|a, b| a.starts_at.cmp(&b.starts_at).then(a.title.cmp(&b.title)));
    events
}

fn trip_drafts_path(from: &str, to: &str) -> Result<String, String> {
    let mut url = reqwest::Url::parse("http://localhost/calendar/api/trip-drafts")
        .map_err(|error| error.to_string())?;
    url.query_pairs_mut()
        .append_pair("from", from)
        .append_pair("to", to)
        .append_pair("max_gap_days", "5");
    let mut path = url.path().to_owned();
    if let Some(query) = url.query() {
        path.push('?');
        path.push_str(query);
    }
    Ok(path)
}

pub(crate) fn details_path(id: &str) -> Result<String, String> {
    plan_path(id, &[])
}

/// `/trips/api/plans/{id}/{rest…}`, each segment percent-encoded so an id cannot add a path.
pub(crate) fn plan_path(id: &str, rest: &[&str]) -> Result<String, String> {
    if !navigation::valid_id(id) {
        return Err("The trip identifier is invalid.".into());
    }
    let mut url =
        reqwest::Url::parse("http://localhost/trips/api/plans/").map_err(|e| e.to_string())?;
    url.path_segments_mut()
        .map_err(|_| "Cannot build the trip address.")?
        .pop_if_empty()
        .push(id)
        .extend(rest);
    Ok(url.path().to_owned())
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Coordinate {
    pub latitude: f64,
    pub longitude: f64,
}

impl Coordinate {
    pub fn new(latitude: f64, longitude: f64) -> Option<Self> {
        (latitude.is_finite()
            && longitude.is_finite()
            && (-90.0..=90.0).contains(&latitude)
            && (-180.0..=180.0).contains(&longitude))
        .then_some(Self {
            latitude,
            longitude,
        })
    }

    pub fn world(self, zoom: u8) -> (f64, f64) {
        self.world_at(f64::from(zoom))
    }

    /// The same projection at a fractional zoom, which is what a gesture produces.
    pub fn world_at(self, zoom: f64) -> (f64, f64) {
        let size = 256.0 * 2_f64.powf(zoom);
        let latitude = self.latitude.clamp(-85.05112878, 85.05112878).to_radians();
        (
            (self.longitude + 180.0) / 360.0 * size,
            (1.0 - latitude.tan().asinh() / std::f64::consts::PI) / 2.0 * size,
        )
    }

    pub fn from_world(x: f64, y: f64, zoom: u8) -> Self {
        Self::from_world_at(x, y, f64::from(zoom))
    }

    pub fn from_world_at(x: f64, y: f64, zoom: f64) -> Self {
        let size = 256.0 * 2_f64.powf(zoom);
        Self {
            longitude: x.rem_euclid(size) / size * 360.0 - 180.0,
            latitude: (std::f64::consts::PI * (1.0 - 2.0 * y.clamp(0.0, size) / size))
                .sinh()
                .atan()
                .to_degrees(),
        }
    }
}

/// Lowest and highest zoom the map offers. Raster tiles exist to z18; below z1 a vector
/// basemap has no tiles at all.
pub const MIN_ZOOM: f64 = 0.0;
pub const MAX_ZOOM: f64 = 18.0;

/// Zooms about a point, keeping the ground under `cursor` still.
///
/// This is what makes a wheel or a pinch feel attached to the map instead of to its centre:
/// the world position under the pointer is taken at the old zoom and put back under the same
/// pointer at the new one. `cursor` and `viewport` are both in layout points, the cursor
/// relative to the map's own top-left corner.
pub fn zoom_about(
    center: Coordinate,
    zoom: f64,
    cursor: (f32, f32),
    viewport: (f32, f32),
    delta: f64,
    min_zoom: f64,
) -> (Coordinate, f64) {
    let target = (zoom + delta).clamp(min_zoom, MAX_ZOOM);
    if target == zoom || viewport.0 < 1. || viewport.1 < 1. {
        return (center, target);
    }
    let offset = (
        f64::from(cursor.0) - f64::from(viewport.0) / 2.,
        f64::from(cursor.1) - f64::from(viewport.1) / 2.,
    );
    let (cx, cy) = center.world_at(zoom);
    let anchored = Coordinate::from_world_at(cx + offset.0, cy + offset.1, zoom);
    let (ax, ay) = anchored.world_at(target);
    (
        Coordinate::from_world_at(ax - offset.0, ay - offset.1, target),
        target,
    )
}

/// Where a tile lands on screen at a fractional zoom.
///
/// `size` is `256 · 2^(zoom − tile.zoom)`, so a tile of the exact level draws at 256 points
/// and its ancestor draws larger. That single formula is what lets a missing tile be covered
/// by coarser imagery already on disk instead of a blank square.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TilePlacement {
    pub tile: Tile,
    pub x: f32,
    pub y: f32,
    pub size: f32,
}

impl Tile {
    /// The tile `levels` tiers coarser that contains this one, if that level exists.
    pub fn ancestor(self, levels: u8) -> Option<Self> {
        (levels < self.zoom).then(|| Self {
            zoom: self.zoom - levels,
            x: self.x >> levels,
            y: self.y >> levels,
        })
    }

    /// Screen position and size of this tile at `zoom`, x placed on the shortest arc from the
    /// centre so the dateline does not tear the world in two.
    pub fn place(self, center: Coordinate, zoom: f64, width: f32, height: f32) -> TilePlacement {
        let size = 256. * 2_f64.powf(zoom - f64::from(self.zoom));
        let (cx, cy) = center.world_at(zoom);
        let world = 256. * 2_f64.powf(zoom);
        let left = f64::from(self.x) * size;
        let dx = (left - cx + world / 2.).rem_euclid(world) - world / 2.;
        TilePlacement {
            tile: self,
            x: (dx + f64::from(width) / 2.) as f32,
            y: (f64::from(self.y) * size - cy + f64::from(height) / 2.) as f32,
            size: size as f32,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Tile {
    pub zoom: u8,
    pub x: u32,
    pub y: u32,
}

/// Every tile of `zoom` that the viewport touches, x wrapped across the dateline.
pub fn visible_tiles(
    center: Coordinate,
    zoom: u8,
    width: f32,
    height: f32,
) -> Vec<(Tile, f32, f32)> {
    let (cx, cy) = center.world(zoom);
    let left = cx - f64::from(width) / 2.0;
    let top = cy - f64::from(height) / 2.0;
    let count = 1_i64 << zoom;
    let mut tiles = Vec::new();
    for y in (top / 256.0).floor() as i64..((top + f64::from(height)) / 256.0).ceil() as i64 {
        if !(0..count).contains(&y) {
            continue;
        }
        for x in (left / 256.0).floor() as i64..((left + f64::from(width)) / 256.0).ceil() as i64 {
            tiles.push((
                Tile {
                    zoom,
                    x: x.rem_euclid(count) as u32,
                    y: y as u32,
                },
                (x as f64 * 256.0 - left) as f32,
                (y as f64 * 256.0 - top) as f32,
            ));
        }
    }
    tiles
}

pub fn cache_dir() -> Option<PathBuf> {
    #[cfg(any(test, feature = "test-support"))]
    {
        None
    }
    #[cfg(not(any(test, feature = "test-support")))]
    {
        if let Some(path) = crate::data::Board::default_path() {
            return path.parent().map(|p| p.join("travel"));
        }
        std::env::var_os("HOME")
            .map(|home| PathBuf::from(home).join("Library/Application Support/dev.sjel/travel"))
    }
}

/// Disk work runs on the HTTP runtime's blocking pool, never GPUI's foreground executor.
pub async fn saved_plans() -> Result<Option<Snapshot>, String> {
    let dir = cache_dir().ok_or("This device has no travel storage directory.")?;
    tokio::task::spawn_blocking(move || match std::fs::read(dir.join("plans.json")) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| format!("Read saved trips: {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("Read saved trips: {e}")),
    })
    .await
    .map_err(|e| e.to_string())?
}

pub async fn save_plans(plans: &[Plan]) -> Result<(), String> {
    let dir = cache_dir().ok_or("This device has no travel storage directory.")?;
    let saved_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_secs();
    let bytes = serde_json::to_vec(&Snapshot {
        saved_at,
        plans: plans.to_vec(),
    })
    .map_err(|e| e.to_string())?;
    tokio::task::spawn_blocking(move || {
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join("plans.json.tmp"), bytes)?;
        std::fs::rename(dir.join("plans.json.tmp"), dir.join("plans.json"))
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| format!("Save trips for offline use: {e}"))
}

/// Fits stops in the viewport using the shortest longitude arc, including across the dateline.
pub fn fit(places: &[Coordinate], width: f32, height: f32) -> Option<(Coordinate, u8)> {
    if places.is_empty() {
        return None;
    }
    let mut xs: Vec<_> = places.iter().map(|p| p.world(0).0).collect();
    xs.sort_by(f64::total_cmp);
    let mut gap = -1.;
    let mut start = xs[0];
    for (i, &x) in xs.iter().enumerate() {
        let next = xs[(i + 1) % xs.len()] + if i + 1 == xs.len() { 256. } else { 0. };
        if next - x > gap {
            gap = next - x;
            start = next.rem_euclid(256.);
        }
    }
    let span_x = 256. - gap;
    let (mut min_y, mut max_y) = (f64::INFINITY, f64::NEG_INFINITY);
    for place in places {
        let y = place.world(0).1;
        min_y = min_y.min(y);
        max_y = max_y.max(y);
    }
    let center = Coordinate::from_world(start + span_x / 2., (min_y + max_y) / 2., 0);
    let zoom = (0..=13)
        .rev()
        .find(|&zoom| {
            let scale = 2_f64.powi(i32::from(zoom));
            span_x * scale <= f64::from((width - 80.).max(40.))
                && (max_y - min_y) * scale <= f64::from((height - 80.).max(40.))
        })
        .unwrap_or(0);
    Some((center, zoom))
}

/// OSM standard tiles: viewport reads only, identified client, persistent cache (>=7 days).
/// No bulk downloads or prefetch: https://operations.osmfoundation.org/policies/tiles/.
pub async fn load_tile(tile: Tile, online: bool) -> Result<PathBuf, String> {
    // Bound requests across consecutive pan/zoom actions, not just within one viewport.
    static REQUESTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);
    let _permit = REQUESTS.acquire().await.map_err(|e| e.to_string())?;
    let path = cache_dir()
        .ok_or("This device has no map storage directory.")?
        .join("osm")
        .join(tile.zoom.to_string())
        .join(tile.x.to_string())
        .join(format!("{}.png", tile.y));
    let check = path.clone();
    if tokio::task::spawn_blocking(move || check.is_file())
        .await
        .map_err(|e| e.to_string())?
    {
        return Ok(path);
    }
    if !online {
        return Err("This area has no cached map. Connect to load it.".into());
    }
    static CLIENT: std::sync::OnceLock<Result<reqwest::Client, String>> =
        std::sync::OnceLock::new();
    let client = CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .user_agent("Sjel/0.1 (+https://github.com/larsboes/Sjel)")
                .timeout(std::time::Duration::from_secs(12))
                .build()
                .map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(Clone::clone)?;
    let mut response = client
        .get(format!(
            "https://tile.openstreetmap.org/{}/{}/{}.png",
            tile.zoom, tile.x, tile.y
        ))
        .send()
        .await
        .map_err(|e| format!("Map connection failed: {e}"))?
        .error_for_status()
        .map_err(|e| format!("Map provider: {e}"))?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
        if bytes.len() + chunk.len() > 2_000_000 {
            return Err("The map tile is too large.".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("The map provider returned an unreadable tile.".into());
    }
    let target = path.clone();
    tokio::task::spawn_blocking(move || -> std::io::Result<()> {
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let sequence = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let temporary = target.with_extension(format!("{}.{sequence}.tmp", std::process::id()));
        std::fs::write(&temporary, bytes)?;
        std::fs::rename(temporary, target)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| format!("Cache map: {e}"))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mercator_round_trip_and_poles_stay_finite() {
        for latitude in [-90., -85., -33.8, 0., 52.5, 85., 90.] {
            let at = Coordinate::new(latitude, 13.4).unwrap();
            let (x, y) = at.world(12);
            assert!(x.is_finite() && y.is_finite());
            let back = Coordinate::from_world(x, y, 12);
            assert!((back.longitude - 13.4).abs() < 1e-8);
            assert!((back.latitude - latitude.clamp(-85.05112878, 85.05112878)).abs() < 1e-6);
        }
        assert!(Coordinate::new(f64::NAN, 0.).is_none());
        assert!(Coordinate::new(0., f64::INFINITY).is_none());
        assert!(Coordinate::new(91., 0.).is_none());
    }

    #[test]
    fn zooming_about_a_cursor_keeps_the_ground_under_it() {
        let center = Coordinate::new(50.8, 8.0).unwrap();
        let viewport = (800., 440.);
        let cursor = (620., 120.);
        let offset = (620. - 400., 120. - 220.);
        let ground = |center: Coordinate, zoom: f64| {
            let (cx, cy) = center.world_at(zoom);
            Coordinate::from_world_at(cx + offset.0, cy + offset.1, zoom)
        };

        let before = ground(center, 7.5);
        let (moved, zoom) = zoom_about(center, 7.5, cursor, viewport, 1.4, 0.);
        assert!((zoom - 8.9).abs() < 1e-9);
        let after = ground(moved, zoom);
        assert!((after.latitude - before.latitude).abs() < 1e-6);
        assert!((after.longitude - before.longitude).abs() < 1e-6);

        // Zooming out about the same point lands on the same ground too.
        let (back, zoom) = zoom_about(moved, zoom, cursor, viewport, -1.4, 0.);
        assert!((zoom - 7.5).abs() < 1e-9);
        let back = ground(back, zoom);
        assert!((back.latitude - before.latitude).abs() < 1e-6);
        assert!((back.longitude - before.longitude).abs() < 1e-6);
    }

    #[test]
    fn zoom_stops_at_its_limits_and_a_small_viewport_does_not_move_the_map() {
        let center = Coordinate::new(50.8, 8.0).unwrap();
        let (at_top, zoom) = zoom_about(center, 17.5, (10., 10.), (800., 440.), 5., 1.);
        assert_eq!(zoom, MAX_ZOOM);
        assert!(at_top.latitude.is_finite() && at_top.longitude.is_finite());
        let (at_bottom, zoom) = zoom_about(center, 1.5, (10., 10.), (800., 440.), -5., 1.);
        assert_eq!(zoom, 1.);
        assert!(at_bottom.latitude.is_finite() && at_bottom.longitude.is_finite());
        // Already at the floor: there is no zoom left to give, so nothing moves at all.
        let (still, zoom) = zoom_about(center, 1., (10., 10.), (800., 440.), -5., 1.);
        assert_eq!((still, zoom), (center, 1.));
        // A map that has not been laid out yet has no cursor geometry to honour.
        let (still, zoom) = zoom_about(center, 7., (10., 10.), (0., 0.), 1., 0.);
        assert_eq!((still, zoom), (center, 8.));
    }

    #[test]
    fn a_coarse_ancestor_covers_a_missing_tile_at_double_size() {
        let center = Coordinate::new(50.8, 8.0).unwrap();
        let (width, height) = (800., 440.);
        let level = 9;
        for (tile, x, y) in visible_tiles(center, level, width, height) {
            let placed = tile.place(center, f64::from(level), width, height);
            assert_eq!(placed.tile, tile);
            assert!((placed.x - x).abs() < 0.001 && (placed.y - y).abs() < 0.001);
            assert!((placed.size - 256.).abs() < 0.001);

            // The parent covers this tile exactly when the scale is doubled.
            let parent = tile.ancestor(1).expect("parent level exists");
            let placed_parent = parent.place(center, f64::from(level), width, height);
            assert!((placed_parent.size - 512.).abs() < 0.001);
            assert!((placed_parent.x - (x - (tile.x % 2) as f32 * 256.)).abs() < 0.001);
            assert!((placed_parent.y - (y - (tile.y % 2) as f32 * 256.)).abs() < 0.001);
        }
        assert!(Tile {
            zoom: 0,
            x: 0,
            y: 0
        }
        .ancestor(1)
        .is_none());
        assert_eq!(
            Tile {
                zoom: 5,
                x: 13,
                y: 7
            }
            .ancestor(2),
            Some(Tile {
                zoom: 3,
                x: 3,
                y: 1
            })
        );
    }

    #[test]
    fn a_fractional_zoom_scales_the_exact_level_around_its_own_anchors() {
        let center = Coordinate::new(50.8, 8.0).unwrap();
        let tile = Tile {
            zoom: 9,
            x: 271,
            y: 168,
        };
        let whole = tile.place(center, 9., 800., 440.);
        let up = tile.place(center, 10., 800., 440.);
        assert!((up.size - 512.).abs() < 0.001);
        // Same tile, one zoom in: twice as wide and twice as far from the centre of the map.
        assert!(((up.x - whole.x) - (whole.x - 400.)).abs() < 0.001);
    }

    #[test]
    fn fitting_uses_short_dateline_arc_and_keeps_stops_in_view() {
        let places = [
            Coordinate::new(35., 179.).unwrap(),
            Coordinate::new(37., -179.).unwrap(),
        ];
        let (center, zoom) = fit(&places, 320., 320.).unwrap();
        assert!(center.longitude.abs() > 179.);
        assert!(zoom >= 6);
        let world = 256. * 2_f64.powi(i32::from(zoom));
        let (cx, cy) = center.world(zoom);
        for point in places {
            let (x, y) = point.world(zoom);
            let dx = (x - cx + world / 2.).rem_euclid(world) - world / 2.;
            assert!(dx.abs() <= 120. && (y - cy).abs() <= 120.);
        }
        assert!(fit(&[], 320., 320.).is_none());
    }

    #[test]
    fn viewport_wraps_dateline_and_clips_poles() {
        for at in [
            Coordinate::new(0., 180.).unwrap(),
            Coordinate::new(90., -180.).unwrap(),
        ] {
            let tiles = visible_tiles(at, 3, 768., 320.);
            assert!(!tiles.is_empty());
            for (tile, x, y) in tiles {
                assert!(tile.x < 8 && tile.y < 8);
                assert!(x < 768. && x + 256. > 0.);
                assert!(y < 320. && y + 256. > 0.);
            }
        }
        let tiles = visible_tiles(Coordinate::new(0., 180.).unwrap(), 3, 320., 320.);
        assert!(tiles.iter().any(|(tile, _, _)| tile.x == 0));
        assert!(tiles.iter().any(|(tile, _, _)| tile.x == 7));
    }

    #[test]
    fn native_plan_reads_the_capability_wire_shape() {
        let plan: Plan = serde_json::from_value(serde_json::json!({
            "id": "plan-1", "title": "Trip", "status": "planning",
            "date_start": "2026-11-01", "date_end": "2026-11-04",
            "origin": {"id": "a", "name": "Origin", "kind": "city", "latitude": null, "longitude": null},
            "destinations": [{"id": "b", "name": "Destination", "kind": "city", "latitude": 48.1, "longitude": 11.5}],
            "stages": [], "travelers": [], "budget_cents": null
        })).unwrap();
        assert_eq!(plan.places().filter_map(Place::coordinate).count(), 1);
        assert_eq!(
            serde_json::from_slice::<Plan>(&serde_json::to_vec(&plan).unwrap())
                .unwrap()
                .id,
            "plan-1"
        );
    }

    #[test]
    fn resolved_coordinates_fill_typed_places_and_never_override_saved_ones() {
        let mut plan: Plan = serde_json::from_value(serde_json::json!({
            "id": "plan-1", "title": "Trip", "status": "saved",
            "date_start": "2026-10-15", "date_end": "2026-10-18",
            "origin": {"id": "place:bonn", "name": "Bonn", "latitude": null, "longitude": null},
            "destinations": [{"id": "place:hamburg", "name": "Hamburg", "kind": "city", "latitude": 53.5, "longitude": 10.0}],
            "stages": [{
                "sequence": 0, "date": "2026-10-15", "status": "booked",
                "origin": {"id": "place:bonn", "name": "Bonn", "latitude": null, "longitude": null},
                "destination": {"id": "place:koeln", "name": "Köln", "latitude": null, "longitude": null}
            }],
            "travelers": [], "budget_cents": null
        }))
        .unwrap();
        let mut coords = PlaceCoords::default();
        coords.insert(
            &plan.origin,
            Coordinate::new(50.733, 7.099).expect("coordinate"),
        );
        coords.insert(
            &plan.stages[0].destination,
            Coordinate::new(50.938, 6.96).expect("coordinate"),
        );
        coords.insert(
            &plan.destinations[0],
            Coordinate::new(0., 0.).expect("coordinate"),
        );

        assert_eq!(plan.fill_coordinates(&coords), 3);
        assert_eq!(plan.origin.coordinate(), Coordinate::new(50.733, 7.099));
        assert_eq!(
            plan.stages[0].destination.coordinate(),
            Coordinate::new(50.938, 6.96)
        );
        // The saved destination keeps what the record said, not what the cache offers.
        assert_eq!(
            plan.destinations[0].coordinate(),
            Coordinate::new(53.5, 10.0)
        );
        // A second pass has nothing left to do.
        assert_eq!(plan.fill_coordinates(&coords), 0);
        assert_eq!(plan.connections().len(), 1);
    }
}
