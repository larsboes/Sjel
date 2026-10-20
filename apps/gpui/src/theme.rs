//! Semantic colours over the blurred window. A tone says what a value means; nothing here is
//! decorative (`Packs/design/skills/ui-craftsmanship`, rule 1).

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
}
