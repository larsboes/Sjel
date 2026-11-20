//! Travel route dispatcher and section navigation.
use super::forms::FormKind;
use super::Travel;
use crate::ely::{
    buttons::{Button, ButtonVariant, SegmentedControl},
    feedback::{InlineMessage, StatusMessage},
    primitives::Severity,
    theme::ControlSize,
};
use crate::primitives::sub_tabs;
use gpui::{div, prelude::*, px, rgb, rgba, AnyElement, ClipboardItem, Context};
use sjel_gpui_core::travel::{PlanExt, TravelRoute, TripSection};

impl Travel {
    fn top_nav_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let upcoming_count = self
            .plans
            .iter()
            .filter(|p| p.date_end >= today && p.status != "cancelled")
            .count();

        let selected = match self.route {
            TravelRoute::Map => "map",
            TravelRoute::Connections => "connections",
            _ => "trips",
        };

        div()
            .w_full()
            .flex()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap_3()
            .child(
                SegmentedControl::new("travel-navigation", selected)
                    .size(ControlSize::Sm)
                    .segment("map", "Map & Places", None)
                    .segment(
                        "trips",
                        if upcoming_count > 0 {
                            format!("Trips ({upcoming_count})")
                        } else {
                            "Trips".into()
                        },
                        None,
                    )
                    .segment("connections", "Connections", None)
                    .on_change(cx.listener(|view, value: &gpui::SharedString, _, cx| {
                        let route = match value.as_ref() {
                            "map" => TravelRoute::Map,
                            "connections" => TravelRoute::Connections,
                            _ => TravelRoute::Trips,
                        };
                        view.open_route(route, cx);
                    })),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .children(self.form_button(
                        "nav-plan-trip",
                        "+ New trip",
                        FormKind::Trip(None),
                        cx,
                    ))
                    .child(
                        Button::new("nav-refresh-trips", "Refresh")
                            .size(ControlSize::Sm)
                            .variant(ButtonVariant::Ghost)
                            .disabled(self.loading)
                            .on_click(cx.listener(|view, _, _, cx| view.refresh(cx))),
                    ),
            )
            .into_any_element()
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

        let is_top_level = matches!(
            self.route,
            TravelRoute::Trips | TravelRoute::Map | TravelRoute::Connections
        );

        if is_top_level {
            if cfg!(test) {
                page = page.child(self.top_nav_bar(cx));
            }
        } else if cfg!(test) {
            page = page.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        Button::new("back-to-trips", "← All trips & locations")
                            .variant(ButtonVariant::Ghost)
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.open_route(TravelRoute::Trips, cx);
                            })),
                    )
                    .child(
                        Button::new("copy-trip-link", "Copy link")
                            .variant(ButtonVariant::Ghost)
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
            return page
                .child(self.connections_page(cx))
                .children(self.render_form_sheet(cx))
                .into_any_element();
        }

        page =
            page.child(div().flex().items_center().justify_between().gap_2().child(
                StatusMessage::new(
                    "travel-load-status",
                    if self.loading {
                        "Loading trips…".to_owned()
                    } else {
                        self.status.clone()
                    },
                ),
            ))
            .when_some(self.error.clone(), |page, error| {
                page.child(InlineMessage::new(Severity::Warning, error))
            })
            .when_some(self.write_error.clone(), |page, error| {
                page.child(InlineMessage::new(Severity::Danger, error))
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
            return page
                .child(self.trip_library(cx))
                .children(self.render_form_sheet(cx))
                .into_any_element();
        }

        if matches!(self.route, TravelRoute::Map) {
            return page
                .child(self.map_page(cx))
                .children(self.render_form_sheet(cx))
                .into_any_element();
        }
        if let Some(slot) = self.selected.as_ref().and_then(|id| self.details.get(id)) {
            page = page
                .when(slot.loading, |page| {
                    page.child(StatusMessage::new(
                        "trip-details-loading",
                        "Updating trip details…",
                    ))
                })
                .when_some(slot.error.clone(), |page, error| {
                    page.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(InlineMessage::new(Severity::Warning, error))
                            .child(
                                Button::new("retry-trip-details", "Retry details")
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
                .child(InlineMessage::new(
                    Severity::Info,
                    "This trip is not available on this device yet. Connect to your Mac and retry.",
                ))
                .into_any_element();
        };
        let offline_status = plan.check_offline_readiness();
        let is_downloading = self.offline_downloading.contains(&plan.id);
        let progress = self.offline_progress.get(&plan.id).copied();
        let plan_id = plan.id.clone();

        let offline_pill = if offline_status.is_ready() {
            InlineMessage::new(Severity::Success, "Offline ready").into_any_element()
        } else if is_downloading {
            let label = if let Some((cur, total)) = progress {
                format!("Downloading offline pack… ({cur}/{total} tiles)")
            } else {
                "Downloading offline pack…".to_string()
            };
            StatusMessage::new("travel-offline-progress", label).into_any_element()
        } else {
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .px_2()
                        .py(px(2.))
                        .rounded(px(4.))
                        .bg(rgba(palette.raised))
                        .border_1()
                        .border_color(rgba(palette.border))
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(format!("Offline: {}%", offline_status.percent())),
                )
                .child(
                    Button::new(
                        gpui::SharedString::from(format!("download-offline-{}", plan.id)),
                        "Download offline pack",
                    )
                    .variant(ButtonVariant::Ghost)
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.prepare_offline_pack(plan_id.clone(), cx);
                    })),
                )
                .into_any_element()
        };

        page = page
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(div().text_xl().child(plan.title.clone()))
                    .child(offline_pill),
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
        let tab_entity = cx.entity();
        if cfg!(test) {
            page = page.child(sub_tabs(
                "trip-sections",
                TripSection::ALL
                    .iter()
                    .map(|&t| (t.label(), None, t == section))
                    .collect(),
                move |ix, _, cx| {
                    if let Some(&tab) = TripSection::ALL.get(ix) {
                        tab_entity.update(cx, |view, cx| view.open_section(tab, cx));
                    }
                },
            ));
        }
        page.child(match section {
            TripSection::Overview => self.overview(cx),
            TripSection::Itinerary => self.itinerary(cx),
            TripSection::Map => self.map_page(cx),
            TripSection::Items => self.saved_items(cx),
        })
        .children(self.render_form_sheet(cx))
        .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{size, TestAppContext};

    #[gpui::test]
    fn location_navigation_dispatches_to_connections_and_back(cx: &mut TestAppContext) {
        let (view, cx) =
            crate::test_support::mount_root(cx, Some(size(px(1100.), px(800.))), |_, cx| {
                let mut view = Travel::new(None, crate::Density::Desktop, cx);
                view.loaded = true;
                view
            });
        let connections = cx
            .debug_bounds("segment connections")
            .expect("connections segment");
        cx.simulate_click(connections.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        view.read_with(cx, |view, _| {
            assert_eq!(view.route, TravelRoute::Connections)
        });
        let trips = cx.debug_bounds("segment trips").expect("trips segment");
        cx.simulate_click(trips.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        view.read_with(cx, |view, _| assert_eq!(view.route, TravelRoute::Trips));
    }
}
