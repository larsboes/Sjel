//! Semantic colors, design tokens, and density rules for desktop and mobile GPUI surfaces.
//!
//! A tone says what a value means; nothing here is decorative (`Packs/design/skills/ui-craftsmanship`, rule 1).

use gpui::px;
use gpui_kit::WindowAppearance;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    pub window_tint: u32,
    pub surface: u32,
    pub raised: u32,
    pub border: u32,
    pub text: u32,
    pub muted: u32,
    pub accent: u32,
    /// Settled, running.
    pub green: u32,
    /// Needs a decision.
    pub amber: u32,
    /// Broken.
    pub red: u32,
}

impl Palette {
    pub fn for_appearance(appearance: WindowAppearance) -> Self {
        match appearance {
            WindowAppearance::Light | WindowAppearance::VibrantLight => Self {
                window_tint: 0xf3f5f9a8,
                surface: 0xfffffff0,
                raised: 0x1118270b,
                border: 0x20273524,
                text: 0x1c1c1e,
                muted: 0x646b77,
                accent: 0x007aff,
                green: 0x248a3d,
                amber: 0xc93400,
                red: 0xd70015,
            },
            WindowAppearance::Dark | WindowAppearance::VibrantDark => Self {
                window_tint: 0x10131ae0,
                surface: 0x252a3499,
                raised: 0xffffff18,
                border: 0xffffff26,
                text: 0xf5f6f8,
                muted: 0xa5aab4,
                accent: 0x0a84ff,
                green: 0x62d992,
                amber: 0xffb340,
                red: 0xff6961,
            },
        }
    }

    /// Light mode palette as default baseline (e.g. for views before window appearance resolves).
    pub fn light() -> Self {
        Self::for_appearance(WindowAppearance::Light)
    }

    /// Dark mode palette.
    pub fn dark() -> Self {
        Self::for_appearance(WindowAppearance::Dark)
    }
}

/// Layout density: handles compact desktop keyboards/mouse vs touch targets on mobile.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum Density {
    Compact,
    #[default]
    Desktop,
    Comfortable,
    Mobile,
}

impl Density {
    pub const ALL: [Self; 4] = [
        Self::Compact,
        Self::Desktop,
        Self::Comfortable,
        Self::Mobile,
    ];
    pub const DESKTOP_ALL: [Self; 3] = [Self::Compact, Self::Desktop, Self::Comfortable];

    pub fn is_mobile(self) -> bool {
        matches!(self, Self::Mobile)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Compact => "Compact",
            Self::Desktop => "Standard",
            Self::Comfortable => "Comfortable",
            Self::Mobile => "Mobile Touch",
        }
    }

    /// Minimum touch target size (44pt on iOS per Apple HIG, 32px on macOS).
    pub fn min_touch_target(self) -> gpui::Pixels {
        match self {
            Self::Compact => px(28.0),
            Self::Desktop => px(32.0),
            Self::Comfortable => px(36.0),
            Self::Mobile => px(44.0),
        }
    }

    /// Standard row height for lists and tables.
    pub fn row_height(self) -> gpui::Pixels {
        match self {
            Self::Compact => px(28.0),
            Self::Desktop => px(36.0),
            Self::Comfortable => px(42.0),
            Self::Mobile => px(48.0),
        }
    }

    /// Standard vertical padding.
    pub fn pad_y(self) -> gpui::Pixels {
        match self {
            Self::Compact => px(4.0),
            Self::Desktop => px(6.0),
            Self::Comfortable => px(8.0),
            Self::Mobile => px(12.0),
        }
    }

    /// Standard horizontal padding.
    pub fn pad_x(self) -> gpui::Pixels {
        match self {
            Self::Compact => px(8.0),
            Self::Desktop => px(12.0),
            Self::Comfortable => px(14.0),
            Self::Mobile => px(16.0),
        }
    }
}

/// Window background opacity level.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum WindowOpacity {
    Opaque,
    #[default]
    Balanced,
    Translucent,
    UltraGlass,
}

impl WindowOpacity {
    pub const ALL: [Self; 4] = [
        Self::Opaque,
        Self::Balanced,
        Self::Translucent,
        Self::UltraGlass,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Opaque => "Opaque (100%)",
            Self::Balanced => "Balanced (75%)",
            Self::Translucent => "Translucent (50%)",
            Self::UltraGlass => "Ultra Glass (25%)",
        }
    }

    pub fn short_label(self) -> &'static str {
        match self {
            Self::Opaque => "100%",
            Self::Balanced => "75%",
            Self::Translucent => "50%",
            Self::UltraGlass => "25%",
        }
    }

    pub fn alpha(self, default_alpha: u8) -> u8 {
        match self {
            Self::Opaque => 0xff,
            Self::Balanced => default_alpha,
            Self::Translucent => 0x80,
            Self::UltraGlass => 0x40,
        }
    }
}

/// Ambient background tint tone.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum BackgroundTone {
    #[default]
    System,
    MidnightSlate,
    ObsidianNoir,
    WarmAmber,
}

impl BackgroundTone {
    pub const ALL: [Self; 4] = [
        Self::System,
        Self::MidnightSlate,
        Self::ObsidianNoir,
        Self::WarmAmber,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::MidnightSlate => "Midnight",
            Self::ObsidianNoir => "Obsidian",
            Self::WarmAmber => "Warm Amber",
        }
    }

    pub fn tint_rgb(self, dark: bool) -> Option<u32> {
        match self {
            Self::System => None,
            Self::MidnightSlate => Some(if dark { 0x0f172a } else { 0xe2e8f0 }),
            Self::ObsidianNoir => Some(if dark { 0x090a0f } else { 0xf8fafc }),
            Self::WarmAmber => Some(if dark { 0x1c1510 } else { 0xfaf5ee }),
        }
    }
}

/// Glass effect style.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum GlassStyle {
    #[default]
    LiquidGlass,
    Flat,
}

impl GlassStyle {
    pub const ALL: [Self; 2] = [Self::LiquidGlass, Self::Flat];

    pub fn label(self) -> &'static str {
        match self {
            Self::LiquidGlass => "Liquid Glass",
            Self::Flat => "Flat Solid",
        }
    }
}

/// Accent color tints for active selection, focus, and buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum AccentColor {
    #[default]
    DefaultBlue,
    Violet,
    Cyan,
    Emerald,
    Amber,
    Rose,
}

impl AccentColor {
    pub const ALL: [Self; 6] = [
        Self::DefaultBlue,
        Self::Violet,
        Self::Cyan,
        Self::Emerald,
        Self::Amber,
        Self::Rose,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::DefaultBlue => "Blue",
            Self::Violet => "Violet",
            Self::Cyan => "Cyan",
            Self::Emerald => "Emerald",
            Self::Amber => "Amber",
            Self::Rose => "Rose",
        }
    }

    pub fn color_rgb(self, dark: bool) -> u32 {
        match self {
            Self::DefaultBlue => {
                if dark {
                    0x0a84ff
                } else {
                    0x007aff
                }
            }
            Self::Violet => {
                if dark {
                    0xa855f7
                } else {
                    0x8b5cf6
                }
            }
            Self::Cyan => {
                if dark {
                    0x22d3ee
                } else {
                    0x06b6d4
                }
            }
            Self::Emerald => {
                if dark {
                    0x34d399
                } else {
                    0x10b981
                }
            }
            Self::Amber => {
                if dark {
                    0xfbbf24
                } else {
                    0xf59e0b
                }
            }
            Self::Rose => {
                if dark {
                    0xfb7185
                } else {
                    0xe11d48
                }
            }
        }
    }

    /// Swatch color for UI display.
    pub fn swatch_rgb(self) -> u32 {
        self.color_rgb(true)
    }
}

/// System appearance mode override (Auto system, forced Dark, forced Light).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum ThemeMode {
    #[default]
    System,
    Dark,
    Light,
}

impl ThemeMode {
    pub const ALL: [Self; 3] = [Self::System, Self::Dark, Self::Light];

    pub fn label(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Dark => "Dark",
            Self::Light => "Light",
        }
    }

    /// The appearance to force on every window, or `None` to follow the system. Forcing it at the
    /// window keeps the native glass, gpui-component's theme and the palette on one mode.
    pub fn forced_appearance(self) -> Option<WindowAppearance> {
        match self {
            Self::System => None,
            Self::Dark => Some(WindowAppearance::Dark),
            Self::Light => Some(WindowAppearance::Light),
        }
    }

    pub fn is_dark(self, system_dark: bool) -> bool {
        match self {
            Self::System => system_dark,
            Self::Dark => true,
            Self::Light => false,
        }
    }
}

/// User-controlled appearance preferences for background, transparency, glass, accent and density.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct AppearanceSettings {
    pub opacity: WindowOpacity,
    pub tone: BackgroundTone,
    pub glass: GlassStyle,
    pub accent: AccentColor,
    pub theme_mode: ThemeMode,
    pub density: Density,
}

impl AppearanceSettings {
    /// Applies user settings over a base appearance palette.
    pub fn apply_to_palette(&self, mut palette: Palette, dark: bool) -> Palette {
        let base_alpha = (palette.window_tint & 0xff) as u8;
        let final_alpha = self.opacity.alpha(base_alpha);

        let rgb = if let Some(tone_rgb) = self.tone.tint_rgb(dark) {
            tone_rgb
        } else {
            palette.window_tint >> 8
        };

        palette.window_tint = (rgb << 8) | (final_alpha as u32);
        palette.accent = self.accent.color_rgb(dark);
        palette
    }

    /// Resolved configuration file path.
    pub fn config_path() -> Option<std::path::PathBuf> {
        if let Ok(dir) = std::env::var("SJEL_CONFIG_DIR") {
            return Some(std::path::PathBuf::from(dir).join("appearance.json"));
        }
        if let Ok(home) = std::env::var("HOME") {
            return Some(std::path::PathBuf::from(home).join(".config/sjel/appearance.json"));
        }
        None
    }

    /// Loads settings from disk with fallbacks to defaults and environment overrides.
    pub fn load() -> Self {
        let mut settings = Self::default();

        if let Some(path) = Self::config_path() {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(loaded) = serde_json::from_str::<Self>(&content) {
                    settings = loaded;
                }
            }
        }

        // Environment variable overrides
        if std::env::var_os("SJEL_GPUI_OPAQUE").is_some() {
            settings.opacity = WindowOpacity::Opaque;
        }
        if std::env::var_os("SJEL_GPUI_NO_GLASS").is_some() {
            settings.glass = GlassStyle::Flat;
        }

        settings
    }

    /// Saves settings to disk.
    pub fn save(&self) -> Result<(), String> {
        let Some(path) = Self::config_path() else {
            return Err("Could not determine config path".into());
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Serialization error: {}", e))?;
        std::fs::write(&path, json).map_err(|e| format!("Write error to {:?}: {}", path, e))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            tone: BackgroundTone::WarmAmber,
            glass: GlassStyle::Flat,
            accent: AccentColor::Emerald,
            theme_mode: ThemeMode::Dark,
            density: Density::Comfortable,
        };
        let json = serde_json::to_string(&original).unwrap();
        let deserialized: AppearanceSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(original, deserialized);
    }
}
