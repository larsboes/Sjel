//! Find a train connection: two stations, a date and a time, then transit's direct journeys and
//! its split-ticket calculation for the same route (`core::travel::connections`).
use std::time::Duration;

use gpui::{
    div, prelude::*, px, rgb, rgba, AnyElement, Context, Entity, SharedString, Subscription, Window,
};
use gpui_kit::component::button::ButtonVariants;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Disableable, Sizable};
use sjel_gpui_core::shell;
use sjel_gpui_core::travel::connections::{
    self, Journey, Query, Rank, Sort, SplitResult, Station, PRIORITIES,
};

use super::Travel;

/// How long typing pauses before a station name is looked up.
const SUGGEST_DEBOUNCE: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tab {
    Direct,
    Split,
}

/// A station field: the text typed, the station picked for it, and what transit suggests.
struct StationField {
    input: Entity<InputState>,
    picked: Option<Station>,
    suggestions: Vec<Station>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum End {
    From,
    To,
}

pub(super) struct Connections {
    from: StationField,
    to: StationField,
    date: Entity<InputState>,
    time: Entity<InputState>,
    phrase: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
    priority: Option<&'static str>,
    sort: Sort,
    tab: Tab,
    searching: bool,
    error: Option<String>,
    journeys: Option<Vec<Journey>>,
    /// The split, or why there is none: transit answers 404 when no cheaper chain exists.
    split: Option<Result<SplitResult, String>>,
    expanded: Option<String>,
    /// Bumped per search, so an answer to an older one is dropped.
    generation: u64,
}

impl Travel {
    /// Builds the fields the first time the page draws: an input needs the window.
    pub(super) fn ensure_connections(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.connections.is_some() {
            return;
        }
        let input = |placeholder: &'static str,
                     value: String,
                     window: &mut Window,
                     cx: &mut Context<Self>| {
            cx.new(|cx| {
                let mut state = InputState::new(window, cx).placeholder(placeholder);
                state.set_value(value, window, cx);
                state
            })
        };
        let from = input("From station", String::new(), window, cx);
        let to = input("To station", String::new(), window, cx);
        let date = input("2026-10-25", connections::tomorrow(), window, cx);
        let time = input("10:00", "10:00".into(), window, cx);
        let phrase = input(
            "or say it: cheapest, direct, schnell und zuverlässig",
            String::new(),
            window,
            cx,
        );
        let station_events = |end: End| {
            move |view: &mut Travel,
                  input: &Entity<InputState>,
                  event: &InputEvent,
                  window: &mut Window,
                  cx: &mut Context<Travel>| match event {
                InputEvent::Change => {
                    let text = input.read(cx).value().trim().to_owned();
                    view.station_typed(end, text, cx);
                }
                InputEvent::PressEnter { .. } => view.station_enter(end, window, cx),
                _ => {}
            }
        };
        let search_on_enter = |view: &mut Travel,
                               _: &Entity<InputState>,
                               event: &InputEvent,
                               window: &mut Window,
                               cx: &mut Context<Travel>| {
            if let InputEvent::PressEnter { .. } = event {
                view.search_connections(window, cx);
            }
        };
        let subscriptions = vec![
            cx.subscribe_in(&from, window, station_events(End::From)),
            cx.subscribe_in(&to, window, station_events(End::To)),
            cx.subscribe_in(&date, window, search_on_enter),
            cx.subscribe_in(&time, window, search_on_enter),
            cx.subscribe_in(&phrase, window, search_on_enter),
        ];
        from.update(cx, |input, cx| input.focus(window, cx));
        let field = |input| StationField {
            input,
            picked: None,
            suggestions: Vec::new(),
        };
        self.connections = Some(Connections {
            from: field(from),
            to: field(to),
            date,
            time,
            phrase,
            _subscriptions: subscriptions,
            priority: None,
            sort: Sort::Ranked,
            tab: Tab::Direct,
            searching: false,
            error: None,
            journeys: None,
            split: None,
            expanded: None,
            generation: 0,
        });
        // transit is on demand and nothing else on the Mac starts it. A failed start is not
        // reported here: the lookup that follows says what is wrong.
        if let Some(shell) = self.writer() {
            shell::spawn(async move {
                let _ = shell.set_running("transit", true).await;
            });
        }
    }

    fn field(&mut self, end: End) -> Option<&mut StationField> {
        let page = self.connections.as_mut()?;
        Some(match end {
            End::From => &mut page.from,
            End::To => &mut page.to,
        })
    }

    /// A typed name forgets the picked station and asks transit once typing pauses.
    fn station_typed(&mut self, end: End, text: String, cx: &mut Context<Self>) {
        let Some(field) = self.field(end) else { return };
        if field.picked.as_ref().is_some_and(|s| s.name == text) {
            return;
        }
        field.picked = None;
        field.suggestions.clear();
        cx.notify();
        let Some(source) = self.source.clone() else {
            return;
        };
        if text.chars().count() < 2 {
            return;
        }
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SUGGEST_DEBOUNCE).await;
            let current = this
                .update(cx, |view, cx| {
                    view.field(end)
                        .is_some_and(|f| f.input.read(cx).value().trim() == text)
                })
                .unwrap_or(false);
            if !current {
                return;
            }
            let asked = text.clone();
            let found = shell::run(async move { source.stations(&asked).await }).await;
            let _ = this.update(cx, |view, cx| {
                let Some(field) = view.field(end) else { return };
                if field.input.read(cx).value().trim() != text {
                    return;
                }
                match found {
                    Some(Ok(stations)) => {
                        field.suggestions = stations.into_iter().take(6).collect()
                    }
                    Some(Err(error)) => {
                        if let Some(page) = view.connections.as_mut() {
                            page.error = Some(error);
                        }
                    }
                    None => {}
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn pick_station(
        &mut self,
        end: End,
        station: Station,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(field) = self.field(end) else { return };
        let input = field.input.clone();
        field.suggestions.clear();
        field.picked = Some(station.clone());
        input.update(cx, |input, cx| input.set_value(station.name, window, cx));
        cx.notify();
    }

    /// Enter takes the first suggestion and moves on to the next field; with both stations
    /// picked, it searches.
    fn station_enter(&mut self, end: End, window: &mut Window, cx: &mut Context<Self>) {
        let Some(field) = self.field(end) else { return };
        let first = field
            .picked
            .is_none()
            .then(|| field.suggestions.first().cloned())
            .flatten();
        let Some(station) = first else {
            self.search_connections(window, cx);
            return;
        };
        self.pick_station(end, station, window, cx);
        let Some(page) = self.connections.as_ref() else {
            return;
        };
        let next = match end {
            End::From if page.to.picked.is_none() => Some(page.to.input.clone()),
            _ => None,
        };
        match next {
            Some(input) => input.update(cx, |input, cx| input.focus(window, cx)),
            None => self.search_connections(window, cx),
        }
    }

    fn swap_stations(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(page) = self.connections.as_mut() else {
            return;
        };
        std::mem::swap(&mut page.from.picked, &mut page.to.picked);
        page.from.suggestions.clear();
        page.to.suggestions.clear();
        let (from, to) = (page.from.input.clone(), page.to.input.clone());
        let (a, b) = (
            from.read(cx).value().to_string(),
            to.read(cx).value().to_string(),
        );
        from.update(cx, |input, cx| input.set_value(b, window, cx));
        to.update(cx, |input, cx| input.set_value(a, window, cx));
        cx.notify();
    }

    fn set_priority(
        &mut self,
        priority: Option<&'static str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(page) = self.connections.as_mut() else {
            return;
        };
        page.priority = priority;
        let phrase = page.phrase.clone();
        phrase.update(cx, |input, cx| input.set_value("", window, cx));
        self.search_connections(window, cx);
    }

    /// Runs the direct search and the split together. The split is a second opinion on the same
    /// query, so its failure never throws away the direct journeys.
    fn search_connections(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let source = self.source.clone();
        let Some(page) = self.connections.as_mut() else {
            return;
        };
        let (Some(from), Some(to)) = (page.from.picked.clone(), page.to.picked.clone()) else {
            page.error = Some("Pick an origin and a destination from the suggestions.".into());
            cx.notify();
            return;
        };
        let Some(source) = source else {
            page.error = Some("Connect to your Mac to search connections.".into());
            cx.notify();
            return;
        };
        let query = Query {
            from,
            to,
            date: page.date.read(cx).value().to_string(),
            time: page.time.read(cx).value().to_string(),
            rank: Rank {
                priority: page.priority,
                phrase: page.phrase.read(cx).value().to_string(),
            },
        };
        if let Err(error) = query.when() {
            page.error = Some(error);
            cx.notify();
            return;
        }
        page.generation += 1;
        let generation = page.generation;
        page.searching = true;
        page.error = None;
        cx.notify();
        let work =
            shell::run(
                async move { futures::join!(source.connections(&query), source.split(&query)) },
            );
        cx.spawn(async move |this, cx| {
            let answer = work.await;
            let _ = this.update(cx, |view, cx| {
                let Some(page) = view.connections.as_mut() else {
                    return;
                };
                if page.generation != generation {
                    return;
                }
                page.searching = false;
                match answer {
                    Some((Ok(journeys), split)) => {
                        page.journeys = Some(journeys);
                        page.split = Some(split);
                        page.expanded = None;
                    }
                    Some((Err(error), _)) => page.error = Some(error),
                    None => page.error = Some("The search stopped.".into()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn connections_page(&self, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        let Some(page) = &self.connections else {
            return div().into_any_element();
        };
        let suggestions = |end: End, field: &StationField, cx: &mut Context<Self>| {
            div()
                .flex()
                .flex_col()
                .children(field.suggestions.iter().enumerate().map(|(i, station)| {
                    let station = station.clone();
                    let name = station.name.clone();
                    self.button(SharedString::from(format!("station-{}-{i}", end as u8)))
                        .ghost()
                        .small()
                        .label(name)
                        .on_click(cx.listener(move |view, _, window, cx| {
                            view.pick_station(end, station.clone(), window, cx)
                        }))
                }))
        };
        let station = |end: End, field: &StationField, cx: &mut Context<Self>| {
            div()
                .flex_1()
                .min_w(px(200.))
                .flex()
                .flex_col()
                .gap_1()
                .child(Input::new(&field.input).small())
                .child(suggestions(end, field, cx))
        };
        let form = div()
            .flex()
            .flex_wrap()
            .items_start()
            .gap_2()
            .child(station(End::From, &page.from, cx))
            .child(
                self.button("swap-stations")
                    .ghost()
                    .small()
                    .label("⇄")
                    .on_click(cx.listener(|view, _, window, cx| view.swap_stations(window, cx))),
            )
            .child(station(End::To, &page.to, cx))
            .child(div().w(px(110.)).child(Input::new(&page.date).small()))
            .child(div().w(px(70.)).child(Input::new(&page.time).small()))
            .child(
                self.button("search-connections")
                    .small()
                    .primary()
                    .label(if page.searching {
                        "Searching…"
                    } else {
                        "Search"
                    })
                    .disabled(page.searching)
                    .on_click(
                        cx.listener(|view, _, window, cx| view.search_connections(window, cx)),
                    ),
            );
        let mut body = div()
            .flex()
            .flex_col()
            .gap_3()
            .child(div().text_xl().child("Find a connection"))
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child("Direct connections and a split-ticket calculation for the same route."),
            )
            .child(form)
            .when_some(page.error.clone(), |el, error| {
                el.child(div().text_sm().text_color(rgb(palette.red)).child(error))
            });
        let Some(journeys) = &page.journeys else {
            return body.into_any_element();
        };
        let tab = |label: String, tab: Tab, cx: &mut Context<Self>| {
            let active = page.tab == tab;
            self.button(SharedString::from(format!("connections-tab-{}", tab as u8)))
                .small()
                .label(label)
                .when(active, |b| b.primary())
                .when(!active, |b| b.ghost())
                .on_click(cx.listener(move |view, _, _, cx| {
                    if let Some(page) = view.connections.as_mut() {
                        page.tab = tab;
                        cx.notify();
                    }
                }))
        };
        body = body.child(
            div()
                .flex()
                .gap_2()
                .child(tab(format!("Direct ({})", journeys.len()), Tab::Direct, cx))
                .child(tab("Split-ticket".into(), Tab::Split, cx)),
        );
        let content = match page.tab {
            Tab::Direct => self.direct(page, journeys, cx),
            Tab::Split => self.split(page),
        };
        body.child(content).into_any_element()
    }

    fn direct(
        &self,
        page: &Connections,
        journeys: &[Journey],
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.palette;
        let sorts = div()
            .flex()
            .flex_wrap()
            .gap_1()
            .children(Sort::ALL.map(|sort| {
                let active = page.sort == sort;
                self.button(SharedString::from(format!("journey-sort-{}", sort.label())))
                    .xsmall()
                    .label(sort.label())
                    .when(active, |b| b.primary())
                    .when(!active, |b| b.ghost())
                    .on_click(cx.listener(move |view, _, _, cx| {
                        if let Some(page) = view.connections.as_mut() {
                            page.sort = sort;
                            cx.notify();
                        }
                    }))
            }));
        let usual = page.priority.is_none();
        let mut presets = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_1()
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child("Rank by"),
            )
            .child(
                self.button("rank-usual")
                    .xsmall()
                    .label("My usual")
                    .when(usual, |b| b.primary())
                    .when(!usual, |b| b.ghost())
                    .on_click(
                        cx.listener(|view, _, window, cx| view.set_priority(None, window, cx)),
                    ),
            );
        for (id, label, _) in PRIORITIES {
            let active = page.priority == Some(id);
            presets = presets.child(
                self.button(SharedString::from(format!("rank-{id}")))
                    .xsmall()
                    .label(label)
                    .when(active, |b| b.primary())
                    .when(!active, |b| b.ghost())
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.set_priority(Some(id), window, cx)
                    })),
            );
        }
        presets = presets.child(div().w(px(320.)).child(Input::new(&page.phrase).small()));
        let mut list = div().flex().flex_col().gap_1();
        if journeys.is_empty() {
            list = list.child(
                div()
                    .text_sm()
                    .text_color(rgb(palette.muted))
                    .child("No connections at this time."),
            );
        }
        for (i, journey) in connections::ordered(journeys, page.sort)
            .into_iter()
            .enumerate()
        {
            list = list.child(self.journey_row(
                i,
                journey,
                page.expanded.as_deref() == Some(journey.id.as_str()),
                cx,
            ));
        }
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(sorts)
            .child(presets)
            .child(list)
            .into_any_element()
    }

    fn journey_row(
        &self,
        i: usize,
        journey: &Journey,
        expanded: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.palette;
        let muted = |text: String| div().text_xs().text_color(rgb(palette.muted)).child(text);
        let trains = journey
            .legs
            .iter()
            .map(|leg| leg.train_name.as_str())
            .collect::<Vec<_>>()
            .join(" · ");
        let changes = match journey.changes() {
            0 => "direct".to_owned(),
            1 => "1 change".to_owned(),
            n => format!("{n} changes"),
        };
        let cancelled = journey.legs.iter().any(|leg| leg.cancelled);
        let id = journey.id.clone();
        let mut row = div()
            .id(("journey", i))
            .flex()
            .flex_col()
            .gap_1()
            .px_3()
            .py_2()
            .rounded(px(6.))
            .when(expanded, |el| el.bg(rgba(palette.raised)))
            .hover(move |style| style.bg(rgba(palette.raised)))
            .cursor_pointer()
            .on_click(cx.listener(move |view, _, _, cx| {
                if let Some(page) = view.connections.as_mut() {
                    page.expanded =
                        (page.expanded.as_deref() != Some(id.as_str())).then(|| id.clone());
                    cx.notify();
                }
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(format!("{} → {}", journey.departs(), journey.arrives())),
                    )
                    .children(journey.delay_minutes().map(|m| {
                        div()
                            .text_xs()
                            .text_color(rgb(palette.red))
                            .child(format!("{m:+} min"))
                    }))
                    .child(muted(connections::duration(journey.total_duration_minutes)))
                    .child(muted(changes))
                    .when(journey.changes() == 0, |el| {
                        el.child(
                            div()
                                .px_1p5()
                                .py(px(1.))
                                .rounded(px(4.))
                                .bg(rgba(crate::theme::tint(palette.green, 0x20)))
                                .text_xs()
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(rgb(palette.green))
                                .child("Direct"),
                        )
                    })
                    .when(cancelled, |el| {
                        el.child(
                            div()
                                .text_xs()
                                .text_color(rgb(palette.red))
                                .child("cancelled"),
                        )
                    })
                    .child(div().flex_1())
                    .children(connections::late(journey.delay_risk_score).map(muted))
                    .children(
                        journey
                            .reliability
                            .as_ref()
                            .map(|r| muted(format!("{:.0}% holds", r.probability * 100.0))),
                    )
                    .child(
                        div()
                            .w(px(80.))
                            .flex()
                            .justify_end()
                            .text_sm()
                            .child(connections::price(journey.total_price)),
                    ),
            )
            .child(muted(trains));
        if expanded {
            for (idx, leg) in journey.legs.iter().enumerate() {
                if idx > 0 {
                    let prev_arr = &journey.legs[idx - 1].arrival_time;
                    let next_dep = &leg.departure_time;
                    let buffer_text = match (
                        chrono::DateTime::parse_from_rfc3339(prev_arr),
                        chrono::DateTime::parse_from_rfc3339(next_dep),
                    ) {
                        (Ok(a), Ok(d)) => {
                            let mins = (d - a).num_minutes();
                            format!("Transfer buffer: {mins} min")
                        }
                        _ => "Transfer".into(),
                    };
                    row = row.child(
                        div()
                            .pl_6()
                            .py(px(1.))
                            .text_xs()
                            .text_color(rgb(palette.accent))
                            .child(format!("↳ {buffer_text}")),
                    );
                }
                let mut parts = vec![
                    format!(
                        "{} {} → {} {}",
                        connections::clock(&leg.departure_time),
                        leg.origin.name,
                        connections::clock(&leg.arrival_time),
                        leg.destination.name
                    ),
                    leg.train_name.clone(),
                ];
                if let Some(platform) = leg.platform.as_deref().filter(|p| !p.is_empty()) {
                    parts.push(format!("platform {platform}"));
                }
                if leg.is_regional {
                    parts.push("Deutschland-Ticket".into());
                }
                if leg.cancelled {
                    parts.push("cancelled".into());
                }
                row = row.child(div().pl_3().text_xs().child(parts.join(" · ")));
            }
            if let Some(ranking) = &journey.ranking {
                row = row.child(muted(format!(
                    "Rank {} by {} weights",
                    ranking.rank,
                    if ranking.source == "request" {
                        "this search's"
                    } else {
                        "your profile's"
                    }
                )));
                for factor in &ranking.factors {
                    row = row.child(muted(format!(
                        "{} ({:.0}%): {}",
                        factor.label,
                        factor.weight * 100.0,
                        factor.rationale
                    )));
                }
            }
        }
        row.into_any_element()
    }

    fn split(&self, page: &Connections) -> AnyElement {
        let palette = self.palette;
        let muted = |text: String| div().text_xs().text_color(rgb(palette.muted)).child(text);
        let split = match &page.split {
            Some(Ok(split)) => split,
            Some(Err(why)) => return muted(why.clone()).into_any_element(),
            None => return muted("No split calculation for this route.".into()).into_any_element(),
        };
        let verdict = match (split.savings, split.saved_percent()) {
            (None, _) => "No direct fare to compare".to_owned(),
            (Some(s), Some(pct)) if s > 0.01 => format!("{pct}% cheaper"),
            _ => "Direct is cheapest".to_owned(),
        };
        let mut panel = div()
            .flex()
            .flex_col()
            .gap_2()
            .child(muted(
                "The same journey split into separately purchased sections.".into(),
            ))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_4()
                    .child(div().text_sm().child(format!(
                        "Direct {}",
                        connections::price(split.original_price)
                    )))
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(format!(
                                "Split {}",
                                connections::price(Some(split.split_price))
                            )),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(palette.green))
                            .child(verdict),
                    ),
            );
        if let Some(note) = split.confidence.note() {
            let mut note = note.to_owned();
            if split.unpriced_pairs > 0 {
                note.push_str(&format!(
                    " ({} of {} fare lookups returned nothing.)",
                    split.unpriced_pairs, split.queried_pairs
                ));
            }
            let tone = if split.confidence == connections::Confidence::Low {
                palette.red
            } else {
                palette.amber
            };
            panel = panel.child(div().text_sm().text_color(rgb(tone)).child(note));
        }
        for (i, segment) in split.segments.iter().enumerate() {
            let journey = &segment.journey;
            let mut card = div()
                .flex()
                .flex_col()
                .gap_1()
                .p_3()
                .rounded(px(6.))
                .bg(rgba(palette.raised))
                .child(
                    div()
                        .flex()
                        .justify_between()
                        .child(muted(format!("Ticket {}", i + 1)))
                        .child(
                            div()
                                .text_sm()
                                .child(connections::price(journey.total_price)),
                        ),
                )
                .child(div().text_sm().child(format!(
                    "{} → {}",
                    journey.start_station.name, journey.end_station.name
                )))
                .child(muted(format!(
                    "departs {} · {}",
                    journey.departs(),
                    connections::duration(journey.total_duration_minutes)
                )));
            if let Some(label) = segment.train_match.label() {
                let why = if segment.expected_trains.is_empty() {
                    "the direct journey carried no train number to compare against".to_owned()
                } else {
                    format!("this journey uses {}", segment.expected_trains.join(", "))
                };
                let tone = if segment.train_match == connections::TrainMatch::Different {
                    palette.red
                } else {
                    palette.amber
                };
                card = card.child(
                    div()
                        .text_xs()
                        .text_color(rgb(tone))
                        .child(format!("{label}: {why}")),
                );
            }
            panel = panel.child(card);
        }
        panel = if split.confidence == connections::Confidence::Low {
            panel.child(muted(
                "No booking link while a ticket is priced for a different train. Search the route \
                 again, or buy the direct connection."
                    .into(),
            ))
        } else {
            panel.child(
                div().child(
                    self.button("book-split")
                        .small()
                        .label("Book on bahn.de ↗")
                        .on_click(|_, _, cx| cx.open_url("https://www.bahn.de")),
                ),
            )
        };
        panel.into_any_element()
    }
}
