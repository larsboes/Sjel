//! GPUI layout for the phone's read-only People and Places register.

use std::{ops::Range, rc::Rc};

use gpui::{
    div, prelude::*, px, rgb, rgba, uniform_list, Context, Entity, Focusable, FontWeight,
    Subscription, Task, Window,
};
use sjel_gpui_core::places;

use super::{
    clear_pending, empty_line, lines, person_fields, place_fields, refresh, resolve_pending,
    second_line, subscribe, Line, Register, Snapshot,
};
use crate::{
    ely::{
        buttons::{Button, ButtonVariant},
        forms::{Input, InputEvent, TextInput},
        theme::ControlSize,
    },
    theme::ThemeContext,
};

/// The GPUI register over the phone's current copy.
pub struct PhonePeople {
    snapshot: Snapshot,
    register: Register,
    selected: Option<(Register, String)>,
    search: Entity<TextInput>,
    visible: Rc<Vec<Line>>,
    keyboard: fn(bool),
    _input: [Subscription; 3],
    _updates: Task<()>,
}

impl PhonePeople {
    /// Subscribes to the phone copy and binds the shared mobile search field.
    pub fn new(window: &mut Window, keyboard: fn(bool), cx: &mut Context<Self>) -> Self {
        let mut source = subscribe();
        let snapshot = source.borrow_and_update().clone();
        let search = cx.new(|cx| TextInput::new(window, cx).placeholder("Search"));
        let input = cx.subscribe_in(&search, window, |view, _, event, _, cx| {
            if matches!(event, InputEvent::Changed) {
                view.refilter(cx);
            }
        });
        let focus = search.read(cx).focus_handle(cx);
        let on_focus = cx.on_focus(&focus, window, move |_, _, _| keyboard(true));
        let on_blur = cx.on_blur(&focus, window, move |_, _, _| keyboard(false));
        let updates = cx.spawn(async move |this, cx| {
            while source.changed().await.is_ok() {
                let snapshot = source.borrow_and_update().clone();
                if this
                    .update(cx, |view, cx| view.update(snapshot, cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        let mut view = Self {
            snapshot: Snapshot::default(),
            register: snapshot.register,
            selected: None,
            search,
            visible: Rc::new(Vec::new()),
            keyboard,
            _input: [input, on_focus, on_blur],
            _updates: updates,
        };
        view.update(snapshot, cx);
        view
    }

    fn update(&mut self, snapshot: Snapshot, cx: &mut Context<Self>) {
        if self.snapshot.register != snapshot.register {
            self.register = snapshot.register;
        }
        if let Some(selection) = resolve_pending(&snapshot) {
            self.register = selection.0;
            self.selected = Some(selection);
            clear_pending(snapshot.pending_open.as_deref());
        }
        if snapshot.loaded
            && self.selected.as_ref().is_some_and(|(side, id)| match side {
                Register::People => !snapshot.people.iter().any(|person| &person.id == id),
                Register::Places => !snapshot.places.iter().any(|place| &place.id == id),
            })
        {
            self.selected = None;
        }
        self.snapshot = snapshot;
        self.refilter(cx);
    }

    fn refilter(&mut self, cx: &mut Context<Self>) {
        self.visible = Rc::new(lines(
            &self.snapshot,
            self.register,
            self.search.read(cx).text(),
        ));
        cx.notify();
    }

    fn detail(&self) -> Option<Card> {
        let (side, id) = self.selected.as_ref()?;
        match side {
            Register::People => {
                let person = self
                    .snapshot
                    .people
                    .iter()
                    .find(|person| &person.id == id)?;
                let linked =
                    places::places_of(&self.snapshot.places, &self.snapshot.links, &person.name)
                        .into_iter()
                        .map(|place| {
                            let location = place.where_line();
                            if location.is_empty() {
                                place.name.clone()
                            } else {
                                format!("{} · {location}", place.name)
                            }
                        })
                        .collect();
                Some(Card {
                    name: person.name.clone(),
                    subtitle: second_line(person),
                    fields: person_fields(person),
                    relation: "Places",
                    linked,
                })
            }
            Register::Places => {
                let place = self.snapshot.places.iter().find(|place| &place.id == id)?;
                let linked = places::people_at(&self.snapshot.links, &place.id)
                    .into_iter()
                    .map(|(name, confirmed)| {
                        format!(
                            "{name} · {}",
                            if confirmed { "confirmed" } else { "proposed" }
                        )
                    })
                    .collect();
                Some(Card {
                    name: place.name.clone(),
                    subtitle: place.kind_label().into(),
                    fields: place_fields(place),
                    relation: "People",
                    linked,
                })
            }
        }
    }
}

struct Card {
    name: String,
    subtitle: String,
    fields: Vec<(String, String)>,
    relation: &'static str,
    linked: Vec<String>,
}

impl Render for PhonePeople {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = ThemeContext::of(window, cx).palette;
        let screen = div()
            .size_full()
            .flex()
            .flex_col()
            .gap_2()
            .p_4()
            .bg(rgba(palette.surface))
            .text_color(rgb(palette.text));
        if let Some(Card {
            name,
            subtitle,
            fields,
            relation,
            linked,
        }) = self.detail()
        {
            return screen
                .child(
                    Button::new("people-back", "Back to register")
                        .size(ControlSize::Lg)
                        .variant(ButtonVariant::Ghost)
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.selected = None;
                            cx.notify();
                        })),
                )
                .child(
                    div()
                        .id("people-place-card")
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child(
                            div()
                                .text_xl()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(name),
                        )
                        .when(!subtitle.is_empty(), |card| {
                            card.child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(palette.muted))
                                    .child(subtitle),
                            )
                        })
                        .children(fields.into_iter().map(|(label, value)| {
                            div()
                                .flex()
                                .flex_col()
                                .child(div().text_sm().text_color(rgb(palette.muted)).child(label))
                                .child(value)
                        }))
                        .child(div().font_weight(FontWeight::SEMIBOLD).child(relation))
                        .when(linked.is_empty(), |card| {
                            card.child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(palette.muted))
                                    .child("None linked"),
                            )
                        })
                        .children(linked.into_iter().map(|label| div().text_sm().child(label))),
                );
        }

        let switches = [
            (Register::People, "People", self.snapshot.people.len()),
            (Register::Places, "Places", self.snapshot.places.len()),
        ];
        let visible = self.visible.clone();
        screen
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .text_xl()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("People & Places"),
                    )
                    .child(
                        Button::new("people-refresh", "Refresh")
                            .variant(ButtonVariant::Ghost)
                            .on_click(|_, _, _| refresh()),
                    ),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(palette.muted))
                    .child(self.snapshot.source.clone()),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .children(switches.map(|(side, label, count)| {
                        Button::new(label, format!("{label} {count}"))
                            .variant(if side == self.register {
                                ButtonVariant::Primary
                            } else {
                                ButtonVariant::Outline
                            })
                            .size(ControlSize::Lg)
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.register = side;
                                view.search.update(cx, |search, cx| search.set_text("", cx));
                                view.refilter(cx);
                            }))
                    })),
            )
            .child(Input::new(&self.search))
            .when_some(self.snapshot.error.clone(), |screen, error| {
                screen.child(div().text_sm().text_color(rgb(palette.red)).child(error))
            })
            .when_some(
                visible
                    .is_empty()
                    .then(|| {
                        empty_line(
                            &self.snapshot,
                            self.register,
                            !self.search.read(cx).text().trim().is_empty(),
                        )
                    })
                    .flatten(),
                |screen, hint| {
                    screen.child(div().text_sm().text_color(rgb(palette.muted)).child(hint))
                },
            )
            .child(
                uniform_list(
                    "phone-people-register",
                    visible.len(),
                    cx.processor(move |_, range: Range<usize>, _, cx| {
                        range
                            .map(|index| {
                                let row = div()
                                    .id(("people-line", index))
                                    .h(px(52.))
                                    .px_2()
                                    .flex()
                                    .flex_col()
                                    .justify_center();
                                let (side, id, title, subtitle) = match &visible[index] {
                                    Line::Header(kind) => {
                                        return row
                                            .text_sm()
                                            .text_color(rgb(palette.muted))
                                            .child(kind.clone())
                                    }
                                    Line::Person(person) => (
                                        Register::People,
                                        person.id.clone(),
                                        person.name.clone(),
                                        second_line(person),
                                    ),
                                    Line::Place(place) => (
                                        Register::Places,
                                        place.id.clone(),
                                        place.name.clone(),
                                        place.where_line(),
                                    ),
                                };
                                row.border_b_1()
                                    .border_color(rgba(palette.border))
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        (view.keyboard)(false);
                                        view.selected = Some((side, id.clone()));
                                        cx.notify();
                                    }))
                                    .child(div().truncate().child(title))
                                    .when(!subtitle.is_empty(), |row| {
                                        row.child(
                                            div()
                                                .truncate()
                                                .text_sm()
                                                .text_color(rgb(palette.muted))
                                                .child(subtitle),
                                        )
                                    })
                            })
                            .collect::<Vec<_>>()
                    }),
                )
                .flex_1()
                .min_h_0(),
            )
    }
}
