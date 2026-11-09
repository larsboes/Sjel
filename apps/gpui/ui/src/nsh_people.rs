//! People and Places, drawn by NSH from the Mac's read-only copy (`docs/plans/gpui.md` §11).
//!
//! One register with two sides: who somebody is, and where the two of you have been. The switch
//! chooses the side and clears the shared search field. The screen is read-only by intent: the
//! registry's writes are the operator's (`/api/people/decide`), so nothing here offers a control
//! that would have to be refused.

use std::sync::{Arc, Mutex, OnceLock};

use nsh_base::{button, input};
use nsh_core::prelude::*;
use sjel_gpui_core::mirror::{Mirror, PeopleCopy};
use sjel_gpui_core::people::{self, Person};
use sjel_gpui_core::places::{self, PersonLink, Place};
use sjel_gpui_core::shell;
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};

/// Which side of the register is on screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Register {
    #[default]
    People,
    Places,
}

/// What the register read from the copy, and what the host said about it.
#[derive(Clone, Debug, Default, PartialEq)]
struct Snapshot {
    source: String,
    people: Vec<Person>,
    places: Vec<Place>,
    links: Vec<PersonLink>,
    error: Option<String>,
    loaded: bool,
    loading: bool,
    has_copy: bool,
    /// A card a launch flag asked for (`SJEL_PERSON`), opened once the rows hold it.
    pending_open: Option<String>,
    register: Register,
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
    snapshot: Mutex<Snapshot>,
    tx: Mutex<Option<UnboundedSender<Snapshot>>>,
    refresh: Mutex<Option<Box<dyn Fn() + Send>>>,
}

fn feed() -> &'static Feed {
    static FEED: OnceLock<Feed> = OnceLock::new();
    FEED.get_or_init(|| Feed {
        state: Mutex::new(State::default()),
        snapshot: Mutex::new(Snapshot::default()),
        tx: Mutex::new(None),
        refresh: Mutex::new(None),
    })
}

fn update(change: impl FnOnce(&mut Snapshot)) {
    let snap = {
        let mut snap = feed().snapshot.lock().unwrap_or_else(|e| e.into_inner());
        change(&mut snap);
        snap.clone()
    };
    if let Some(tx) = feed().tx.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        let _ = tx.send(snap);
    }
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
fn resolve_pending(snap: &Snapshot) -> Option<(Register, String)> {
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
enum Line {
    Header(String),
    Person(Person),
    Place(Place),
}

fn lines(snap: &Snapshot, register: Register, query: &str) -> Vec<Line> {
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

fn empty_line(snap: &Snapshot, register: Register, searching: bool) -> Option<&'static str> {
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

fn second_line(person: &Person) -> String {
    [person.role.as_deref(), person.company.as_deref()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ")
}

fn person_fields(person: &Person) -> Vec<(String, String)> {
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
    for (key, value) in &person.custom_fields {
        fields.push((key.to_uppercase(), value.clone()));
    }
    fields
}

fn place_fields(place: &Place) -> Vec<(String, String)> {
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

fn muted() -> AdaptiveColor {
    nsh_base::theme().muted_foreground
}

fn label(text_: impl Into<String>) -> impl IntoElement {
    text(text_.into())
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(move || muted())
}

fn field_list(fields: Vec<(String, String)>) -> impl IntoElement {
    let mut list = view().gap_3();
    for (key, value) in fields {
        list = list.child(
            view()
                .gap_1()
                .child(label(key))
                .child(text(value).text_base()),
        );
    }
    list
}

/// Which card is open, by id. A card replaces the list, so a thumb on a row never scrolls past a
/// panel that opened below it.
#[derive(Clone, Copy)]
struct Open {
    person: Signal<Option<String>>,
    place: Signal<Option<String>>,
}

fn back(label_: &'static str, open: Open) -> impl IntoElement {
    button()
        .py_2()
        .on_press(move || {
            open.person.set(None);
            open.place.set(None);
        })
        .child(text(label_).text_base())
}

fn person_card(person: Person, snap: &Snapshot, open: Open) -> impl IntoElement {
    let linked: Vec<String> = places::places_of(&snap.places, &snap.links, &person.name)
        .into_iter()
        .map(|place| {
            let where_line = place.where_line();
            if where_line.is_empty() {
                place.name.clone()
            } else {
                format!("{} · {where_line}", place.name)
            }
        })
        .collect();
    let mut places_block = view().gap_1().child(label(if linked.is_empty() {
        "NO PLACES LINKED"
    } else {
        "PLACES"
    }));
    if linked.is_empty() {
        places_block = places_block.child(
            text("The register has no place for this name yet.")
                .text_sm()
                .text_color(move || muted()),
        );
    }
    for line in linked {
        places_block = places_block.child(text(line).text_sm());
    }
    let role = second_line(&person);
    scroll().flex_1().child(
        view()
            .px_4()
            .pb_6()
            .gap_4()
            .child(back("‹ All people", open))
            .child(
                view()
                    .gap_1()
                    .child(
                        text(person.name.clone())
                            .text_2xl()
                            .font_weight(FontWeight::SEMIBOLD),
                    )
                    .child(
                        (!role.is_empty())
                            .then(|| text(role).text_sm().text_color(move || muted())),
                    ),
            )
            .child(field_list(person_fields(&person)))
            .child(places_block),
    )
}

fn place_card(place: Place, snap: &Snapshot, open: Open) -> impl IntoElement {
    let names = places::people_at(&snap.links, &place.id);
    let mut people_block = view().gap_1().child(label(if names.is_empty() {
        "NOBODY LINKED"
    } else {
        "PEOPLE"
    }));
    if names.is_empty() {
        people_block = people_block.child(
            text("The register links no person to this place yet.")
                .text_sm()
                .text_color(move || muted()),
        );
    }
    for (name, confirmed) in names {
        people_block = people_block.child(
            text(format!(
                "{name} · {}",
                if confirmed { "confirmed" } else { "proposed" }
            ))
            .text_sm(),
        );
    }
    scroll().flex_1().child(
        view()
            .px_4()
            .pb_6()
            .gap_4()
            .child(back("‹ All places", open))
            .child(
                view()
                    .gap_1()
                    .child(
                        text(place.name.clone())
                            .text_2xl()
                            .font_weight(FontWeight::SEMIBOLD),
                    )
                    .child(
                        text(place.kind_label())
                            .text_sm()
                            .text_color(move || muted()),
                    ),
            )
            .child(field_list(place_fields(&place)))
            .child(people_block),
    )
}

fn line_view(line: Line, open: Open) -> AnyElement {
    match line {
        Line::Header(kind) => view()
            .px_4()
            .pt_4()
            .pb_1()
            .child(label(kind))
            .into_element(),
        Line::Person(person) => {
            let id = person.id.clone();
            let second = second_line(&person);
            button()
                .flex_row()
                .items_center()
                .gap_3()
                .px_4()
                .py_3()
                .on_press(move || open.person.set(Some(id.clone())))
                .child(
                    view()
                        .w(px(36.0))
                        .h(px(36.0))
                        .rounded_full()
                        .items_center()
                        .justify_center()
                        .bg(move || nsh_base::theme().muted)
                        .child(text(person.initials.clone()).text_xs()),
                )
                .child(
                    view()
                        .flex_1()
                        .gap_1()
                        .child(text(person.name.clone()).text_base())
                        .child(
                            (!second.is_empty())
                                .then(|| text(second).text_xs().text_color(move || muted())),
                        ),
                )
                .into_element()
        }
        Line::Place(place) => {
            let id = place.id.clone();
            let where_line = place.where_line();
            button()
                .px_4()
                .py_3()
                .gap_1()
                .on_press(move || open.place.set(Some(id.clone())))
                .child(text(place.name.clone()).text_base())
                .child(
                    (!where_line.is_empty())
                        .then(|| text(where_line).text_xs().text_color(move || muted())),
                )
                .into_element()
        }
    }
}

#[component]
pub fn PeopleScreen() -> impl IntoElement {
    let initial = feed()
        .snapshot
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let register = signal(initial.register);
    let snap = signal(initial);
    let query = signal(String::new());
    let open = Open {
        person: signal(None::<String>),
        place: signal(None::<String>),
    };
    let open_pending = move |current: &Snapshot| {
        if let Some((side, id)) = resolve_pending(current) {
            register.set(side);
            match side {
                Register::People => open.person.set(Some(id)),
                Register::Places => open.place.set(Some(id)),
            }
            feed()
                .snapshot
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .pending_open = None;
        }
    };
    open_pending(&snap.get());
    let (tx, mut rx) = unbounded_channel::<Snapshot>();
    *feed().tx.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx);
    spawn(async move {
        while let Some(next) = rx.recv().await {
            if next.register != snap.with(|s| s.register) {
                register.set(next.register);
            }
            open_pending(&next);
            // A newer read keeps the open card only while the register still holds it.
            if let Some(id) = open.person.get() {
                if !next.people.iter().any(|p| p.id == id) && next.loaded {
                    open.person.set(None);
                }
            }
            if let Some(id) = open.place.get() {
                if !next.places.iter().any(|p| p.id == id) && next.loaded {
                    open.place.set(None);
                }
            }
            snap.set(next);
        }
    });
    let visible = computed(move || {
        let side = register.get();
        let typed = query.get();
        snap.with(|s| lines(s, side, &typed))
    });
    let switch = move |side: Register, name: &'static str| {
        button()
            .flex_1()
            .py_2()
            .items_center()
            .rounded_lg()
            .bg(move || {
                if register.get() == side {
                    nsh_base::theme().primary
                } else {
                    nsh_base::theme().background
                }
            })
            .on_press(move || {
                register.set(side);
                query.set(String::new());
            })
            .child(
                text(move || {
                    let count = snap.with(|s| match side {
                        Register::People => s.people.len(),
                        Register::Places => s.places.len(),
                    });
                    format!("{name} {count}")
                })
                .text_sm()
                .text_color(move || {
                    if register.get() == side {
                        nsh_base::theme().primary_foreground
                    } else {
                        nsh_base::theme().foreground
                    }
                }),
            )
    };
    let list = move || {
        view()
            .flex_1()
            .child(
                view()
                    .px_4()
                    .pt_4()
                    .pb_2()
                    .gap_2()
                    .child(
                        view()
                            .flex_row()
                            .justify_between()
                            .items_center()
                            .child(
                                text("People & Places")
                                    .text_2xl()
                                    .font_weight(FontWeight::SEMIBOLD),
                            )
                            .child(
                                button()
                                    .px_2()
                                    .py_1()
                                    .on_press(|| {
                                        if let Some(refresh) = feed()
                                            .refresh
                                            .lock()
                                            .unwrap_or_else(|e| e.into_inner())
                                            .as_ref()
                                        {
                                            refresh();
                                        }
                                        load();
                                    })
                                    .child(text("Refresh").text_sm()),
                            ),
                    )
                    .child(
                        text(move || snap.with(|s| s.source.clone()))
                            .text_xs()
                            .text_color(move || muted()),
                    )
                    .child(
                        view()
                            .flex_row()
                            .gap_2()
                            .child(switch(Register::People, "People"))
                            .child(switch(Register::Places, "Places")),
                    )
                    .child(
                        input()
                            .value(move || query.get())
                            .on_change(move |value: &str| query.set(value.to_owned()))
                            .placeholder("Search")
                            .px_3()
                            .py_2()
                            .border(px(1.0))
                            .border_color(move || nsh_base::theme().input)
                            .rounded_lg(),
                    )
                    .child(dynamic(move || {
                        snap.with(|s| s.error.clone()).map(|error| {
                            text(error)
                                .text_xs()
                                .text_color(move || nsh_base::theme().destructive)
                                .into_element()
                        })
                    }))
                    .child(dynamic(move || {
                        let empty = visible.with(Vec::is_empty);
                        let searching = !query.with(|q| q.trim().is_empty());
                        let side = register.get();
                        empty
                            .then(|| snap.with(|s| empty_line(s, side, searching)))
                            .flatten()
                            .map(|line| {
                                text(line)
                                    .text_sm()
                                    .text_color(move || muted())
                                    .into_element()
                            })
                    })),
            )
            .child(
                virtual_list(
                    move || visible.with(Vec::len),
                    move |at| {
                        let line = visible
                            .with(|all| all.get(at.index()).cloned())
                            .unwrap_or(Line::Header(String::new()));
                        dynamic(move || line_view(line.clone(), open))
                    },
                )
                .estimated_item_size(px(60.0))
                .flex_1(),
            )
    };
    view().flex_1().child(dynamic(move || {
        if let Some(id) = open.person.get() {
            if let Some(person) = snap.with(|s| s.people.iter().find(|p| p.id == id).cloned()) {
                return snap.with(|s| person_card(person, s, open).into_element());
            }
        }
        if let Some(id) = open.place.get() {
            if let Some(place) = snap.with(|s| s.places.iter().find(|p| p.id == id).cloned()) {
                return snap.with(|s| place_card(place, s, open).into_element());
            }
        }
        list().into_element()
    }))
}

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

    #[test]
    fn the_screen_lists_people_and_opens_a_card() {
        use nsh_core::element::Root;
        use nsh_engine::EngineConfig;
        use nsh_testing::Headless;

        update(|snap| {
            *snap = Snapshot {
                source: "Copy from 2026-11-09 · read-only".to_owned(),
                people: vec![person("p1", "Ada Lovelace")],
                places: vec![place("a", "Bar Raval", "bar")],
                loaded: true,
                has_copy: true,
                ..Default::default()
            };
        });
        let mut app = Headless::start(EngineConfig {
            ui_sets: vec![nsh_base::registry()],
            ..EngineConfig::new(Root::of::<PeopleScreen>())
        })
        .expect("the People screen starts");
        app.wait_for("text:Ada Lovelace").unwrap();
        app.press("text:Ada Lovelace").unwrap();
        app.wait_for("text:‹ All people").unwrap();
        app.wait_for("text:NO PLACES LINKED").unwrap();
        app.press("text:‹ All people").unwrap();
        app.press("text:Places 1").unwrap();
        app.wait_for("text:Bar Raval").unwrap();
    }
}
