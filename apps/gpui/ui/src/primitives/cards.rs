//! Card frames, lists, density rows, progress bars, and metrics.

use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::{
    div, list as gpui_list, prelude::*, px, rgb, rgba, AnyElement, App, ClickEvent, ElementId,
    ListState, SharedString, Window,
};

use crate::ely::{
    buttons::{Button, ButtonVariant},
    dashboard::{MetricStrip, MetricTile, QuickActionTile},
    data_display::ProgressBar,
    layout::{Card, CardHeader, Section},
    lists::List,
    primitives::{Icon, IconName},
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

pub fn progress_bar(progress: f32, color: u32, _palette: Palette) -> AnyElement {
    ProgressBar::new(progress)
        .color(rgb(color))
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
    _palette: Palette,
) -> AnyElement {
    let label = label.into();
    let mut tile = MetricTile::new(label.clone(), label, value).color(rgb(tone));
    if let Some(det) = detail {
        tile = tile.detail(det);
    }
    tile.into_any_element()
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

/// Horizontal strip of [`stat_card`] metrics.
pub fn stat_strip(cards: Vec<AnyElement>) -> AnyElement {
    MetricStrip::new().children(cards).into_any_element()
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
        palette: _,
    } = card;
    let mut tile = MetricTile::new(id, label, value)
        .color(rgb(tone))
        .active(active)
        .min_w(px(136.))
        .on_click(on_click);
    if let Some(det) = detail {
        tile = tile.detail(det);
    }
    tile.into_any_element()
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
    QuickActionTile::with_element(id, div().text_base().child(Icon::new(icon)), label)
        .tooltip(tooltip)
        .active(active)
        .color(rgb(accent))
        .on_click(on_click)
        .into_any_element()
}
