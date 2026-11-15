//! Gives Ely's controls Sjel's palette, so an Ely control matches the page around it.
//!
//! A field with a Sjel token takes that token. Every other field keeps Ely's own value for the
//! mode (`docs/plans/gpui.md` §11, Components from Ely).
use gpui::{rgb, rgba, App, Hsla};

use super::{AppearanceSettings, Palette};
use crate::ely::theme::{Mode, Palette as ElyPalette, Theme};

/// A Sjel color as an `Hsla`: `0x00RRGGBB` tones and `0xRRGGBBAA` surfaces, as [`super::tint`] reads them.
fn hsla(color: u32) -> Hsla {
    if color > 0x00ff_ffff {
        rgba(color).into()
    } else {
        rgb(color).into()
    }
}

/// Ely's palette for one mode, with Sjel's tokens and the owner's accent.
pub fn palette(appearance: AppearanceSettings, dark: bool) -> ElyPalette {
    let base = if dark {
        Palette::dark()
    } else {
        Palette::light()
    };
    let mut ely = if dark {
        ElyPalette::dark(false)
    } else {
        ElyPalette::light(false)
    };
    let accent = hsla(appearance.accent.color_rgb(dark));
    ely.bg = hsla(base.surface);
    ely.surface = hsla(base.surface);
    ely.sunken = hsla(base.panel_surface);
    ely.overlay = hsla(base.sidebar_surface);
    ely.hover = hsla(base.raised);
    ely.border = hsla(base.border);
    ely.fg = hsla(base.text);
    ely.fg_muted = hsla(base.muted);
    ely.accent = accent;
    ely.accent_hover = accent.opacity(0.9);
    ely.on_accent = hsla(0xffffff);
    ely.focus = accent;
    ely.link = accent;
    ely.selection = Hsla {
        a: ely.selection.a,
        ..accent
    };
    ely.success = hsla(base.green);
    ely.warning = hsla(base.amber);
    ely.danger = hsla(base.red);
    ely
}

/// Sets Ely's palettes for both modes and switches Ely to the mode shown, at once.
pub fn sync(appearance: AppearanceSettings, dark: bool, cx: &mut App) {
    Theme::set_palette(Mode::Light, Some(palette(appearance, false)), cx);
    Theme::set_palette(Mode::Dark, Some(palette(appearance, true)), cx);
    Theme::set_mode_now(if dark { Mode::Dark } else { Mode::Light }, cx);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sjel_tokens_reach_ely_and_the_rest_stay_elys() {
        let settings = AppearanceSettings::default();
        for dark in [false, true] {
            let (sjel, own) = if dark {
                (Palette::dark(), ElyPalette::dark(false))
            } else {
                (Palette::light(), ElyPalette::light(false))
            };
            let ely = palette(settings, dark);
            assert_eq!(ely.fg, hsla(sjel.text));
            assert_eq!(ely.border, hsla(sjel.border));
            assert_eq!(ely.danger, hsla(sjel.red));
            assert_eq!(ely.accent, hsla(settings.accent.color_rgb(dark)));
            assert_eq!(ely.selection.a, own.selection.a);
            assert_eq!(ely.active, own.active);
            assert_eq!(ely.chart, own.chart);
            assert_eq!(ely.syntax, own.syntax);
        }
    }

    #[test]
    fn the_owners_accent_reaches_ely() {
        let mut settings = AppearanceSettings::default();
        for accent in super::super::AccentColor::ALL {
            settings.accent = accent;
            assert_eq!(palette(settings, true).focus, hsla(accent.color_rgb(true)));
        }
    }
}
