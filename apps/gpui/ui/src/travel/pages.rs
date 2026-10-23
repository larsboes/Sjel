//! Travel's trip library and the four subpages of an opened trip.
use super::Travel;
use gpui::{div, prelude::*, rgb, rgba, AnyElement, ClipboardItem, Context};
use gpui_kit::component::button::ButtonVariants;
use gpui_kit::component::Disableable;
use sjel_gpui_core::travel::{Item, Place, TravelRoute, TripSection};

impl Travel {
    pub(super) fn render_page(&self, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        let mut page = div()
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_3()
            .text_color(rgb(palette.text));
        if !matches!(self.route, TravelRoute::Trips) {
            page = page.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        self.button("back-to-trips")
                            .ghost()
                            .label("← Trips")
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.open_route(TravelRoute::Trips, cx)
                            })),
                    )
                    .child(
                        self.button("copy-trip-link")
                            .ghost()
                            .label("Copy link")
                            .on_click(cx.listener(|view, _, _, cx| {
                                let path = view.route.href();
                                cx.write_to_clipboard(ClipboardItem::new_string(format!(
                                    "sjel://{}",
                                    path.trim_start_matches('/')
                                )));
                            })),
                    ),
            );
        }
        page = page
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(if self.loading {
                                "Loading trips…".to_owned()
                            } else {
                                self.status.clone()
                            }),
                    )
                    .child(
                        self.button("refresh-trips")
                            .label("Refresh")
                            .disabled(self.loading)
                            .on_click(cx.listener(|view, _, _, cx| view.refresh(cx))),
                    ),
            )
            .when_some(self.error.clone(), |page, error| {
                page.child(div().text_sm().text_color(rgb(palette.amber)).child(error))
            });
        if matches!(self.route, TravelRoute::Trips) {
            if let Some(stamp) = self
                .saved_at
                .and_then(|stamp| chrono::DateTime::from_timestamp(stamp as i64, 0))
            {
                page = page.child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(format!(
                            "Trip list read {}",
                            stamp.with_timezone(&chrono::Local).format("%b %-d, %H:%M")
                        )),
                );
            }
            return page.child(self.trip_library(cx)).into_any_element();
        }
        if let Some(slot) = self.selected.as_ref().and_then(|id| self.details.get(id)) {
            page = page
                .when(slot.loading, |page| {
                    page.child(div().text_xs().child("Updating trip details…"))
                })
                .when_some(slot.error.clone(), |page, error| {
                    page.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(div().text_sm().text_color(rgb(palette.amber)).child(error))
                            .child(
                                self.button("retry-trip-details")
                                    .label("Retry details")
                                    .disabled(slot.loading)
                                    .on_click(cx.listener(|view, _, _, cx| {
                                        if let Some(id) = view.selected.clone() {
                                            view.read_details(id, true, cx);
                                        }
                                    })),
                            ),
                    )
                });
            if let Some(saved) = &slot.value {
                if let Some(stamp) = chrono::DateTime::from_timestamp(saved.saved_at as i64, 0) {
                    page = page.child(div().text_xs().text_color(rgb(palette.muted)).child(
                        format!(
                            "Details read {}",
                            stamp.with_timezone(&chrono::Local).format("%b %-d, %H:%M")
                        ),
                    ));
                }
            }
        }
        let Some(plan) = self.current() else {
            return page
                .child(
                    "This trip is not available on this device yet. Connect to your Mac and retry.",
                )
                .into_any_element();
        };
        page = page.child(div().text_xl().child(plan.title.clone())).child(
            div()
                .text_sm()
                .text_color(rgb(palette.muted))
                .child(format!(
                    "{} – {} · {}",
                    plan.date_start,
                    plan.date_end,
                    plan.status.replace('_', " ")
                )),
        );
        let section = self.section();
        let mut tabs = div().flex().flex_wrap().gap_1();
        for tab in TripSection::ALL {
            tabs = tabs.child(
                self.button(tab.path())
                    .label(tab.label())
                    .when(tab == section, |button| button.primary())
                    .on_click(cx.listener(move |view, _, _, cx| view.open_section(tab, cx))),
            );
        }
        page = page.child(tabs);
        page.child(match section {
            TripSection::Overview => self.overview(cx),
            TripSection::Itinerary => self.itinerary(cx),
            TripSection::Map => self.map_page(cx),
            TripSection::Items => self.saved_items(cx),
        })
        .into_any_element()
    }

    fn trip_library(&self, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let mut list = div().flex().flex_col().gap_3();
        if self.loaded && self.plans.is_empty() {
            list = list
                .child("No trips saved here yet. Create a trip in the dashboard, then refresh.");
        }
        let mut previous = None;
        for plan in self
            .plans
            .iter()
            .filter(|plan| self.show_all || plan.date_end >= today)
        {
            let past = plan.date_end < today;
            if previous != Some(past) {
                list = list.child(
                    div()
                        .text_sm()
                        .text_color(rgb(palette.muted))
                        .child(if past {
                            "Past trips"
                        } else {
                            "Current & upcoming"
                        }),
                );
                previous = Some(past);
            }
            let id = plan.id.clone();
            list = list.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .pb_3()
                    .border_b_1()
                    .border_color(rgba(palette.border))
                    .child(
                        self.button(gpui::SharedString::from(format!("trip-{}", plan.id)))
                            .ghost()
                            .label(plan.title.clone())
                            .on_click(
                                cx.listener(move |view, _, _, cx| view.select(id.clone(), cx)),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(format!(
                                "{} – {} · {}",
                                plan.date_start,
                                plan.date_end,
                                plan.status.replace('_', " ")
                            )),
                    )
                    .child(
                        div().text_sm().child(
                            plan.destinations
                                .iter()
                                .map(|p| p.name.as_str())
                                .collect::<Vec<_>>()
                                .join(" → "),
                        ),
                    ),
            );
        }
        let past = self.plans.iter().filter(|p| p.date_end < today).count();
        if past > 0 {
            list = list.child(
                self.button("show-past-trips")
                    .label(if self.show_all {
                        "Hide past trips".into()
                    } else {
                        format!("Show {past} past trips")
                    })
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.show_all = !view.show_all;
                        cx.notify();
                    })),
            );
        }
        list.into_any_element()
    }

    fn overview(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut page = div().flex().flex_col().gap_3();
        let Some(plan) = self.current() else {
            return page.into_any_element();
        };
        page = page
            .child(div().text_sm().child(format!("From {}", plan.origin.name)))
            .child(div().text_sm().child(format!(
                "{} stages · {} locations",
                plan.stages.len(),
                plan.places().count()
            )))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        self.button("open-itinerary")
                            .label("Open itinerary")
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.open_section(TripSection::Itinerary, cx)
                            })),
                    )
                    .child(self.button("open-map").label("View map").on_click(
                        cx.listener(|view, _, _, cx| view.open_section(TripSection::Map, cx)),
                    )),
            );
        for (i, place) in plan.places().enumerate() {
            page = page.child(self.place_row(i, place, cx));
        }
        page.into_any_element()
    }

    pub(super) fn place_row(
        &self,
        index: usize,
        place: &Place,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let place = place.clone();
        let mut row = div()
            .flex()
            .flex_col()
            .gap_1()
            .py_2()
            .border_b_1()
            .border_color(rgba(self.palette.border))
            .child(div().text_sm().child(place.name.clone()))
            .when_some(place.address.clone(), |row, address| {
                row.child(
                    div()
                        .text_xs()
                        .text_color(rgb(self.palette.muted))
                        .child(address),
                )
            });
        if place.coordinate().is_some() {
            row = row.child(
                self.button(gpui::SharedString::from(format!("place-{index}")))
                    .label("Show on map")
                    .on_click(cx.listener(move |view, _, _, cx| view.show_stop(place.clone(), cx))),
            );
        } else {
            row = row.child(
                div()
                    .text_xs()
                    .text_color(rgb(self.palette.muted))
                    .child("Coordinates not available"),
            );
        }
        row.into_any_element()
    }

    fn itinerary(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut page = div().flex().flex_col().gap_3();
        let Some(plan) = self.current() else {
            return page.into_any_element();
        };
        let mut stages: Vec<_> = plan.stages.iter().collect();
        stages.sort_by_key(|stage| stage.sequence);
        if stages.is_empty() {
            page = page.child("No travel stages saved for this trip.");
        }
        for (i, stage) in stages.into_iter().enumerate() {
            let destination = stage.destination.clone();
            page = page.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .py_2()
                    .border_b_1()
                    .border_color(rgba(self.palette.border))
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(self.palette.muted))
                            .child(format!(
                                "{} · {}",
                                stage.date.as_deref().unwrap_or("Date open"),
                                stage.status.replace('_', " ")
                            )),
                    )
                    .child(div().text_sm().child(format!(
                        "{} → {}",
                        stage.origin.name, stage.destination.name
                    )))
                    .child(
                        self.button(gpui::SharedString::from(format!("stage-map-{i}")))
                            .label("Destination on map")
                            .disabled(destination.coordinate().is_none())
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.show_stop(destination.clone(), cx)
                            })),
                    ),
            );
        }
        page.child(
            div()
                .text_sm()
                .text_color(rgb(self.palette.muted))
                .child("Items by day"),
        )
        .child(self.saved_items(cx))
        .into_any_element()
    }

    fn saved_items(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut page = div().flex().flex_col().gap_3();
        let Some(saved) = self
            .selected
            .as_ref()
            .and_then(|id| self.details.get(id))
            .and_then(|s| s.value.as_ref())
        else {
            return page
                .child("Saved activities, stays and bookings appear when trip details have loaded.")
                .into_any_element();
        };
        let mut items: Vec<_> = saved.details.items.iter().collect();
        items.sort_by(|a, b| {
            a.day
                .is_none()
                .cmp(&b.day.is_none())
                .then(a.day.cmp(&b.day))
                .then(a.title.cmp(&b.title))
        });
        if items.is_empty() {
            return page
                .child("No saved activities, stays or bookings for this trip.")
                .into_any_element();
        }
        let mut day = None;
        for item in items {
            if day != Some(&item.day) {
                day = Some(&item.day);
                page = page.child(
                    div()
                        .text_sm()
                        .text_color(rgb(self.palette.muted))
                        .child(item.day.clone().unwrap_or("No date assigned".into())),
                );
            }
            page = page.child(self.item_row(item, cx));
        }
        page.into_any_element()
    }

    fn item_row(&self, item: &Item, _: &mut Context<Self>) -> AnyElement {
        let mut row = div()
            .flex()
            .flex_col()
            .gap_1()
            .py_2()
            .border_b_1()
            .border_color(rgba(self.palette.border))
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(self.palette.muted))
                    .child(item.item_type.replace('_', " ")),
            )
            .child(div().text_sm().child(item.title.clone()));
        for (label, value) in item.fields() {
            row = row.child(div().text_sm().child(format!("{label}: {value}")));
        }
        if let Some(url) = item.source_url() {
            row = row.child(
                self.button(gpui::SharedString::from(format!("open-item-{}", item.id)))
                    .label("Open source")
                    .on_click(move |_, _, cx| cx.open_url(url.as_str())),
            );
        }
        row.into_any_element()
    }
}
