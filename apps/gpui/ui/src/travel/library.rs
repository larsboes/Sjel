//! Trip discovery, calendar drafts, and library list.
use super::forms::FormKind;
use super::{trip_draft_key, Travel};
use gpui::{div, prelude::*, px, rgb, rgba, AnyElement, Context, SharedString};
use gpui_kit::component::button::ButtonVariants;
use gpui_kit::component::{Disableable, Sizable};
use sjel_gpui_core::travel::{TravelRoute, TripSection};

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

    pub(super) fn trip_library(&self, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();

        let top_map = div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child("Map"),
                    )
                    .child(
                        div().flex().items_center().gap_1().child(
                            self.button("open-location-map")
                                .small()
                                .ghost()
                                .label("Open map")
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.open_route(TravelRoute::Map, cx)
                                })),
                        ),
                    ),
            )
            .child(self.category_chips(cx))
            .child(self.map(cx));

        let mut list = div()
            .flex()
            .flex_col()
            .gap_3()
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
        let (mut upcoming, mut past): (Vec<_>, Vec<_>) = self
            .plans
            .iter()
            .partition(|plan| plan.date_end >= today && plan.status != "cancelled");
        upcoming.sort_by(|a, b| a.date_start.cmp(&b.date_start));
        past.sort_by(|a, b| b.date_start.cmp(&a.date_start));

        if !upcoming.is_empty() {
            list = list.child(
                div()
                    .text_xs()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(rgb(palette.muted))
                    .child("Upcoming trips"),
            );
            for plan in upcoming {
                let id = plan.id.clone();
                let chosen = self.selected.as_deref() == Some(&plan.id);
                let offline_status = plan.check_offline_readiness();
                let offline_indicator = if offline_status.is_ready() {
                    div()
                        .px_1p5()
                        .py(px(1.))
                        .rounded(px(3.))
                        .bg(rgba(crate::theme::tint(palette.green, 0x18)))
                        .text_xs()
                        .text_color(rgb(palette.green))
                        .child("✓")
                        .into_any_element()
                } else {
                    div()
                        .px_1p5()
                        .py(px(1.))
                        .rounded(px(3.))
                        .bg(rgba(palette.surface))
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(format!("{}%", offline_status.percent()))
                        .into_any_element()
                };

                let row =
                    div()
                        .id(gpui::SharedString::from(format!("trip-row-{}", plan.id)))
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_3()
                        .p_3()
                        .rounded(px(8.))
                        .bg(if chosen {
                            rgba(palette.surface)
                        } else {
                            rgba(palette.raised)
                        })
                        .border_1()
                        .border_color(if chosen {
                            rgba(palette.accent)
                        } else {
                            rgba(palette.border)
                        })
                        .cursor_pointer()
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.select(id.clone(), cx);
                        }))
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
                                        .font_weight(gpui::FontWeight::MEDIUM)
                                        .child(plan.title.clone()),
                                )
                                .child(div().text_xs().text_color(rgb(palette.muted)).child(
                                    format!(
                                        "{} – {} · {} stages",
                                        plan.date_start,
                                        plan.date_end,
                                        plan.stages.len()
                                    ),
                                )),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(offline_indicator)
                                .child(
                                    div()
                                        .px_2()
                                        .py(px(2.))
                                        .rounded(px(4.))
                                        .bg(rgba(palette.surface))
                                        .text_xs()
                                        .text_color(rgb(palette.muted))
                                        .child(plan.status.replace('_', " ")),
                                ),
                        );
                list = list.child(row);
            }
        }

        if !past.is_empty() {
            let past_count = past.len();
            list = list.child(
                div()
                    .pt_3()
                    .text_xs()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(rgb(palette.muted))
                    .child(format!("Past trips ({past_count})")),
            );
            if self.show_all {
                for plan in past {
                    let id = plan.id.clone();
                    let chosen = self.selected.as_deref() == Some(&plan.id);
                    let row = div()
                        .id(gpui::SharedString::from(format!("trip-row-{}", plan.id)))
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_3()
                        .p_3()
                        .rounded(px(8.))
                        .bg(if chosen {
                            rgba(palette.surface)
                        } else {
                            rgba(palette.raised)
                        })
                        .border_1()
                        .border_color(if chosen {
                            rgba(palette.accent)
                        } else {
                            rgba(palette.border)
                        })
                        .opacity(0.8)
                        .cursor_pointer()
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.select(id.clone(), cx);
                        }))
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
                                        .font_weight(gpui::FontWeight::MEDIUM)
                                        .child(plan.title.clone()),
                                )
                                .child(div().text_xs().text_color(rgb(palette.muted)).child(
                                    format!(
                                        "{} – {} · {} stages",
                                        plan.date_start,
                                        plan.date_end,
                                        plan.stages.len()
                                    ),
                                )),
                        )
                        .child(
                            div()
                                .px_2()
                                .py(px(2.))
                                .rounded(px(4.))
                                .bg(rgba(palette.surface))
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child(plan.status.replace('_', " ")),
                        );
                    list = list.child(row);
                }
            }
            list = list.child(
                self.button("toggle-past-trips")
                    .ghost()
                    .label(if self.show_all {
                        "Hide past trips".to_string()
                    } else {
                        format!("Show {past_count} past trips")
                    })
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.show_all = !view.show_all;
                        cx.notify();
                    })),
            );
        }
        if self.density.is_mobile() {
            div()
                .flex()
                .flex_col()
                .gap_3()
                .child(top_map)
                .child(list)
                .into_any_element()
        } else {
            let master = div().w(px(380.)).flex_none().child(list);
            let detail = div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap_3()
                .child(top_map)
                .child(self.map_stops(cx));
            div()
                .w_full()
                .flex()
                .items_start()
                .gap_4()
                .child(master)
                .child(detail)
                .into_any_element()
        }
    }

    pub(super) fn trip_suggestions(&self, cx: &mut Context<Self>) -> AnyElement {
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
}
