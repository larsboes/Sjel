//! Semantic colors, design tokens, and density rules for desktop and mobile GPUI surfaces.
//!
//! A tone says what a value means; nothing here is decorative (`Packs/design/skills/ui-craftsmanship`, rule 1).

pub mod density;
pub mod nsh;
pub mod palette;
pub mod settings;
pub mod terminal;

pub use density::*;
pub use nsh::{forced_dark, nsh_background, nsh_theme_overrides};
pub use palette::*;
pub use settings::*;
pub use terminal::*;

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::px;
    use gpui_kit::WindowAppearance;

    #[test]
    fn soundscape_settings_keep_their_shape() {
        let prefs = SoundscapePrefs {
            mixer: true,
            preset: "lofi".to_owned(),
            volume: 0.3,
        };
        let json = serde_json::to_string(&prefs).expect("serialize");
        assert_eq!(
            serde_json::from_str::<SoundscapePrefs>(&json).expect("read"),
            prefs
        );
        assert!(serde_json::from_str::<SoundscapePrefs>("{").is_err());
        let default = SoundscapePrefs::default();
        assert_eq!(default.preset, "focus");
        assert!(default.volume > 0. && default.volume <= 1.);
        let older = r#"{"open":true,"mixer":false,"preset":"lofi","volume":0.4}"#;
        let read: SoundscapePrefs = serde_json::from_str(older).expect("an older file reads");
        assert_eq!(read.preset, "lofi");
    }

    #[test]
    fn the_terminal_background_is_its_tone_at_its_opacity() {
        let settings = AppearanceSettings {
            terminal_tone: BackgroundTone::ObsidianNoir,
            terminal_opacity: WindowOpacity::Translucent,
            ..AppearanceSettings::default()
        };
        let palette = settings.apply_to_palette(Palette::dark(), true);
        assert_eq!(palette.terminal_surface, 0x090a0f8c);
        let palette = AppearanceSettings::default().apply_to_palette(Palette::light(), false);
        assert_eq!(palette.terminal_surface, tint(MONOKAI.background, 0xff));
    }

    #[test]
    fn a_pinch_steps_through_the_sizes_on_offer() {
        assert_eq!(step_terminal_font_size(13, 1), 14);
        assert_eq!(step_terminal_font_size(13, -2), 11);
        assert_eq!(step_terminal_font_size(16, 1), 16);
        assert_eq!(step_terminal_font_size(11, -1), 11);
        assert_eq!(step_terminal_font_size(15, 0), 16);
    }

    #[test]
    fn the_terminal_font_resolves_to_what_is_installed() {
        let families = vec!["Menlo".to_owned(), "Hack Nerd Font Mono".to_owned()];
        assert_eq!(
            TerminalFont::Auto.resolve(&families, "Lilex"),
            "Hack Nerd Font Mono"
        );
        assert_eq!(TerminalFont::Auto.resolve(&[], "Lilex"), "Lilex");
        assert_eq!(TerminalFont::Menlo.resolve(&families, "Lilex"), "Menlo");
        assert_eq!(TerminalFont::SfMono.resolve(&families, "Lilex"), "Lilex");
        assert!(!TerminalFont::MesloNerd.installed(&families));
    }

    #[test]
    fn sidebar_settings_keep_their_shape() {
        let prefs = SidebarPrefs {
            open: true,
            seat: "activity".to_owned(),
            width: 520.,
        };
        let json = serde_json::to_string(&prefs).expect("serialize");
        assert_eq!(
            serde_json::from_str::<SidebarPrefs>(&json).expect("read"),
            prefs
        );
        assert!(serde_json::from_str::<SidebarPrefs>("{").is_err());
        assert!(!SidebarPrefs::default().open);
        assert_eq!(SidebarPrefs::default().seat, "soundscape");
    }

    #[test]
    fn reminder_settings_keep_their_shape_and_an_unknown_lead_falls_back() {
        let prefs = NotifyPrefs {
            enabled: true,
            lead: "hour".to_owned(),
            seen_fare_alert_ids: Vec::new(),
            fare_alerts_last_check_ms: 0,
        };
        let json = serde_json::to_string(&prefs).expect("serialize");
        assert_eq!(
            serde_json::from_str::<NotifyPrefs>(&json).expect("read"),
            prefs
        );
        assert!(serde_json::from_str::<NotifyPrefs>("{").is_err());
        assert!(
            !NotifyPrefs::default().enabled,
            "a phone asks nothing of you by itself"
        );
        assert_eq!(
            NotifyPrefs::default().lead(),
            sjel_gpui_core::notify::Lead::Ten
        );
        let stale = NotifyPrefs {
            enabled: true,
            lead: "fifteen".to_owned(),
            seen_fare_alert_ids: Vec::new(),
            fare_alerts_last_check_ms: 0,
        };
        assert_eq!(
            stale.lead(),
            sjel_gpui_core::notify::Lead::default(),
            "an old file is not a wrong number of minutes"
        );
    }

    #[test]
    fn a_tint_keeps_the_hue_and_sets_only_the_alpha() {
        let red = Palette::light().red;
        let painted = gpui::rgba(tint(red, 0x22));
        let tone = gpui::rgb(red);
        assert_eq!(
            (painted.r, painted.g, painted.b),
            (tone.r, tone.g, tone.b),
            "the alpha must not steal a channel"
        );
        assert_eq!(painted.a, 0x22 as f32 / 255.0);

        let border = Palette::dark().border;
        let border_tinted = gpui::rgba(tint(border, 0x40));
        let border_base = gpui::rgba(border);
        assert_eq!(
            (border_tinted.r, border_tinted.g, border_tinted.b),
            (border_base.r, border_base.g, border_base.b),
            "32-bit surface tokens must keep their RGB channels when re-tinted"
        );
        assert_eq!(border_tinted.a, 0x40 as f32 / 255.0);
    }

    #[test]
    fn appearance_selects_distinct_palettes() {
        let light = Palette::for_appearance(WindowAppearance::Light);
        let dark = Palette::for_appearance(WindowAppearance::Dark);
        assert_ne!(light.window_tint, dark.window_tint);
        assert_ne!(light.surface, dark.surface);
        assert_ne!(light.text, dark.text);
        assert_ne!(light.accent, dark.accent);
    }

    #[test]
    fn density_respects_apple_hig_for_mobile() {
        assert!(Density::Mobile.min_touch_target() >= px(44.0));
        assert!(Density::Mobile.row_height() >= px(44.0));
    }

    #[test]
    fn appearance_settings_adjust_opacity_and_tone() {
        let base = Palette::for_appearance(WindowAppearance::Dark);
        let opaque = AppearanceSettings {
            opacity: WindowOpacity::Opaque,
            tone: BackgroundTone::System,
            glass: GlassStyle::LiquidGlass,
            ..Default::default()
        }
        .apply_to_palette(base, true);
        assert_eq!(opaque.window_tint & 0xff, 0xff);

        let ultra = AppearanceSettings {
            opacity: WindowOpacity::UltraGlass,
            tone: BackgroundTone::System,
            glass: GlassStyle::LiquidGlass,
            ..Default::default()
        }
        .apply_to_palette(base, true);
        assert_eq!(ultra.window_tint & 0xff, 0x40);

        let midnight = AppearanceSettings {
            opacity: WindowOpacity::Balanced,
            tone: BackgroundTone::MidnightSlate,
            glass: GlassStyle::LiquidGlass,
            ..Default::default()
        }
        .apply_to_palette(base, true);
        assert_eq!(midnight.window_tint >> 8, 0x0f172a);

        let violet = AppearanceSettings {
            accent: AccentColor::Violet,
            ..Default::default()
        }
        .apply_to_palette(base, true);
        assert_eq!(violet.accent, AccentColor::Violet.color_rgb(true));
    }

    #[test]
    fn theme_mode_overrides_system() {
        assert!(ThemeMode::Dark.is_dark(false));
        assert!(!ThemeMode::Light.is_dark(true));
        assert!(ThemeMode::System.is_dark(true));
        assert!(!ThemeMode::System.is_dark(false));
    }

    #[test]
    fn settings_serialization_roundtrip() {
        let original = AppearanceSettings {
            opacity: WindowOpacity::Translucent,
            sidebar_opacity: WindowOpacity::Balanced,
            panel_opacity: WindowOpacity::Balanced,
            tone: BackgroundTone::WarmAmber,
            glass: GlassStyle::Flat,
            accent: AccentColor::Emerald,
            theme_mode: ThemeMode::Dark,
            density: Density::Comfortable,
            terminal_tone: BackgroundTone::MidnightSlate,
            terminal_opacity: WindowOpacity::SoftGlass,
            terminal_font: TerminalFont::Menlo,
            terminal_font_size: 14,
            ui_font_size: 18,
        };
        let json = serde_json::to_string(&original).unwrap();
        let deserialized: AppearanceSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(original, deserialized);
    }

    #[test]
    fn ui_zoom_steps_hold_at_both_ends_and_older_files_get_the_default() {
        assert_eq!(step_font_size(&UI_FONT_SIZES, DEFAULT_UI_FONT_SIZE, 1), 18);
        assert_eq!(step_font_size(&UI_FONT_SIZES, DEFAULT_UI_FONT_SIZE, -1), 14);
        assert_eq!(step_font_size(&UI_FONT_SIZES, 24, 3), 24);
        assert_eq!(step_font_size(&UI_FONT_SIZES, 11, -3), 11);
        assert_eq!(step_font_size(&UI_FONT_SIZES, 15, 0), 16);
        let saved: AppearanceSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(saved.ui_font_size, DEFAULT_UI_FONT_SIZE);
    }
}
