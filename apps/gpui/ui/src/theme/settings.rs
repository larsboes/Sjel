//! Window background tone, glass style, accent colors, and user appearance settings.
#[cfg(feature = "gpui")]
use gpui_kit::WindowAppearance;

use super::density::Density;
use super::palette::{tint, with_alpha, Palette};
use super::terminal::{default_terminal_font_size, default_ui_font_size, TerminalFont, MONOKAI};

/// Window background opacity level.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum WindowOpacity {
    Opaque,
    MostlyOpaque,
    #[default]
    Balanced,
    Translucent,
    SoftGlass,
    UltraGlass,
}

impl WindowOpacity {
    pub const ALL: [Self; 6] = [
        Self::Opaque,
        Self::MostlyOpaque,
        Self::Balanced,
        Self::Translucent,
        Self::SoftGlass,
        Self::UltraGlass,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Opaque => "Opaque (100%)",
            Self::MostlyOpaque => "Mostly opaque (85%)",
            Self::Balanced => "Balanced (70%)",
            Self::Translucent => "Translucent (55%)",
            Self::SoftGlass => "Soft glass (40%)",
            Self::UltraGlass => "Ultra Glass (25%)",
        }
    }

    pub fn short_label(self) -> &'static str {
        match self {
            Self::Opaque => "100%",
            Self::MostlyOpaque => "85%",
            Self::Balanced => "70%",
            Self::Translucent => "55%",
            Self::SoftGlass => "40%",
            Self::UltraGlass => "25%",
        }
    }

    pub fn alpha(self) -> u8 {
        match self {
            Self::Opaque => 0xff,
            Self::MostlyOpaque => 0xd9,
            Self::Balanced => 0xb3,
            Self::Translucent => 0x8c,
            Self::SoftGlass => 0x66,
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

    /// The appearance to force on every window, or `None` to follow the system.
    #[cfg(feature = "gpui")]
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct AppearanceSettings {
    pub opacity: WindowOpacity,
    #[serde(default = "default_sidebar_opacity")]
    pub sidebar_opacity: WindowOpacity,
    #[serde(default = "default_panel_opacity")]
    pub panel_opacity: WindowOpacity,
    pub tone: BackgroundTone,
    pub glass: GlassStyle,
    pub accent: AccentColor,
    pub theme_mode: ThemeMode,
    pub density: Density,
    /// The terminal grid's own background: a tone (`System` follows the theme) and how much of
    /// what is behind it shows through.
    pub terminal_tone: BackgroundTone,
    #[serde(default = "default_panel_opacity")]
    pub terminal_opacity: WindowOpacity,
    pub terminal_font: TerminalFont,
    #[serde(default = "default_terminal_font_size")]
    pub terminal_font_size: u8,
    /// The app's zoom: the base font size every `rem` resolves to.
    #[serde(default = "default_ui_font_size")]
    pub ui_font_size: u8,
}

impl Default for AppearanceSettings {
    fn default() -> Self {
        Self {
            opacity: WindowOpacity::default(),
            sidebar_opacity: default_sidebar_opacity(),
            panel_opacity: default_panel_opacity(),
            tone: BackgroundTone::default(),
            glass: GlassStyle::default(),
            accent: AccentColor::default(),
            theme_mode: ThemeMode::default(),
            density: Density::default(),
            terminal_tone: BackgroundTone::default(),
            terminal_opacity: default_panel_opacity(),
            terminal_font: TerminalFont::default(),
            terminal_font_size: default_terminal_font_size(),
            ui_font_size: default_ui_font_size(),
        }
    }
}

impl AppearanceSettings {
    /// Applies user settings over a base appearance palette.
    pub fn apply_to_palette(&self, mut palette: Palette, dark: bool) -> Palette {
        let final_alpha = self.opacity.alpha();

        let rgb = if let Some(tone_rgb) = self.tone.tint_rgb(dark) {
            tone_rgb
        } else {
            palette.window_tint >> 8
        };

        palette.window_tint = (rgb << 8) | (final_alpha as u32);
        palette.sidebar_surface = with_alpha(palette.sidebar_surface, self.sidebar_opacity.alpha());
        palette.panel_surface = with_alpha(palette.surface, self.panel_opacity.alpha());
        palette.accent = self.accent.color_rgb(dark);
        let terminal = self
            .terminal_tone
            .tint_rgb(true)
            .unwrap_or(MONOKAI.background);
        palette.terminal_surface = tint(terminal, self.terminal_opacity.alpha());
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

pub(crate) fn default_sidebar_opacity() -> WindowOpacity {
    WindowOpacity::Balanced
}

pub(crate) fn default_panel_opacity() -> WindowOpacity {
    WindowOpacity::Opaque
}

/// Active theme and density context stored as a GPUI [`gpui::Global`] so every view can read the
/// user-configured [`Palette`], [`Density`], and [`AppearanceSettings`] without manual prop-drilling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg(feature = "gpui")]
pub struct ThemeContext {
    pub palette: Palette,
    pub density: Density,
    pub appearance: AppearanceSettings,
}

#[cfg(feature = "gpui")]
impl gpui::Global for ThemeContext {}

#[cfg(feature = "gpui")]
impl ThemeContext {
    /// Reads the active [`ThemeContext`] from the GPUI app context, or falls back to the window's
    /// system appearance when rendered inside an isolated unit test where no global was set.
    pub fn of(window: &gpui::Window, cx: &gpui::App) -> Self {
        if let Some(ctx) = cx.try_global::<Self>() {
            *ctx
        } else {
            let appearance = AppearanceSettings::default();
            let system_dark = matches!(
                window.appearance(),
                WindowAppearance::Dark | WindowAppearance::VibrantDark
            );
            let is_dark = appearance.theme_mode.is_dark(system_dark);
            let base = if is_dark {
                Palette::dark()
            } else {
                Palette::light()
            };
            Self {
                palette: appearance.apply_to_palette(base, is_dark),
                density: appearance.density,
                appearance,
            }
        }
    }

    /// Synchronizes the global [`ThemeContext`] from [`AppearanceSettings`] and the current window.
    pub fn sync(
        appearance: AppearanceSettings,
        window: &gpui::Window,
        cx: &mut gpui::App,
    ) -> Palette {
        let system_dark = matches!(
            window.appearance(),
            WindowAppearance::Dark | WindowAppearance::VibrantDark
        );
        let is_dark = appearance.theme_mode.is_dark(system_dark);
        let base = if is_dark {
            Palette::dark()
        } else {
            Palette::light()
        };
        let palette = appearance.apply_to_palette(base, is_dark);
        cx.set_global(Self {
            palette,
            density: appearance.density,
            appearance,
        });
        palette
    }
}

/// What the soundscape drawer remembers between launches.
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
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct NotifyPrefs {
    pub enabled: bool,
    /// A [`sjel_gpui_core::notify::Lead`] key: `at_start`, `ten` or `hour`.
    pub lead: String,
    #[serde(default)]
    pub seen_fare_alert_ids: Vec<String>,
    #[serde(default)]
    pub fare_alerts_last_check_ms: i64,
}

impl Default for NotifyPrefs {
    fn default() -> Self {
        Self {
            enabled: false,
            lead: sjel_gpui_core::notify::Lead::default().key().to_owned(),
            seen_fare_alert_ids: Vec::new(),
            fare_alerts_last_check_ms: 0,
        }
    }
}

impl NotifyPrefs {
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

    pub fn lead(&self) -> sjel_gpui_core::notify::Lead {
        sjel_gpui_core::notify::Lead::from_key(&self.lead).unwrap_or_default()
    }

    pub fn set_lead(&mut self, lead: sjel_gpui_core::notify::Lead) {
        self.lead = lead.key().to_owned();
    }

    pub fn mark_fare_alerts_seen(&mut self, ids: &[String]) {
        for id in ids {
            if !self.seen_fare_alert_ids.contains(id) {
                self.seen_fare_alert_ids.push(id.clone());
            }
        }
    }

    pub fn begin_fare_alert_check(&mut self, now_ms: i64) -> bool {
        if now_ms.saturating_sub(self.fare_alerts_last_check_ms) < 600_000 {
            return false;
        }
        self.fare_alerts_last_check_ms = now_ms;
        true
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
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SidebarPrefs {
    pub open: bool,
    /// A [`crate::sidebar::Seat`] key: `soundscape`, `activity` or `agent`.
    pub seat: String,
    /// The docked edge's width in points on a Mac, as the person last dragged it.
    #[serde(default = "default_sidebar_width")]
    pub width: f32,
}

fn default_sidebar_width() -> f32 {
    390.
}

impl Default for SidebarPrefs {
    fn default() -> Self {
        Self {
            open: false,
            seat: sjel_gpui_core::panel::Seat::default().key().to_owned(),
            width: default_sidebar_width(),
        }
    }
}

impl SidebarPrefs {
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
