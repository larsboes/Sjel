//! Trips and station search over the phone travel controller.

use gpui::{
    div, prelude::*, ClipboardItem, Context, Div, Entity, Focusable, FontWeight, SharedString,
    Subscription, Task, Window,
};

use super::{
    connections, download, feed, load, nsh_map, open, resolve, search, suggest, travel, Plan,
    PlanExt, Query, Rank, Snapshot, Sort, Station, TravelRoute, TripRead, TripSection,
};
use crate::{
    ely::{
        buttons::{Button, ButtonVariant},
        forms::{Input, InputEvent, TextInput},
        theme::ControlSize,
    },
    phone_calendar,
    phone_navigation::{self, Page},
};

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

    fn connections(&self, cx: &Context<Self>) -> Div {
        let mut screen = column().child(heading("Connections"));
        for index in 0..2 {
            screen = screen.child(Input::new(&self.fields[index]));
            if self.stations[index].is_none() {
                screen = screen.children(self.snapshot.suggestions[index].iter().cloned().map(
                    |station| {
                        button(station.name.clone()).on_click(cx.listener(
                            move |view, _, window, cx| {
                                view.stations[index] = Some(station.clone());
                                view.fields[index].update(cx, |field, cx| {
                                    field.set_text(station.name.clone(), cx)
                                });
                                window.blur(cx);
                                cx.notify();
                            },
                        ))
                    },
                ));
            }
        }
        screen
            .children(self.fields[2..].iter().map(Input::new))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .children(connections::PRIORITIES.iter().map(|&(key, label, _)| {
                        choice(label, self.priority == Some(key)).on_click(cx.listener(
                            move |view, _, _, cx| {
                                view.priority = Some(key);
                                cx.notify();
                            },
                        ))
                    })),
            )
            .child(
                button("Search")
                    .disabled(self.stations.iter().any(Option::is_none) || self.snapshot.searching)
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
            )
            .when(self.snapshot.searching, |list| list.child("Searching…"))
            .children(self.snapshot.search_error.clone())
            .child(div().flex().flex_wrap().children(Sort::ALL.map(|sort| {
                choice(sort.label(), self.sort == sort).on_click(cx.listener(
                    move |view, _, _, cx| {
                        view.sort = sort;
                        cx.notify();
                    },
                ))
            })))
            .child(
                div()
                    .flex()
                    .children(
                        [(false, "Direct"), (true, "Split tickets")].map(|(split, name)| {
                            choice(name, self.split == split).on_click(cx.listener(
                                move |view, _, _, cx| {
                                    view.split = split;
                                    cx.notify();
                                },
                            ))
                        }),
                    ),
            )
            .when(self.split, |list| {
                list.when_some(self.snapshot.split.as_ref(), |list, result| match result {
                    Err(error) => list.child(error.clone()),
                    Ok(result) => list
                        .child(heading(format!(
                            "Split fare · {:.2} EUR",
                            result.split_price
                        )))
                        .children(result.confidence.note())
                        .children(result.segments.iter().map(|part| journey(&part.journey))),
                })
            })
            .when(!self.split, |list| {
                list.children(
                    connections::ordered(&self.snapshot.journeys, self.sort)
                        .into_iter()
                        .map(journey),
                )
            })
    }

    fn trip(
        &self,
        plan: &Plan,
        read: Option<&TripRead>,
        section: TripSection,
        cx: &Context<Self>,
    ) -> Div {
        let id = plan.id.clone();
        let download_id = id.clone();
        let locate_id = id.clone();
        let readiness = plan.check_offline_readiness();
        let offline = self.snapshot.offline.get(&id).cloned();
        let mut screen = column()
            .child(button("‹ All trips").on_click(|_, _, _| open(TravelRoute::Trips)))
            .child(heading(plan.title.clone()))
            .child(note(format!(
                "{}–{} · {}",
                plan.date_start, plan.date_end, plan.status
            )))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .children(TripSection::ALL.map(|tab| {
                        let id = id.clone();
                        choice(tab.label(), section == tab).on_click(move |_, _, _| {
                            open(TravelRoute::Trip {
                                id: id.clone(),
                                section: tab,
                            })
                        })
                    })),
            )
            .child(note(if readiness.is_ready() {
                "Offline ready".into()
            } else {
                format!("Offline: {}%", readiness.percent())
            }))
            .children(offline.clone())
            .when(offline.is_none() && !readiness.is_ready(), |view| {
                view.child(
                    button("Download offline pack")
                        .on_click(move |_, _, _| download(download_id.clone())),
                )
            })
            .child(button("Locate stops").on_click(move |_, _, _| resolve(locate_id.clone())));
        let Some(read) = read else {
            return screen.child(note("Reading trip details…"));
        };
        screen = screen.children(read.error.clone());
        let Some(saved) = &read.saved else {
            return screen;
        };
        let items = &saved.details.items;
        match section {
            TripSection::Overview => screen
                .child(heading(format!(
                    "{} → {}",
                    plan.origin.name,
                    plan.destinations
                        .iter()
                        .map(|place| place.name.as_str())
                        .collect::<Vec<_>>()
                        .join(" · ")
                )))
                .when(!plan.travelers.is_empty(), |view| {
                    view.child(note(format!("Travelers: {}", plan.travelers.join(", "))))
                })
                .child(note(plan.interests.clone()))
                .children(read.cost.iter().flat_map(|cost| {
                    cost.by_currency.iter().map(|money| {
                        note(format!(
                            "Booked: {:.2} {}",
                            money.booked_cents as f64 / 100.,
                            money.currency
                        ))
                    })
                }))
                .children(read.cost.iter().filter_map(|cost| {
                    match (cost.actuals.personal_cents, &cost.actuals.currency) {
                        (Some(cents), Some(currency)) => Some(note(format!(
                            "Actual personal cost: {:.2} {currency}",
                            cents as f64 / 100.
                        ))),
                        _ => cost.actuals.reason.clone().map(note),
                    }
                }))
                .children(read.calendar.iter().map(calendar))
                .children(read.links.iter().map(|link| note(link.title.clone()))),
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
                                column()
                                    .child(heading(day.clone()))
                                    .children(stages.iter().enumerate().map(|(index, stage)| {
                                        let id = id.clone();
                                        column()
                                            .child(heading(format!(
                                                "{} → {}",
                                                stage.origin.name, stage.destination.name
                                            )))
                                            .child(note(format!(
                                                "{} · {}",
                                                stage
                                                    .transport_modes
                                                    .iter()
                                                    .map(|mode| mode.as_str())
                                                    .collect::<Vec<_>>()
                                                    .join(" / "),
                                                stage.status.as_str()
                                            )))
                                            .children(stage.branch_note.clone())
                                            .when_some(stages.get(index + 1), |card, next| {
                                                let from = &stage.destination.name;
                                                let to = &next.origin.name;
                                                card.child(note(if from == to {
                                                    format!("Transfer at {from} · same station")
                                                } else {
                                                    format!(
                                                        "Transfer buffer required · {from} → {to}"
                                                    )
                                                }))
                                            })
                                            .when_some(
                                                stage.destination.coordinate(),
                                                |card, at| {
                                                    card.child(
                                                        button("Show destination on map").on_click(
                                                            move |_, _, _| {
                                                                open(TravelRoute::Trip {
                                                                    id: id.clone(),
                                                                    section: TripSection::Map,
                                                                });
                                                                nsh_map::show(Some(at));
                                                            },
                                                        ),
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
                                            .map(calendar),
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
                                                item(entry).when_some(at, |card, at| {
                                                    card.child(
                                                        button("Show booking on map").on_click(
                                                            move |_, _, _| {
                                                                open(TravelRoute::Trip {
                                                                    id: id.clone(),
                                                                    section: TripSection::Map,
                                                                });
                                                                nsh_map::show(Some(at));
                                                            },
                                                        ),
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
                                            .map(|link| note(link.title.clone())),
                                    )
                            }),
                    )
                    .children(items.iter().filter(|entry| entry.day.is_none()).map(item))
            }
            _ => screen.children(items.iter().map(item)),
        }
    }

    fn trips(&self, cx: &Context<Self>) -> Div {
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        column()
            .child(heading("Trips"))
            .child(note(self.snapshot.status.clone()))
            .child(
                button(if self.past {
                    "Upcoming trips"
                } else {
                    "Include past trips"
                })
                .on_click(cx.listener(|view, _, _, cx| {
                    view.past = !view.past;
                    cx.notify();
                })),
            )
            .when(
                !self.snapshot.loading && self.snapshot.plans.is_empty(),
                |view| view.child("No saved trips. Refresh while connected to the Mac."),
            )
            .children(
                self.snapshot
                    .plans
                    .iter()
                    .filter(|plan| self.past || travel::upcoming(plan, &today))
                    .map(|plan| {
                        let id = plan.id.clone();
                        column()
                            .py_2()
                            .child(button(plan.title.clone()).on_click(move |_, _, _| {
                                open(TravelRoute::Trip {
                                    id: id.clone(),
                                    section: TripSection::Overview,
                                })
                            }))
                            .child(note(format!(
                                "{}–{} · {}",
                                plan.date_start, plan.date_end, plan.status
                            )))
                    }),
            )
    }
}

impl Render for PhoneTravel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match &self.snapshot.route {
            TravelRoute::Connections => self.connections(cx),
            TravelRoute::Trip { id, section } => {
                let read = self.snapshot.details.get(id);
                let plan = read
                    .and_then(|read| read.saved.as_ref())
                    .map(|saved| &saved.details.plan)
                    .or_else(|| self.snapshot.plans.iter().find(|plan| &plan.id == id));
                plan.map_or_else(
                    || column().child("This trip has not been saved on the phone."),
                    |plan| self.trip(plan, read, *section, cx),
                )
            }
            _ => self.trips(cx),
        };
        column()
            .size_full()
            .p_3()
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .children(
                        [
                            ("Trips", TravelRoute::Trips),
                            ("Map & Places", TravelRoute::Map),
                            ("Connections", TravelRoute::Connections),
                        ]
                        .map(|(name, route)| {
                            choice(name, self.snapshot.route == route)
                                .on_click(move |_, _, _| open(route.clone()))
                        }),
                    )
                    .child(button("Refresh").on_click(|_, _, _| load())),
            )
            .when(self.snapshot.loading, |view| {
                view.child(note("Reading trips…"))
            })
            .children(self.snapshot.error.clone())
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

fn item(item: &travel::Item) -> Div {
    column()
        .py_2()
        .child(heading(item.title.clone()))
        .child(note(format!(
            "{} · {}",
            if item.item_type == "packing" {
                if item.is_packed() {
                    "Packed"
                } else {
                    "Still to pack"
                }
            } else {
                &item.item_type
            },
            item.day.as_deref().unwrap_or("Undated")
        )))
        .children(
            item.fields()
                .into_iter()
                .map(|(key, value)| note(format!("{key}: {value}"))),
        )
        .when_some(item.booking_code().map(str::to_owned), |card, code| {
            card.child(
                button(format!("Copy booking code · {code}")).on_click(move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(code.clone()))
                }),
            )
        })
        .when_some(item.source_url(), |card, url| {
            card.child(button("Open source").on_click(move |_, _, cx| cx.open_url(url.as_str())))
        })
}

fn journey(journey: &connections::Journey) -> Div {
    column()
        .py_2()
        .child(heading(format!(
            "{}–{} · {}",
            journey.departs(),
            journey.arrives(),
            connections::price(journey.total_price)
        )))
        .child(note(format!(
            "{} · {} changes",
            connections::duration(journey.total_duration_minutes),
            journey.changes()
        )))
        .children(journey.legs.iter().map(|leg| {
            note(format!(
                "{} {} → {} {} · {}",
                connections::clock(&leg.departure_time),
                leg.origin.name,
                connections::clock(&leg.arrival_time),
                leg.destination.name,
                leg.train_name
            ))
        }))
        .children(
            journey
                .ranking
                .iter()
                .flat_map(|rank| rank.factors.iter().map(|factor| note(factor.label.clone()))),
        )
        .children(connections::late(journey.delay_risk_score))
}

fn calendar(entry: &sjel_gpui_core::shell::CalendarEntry) -> Div {
    let day = entry
        .starts_at
        .get(..10)
        .and_then(|day| chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d").ok());
    column()
        .child(
            button(format!("Calendar · {}", entry.title))
                .disabled(day.is_none())
                .on_click(move |_, _, _| {
                    if let Some(day) = day {
                        phone_calendar::open_day(day);
                        phone_navigation::open(Page::Calendar);
                    }
                }),
        )
        .child(note(if entry.all_day {
            "All day".into()
        } else {
            format!("{}–{}", entry.starts_at, entry.ends_at)
        }))
        .children(entry.location.clone())
        .children(entry.notes.clone())
}

fn column() -> Div {
    div().flex().flex_col().gap_1()
}
fn heading(text: impl Into<SharedString>) -> Div {
    div().font_weight(FontWeight::SEMIBOLD).child(text.into())
}
fn note(text: impl Into<SharedString>) -> Div {
    div().text_sm().child(text.into())
}
fn button(label: impl Into<SharedString>) -> Button {
    choice(label, false)
}
fn choice(label: impl Into<SharedString>, selected: bool) -> Button {
    let label = label.into();
    Button::new(label.clone(), label)
        .size(ControlSize::Sm)
        .variant(if selected {
            ButtonVariant::Primary
        } else {
            ButtonVariant::Ghost
        })
}
