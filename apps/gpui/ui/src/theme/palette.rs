//! Semantic color palette and tinting primitives.
use gpui_kit::WindowAppearance;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    pub window_tint: u32,
    pub surface: u32,
    pub sidebar_surface: u32,
    pub panel_surface: u32,
    /// The terminal grid's background, as `0xRRGGBBAA`.
    pub terminal_surface: u32,
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

/// A palette tone or surface at `alpha`, as `0xRRGGBBAA` for `rgba()`.
///
/// Works on both 24-bit RGB tones (`0x00RRGGBB`, e.g. `palette.accent`) and 32-bit RGBA surfaces
/// (`0xRRGGBBAA`, e.g. `palette.surface`, `palette.raised`, `palette.border`): any existing alpha
/// channel on a 32-bit surface token is replaced rather than shifting the red channel off the top.
pub const fn tint(tone: u32, alpha: u8) -> u32 {
    if tone > 0x00ff_ffff {
        (tone & 0xffff_ff00) | alpha as u32
    } else {
        (tone << 8) | alpha as u32
    }
}

pub(crate) const fn with_alpha(color: u32, alpha: u8) -> u32 {
    (color & 0xffff_ff00) | alpha as u32
}

impl Palette {
    /// The right-side panel's tinted surface and independent opacity.
    pub fn panel(&self) -> u32 {
        self.panel_surface
    }

    pub fn for_appearance(appearance: WindowAppearance) -> Self {
        match appearance {
            WindowAppearance::Light | WindowAppearance::VibrantLight => Self {
                window_tint: 0xf3f5f9a8,
                surface: 0xfffffff0,
                terminal_surface: 0xfffffff0,
                sidebar_surface: 0xffffffff,
                panel_surface: 0xf3f5f9ff,
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
                terminal_surface: 0x252a3499,
                sidebar_surface: 0x252a34ff,
                panel_surface: 0x10131aff,
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
