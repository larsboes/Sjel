//! Shared navigation primitives for mobile bottom tabs and headers.

use gpui_kit::{div, prelude::*, px, rgb, rgba, AnyElement, FontWeight, SharedString, Window};

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
    palette: Palette,
    on_select: F,
) -> AnyElement
where
    F: Fn(&'static str, &mut Window, &mut gpui_kit::App) + 'static + Clone,
{
    let mut bar = div()
        .flex()
        .items_center()
        .justify_around()
        .h(px(56.))
        .bg(rgba(palette.surface))
        .border_t_1()
        .border_color(rgba(palette.border))
        .px_2();

    for (idx, item) in items.iter().enumerate() {
        let is_active = item.id == active_id;
        let select = on_select.clone();
        let item_id = item.id;

        let mut tab_button = div()
            .id(SharedString::from(format!("nav-tab-{idx}")))
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .min_w(px(64.))
            .min_h(px(44.))
            .cursor_pointer()
            .gap_1()
            .on_click(move |_, window, app| {
                select(item_id, window, app);
            });

        // Icon + optional badge
        let mut icon_container = div().relative().flex().items_center().justify_center();

        let icon_el = div()
            .text_sm()
            .font_weight(if is_active {
                FontWeight::BOLD
            } else {
                FontWeight::MEDIUM
            })
            .text_color(if is_active {
                rgb(palette.accent)
            } else {
                rgb(palette.muted)
            })
            .child(item.icon);

        icon_container = icon_container.child(icon_el);

        if let Some(count) = item.badge.filter(|c| *c > 0) {
            let badge = div()
                .absolute()
                .top(px(-4.))
                .right(px(-10.))
                .min_w(px(16.))
                .h(px(16.))
                .px(px(4.))
                .rounded_full()
                .bg(rgb(palette.amber))
                .flex()
                .items_center()
                .justify_center()
                .text_color(rgb(0xffffff))
                .text_xs()
                .font_weight(FontWeight::BOLD)
                .child(format!("{count}"));
            icon_container = icon_container.child(badge);
        }

        let label_el = div()
            .text_xs()
            .font_weight(if is_active {
                FontWeight::SEMIBOLD
            } else {
                FontWeight::NORMAL
            })
            .text_color(if is_active {
                rgb(palette.accent)
            } else {
                rgb(palette.muted)
            })
            .child(item.label);

        tab_button = tab_button.child(icon_container).child(label_el);
        bar = bar.child(tab_button);
    }

    bar.into_any_element()
}

/// Renders a top navigation bar (for mobile detail views or compact headers).
pub fn nav_header<B, A>(
    title: impl Into<SharedString>,
    subtitle: Option<impl Into<SharedString>>,
    palette: Palette,
    back_action: Option<B>,
    trailing_actions: Option<A>,
) -> AnyElement
where
    B: Fn(&mut Window, &mut gpui_kit::App) + 'static,
    A: IntoElement + 'static,
{
    let mut header = div()
        .flex()
        .items_center()
        .justify_between()
        .px_4()
        .py_3()
        .bg(rgba(palette.surface))
        .border_b_1()
        .border_color(rgba(palette.border));

    // Left (Back button or placeholder)
    if let Some(on_back) = back_action {
        header = header.child(
            div()
                .id("nav-back")
                .flex()
                .items_center()
                .gap_1()
                .cursor_pointer()
                .min_h(px(44.))
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(palette.accent))
                .child("← Back")
                .on_click(move |_, window, app| {
                    on_back(window, app);
                }),
        );
    } else {
        header = header.child(div().w(px(60.)));
    }

    // Center: Title + Subtitle
    let mut center = div().flex().flex_col().items_center().child(
        div()
            .text_sm()
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(rgb(palette.text))
            .child(title.into()),
    );

    if let Some(sub) = subtitle {
        center = center.child(
            div()
                .text_xs()
                .text_color(rgb(palette.muted))
                .child(sub.into()),
        );
    }
    header = header.child(center);

    // Right: Trailing actions
    if let Some(actions) = trailing_actions {
        header = header.child(div().flex().items_center().gap_2().child(actions));
    } else {
        header = header.child(div().w(px(60.)));
    }

    header.into_any_element()
}
