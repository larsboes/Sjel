//! The header: the area's name, the page's one line, its tab strip when it has more than one tab,
//! and the two window-wide actions.
//!
//! It reads the workspace and dispatches its actions, so the header holds no state of its own.

use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonGroup, ButtonVariants};
use gpui_kit::component::{IconName, Selectable, Sizable};
use gpui_kit::{div, prelude::*, px, rgb, AnyElement, Context, FontWeight, SharedString};
use sjel_gpui_core::area::Area;

use crate::route::Page;
use crate::theme::Palette;
use crate::workspace::Workspace;

pub fn render(view: &Workspace, palette: Palette, cx: &mut Context<Workspace>) -> impl IntoElement {
    let checked = view
        .health
        .age()
        .map(|elapsed| format!("checked {} ago", ago(elapsed)));
    let area = Area::of(view.page);
    let tabs = area
        .filter(|area| area.has_tabs())
        .map(|area| tab_strip(area, view.page, cx));
    // Settings › Updates says the same at length, so the pill stays out of Settings.
    let update_pill = view
        .update_info
        .as_ref()
        .filter(|info| info.is_update_available() && view.page != Page::Settings)
        .map(|info| {
            div().flex_none().mr_2().child(crate::update::header_pill(
                info,
                view.rebuild_state == crate::update::RebuildState::Building,
                palette,
                cx.entity(),
            ))
        });
    div()
        .flex()
        .flex_col()
        .gap_3()
        .px_8()
        .pt(px(44.))
        .pb_5()
        .child(
            div()
                .flex()
                .items_end()
                .justify_between()
                .gap_4()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_2xl()
                                .font_weight(FontWeight::SEMIBOLD)
                                .whitespace_nowrap()
                                .truncate()
                                .child(area.map_or(view.page.label(), Area::label)),
                        )
                        .child(
                            div()
                                .text_sm()
                                .truncate()
                                .text_color(rgb(palette.muted))
                                .child(subtitle(view)),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap_1()
                        .children(update_pill)
                        .children(checked.map(|label| {
                            div()
                                .text_xs()
                                .text_color(rgb(palette.amber))
                                .mr_2()
                                .child(label)
                        }))
                        .child(
                            Button::new("refresh")
                                .ghost()
                                .small()
                                .icon(IconName::RotateCw)
                                .loading(view.health.loading || view.agent.loading)
                                .tooltip("Refresh (⌘R)")
                                .on_click(cx.listener(|view, _, _, cx| {
                                    view.refresh_current_page(cx);
                                })),
                        )
                        .child(
                            Button::new("settings")
                                .ghost()
                                .small()
                                .icon(IconName::Settings)
                                .tooltip("Settings (⌘,)")
                                .on_click(
                                    cx.listener(|view, _, _, cx| view.open_settings(None, cx)),
                                ),
                        )
                        .child(
                            Button::new("open-in-dashboard")
                                .ghost()
                                .small()
                                .icon(IconName::ExternalLink)
                                .tooltip("Open in the dashboard (⌘D)")
                                .on_click(cx.listener(|view, _, window, cx| {
                                    view.open_dashboard(view.page.href().to_owned(), window, cx);
                                })),
                        ),
                ),
        )
        // Its own row, so eight tabs can never squeeze the name of the area they belong to.
        .children(tabs)
}

/// The area's tabs, one segment each, the open one selected. A click switches pages in memory:
/// nothing in an area is read again to show that its neighbour exists.
fn tab_strip(area: Area, current: Page, cx: &mut Context<Workspace>) -> AnyElement {
    let pages = area.pages();
    ButtonGroup::new("area-tabs")
        .outline()
        .small()
        .children(pages.iter().enumerate().map(|(ix, page)| {
            Button::new(("area-tab", ix))
                .label(page.label())
                .selected(*page == current)
        }))
        .on_click(cx.listener(move |view, picked: &Vec<usize>, _, cx| {
            if let Some(page) = picked.first().and_then(|ix| pages.get(*ix)) {
                view.go(*page, cx);
            }
        }))
        .into_any_element()
}

/// The one line under the title: what this page is for, or what it is looking at.
fn subtitle(view: &Workspace) -> SharedString {
    match view.page {
        // `/` and `/context` are one page, and the day block below carries its own date.
        Page::Home | Page::Context => {
            "What needs you now, what is urgent, and the day itself".into()
        }
        Page::Capabilities => view.endpoint(),
        Page::Projects => "Projects, tasks and notes, in one place".into(),
        Page::Ask => "Multi-turn capability session · ⌥Space from any app".into(),
        Page::Travel => "Trips, itinerary stops and maps".into(),
        Page::Feed => "What the collectors brought in, ranked for you".into(),
        Page::Inbox => "Captured, not sorted yet".into(),
        Page::Calendar => "Your days, from the calendar capability".into(),
        Page::Documents => "Personal documents, filing and review".into(),
        Page::Backup => "Backup targets, rehearsal verification, and attempt history".into(),
        Page::Finance => "Personal ledger, accounts and recurring subscriptions".into(),
        Page::People => "Contact registry and personal relationship profiles".into(),
        Page::Inventory => {
            "Household inventory, wishlist budget, and interior spatial layout".into()
        }
        Page::Software => "Apps on this Mac: size, last used, and what to keep".into(),
        Page::Import => "Bring a folder into Sjel, each file to its owner".into(),
        Page::Systems => {
            "This Mac now: temperature, power, disk, updates and the agent's reach".into()
        }
        Page::Packs => "Agent skills across installed harnesses, drift status, and sync".into(),
        Page::SelfModel => {
            "Sjel about Sjel: units, compile-time couplings, and live topology".into()
        }
        Page::Cv => "CV builder, profile variations, applications ledger, and LinkedIn".into(),
        Page::Media => "Exact-byte ingest gate, volume integrity ledger, and run inspection".into(),
        Page::Research => "Why Sjel exists, evidence behind it, and questions still open".into(),
        Page::Setup => {
            "What this machine still needs, primary replica, and companion devices".into()
        }
        Page::Settings => view.settings_section.summary().into(),
    }
}

fn ago(elapsed: Duration) -> String {
    match elapsed.as_secs() {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m", s / 60),
        s => format!("{}h", s / 3600),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn age_reads_in_the_largest_unit_that_fits() {
        assert_eq!(ago(Duration::ZERO), "0s");
        assert_eq!(ago(Duration::from_secs(59)), "59s");
        assert_eq!(ago(Duration::from_secs(60)), "1m");
        assert_eq!(ago(Duration::from_secs(7_200)), "2h");
    }
}
