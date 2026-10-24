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
    /// The colour a surface that covers something is painted in: the window tint with its alpha
    /// forced to opaque.
    ///
    /// The tint is translucent on purpose, so the desktop shows through the window. A panel that
    /// slides *over* the page cannot be: its own labels would be read against whatever it hides,
    /// which is how the soundscape drawer first shipped -- the page's header row came through the
    /// drawn field and the preset name sat on top of it. Deriving it from `window_tint` keeps the
    /// tone and the appearance settings in one place, and only the transparency goes.
    pub fn panel(&self) -> u32 {
        (self.window_tint & 0xffff_ff00) | 0xff
    }

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

/// What the soundscape drawer remembers between launches.
///
/// The mixer disclosure, the preset and the volume: a drawer that forgets them is one the operator
/// has to set up again every morning. `playing` is deliberately *not* here — a phone that starts
/// making sound at launch is a different thing from one that remembers its settings — and neither is
/// the drawer's open state: the right edge is one panel with seats, and whether it is showing is
/// [`SidebarPrefs`]' business (`ui/src/sidebar.rs`).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SoundscapePrefs {
    pub mixer: bool,
    /// The preset's key ([`sjel_gpui_core::soundscape::Preset::key`]).
    pub preset: String,
    pub volume: f32,
}

impl Default for SoundscapePrefs {
    fn default() -> Self {
        Self {
            mixer: false,
            preset: "focus".to_owned(),
            volume: 0.55,
        }
    }
}

impl SoundscapePrefs {
    /// Beside `appearance.json`, because they are the same kind of thing: what this person likes,
    /// kept on this device.
    pub fn config_path() -> Option<std::path::PathBuf> {
        let dir = std::env::var("SJEL_CONFIG_DIR")
            .ok()
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var("HOME")
                    .ok()
                    .map(|home| std::path::PathBuf::from(home).join(".config/sjel"))
            })?;
        Some(dir.join("soundscape.json"))
    }

    pub fn load() -> Self {
        let Some(path) = Self::config_path() else {
            return Self::default();
        };
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        let Some(path) = Self::config_path() else {
            return Err("this platform names no place for it".to_owned());
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(&path, json).map_err(|e| format!("Write error to {:?}: {e}", path))
    }
}

/// What the phone remembers about its lock screen.
///
/// Two things only: whether reminders were turned on, and the lead time they use. What is
/// *scheduled* is never read from here — the requests themselves live in
/// `UNUserNotificationCenter`, and the phone asks the system what it holds rather than trusting a
/// file to describe it. `enabled` therefore means "this person turned it on", which is not the
/// same as "the system allowed it": a person who chose reminders and then denied the permission
/// in iOS Settings keeps the choice here and is told on screen where to change it.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct NotifyPrefs {
    pub enabled: bool,
    /// A [`sjel_gpui_core::notify::Lead`] key: `at_start`, `ten` or `hour`.
    pub lead: String,
}

impl Default for NotifyPrefs {
    fn default() -> Self {
        Self {
            // Off until somebody says otherwise: a phone that asks for the permission at launch
            // has spent its only prompt before the person knew what it was for.
            enabled: false,
            lead: sjel_gpui_core::notify::Lead::default().key().to_owned(),
        }
    }
}

impl NotifyPrefs {
    /// Beside `appearance.json` and `soundscape.json`: what this person likes, kept on this device.
    pub fn config_path() -> Option<std::path::PathBuf> {
        let dir = std::env::var("SJEL_CONFIG_DIR")
            .ok()
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var("HOME")
                    .ok()
                    .map(|home| std::path::PathBuf::from(home).join(".config/sjel"))
            })?;
        Some(dir.join("notifications.json"))
    }

    /// The lead the file names, or the default for a key this build does not know: an edited or
    /// older file must not turn every reminder on at the same minute as the entry.
    pub fn lead(&self) -> sjel_gpui_core::notify::Lead {
        sjel_gpui_core::notify::Lead::from_key(&self.lead)
            .unwrap_or_else(sjel_gpui_core::notify::Lead::default)
    }

    pub fn set_lead(&mut self, lead: sjel_gpui_core::notify::Lead) {
        self.lead = lead.key().to_owned();
    }

    pub fn load() -> Self {
        let Some(path) = Self::config_path() else {
            return Self::default();
        };
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        let Some(path) = Self::config_path() else {
            return Err("this platform names no place for it".to_owned());
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(&path, json).map_err(|e| format!("Write error to {:?}: {e}", path))
    }
}

/// What the right edge remembers: whether it is showing, and which seat it was on.
///
/// Its own file rather than a field of [`SoundscapePrefs`], because the edge is not the soundscape's:
/// the soundscape is one seat on it (`ui/src/sidebar.rs`). A file written before the edge had seats
/// carries an `open` key in `soundscape.json`; serde ignores what it does not know, so that file still
/// reads and the person's first visit to the edge is the one that writes this one.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SidebarPrefs {
    pub open: bool,
    /// A [`crate::sidebar::Seat`] key: `soundscape` or `activity`.
    pub seat: String,
}

impl Default for SidebarPrefs {
    fn default() -> Self {
        Self {
            // A phone that opens a panel by itself covers its own page before the person asked.
            open: false,
            seat: crate::sidebar::Seat::default().key().to_owned(),
        }
    }
}

impl SidebarPrefs {
    /// Beside `appearance.json`, `soundscape.json` and `notifications.json`.
    pub fn config_path() -> Option<std::path::PathBuf> {
        let dir = std::env::var("SJEL_CONFIG_DIR")
            .ok()
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var("HOME")
                    .ok()
                    .map(|home| std::path::PathBuf::from(home).join(".config/sjel"))
            })?;
        Some(dir.join("sidebar.json"))
    }

    pub fn load() -> Self {
        let Some(path) = Self::config_path() else {
            return Self::default();
        };
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        let Some(path) = Self::config_path() else {
            return Err("this platform names no place for it".to_owned());
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(&path, json).map_err(|e| format!("Write error to {:?}: {e}", path))
    }
}

#[cfg(test)]
mod tests {
    /// The drawer's settings keep their shape through JSON, and a damaged file is not a half-applied
    /// panel. The write itself is covered on the device: `SJEL_CONFIG_DIR` is not something this test
    /// may set, because `ui` denies unsafe code and `set_var` is unsafe in this edition.
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
        // A file from the build before the edge had seats carries `open`; it still reads, and the
        // key is dropped rather than mistaken for a setting this build has.
        let older = r#"{"open":true,"mixer":false,"preset":"lofi","volume":0.4}"#;
        let read: SoundscapePrefs = serde_json::from_str(older).expect("an older file reads");
        assert_eq!(read.preset, "lofi");
    }

    /// The right edge's settings keep their shape, and a damaged file is a closed edge rather than a
    /// panel over the page at launch.
    #[test]
    fn sidebar_settings_keep_their_shape() {
        let prefs = SidebarPrefs {
            open: true,
            seat: "activity".to_owned(),
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

    /// The reminder settings keep their shape, and a lead this build does not know falls back to
    /// the default instead of turning every reminder on at the entry's own minute.
    #[test]
    fn reminder_settings_keep_their_shape_and_an_unknown_lead_falls_back() {
        let prefs = NotifyPrefs {
            enabled: true,
            lead: "hour".to_owned(),
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
        };
        assert_eq!(
            stale.lead(),
            sjel_gpui_core::notify::Lead::default(),
            "an old file is not a wrong number of minutes"
        );
    }

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
