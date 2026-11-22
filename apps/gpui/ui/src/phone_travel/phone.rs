//! Trips and station search over the phone travel controller.

use gpui::{
    div, prelude::*, px, rgb, rgba, AnyElement, ClipboardItem, Context, Div, Entity, Focusable,
    FontWeight, SharedString, Subscription, Task, Window,
};

use super::{
    download, feed, load, open, phone_map, resolve, search, suggest, travel, Plan, PlanExt, Query,
    Snapshot, Station, TravelRoute, TripRead, TripSection,
};
use crate::{
    ely::{
        buttons::{Button, ButtonVariant},
        dashboard::{MetricStrip, MetricTile},
        data_display::{Badge, Tag, Tone},
        forms::{Input, InputEvent, TextInput},
        layout::Section as ElySection,
        lists::List,
        theme::ControlSize,
    },
    phone_calendar,
    phone_navigation::{self, Page},
    primitives::{back_button, sub_tabs, surface_card},
    theme::{Palette, ThemeContext},
};
use sjel_gpui_core::travel::connections::{self, Rank, Sort};

pub struct PhoneTravel {
    snapshot: Snapshot,
    fields: [Entity<TextInput>; 5],
    stations: [Option<Station>; 2],
    sort: Sort,
    priority: Option<&'static str>,
    split: bool,
    past: bool,
    day: Option<String>,
    _input: Vec<Subscription>,
    _updates: Task<()>,
}

impl PhoneTravel {
    pub fn new(window: &mut Window, keyboard: fn(bool), cx: &mut Context<Self>) -> Self {
        let mut source = feed().tx.subscribe();
        let snapshot = source.borrow_and_update().clone();
        let fields = [
            "From station",
            "To station",
            "Date (YYYY-MM-DD)",
            "Time (HH:MM)",
            "Ranking request",
        ]
        .map(|label| cx.new(|cx| TextInput::new(window, cx).placeholder(label)));
        fields[2].update(cx, |field, cx| field.set_text(connections::tomorrow(), cx));
        fields[3].update(cx, |field, cx| field.set_text("10:00", cx));
        let mut subscriptions = Vec::new();
        for (index, field) in fields.iter().enumerate() {
            let focus = field.read(cx).focus_handle(cx);
            subscriptions.push(cx.on_focus(&focus, window, move |_, _, _| keyboard(true)));
            subscriptions.push(cx.on_blur(&focus, window, move |_, _, _| keyboard(false)));
            if index < 2 {
                subscriptions.push(cx.subscribe_in(
                    field,
                    window,
                    move |view, field, event, _, cx| {
                        let text = field.read(cx).text();
                        if matches!(event, InputEvent::Changed)
                            && view.stations[index]
                                .as_ref()
                                .is_none_or(|station| station.name != text)
                        {
                            view.stations[index] = None;
                            suggest(index, text.to_owned());
                            cx.notify();
                        }
                    },
                ));
            }
        }
        let updates = cx.spawn(async move |view, cx| {
            while source.changed().await.is_ok() {
                let snapshot = source.borrow_and_update().clone();
                if view
                    .update(cx, |view, cx| {
                        if snapshot.route != view.snapshot.route {
                            view.day = None;
                        }
                        view.snapshot = snapshot;
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            snapshot,
            fields,
            stations: [None, None],
            sort: Sort::Ranked,
            priority: None,
            split: false,
            past: false,
            day: None,
            _input: subscriptions,
            _updates: updates,
        }
    }

    fn connections(&self, palette: Palette, cx: &Context<Self>) -> Div {
        let mut form = surface_card(palette)
            .p_4()
            .flex()
            .flex_col()
            .gap_2()
            .child(heading("Connections", palette));
        for index in 0..2 {
            form = form.child(Input::new(&self.fields[index]));
            if self.stations[index].is_none() && !self.snapshot.suggestions[index].is_empty() {
                form = form.child(
                    div().flex().flex_wrap().gap_1().children(
                        self.snapshot.suggestions[index]
                            .iter()
                            .cloned()
                            .map(|station| {
                                secondary_button(station.name.clone()).on_click(cx.listener(
                                    move |view, _, window, cx| {
                                        view.stations[index] = Some(station.clone());
                                        view.fields[index].update(cx, |field, cx| {
                                            field.set_text(station.name.clone(), cx)
                                        });
                                        window.blur(cx);
                                        cx.notify();
                                    },
                                ))
                            }),
                    ),
                );
            }
        }
        form =
            form.children(self.fields[2..].iter().map(Input::new))
                .child(div().flex().flex_wrap().gap_1().children(
                    connections::PRIORITIES.iter().map(|&(key, label, _)| {
                        choice(label, self.priority == Some(key)).on_click(cx.listener(
                            move |view, _, _, cx| {
                                view.priority = Some(key);
                                cx.notify();
                            },
                        ))
                    }),
                ))
                .child(
                    Button::new("travel-search-connections", "Search connections")
                        .size(ControlSize::Md)
                        .variant(ButtonVariant::Primary)
                        .full_width()
                        .disabled(
                            self.stations.iter().any(Option::is_none) || self.snapshot.searching,
                        )
                        .on_click(cx.listener(|view, _, window, cx| {
                            if let [Some(from), Some(to)] = &view.stations {
                                search(Query {
                                    from: from.clone(),
                                    to: to.clone(),
                                    date: view.fields[2].read(cx).text().to_owned(),
                                    time: view.fields[3].read(cx).text().to_owned(),
                                    rank: Rank {
                                        priority: view.priority,
                                        phrase: view.fields[4].read(cx).text().to_owned(),
                                    },
                                });
                                window.blur(cx);
                            }
                        })),
                );

        let mut screen = column().gap_3().child(form);
        if self.snapshot.searching {
            screen = screen.child(note("Searching…", palette));
        }
        if let Some(error) = &self.snapshot.search_error {
            screen = screen.child(
                div()
                    .text_sm()
                    .text_color(rgb(palette.red))
                    .child(error.clone()),
            );
        }
        screen
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_1()
                    .children(Sort::ALL.map(|sort| {
                        choice(sort.label(), self.sort == sort).on_click(cx.listener(
                            move |view, _, _, cx| {
                                view.sort = sort;
                                cx.notify();
                            },
                        ))
                    })),
            )
            .child({
                let on_select = cx.listener(|view, idx: &usize, _, cx| {
                    view.split = *idx == 1;
                    cx.notify();
                });
                sub_tabs(
                    "travel-connections-mode",
                    vec![
                        ("Direct", None, !self.split),
                        ("Split tickets", None, self.split),
                    ],
                    move |idx, window, cx| on_select(&idx, window, cx),
                )
            })
            .when(self.split, |list| {
                list.when_some(self.snapshot.split.as_ref(), |list, result| match result {
                    Err(error) => list.child(
                        div()
                            .text_sm()
                            .text_color(rgb(palette.red))
                            .child(error.clone()),
                    ),
                    Ok(result) => list
                        .child(
                            surface_card(palette)
                                .p_3()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .child(heading(
                                    format!("Split fare · {:.2} EUR", result.split_price),
                                    palette,
                                ))
                                .children(result.confidence.note().map(|n| note(n, palette))),
                        )
                        .children(
                            result
                                .segments
                                .iter()
                                .map(|part| journey(&part.journey, palette)),
                        ),
                })
            })
            .when(!self.split, |list| {
                list.children(
                    connections::ordered(&self.snapshot.journeys, self.sort)
                        .into_iter()
                        .map(|j| journey(j, palette)),
                )
            })
    }

    fn trip(
        &self,
        plan: &Plan,
        read: Option<&TripRead>,
        section: TripSection,
        palette: Palette,
        cx: &Context<Self>,
    ) -> Div {
        let id = plan.id.clone();
        let download_id = id.clone();
        let locate_id = id.clone();
        let readiness = plan.check_offline_readiness();
        let offline = self.snapshot.offline.get(&id).cloned();
        let route_summary = format!(
            "{} → {}",
            plan.origin.name,
            plan.destinations
                .iter()
                .map(|place| place.name.as_str())
                .collect::<Vec<_>>()
                .join(" · ")
        );

        let hero = surface_card(palette)
            .p_4()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(back_button(
                        "All trips",
                        |_, _, _| open(TravelRoute::Trips),
                        palette,
                    ))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1p5()
                            .child(offline_badge(plan))
                            .child(status_badge(&plan.status)),
                    ),
            )
            .child(
                div()
                    .text_xl()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(palette.text))
                    .child(plan.title.clone()),
            )
            .child(note(
                format!("{} – {} · {route_summary}", plan.date_start, plan.date_end),
                palette,
            ))
            .when_some(offline.clone(), |card, msg| card.child(note(msg, palette)))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .pt_1()
                    .when(offline.is_none() && !readiness.is_ready(), |bar| {
                        bar.child(
                            secondary_button("Download offline pack")
                                .on_click(move |_, _, _| download(download_id.clone())),
                        )
                    })
                    .child(
                        secondary_button("Locate stops")
                            .on_click(move |_, _, _| resolve(locate_id.clone())),
                    ),
            );

        let section_tabs = {
            let id = id.clone();
            sub_tabs(
                "phone-trip-section",
                TripSection::ALL
                    .iter()
                    .map(|tab| (tab.label(), None, *tab == section))
                    .collect(),
                move |idx, _, _| {
                    if let Some(&tab) = TripSection::ALL.get(idx) {
                        open(TravelRoute::Trip {
                            id: id.clone(),
                            section: tab,
                        });
                    }
                },
            )
        };

        let mut screen = column().gap_3().child(hero).child(section_tabs);
        let Some(read) = read else {
            return screen.child(
                surface_card(palette)
                    .p_4()
                    .child(note("Reading trip details…", palette)),
            );
        };
        if let Some(error) = &read.error {
            screen = screen.child(
                div()
                    .text_sm()
                    .text_color(rgb(palette.red))
                    .child(error.clone()),
            );
        }
        let Some(saved) = &read.saved else {
            return screen;
        };
        let items = &saved.details.items;
        match section {
            TripSection::Overview => {
                let mut overview_card = surface_card(palette)
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(heading("Route & companions", palette))
                    .child(note(route_summary, palette));
                if !plan.travelers.is_empty() {
                    overview_card =
                        overview_card.child(div().flex().flex_wrap().gap_1p5().children(
                            plan.travelers.iter().map(|traveler| {
                                Tag::new(
                                    SharedString::from(format!("traveler-{traveler}")),
                                    traveler.clone(),
                                )
                                .tone(Tone::Accent)
                            }),
                        ));
                }
                if !plan.interests.is_empty() {
                    overview_card = overview_card.child(note(plan.interests.clone(), palette));
                }
                screen = screen.child(overview_card);

                if let Some(cost) = &read.cost {
                    let mut strip = MetricStrip::new();
                    for money in &cost.by_currency {
                        strip = strip.child(MetricTile::new(
                            SharedString::from(format!("booked-{}", money.currency)),
                            "Booked",
                            format!("{:.2} {}", money.booked_cents as f64 / 100., money.currency),
                        ));
                    }
                    if let (Some(cents), Some(currency)) =
                        (cost.actuals.personal_cents, &cost.actuals.currency)
                    {
                        strip = strip.child(MetricTile::new(
                            "actual-personal-cost",
                            "Actual personal",
                            format!("{:.2} {currency}", cents as f64 / 100.),
                        ));
                    }
                    let mut cost_card = surface_card(palette)
                        .p_3()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(ElySection::new("COSTS").child(strip));
                    if cost.actuals.personal_cents.is_none() {
                        if let Some(reason) = &cost.actuals.reason {
                            cost_card = cost_card.child(note(reason.clone(), palette));
                        }
                    }
                    screen = screen.child(cost_card);
                }

                if !read.calendar.is_empty() {
                    let rows: Vec<AnyElement> = read
                        .calendar
                        .iter()
                        .map(|entry| calendar(entry, palette).into_any_element())
                        .collect();
                    screen = screen.child(surface_card(palette).p_3().child(
                        ElySection::new("CALENDAR").child(List::new().divided().children(rows)),
                    ));
                }
                if !read.links.is_empty() {
                    let rows: Vec<AnyElement> = read
                        .links
                        .iter()
                        .map(|link| note(link.title.clone(), palette).into_any_element())
                        .collect();
                    screen = screen.child(surface_card(palette).p_3().child(
                        ElySection::new("LINKS").child(List::new().divided().children(rows)),
                    ));
                }
                screen
            }
            TripSection::Itinerary => {
                let days: std::collections::BTreeSet<_> = plan
                    .stages
                    .iter()
                    .filter_map(|stage| stage.date.clone())
                    .chain(items.iter().filter_map(|item| item.day.clone()))
                    .chain(
                        read.calendar
                            .iter()
                            .filter_map(|entry| entry.starts_at.get(..10).map(str::to_owned)),
                    )
                    .chain(
                        read.links
                            .iter()
                            .filter_map(|link| link.at.as_deref()?.get(..10).map(str::to_owned)),
                    )
                    .collect();
                screen
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_1()
                            .child(choice("All days", self.day.is_none()).on_click(cx.listener(
                                |view, _, _, cx| {
                                    view.day = None;
                                    cx.notify();
                                },
                            )))
                            .children(days.iter().cloned().map(|day| {
                                choice(day.clone(), self.day.as_ref() == Some(&day)).on_click(
                                    cx.listener(move |view, _, _, cx| {
                                        view.day = Some(day.clone());
                                        cx.notify();
                                    }),
                                )
                            })),
                    )
                    .children(
                        days.iter()
                            .filter(|day| self.day.as_ref().is_none_or(|active| active == *day))
                            .map(|day| {
                                let stages: Vec<_> = plan
                                    .stages
                                    .iter()
                                    .filter(|stage| stage.date.as_ref() == Some(day))
                                    .collect();
                                surface_card(palette)
                                    .p_3()
                                    .flex()
                                    .flex_col()
                                    .gap_2()
                                    .child(heading(day.clone(), palette))
                                    .children(stages.iter().enumerate().map(|(index, stage)| {
                                        let id = id.clone();
                                        let modes = stage
                                            .transport_modes
                                            .iter()
                                            .map(|mode| mode.as_str())
                                            .collect::<Vec<_>>()
                                            .join(" / ");
                                        sub_card(palette)
                                            .child(
                                                div()
                                                    .flex()
                                                    .items_center()
                                                    .justify_between()
                                                    .gap_2()
                                                    .child(heading(
                                                        format!(
                                                            "{} → {}",
                                                            stage.origin.name,
                                                            stage.destination.name
                                                        ),
                                                        palette,
                                                    ))
                                                    .child(status_badge(stage.status.as_str())),
                                            )
                                            .child(note(modes, palette))
                                            .when_some(stage.branch_note.clone(), |c, branch| {
                                                c.child(note(branch, palette))
                                            })
                                            .when_some(stages.get(index + 1), |card, next| {
                                                let from = &stage.destination.name;
                                                let to = &next.origin.name;
                                                let same = from == to;
                                                card.child(
                                                    Badge::new(if same {
                                                        format!("Transfer at {from} · same station")
                                                    } else {
                                                        format!("Transfer buffer · {from} → {to}")
                                                    })
                                                    .tone(if same {
                                                        Tone::Success
                                                    } else {
                                                        Tone::Warning
                                                    }),
                                                )
                                            })
                                            .when_some(
                                                stage.destination.coordinate(),
                                                |card, at| {
                                                    card.child(
                                                        secondary_button("Show destination on map")
                                                            .on_click(move |_, _, _| {
                                                                open(TravelRoute::Trip {
                                                                    id: id.clone(),
                                                                    section: TripSection::Map,
                                                                });
                                                                phone_map::show(Some(at));
                                                            }),
                                                    )
                                                },
                                            )
                                    }))
                                    .children(
                                        read.calendar
                                            .iter()
                                            .filter(|entry| {
                                                entry.starts_at.starts_with(day.as_str())
                                            })
                                            .map(|entry| calendar(entry, palette)),
                                    )
                                    .children(
                                        items
                                            .iter()
                                            .filter(|entry| entry.day.as_ref() == Some(day))
                                            .map(|entry| {
                                                let location = entry
                                                    .payload
                                                    .get("address")
                                                    .or_else(|| entry.payload.get("location"))
                                                    .and_then(|value| value.as_str());
                                                let at = location
                                                    .and_then(|name| {
                                                        travel::itinerary::find_place_for_name(
                                                            name, plan,
                                                        )
                                                    })
                                                    .and_then(|place| place.coordinate());
                                                let id = id.clone();
                                                item(entry, palette).when_some(at, |card, at| {
                                                    card.child(
                                                        secondary_button("Show booking on map")
                                                            .on_click(move |_, _, _| {
                                                                open(TravelRoute::Trip {
                                                                    id: id.clone(),
                                                                    section: TripSection::Map,
                                                                });
                                                                phone_map::show(Some(at));
                                                            }),
                                                    )
                                                })
                                            }),
                                    )
                                    .children(
                                        read.links
                                            .iter()
                                            .filter(|link| {
                                                link.at
                                                    .as_deref()
                                                    .is_some_and(|at| at.starts_with(day.as_str()))
                                            })
                                            .map(|link| note(link.title.clone(), palette)),
                                    )
                            }),
                    )
                    .children(
                        items
                            .iter()
                            .filter(|entry| entry.day.is_none())
                            .map(|entry| item(entry, palette)),
                    )
            }
            _ => screen.children(items.iter().map(|entry| item(entry, palette))),
        }
    }

    fn trips(&self, palette: Palette, cx: &Context<Self>) -> Div {
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let (mut upcoming, mut past): (Vec<_>, Vec<_>) = self
            .snapshot
            .plans
            .iter()
            .partition(|plan| travel::upcoming(plan, &today));
        upcoming.sort_by(|a, b| a.date_start.cmp(&b.date_start));
        past.sort_by(|a, b| b.date_start.cmp(&a.date_start));

        let mut list = column()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .text_xl()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(palette.text))
                            .child("Trips"),
                    )
                    .child(
                        secondary_button(if self.past {
                            "Upcoming only"
                        } else {
                            "Include past"
                        })
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.past = !view.past;
                            cx.notify();
                        })),
                    ),
            )
            .when(!self.snapshot.status.is_empty(), |col| {
                col.child(note(self.snapshot.status.clone(), palette))
            })
            .when(
                !self.snapshot.loading && self.snapshot.plans.is_empty(),
                |view| {
                    view.child(surface_card(palette).p_4().child(note(
                        "No saved trips. Refresh while connected to the Mac.",
                        palette,
                    )))
                },
            );

        let trip_card = |plan: &Plan| {
            let id = plan.id.clone();
            surface_card(palette)
                .id(SharedString::from(format!("phone-trip-{}", plan.id)))
                .p_3()
                .flex()
                .items_center()
                .justify_between()
                .gap_3()
                .cursor_pointer()
                .on_click(move |_, _, _| {
                    open(TravelRoute::Trip {
                        id: id.clone(),
                        section: TripSection::Overview,
                    })
                })
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_base()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(palette.text))
                                .truncate()
                                .child(plan.title.clone()),
                        )
                        .child(note(
                            format!(
                                "{} – {} · {} stages",
                                plan.date_start,
                                plan.date_end,
                                plan.stages.len()
                            ),
                            palette,
                        )),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .child(offline_badge(plan))
                        .child(status_badge(&plan.status)),
                )
        };

        if !upcoming.is_empty() {
            list = list
                .child(section_label("UPCOMING TRIPS", palette))
                .children(upcoming.into_iter().map(trip_card));
        }
        if self.past && !past.is_empty() {
            list = list
                .child(section_label("PAST TRIPS", palette))
                .children(past.into_iter().map(trip_card));
        }
        list
    }
}

impl Render for PhoneTravel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = ThemeContext::of(window, cx).palette;
        let body = match &self.snapshot.route {
            TravelRoute::Connections => self.connections(palette, cx),
            TravelRoute::Trip { id, section } => {
                let read = self.snapshot.details.get(id);
                let plan = read
                    .and_then(|read| read.saved.as_ref())
                    .map(|saved| &saved.details.plan)
                    .or_else(|| self.snapshot.plans.iter().find(|plan| &plan.id == id));
                plan.map_or_else(
                    || {
                        column().child(
                            surface_card(palette)
                                .p_4()
                                .child(note("This trip has not been saved on the phone.", palette)),
                        )
                    },
                    |plan| self.trip(plan, read, *section, palette, cx),
                )
            }
            _ => self.trips(palette, cx),
        };
        let top_tabs = sub_tabs(
            "phone-travel-nav",
            vec![
                (
                    "Trips",
                    None,
                    matches!(
                        self.snapshot.route,
                        TravelRoute::Trips | TravelRoute::Trip { .. }
                    ),
                ),
                (
                    "Map & Places",
                    None,
                    self.snapshot.route == TravelRoute::Map,
                ),
                (
                    "Connections",
                    None,
                    self.snapshot.route == TravelRoute::Connections,
                ),
            ],
            |idx, _, _| {
                let route = match idx {
                    1 => TravelRoute::Map,
                    2 => TravelRoute::Connections,
                    _ => TravelRoute::Trips,
                };
                open(route);
            },
        );
        column()
            .size_full()
            .gap_3()
            .p_3()
            .bg(rgba(palette.window_tint))
            .text_color(rgb(palette.text))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(div().flex_1().min_w_0().child(top_tabs))
                    .child(
                        Button::new("travel-refresh", "Refresh")
                            .size(ControlSize::Sm)
                            .variant(ButtonVariant::Ghost)
                            .disabled(self.snapshot.loading)
                            .on_click(|_, _, _| load()),
                    ),
            )
            .when(self.snapshot.loading, |view| {
                view.child(note("Reading trips…", palette))
            })
            .when_some(self.snapshot.error.clone(), |view, error| {
                view.child(div().text_sm().text_color(rgb(palette.red)).child(error))
            })
            .child(
                div()
                    .id("travel-content")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(body),
            )
    }
}

fn sub_card(palette: Palette) -> Div {
    div()
        .p_3()
        .rounded(px(10.))
        .bg(rgba(palette.raised))
        .border_1()
        .border_color(rgba(palette.border))
        .flex()
        .flex_col()
        .gap_1p5()
}

fn section_label(label: &'static str, palette: Palette) -> Div {
    div()
        .pt_1()
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(palette.muted))
        .child(label)
}

fn status_badge(status: &str) -> Badge {
    let tone = match status {
        "confirmed" | "booked" | "done" => Tone::Success,
        "planned" | "active" => Tone::Accent,
        "cancelled" => Tone::Danger,
        _ => Tone::Neutral,
    };
    Badge::new(status.replace('_', " ")).tone(tone)
}

fn offline_badge(plan: &Plan) -> Badge {
    let readiness = plan.check_offline_readiness();
    if readiness.is_ready() {
        Badge::new("Offline").tone(Tone::Success)
    } else {
        Badge::new(format!("{}%", readiness.percent())).tone(Tone::Neutral)
    }
}

fn item(item: &travel::Item, palette: Palette) -> Div {
    let badge = if item.item_type == "packing" {
        if item.is_packed() {
            Badge::new("Packed").tone(Tone::Success)
        } else {
            Badge::new("Still to pack").tone(Tone::Warning)
        }
    } else {
        Badge::new(item.item_type.replace('_', " ")).tone(Tone::Neutral)
    };
    sub_card(palette)
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_2()
                .child(heading(item.title.clone(), palette))
                .child(badge),
        )
        .child(note(
            item.day.as_deref().unwrap_or("Undated").to_owned(),
            palette,
        ))
        .children(
            item.fields()
                .into_iter()
                .map(|(key, value)| note(format!("{key}: {value}"), palette)),
        )
        .when_some(item.booking_code().map(str::to_owned), |card, code| {
            card.child(
                secondary_button(format!("Copy booking code · {code}")).on_click(
                    move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(code.clone())),
                ),
            )
        })
        .when_some(item.source_url(), |card, url| {
            card.child(
                secondary_button("Open source").on_click(move |_, _, cx| cx.open_url(url.as_str())),
            )
        })
}

fn journey(journey: &connections::Journey, palette: Palette) -> Div {
    surface_card(palette)
        .p_3()
        .flex()
        .flex_col()
        .gap_1p5()
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_2()
                .child(heading(
                    format!("{} – {}", journey.departs(), journey.arrives()),
                    palette,
                ))
                .child(Badge::new(connections::price(journey.total_price)).tone(Tone::Accent)),
        )
        .child(note(
            format!(
                "{} · {} changes",
                connections::duration(journey.total_duration_minutes),
                journey.changes()
            ),
            palette,
        ))
        .children(journey.legs.iter().map(|leg| {
            note(
                format!(
                    "{} {} → {} {} · {}",
                    connections::clock(&leg.departure_time),
                    leg.origin.name,
                    connections::clock(&leg.arrival_time),
                    leg.destination.name,
                    leg.train_name
                ),
                palette,
            )
        }))
        .children(journey.ranking.iter().flat_map(|rank| {
            rank.factors
                .iter()
                .map(|factor| note(factor.label.clone(), palette))
        }))
        .children(
            connections::late(journey.delay_risk_score)
                .map(|risk| Badge::new(risk).tone(Tone::Warning)),
        )
}

fn calendar(entry: &sjel_gpui_core::shell::CalendarEntry, palette: Palette) -> Div {
    let day = entry
        .starts_at
        .get(..10)
        .and_then(|day| chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d").ok());
    sub_card(palette)
        .child(
            secondary_button(format!("Calendar · {}", entry.title))
                .disabled(day.is_none())
                .on_click(move |_, _, _| {
                    if let Some(day) = day {
                        phone_calendar::open_day(day);
                        phone_navigation::open(Page::Calendar);
                    }
                }),
        )
        .child(note(
            if entry.all_day {
                "All day".into()
            } else {
                format!("{}–{}", entry.starts_at, entry.ends_at)
            },
            palette,
        ))
        .children(entry.location.clone().map(|l| note(l, palette)))
        .children(entry.notes.clone().map(|n| note(n, palette)))
}

fn column() -> Div {
    div().flex().flex_col().gap_1()
}
fn heading(text: impl Into<SharedString>, palette: Palette) -> Div {
    div()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(palette.text))
        .child(text.into())
}
fn note(text: impl Into<SharedString>, palette: Palette) -> Div {
    div()
        .text_sm()
        .text_color(rgb(palette.muted))
        .child(text.into())
}
fn secondary_button(label: impl Into<SharedString>) -> Button {
    let label = label.into();
    Button::new(label.clone(), label)
        .size(ControlSize::Sm)
        .variant(ButtonVariant::Secondary)
}
fn choice(label: impl Into<SharedString>, selected: bool) -> Button {
    let label = label.into();
    Button::new(label.clone(), label)
        .size(ControlSize::Sm)
        .variant(if selected {
            ButtonVariant::Primary
        } else {
            ButtonVariant::Secondary
        })
}
