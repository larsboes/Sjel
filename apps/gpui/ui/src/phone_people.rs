//! People and Places from the Mac's read-only copy.
//!
//! One register with two sides: who somebody is, and where the two of you have been. The switch
//! chooses the side and clears the shared search field. The screen is read-only by intent: the
//! registry's writes are the operator's (`/api/people/decide`), so nothing here offers a control
//! that would have to be refused.

use std::sync::{Arc, Mutex, OnceLock};

use sjel_gpui_core::mirror::{Mirror, PeopleCopy};
use sjel_gpui_core::people::{self, Person};
use sjel_gpui_core::places::{self, PersonLink, Place};
use sjel_gpui_core::shell;
use tokio::sync::watch;

/// Which side of the register is on screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Register {
    #[default]
    People,
    Places,
}

/// What the register read from the copy, and what the host said about it.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Snapshot {
    pub(crate) source: String,
    pub(crate) people: Vec<Person>,
    pub(crate) places: Vec<Place>,
    pub(crate) links: Vec<PersonLink>,
    pub(crate) error: Option<String>,
    pub(crate) loaded: bool,
    pub(crate) loading: bool,
    pub(crate) has_copy: bool,
    /// A card a launch flag asked for (`SJEL_PERSON`), opened once the rows hold it.
    pub(crate) pending_open: Option<String>,
    pub(crate) register: Register,
}

#[derive(Default)]
struct State {
    copy: Option<Arc<Mirror>>,
    missing: Option<String>,
    note: Option<String>,
    generation: u64,
}

struct Feed {
    state: Mutex<State>,
    snapshot: watch::Sender<Snapshot>,
    refresh: Mutex<Option<Box<dyn Fn() + Send>>>,
}

fn feed() -> &'static Feed {
    static FEED: OnceLock<Feed> = OnceLock::new();
    FEED.get_or_init(|| Feed {
        state: Mutex::new(State::default()),
        snapshot: watch::channel(Snapshot::default()).0,
        refresh: Mutex::new(None),
    })
}

fn update(change: impl FnOnce(&mut Snapshot)) {
    feed().snapshot.send_modify(change);
}

pub(crate) fn subscribe() -> watch::Receiver<Snapshot> {
    feed().snapshot.subscribe()
}

pub(crate) fn clear_pending(name: Option<&str>) {
    feed().snapshot.send_if_modified(|snap| {
        if snap.pending_open.as_deref() != name || name.is_none() {
            return false;
        }
        snap.pending_open = None;
        true
    });
}

pub(crate) fn refresh() {
    if let Some(refresh) = feed()
        .refresh
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
    {
        refresh();
    }
    load();
}

/// The register's own line: which copy this is, and how old.
fn source_line(state: &State) -> String {
    let Some(copy) = state.copy.as_ref() else {
        return state
            .missing
            .clone()
            .unwrap_or_else(|| "No copy on this phone yet.".to_owned());
    };
    let line = format!("Copy from {} · read-only", copy.manifest().built_label());
    match state.note.as_ref() {
        Some(note) => format!("{line} · {note}"),
        None => line,
    }
}

/// What "Refresh" asks the host for: a newer copy, or a sync that carries one.
pub fn set_refresh(refresh: impl Fn() + Send + 'static) {
    *feed().refresh.lock().unwrap_or_else(|e| e.into_inner()) = Some(Box::new(refresh));
}

/// Chooses the register a launch opens on (`SJEL_REGISTER`).
pub fn set_register(register: Register) {
    update(|snap| snap.register = register);
}

/// Opens the card of the first person, else the first place, whose name contains `text`
/// (`SJEL_PERSON`). A name that matches nothing yet is kept for the next read.
pub fn open_named(text: &str) {
    let text = text.to_owned();
    update(|snap| snap.pending_open = Some(text));
}

/// Hands the screen the Mac's copy, or the reason it is not there, and reads both registers.
/// A newer copy moves the rows, not the person: the open card and the side stay.
pub fn set_copy(copy: Option<Arc<Mirror>>, missing: Option<String>) {
    let source = {
        let mut state = feed().state.lock().unwrap_or_else(|e| e.into_inner());
        state.copy = copy;
        state.missing = missing;
        source_line(&state)
    };
    let has_copy = feed()
        .state
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .copy
        .is_some();
    update(|snap| {
        snap.source = source;
        snap.has_copy = has_copy;
        snap.error = None;
    });
    load();
}

/// A sentence from the host, such as why a newer copy could not be fetched. The rows stay.
pub fn set_note(note: Option<String>) {
    let source = {
        let mut state = feed().state.lock().unwrap_or_else(|e| e.into_inner());
        state.note = note;
        source_line(&state)
    };
    update(|snap| snap.source = source);
}

/// Reads both registers from the copy. A newer read makes an older answer stale.
pub fn load() {
    let (copy, generation) = {
        let mut state = feed().state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(copy) = state.copy.clone() else {
            return;
        };
        state.generation += 1;
        (copy, state.generation)
    };
    update(|snap| snap.loading = true);
    shell::spawn(async move {
        let answer = copy.people().await;
        if feed()
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .generation
            != generation
        {
            return;
        }
        update(|snap| {
            snap.loading = false;
            snap.loaded = true;
            match answer {
                Ok(copy) => apply(snap, copy),
                Err(error) => snap.error = Some(error),
            }
        });
    });
}

fn apply(snap: &mut Snapshot, copy: PeopleCopy) {
    snap.people = copy.people.iter().map(Person::from_record).collect();
    snap.places = copy.places;
    snap.links = copy.links;
    snap.error = None;
}

/// The card a pending name opens: a person first, else a place.
pub(crate) fn resolve_pending(snap: &Snapshot) -> Option<(Register, String)> {
    let needle = snap.pending_open.as_ref()?.to_lowercase();
    if let Some(person) = snap
        .people
        .iter()
        .find(|p| p.name.to_lowercase().contains(&needle))
    {
        return Some((Register::People, person.id.clone()));
    }
    snap.places
        .iter()
        .find(|p| p.name.to_lowercase().contains(&needle))
        .map(|place| (Register::Places, place.id.clone()))
}

/// One line of the list: a kind header in Places, or a row.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Line {
    Header(String),
    Person(Person),
    Place(Place),
}

pub(crate) fn lines(snap: &Snapshot, register: Register, query: &str) -> Vec<Line> {
    match register {
        Register::People => people::filter_people(&snap.people, query)
            .into_iter()
            .cloned()
            .map(Line::Person)
            .collect(),
        Register::Places => {
            // Grouped by kind: the register sorts by it, and hundreds of places in one column
            // read as one long list of nothing in particular.
            let mut out = Vec::new();
            let mut last_kind = String::new();
            for place in places::filter_places(&snap.places, query) {
                if place.kind != last_kind {
                    out.push(Line::Header(place.kind_label().to_uppercase()));
                    last_kind = place.kind.clone();
                }
                out.push(Line::Place(place.clone()));
            }
            out
        }
    }
}

pub(crate) fn empty_line(
    snap: &Snapshot,
    register: Register,
    searching: bool,
) -> Option<&'static str> {
    if !snap.has_copy {
        return None;
    }
    if snap.loading && !snap.loaded {
        return Some("Reading the register…");
    }
    if !snap.loaded {
        return Some("Not read yet.");
    }
    Some(match (register, searching) {
        (Register::People, true) => "Nobody matches that.",
        (Register::Places, true) => "No place matches that.",
        (Register::People, false) => "The copy holds no contacts.",
        (Register::Places, false) => "The copy holds no places.",
    })
}

pub(crate) fn second_line(person: &Person) -> String {
    [person.role.as_deref(), person.company.as_deref()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ")
}

pub(crate) fn person_fields(person: &Person) -> Vec<(String, String)> {
    let mut fields = Vec::new();
    for (label, value) in [
        ("EMAIL", &person.email),
        ("PHONE", &person.phone),
        ("LOCATION", &person.location),
        ("NOTE", &person.note_ref),
    ] {
        if let Some(value) = value {
            fields.push((label.to_owned(), value.clone()));
        }
    }
    for (label, value) in person.network_rows() {
        fields.push((label.to_uppercase(), value));
    }
    for (key, value) in &person.custom_fields {
        fields.push((key.to_uppercase(), value.clone()));
    }
    fields
}

pub(crate) fn place_fields(place: &Place) -> Vec<(String, String)> {
    let mut fields = Vec::new();
    let where_line = place.where_line();
    if !where_line.is_empty() {
        fields.push(("WHERE".to_owned(), where_line));
    }
    if let Some(address) = &place.address {
        fields.push(("ADDRESS".to_owned(), address.clone()));
    }
    if let (Some(lat), Some(lon)) = (place.latitude, place.longitude) {
        fields.push(("COORDINATES".to_owned(), format!("{lat:.4}, {lon:.4}")));
    }
    fields
}

mod view;
pub use view::PhonePeople;

#[cfg(test)]
mod tests {
    use super::*;

    fn place(id: &str, name: &str, kind: &str) -> Place {
        Place {
            id: id.to_owned(),
            name: name.to_owned(),
            kind: kind.to_owned(),
            address: None,
            city: Some("Berlin".to_owned()),
            country_code: Some("DE".to_owned()),
            latitude: None,
            longitude: None,
            source: "manual".to_owned(),
            external_ref: None,
            ..Place::default()
        }
    }

    fn person(id: &str, name: &str) -> Person {
        Person {
            id: id.to_owned(),
            name: name.to_owned(),
            initials: name.chars().take(1).collect(),
            note_ref: None,
            revision: 1,
            email: None,
            phone: None,
            company: Some("Acme".to_owned()),
            role: Some("Engineer".to_owned()),
            location: None,
            photo: None,
            tags: Vec::new(),
            custom_fields: Vec::new(),
            values: Default::default(),
            sources: Default::default(),
            facts: Vec::new(),
        }
    }

    #[test]
    fn places_group_under_their_kind_and_a_name_opens_a_card() {
        let snap = Snapshot {
            people: vec![person("p1", "Ada Lovelace")],
            places: vec![
                place("a", "Bar Raval", "bar"),
                place("b", "Café Einstein", "cafe"),
                place("c", "Kaffee Kirsch", "cafe"),
            ],
            pending_open: Some("kirsch".to_owned()),
            ..Default::default()
        };
        let all = lines(&snap, Register::Places, "");
        assert_eq!(all.len(), 5, "two headers and three rows");
        assert!(matches!(&all[0], Line::Header(_)));
        assert!(matches!(&all[2], Line::Header(_)));
        assert_eq!(
            resolve_pending(&snap),
            Some((Register::Places, "c".to_owned()))
        );
        assert_eq!(second_line(&snap.people[0]), "Engineer · Acme");
    }
}
