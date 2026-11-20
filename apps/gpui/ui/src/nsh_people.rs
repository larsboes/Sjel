//! NSH rendering for the phone's read-only people and places.

use nsh_base::{button, input};
use nsh_core::prelude::*;
use sjel_gpui_core::{
    people::Person,
    places::{self, Place},
};

use crate::phone_people::{
    clear_pending, empty_line, lines, person_fields, place_fields, refresh, resolve_pending,
    second_line, subscribe, Line, Register, Snapshot,
};

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
    let mut source = subscribe();
    let initial = source.borrow_and_update().clone();
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
            clear_pending(current.pending_open.as_deref());
        }
    };
    open_pending(&snap.get());
    spawn(async move {
        while source.changed().await.is_ok() {
            let next = source.borrow_and_update().clone();
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
        dynamic(move || {
            let count = snap.with(|s| match side {
                Register::People => s.people.len(),
                Register::Places => s.places.len(),
            });
            button()
                .min_h(px(44.))
                .px_3()
                .py_2()
                .bg(move || {
                    let theme = nsh_base::theme();
                    if register.get() == side {
                        theme.muted
                    } else {
                        theme.background
                    }
                })
                .on_press(move || {
                    register.set(side);
                    query.set(String::new());
                })
                .child(
                    text(format!("{name} {count}"))
                        .text_sm()
                        .number_of_lines(1u32),
                )
                .into_element()
        })
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
                                    .on_press(refresh)
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
