//! Travel route dispatcher and section navigation.
use super::Travel;
use gpui::{div, prelude::*, px, rgb, rgba, AnyElement, ClipboardItem, Context};
use gpui_kit::component::button::ButtonVariants;
use gpui_kit::component::Disableable;
use sjel_gpui_core::travel::{TravelRoute, TripSection};

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
        let offline_status = plan.check_offline_readiness();
        let is_downloading = self.offline_downloading.contains(&plan.id);
        let progress = self.offline_progress.get(&plan.id).copied();
        let plan_id = plan.id.clone();

        let offline_pill = if offline_status.is_ready() {
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
                .child("✓ Offline ready")
                .into_any_element()
        } else if is_downloading {
            let label = if let Some((cur, total)) = progress {
                format!("Downloading offline pack… ({cur}/{total} tiles)")
            } else {
                "Downloading offline pack…".to_string()
            };
            div()
                .flex()
                .items_center()
                .gap_1()
                .px_2()
                .py(px(2.))
                .rounded(px(4.))
                .bg(rgba(crate::theme::tint(palette.accent, 0x18)))
                .border_1()
                .border_color(rgba(crate::theme::tint(palette.accent, 0x35)))
                .text_xs()
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(rgb(palette.accent))
                .child(label)
                .into_any_element()
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
                    self.button(gpui::SharedString::from(format!(
                        "download-offline-{}",
                        plan.id
                    )))
                    .ghost()
                    .label("Download offline pack")
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
        let mut tabs = div().flex().flex_wrap().gap_1();
        for tab in TripSection::ALL {
            tabs = tabs.child(
                self.button(tab.path())
                    .label(tab.label())
                    .when(tab == section, |button| button.primary())
                    .when(tab != section, |button| button.ghost())
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
}
