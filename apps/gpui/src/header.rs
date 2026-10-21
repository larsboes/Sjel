//! The page header: the page's name, one line about it, and the two window-wide actions.
//!
//! It reads the workspace and dispatches its actions, so the header holds no state of its own.

use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{IconName, Sizable};
use gpui_kit::{div, prelude::*, px, rgb, Action, FontWeight, SharedString};

use crate::route::Page;
use crate::theme::Palette;
use crate::workspace::{OpenInDashboard, Refresh, Workspace};

pub fn render(view: &Workspace, palette: Palette) -> impl IntoElement {
    let checked = view
        .health
        .age()
        .map(|elapsed| format!("checked {} ago", ago(elapsed)));
    div()
        .flex()
        .items_end()
        .justify_between()
        .px_8()
        .pt(px(44.))
        .pb_5()
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_2xl()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(view.page.label()),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(palette.muted))
                        .child(subtitle(view)),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_1()
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
                        .loading(view.health.loading || view.agent.loading || view.calendar.loading)
                        .tooltip("Refresh (⌘R)")
                        .on_click(|_, window, cx| {
                            window.dispatch_action(Refresh.boxed_clone(), cx)
                        }),
                )
                .child(
                    Button::new("open-in-dashboard")
                        .ghost()
                        .small()
                        .icon(IconName::ExternalLink)
                        .tooltip("Open in the dashboard (⌘D)")
                        .on_click(|_, window, cx| {
                            window.dispatch_action(OpenInDashboard.boxed_clone(), cx)
                        }),
                ),
        )
}

/// The one line under the title: what this page is for, or what it is looking at.
fn subtitle(view: &Workspace) -> SharedString {
    match view.page {
        Page::Home => chrono::Local::now().format("%A, %B %-d").to_string().into(),
        Page::Capabilities => view.endpoint(),
        Page::Projects => "Projects, tasks and notes, in one place".into(),
        Page::Ask => "Multi-turn capability session · ⌥Space from any app".into(),
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
