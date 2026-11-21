//! Phone travel reads, saved trips, itinerary and connection search, independent of GPUI.
use crate::phone_navigation::{open as set_nsh_page, Page as NshPage};

use crate::nsh_map::{self, MapEvent, MapScene};
use sjel_gpui_core::{
    shell,
    travel::{
        self,
        connections::{Journey, Query, SplitResult, Station},
        markers, CostRollup, Plan, PlanExt, SavedDetails, Source, TravelRoute, TripSection,
    },
};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use tokio::sync::watch;

#[derive(Clone, Default)]
struct TripRead {
    saved: Option<SavedDetails>,
    error: Option<String>,
    cost: Option<CostRollup>,
    calendar: Vec<sjel_gpui_core::shell::CalendarEntry>,
    links: Vec<sjel_gpui_core::record::Link>,
}

#[derive(Clone, Default)]
struct Snapshot {
    route: TravelRoute,
    plans: Vec<Plan>,
    details: HashMap<String, TripRead>,
    loading: bool,
    error: Option<String>,
    saved_at: Option<u64>,
    status: String,
    journeys: Vec<Journey>,
    split: Option<Result<SplitResult, String>>,
    searching: bool,
    search_error: Option<String>,
    suggestions: [Vec<Station>; 2],
    offline: HashMap<String, String>,
}

struct State {
    snap: Snapshot,
    source: Option<Source>,
    generation: u64,
    search_generation: u64,
    suggestions_generation: [u64; 2],
    coords: travel::PlaceCoords,
}
struct Feed {
    state: Mutex<State>,
    tx: watch::Sender<Snapshot>,
}
fn feed() -> &'static Feed {
    static FEED: OnceLock<Feed> = OnceLock::new();
    FEED.get_or_init(|| {
        let snap = Snapshot::default();
        let (tx, _) = watch::channel(snap.clone());
        Feed {
            state: Mutex::new(State {
                snap,
                source: None,
                generation: 0,
                search_generation: 0,
                suggestions_generation: [0; 2],
                coords: Default::default(),
            }),
            tx,
        }
    })
}
fn state() -> std::sync::MutexGuard<'static, State> {
    feed().state.lock().unwrap_or_else(|e| e.into_inner())
}
fn publish(s: &State) {
    feed().tx.send_replace(s.snap.clone());
}

/// Supplies the signed device source. Successful trip reads keep the existing offline cache.
pub fn connect(source: Source) {
    {
        let mut s = state();
        s.source = Some(source);
        s.generation += 1;
    }
    load();
}

/// Opens a native travel route and loads its saved data before contacting the Mac.
pub fn open(route: TravelRoute) {
    {
        let mut s = state();
        s.snap.route = route.clone();
        publish(&s);
    }
    if matches!(
        route,
        TravelRoute::Map
            | TravelRoute::Trip {
                section: TripSection::Map,
                ..
            }
    ) {
        sync_map();
        nsh_map::show(None);
    } else {
        set_nsh_page(NshPage::Travel);
    }
    load();
}

/// Refreshes trips, retaining saved values when the paired Mac cannot answer.
pub fn load() {
    let (source, generation) = {
        let mut s = state();
        s.generation += 1;
        s.snap.loading = true;
        s.snap.error = None;
        publish(&s);
        (s.source.clone(), s.generation)
    };
    shell::spawn(async move {
        let cached = travel::saved_plans().await;
        let coords = travel::places::load().await;
        {
            let mut s = state();
            if s.generation != generation {
                return;
            }
            if let Ok(coords) = coords {
                s.coords = coords;
            }
            if s.snap.plans.is_empty() {
                if let Ok(Some(saved)) = cached {
                    s.snap.plans = saved.plans;
                    s.snap.saved_at = Some(saved.saved_at);
                    s.snap.status = "Saved trips · checking for updates".into();
                }
            }
            let coords = s.coords.clone();
            for plan in &mut s.snap.plans {
                plan.fill_coordinates(&coords);
            }
            publish(&s);
        }
        sync_map();
        let result = match source.as_ref() {
            Some(source) => source.plans().await,
            None => Err("Connect to the paired Mac to update saved trips.".into()),
        };
        let saved = if let Ok(plans) = &result {
            travel::save_plans(plans).await
        } else {
            Ok(())
        };
        let ids = {
            let mut s = state();
            if s.generation != generation {
                return;
            }
            s.snap.loading = false;
            match result {
                Ok(mut plans) => {
                    plans.retain(|p| p.status != "archived");
                    for plan in &mut plans {
                        plan.fill_coordinates(&s.coords);
                    }
                    plans.sort_by(|a, b| a.date_start.cmp(&b.date_start));
                    s.snap.plans = plans;
                    s.snap.error = saved.err();
                    if s.snap.error.is_none() {
                        s.snap.saved_at = Some(chrono::Utc::now().timestamp().max(0) as u64);
                    }
                    s.snap.status = if s.snap.error.is_some() {
                        "Trips updated · saving failed"
                    } else {
                        "Trips updated · saved on this device"
                    }
                    .into();
                }
                Err(error) => {
                    s.snap.error = Some(error);
                    s.snap.status = "Showing saved trips".into();
                }
            }
            let ids = match &s.snap.route {
                TravelRoute::Trip { id, .. } => vec![id.clone()],
                TravelRoute::Map => s
                    .snap
                    .plans
                    .iter()
                    .filter(|p| {
                        travel::upcoming(p, &chrono::Local::now().format("%Y-%m-%d").to_string())
                    })
                    .take(8)
                    .map(|p| p.id.clone())
                    .collect(),
                _ => Vec::new(),
            };
            publish(&s);
            ids
        };
        sync_map();
        for id in ids {
            read_trip(id, source.clone(), generation).await;
        }
    });
}

async fn read_trip(id: String, source: Option<Source>, generation: u64) {
    if let Ok(Some(saved)) = travel::saved_details(&id).await {
        let mut s = state();
        if s.generation != generation {
            return;
        }
        let mut saved = saved;
        saved.details.plan.fill_coordinates(&s.coords);
        s.snap.details.entry(id.clone()).or_default().saved = Some(saved);
        publish(&s);
    }
    sync_map();
    let Some(source) = source else {
        return;
    };
    let result = source.details(&id).await;
    let mut errors = Vec::new();
    let (cost, calendar, links) = if let Ok(details) = &result {
        let (cost, calendar, links) = futures::join!(
            source.cost(&id),
            source.related_calendar_events(&details.plan),
            source.calendar_plan_links(&id)
        );
        if let Err(e) = &cost {
            errors.push(format!("Cost unavailable: {e}"));
        }
        if let Err(e) = &calendar {
            errors.push(format!("Calendar unavailable: {e}"));
        }
        if let Err(e) = &links {
            errors.push(format!("Calendar links unavailable: {e}"));
        }
        (
            cost.ok(),
            calendar.unwrap_or_default(),
            links.map(|l| l.links).unwrap_or_default(),
        )
    } else {
        (None, Vec::new(), Vec::new())
    };
    let saved = match result {
        Ok(details) => match travel::save_details(details.clone()).await {
            Ok(saved) => Ok(saved),
            Err(e) => {
                errors.push(e);
                Ok(SavedDetails {
                    details,
                    saved_at: 0,
                })
            }
        },
        Err(e) => Err(e),
    };
    let mut s = state();
    if s.generation != generation {
        return;
    }
    let coords = s.coords.clone();
    let slot = s.snap.details.entry(id).or_default();
    match saved {
        Ok(mut saved) => {
            saved.details.plan.fill_coordinates(&coords);
            slot.saved = Some(saved);
            slot.error = (!errors.is_empty()).then(|| errors.join(" · "));
        }
        Err(e) => slot.error = Some(e),
    }
    slot.cost = cost;
    slot.calendar = calendar;
    slot.links = links;
    publish(&s);
    drop(s);
    sync_map();
}

fn download(id: String) {
    let (source, generation, plan) = {
        let mut s = state();
        if s.snap.offline.contains_key(&id) {
            return;
        }
        let Some(plan) = s
            .snap
            .details
            .get(&id)
            .and_then(|r| r.saved.as_ref())
            .map(|r| r.details.plan.clone())
            .or_else(|| s.snap.plans.iter().find(|p| p.id == id).cloned())
        else {
            return;
        };
        s.snap
            .offline
            .insert(id.clone(), "Preparing offline pack…".into());
        publish(&s);
        (s.source.clone(), s.generation, plan)
    };
    shell::spawn(async move {
        read_trip(id.clone(), source, generation).await;
        let missing: Vec<_> = plan
            .offline_tiles()
            .into_iter()
            .filter(|&tile| !travel::tile_cached(tile))
            .collect();
        let total = missing.len();
        for (index, tile) in missing.into_iter().enumerate() {
            let result = travel::load_tile(tile, true).await;
            let mut s = state();
            if let Err(error) = result {
                s.snap.error = Some(error);
            }
            s.snap.offline.insert(
                id.clone(),
                format!("Saving map tiles · {} / {total}", index + 1),
            );
            publish(&s);
        }
        let mut s = state();
        s.snap.offline.remove(&id);
        publish(&s);
    });
}

fn sync_map() {
    let scene = {
        let s = state();
        let details: HashMap<_, _> = s
            .snap
            .details
            .iter()
            .filter_map(|(id, r)| r.saved.as_ref().map(|v| (id.clone(), v)))
            .collect();
        let (title, markers, connections) = match &s.snap.route {
            TravelRoute::Map => (
                "Map & Places".to_owned(),
                markers::build_location_markers(&s.snap.plans, &details, None),
                s.snap.plans.iter().flat_map(Plan::connections).collect(),
            ),
            TravelRoute::Trip {
                id,
                section: TripSection::Map,
            } => {
                let plan = details
                    .get(id)
                    .map(|d| &d.details.plan)
                    .or_else(|| s.snap.plans.iter().find(|p| &p.id == id));
                let Some(plan) = plan else {
                    return;
                };
                (
                    plan.title.clone(),
                    markers::build_current_plan_markers(plan, details.get(id).copied(), None),
                    plan.connections(),
                )
            }
            _ => return,
        };
        MapScene {
            route: s.snap.route.clone(),
            title,
            markers,
            connections,
            status: s.snap.status.clone(),
            error: s.snap.error.clone(),
            loading: s.snap.loading,
        }
    };
    nsh_map::set_scene(scene);
}

/// Handles the map's actions after Travel no longer has a GPUI controller.
pub fn map_event(event: MapEvent) {
    match event {
        MapEvent::Show => set_nsh_page(NshPage::Map),
        MapEvent::Open(route) => open(route),
        MapEvent::Refresh => load(),
        MapEvent::Resolve(id) => resolve(id),
    }
}

fn resolve(id: String) {
    let (source, generation) = {
        let s = state();
        (s.source.clone(), s.generation)
    };
    let Some(source) = source else {
        return;
    };
    shell::spawn(async move {
        let result = source.resolve_places(&id, true).await;
        if let Err(e) = result {
            let mut s = state();
            if s.generation == generation {
                s.snap.error = Some(e);
                publish(&s);
            }
        } else {
            read_trip(id, Some(source), generation).await;
        }
        sync_map();
    });
}

fn suggest(end: usize, query: String) {
    let (source, generation) = {
        let mut s = state();
        s.suggestions_generation[end] += 1;
        (s.source.clone(), s.suggestions_generation[end])
    };
    let Some(source) = source else {
        return;
    };
    shell::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        if state().suggestions_generation[end] != generation {
            return;
        }
        let result = source.stations(&query).await;
        let mut s = state();
        if s.suggestions_generation[end] != generation {
            return;
        }
        match result {
            Ok(stations) => s.snap.suggestions[end] = stations,
            Err(e) => s.snap.search_error = Some(e),
        }
        publish(&s);
    });
}

fn search(query: Query) {
    if let Err(e) = query.search_path() {
        let mut s = state();
        s.snap.search_error = Some(e);
        publish(&s);
        return;
    }
    let (source, generation) = {
        let mut s = state();
        s.search_generation += 1;
        s.snap.searching = true;
        s.snap.search_error = None;
        publish(&s);
        (s.source.clone(), s.search_generation)
    };
    shell::spawn(async move {
        let (journeys, split) = match source {
            Some(source) => futures::join!(source.connections(&query), source.split(&query)),
            None => (
                Err("Connect to the paired Mac to search connections.".into()),
                Err("The Mac is unavailable.".into()),
            ),
        };
        let mut s = state();
        if s.search_generation != generation {
            return;
        }
        s.snap.searching = false;
        s.snap.split = Some(split);
        match journeys {
            Ok(journeys) => s.snap.journeys = journeys,
            Err(e) => s.snap.search_error = Some(e),
        }
        publish(&s);
    });
}

#[cfg(target_os = "ios")]
mod phone;
#[cfg(target_os = "ios")]
pub use phone::PhoneTravel;
