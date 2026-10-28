//! Travel's trip library and the four subpages of an opened trip.
use super::forms::{FormKind, Remove};
use super::{trip_draft_key, Travel};
use gpui::{div, prelude::*, px, rgb, rgba, AnyElement, ClipboardItem, Context, SharedString};
use gpui_kit::component::button::ButtonVariants;
use gpui_kit::component::Disableable;
use sjel_gpui_core::{
    shell::CalendarEntry,
    travel::{Item, Place, TravelRoute, TripSection},
};

impl Travel {
    /// The nearest trip that has not ended: a saved plan, or a Calendar draft that is not a
    /// plan yet, with the plan's id so Home can open it. `None` when there is neither, so
    /// Home spends no height on it.
    pub fn home_next_trip(
        &mut self,
        palette: crate::Palette,
        cx: &mut Context<Self>,
    ) -> Option<(AnyElement, Option<String>)> {
        self.set_palette(palette, cx);
        if !self.loaded && !self.loading {
            self.refresh(cx);
        }
        self.load_trip_drafts(cx, false);
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let plan = self
            .plans
            .iter()
            .filter(|plan| plan.date_end >= today && plan.status != "cancelled")
            .min_by(|a, b| a.date_start.cmp(&b.date_start))
            .cloned();
        let draft = self.trip_drafts.as_ref().and_then(|drafts| {
            drafts
                .drafts
                .iter()
                .filter(|draft| !self.trip_draft_results.contains_key(&trip_draft_key(draft)))
                .min_by(|a, b| a.starts_on.cmp(&b.starts_on))
                .cloned()
        });
        let show_draft = match (&plan, &draft) {
            (Some(plan), Some(draft)) => draft.starts_on < plan.date_start,
            (None, Some(_)) => true,
            _ => false,
        };
        let days_until = |start: &str| {
            chrono::NaiveDate::parse_from_str(start, "%Y-%m-%d")
                .ok()
                .map(|day| (day - chrono::Local::now().date_naive()).num_days())
        };
        let when = |start: &str| match days_until(start) {
            Some(days) if days <= 0 => "now".to_string(),
            Some(1) => "tomorrow".to_string(),
            Some(days) => format!("in {days} days"),
            None => start.to_string(),
        };
        let line = |title: String, detail: String| {
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .child(title),
                )
                .child(div().text_xs().text_color(rgb(palette.muted)).child(detail))
        };
        if show_draft {
            let draft = draft?;
            let busy = self.trip_draft_action.is_some();
            let detail = format!(
                "{} · {} · found in Calendar, not a trip yet",
                draft.starts_on,
                when(&draft.starts_on)
            );
            return Some((
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(line(draft.place.clone(), detail))
                    .child(
                        self.button("home-create-trip")
                            .primary()
                            .label(if busy {
                                "Creating trip…"
                            } else {
                                "Create trip"
                            })
                            .disabled(busy || self.writer().is_none())
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.materialize_trip_draft(draft.clone(), cx)
                            })),
                    )
                    .into_any_element(),
                None,
            ));
        }
        let plan = plan?;
        let detail = format!(
            "{} – {} · {}",
            plan.date_start,
            plan.date_end,
            when(&plan.date_start)
        );
        Some((
            line(plan.title.clone(), detail).into_any_element(),
            Some(plan.id),
        ))
    }

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
                            .label("← Location overview")
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
        if self.route == TravelRoute::Connections {
            return page.child(self.connections_page(cx)).into_any_element();
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
            })
            .when_some(self.write_error.clone(), |page, error| {
                page.child(div().text_sm().text_color(rgb(palette.red)).child(error))
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
        if matches!(self.route, TravelRoute::Map) {
            return page
                .child(div().text_xl().child("Location overview"))
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(palette.muted))
                        .child("Current and upcoming trip locations"),
                )
                .child(self.map_page(cx))
                .into_any_element();
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
        page = page
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(div().text_xl().child(plan.title.clone()))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .px_2()
                            .py(px(2.))
                            .rounded(px(4.))
                            .bg(rgba(crate::theme::tint(palette.green, 0x18)))
                            .border_1()
                            .border_color(rgba(crate::theme::tint(palette.green, 0x35)))
                            .text_xs()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(rgb(palette.green))
                            .child("✓ Offline ready"),
                    ),
            )
            .child(
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
        let mut list = div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                self.button("open-location-map")
                    .label("Explore locations on map")
                    .on_click(cx.listener(|view, _, _, cx| view.open_route(TravelRoute::Map, cx))),
            )
            .child(self.trip_suggestions(cx))
            .child(
                div().child(
                    self.button("open-connections")
                        .label("Find a connection")
                        .on_click(cx.listener(|view, _, _, cx| {
                            view.open_route(TravelRoute::Connections, cx)
                        })),
                ),
            )
            .children(self.form_button("new-trip", "New trip", FormKind::Trip(None), cx))
            .children(self.form_card(&FormKind::Trip(None), cx));
        if self.loaded && self.plans.is_empty() {
            list = list.child(if self.writer().is_some() {
                "No trips yet."
            } else {
                "No trips saved here yet. Create one on the Mac, then refresh."
            });
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
            let open_id = id.clone();
            let trip = div()
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
                            cx.listener(move |view, _, _, cx| view.select(open_id.clone(), cx)),
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
                );
            list = list.child(
                div()
                    .flex()
                    .items_start()
                    .gap_2()
                    .child(trip.flex_1().min_w_0())
                    .children(self.remove_button(Remove::Plan(id), cx)),
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

    fn trip_suggestions(&self, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        let mut panel = div()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .rounded(px(8.))
            .border_1()
            .border_color(rgba(palette.border))
            .bg(rgba(palette.surface))
            .child(
                div()
                    .text_sm()
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .child("Trip suggestions from Calendar"),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child("Calendar groups dated events, including events found in mail, by destination. Review the dates and place before creating a trip."),
            );

        if let Some(error) = &self.trip_drafts_error {
            panel = panel.child(
                div()
                    .text_sm()
                    .text_color(rgb(palette.amber))
                    .child(error.clone()),
            );
        } else if self.trip_drafts_loading && self.trip_drafts.is_none() {
            panel = panel.child(
                div()
                    .text_sm()
                    .text_color(rgb(palette.muted))
                    .child("Checking the next 180 days…"),
            );
        } else if let Some(drafts) = &self.trip_drafts {
            if drafts.drafts.is_empty() {
                panel = panel.child(div().text_sm().text_color(rgb(palette.muted)).child(
                    if drafts.unclustered.is_empty() {
                        "No trip suggestions yet. Dated events found in mail appear here after Calendar reviews them."
                    } else {
                        "No events are grouped into a trip yet. Some events lack a place or are at home."
                    },
                ));
            }
            for draft in &drafts.drafts {
                let key = trip_draft_key(draft);
                let title = format!("{} — {}", draft.place, draft.starts_on);
                let end = chrono::NaiveDate::parse_from_str(&draft.ends_before, "%Y-%m-%d")
                    .ok()
                    .map(|day| {
                        (day - chrono::Duration::days(1))
                            .format("%b %-d")
                            .to_string()
                    })
                    .unwrap_or_else(|| draft.ends_before.clone());
                let dates = format!("{} – {}", draft.starts_on, end);
                let materialized = self.trip_draft_results.get(&key);
                let busy = self.trip_draft_action.as_deref() == Some(key.as_str());
                panel = panel.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .py_2()
                        .border_t_1()
                        .border_color(rgba(palette.border))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap_2()
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(gpui::FontWeight::MEDIUM)
                                        .child(draft.place.clone()),
                                )
                                .child(
                                    div()
                                        .px_2()
                                        .py(px(2.))
                                        .rounded(px(5.))
                                        .bg(rgba(palette.raised))
                                        .text_xs()
                                        .text_color(rgb(palette.muted))
                                        .child(match draft.commitment.as_str() {
                                            "committed" => "Committed",
                                            "planned" => "Planned",
                                            _ => "Possible",
                                        }),
                                ),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child(format!("{dates} · {} events", draft.titles.len())),
                        )
                        .children(draft.titles.iter().map(|event| {
                            div()
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child(format!("• {event}"))
                        }))
                        .child(if let Some(result) = materialized {
                            let id = result.plan_id.clone();
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(div().text_xs().text_color(rgb(palette.accent)).child(
                                    if result.created {
                                        "Trip created"
                                    } else {
                                        "Already linked to a trip"
                                    },
                                ))
                                .child(
                                    self.button(SharedString::from(format!(
                                        "open-draft-trip-{id}"
                                    )))
                                    .ghost()
                                    .label("Open trip")
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        view.open_route(
                                            TravelRoute::Trip {
                                                id: id.clone(),
                                                section: TripSection::Overview,
                                            },
                                            cx,
                                        )
                                    })),
                                )
                                .into_any_element()
                        } else if self.writer().is_some() {
                            let draft = draft.clone();
                            div()
                                .child(
                                    self.button(SharedString::from(format!(
                                        "create-draft-trip-{key}"
                                    )))
                                    .primary()
                                    .label(if busy {
                                        "Creating trip…".to_owned()
                                    } else {
                                        format!("Create {title}")
                                    })
                                    .disabled(self.trip_draft_action.is_some())
                                    .on_click(cx.listener(move |view, _, _, cx| {
                                        view.materialize_trip_draft(draft.clone(), cx)
                                    })),
                                )
                                .into_any_element()
                        } else {
                            div()
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child("Connect to your Mac to create this trip.")
                                .into_any_element()
                        }),
                );
            }
            if !drafts.unclustered.is_empty() {
                panel = panel.child(
                    div()
                        .pt_2()
                        .border_t_1()
                        .border_color(rgba(palette.border))
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(format!(
                            "{} event{} need review before they can shape a trip:",
                            drafts.unclustered.len(),
                            if drafts.unclustered.len() == 1 {
                                ""
                            } else {
                                "s"
                            }
                        )),
                );
                for entry in drafts.unclustered.iter().take(8) {
                    panel = panel.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .py_1()
                            .child(div().text_xs().child(entry.title.clone()))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(palette.muted))
                                    .child(entry.reason.clone()),
                            ),
                    );
                }
            }
        }
        panel.into_any_element()
    }

    fn overview(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut page = div().flex().flex_col().gap_3();
        let Some(plan) = self.current() else {
            return page.into_any_element();
        };
        let items: Vec<Item> = self
            .selected
            .as_ref()
            .and_then(|id| self.details.get(id))
            .and_then(|s| s.value.as_ref())
            .map(|s| s.details.items.clone())
            .unwrap_or_default();

        page = page.child(self.readiness_strip(plan, &items));
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
        page = page.child(self.trip_calendar_timeline());
        for (i, place) in plan.places().enumerate() {
            page = page.child(self.place_row(i, place, cx));
        }
        page.into_any_element()
    }

    fn readiness_strip(&self, plan: &sjel_gpui_core::travel::Plan, items: &[Item]) -> AnyElement {
        let palette = self.palette;
        let start_date = chrono::NaiveDate::parse_from_str(&plan.date_start, "%Y-%m-%d").ok();
        let end_date = chrono::NaiveDate::parse_from_str(&plan.date_end, "%Y-%m-%d").ok();
        let total_nights = match (start_date, end_date) {
            (Some(s), Some(e)) => (e - s).num_days().max(0) as usize,
            _ => 0,
        };
        let lodging_count = items
            .iter()
            .filter(|it| {
                let ty = it.item_type.to_lowercase();
                ty.contains("lodging") || ty.contains("stay") || ty.contains("hotel")
            })
            .count();
        let unbooked_stages = plan.stages.iter().filter(|s| s.status != "booked").count();
        let missing_codes = items
            .iter()
            .filter(|it| it.booking_code().is_none())
            .count();

        let accom_pill = if total_nights == 0 {
            div()
                .px_2()
                .py(px(2.))
                .rounded(px(6.))
                .bg(rgba(palette.raised))
                .text_xs()
                .text_color(rgb(palette.muted))
                .child("Day trip")
        } else if lodging_count >= total_nights {
            div()
                .px_2()
                .py(px(2.))
                .rounded(px(6.))
                .bg(rgba(crate::theme::tint(palette.green, 0x22)))
                .text_xs()
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(rgb(palette.green))
                .child(format!("✓ {lodging_count}/{total_nights} nights covered"))
        } else {
            div()
                .px_2()
                .py(px(2.))
                .rounded(px(6.))
                .bg(rgba(crate::theme::tint(palette.amber, 0x22)))
                .text_xs()
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(rgb(palette.amber))
                .child(format!("⚠ {lodging_count}/{total_nights} nights covered"))
        };

        let stages_pill = if plan.stages.is_empty() {
            div()
                .px_2()
                .py(px(2.))
                .rounded(px(6.))
                .bg(rgba(palette.raised))
                .text_xs()
                .text_color(rgb(palette.muted))
                .child("No stages")
        } else if unbooked_stages == 0 {
            div()
                .px_2()
                .py(px(2.))
                .rounded(px(6.))
                .bg(rgba(crate::theme::tint(palette.green, 0x22)))
                .text_xs()
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(rgb(palette.green))
                .child(format!("✓ All {} stages booked", plan.stages.len()))
        } else {
            div()
                .px_2()
                .py(px(2.))
                .rounded(px(6.))
                .bg(rgba(crate::theme::tint(palette.amber, 0x22)))
                .text_xs()
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(rgb(palette.amber))
                .child(format!("⚠ {unbooked_stages} unbooked stage(s)"))
        };

        let codes_pill = if items.is_empty() {
            div().into_any_element()
        } else if missing_codes == 0 {
            div()
                .px_2()
                .py(px(2.))
                .rounded(px(6.))
                .bg(rgba(crate::theme::tint(palette.green, 0x22)))
                .text_xs()
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(rgb(palette.green))
                .child(format!("✓ All {} booking codes", items.len()))
                .into_any_element()
        } else {
            div()
                .px_2()
                .py(px(2.))
                .rounded(px(6.))
                .bg(rgba(palette.raised))
                .text_xs()
                .text_color(rgb(palette.muted))
                .child(format!(
                    "{}/{} booking codes",
                    items.len() - missing_codes,
                    items.len()
                ))
                .into_any_element()
        };

        div()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .rounded(px(8.))
            .bg(rgba(palette.surface))
            .border_1()
            .border_color(rgba(palette.border))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(palette.muted))
                            .child("TRIP READINESS & GAPS"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(format!("{} – {}", plan.date_start, plan.date_end)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(accom_pill)
                    .child(stages_pill)
                    .child(codes_pill),
            )
            .into_any_element()
    }

    fn trip_calendar_timeline(&self) -> AnyElement {
        let palette = self.palette;
        let mut section = div()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .rounded(px(8.))
            .border_1()
            .border_color(rgba(palette.border))
            .bg(rgba(palette.surface))
            .child(
                div()
                    .text_sm()
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .child("Trip timeline"),
            );
        let Some(id) = self.selected.as_ref() else {
            return section.into_any_element();
        };
        let Some(slot) = self.details.get(id) else {
            return section
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(palette.muted))
                        .child("Loading trip events…"),
                )
                .into_any_element();
        };
        if !slot.calendar_links_loaded
            || slot.calendar_links_loading
            || !slot.calendar_events_loaded
            || slot.calendar_events_loading
        {
            section = section.child(
                div()
                    .text_sm()
                    .text_color(rgb(palette.muted))
                    .child("Loading trip events…"),
            );
        } else if let Some(error) = &slot.calendar_links_error {
            section = section.child(
                div()
                    .text_sm()
                    .text_color(rgb(palette.amber))
                    .child(format!("Trip events could not load: {error}")),
            );
        } else if let Some(error) = &slot.calendar_events_error {
            section = section.child(
                div()
                    .text_sm()
                    .text_color(rgb(palette.amber))
                    .child(format!("Nearby Calendar events could not load: {error}")),
            );
        } else {
            let linked: Vec<_> = slot
                .calendar_links
                .iter()
                .filter(|link| link.via == "trip materialization")
                .collect();
            let linked_ids: std::collections::HashSet<_> =
                linked.iter().map(|link| link.id.as_str()).collect();
            let nearby: Vec<_> = slot
                .calendar_events
                .iter()
                .filter(|event| !linked_ids.contains(event.id.as_str()))
                .collect();
            if linked.is_empty() && nearby.is_empty() {
                section = section.child(
                    div()
                        .text_sm()
                        .text_color(rgb(palette.muted))
                        .child("No mail or Calendar events fall in this trip window."),
                );
            } else {
                for event in linked {
                    let entry = slot
                        .calendar_events
                        .iter()
                        .find(|entry| entry.id == event.id);
                    section = section.child(calendar_link_row(event, entry, palette));
                }
                if !nearby.is_empty() {
                    section = section
                        .child(
                            div()
                                .pt_2()
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child("Nearby Calendar events · not linked to this trip"),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child("Calendar grouped these by destination and travel window. Add one to the itinerary to link it."),
                        );
                    for event in nearby {
                        section = section.child(calendar_event_row(event, palette));
                    }
                }
            }
        }
        section.into_any_element()
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
        let palette = self.palette;
        let mut page = div().flex().flex_col().gap_3();
        let Some(plan) = self.current() else {
            return page.into_any_element();
        };

        page = page
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .children(self.form_button("add-stage", "Add stage", FormKind::Stage(None), cx))
                    .children(self.form_button(
                        "add-booking",
                        "Add booking",
                        FormKind::Booking(None),
                        cx,
                    )),
            )
            .children(self.form_card(&FormKind::Stage(None), cx))
            .children(self.form_card(&FormKind::Booking(None), cx));

        let mut stages: Vec<_> = plan.stages.iter().collect();
        stages.sort_by_key(|stage| stage.sequence);

        let items: Vec<Item> = self
            .selected
            .as_ref()
            .and_then(|id| self.details.get(id))
            .and_then(|s| s.value.as_ref())
            .map(|s| s.details.items.clone())
            .unwrap_or_default();

        let mut days = std::collections::BTreeSet::new();
        for stage in &stages {
            if let Some(date) = &stage.date {
                days.insert(date.clone());
            }
        }
        for item in &items {
            if let Some(day) = &item.day {
                days.insert(day.clone());
            }
        }

        if stages.is_empty() && items.is_empty() {
            return page
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(palette.muted))
                        .child("No travel stages or items saved for this trip."),
                )
                .into_any_element();
        }

        let day_filters = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_1()
            .child(
                div()
                    .id("filter-day-all")
                    .px_2()
                    .py(px(2.))
                    .rounded(px(6.))
                    .text_xs()
                    .cursor_pointer()
                    .when(self.selected_day_filter.is_none(), |el| {
                        el.bg(rgba(palette.accent))
                            .text_color(rgb(0xffffff))
                            .font_weight(gpui::FontWeight::MEDIUM)
                    })
                    .when(self.selected_day_filter.is_some(), |el| {
                        el.bg(rgba(palette.raised)).text_color(rgb(palette.muted))
                    })
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.selected_day_filter = None;
                        cx.notify();
                    }))
                    .child("All days"),
            )
            .children(days.iter().enumerate().map(|(idx, day_str)| {
                let this_day = day_str.clone();
                let active = self.selected_day_filter.as_deref() == Some(day_str.as_str());
                let short_date = chrono::NaiveDate::parse_from_str(day_str, "%Y-%m-%d")
                    .ok()
                    .map(|d| d.format("%b %-d").to_string())
                    .unwrap_or_else(|| day_str.clone());
                div()
                    .id(gpui::SharedString::from(format!("filter-day-{idx}")))
                    .px_2()
                    .py(px(2.))
                    .rounded(px(6.))
                    .text_xs()
                    .cursor_pointer()
                    .when(active, |el| {
                        el.bg(rgba(palette.accent))
                            .text_color(rgb(0xffffff))
                            .font_weight(gpui::FontWeight::MEDIUM)
                    })
                    .when(!active, |el| {
                        el.bg(rgba(palette.raised)).text_color(rgb(palette.muted))
                    })
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.selected_day_filter = Some(this_day.clone());
                        cx.notify();
                    }))
                    .child(format!("Day {} · {}", idx + 1, short_date))
            }));

        page = page.child(day_filters);

        let filtered_days: Vec<&String> = if let Some(target) = self.selected_day_filter.as_ref() {
            days.iter().filter(|d| *d == target).collect()
        } else {
            days.iter().collect()
        };

        for day_str in filtered_days {
            let day_stages: Vec<(usize, &sjel_gpui_core::travel::Stage)> = stages
                .iter()
                .enumerate()
                .filter(|(_, s)| s.date.as_deref() == Some(day_str.as_str()))
                .map(|(i, s)| (i, *s))
                .collect();
            let day_items: Vec<&Item> = items
                .iter()
                .filter(|item| item.day.as_deref() == Some(day_str.as_str()))
                .collect();

            let mut day_card = div()
                .flex()
                .flex_col()
                .gap_2()
                .p_3()
                .rounded(px(10.))
                .bg(rgba(palette.surface))
                .border_1()
                .border_color(rgba(palette.border));

            day_card = day_card.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .border_b_1()
                    .border_color(rgba(palette.border))
                    .pb_2()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgb(palette.text))
                            .child(day_str.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(format!(
                                "{} stage(s) · {} item(s)",
                                day_stages.len(),
                                day_items.len()
                            )),
                    ),
            );

            for (stage_idx, (orig_i, stage)) in day_stages.iter().enumerate() {
                let editing = FormKind::Stage(Some(stage.id.clone()));
                if let Some(card) = self.form_card(&editing, cx) {
                    day_card = day_card.child(card);
                    continue;
                }
                let mode = stage
                    .transport_modes
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "transit".into());

                let destination = stage.destination.clone();
                let mut stage_row = div().flex().flex_col().gap_1().py_2().child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .px_2()
                                .py(px(1.5))
                                .rounded(px(4.))
                                .bg(rgba(crate::theme::tint(palette.accent, 0x18)))
                                .border_1()
                                .border_color(rgba(crate::theme::tint(palette.accent, 0x35)))
                                .text_xs()
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(rgb(palette.accent))
                                .child(mode.to_uppercase()),
                        )
                        .child(div().text_sm().font_weight(gpui::FontWeight::MEDIUM).child(
                            format!("{} → {}", stage.origin.name, stage.destination.name),
                        ))
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child(stage.status.replace('_', " ")),
                        ),
                );

                let mut actions = div().flex().flex_wrap().gap_2();
                actions = actions.child(
                    self.button(gpui::SharedString::from(format!("stage-map-{orig_i}")))
                        .ghost()
                        .label("Map")
                        .disabled(destination.coordinate().is_none())
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.show_stop(destination.clone(), cx)
                        })),
                );
                if !stage.id.is_empty() {
                    actions = actions
                        .children(self.form_button(
                            gpui::SharedString::from(format!("stage-edit-{orig_i}")),
                            "Edit",
                            editing,
                            cx,
                        ))
                        .children(self.remove_button(Remove::Stage(stage.id.clone()), cx));
                }
                stage_row = stage_row.child(actions);
                day_card = day_card.child(stage_row);

                if stage_idx + 1 < day_stages.len() {
                    let (_, next_stage) = day_stages[stage_idx + 1];
                    let same_stop = stage.destination.name == next_stage.origin.name;
                    day_card = day_card.child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .px_3()
                            .py(px(4.))
                            .rounded(px(6.))
                            .bg(rgba(palette.raised))
                            .text_xs()
                            .child(
                                div()
                                    .text_color(rgb(palette.muted))
                                    .child("Transfer connection:"),
                            )
                            .child(
                                div()
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .text_color(rgb(if same_stop {
                                        palette.green
                                    } else {
                                        palette.amber
                                    }))
                                    .child(if same_stop {
                                        format!(
                                            "At {} (seamless station transfer)",
                                            stage.destination.name
                                        )
                                    } else {
                                        format!(
                                            "{} → {} (transfer buffer required)",
                                            stage.destination.name, next_stage.origin.name
                                        )
                                    }),
                            ),
                    );
                }
            }

            if !day_items.is_empty() {
                day_card = day_card.child(
                    div()
                        .pt_2()
                        .border_t_1()
                        .border_color(rgba(palette.border))
                        .text_xs()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(rgb(palette.muted))
                        .child("Bookings & Activities"),
                );
                for item in day_items {
                    day_card = day_card.child(self.item_row(item, cx));
                }
            }

            page = page.child(day_card);
        }

        let undated_stages: Vec<(usize, &sjel_gpui_core::travel::Stage)> = stages
            .iter()
            .enumerate()
            .filter(|(_, s)| s.date.is_none())
            .map(|(i, s)| (i, *s))
            .collect();
        let undated_items: Vec<&Item> = items.iter().filter(|i| i.day.is_none()).collect();

        if !undated_stages.is_empty() || !undated_items.is_empty() {
            let mut undated_card = div()
                .flex()
                .flex_col()
                .gap_2()
                .p_3()
                .rounded(px(10.))
                .bg(rgba(palette.surface))
                .border_1()
                .border_color(rgba(palette.border))
                .child(
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(rgb(palette.muted))
                        .child("Undated Stages & Items"),
                );
            for (orig_i, stage) in undated_stages {
                let editing = FormKind::Stage(Some(stage.id.clone()));
                if let Some(card) = self.form_card(&editing, cx) {
                    undated_card = undated_card.child(card);
                    continue;
                }
                undated_card = undated_card.child(
                    div()
                        .py_1()
                        .child(format!(
                            "{} → {}",
                            stage.origin.name, stage.destination.name
                        ))
                        .children(self.form_button(
                            gpui::SharedString::from(format!("stage-edit-{orig_i}")),
                            "Edit",
                            editing,
                            cx,
                        )),
                );
            }
            for item in undated_items {
                undated_card = undated_card.child(self.item_row(item, cx));
            }
            page = page.child(undated_card);
        }

        page.into_any_element()
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
        page = page
            .children(self.form_button("add-booking", "Add booking", FormKind::Booking(None), cx))
            .children(self.form_card(&FormKind::Booking(None), cx));
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

    fn item_row(&self, item: &Item, cx: &mut Context<Self>) -> AnyElement {
        let editing = FormKind::Booking(Some(item.id.clone()));
        if let Some(card) = self.form_card(&editing, cx) {
            return card;
        }
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
        let mut actions = div().flex().flex_wrap().gap_2();
        if let Some(code) = item.booking_code() {
            let code_str = code.to_owned();
            actions = actions.child(
                self.button(gpui::SharedString::from(format!("copy-code-{}", item.id)))
                    .primary()
                    .label(format!("Copy code: {code_str}"))
                    .on_click(cx.listener(move |_view, _, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(code_str.clone()));
                    })),
            );
        }
        if let Some(url) = item.source_url() {
            actions = actions.child(
                self.button(gpui::SharedString::from(format!("open-item-{}", item.id)))
                    .label("Open source")
                    .on_click(move |_, _, cx| cx.open_url(url.as_str())),
            );
        }
        if item.item_type == "booking" {
            actions = actions.children(self.form_button(
                gpui::SharedString::from(format!("edit-item-{}", item.id)),
                "Edit",
                editing,
                cx,
            ));
        }
        row.child(actions.children(self.remove_button(Remove::Item(item.id.clone()), cx)))
            .into_any_element()
    }
}

fn calendar_event_row(event: &CalendarEntry, palette: crate::Palette) -> AnyElement {
    let when = calendar_event_when(event);
    div()
        .flex()
        .flex_col()
        .gap_1()
        .py_2()
        .border_t_1()
        .border_color(rgba(palette.border))
        .child(div().text_xs().text_color(rgb(palette.muted)).child(when))
        .child(
            div()
                .text_sm()
                .font_weight(gpui::FontWeight::MEDIUM)
                .child(event.title.clone()),
        )
        .when_some(event.location.clone(), |row, location| {
            row.child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child(location),
            )
        })
        .when_some(event.notes.clone(), |row, notes| {
            row.child(calendar_agenda(&notes, palette))
        })
        .into_any_element()
}

fn calendar_agenda(notes: &str, palette: crate::Palette) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(crate::doc_view::render_markdown(
            notes,
            palette,
            crate::Density::Desktop,
        ))
        .into_any_element()
}

fn calendar_event_when(event: &CalendarEntry) -> String {
    if event.all_day {
        format!(
            "{} · all day",
            event.starts_at.get(..10).unwrap_or(&event.starts_at)
        )
    } else if let Some((day, time)) = event.starts_at.split_once('T') {
        let start = time.get(..5).unwrap_or(time);
        let end = event
            .ends_at
            .split_once('T')
            .map(|(_, time)| time.get(..5).unwrap_or(time))
            .unwrap_or("");
        if end.is_empty() {
            format!("{day} · {start}")
        } else {
            let end_day = event.ends_at.get(..10).unwrap_or(day);
            let end = if end_day == day {
                end.to_owned()
            } else {
                format!("{end_day} {end}")
            };
            format!("{day} · {start}–{end}")
        }
    } else {
        event.starts_at.clone()
    }
}

fn calendar_link_row(
    link: &sjel_gpui_core::record::Link,
    entry: Option<&CalendarEntry>,
    palette: crate::Palette,
) -> AnyElement {
    let when = entry
        .map(calendar_event_when)
        .or_else(|| {
            link.at.as_deref().map(|at| {
                at.split_once('T')
                    .map(|(day, time)| format!("{day} · {}", time.get(..5).unwrap_or(time)))
                    .unwrap_or_else(|| at.to_owned())
            })
        })
        .unwrap_or_else(|| "Date unavailable".into());
    div()
        .flex()
        .flex_col()
        .gap_1()
        .py_2()
        .border_t_1()
        .border_color(rgba(palette.border))
        .child(div().text_xs().text_color(rgb(palette.muted)).child(when))
        .child(
            div()
                .text_sm()
                .font_weight(gpui::FontWeight::MEDIUM)
                .child(link.title.clone()),
        )
        .when_some(link.meta.clone(), |row, location| {
            row.child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child(location),
            )
        })
        .when_some(entry.and_then(|entry| entry.notes.clone()), |row, notes| {
            row.child(calendar_agenda(&notes, palette))
        })
        .into_any_element()
}
