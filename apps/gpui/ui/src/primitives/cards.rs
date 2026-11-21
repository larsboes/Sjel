//! Card frames, lists, density rows, progress bars, and metrics.

use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::IconName;
use gpui_kit::{
    div, list as gpui_list, prelude::*, px, rgb, rgba, AnyElement, App, ClickEvent, ElementId,
    FontWeight, ListState, SharedString, Window,
};

use super::tokens::FONT_MONO_TABULAR;
use crate::ely::{
    buttons::{Button, ButtonVariant},
    layout::{Card, CardHeader, Section},
    lists::List,
    primitives::Tooltip,
    theme::ControlSize,
};
use crate::theme::{tint, Density, Palette};

pub fn section(title: &'static str, body: AnyElement, _palette: Palette) -> impl IntoElement {
    Section::new(title).child(body)
}

pub fn list(rows: Vec<AnyElement>, palette: Palette) -> AnyElement {
    div()
        .rounded(px(16.))
        .bg(rgba(palette.surface))
        .border_1()
        .border_color(rgba(palette.border))
        .overflow_hidden()
        .child(List::new().divided().children(rows))
        .into_any_element()
}

/// Virtualized vertical list with a bound scrollbar and flex height contract (`flex_1` + `min_h_0`).
pub fn virtual_list(
    id: impl Into<ElementId>,
    state: &ListState,
    render_item: impl Fn(usize, &mut Window, &mut App) -> AnyElement + 'static,
) -> AnyElement {
    div()
        .id(id.into())
        .relative()
        .flex_1()
        .min_h_0()
        .w_full()
        .vertical_scrollbar(state)
        .child(gpui_list(state.clone(), render_item).size_full())
        .into_any_element()
}

/// Standard desktop row.
pub fn row() -> gpui_kit::Div {
    row_for_density(Density::Desktop)
}

/// Density-aware row respecting Apple HIG touch targets on mobile (44pt+).
pub fn row_for_density(density: Density) -> gpui_kit::Div {
    div()
        .flex()
        .items_center()
        .gap_3()
        .px(density.pad_x())
        .py(density.pad_y())
        .min_h(density.row_height())
}

/// Elevated surface card frame (`16px` radius, surface fill, subtle border).
pub fn surface_card(palette: Palette) -> gpui_kit::Div {
    div()
        .rounded(px(16.))
        .bg(rgba(palette.surface))
        .border_1()
        .border_color(rgba(tint(palette.border, 0x77)))
}

pub fn card(body: AnyElement, _palette: Palette) -> AnyElement {
    Card::new().child(body).into_any_element()
}

/// Framed chart card with title, optional subtitle, and Ely card surface.
pub fn chart_card(
    title: impl Into<SharedString>,
    subtitle: Option<SharedString>,
    body: AnyElement,
    _palette: Palette,
) -> AnyElement {
    let mut header = CardHeader::new(title);
    if let Some(sub) = subtitle {
        header = header.description(sub);
    }
    Card::new().header(header).child(body).into_any_element()
}

pub fn progress_bar(progress: f32, color: u32, palette: Palette) -> AnyElement {
    let width = 48.0;
    let clamped = progress.clamp(0.0, 1.0);
    div()
        .w(px(width))
        .h(px(4.))
        .rounded_full()
        .bg(rgba(palette.border))
        .child(
            div()
                .h_full()
                .rounded_full()
                .bg(rgb(color))
                .w(px(width * clamped)),
        )
        .into_any_element()
}

pub fn back_button(
    label: impl Into<SharedString>,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    _palette: Palette,
) -> AnyElement {
    Button::new("back", label)
        .variant(ButtonVariant::Link)
        .size(ControlSize::Sm)
        .on_click(on_click)
        .into_any_element()
}

/// Compact horizontal metric cell (`label`, `value`, optional `detail`, semantic `tone`).
pub fn stat_card(
    label: impl Into<SharedString>,
    value: impl Into<SharedString>,
    detail: Option<SharedString>,
    tone: u32,
    palette: Palette,
) -> AnyElement {
    div()
        .flex_1()
        .min_w(px(120.))
        .px_4()
        .py_3p5()
        .rounded(px(14.))
        .bg(rgba(palette.surface))
        .border_1()
        .border_color(rgba(tint(palette.border, 0x77)))
        .flex()
        .flex_col()
        .gap_1p5()
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(palette.muted))
                .child(label.into()),
        )
        .child(
            div()
                .text_2xl()
                .font_weight(FontWeight::BOLD)
                .font_family(FONT_MONO_TABULAR)
                .text_color(rgb(tone))
                .child(value.into()),
        )
        .when_some(detail, |el, det| {
            el.child(
                div()
                    .truncate()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child(det),
            )
        })
        .into_any_element()
}

/// Content, styling state, and semantic color for an interactive metric card.
pub struct InteractiveStatCard {
    pub id: ElementId,
    pub label: SharedString,
    pub value: SharedString,
    pub detail: Option<SharedString>,
    pub tone: u32,
    pub active: bool,
    pub palette: Palette,
}

/// Render an interactive horizontal metric cell.
pub fn stat_card_interactive(
    card: InteractiveStatCard,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let InteractiveStatCard {
        id,
        label,
        value,
        detail,
        tone,
        active,
        palette,
    } = card;
    div()
        .id(id)
        .flex_1()
        .min_w(px(120.))
        .p_3()
        .rounded(px(12.))
        .bg(rgba(if active {
            tint(palette.accent, 0x18)
        } else {
            palette.surface
        }))
        .border_1()
        .border_color(if active {
            rgb(palette.accent)
        } else {
            rgba(tint(palette.border, 0x66))
        })
        .cursor_pointer()
        .hover(move |s| {
            s.bg(rgba(if active {
                tint(palette.accent, 0x24)
            } else {
                palette.raised
            }))
            .border_color(if active {
                rgb(palette.accent)
            } else {
                rgba(tint(palette.accent, 0x44))
            })
        })
        .on_click(on_click)
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(if active {
                            palette.accent
                        } else {
                            palette.muted
                        }))
                        .child(label),
                )
                .when(active, |el| {
                    el.child(
                        div()
                            .px_1()
                            .py(px(1.))
                            .rounded(px(4.))
                            .bg(rgba(tint(palette.accent, 0x24)))
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(palette.accent))
                            .child("Filtered"),
                    )
                }),
        )
        .child(
            div()
                .text_lg()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(tone))
                .child(value),
        )
        .when_some(detail, |el, det| {
            el.child(
                div()
                    .text_xs()
                    .text_color(rgb(if active { palette.text } else { palette.muted }))
                    .child(det),
            )
        })
        .into_any_element()
}

/// Horizontal strip of [`stat_card`] metrics.
pub fn stat_strip(cards: Vec<AnyElement>) -> AnyElement {
    div()
        .flex()
        .flex_wrap()
        .items_stretch()
        .gap_3()
        .children(cards)
        .into_any_element()
}

/// Tactile dashboard metric tile with large tabular value and quiet caption.
pub fn dashboard_metric_tile(
    card: InteractiveStatCard,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let InteractiveStatCard {
        id,
        label,
        value,
        detail,
        tone,
        active,
        palette,
    } = card;
    div()
        .id(id)
        .flex_1()
        .min_w(px(136.))
        .px_4()
        .py_3p5()
        .rounded(px(14.))
        .bg(rgba(if active {
            tint(palette.accent, 0x1a)
        } else {
            palette.surface
        }))
        .border_1()
        .border_color(if active {
            rgb(palette.accent)
        } else {
            rgba(tint(palette.border, 0x77))
        })
        .cursor_pointer()
        .hover(move |s| {
            s.bg(rgba(if active {
                tint(palette.accent, 0x24)
            } else {
                palette.raised
            }))
            .border_color(if active {
                rgb(palette.accent)
            } else {
                rgba(tint(palette.accent, 0x55))
            })
        })
        .on_click(on_click)
        .flex()
        .flex_col()
        .gap_1p5()
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(if active {
                            palette.accent
                        } else {
                            palette.muted
                        }))
                        .child(label),
                )
                .when(active, |el| {
                    el.child(
                        div()
                            .px_1p5()
                            .py(px(1.))
                            .rounded_full()
                            .bg(rgba(tint(palette.accent, 0x26)))
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(palette.accent))
                            .child("Active"),
                    )
                }),
        )
        .child(
            div()
                .text_2xl()
                .font_weight(FontWeight::BOLD)
                .font_family(FONT_MONO_TABULAR)
                .text_color(rgb(tone))
                .child(value),
        )
        .when_some(detail, |el, det| {
            el.child(
                div()
                    .truncate()
                    .text_xs()
                    .text_color(rgb(if active { palette.text } else { palette.muted }))
                    .child(det),
            )
        })
        .into_any_element()
}

/// Circular icon-first quick action with caption underneath.
pub struct QuickAction {
    pub id: ElementId,
    pub icon: IconName,
    pub label: SharedString,
    pub tooltip: SharedString,
    pub active: bool,
    pub tone: Option<u32>,
    pub palette: Palette,
}

pub fn quick_action_button(
    action: QuickAction,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let QuickAction {
        id,
        icon,
        label,
        tooltip,
        active,
        tone,
        palette,
    } = action;
    let accent = tone.unwrap_or(palette.accent);
    div()
        .id(id)
        .flex()
        .flex_col()
        .items_center()
        .gap_1p5()
        .min_w(px(64.))
        .cursor_pointer()
        .tooltip(Tooltip::text(tooltip))
        .on_click(on_click)
        .child(
            div()
                .size(px(42.))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(rgba(if active {
                    tint(accent, 0x28)
                } else {
                    palette.raised
                }))
                .border_1()
                .border_color(if active {
                    rgb(accent)
                } else {
                    rgba(tint(palette.border, 0x88))
                })
                .text_color(rgb(if active { accent } else { palette.text }))
                .hover(move |s| {
                    s.bg(rgba(if active {
                        tint(accent, 0x36)
                    } else {
                        tint(palette.accent, 0x18)
                    }))
                    .border_color(rgb(accent))
                })
                .child(div().text_base().child(icon)),
        )
        .child(
            div()
                .text_xs()
                .font_weight(if active {
                    FontWeight::SEMIBOLD
                } else {
                    FontWeight::MEDIUM
                })
                .text_color(rgb(if active { accent } else { palette.muted }))
                .child(label),
        )
        .into_any_element()
}
