//! Shared navigation primitives for mobile bottom tabs and headers.

use gpui_kit::{prelude::*, AnyElement, SharedString, Window};

use crate::ely::navigation::{BottomTabBar, BottomTabItem, NavHeader};
use crate::theme::Palette;

/// An item in the mobile bottom navigation bar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavItem {
    pub id: &'static str,
    pub label: &'static str,
    pub icon: &'static str,
    pub badge: Option<usize>,
}

/// Renders a touch-friendly mobile bottom tab bar respecting Apple HIG (44pt+ minimum height).
pub fn mobile_tab_bar<F>(
    items: &[NavItem],
    active_id: &str,
    _palette: Palette,
    on_select: F,
) -> AnyElement
where
    F: Fn(&'static str, &mut Window, &mut gpui_kit::App) + 'static + Clone,
{
    let static_ids: Vec<&'static str> = items.iter().map(|item| item.id).collect();
    let tab_items = items
        .iter()
        .map(|item| BottomTabItem::new(item.id, item.icon, item.label).badge(item.badge));

    BottomTabBar::new(
        "mobile-tab-bar",
        tab_items,
        SharedString::from(active_id.to_owned()),
    )
    .on_select(move |selected, window, cx| {
        if let Some(static_id) = static_ids
            .iter()
            .copied()
            .find(|id| *id == selected.as_ref())
        {
            on_select(static_id, window, cx);
        }
    })
    .into_any_element()
}

/// Renders a top navigation bar (for mobile detail views or compact headers).
pub fn nav_header<B, A>(
    title: impl Into<SharedString>,
    subtitle: Option<impl Into<SharedString>>,
    _palette: Palette,
    back_action: Option<B>,
    trailing_actions: Option<A>,
) -> AnyElement
where
    B: Fn(&mut Window, &mut gpui_kit::App) + 'static,
    A: IntoElement + 'static,
{
    let mut header = NavHeader::new(title);
    if let Some(sub) = subtitle {
        header = header.subtitle(sub);
    }
    if let Some(on_back) = back_action {
        header = header.on_back(on_back);
    }
    if let Some(actions) = trailing_actions {
        header = header.trailing(actions);
    }
    header.into_any_element()
}
