//! Terminal schemes, Monokai color tokens, and font resolution.

/// A terminal's colours: the sixteen ANSI tones, and the colours a program names by role.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerminalScheme {
    pub ansi: [u32; 16],
    pub foreground: u32,
    pub background: u32,
    pub cursor: u32,
    pub selection: u32,
}

/// Monokai, the terminal's colours. Dark in either app theme, the way a terminal usually is.
pub const MONOKAI: TerminalScheme = TerminalScheme {
    ansi: [
        0x272822, 0xf92672, 0xa6e22e, 0xf4bf75, 0x66d9ef, 0xae81ff, 0xa1efe4,
        0xf8f8f2, // normal
        0x75715e, 0xf92672, 0xa6e22e, 0xf4bf75, 0x66d9ef, 0xae81ff, 0xa1efe4,
        0xf9f8f5, // bright
    ],
    foreground: 0xf8f8f2,
    background: 0x272822,
    cursor: 0xf8f8f0,
    selection: 0x49483e,
};

/// The terminal's font. `Auto` takes the first Nerd Font installed, so the icons `ls` and
/// prompts print draw whole, and falls back to the theme's monospace font.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum TerminalFont {
    #[default]
    Auto,
    Theme,
    Menlo,
    SfMono,
    HackNerd,
    JetBrainsNerd,
    MesloNerd,
}

impl TerminalFont {
    pub const ALL: [Self; 7] = [
        Self::Auto,
        Self::Theme,
        Self::Menlo,
        Self::SfMono,
        Self::HackNerd,
        Self::JetBrainsNerd,
        Self::MesloNerd,
    ];

    /// What `Auto` looks for, in order.
    const NERD: [Self; 3] = [Self::HackNerd, Self::JetBrainsNerd, Self::MesloNerd];

    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::Theme => "Theme",
            Self::Menlo => "Menlo",
            Self::SfMono => "SF Mono",
            Self::HackNerd => "Hack Nerd",
            Self::JetBrainsNerd => "JetBrains Nerd",
            Self::MesloNerd => "Meslo Nerd",
        }
    }

    /// The family name the text system knows, or `None` for the two that resolve to another.
    fn family(self) -> Option<&'static str> {
        match self {
            Self::Auto | Self::Theme => None,
            Self::Menlo => Some("Menlo"),
            Self::SfMono => Some("SF Mono"),
            Self::HackNerd => Some("Hack Nerd Font Mono"),
            Self::JetBrainsNerd => Some("JetBrainsMono Nerd Font Mono"),
            Self::MesloNerd => Some("MesloLGS NF"),
        }
    }

    /// The font families this machine has, read once: the list does not change while the app
    /// runs, and a settings page asks for it on every frame.
    #[cfg(feature = "gpui")]
    pub fn families(cx: &gpui::App) -> &'static [String] {
        static FAMILIES: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
        FAMILIES.get_or_init(|| cx.text_system().all_font_names())
    }

    /// The choices that can be drawn here, for a settings row.
    #[cfg(feature = "gpui")]
    pub fn available(cx: &gpui::App) -> Vec<Self> {
        let families = Self::families(cx);
        Self::ALL
            .into_iter()
            .filter(|font| font.installed(families))
            .collect()
    }

    /// Whether this choice can be drawn here. `Auto` and `Theme` always can.
    pub fn installed(self, families: &[String]) -> bool {
        self.family()
            .is_none_or(|family| families.iter().any(|name| name == family))
    }

    /// The family to draw with, given the installed families and the theme's monospace font.
    /// A choice that is not installed here draws in the theme's font rather than a fallback.
    pub fn resolve(self, families: &[String], theme_mono: &str) -> String {
        let pick = match self {
            Self::Auto => Self::NERD.into_iter().find(|font| font.installed(families)),
            Self::Theme => None,
            chosen => chosen.installed(families).then_some(chosen),
        };
        pick.and_then(Self::family).unwrap_or(theme_mono).to_owned()
    }
}

/// The terminal's font sizes on offer, in points.
pub const TERMINAL_FONT_SIZES: [u8; 5] = [11, 12, 13, 14, 16];

/// The app's base font sizes on offer, in points. Every `rem` in the UI is this size.
pub const UI_FONT_SIZES: [u8; 9] = [11, 12, 13, 14, 16, 18, 20, 22, 24];

/// The default base font size, gpui-component's own.
pub const DEFAULT_UI_FONT_SIZE: u8 = 16;

/// The size `steps` places away from `current` among `sizes`, held at both ends.
/// A size that is not on offer (an edited file) steps from the nearest one that is.
pub fn step_font_size(sizes: &[u8], current: u8, steps: i32) -> u8 {
    let at = sizes
        .iter()
        .position(|size| *size >= current)
        .unwrap_or(sizes.len() - 1) as i32;
    let to = (at + steps).clamp(0, sizes.len() as i32 - 1);
    sizes[to as usize]
}

pub fn step_terminal_font_size(current: u8, steps: i32) -> u8 {
    step_font_size(&TERMINAL_FONT_SIZES, current, steps)
}

pub(crate) fn default_terminal_font_size() -> u8 {
    13
}

pub(crate) fn default_ui_font_size() -> u8 {
    DEFAULT_UI_FONT_SIZE
}
