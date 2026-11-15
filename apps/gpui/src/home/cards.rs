use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::Sizable;
use gpui_kit::{div, prelude::*, px, rgb, rgba, AnyElement, Context, SharedString};

use crate::route::Page;
use crate::workspace::Workspace;
use sjel_gpui_ui::primitives::{list, row, section};
use sjel_gpui_ui::theme::Palette;

/// Prompts an agent drafted for you to paste elsewhere. Copy empties the card; none draws nothing.
pub fn prompt_outbox(
    view: &Workspace,
    palette: Palette,
    cx: &mut Context<Workspace>,
) -> Option<AnyElement> {
    if view.prompt_drafts.is_empty() {
        return None;
    }
    let rows = view
        .prompt_drafts
        .iter()
        .map(|draft| {
            let id = draft.id;
            row()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(div().truncate().text_sm().child(draft.title.clone()))
                        .child(
                            div()
                                .truncate()
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child(draft.body.lines().next().unwrap_or_default().to_string()),
                        ),
                )
                .child(
                    Button::new(SharedString::from(format!("copy-prompt-{id}")))
                        .primary()
                        .small()
                        .label("Copy")
                        .tooltip("Copy the prompt and remove it from this list")
                        .on_click(
                            cx.listener(move |view, _, _, cx| view.copy_prompt_draft(id, cx)),
                        ),
                )
                .into_any_element()
        })
        .collect();
    Some(section("Prompts for you", list(rows, palette), palette).into_any_element())
}

/// The trip that matters now: full map card when active or within 48h (or Travel mode), compact strip otherwise.
/// Returns `None` when there is no upcoming trip (unless Travel mode is active).
pub fn travel_card(
    view: &Workspace,
    palette: Palette,
    cx: &mut Context<Workspace>,
) -> Option<AnyElement> {
    if !view.home_prefs.effective_show_travel() {
        return None;
    }
    let force_map = view.home_prefs.force_expanded_map();
    let large_map = view.home_prefs.large_map;
    let days_away = view.travel.read(cx).home_trip_days_away();
    let trip_info = view
        .travel
        .update(cx, |travel, cx| travel.home_next_trip(palette, cx));
    let imminent = force_map || days_away.is_some_and(|days| days <= 2);

    let apple_maps_button = Button::new("home-apple-maps")
        .ghost()
        .small()
        .label("Apple Maps")
        .tooltip("Open interactive native Apple Maps window")
        .on_click(cx.listener(|view, _, _, cx| {
            view.travel
                .update(cx, |travel, cx| travel.open_apple_maps(cx));
        }));

    let size_button = Button::new("home-map-size")
        .ghost()
        .small()
        .label(if large_map { "Compact" } else { "Larger" })
        .tooltip("Toggle map size on Today")
        .on_click(cx.listener(|view, _, _, cx| {
            view.home_prefs.large_map = !view.home_prefs.large_map;
            let _ = view.home_prefs.save();
            cx.notify();
        }));

    let map_button = Button::new("home-open-map")
        .ghost()
        .small()
        .label(if imminent { "Expand ↗" } else { "Map" })
        .tooltip("Open full map in Travel")
        .on_click(cx.listener(|view, _, _, cx| {
            view.travel.update(cx, |travel, cx| {
                travel.set_compact_map(false);
                travel.open_route(sjel_gpui_core::travel::TravelRoute::Map, cx);
            });
            view.go(Page::Travel, cx);
        }));

    let Some((trip, plan_id)) = trip_info else {
        if !force_map {
            return None;
        }
        let map = view.travel.update(cx, |travel, cx| {
            travel.set_home_map_large(large_map);
            travel.home_map(palette, cx)
        });
        let body = div()
            .flex()
            .flex_col()
            .gap_3()
            .rounded(px(16.))
            .bg(rgba(palette.surface))
            .border_1()
            .border_color(rgba(palette.border))
            .p_3()
            .child(map)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap_1()
                    .child(apple_maps_button)
                    .child(size_button)
                    .child(map_button),
            );
        return Some(section("Location", body.into_any_element(), palette).into_any_element());
    };

    let open_trip_button = plan_id.map(|id| {
        Button::new("home-open-trip")
            .ghost()
            .small()
            .label("Open trip")
            .on_click(cx.listener(move |view, _, _, cx| {
                let route = sjel_gpui_core::travel::TravelRoute::Trip {
                    id: id.clone(),
                    section: sjel_gpui_core::travel::TripSection::Overview,
                };
                view.travel
                    .update(cx, |travel, cx| travel.open_route(route, cx));
                view.go(Page::Travel, cx);
            }))
    });

    if imminent {
        let map = view.travel.update(cx, |travel, cx| {
            travel.set_home_map_large(large_map);
            travel.home_map(palette, cx)
        });
        let body = div()
            .flex()
            .flex_col()
            .gap_3()
            .rounded(px(16.))
            .bg(rgba(palette.surface))
            .border_1()
            .border_color(rgba(palette.border))
            .p_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .px_1()
                    .child(trip)
                    .children(open_trip_button),
            )
            .child(map)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap_1()
                    .child(apple_maps_button)
                    .child(size_button)
                    .child(map_button),
            );

        Some(section("Active & upcoming trip", body.into_any_element(), palette).into_any_element())
    } else {
        let body = div()
            .flex()
            .flex_col()
            .gap_2()
            .rounded(px(12.))
            .bg(rgba(palette.surface))
            .border_1()
            .border_color(rgba(palette.border))
            .p_3()
            .child(div().w_full().min_w_0().child(trip))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap_1()
                    .child(map_button)
                    .children(open_trip_button),
            );

        Some(section("Next trip", body.into_any_element(), palette).into_any_element())
    }
}

/// The day on screen: the date, its three counts, and the timeline, from the Today view.
pub fn today(view: &Workspace, palette: Palette, cx: &mut Context<Workspace>) -> AnyElement {
    let show_timeline = view.home_prefs.show_timeline;
    let show_spending = view.home_prefs.effective_show_spending();
    view.context_page.update(cx, |page, cx| {
        page.set_sections(show_timeline, show_spending);
        page.set_palette(palette, cx);
    });
    view.context_page.clone().into_any_element()
}
