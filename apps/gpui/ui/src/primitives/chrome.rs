//! Sub-tabs, keyboard shortcut bars, tabular numeric cells, and sort/column-width helpers.

use gpui_kit::{div, prelude::*, px, rgb, rgba, AnyElement, App, Pixels, SharedString, Window};

use super::tokens::RADIUS_SM;
use crate::ely::{forms::Choice, navigation::Tabs};
use crate::theme::Palette;

/// Standardised sub-tab strip with optional counts.
pub fn sub_tabs(
    id_prefix: &'static str,
    tabs: Vec<(&'static str, Option<usize>, bool)>,
    on_select: impl Fn(usize, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let values: Vec<SharedString> = (0..tabs.len())
        .map(|idx| format!("{id_prefix}-tab-{idx}").into())
        .collect();
    let selected = tabs
        .iter()
        .position(|(_, _, active)| *active)
        .map(|idx| values[idx].clone())
        .unwrap_or_default();
    let choices = tabs
        .into_iter()
        .enumerate()
        .map(|(idx, (label, count, _))| {
            let text = match count {
                Some(c) => format!("{label} ({c})"),
                None => label.to_owned(),
            };
            Choice::new(values[idx].clone(), text)
        })
        .collect::<Vec<_>>();
    let strip = Tabs::new(id_prefix, choices, selected)
        .scrollable()
        .on_change(move |value, window, cx| {
            if let Some(idx) = values.iter().position(|candidate| candidate == value) {
                on_select(idx, window, cx);
            }
        });
    div()
        .id(SharedString::from(format!("{id_prefix}-scroll")))
        .debug_selector(move || format!("{id_prefix}-tabs"))
        .min_w_0()
        .child(strip)
        .into_any_element()
}

/// Active sort direction for a sortable table column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortOrder {
    Ascending,
    Descending,
}

impl SortOrder {
    /// Convert a boolean ascending flag into [`SortOrder`].
    pub fn from_bool(ascending: bool) -> Self {
        if ascending {
            Self::Ascending
        } else {
            Self::Descending
        }
    }

    /// Whether this order is ascending.
    pub fn is_ascending(self) -> bool {
        matches!(self, Self::Ascending)
    }

    /// Flip between ascending and descending.
    pub fn flip(self) -> Self {
        match self {
            Self::Ascending => Self::Descending,
            Self::Descending => Self::Ascending,
        }
    }

    /// Arrow glyph shown beside an active column title.
    pub fn glyph(self) -> &'static str {
        match self {
            Self::Ascending => " ↑",
            Self::Descending => " ↓",
        }
    }
}

/// Resolve a column's current width from the table's width override map, falling back to `default`.
pub fn column_width(
    widths: &std::collections::HashMap<&'static str, f32>,
    id: &'static str,
    default: f32,
) -> f32 {
    widths.get(id).copied().unwrap_or(default)
}

/// Cycle a fixed column's width through `1.0×` → `1.35×` → `1.70×` → `1.0×`.
pub fn cycle_column_width(
    widths: &mut std::collections::HashMap<&'static str, f32>,
    id: &'static str,
    default: f32,
) -> f32 {
    let current = column_width(widths, id, default);
    let medium = (default * 1.35).round();
    let wide = (default * 1.70).round();
    if (current - default).abs() < 1.0 {
        widths.insert(id, medium);
        medium
    } else if (current - medium).abs() < 1.0 {
        widths.insert(id, wide);
        wide
    } else {
        widths.remove(id);
        default
    }
}

/// Right-aligned monospace tabular numeric cell for amounts, counts, and timestamps.
pub fn tabular_cell(value: impl Into<SharedString>, width: Pixels, color: u32) -> AnyElement {
    div()
        .w(width)
        .flex_none()
        .text_right()
        .font_family(".AppleSystemUIFontMonospaced")
        .text_xs()
        .text_color(rgb(color))
        .truncate()
        .child(value.into())
        .into_any_element()
}

/// Single keyboard shortcut hint (`[key] action`) for footer bars.
pub fn shortcut_hint(key: &'static str, action: &'static str, palette: Palette) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap_1()
        .child(
            div()
                .px_1p5()
                .py(px(1.))
                .rounded(RADIUS_SM)
                .bg(rgba(palette.raised))
                .border_1()
                .border_color(rgba(palette.border))
                .font_family(".AppleSystemUIFontMonospaced")
                .text_xs()
                .text_color(rgb(palette.text))
                .child(key),
        )
        .child(div().text_xs().text_color(rgb(palette.muted)).child(action))
        .into_any_element()
}

/// Footer bar of keyboard shortcut hints.
pub fn key_bar(hints: &[(&'static str, &'static str)], palette: Palette) -> AnyElement {
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap_3()
        .pt_2()
        .text_xs()
        .text_color(rgb(palette.muted))
        .children(hints.iter().map(|(k, a)| shortcut_hint(k, a, palette)))
        .into_any_element()
}
