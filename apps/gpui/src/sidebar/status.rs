use gpui_kit::{div, prelude::*, px, rgb, Context, IntoElement, SharedString};

use super::types::uptime;
use crate::data::Remote;
use crate::route::Page;
use crate::shell::NodeHealth;
use crate::workspace::Workspace;
use sjel_gpui_ui::theme::Palette;

/// The shell's own health, then the path the app reads it through.
pub fn footer(view: &Workspace, palette: Palette, cx: &mut Context<Workspace>) -> impl IntoElement {
    let (color, label) = status(&view.health, palette);
    let (backend, backend_color) = storage(view.has_direct_db(), palette);
    div()
        .flex()
        .items_center()
        .justify_between()
        .px_3()
        .pt_2()
        .text_xs()
        .text_color(rgb(palette.muted))
        .child(
            div()
                .id("sidebar-footer-health")
                .flex()
                .items_center()
                .gap_2()
                .min_w_0()
                .cursor_pointer()
                .hover(move |s| s.text_color(rgb(palette.text)))
                .on_click(cx.listener(|view, _, _, cx| view.go(Page::Capabilities, cx)))
                .child(div().flex_none().size(px(7.)).rounded_full().bg(rgb(color)))
                .child(div().truncate().child(label)),
        )
        .child(
            div()
                .id("sidebar-footer-storage")
                .flex()
                .items_center()
                .gap_1()
                .flex_none()
                .cursor_pointer()
                .hover(move |s| s.text_color(rgb(palette.text)))
                .on_click(cx.listener(|view, _, _, cx| view.go(Page::Setup, cx)))
                .child(div().size(px(6.)).rounded_full().bg(rgb(backend_color)))
                .child(div().text_xs().child(backend)),
        )
}

/// What the status dot and its line say: refused, degraded, settled, or still connecting. The
/// uptime counts up between frames, so it adds the age of the answer it came with.
pub fn status(health: &Remote<NodeHealth>, palette: Palette) -> (u32, SharedString) {
    match (&health.value, &health.error) {
        (_, Some(error)) => (palette.red, SharedString::from(error.to_string())),
        (Some(node), None) => (
            if node.down().is_empty() {
                palette.green
            } else {
                palette.amber
            },
            format!(
                "{} · up {}",
                node.version,
                uptime(node.uptime_seconds + health.age_of_value())
            )
            .into(),
        ),
        (None, None) => (palette.muted, "Connecting…".into()),
    }
}

/// The read path in use, and a dot that says which one it is.
pub fn storage(direct_db: bool, palette: Palette) -> (&'static str, u32) {
    if direct_db {
        ("libsql", palette.accent)
    } else {
        ("http", palette.muted)
    }
}
