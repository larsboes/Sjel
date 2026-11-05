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

/// The trip that matters now, together with the location map: contextual travel card.
pub fn travel_card(view: &Workspace, palette: Palette, cx: &mut Context<Workspace>) -> AnyElement {
    let trip_info = view
        .travel
        .update(cx, |travel, cx| travel.home_next_trip(palette, cx));
    let map = view
        .travel
        .update(cx, |travel, cx| travel.home_map(palette, cx));

    if let Some((trip, plan_id)) = trip_info {
        let mut trip_header = div()
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .px_1()
            .child(trip);
        if let Some(id) = plan_id {
            trip_header = trip_header.child(
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
                    })),
            );
        }

        let body = div()
            .flex()
            .flex_col()
            .gap_3()
            .rounded(px(16.))
            .bg(rgba(palette.surface))
            .border_1()
            .border_color(rgba(palette.border))
            .p_3()
            .child(trip_header)
            .child(map)
            .child(
                div().flex().justify_end().child(
                    Button::new("open-location")
                        .ghost()
                        .small()
                        .label("Open Location")
                        .on_click(cx.listener(|view, _, _, cx| view.go(Page::Travel, cx))),
                ),
            );

        section("Next trip & Location", body.into_any_element(), palette).into_any_element()
    } else {
        let body = div()
            .id("home-compact-location")
            .flex()
            .flex_col()
            .gap_3()
            .rounded(px(14.))
            .bg(rgba(palette.surface))
            .border_1()
            .border_color(rgba(palette.border))
            .p_3()
            .cursor_pointer()
            .hover(move |s| s.border_color(rgba(palette.accent)))
            .on_click(cx.listener(|view, _, _, cx| view.go(Page::Travel, cx)))
            .child(map)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child("No active trip · Click to open Location & Trips"),
                    )
                    .child(
                        Button::new("open-location")
                            .ghost()
                            .small()
                            .label("Open Location →")
                            .on_click(cx.listener(|view, _, _, cx| view.go(Page::Travel, cx))),
                    ),
            );

        section("Location", body.into_any_element(), palette).into_any_element()
    }
}

/// The day on screen: the date, its three counts, and the timeline, from the Today view.
pub fn today(view: &Workspace, palette: Palette, cx: &mut Context<Workspace>) -> AnyElement {
    view.context_page
        .update(cx, |page, cx| page.set_palette(palette, cx));
    view.context_page.clone().into_any_element()
}
