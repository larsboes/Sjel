//! The sidebar: navigation, the soundscape player, the palette hint, and what the shell answered.
//!
//! It draws the workspace's state and owns none of it. Every click calls a `Workspace` method, so
//! the sidebar decides layout and wording only, and its geometry lives here so the glass panel
//! that AppKit paints under it cannot drift from the column GPUI reserves.

use gpui_kit::component::kbd::Kbd;
use gpui_kit::{div, prelude::*, px, rgb, rgba, Action, AnyElement, Context, SharedString, Window};

use crate::data::Remote;
use crate::route::Page;
use crate::shell::NodeHealth;
use crate::theme::Palette;
use crate::workspace::{GoAsk, GoCapabilities, GoHome, GoProjects, OpenPalette, Workspace};

/// The column the sidebar occupies, and the gap the glass panel leaves to the window edge.
pub const WIDTH: f32 = 208.;
pub const GLASS_INSET: f32 = 8.;

pub fn render(
    view: &Workspace,
    palette: Palette,
    glass: bool,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
    div()
        .size_full()
        .flex()
        .flex_col()
        .gap_1()
        // Clears the transparent titlebar's traffic lights; the glass inset already moves it down.
        .pt(px(if glass { 52. - GLASS_INSET } else { 52. }))
        .px_3()
        .pb_4()
        .when(!glass, |sidebar| {
            sidebar
                .bg(rgba(palette.raised))
                .border_r_1()
                .border_color(rgba(palette.border))
        })
        .children(nav(view, palette, window, cx))
        .child(div().flex_1())
        .child(soundscape(view, palette, cx))
        .child(palette_hint(palette, window))
        .child(footer(view, palette))
}

/// One row per native page, each with the shortcut that opens it.
fn nav(
    view: &Workspace,
    palette: Palette,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> Vec<AnyElement> {
    Page::ALL
        .into_iter()
        .map(move |page| {
            let selected = page == view.page;
            let shortcut = match page {
                Page::Home => Kbd::binding_for_action(&GoHome, None, window),
                Page::Capabilities => Kbd::binding_for_action(&GoCapabilities, None, window),
                Page::Projects => Kbd::binding_for_action(&GoProjects, None, window),
                Page::Ask => Kbd::binding_for_action(&GoAsk, None, window),
            };
            div()
                .id(page.href())
                .flex()
                .items_center()
                .justify_between()
                .px_3()
                .py(px(6.))
                .rounded(px(7.))
                .when(selected, |row| row.bg(rgba(palette.raised)))
                .text_color(rgb(if selected {
                    palette.text
                } else {
                    palette.muted
                }))
                .cursor_pointer()
                .hover(move |style| style.bg(rgba(palette.raised)).text_color(rgb(palette.text)))
                .on_click(cx.listener(move |view, _, _, cx| view.go(page, cx)))
                .child(page.label())
                .children(shortcut.map(|kbd| kbd.appearance(false)))
                .into_any_element()
        })
        .collect()
}

/// The mini player, shown only while the soundscape capability answers or has answered.
fn soundscape(view: &Workspace, palette: Palette, cx: &mut Context<Workspace>) -> impl IntoElement {
    let state = view.soundscape.value.as_ref();
    let is_up = view
        .health
        .value
        .as_ref()
        .map(|h| {
            h.capabilities
                .get("soundscape")
                .map(|c| c.up)
                .unwrap_or(false)
        })
        .unwrap_or(false);

    if !is_up && state.is_none() {
        return div().into_any_element();
    }

    let playing = state.map(|s| s.playing).unwrap_or(false);
    let preset = state.map(|s| s.preset.as_str()).unwrap_or("edm");

    div()
        .id("soundscape-mini-player")
        .mx_1()
        .mb_1()
        .px_2()
        .py(px(5.))
        .rounded(px(7.))
        .bg(rgba(palette.raised))
        .flex()
        .items_center()
        .justify_between()
        .gap_2()
        .text_xs()
        .child(
            div()
                .flex()
                .items_center()
                .gap_1()
                .min_w_0()
                .child(equalizer_indicator(playing, palette))
                .child(
                    div()
                        .id("soundscape-preset-btn")
                        .cursor_pointer()
                        .truncate()
                        .text_color(rgb(palette.text))
                        .hover(move |s| s.text_color(rgb(palette.accent)))
                        .on_click(cx.listener(|view, _, window, cx| {
                            view.cycle_soundscape_preset(window, cx);
                        }))
                        .child(format!("♪ {preset}")),
                ),
        )
        .child(
            div()
                .id("soundscape-toggle-btn")
                .flex_none()
                .px_2()
                .py(px(2.))
                .rounded(px(4.))
                .bg(rgba(if playing {
                    palette.accent << 8 | 0x24
                } else {
                    palette.raised
                }))
                .text_color(rgb(if playing {
                    palette.accent
                } else {
                    palette.muted
                }))
                .cursor_pointer()
                .hover(move |s| s.text_color(rgb(palette.text)))
                .on_click(cx.listener(move |view, _, window, cx| {
                    view.toggle_soundscape(!playing, window, cx);
                }))
                .child(if playing { "Pause" } else { "Play" }),
        )
        .into_any_element()
}

fn equalizer_indicator(playing: bool, palette: Palette) -> impl IntoElement {
    if !playing {
        return div()
            .flex_none()
            .size(px(6.))
            .rounded_full()
            .bg(rgb(palette.muted))
            .into_any_element();
    }
    div()
        .flex_none()
        .flex()
        .items_end()
        .gap(px(1.5))
        .h(px(8.))
        .w(px(8.))
        .child(
            div()
                .w(px(1.5))
                .h(px(5.))
                .rounded(px(0.5))
                .bg(rgb(palette.green)),
        )
        .child(
            div()
                .w(px(1.5))
                .h(px(8.))
                .rounded(px(0.5))
                .bg(rgb(palette.accent)),
        )
        .child(
            div()
                .w(px(1.5))
                .h(px(4.))
                .rounded(px(0.5))
                .bg(rgb(palette.green)),
        )
        .into_any_element()
}

fn palette_hint(palette: Palette, window: &mut Window) -> impl IntoElement {
    div()
        .id("palette-hint")
        .flex()
        .items_center()
        .justify_between()
        .px_3()
        .py(px(6.))
        .rounded(px(7.))
        .text_sm()
        .text_color(rgb(palette.muted))
        .cursor_pointer()
        .hover(move |style| style.bg(rgba(palette.raised)))
        .on_click(|_, window, cx| window.dispatch_action(OpenPalette.boxed_clone(), cx))
        .child("Search")
        .children(Kbd::binding_for_action(&OpenPalette, None, window).map(|k| k.appearance(false)))
}

/// The shell's own health, then the path the app reads it through.
fn footer(view: &Workspace, palette: Palette) -> impl IntoElement {
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
                .flex()
                .items_center()
                .gap_2()
                .min_w_0()
                .child(div().flex_none().size(px(7.)).rounded_full().bg(rgb(color)))
                .child(div().truncate().child(label)),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_1()
                .flex_none()
                .child(div().size(px(6.)).rounded_full().bg(rgb(backend_color)))
                .child(div().text_xs().child(backend)),
        )
}

/// What the status dot and its line say: refused, degraded, settled, or still connecting. The
/// uptime counts up between frames, so it adds the age of the answer it came with.
fn status(health: &Remote<NodeHealth>, palette: Palette) -> (u32, SharedString) {
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
fn storage(direct_db: bool, palette: Palette) -> (&'static str, u32) {
    if direct_db {
        ("libsql", palette.accent)
    } else {
        ("http", palette.muted)
    }
}

fn uptime(seconds: u64) -> String {
    match seconds {
        s if s < 3600 => format!("{}m", s / 60),
        s if s < 86_400 => format!("{}h", s / 3600),
        s => format!("{}d {}h", s / 86_400, s % 86_400 / 3600),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::ShellError;
    use gpui_kit::WindowAppearance;

    fn palette() -> Palette {
        Palette::for_appearance(WindowAppearance::Dark)
    }

    fn health(json: &str) -> Remote<NodeHealth> {
        Remote {
            value: Some(serde_json::from_str(json).expect("health fixture")),
            ..Remote::default()
        }
    }

    #[test]
    fn a_refused_shell_is_red_and_says_why() {
        let health = Remote {
            value: None,
            error: Some(ShellError::Refused(502, Some("no connection".into()))),
            ..Remote::default()
        };
        let (color, label) = status(&health, palette());
        assert_eq!(color, palette().red);
        assert!(label.contains("no connection"), "{label}");
    }

    #[test]
    fn a_capability_that_is_down_turns_the_dot_amber() {
        let health =
            health(r#"{"version":"v9","uptime_seconds":60,"capabilities":{"a":{"up":false}}}"#);
        let (color, label) = status(&health, palette());
        assert_eq!(color, palette().amber);
        assert!(label.contains("v9"), "{label}");
    }

    #[test]
    fn every_capability_up_is_green() {
        let health =
            health(r#"{"version":"v9","uptime_seconds":60,"capabilities":{"a":{"up":true}}}"#);
        assert_eq!(status(&health, palette()).0, palette().green);
    }

    #[test]
    fn before_the_first_answer_the_dot_says_connecting() {
        let (color, label) = status(&Remote::default(), palette());
        assert_eq!(color, palette().muted);
        assert_eq!(label, "Connecting…");
    }

    #[test]
    fn the_storage_dot_names_the_read_path() {
        let palette = palette();
        assert_eq!(storage(true, palette), ("libsql", palette.accent));
        assert_eq!(storage(false, palette), ("http", palette.muted));
    }

    #[test]
    fn uptime_reads_in_the_largest_unit_that_fits() {
        assert_eq!(uptime(30), "0m");
        assert_eq!(uptime(3_599), "59m");
        assert_eq!(uptime(3_600), "1h");
        assert_eq!(uptime(90_000), "1d 1h");
    }
}
