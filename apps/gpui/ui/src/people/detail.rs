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
use super::{avatar_badge, PeoplePage};

impl PeoplePage {
    pub(super) fn render_person_detail(
        &self,
        person: Person,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let links = self.render_links(&person.id, palette, cx);
        let confirming = self.confirm_remove.as_deref() == Some(person.id.as_str());
        render_person_detail(person, palette, cx, &self.detail_scroll, confirming, links)
    }

    /// Probable duplicates with side-by-side field comparison and explicit keep choice.
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

        let extract_fields = |side: &sjel_gpui_core::shell::DuplicateSide| {
            let mut map = std::collections::BTreeMap::<String, String>::new();
            let parsed = self.derived.parsed.iter().find(|p| p.id == side.id);
            let name = parsed
                .map(|p| p.name.clone())
                .or_else(|| side.profile["name"].as_str().map(str::to_owned))
                .unwrap_or_else(|| "?".to_owned());
            let initials = parsed
                .map(|p| p.initials.clone())
                .unwrap_or_else(|| sjel_gpui_core::people::extract_initials(&name));
            let photo = parsed.and_then(|p| p.photo.clone());
            if let Some(note) = parsed.and_then(|p| p.note_ref.as_ref()) {
                map.insert("note".into(), note.clone());
            } else if let Some(note) = side.profile["note_ref"].as_str() {
                map.insert("note".into(), note.to_owned());
            }
            if let Some(p) = parsed {
                for (k, v) in &p.values {
                    if !v.trim().is_empty() {
                        map.insert(k.clone(), v.clone());
                    }
                }
            }
            if let Some(obj) = side.profile.get("values").and_then(|v| v.as_object()) {
                for (k, v) in obj {
                    if let Some(text) = sjel_gpui_core::people::field_text(v) {
                        if !text.trim().is_empty() {
                            map.entry(k.clone()).or_insert(text);
                        }
                    }
                }
            }
            (name, initials, photo, map)
        };

        let mut list = div().flex().flex_col().gap_3().w_full();
        for (i, pair) in pairs.into_iter().enumerate() {
            let (name_a, init_a, photo_a, map_a) = extract_fields(&pair.a);
            let (name_b, init_b, photo_b, map_b) = extract_fields(&pair.b);
            let mut all_keys = std::collections::BTreeSet::new();
            all_keys.extend(map_a.keys().cloned());
            all_keys.extend(map_b.keys().cloned());

            let default_keep_b = pair.keep == "b";
            let (id_a, id_b) = (pair.a.id.clone(), pair.b.id.clone());
            let why = if pair.reasons.is_empty() {
                pair.strength.clone()
            } else {
                format!("{} · {}", pair.strength, pair.reasons.join(", "))
            };

            let render_side_card = |label_side: &'static str,
                                    keep_id: String,
                                    other_id: String,
                                    target_name: String,
                                    initials: String,
                                    photo: Option<String>,
                                    fields: &std::collections::BTreeMap<String, String>,
                                    other_fields: &std::collections::BTreeMap<String, String>,
                                    is_recommended: bool,
                                    btn_id: String| {
                let mut rows = div().flex().flex_col().gap_1();
                if all_keys.is_empty() {
                    rows = rows.child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child("No stored field values."),
                    );
                } else {
                    for k in &all_keys {
                        let val = fields.get(k).cloned().unwrap_or_else(|| "—".to_owned());
                        let other_val = other_fields.get(k);
                        let differs = other_val.is_some_and(|ov| ov != &val) && val != "—";
                        rows = rows.child(
                            div()
                                .flex()
                                .items_baseline()
                                .justify_between()
                                .gap_2()
                                .py_0p5()
                                .border_b_1()
                                .border_color(rgba(palette.border))
                                .child(
                                    div()
                                        .w(px(92.))
                                        .flex_none()
                                        .text_xs()
                                        .text_color(rgb(palette.muted))
                                        .truncate()
                                        .child(k.replace('_', " ")),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_xs()
                                        .text_color(rgb(if val == "—" {
                                            palette.muted
                                        } else if differs {
                                            palette.amber
                                        } else {
                                            palette.text
                                        }))
                                        .child(val),
                                ),
                        );
                    }
                }
                let btn_label = if is_recommended {
                    format!("Keep {label_side} (Recommended)")
                } else {
                    format!("Keep {label_side}")
                };
                div()
                    .flex_1()
                    .min_w(px(260.))
                    .p_3()
                    .rounded(px(6.))
                    .border_1()
                    .border_color(if is_recommended {
                        rgb(palette.accent)
                    } else {
                        rgba(palette.border)
                    })
                    .bg(rgba(palette.raised))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .min_w_0()
                                    .child(avatar_badge(&initials, photo.as_deref(), 28.0, palette))
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(rgb(palette.text))
                                            .truncate()
                                            .child(target_name.clone()),
                                    ),
                            )
                            .when(is_recommended, |el| {
                                el.child(
                                    div()
                                        .flex_none()
                                        .px_1p5()
                                        .py(px(1.))
                                        .rounded(px(4.))
                                        .bg(rgba(tint(palette.green, 0x20)))
                                        .text_xs()
                                        .text_color(rgb(palette.green))
                                        .child("KEEP"),
                                )
                            }),
                    )
                    .child(rows)
                    .child(
                        Button::new(SharedString::from(btn_id), btn_label)
                            .size(ControlSize::Sm)
                            .variant(if is_recommended {
                                ButtonVariant::Primary
                            } else {
                                ButtonVariant::Ghost
                            })
                            .on_click(cx.listener(move |page, _, window, cx| {
                                let Some(shell) = page.writer() else { return };
                                let work = Work::Merge {
                                    keep: keep_id.clone(),
                                    other: other_id.clone(),
                                    name: target_name.clone(),
                                };
                                page.write(shell, work, window, cx);
                            })),
                    )
            };

            list = list.child(
                div()
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
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_0p5()
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(rgb(palette.text))
                                            .child(format!("{name_a} ↔ {name_b}")),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(rgb(palette.muted))
                                            .child(why),
                                    ),
                            )
                            .child(
                                Button::new(
                                    SharedString::from(format!("dup-distinct-{i}")),
                                    "Different people",
                                )
                                .size(ControlSize::Sm)
                                .variant(ButtonVariant::Ghost)
                                .on_click(cx.listener({
                                    let a = id_a.clone();
                                    let b = id_b.clone();
                                    move |page, _, window, cx| {
                                        let Some(shell) = page.writer() else { return };
                                        page.write(
                                            shell,
                                            Work::Distinct(a.clone(), b.clone()),
                                            window,
                                            cx,
                                        );
                                    }
                                })),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_3()
                            .child(render_side_card(
                                "A",
                                id_a.clone(),
                                id_b.clone(),
                                name_a.clone(),
                                init_a,
                                photo_a,
                                &map_a,
                                &map_b,
                                !default_keep_b,
                                format!("dup-merge-a-{i}"),
                            ))
                            .child(render_side_card(
                                "B",
                                id_b.clone(),
                                id_a.clone(),
                                name_b.clone(),
                                init_b,
                                photo_b,
                                &map_b,
                                &map_a,
                                default_keep_b,
                                format!("dup-merge-b-{i}"),
                            )),
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
            .gap_3()
            .overflow_y_scroll()
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child("Compare both cards side by side (differing values are highlighted in amber), then choose which record to keep."),
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
    links: AnyElement,
) -> AnyElement {
    let today = chrono::Local::now().date_naive();
    let company_role = [person.role.as_deref(), person.company.as_deref()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" at ");
    let follow_up_due = person.follow_up_due();
    let overdue = person.is_overdue(today);
    let person_id = person.id.clone();
    let person_rev = person.revision;

    let status_badge = follow_up_due.map(|due| {
        let (label, color) = if overdue {
            (format!("Overdue · due {due}"), palette.red)
        } else {
            (format!("Follow-up {due}"), palette.green)
        };
        div()
            .px_2()
            .py_0p5()
            .rounded_full()
            .bg(rgba(tint(color, 0x1e)))
            .text_xs()
            .font_weight(FontWeight::MEDIUM)
            .text_color(rgb(color))
            .child(label)
    });

    let header_card = div()
        .p_5()
        .rounded(px(8.))
        .border_1()
        .border_color(rgba(palette.border))
        .bg(rgba(palette.surface))
        .flex()
        .flex_wrap()
        .items_center()
        .justify_between()
        .gap_4()
        .child(
            div()
                .flex()
                .items_center()
                .gap_4()
                .child(avatar_badge(
                    &person.initials,
                    person.photo.as_deref(),
                    54.0,
                    palette,
                ))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .text_xl()
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(rgb(palette.text))
                                        .child(person.name.clone()),
                                )
                                .children(status_badge),
                        )
                        .when(!company_role.is_empty(), |el| {
                            el.child(
                                div()
                                    .text_sm()
                                    .text_color(rgb(palette.muted))
                                    .child(company_role),
                            )
                        }),
                ),
        )
        .child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_1p5()
                .child(
                    Button::new("person-edit", "Edit")
                        .size(ControlSize::Sm)
                        .primary()
                        .on_click(cx.listener(|page, _, window, cx| {
                            if let Some(id) = page.selected_id.clone() {
                                page.open_form(FormKind::Edit(id), window, cx);
                            }
                        })),
                )
                .child(
                    Button::new("person-log-contact", "Log contact today")
                        .size(ControlSize::Sm)
                        .variant(ButtonVariant::Ghost)
                        .on_click(cx.listener({
                            let pid = person_id.clone();
                            move |page, _, window, cx| {
                                page.mark_contacted_today(&pid, person_rev, window, cx);
                            }
                        })),
                )
                .child(
                    Button::new("person-pick-photo", "Photo…")
                        .size(ControlSize::Sm)
                        .variant(ButtonVariant::Ghost)
                        .on_click(cx.listener(move |page, _, _, cx| {
                            page.pick_photo(person_id.clone(), person_rev, cx);
                        })),
                )
                .child(
                    Button::new("person-add-fact", "+ Fact")
                        .size(ControlSize::Sm)
                        .variant(ButtonVariant::Ghost)
                        .on_click(cx.listener(|page, _, window, cx| {
                            if let Some(id) = page.selected_id.clone() {
                                page.open_form(FormKind::Fact(id), window, cx);
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
                ),
        );

    // Card 1: Contact & Identity (2-column grid of tiles with source provenance)
    let mut contact_tiles: Vec<AnyElement> = Vec::new();
    let src_of = |key: &str| person.sources.get(key).map(String::as_str);
    if let Some(email) = &person.email {
        let mailto = format!("mailto:{}", email.split(',').next().unwrap_or(email).trim());
        contact_tiles.push(field_tile(
            "EMAIL",
            email.clone(),
            src_of("emails").or_else(|| src_of("email")),
            Some(mailto),
            palette,
        ));
    }
    if let Some(phone) = &person.phone {
        contact_tiles.push(field_tile(
            "PHONE",
            phone.clone(),
            src_of("phones").or_else(|| src_of("phone")),
            None,
            palette,
        ));
    }
    if let Some(location) = &person.location {
        contact_tiles.push(field_tile(
            "LOCATION",
            location.clone(),
            src_of("location"),
            None,
            palette,
        ));
    }
    if let Some(linkedin) = person.values.get("linkedin") {
        let url = if linkedin.starts_with("http") {
            Some(linkedin.clone())
        } else {
            None
        };
        contact_tiles.push(field_tile(
            "LINKEDIN",
            linkedin.clone(),
            src_of("linkedin"),
            url,
            palette,
        ));
    }
    if let Some(aliases) = person.values.get("aliases") {
        contact_tiles.push(field_tile(
            "ALIASES",
            aliases.clone(),
            src_of("aliases"),
            None,
            palette,
        ));
    }
    if let Some(note) = &person.note_ref {
        contact_tiles.push(field_tile("NOTE", note.clone(), None, None, palette));
    }

    let contact_card = (!contact_tiles.is_empty()).then(|| {
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
                    .child("CONTACT & IDENTITY"),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .children(contact_tiles),
            )
    });

    // Card 2: Relationship & Cadence
    let mut rel_tiles: Vec<AnyElement> = Vec::new();
    if let Some(met_at) = person.values.get("met_at") {
        rel_tiles.push(field_tile("HOW WE MET", met_at.clone(), src_of("met_at"), None, palette));
    }
    if let Some(connected) = person.values.get("connected_on") {
        rel_tiles.push(field_tile("CONNECTED", connected.clone(), src_of("connected_on"), None, palette));
    }
    if let Some(last) = person.values.get("last_contact") {
        rel_tiles.push(field_tile("LAST CONTACT", last.clone(), src_of("last_contact"), None, palette));
    }
    if let Some(weeks) = person.values.get(sjel_gpui_core::people::FOLLOW_UP_WEEKS) {
        rel_tiles.push(field_tile("CADENCE", format!("Every {weeks}w"), None, None, palette));
    }
    if let Some(due) = follow_up_due {
        rel_tiles.push(field_tile("NEXT FOLLOW-UP", due.to_string(), None, None, palette));
    }

    let relationship_card = (!rel_tiles.is_empty()).then(|| {
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
                    .child("RELATIONSHIP & CADENCE"),
            )
            .child(div().flex().flex_wrap().gap_2().children(rel_tiles))
    });

    // Card 3: Tags & Custom Fields (skip `aliases` since shown in Identity)
    let custom_rows: Vec<_> = person
        .custom_fields
        .iter()
        .filter(|(k, _)| k != "aliases")
        .collect();
    let extra_card = (!person.tags.is_empty() || !custom_rows.is_empty()).then(|| {
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
                    .child("TAGS & CUSTOM FIELDS"),
            )
            .when(!person.tags.is_empty(), |el| {
                el.child(div().flex().flex_wrap().gap_1p5().children(person.tags.iter().map(|tag| {
                    div()
                        .px_2()
                        .py_0p5()
                        .rounded_full()
                        .bg(rgba(tint(palette.accent, 0x18)))
                        .text_xs()
                        .text_color(rgb(palette.accent))
                        .child(tag.clone())
                })))
            })
            .when(!custom_rows.is_empty(), |el| {
                el.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_2()
                        .children(custom_rows.into_iter().map(|(k, v)| {
                            field_tile(&k.replace('_', " ").to_uppercase(), v.clone(), src_of(k), None, palette)
                        })),
                )
            })
    });

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
        .children(contact_card)
        .children(relationship_card)
        .child(links)
        .children(extra_card)
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

fn field_tile(
    label: &str,
    val: String,
    source: Option<&str>,
    open_url: Option<String>,
    palette: Palette,
) -> AnyElement {
    let source_pill = source
        .filter(|s| *s != "operator")
        .map(|s| {
            div()
                .px_1()
                .rounded(px(3.))
                .bg(rgba(palette.surface))
                .text_xs()
                .text_color(rgb(palette.muted))
                .child(s.to_owned())
        });
    let value_el = if let Some(url) = open_url {
        div()
            .id(SharedString::from(format!("person-tile-{label}-{val}")))
            .text_sm()
            .text_color(rgb(palette.accent))
            .truncate()
            .cursor_pointer()
            .on_click(move |_, _, cx| cx.open_url(&url))
            .child(val)
            .into_any_element()
    } else {
        div()
            .text_sm()
            .text_color(rgb(palette.text))
            .truncate()
            .child(val)
            .into_any_element()
    };
    div()
        .min_w(px(210.))
        .flex_1()
        .p_2p5()
        .rounded(px(6.))
        .bg(rgba(palette.raised))
        .flex()
        .flex_col()
        .gap_0p5()
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(palette.muted))
                        .child(label.to_owned()),
                )
                .children(source_pill),
        )
        .child(value_el)
        .into_any_element()
}
