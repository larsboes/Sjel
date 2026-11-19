//! Person detail view, contact properties, place facts timeline, and duplicate merger.
use std::rc::Rc;

use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::{
    div, prelude::*, px, rems, rgb, rgba, AnyElement, Context, FontWeight, ScrollHandle,
    SharedString,
};
use sjel_gpui_core::people::Person;

use crate::ely::buttons::{Button, ButtonVariant};
use crate::ely::data_display::Tone;
use crate::ely::tables::{Cell, Column, DataTable, Row, RowAction};
use crate::ely::theme::ControlSize;
use crate::theme::{tint, Palette};

use super::forms::{FormKind, Work};
use super::PeoplePage;

impl PeoplePage {
    pub(super) fn render_person_detail(
        &self,
        person: Person,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        render_person_detail(
            person.clone(),
            palette,
            cx,
            &self.detail_scroll,
            self.confirm_remove.as_deref() == Some(person.id.as_str()),
        )
    }

    /// Probable duplicates with the registry's default merge: the side to keep and the name.
    pub(super) fn render_duplicates(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let pairs = self.duplicates.value.clone().unwrap_or_default();
        let status = if self.duplicates.loading && pairs.is_empty() {
            Some("Looking for duplicates…".to_owned())
        } else if let Some(error) = self.duplicates.error.as_ref() {
            Some(format!("Could not read duplicates: {error}"))
        } else if pairs.is_empty() {
            Some("No probable duplicates.".to_owned())
        } else {
            None
        };
        let mut list = div().flex().flex_col().gap_2().w_full();
        for (i, pair) in pairs.into_iter().enumerate() {
            let (keep, other) = if pair.keep == "b" {
                (&pair.b, &pair.a)
            } else {
                (&pair.a, &pair.b)
            };
            let name_of = |side: &sjel_gpui_core::shell::DuplicateSide| {
                side.profile["name"].as_str().unwrap_or("?").to_owned()
            };
            let (keep_id, other_id, name) =
                (keep.id.clone(), other.id.clone(), pair.merged_name.clone());
            let (a, b) = (pair.a.id.clone(), pair.b.id.clone());
            let why = if pair.reasons.is_empty() {
                pair.strength.clone()
            } else {
                format!("{} · {}", pair.strength, pair.reasons.join(", "))
            };
            list = list.child(
                div()
                    .p_3()
                    .rounded(px(6.))
                    .border_1()
                    .border_color(rgba(palette.border))
                    .bg(rgba(palette.surface))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(palette.text))
                            .child(format!("{} ↔ {}", name_of(&pair.a), name_of(&pair.b))),
                    )
                    .child(div().text_xs().text_color(rgb(palette.muted)).child(why))
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                Button::new(
                                    SharedString::from(format!("dup-merge-{i}")),
                                    format!("Merge into \"{}\"", pair.merged_name),
                                )
                                .size(ControlSize::Sm)
                                .primary()
                                .on_click(cx.listener(
                                    move |page, _, window, cx| {
                                        let Some(shell) = page.writer() else { return };
                                        let work = Work::Merge {
                                            keep: keep_id.clone(),
                                            other: other_id.clone(),
                                            name: name.clone(),
                                        };
                                        page.write(shell, work, window, cx);
                                    },
                                )),
                            )
                            .child(
                                Button::new(
                                    SharedString::from(format!("dup-distinct-{i}")),
                                    "Different people",
                                )
                                .size(ControlSize::Sm)
                                .variant(ButtonVariant::Ghost)
                                .on_click(cx.listener(
                                    move |page, _, window, cx| {
                                        let Some(shell) = page.writer() else { return };
                                        page.write(
                                            shell,
                                            Work::Distinct(a.clone(), b.clone()),
                                            window,
                                            cx,
                                        );
                                    },
                                )),
                            ),
                    ),
            );
        }
        div()
            .id("people-duplicates")
            .flex_1()
            .min_h_0()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_2()
            .overflow_y_scroll()
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child("The kept side has a note or more filled fields. Its values win where both have one."),
            )
            .when_some(status, |el, status| {
                el.child(div().text_sm().text_color(rgb(palette.muted)).child(status))
            })
            .child(list)
            .into_any_element()
    }
}

pub(super) fn render_person_detail(
    person: Person,
    palette: Palette,
    cx: &mut Context<PeoplePage>,
    scroll: &ScrollHandle,
    confirming: bool,
) -> AnyElement {
    let company_role = [person.role.as_deref(), person.company.as_deref()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" at ");
    let network = person.network_rows();
    let origins = person.origins().into_iter().collect::<Vec<_>>().join(", ");

    let header_card = div()
        .p_6()
        .rounded(px(8.))
        .border_1()
        .border_color(rgba(palette.border))
        .bg(rgba(palette.surface))
        .flex()
        .items_center()
        .gap_4()
        .child(
            div()
                .w(px(52.))
                .h(px(52.))
                .rounded_full()
                .bg(rgba(tint(palette.accent, 0x22)))
                .border_2()
                .border_color(rgb(palette.accent))
                .flex()
                .items_center()
                .justify_center()
                .text_lg()
                .font_weight(FontWeight::BOLD)
                .text_color(rgb(palette.accent))
                .child(person.initials),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_xl()
                        .font_weight(FontWeight::BOLD)
                        .text_color(rgb(palette.text))
                        .child(person.name),
                )
                .when(!company_role.is_empty(), |el| {
                    el.child(
                        div()
                            .text_sm()
                            .text_color(rgb(palette.muted))
                            .child(company_role),
                    )
                }),
        );

    // Contact fields card
    let mut contact_fields = div().flex().flex_col().gap_2().flex_none().w_full();

    if let Some(email) = person.email {
        contact_fields = contact_fields.child(detail_row("EMAIL", email, palette));
    }
    if let Some(phone) = person.phone {
        contact_fields = contact_fields.child(detail_row("PHONE", phone, palette));
    }
    if let Some(location) = person.location {
        contact_fields = contact_fields.child(detail_row("LOCATION", location, palette));
    }
    if let Some(note) = person.note_ref {
        contact_fields = contact_fields.child(detail_row("NOTE", note, palette));
    }

    for (label, value) in network {
        contact_fields = contact_fields.child(detail_row(&label.to_uppercase(), value, palette));
    }
    if !origins.is_empty() {
        contact_fields = contact_fields.child(detail_row("SOURCES", origins, palette));
    }

    for (k, v) in person.custom_fields {
        contact_fields = contact_fields.child(detail_row(&k.to_uppercase(), v, palette));
    }

    div()
        .id("person-detail-scroll")
        .flex_1()
        .min_h_0()
        .min_w_0()
        .overflow_y_scroll()
        .track_scroll(scroll)
        .vertical_scrollbar(scroll)
        .flex()
        .flex_col()
        .gap_4()
        .child(header_card.flex_none().w_full())
        .child(
            div()
                .flex_none()
                .w_full()
                .flex()
                .gap_2()
                .child(
                    Button::new("person-edit", "Edit")
                        .size(ControlSize::Sm)
                        .variant(ButtonVariant::Ghost)
                        .on_click(cx.listener(|page, _, window, cx| {
                            if let Some(id) = page.selected_id.clone() {
                                page.open_form(FormKind::Edit(id), window, cx);
                            }
                        })),
                )
                .child(
                    Button::new(
                        "person-delete",
                        if confirming {
                            "Confirm delete"
                        } else {
                            "Delete"
                        },
                    )
                    .size(ControlSize::Sm)
                    .variant(ButtonVariant::Ghost)
                    .on_click(cx.listener(|page, _, window, cx| page.remove_person(window, cx))),
                )
                .child(
                    Button::new("person-add-fact", "Add fact")
                        .size(ControlSize::Sm)
                        .variant(ButtonVariant::Ghost)
                        .on_click(cx.listener(|page, _, window, cx| {
                            if let Some(id) = page.selected_id.clone() {
                                page.open_form(FormKind::Fact(id), window, cx);
                            }
                        })),
                ),
        )
        .child(
            div()
                .flex_none()
                .w_full()
                .p_4()
                .rounded(px(8.))
                .border_1()
                .border_color(rgba(palette.border))
                .bg(rgba(palette.surface))
                .flex()
                .flex_col()
                .gap_3()
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(palette.muted))
                        .child("CONTACT & PROPERTIES"),
                )
                .child(contact_fields),
        )
        .when(!person.facts.is_empty(), |el| {
            let columns = vec![
                Column::new("predicate", "PREDICATE").width(rems(8.)),
                Column::new("place", "PLACE").min_width(rems(10.)),
                Column::new("window", "WINDOW").width(rems(12.)),
                Column::new("note", "NOTE").min_width(rems(10.)),
                Column::new("actions", "")
                    .width(rems(6.))
                    .actions([RowAction::new("remove", "Remove")]),
            ];
            let rows: Rc<Vec<Row>> = Rc::new(
                person
                    .facts
                    .iter()
                    .map(|fact| {
                        let window = match (fact.valid_from.as_deref(), fact.valid_to.as_deref()) {
                            (Some(from), Some(to)) => format!("{from} → {to}"),
                            (Some(from), None) => format!("from {from}"),
                            (None, Some(to)) => format!("until {to}"),
                            (None, None) => "Always".into(),
                        };
                        Row::new(
                            fact.id.clone(),
                            [
                                Cell::Tag(fact.predicate.clone().into(), Tone::Accent),
                                Cell::Text(fact.place.clone().into()),
                                Cell::Text(window.into()),
                                Cell::Text(
                                    fact.note
                                        .as_deref()
                                        .unwrap_or("—")
                                        .replace(['\r', '\n'], " ")
                                        .into(),
                                ),
                                Cell::Actions(vec!["remove".into()]),
                            ],
                        )
                    })
                    .collect(),
            );
            let on_act = cx.entity();
            let facts_table = DataTable::new("person-place-facts-table", columns)
                .label("Place facts")
                .rows(rows)
                .on_action(move |key, action, window, cx| {
                    if action.as_ref() == "remove" {
                        let fact_id = key.to_string();
                        on_act.update(cx, |page, cx| {
                            page.remove_fact(&fact_id, window, cx);
                        });
                    }
                })
                .empty("No place facts recorded.");
            el.child(
                div()
                    .flex_none()
                    .w_full()
                    .p_4()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(rgba(palette.border))
                    .bg(rgba(palette.surface))
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(palette.muted))
                            .child("PLACE FACTS"),
                    )
                    .child(facts_table),
            )
        })
        .into_any_element()
}

pub(super) fn detail_row(label: &str, val: String, palette: Palette) -> AnyElement {
    div()
        .flex()
        .items_center()
        .justify_between()
        .py_1p5()
        .border_b_1()
        .border_color(rgba(palette.border))
        .child(
            div()
                .w(px(120.))
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(palette.muted))
                .child(label.to_owned()),
        )
        .child(
            div()
                .flex_1()
                .text_sm()
                .text_color(rgb(palette.text))
                .child(val),
        )
        .into_any_element()
}
