//! Appearance shared by the GPUI window and its native phone host.

use super::{AppearanceSettings, Palette, ThemeMode};

/// The page background as `0xRRGGBB` for the native phone cover.
pub fn phone_background(settings: &AppearanceSettings, dark: bool) -> u32 {
    let base = if dark {
        Palette::dark()
    } else {
        Palette::light()
    };
    settings.apply_to_palette(base, dark).window_tint >> 8
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
    fn phone_cover_uses_both_palettes_and_respects_the_appearance_override() {
        let mut settings = AppearanceSettings::default();
        assert_ne!(
            phone_background(&settings, false),
            phone_background(&settings, true)
        );
        settings.theme_mode = ThemeMode::System;
        assert_eq!(forced_dark(&settings), None);
        settings.theme_mode = ThemeMode::Dark;
        assert_eq!(forced_dark(&settings), Some(true));
        settings.theme_mode = ThemeMode::Light;
        assert_eq!(forced_dark(&settings), Some(false));
    }
}
