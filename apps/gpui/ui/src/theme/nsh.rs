//! The appearance settings as NSH theme overrides, so NSH screens and GPUI pages draw from one
//! palette (`docs/plans/gpui.md` §11).
//!
//! NSH takes a shadcn/ui theme file (`cssVars` with `light` and `dark` tokens) from the embedding
//! app (`nsh_platform_ios::AppView::set_theme_overrides`). Light or dark itself is the view's
//! trait collection, which [`forced_dark`] decides.

use super::{AppearanceSettings, Palette, ThemeMode};

fn hex_rgb(tone: u32) -> String {
    format!("#{:06x}", tone & 0x00ff_ffff)
}

/// A translucent `0xRRGGBBAA` surface laid over the opaque page `0xRRGGBB`, as `0xRRGGBB`: NSH
/// tokens are opaque, and the palette's surfaces are tints of the page under them.
fn over(surface: u32, page: u32) -> u32 {
    let alpha = surface & 0xff;
    let mix = |shift: u32| {
        let top = (surface >> (shift + 8)) & 0xff;
        let bottom = (page >> shift) & 0xff;
        (top * alpha + bottom * (255 - alpha) + 127) / 255
    };
    (mix(16) << 16) | (mix(8) << 8) | mix(0)
}

fn tokens(palette: Palette) -> serde_json::Value {
    let page = palette.window_tint >> 8;
    let layer = |surface: u32| hex_rgb(over(surface, page));
    serde_json::json!({
        "background": hex_rgb(page),
        "foreground": hex_rgb(palette.text),
        "card": layer(palette.surface),
        "card-foreground": hex_rgb(palette.text),
        "primary": hex_rgb(palette.accent),
        "primary-foreground": "#ffffff",
        "muted": layer(palette.raised),
        "muted-foreground": hex_rgb(palette.muted),
        "border": layer(palette.border),
        "input": layer(palette.border),
        "ring": hex_rgb(palette.accent),
        "destructive": hex_rgb(palette.red),
    })
}

/// The overrides file for `settings`, light and dark.
pub fn nsh_theme_overrides(settings: &AppearanceSettings) -> String {
    let light = settings.apply_to_palette(Palette::light(), false);
    let dark = settings.apply_to_palette(Palette::dark(), true);
    serde_json::json!({ "cssVars": { "light": tokens(light), "dark": tokens(dark) } }).to_string()
}

/// The page background NSH screens draw on, as `0xRRGGBB`, for the bands around them.
pub fn nsh_background(settings: &AppearanceSettings, dark: bool) -> u32 {
    let base = if dark {
        Palette::dark()
    } else {
        Palette::light()
    };
    settings.apply_to_palette(base, dark).window_tint >> 8
}

/// The terminal surface blended over the phone page, using the shared palette.
pub fn nsh_terminal_background(settings: &AppearanceSettings, dark: bool) -> u32 {
    let palette = settings.apply_to_palette(
        if dark {
            Palette::dark()
        } else {
            Palette::light()
        },
        dark,
    );
    over(palette.terminal_surface, palette.window_tint >> 8)
}

/// `Some(true)` or `Some(false)` when the settings force dark or light; `None` follows the system.
pub fn forced_dark(settings: &AppearanceSettings) -> Option<bool> {
    match settings.theme_mode {
        ThemeMode::System => None,
        ThemeMode::Dark => Some(true),
        ThemeMode::Light => Some(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_translucent_surface_blends_into_the_page() {
        assert_eq!(
            over(0xffffff00, 0x101418),
            0x101418,
            "transparent shows the page"
        );
        assert_eq!(over(0xffffffff, 0x101418), 0xffffff, "opaque shows itself");
        assert_eq!(over(0xffffff1a, 0x000000), 0x1a1a1a);
    }

    #[test]
    fn the_overrides_carry_the_accent_and_both_schemes() {
        let mut settings = AppearanceSettings::default();
        let json: serde_json::Value =
            serde_json::from_str(&nsh_theme_overrides(&settings)).unwrap();
        let light = &json["cssVars"]["light"];
        let dark = &json["cssVars"]["dark"];
        assert!(dark["primary"].as_str().unwrap().starts_with('#'));
        assert!(light["background"].as_str().unwrap().starts_with('#'));
        assert_ne!(light["background"], dark["background"]);
        assert_eq!(forced_dark(&settings), None);
        settings.theme_mode = ThemeMode::Dark;
        assert_eq!(forced_dark(&settings), Some(true));
        let before = light["primary"].clone();
        settings.accent = *super::super::AccentColor::ALL
            .iter()
            .find(|a| **a != settings.accent)
            .unwrap();
        let after: serde_json::Value =
            serde_json::from_str(&nsh_theme_overrides(&settings)).unwrap();
        assert_ne!(after["cssVars"]["light"]["primary"], before);
    }
}
