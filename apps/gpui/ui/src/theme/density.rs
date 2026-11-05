//! Layout density options and Apple HIG touch target dimensions.
use gpui::{px, Pixels};

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
    pub fn min_touch_target(self) -> Pixels {
        match self {
            Self::Compact => px(28.0),
            Self::Desktop => px(32.0),
            Self::Comfortable => px(36.0),
            Self::Mobile => px(44.0),
        }
    }

    /// Standard row height for lists and tables.
    pub fn row_height(self) -> Pixels {
        match self {
            Self::Compact => px(28.0),
            Self::Desktop => px(36.0),
            Self::Comfortable => px(42.0),
            Self::Mobile => px(48.0),
        }
    }

    /// Standard vertical padding.
    pub fn pad_y(self) -> Pixels {
        match self {
            Self::Compact => px(4.0),
            Self::Desktop => px(6.0),
            Self::Comfortable => px(8.0),
            Self::Mobile => px(12.0),
        }
    }

    /// Standard horizontal padding.
    pub fn pad_x(self) -> Pixels {
        match self {
            Self::Compact => px(8.0),
            Self::Desktop => px(12.0),
            Self::Comfortable => px(14.0),
            Self::Mobile => px(16.0),
        }
    }
}
