//! Phone travel reads, saved trips, itinerary and connection search, independent of GPUI.
use crate::{
    nsh_documents::{self, NshPage},
    nsh_map::{self, MapEvent, MapScene},
    nsh_ui::{action, caption, field, heading, snapshot},
};
use nsh_core::prelude::*;
use sjel_gpui_core::{
    shell,
    travel::{
        self,
        connections::{self, Journey, Query, Rank, Sort, SplitResult, Station},
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
        nsh_documents::set_nsh_page(NshPage::Travel);
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
        MapEvent::Show => nsh_documents::set_nsh_page(NshPage::Map),
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

fn item(item: &travel::Item) -> impl IntoElement {
    let mut row = view().gap_1().py_3().child(heading(item.title.clone()));
    row = row.child(caption(format!(
        "{} · {}",
        item.item_type,
        item.day.as_deref().unwrap_or("Undated")
    )));
    for (key, value) in item.fields() {
        row = row.child(text(format!("{key}: {value}")).text_sm());
    }
    if let Some(code) = item.booking_code() {
        let code = code.to_owned();
        row = row.child(action(format!("Copy booking code · {code}"), move || {
            set_clipboard(code.clone())
        }));
    }
    if let Some(url) = item.source_url() {
        row = row.child(action("Open source", move || {
            drop(open_url(url.to_string()))
        }));
    }
    if item.item_type == "packing" {
        row = row.child(caption(if item.is_packed() {
            "Packed"
        } else {
            "Still to pack"
        }));
    }
    row
}

fn trip_view(
    plan: &Plan,
    read: Option<&TripRead>,
    section: TripSection,
    day: Signal<Option<String>>,
) -> impl IntoElement {
    let mut out = view()
        .gap_3()
        .child(heading(plan.title.clone()))
        .child(caption(format!(
            "{}–{} · {}",
            plan.date_start, plan.date_end, plan.status
        )));
    let mut tabs = view().flex_row().flex_wrap();
    for tab in TripSection::ALL {
        let id = plan.id.clone();
        tabs = tabs.child(action(tab.label(), move || {
            open(TravelRoute::Trip {
                id: id.clone(),
                section: tab,
            })
        }));
    }
    out = out.child(tabs);
    let readiness = plan.check_offline_readiness();
    out = out.child(caption(if readiness.is_ready() {
        "Offline ready".into()
    } else {
        format!("Offline: {}%", readiness.percent())
    }));
    let id = plan.id.clone();
    if let Some(progress) = state().snap.offline.get(&id) {
        out = out.child(caption(progress.clone()));
    } else if !readiness.is_ready() {
        out = out.child(action("Download offline pack", move || {
            download(id.clone())
        }));
    }
    let id = plan.id.clone();
    out = out.child(action("Locate stops", move || resolve(id.clone())));
    let Some(read) = read else {
        return out.child(caption("Reading trip details…"));
    };
    if let Some(error) = &read.error {
        out = out.child(text(error.clone()).text_sm());
    }
    let Some(saved) = &read.saved else {
        return out;
    };
    let details = &saved.details;
    if section == TripSection::Overview {
        out = out.child(
            text(format!(
                "{} → {}",
                plan.origin.name,
                plan.destinations
                    .iter()
                    .map(|p| p.name.as_str())
                    .collect::<Vec<_>>()
                    .join(" · ")
            ))
            .text_base(),
        );
        if !plan.travelers.is_empty() {
            out = out.child(caption(format!("Travelers: {}", plan.travelers.join(", "))));
        }
        out = out
            .child(text(plan.interests.clone()).text_sm())
            .child(caption(format!(
                "Saved {}",
                (saved.saved_at > 0)
                    .then(|| chrono::DateTime::from_timestamp(saved.saved_at as i64, 0))
                    .flatten()
                    .map(|t| t.to_rfc3339())
                    .unwrap_or_else(|| "Not saved on this device".into())
            )));
        if let Some(cost) = &read.cost {
            for total in &cost.by_currency {
                out = out.child(
                    text(format!(
                        "Booked: {:.2} {}",
                        total.booked_cents as f64 / 100.,
                        total.currency
                    ))
                    .text_sm(),
                );
            }
            if let (Some(cents), Some(currency)) =
                (cost.actuals.personal_cents, &cost.actuals.currency)
            {
                out = out.child(
                    text(format!(
                        "Actual personal cost: {:.2} {currency}",
                        cents as f64 / 100.
                    ))
                    .text_sm(),
                );
            }
            if let Some(reason) = &cost.actuals.reason {
                out = out.child(caption(reason.clone()));
            }
        }
        for entry in &read.calendar {
            let date = entry.starts_at.get(..10).and_then(travel_day);
            let title = entry.title.clone();
            out = out.child(action(format!("Calendar · {title}"), move || {
                if let Some(date) = date {
                    crate::nsh_calendar::open_day(date);
                    nsh_documents::set_nsh_page(NshPage::Calendar);
                }
            }));
        }
        for link in &read.links {
            out = out.child(caption(format!("{} · {}", link.title, link.id)));
        }
    } else if section == TripSection::Itinerary {
        out = out.child(itinerary(plan, details, read, day));
    } else {
        for entry in &details.items {
            out = out.child(item(entry));
        }
    }
    out
}
fn calendar_entry(entry: &sjel_gpui_core::shell::CalendarEntry) -> impl IntoElement {
    let date = entry.starts_at.get(..10).and_then(travel_day);
    let mut row = view()
        .gap_1()
        .py_2()
        .child(action(entry.title.clone(), move || {
            if let Some(date) = date {
                crate::nsh_calendar::open_day(date);
                nsh_documents::set_nsh_page(NshPage::Calendar);
            }
        }))
        .child(caption(if entry.all_day {
            "All day".into()
        } else {
            format!("{}–{}", entry.starts_at, entry.ends_at)
        }));
    if let Some(location) = &entry.location {
        row = row.child(caption(location.clone()));
    }
    if let Some(notes) = &entry.notes {
        row = row.child(nsh_base::text_view().markdown(notes.clone()));
    }
    row
}

fn itinerary(
    plan: &Plan,
    details: &travel::Details,
    read: &TripRead,
    day: Signal<Option<String>>,
) -> impl IntoElement {
    use travel::itinerary::{find_place_for_name, parse_transit_title};
    let mut days: Vec<_> = details
        .items
        .iter()
        .filter_map(|i| i.day.clone())
        .chain(plan.stages.iter().filter_map(|s| s.date.clone()))
        .chain(
            read.calendar
                .iter()
                .filter_map(|e| e.starts_at.get(..10).map(str::to_owned)),
        )
        .chain(
            read.links
                .iter()
                .filter_map(|l| l.at.as_ref().and_then(|a| a.get(..10)).map(str::to_owned)),
        )
        .collect();
    days.push(plan.date_start.clone());
    days.push(plan.date_end.clone());
    days.sort();
    days.dedup();
    let mut filter = view()
        .flex_row()
        .flex_wrap()
        .child(action("All days", move || day.set(None)));
    for date in &days {
        let value = date.clone();
        filter = filter.child(action(date.clone(), move || day.set(Some(value.clone()))));
    }
    let mut out = view().gap_3().child(filter);
    for date in days {
        if day.get().as_ref().is_some_and(|d| d != &date) {
            continue;
        }
        let stages: Vec<_> = plan
            .stages
            .iter()
            .filter(|s| s.date.as_deref() == Some(&date))
            .collect();
        let mut events: Vec<_> = read
            .calendar
            .iter()
            .filter(|e| e.starts_at.starts_with(&date))
            .collect();
        events.sort_by(|a, b| a.starts_at.cmp(&b.starts_at));
        let mut items: Vec<_> = details
            .items
            .iter()
            .filter(|i| i.day.as_deref() == Some(&date))
            .collect();
        items.sort_by(|a, b| {
            a.payload
                .get("time")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .cmp(b.payload.get("time").and_then(|v| v.as_str()).unwrap_or(""))
        });
        let mut card = view()
            .gap_2()
            .p_3()
            .bg(|| nsh_base::theme().card)
            .child(heading(
                travel_day(&date)
                    .map(|d| d.format("%A, %-d %B").to_string())
                    .unwrap_or(date.clone()),
            ))
            .child(caption(format!(
                "{} journeys · {} entries · {} bookings",
                stages.len(),
                events.len(),
                items.len()
            )));
        let title_matches = |title: &str, stage: &travel::Stage| {
            parse_transit_title(title).is_some_and(|(from, to)| {
                let from = from.to_lowercase();
                let to = to.to_lowercase();
                let origin = stage.origin.name.to_lowercase();
                let destination = stage.destination.name.to_lowercase();
                !origin.is_empty()
                    && !destination.is_empty()
                    && (from.contains(&origin) || origin.contains(&from))
                    && (to.contains(&destination) || destination.contains(&to))
            })
        };
        for (index, stage) in stages.iter().enumerate() {
            card = card
                .child(heading(format!(
                    "{} → {}",
                    stage.origin.name, stage.destination.name
                )))
                .child(caption(format!(
                    "{} · {}",
                    stage
                        .transport_modes
                        .iter()
                        .map(|m| m.as_str())
                        .collect::<Vec<_>>()
                        .join(" / "),
                    stage.status.as_str()
                )));
            if let Some(note) = &stage.branch_note {
                card = card.child(caption(note.clone()));
            }
            if let Some(event) = events.iter().find(|e| title_matches(&e.title, stage)) {
                card = card.child(calendar_entry(event));
            }
            for booking in items.iter().filter(|i| title_matches(&i.title, stage)) {
                card = card.child(item(booking));
            }
            if let Some(next) = stages.get(index + 1) {
                card = card.child(caption(if stage.destination.name == next.origin.name {
                    format!("Transfer at {} · same station", stage.destination.name)
                } else {
                    format!(
                        "Transfer buffer required · {} → {}",
                        stage.destination.name, next.origin.name
                    )
                }));
            }
            let id = plan.id.clone();
            let place = stage.destination.clone();
            if let Some(at) = place.coordinate() {
                card = card.child(action("Show destination on map", move || {
                    open(TravelRoute::Trip {
                        id: id.clone(),
                        section: TripSection::Map,
                    });
                    nsh_map::show(Some(at));
                }));
            }
        }
        for event in events
            .iter()
            .filter(|e| !stages.iter().any(|s| title_matches(&e.title, s)))
        {
            card = card.child(calendar_entry(event));
        }
        for booking in items
            .iter()
            .filter(|i| !stages.iter().any(|s| title_matches(&i.title, s)))
        {
            if !events.iter().any(|e| e.title == booking.title) {
                card = card.child(item(booking));
            } else {
                // Keep booking fields even when Calendar supplies its schedule.
                if booking.booking_code().is_some() {
                    card = card.child(item(booking));
                }
            }
            let location = booking
                .payload
                .get("address")
                .and_then(|v| v.as_str())
                .or_else(|| booking.payload.get("location").and_then(|v| v.as_str()));
            if let Some(at) = location
                .and_then(|l| find_place_for_name(l, plan))
                .and_then(|p| p.coordinate())
            {
                let id = plan.id.clone();
                card = card.child(action("Show booking on map", move || {
                    open(TravelRoute::Trip {
                        id: id.clone(),
                        section: TripSection::Map,
                    });
                    nsh_map::show(Some(at));
                }));
            }
        }
        for link in read.links.iter().filter(|l| {
            l.at.as_ref().is_some_and(|a| a.starts_with(&date))
                && !events.iter().any(|e| e.id == l.id)
        }) {
            card = card.child(caption(format!(
                "{} · {}",
                link.at.as_deref().unwrap_or(""),
                link.title
            )));
        }
        out = out.child(card);
    }
    let undated: Vec<_> = details.items.iter().filter(|i| i.day.is_none()).collect();
    if !undated.is_empty() {
        out = out.child(heading("Undated / packing"));
        for entry in undated {
            out = out.child(item(entry));
        }
    }
    out
}

fn travel_day(value: &str) -> Option<chrono::NaiveDate> {
    chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d").ok()
}

#[component]
fn ConnectionsScreen() -> impl IntoElement {
    let snap = snapshot(&feed().tx);
    let from = signal(String::new());
    let to = signal(String::new());
    let picked = signal([None::<Station>, None::<Station>]);
    let date = signal(connections::tomorrow());
    let time = signal("10:00".to_owned());
    let phrase = signal(String::new());
    let priority = signal(None::<&'static str>);
    let sort = signal(Sort::Ranked);
    let split = signal(false);
    let mut presets = view().flex_row().flex_wrap();
    for (key, label, _) in connections::PRIORITIES {
        presets = presets.child(action(label, move || priority.set(Some(key))));
    }
    let station_field = move |end: usize, label, value: Signal<String>| {
        view()
            .child(text(label).text_xs())
            .child(
                nsh_base::input()
                    .test_id(if end == 0 { "station-0" } else { "station-1" })
                    .value(move || value.get())
                    .on_change(move |v: &str| {
                        value.set(v.to_owned());
                        picked.update(|p| p[end] = None);
                        suggest(end, v.to_owned());
                    }),
            )
            .child(dynamic(move || {
                let mut choices = view();
                if picked.with(|p| p[end].is_some()) {
                    return choices;
                }
                for station in snap.with(|s| s.suggestions[end].clone()) {
                    choices = choices.child(action(station.name.clone(), move || {
                        value.set(station.name.clone());
                        picked.update(|p| p[end] = Some(station.clone()));
                    }));
                }
                choices
            }))
    };
    let mut sorts = view().flex_row().flex_wrap();
    for value in Sort::ALL {
        sorts = sorts.child(action(value.label(), move || sort.set(value)));
    }
    view()
        .gap_2()
        .child(heading("Connections"))
        .child(station_field(0, "From station", from))
        .child(station_field(1, "To station", to))
        .child(field("Date · YYYY-MM-DD", date))
        .child(field("Time · HH:MM", time))
        .child(field("Ranking request", phrase))
        .child(presets)
        .child(action("Search", move || {
            let stations = picked.get();
            if let (Some(from), Some(to)) = (stations[0].clone(), stations[1].clone()) {
                search(Query {
                    from,
                    to,
                    date: date.get(),
                    time: time.get(),
                    rank: Rank {
                        priority: priority.get(),
                        phrase: phrase.get(),
                    },
                });
            } else {
                let mut s = state();
                s.snap.search_error = Some("Pick both stations from the suggestions.".into());
                publish(&s);
            }
        }))
        .child(sorts)
        .child(
            view()
                .flex_row()
                .child(action("Direct", move || split.set(false)))
                .child(action("Split tickets", move || split.set(true))),
        )
        .child(dynamic(move || {
            let current = snap.get();
            let mut out = view().gap_2();
            if current.searching {
                out = out.child(caption("Searching…"));
            }
            if let Some(error) = current.search_error {
                out = out.child(text(error).text_sm());
            }
            if split.get() {
                if let Some(result) = current.split {
                    match result {
                        Err(error) => out = out.child(caption(error)),
                        Ok(result) => {
                            out = out.child(heading(format!(
                                "Split fare · {:.2} EUR",
                                result.split_price
                            )));
                            if let Some(note) = result.confidence.note() {
                                out = out.child(text(note).text_sm());
                            }
                            for segment in result.segments {
                                out = out.child(caption(format!(
                                    "{} → {} · {}",
                                    segment.journey.start_station.name,
                                    segment.journey.end_station.name,
                                    connections::price(segment.journey.total_price)
                                )));
                            }
                        }
                    }
                }
            } else {
                for journey in connections::ordered(&current.journeys, sort.get()) {
                    out = out
                        .child(heading(format!(
                            "{}–{} · {}",
                            journey.departs(),
                            journey.arrives(),
                            connections::price(journey.total_price)
                        )))
                        .child(caption(format!(
                            "{} · {} changes",
                            connections::duration(journey.total_duration_minutes),
                            journey.changes()
                        )));
                    for leg in &journey.legs {
                        out = out.child(
                            text(format!(
                                "{} {} → {} {} · {}",
                                connections::clock(&leg.departure_time),
                                leg.origin.name,
                                connections::clock(&leg.arrival_time),
                                leg.destination.name,
                                leg.train_name
                            ))
                            .text_sm(),
                        );
                    }
                    if let Some(rank) = &journey.ranking {
                        for factor in &rank.factors {
                            out = out.child(caption(factor.label.clone()));
                        }
                    }
                    if let Some(late) = connections::late(journey.delay_risk_score) {
                        out = out.child(caption(late));
                    }
                }
            }
            out
        }))
}

/// Trips, per-trip views and connection search; map routes use the shared map screen.
#[component]
fn TravelContent() -> impl IntoElement {
    let snap = snapshot(&feed().tx);
    let past = signal(false);
    let day = signal(None::<String>);
    view()
        .flex_1()
        .child(scroll().flex_1().child(dynamic(move || {
            let current = snap.get();
            let mut out = view().gap_2().px_4();
            if current.loading {
                out = out.child(caption("Reading trips…"));
            }
            if let Some(error) = &current.error {
                out = out.child(text(error.clone()).text_sm());
            }
            match &current.route {
                TravelRoute::Trip { id, section } => {
                    let read = current.details.get(id);
                    if let Some(plan) = read
                        .and_then(|r| r.saved.as_ref())
                        .map(|d| &d.details.plan)
                        .or_else(|| current.plans.iter().find(|p| &p.id == id))
                    {
                        out = out.child(trip_view(plan, read, *section, day));
                    } else {
                        out = out.child(caption("This trip has not been saved on the phone."));
                    }
                }
                _ => {
                    out = out
                        .child(heading("Trips"))
                        .child(caption(current.status))
                        .child(action(
                            if past.get() {
                                "Upcoming trips"
                            } else {
                                "Include past trips"
                            },
                            move || past.update(|v| *v = !*v),
                        ));
                    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
                    for plan in current
                        .plans
                        .iter()
                        .filter(|p| past.get() || travel::upcoming(p, &today))
                    {
                        let id = plan.id.clone();
                        out = out
                            .child(action(plan.title.clone(), move || {
                                open(TravelRoute::Trip {
                                    id: id.clone(),
                                    section: TripSection::Overview,
                                })
                            }))
                            .child(caption(format!(
                                "{}–{} · {}",
                                plan.date_start, plan.date_end, plan.status
                            )));
                    }
                    if current.plans.is_empty() && !current.loading {
                        out = out.child(caption(
                            "No saved trips. Refresh while connected to the Mac.",
                        ));
                    }
                }
            }
            out.into_element()
        })))
}

/// Stable route containers keep station inputs mounted while search answers arrive.
#[component]
pub fn TravelScreen() -> impl IntoElement {
    let snap = snapshot(&feed().tx);
    let connections = computed(move || snap.with(|s| s.route == TravelRoute::Connections));
    view()
        .flex_1()
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .child(action("Trips", || open(TravelRoute::Trips)))
                .child(action("Map & Places", || open(TravelRoute::Map)))
                .child(action("Connections", || open(TravelRoute::Connections)))
                .child(action("Refresh", load)),
        )
        .child(view().flex_1().child(dynamic(move || {
            if connections.get() {
                scroll()
                    .flex_1()
                    .px_4()
                    .child(connections_screen())
                    .into_element()
            } else {
                travel_content().into_element()
            }
        })))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn connection_suggestions_do_not_replace_input_or_accept_free_text() {
        use nsh_engine::EngineConfig;
        use nsh_testing::Headless;
        let mut app = Headless::start(EngineConfig {
            ui_sets: vec![nsh_base::registry()],
            ..EngineConfig::new(nsh_core::element::Root::of::<ConnectionsScreen>())
        })
        .unwrap();
        app.wait_for("text:Connections").unwrap();
        app.type_text("id:station-0", "Test").unwrap();
        {
            let mut s = state();
            s.snap.suggestions[0] = vec![Station {
                id: "station-a".into(),
                name: "Test Central".into(),
                ..Default::default()
            }];
            publish(&s);
        }
        app.wait_for("text:Test Central").unwrap();
        assert_eq!(app.screen().input_text("id:station-0"), Ok(Some("Test")));
        app.press("text:Search").unwrap();
        app.wait_for("text:Pick both stations from the suggestions.")
            .unwrap();
        app.press("text:Test Central").unwrap();
        assert_eq!(
            app.screen().input_text("id:station-0"),
            Ok(Some("Test Central"))
        );
        app.type_text("id:station-0", " changed").unwrap();
        app.press("text:Search").unwrap();
        assert_eq!(
            state().snap.search_error.as_deref(),
            Some("Pick both stations from the suggestions.")
        );
    }
}
