//! Tables, subtabs, scroll containers, keyboard shortcuts, and remote state boundaries.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::Sizable;
use gpui_kit::{
    div, prelude::*, px, rgb, rgba, AnyElement, App, ClickEvent, ElementId, FontWeight,
    MouseButton, Pixels, ScrollHandle, SharedString, Window,
};

use super::cards::row_for_density;
use super::tokens::{placeholder, RADIUS_MD, RADIUS_SM};
use crate::data::Remote;
use crate::theme::{tint, Density, Palette};

/// Standardised sub-tab strip with optional counts.
pub fn sub_tabs(
    id_prefix: &'static str,
    tabs: Vec<(&'static str, Option<usize>, bool)>,
    palette: Palette,
    on_select: impl Fn(usize, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let on_select = std::rc::Rc::new(on_select);
    let mut strip = div().flex().flex_wrap().items_center().gap_1p5();
    for (idx, (label, count, active)) in tabs.into_iter().enumerate() {
        let on_select = on_select.clone();
        let text = match count {
            Some(c) => format!("{label} ({c})"),
            None => label.to_owned(),
        };
        strip = strip.child(
            div()
                .id(SharedString::from(format!("{id_prefix}-tab-{idx}")))
                .px_3()
                .py_1()
                .rounded(RADIUS_MD)
                .cursor_pointer()
                .text_xs()
                .font_weight(if active {
                    FontWeight::SEMIBOLD
                } else {
                    FontWeight::MEDIUM
                })
                .bg(if active {
                    rgba(tint(palette.accent, 0x20))
                } else {
                    rgba(palette.raised)
                })
                .text_color(if active {
                    rgb(palette.accent)
                } else {
                    rgb(palette.muted)
                })
                .hover(move |s| {
                    s.bg(rgba(if active {
                        tint(palette.accent, 0x28)
                    } else {
                        tint(palette.border, 0x35)
                    }))
                    .text_color(rgb(if active {
                        palette.accent
                    } else {
                        palette.text
                    }))
                })
                .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                    on_select(idx, window, cx)
                })
                .child(text),
        );
    }
    strip.into_any_element()
}

/// Column header row for dense data tables.
pub fn table_header(
    columns: Vec<(&'static str, Option<Pixels>, bool)>,
    density: Density,
    palette: Palette,
) -> AnyElement {
    let mut row = row_for_density(density)
        .min_h(px(30.))
        .py_1()
        .bg(rgba(tint(palette.raised, 0x30)))
        .border_b_1()
        .border_color(rgba(palette.border))
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(palette.muted));
    for (label, width, right_align) in columns {
        let cell = match width {
            Some(w) => div().w(w).flex_none(),
            None => div().flex_1().min_w_0(),
        };
        row = row.child(
            cell.when(right_align, |c| c.text_right())
                .truncate()
                .child(label),
        );
    }
    row.into_any_element()
}

/// Active sort direction for a sortable table column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortOrder {
    Ascending,
    Descending,
}

impl SortOrder {
    /// Arrow glyph shown beside an active column title.
    pub fn glyph(self) -> &'static str {
        match self {
            Self::Ascending => " ↑",
            Self::Descending => " ↓",
        }
    }
}

/// Specification of a column in [`sortable_table_header`].
pub struct SortableColumn {
    pub id: &'static str,
    pub label: &'static str,
    pub width: Option<Pixels>,
    pub right_align: bool,
    pub sort: Option<SortOrder>,
}

/// Interactive sortable column header strip with right-alignment for numeric columns.
pub fn sortable_table_header(
    id_prefix: &'static str,
    columns: Vec<SortableColumn>,
    density: Density,
    palette: Palette,
    on_sort: impl Fn(&'static str, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let on_sort = std::rc::Rc::new(on_sort);
    let mut row = row_for_density(density)
        .min_h(px(32.))
        .py_1()
        .bg(rgba(tint(palette.raised, 0x30)))
        .border_b_1()
        .border_color(rgba(palette.border))
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(palette.muted));
    for col in columns {
        let on_sort = on_sort.clone();
        let col_id = col.id;
        let active = col.sort.is_some();
        let text = match col.sort {
            Some(order) => format!("{}{}", col.label, order.glyph()),
            None => col.label.to_owned(),
        };
        let cell = match col.width {
            Some(w) => div().w(w).flex_none(),
            None => div().flex_1().min_w_0(),
        };
        row = row.child(
            cell.id(SharedString::from(format!("{id_prefix}-col-{col_id}")))
                .cursor_pointer()
                .when(col.right_align, |c| c.text_right())
                .when(active, |c| c.text_color(rgb(palette.text)))
                .hover(move |s| s.text_color(rgb(palette.text)))
                .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                    on_sort(col_id, window, cx)
                })
                .truncate()
                .child(text),
        );
    }
    row.into_any_element()
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

/// Density-aware interactive data row with hover and selection states.
pub fn data_row(
    id: impl Into<ElementId>,
    density: Density,
    selected: bool,
    palette: Palette,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    row_for_density(density)
        .id(id.into())
        .rounded(RADIUS_SM)
        .cursor_pointer()
        .when(selected, |r| {
            r.bg(rgba(tint(palette.accent, 0x18)))
                .border_l_2()
                .border_color(rgb(palette.accent))
        })
        .when(!selected, |r| r.hover(move |s| s.bg(rgba(palette.raised))))
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

/// Scrollable page body container that always attaches `.track_scroll` and `.vertical_scrollbar`.
pub fn page_scroll(
    id: impl Into<ElementId>,
    scroll: &ScrollHandle,
    body: impl IntoElement,
) -> AnyElement {
    div()
        .id(id.into())
        .flex_1()
        .min_h_0()
        .w_full()
        .overflow_y_scroll()
        .track_scroll(scroll)
        .vertical_scrollbar(scroll)
        .child(body)
        .into_any_element()
}

/// Uniform state presenter for [`Remote<T>`]: renders a reserved-height loading placeholder,
/// an inline error with a Retry button, or the loaded content.
pub fn remote_state<T>(
    remote: &Remote<T>,
    height: f32,
    palette: Palette,
    on_retry: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    render_value: impl FnOnce(&T) -> AnyElement,
) -> AnyElement {
    if let Some(value) = remote.value.as_ref() {
        return render_value(value);
    }
    if let Some(error) = remote.error.as_ref() {
        return div()
            .p_4()
            .rounded(RADIUS_MD)
            .bg(rgba(palette.surface))
            .border_1()
            .border_color(rgba(tint(palette.red, 0x40)))
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .text_color(rgb(palette.red))
                    .child(error.to_string()),
            )
            .child(
                Button::new("remote-retry")
                    .small()
                    .ghost()
                    .label("Retry")
                    .on_click(on_retry),
            )
            .into_any_element();
    }
    placeholder(height)
}
