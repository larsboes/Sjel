//! People and Places, drawn from the Mac's read-only copy.
//!
//! One register with two sides — who somebody is, and where the two of you have been — because the
//! question is one question and the phone has no room for two tabs about it. The switch chooses the
//! side; the search field is shared, so a name typed for a person is cleared by the switch instead of
//! filtering the places to nothing.
//!
//! The page is read-only by intent: `places_person_places` holds the operator's review, and the
//! registry's writes are the operator's (`/api/people/decide`). Nothing here offers a control that
//! would have to be refused, and the line under the title says which copy drew what is on screen.
//!
//! The page knows nothing about the network. Its host hands it the copy and emits
//! [`PeopleEvent::Refresh`] when the person asks for newer data.

use std::sync::Arc;

use gpui::{
    div, prelude::*, px, rgb, rgba, AnyElement, Context, Entity, EventEmitter, MouseButton, Render,
    ScrollHandle, SharedString, Subscription, Window,
};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::Sizable;
use sjel_gpui_core::mirror::{Mirror, PeopleCopy};
use sjel_gpui_core::people::{self, Person};
use sjel_gpui_core::places::{self, PersonLink, Place};

use crate::{Density, Palette};

/// What the page asks its host to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PeopleEvent {
    /// The person asked for newer data: a newer copy, or a sync that carries one.
    Refresh,
    /// The search field took or lost the keyboard, so the host can show or hide it.
    Editing(bool),
}

/// Which side of the register is on screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Register {
    People,
    Places,
}

/// The two registers, one page.
pub struct People {
    /// The Mac's read-only copy, when one is on disk.
    copy: Option<Arc<Mirror>>,
    /// Why the copy could not be opened, in the words the person needs.
    missing: Option<SharedString>,
    /// A host's word about this copy, e.g. why a newer one could not be fetched.
    note: Option<SharedString>,
    /// A failed read, kept beside the rows that answered before it.
    error: Option<SharedString>,
    register: Register,
    /// The typed filter, one field for both registers.
    query: String,
    search: Option<(Entity<InputState>, Subscription)>,
    contacts: Vec<Person>,
    places: Vec<Place>,
    links: Vec<PersonLink>,
    /// The open card, by person id or place id. A card replaces the list, so a thumb on a row
    /// never has to scroll past a panel that opened below it.
    open_person: Option<SharedString>,
    open_place: Option<SharedString>,
    /// A name a launch flag asked to open before the copy arrived (`SJEL_PERSON`).
    pending_open: Option<SharedString>,
    loaded: bool,
    loading: bool,
    palette: Palette,
    density: Density,
    scroll: ScrollHandle,
    /// Bumped per read, so a slow answer for a copy the person has left cannot replace the rows on
    /// screen.
    generation: u64,
}

impl EventEmitter<PeopleEvent> for People {}

impl People {
    pub fn new(density: Density) -> Self {
        Self {
            copy: None,
            missing: None,
            note: None,
            error: None,
            register: Register::People,
            query: String::new(),
            search: None,
            contacts: Vec::new(),
            places: Vec::new(),
            links: Vec::new(),
            open_person: None,
            open_place: None,
            pending_open: None,
            loaded: false,
            loading: false,
            palette: Palette::dark(),
            density,
            scroll: ScrollHandle::new(),
            generation: 0,
        }
    }

    /// Hands the page the Mac's copy, or the reason it is not there.
    ///
    /// A newer copy replaces the old one without the page forgetting which register or which card is
    /// open: a refresh that lands while somebody is reading moves the rows, not the person.
    pub fn set_copy(
        &mut self,
        copy: Option<Arc<Mirror>>,
        missing: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.copy = copy;
        self.missing = missing.map(SharedString::from);
        self.loaded = false;
        self.error = None;
        self.load(cx);
    }

    /// A sentence about the data on screen from the host. A failed fetch does not take away the rows
    /// already there: they are still true, only older, and the line above says how old.
    pub fn set_note(&mut self, note: Option<String>, cx: &mut Context<Self>) {
        self.note = note.map(SharedString::from);
        cx.notify();
    }

    pub fn set_palette(&mut self, palette: Palette, cx: &mut Context<Self>) {
        if self.palette != palette {
            self.palette = palette;
            cx.notify();
        }
    }

    /// True once a copy has answered, so a host can tell a blank page from a loading one.
    pub fn loaded(&self) -> bool {
        self.loaded
    }

    /// Reads both registers from the copy. Nothing happens without one: a phone that has never
    /// reached its Mac says so through [`Self::set_copy`]'s `missing` rather than drawing an empty
    /// register that looks like an answer.
    pub fn load(&mut self, cx: &mut Context<Self>) {
        let Some(copy) = self.copy.clone() else {
            return;
        };
        self.generation += 1;
        let generation = self.generation;
        self.loading = true;
        let work = sjel_gpui_core::shell::run(async move { copy.people().await });
        cx.spawn(async move |this, cx| {
            // The runtime said nothing (a dropped sender), so the copy answered nothing either:
            // that is a read that did not happen, which the page can say.
            let answer = work
                .await
                .unwrap_or_else(|| Err("The copy was not read.".to_owned()));
            this.update(cx, |view, cx| {
                if view.generation != generation {
                    return;
                }
                view.loading = false;
                view.loaded = true;
                match answer {
                    Ok(copy) => view.apply(copy),
                    Err(error) => {
                        // The rows that answered before stay: a failed refresh is not a reason to
                        // blank a page that is still readable.
                        view.error = Some(SharedString::from(error));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Puts one read on screen, keeping the open card if the register still holds it.
    ///
    /// The caller notifies: this is also the door the tests come in by, and a test has no
    /// `Context` to notify.
    fn apply(&mut self, copy: PeopleCopy) {
        self.contacts = copy
            .people
            .iter()
            .map(Person::from_record)
            .collect::<Vec<_>>();
        self.places = copy.places;
        self.links = copy.links;
        self.error = None;
        self.loaded = true;
        if let Some(open) = self.open_person.clone() {
            if !self
                .contacts
                .iter()
                .any(|person| person.id == open.as_ref())
            {
                self.open_person = None;
            }
        }
        if let Some(open) = self.open_place.clone() {
            if !self.places.iter().any(|place| place.id == open.as_ref()) {
                self.open_place = None;
            }
        }
        // A flag's requested card is retried against the rows that just arrived.
        self.try_pending_open();
    }

    /// The people the query leaves, in the copy's order.
    pub fn visible_people(&self) -> Vec<&Person> {
        people::filter_people(&self.contacts, &self.query)
    }

    /// The places the query leaves, in the copy's order.
    pub fn visible_places(&self) -> Vec<&Place> {
        places::filter_places(&self.places, &self.query)
    }

    /// The card that is open, if one is.
    pub fn open_person(&self) -> Option<&Person> {
        let id = self.open_person.as_ref()?;
        self.contacts.iter().find(|person| person.id == id.as_ref())
    }

    pub fn open_place(&self) -> Option<&Place> {
        let id = self.open_place.as_ref()?;
        self.places.iter().find(|place| place.id == id.as_ref())
    }

    /// The places the register links to one person, confirmed first.
    pub fn places_of(&self, person: &Person) -> Vec<&Place> {
        places::places_of(&self.places, &self.links, &person.name)
    }

    /// The names the register links to one place, confirmed first.
    pub fn people_at(&self, place: &Place) -> Vec<(String, bool)> {
        places::people_at(&self.links, &place.id)
    }

    fn open(&mut self, person: Option<String>, place: Option<String>, cx: &mut Context<Self>) {
        self.open_person = person.map(SharedString::from);
        self.open_place = place.map(SharedString::from);
        cx.notify();
    }

    /// Chooses the register on screen, as the switch does. Used by the launch flags that let a
    /// screenshot reach a side a hand did not tap (`SJEL_REGISTER`, `ios/src/lib.rs`).
    pub fn set_register(&mut self, register: Register, cx: &mut Context<Self>) {
        if self.register != register {
            self.register = register;
            self.query.clear();
            cx.notify();
        }
    }

    /// Opens the card of the first person whose name contains `text`, case-insensitively, and of the
    /// first place when no person matches. `SJEL_PERSON` is its one caller, for a screenshot; a real
    /// finger uses a row. A name that matches nothing yet is kept, because the copy that holds it may
    /// still be arriving (`Self::apply` retries it once per read).
    pub fn open_named(&mut self, text: &str, cx: &mut Context<Self>) {
        self.pending_open = Some(SharedString::from(text));
        self.try_pending_open();
        cx.notify();
    }

    /// Retries the name a launch flag asked for, and forgets it once a card is open.
    fn try_pending_open(&mut self) {
        let Some(text) = self.pending_open.clone() else {
            return;
        };
        let needle = text.to_lowercase();
        let person = self
            .contacts
            .iter()
            .find(|person| person.name.to_lowercase().contains(&needle))
            .map(|person| person.id.clone());
        let place = self
            .places
            .iter()
            .find(|place| place.name.to_lowercase().contains(&needle))
            .map(|place| place.id.clone());
        match (person, place) {
            (Some(person), _) => {
                self.register = Register::People;
                self.open_person = Some(person.into());
                self.open_place = None;
                self.pending_open = None;
            }
            (None, Some(place)) => {
                self.register = Register::Places;
                self.open_place = Some(place.into());
                self.open_person = None;
                self.pending_open = None;
            }
            // Nothing matches yet: the next read may hold it.
            (None, None) => {}
        }
    }

    /// The register's own line: which copy this is, and how old.
    fn source_line(&self) -> String {
        let Some(copy) = self.copy.as_ref() else {
            return self
                .missing
                .as_ref()
                .map(|missing| missing.to_string())
                .unwrap_or_else(|| "No copy on this phone yet.".to_owned());
        };
        let line = format!("Copy from {} · read-only", copy.manifest().built_label());
        match self.note.as_ref() {
            Some(note) => format!("{line} · {note}"),
            None => line,
        }
    }

    fn search_field(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<InputState> {
        if let Some((input, _)) = &self.search {
            return input.clone();
        }
        // One placeholder for both registers: the field is created once, so a placeholder that
        // named only the side it was born on would go on lying after the switch.
        let input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search people and places…"));
        let subscription =
            cx.subscribe_in(&input, window, |view, input, event, _, cx| match event {
                InputEvent::Change => {
                    view.query = input.read(cx).value().to_string();
                    cx.notify();
                }
                InputEvent::Focus => cx.emit(PeopleEvent::Editing(true)),
                InputEvent::Blur => cx.emit(PeopleEvent::Editing(false)),
                _ => {}
            });
        self.search = Some((input.clone(), subscription));
        input
    }

    /// The switch between the two sides. It clears the query: a name typed for a person would
    /// otherwise empty the places for no visible reason.
    fn register_switch(&self, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        let people = self.contacts.len();
        let places = self.places.len();
        let sides = [
            (Register::People, "People", people),
            (Register::Places, "Places", places),
        ];
        let mut row = div().flex().items_center().gap_2();
        for (side, label, count) in sides {
            let on = self.register == side;
            row = row.child(
                div()
                    .id(SharedString::from(format!("people-register-{label}")))
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .py(px(8.))
                    .rounded(px(8.))
                    .when(on, |chip| chip.bg(rgb(palette.accent)))
                    .when(!on, |chip| chip.bg(rgba(palette.surface)))
                    .text_sm()
                    .font_weight(if on {
                        gpui::FontWeight::SEMIBOLD
                    } else {
                        gpui::FontWeight::NORMAL
                    })
                    .text_color(rgb(if on { 0xffffff } else { palette.text }))
                    .child(label)
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(if on { 0xffffff } else { palette.muted }))
                            .child(count.to_string()),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |view, _, window, cx| {
                            if view.register != side {
                                view.register = side;
                                view.query.clear();
                                if let Some((input, _)) = view.search.as_ref() {
                                    let input = input.clone();
                                    input.update(cx, |input, cx| input.set_value("", window, cx));
                                }
                            }
                            cx.notify();
                        }),
                    ),
            );
        }
        row.into_any_element()
    }

    fn refresh_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        div()
            .id("people-refresh")
            .flex_none()
            .px_3()
            .py(px(6.))
            .rounded(px(8.))
            .bg(rgba(palette.surface))
            .border_1()
            .border_color(rgba(palette.border))
            .text_xs()
            .text_color(rgb(palette.text))
            .child(if self.loading {
                "Fetching…"
            } else {
                "Refresh"
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, _, cx| cx.emit(PeopleEvent::Refresh)),
            )
            .into_any_element()
    }

    fn initials_badge(&self, initials: &str) -> AnyElement {
        let palette = self.palette;
        div()
            .flex_none()
            .size(px(30.))
            .rounded_full()
            .bg(rgba((palette.accent & 0xffffff) | 0x22000000))
            .flex()
            .items_center()
            .justify_center()
            .text_xs()
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(rgb(palette.accent))
            .child(initials.to_owned())
            .into_any_element()
    }

    fn person_row(&self, person: &Person, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        let second = [person.role.as_deref(), person.company.as_deref()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ");
        let id = person.id.clone();
        let mut row = crate::row_for_density(self.density)
            .id(SharedString::from(format!("person-{id}")))
            .cursor_pointer()
            .child(self.initials_badge(&person.initials))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(palette.text))
                            .child(person.name.clone()),
                    )
                    .when(!second.is_empty(), |column| {
                        column.child(div().text_xs().text_color(rgb(palette.muted)).child(second))
                    }),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |view, _, _, cx| view.open(Some(id.clone()), None, cx)),
            );
        row = row.border_b_1().border_color(rgba(palette.border));
        row.into_any_element()
    }

    fn place_row(&self, place: &Place, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        let id = place.id.clone();
        let where_line = place.where_line();
        let mut row = crate::row_for_density(self.density)
            .id(SharedString::from(format!("place-{id}")))
            .cursor_pointer()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(palette.text))
                            .child(place.name.clone()),
                    )
                    .when(!where_line.is_empty(), |column| {
                        column.child(
                            div()
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child(where_line),
                        )
                    }),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |view, _, _, cx| view.open(None, Some(id.clone()), cx)),
            );
        row = row.border_b_1().border_color(rgba(palette.border));
        row.into_any_element()
    }

    /// The way back to the list, at the top of a card where a thumb looks for it.
    fn back_link(
        &self,
        id: &'static str,
        label: &'static str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.palette;
        div()
            .id(id)
            .text_sm()
            .text_color(rgb(palette.accent))
            .cursor_pointer()
            .child(label)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, _, _, cx| view.open(None, None, cx)),
            )
            .into_any_element()
    }

    fn detail_line(&self, label: &str, value: String) -> AnyElement {
        let palette = self.palette;
        div()
            .flex()
            .items_start()
            .justify_between()
            .gap_3()
            .py(px(8.))
            .border_b_1()
            .border_color(rgba(palette.border))
            .child(
                div()
                    .flex_none()
                    .w(px(92.))
                    .text_xs()
                    .whitespace_nowrap()
                    .text_color(rgb(palette.muted))
                    .child(label.to_owned()),
            )
            .child(
                div()
                    .flex_1()
                    .text_sm()
                    .text_color(rgb(palette.text))
                    .child(value),
            )
            .into_any_element()
    }

    fn person_card(&self, person: &Person, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        let company_role = [person.role.as_deref(), person.company.as_deref()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ");
        let mut fields: Vec<AnyElement> = Vec::new();
        if let Some(email) = person.email.clone() {
            fields.push(self.detail_line("EMAIL", email));
        }
        if let Some(phone) = person.phone.clone() {
            fields.push(self.detail_line("PHONE", phone));
        }
        if let Some(location) = person.location.clone() {
            fields.push(self.detail_line("LOCATION", location));
        }
        if let Some(note) = person.note_ref.clone() {
            fields.push(self.detail_line("NOTE", note));
        }
        for (key, value) in person.custom_fields.clone() {
            fields.push(self.detail_line(&key.to_uppercase(), value));
        }

        let linked = self.places_of(person);
        let places: Vec<AnyElement> = linked
            .iter()
            .map(|place| {
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .py(px(6.))
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(palette.text))
                            .child(place.name.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(place.where_line()),
                    )
                    .into_any_element()
            })
            .collect();

        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(self.back_link("person-back", "‹ All people", cx))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(self.initials_badge(&person.initials))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_lg()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(rgb(palette.text))
                                    .child(person.name.clone()),
                            )
                            .when(!company_role.is_empty(), |column| {
                                column.child(
                                    div()
                                        .text_xs()
                                        .text_color(rgb(palette.muted))
                                        .child(company_role.clone()),
                                )
                            }),
                    ),
            )
            .when(!fields.is_empty(), |card| {
                card.child(div().flex().flex_col().children(fields))
            })
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(palette.muted))
                            .child(if places.is_empty() {
                                "NO PLACES LINKED"
                            } else {
                                "PLACES"
                            }),
                    )
                    .when(places.is_empty(), |block| {
                        block.child(
                            div()
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child("The register has no place for this name yet."),
                        )
                    })
                    .children(places),
            )
            .into_any_element()
    }

    fn place_card(&self, place: &Place, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        let mut fields: Vec<AnyElement> = Vec::new();
        let where_line = place.where_line();
        if !where_line.is_empty() {
            fields.push(self.detail_line("WHERE", where_line));
        }
        if let Some(address) = place.address.clone() {
            fields.push(self.detail_line("ADDRESS", address));
        }
        if let (Some(latitude), Some(longitude)) = (place.latitude, place.longitude) {
            fields.push(self.detail_line("COORDINATES", format!("{latitude:.4}, {longitude:.4}")));
        }

        let people = self.people_at(place);
        let names: Vec<AnyElement> = people
            .iter()
            .map(|(name, confirmed)| {
                let name = name.clone();
                let confirmed = *confirmed;
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .py(px(6.))
                    .child(div().text_sm().text_color(rgb(palette.text)).child(name))
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(if confirmed { "confirmed" } else { "proposed" }),
                    )
                    .into_any_element()
            })
            .collect();

        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(self.back_link("place-back", "‹ All places", cx))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_lg()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(palette.text))
                            .child(place.name.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(place.kind_label()),
                    ),
            )
            .when(!fields.is_empty(), |card| {
                card.child(div().flex().flex_col().children(fields))
            })
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(palette.muted))
                            .child(if names.is_empty() {
                                "NOBODY LINKED"
                            } else {
                                "PEOPLE"
                            }),
                    )
                    .when(names.is_empty(), |block| {
                        block.child(
                            div()
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child("The register links no person to this place yet."),
                        )
                    })
                    .children(names),
            )
            .into_any_element()
    }

    fn empty_line(&self) -> Option<String> {
        if self.missing.is_some() {
            return None;
        }
        if self.loading {
            return Some("Reading the register…".to_owned());
        }
        if !self.loaded {
            return Some("Not read yet.".to_owned());
        }
        let count = match self.register {
            Register::People => self.visible_people().len(),
            Register::Places => self.visible_places().len(),
        };
        if count > 0 {
            return None;
        }
        let searching = !self.query.trim().is_empty();
        match self.register {
            Register::People if searching => Some("Nobody matches that.".to_owned()),
            Register::Places if searching => Some("No place matches that.".to_owned()),
            Register::People => Some("The copy holds no contacts.".to_owned()),
            Register::Places => Some("The copy holds no places.".to_owned()),
        }
    }
}

impl Render for People {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self.palette;
        // The register's name, not the card's: the card carries its own heading, and a title that
        // changed to "Person" said less than the name under it already did.
        let title = match self.register {
            Register::People => "People",
            Register::Places => "Places",
        };
        let source_line = self.source_line();
        let search = self.search_field(window, cx);
        let switch = self.register_switch(cx);
        let refresh = self.refresh_button(cx);
        let error = self.error.clone();
        let body: AnyElement = match (self.open_person(), self.open_place()) {
            (Some(person), _) => self.person_card(person, cx),
            (_, Some(place)) => self.place_card(place, cx),
            _ => {
                let rows: Vec<AnyElement> = match self.register {
                    Register::People => {
                        let people = self.visible_people();
                        people
                            .into_iter()
                            .map(|person| {
                                let person = person.clone();
                                self.person_row(&person, cx)
                            })
                            .collect()
                    }
                    Register::Places => {
                        let places = self.visible_places();
                        // Grouped by kind, because the register sorts by it and 253 rows in one
                        // column read as one long list of nothing in particular. The header names
                        // the group the rows under it belong to.
                        let mut rows: Vec<AnyElement> = Vec::new();
                        let mut last_kind = String::new();
                        for place in places {
                            let place = place.clone();
                            if place.kind != last_kind {
                                rows.push(
                                    div()
                                        .pt_2()
                                        .text_xs()
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .text_color(rgb(palette.muted))
                                        .child(place.kind_label().to_uppercase())
                                        .into_any_element(),
                                );
                                last_kind = place.kind.clone();
                            }
                            rows.push(self.place_row(&place, cx));
                        }
                        rows
                    }
                };
                let empty = rows.is_empty().then(|| self.empty_line()).flatten();
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .children(rows)
                    .when_some(empty, |list, line| {
                        list.child(
                            div()
                                .py_3()
                                .text_sm()
                                .text_color(rgb(palette.muted))
                                .child(line),
                        )
                    })
                    .into_any_element()
            }
        };

        div()
            .flex()
            .flex_col()
            .size_full()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_start()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_lg()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child(title.to_owned()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(palette.muted))
                                    .child(source_line),
                            ),
                    )
                    .child(refresh),
            )
            .when_some(error, |page, error| {
                page.child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(error.to_string()),
                )
            })
            .when(
                self.open_person.is_none() && self.open_place.is_none(),
                |page| {
                    page.child(switch).child(
                        div()
                            .px_3()
                            .py(px(8.))
                            .rounded(px(8.))
                            .bg(rgba(palette.surface))
                            .border_1()
                            .border_color(rgba(palette.border))
                            .child(Input::new(&search).small()),
                    )
                },
            )
            .child(
                div()
                    .id("people-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .vertical_scrollbar(&self.scroll)
                    .child(body),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use sjel_gpui_core::shell::EntityRecord;

    fn record(id: &str, name: &str, values: serde_json::Value) -> EntityRecord {
        let mut map = std::collections::BTreeMap::new();
        if let Some(object) = values.as_object() {
            for (key, value) in object {
                map.insert(key.clone(), value.clone());
            }
        }
        EntityRecord {
            id: id.into(),
            kind: "person".into(),
            name: name.into(),
            note_ref: None,
            revision: 1,
            values: map,
            facts: Vec::new(),
        }
    }

    fn place(id: &str, name: &str, kind: &str, city: &str) -> Place {
        Place {
            id: id.into(),
            name: name.into(),
            kind: kind.into(),
            city: Some(city.into()),
            country_code: Some("DE".into()),
            address: None,
            latitude: None,
            longitude: None,
        }
    }

    fn page() -> People {
        People::new(Density::Mobile)
    }

    fn copy() -> PeopleCopy {
        PeopleCopy {
            people: vec![
                record(
                    "ent:1",
                    "Ada Lovelace",
                    json!({"emails": ["ada@example.com"], "phones": ["+44 20 7946 0958"],
                           "company": "Analytical Engine Inc"}),
                ),
                record("ent:2", "Alan Turing", json!({"company": "Bletchley Park"})),
            ],
            places: vec![
                place("pl:1", "Konzerthaus", "venue", "Berlin"),
                place("pl:2", "Zürich", "city", "Zürich"),
            ],
            links: vec![
                PersonLink {
                    person: "Ada Lovelace".into(),
                    place_id: "pl:1".into(),
                    state: "confirmed".into(),
                },
                PersonLink {
                    person: "Ada Lovelace".into(),
                    place_id: "pl:2".into(),
                    state: "dismissed".into(),
                },
            ],
        }
    }

    #[test]
    fn both_registers_come_from_one_read_and_the_switch_swaps_them() {
        let mut page = page();
        // Nothing has been read, so the page says so instead of drawing an empty register.
        assert_eq!(page.empty_line().as_deref(), Some("Not read yet."));

        page.apply(copy());
        assert_eq!(page.contacts.len(), 2);
        assert_eq!(page.places.len(), 2);
        assert_eq!(page.visible_people().len(), 2);
        assert_eq!(page.visible_places().len(), 2);
        assert!(page.empty_line().is_none());
    }

    #[test]
    fn the_card_of_a_person_that_left_the_register_does_not_stay_open() {
        let mut page = page();
        page.apply(copy());
        page.open_person = Some("ent:1".into());
        assert_eq!(
            page.open_person().map(|person| person.name.as_str()),
            Some("Ada Lovelace")
        );

        // The next copy no longer holds her: the page falls back to the list rather than drawing a
        // card with nothing in it.
        let mut smaller = copy();
        smaller.people.retain(|record| record.id != "ent:1");
        page.apply(smaller);
        assert!(page.open_person().is_none());
    }

    #[test]
    fn a_contact_card_shows_the_fields_the_registry_writes_and_the_places_it_links() {
        let mut page = page();
        page.contacts = copy().people.iter().map(Person::from_record).collect();
        page.places = copy().places;
        page.links = copy().links;

        let ada = page
            .visible_people()
            .into_iter()
            .find(|person| person.name == "Ada Lovelace")
            .cloned()
            .expect("Ada");
        assert_eq!(ada.email.as_deref(), Some("ada@example.com"));
        assert_eq!(ada.phone.as_deref(), Some("+44 20 7946 0958"));
        // Only the link the operator did not dismiss.
        assert_eq!(
            page.places_of(&ada)
                .iter()
                .map(|place| place.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Konzerthaus"]
        );
    }

    #[test]
    fn the_query_filters_both_sides_and_a_place_knows_who_is_linked_to_it() {
        let mut page = page();
        page.contacts = copy().people.iter().map(Person::from_record).collect();
        page.places = copy().places;
        page.links = copy().links;

        page.query = "turing".into();
        assert_eq!(page.visible_people().len(), 1);
        assert_eq!(page.visible_places().len(), 0);

        page.query = String::new();
        let konzerthaus = page
            .visible_places()
            .into_iter()
            .find(|place| place.name == "Konzerthaus")
            .cloned()
            .expect("Konzerthaus");
        assert_eq!(
            page.people_at(&konzerthaus),
            vec![("Ada Lovelace".to_string(), true)]
        );
    }
}
