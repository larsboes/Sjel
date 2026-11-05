//! The sidebar: the nine areas, the soundscape player, the palette hint, and what the shell
//! answered.
//!
//! It draws the workspace's state and owns none of it. Every click calls a `Workspace` method, so
//! the sidebar decides layout and wording only, and its geometry lives here so the glass panel
//! that AppKit paints under it cannot drift from the column GPUI reserves.

pub mod nav;
pub mod status;
pub mod types;

use gpui_kit::{div, prelude::*, px, rgba, Context, IntoElement, Window};

use crate::workspace::Workspace;
use sjel_gpui_ui::theme::Palette;

pub use nav::{nav, palette_hint, settings_row, soundscape};
pub use status::{footer, status, storage};
pub use types::{GLASS_INSET, WIDTH};

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
                .bg(rgba(palette.sidebar_surface))
                .border_r_1()
                .border_color(rgba(palette.border))
        })
        // Nine rows, so the list never scrolls: every area is one keystroke away and none of them
        // hides below the fold. The player, Search, Settings and the footer sit under it.
        .child(
            div()
                .id("sidebar-nav")
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .children(nav(view, palette, cx)),
        )
        .child(soundscape(view, palette, window, cx))
        .child(palette_hint(palette, window))
        .child(settings_row(view, palette, window, cx))
        .child(footer(view, palette, cx))
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::types::uptime;
    use crate::data::Remote;
    use crate::shell::{NodeHealth, ShellError};
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
