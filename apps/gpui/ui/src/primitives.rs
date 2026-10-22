//! Shared UI primitives: sections, framed lists, cards, badges, pills, progress bars, and density-aware rows.

use gpui_kit::{
    div, prelude::*, px, rgb, rgba, AnyElement, App, ClickEvent, FontWeight, SharedString, Window,
};

use crate::theme::{
    AccentColor, AppearanceSettings, BackgroundTone, Density, GlassStyle, Palette, ThemeMode,
    WindowOpacity,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DueStatus {
    Overdue(String),
    Today,
    Tomorrow,
    Upcoming(String),
    None,
}

pub fn due_status(due: Option<&str>, done: bool, today: &str) -> DueStatus {
    let Some(due) = due.filter(|d| !d.is_empty()) else {
        return DueStatus::None;
    };
    if done {
        return DueStatus::Upcoming(due.to_string());
    }
    if due < today {
        DueStatus::Overdue(due.to_string())
    } else if due == today {
        DueStatus::Today
    } else {
        if let (Ok(d), Ok(t)) = (
            chrono::NaiveDate::parse_from_str(due, "%Y-%m-%d"),
            chrono::NaiveDate::parse_from_str(today, "%Y-%m-%d"),
        ) {
            if d == t + chrono::Duration::days(1) {
                return DueStatus::Tomorrow;
            }
        }
        DueStatus::Upcoming(due.to_string())
    }
}

pub fn section(title: &'static str, body: AnyElement, palette: Palette) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(palette.muted))
                .child(title),
        )
        .child(body)
}

pub fn list(rows: Vec<AnyElement>, palette: Palette) -> AnyElement {
    let count = rows.len();
    let mut list = div()
        .flex()
        .flex_col()
        .rounded(px(12.))
        .bg(rgba(palette.surface))
        .border_1()
        .border_color(rgba(palette.border))
        .overflow_hidden();
    for (index, row) in rows.into_iter().enumerate() {
        list = list.child(row);
        if index + 1 < count {
            list = list.child(div().h(px(1.)).mx_4().bg(rgba(palette.border)));
        }
    }
    list.into_any_element()
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

pub fn card(body: AnyElement, palette: Palette) -> AnyElement {
    div()
        .rounded(px(12.))
        .bg(rgba(palette.surface))
        .border_1()
        .border_color(rgba(palette.border))
        .p_4()
        .child(body)
        .into_any_element()
}

pub fn dot(color: u32) -> impl IntoElement {
    div().flex_none().size(px(7.)).rounded_full().bg(rgb(color))
}

pub fn note(text: impl Into<SharedString>, color: u32) -> AnyElement {
    div()
        .text_sm()
        .text_color(rgb(color))
        .child(text.into())
        .into_any_element()
}

/// Reserves the final height while the first answer is on its way, so nothing jumps.
pub fn placeholder(height: f32) -> AnyElement {
    div().h(px(height)).into_any_element()
}

pub fn filter_pill(
    label: &'static str,
    count: usize,
    active: bool,
    palette: Palette,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .id(SharedString::from(format!("filter-pill-{label}")))
        .flex()
        .items_center()
        .gap_1()
        .px_2()
        .py(px(2.))
        .rounded(px(6.))
        .text_xs()
        .cursor_pointer()
        .when(active, |el| {
            el.bg(rgba(palette.accent & 0xffffff | 0x22000000))
                .text_color(rgb(palette.accent))
                .font_weight(FontWeight::SEMIBOLD)
        })
        .when(!active, |el| {
            el.bg(rgba(palette.raised))
                .text_color(rgb(palette.muted))
                .hover(|s| s.text_color(rgb(palette.text)))
        })
        .on_click(on_click)
        .child(label)
        .child(
            div()
                .opacity(if active { 1.0 } else { 0.7 })
                .child(format!("({count})")),
        )
        .into_any_element()
}

pub fn priority_pill(priority: u8, palette: Palette) -> AnyElement {
    let (bg_color, border_color, text_color, label) = match priority {
        1 => (
            palette.red & 0xffffff | 0x20000000,
            palette.red & 0xffffff | 0x40000000,
            palette.red,
            "P1",
        ),
        2 => (
            palette.amber & 0xffffff | 0x18000000,
            palette.amber & 0xffffff | 0x35000000,
            palette.amber,
            "P2",
        ),
        _ => (palette.raised, palette.border, palette.muted, "P3"),
    };
    div()
        .px_2()
        .py(px(1.5))
        .rounded(px(4.))
        .bg(rgba(bg_color))
        .border_1()
        .border_color(rgba(border_color))
        .text_color(rgb(text_color))
        .text_xs()
        .font_weight(if priority == 1 {
            FontWeight::SEMIBOLD
        } else {
            FontWeight::MEDIUM
        })
        .child(label)
        .into_any_element()
}

pub fn due_badge(due: Option<&str>, done: bool, today: &str, palette: Palette) -> AnyElement {
    match due_status(due, done, today) {
        DueStatus::Today => div()
            .px_2()
            .py(px(1.5))
            .rounded(px(4.))
            .bg(rgba(palette.amber & 0xffffff | 0x22000000))
            .border_1()
            .border_color(rgba(palette.amber & 0xffffff | 0x38000000))
            .text_color(rgb(palette.amber))
            .text_xs()
            .font_weight(FontWeight::MEDIUM)
            .child("Today")
            .into_any_element(),
        DueStatus::Tomorrow => div()
            .px_2()
            .py(px(1.5))
            .rounded(px(4.))
            .bg(rgba(palette.raised))
            .border_1()
            .border_color(rgba(palette.border))
            .text_color(rgb(palette.text))
            .text_xs()
            .font_weight(FontWeight::MEDIUM)
            .child("Tomorrow")
            .into_any_element(),
        DueStatus::Overdue(d) => div()
            .px_2()
            .py(px(1.5))
            .rounded(px(4.))
            .bg(rgba(palette.red & 0xffffff | 0x22000000))
            .border_1()
            .border_color(rgba(palette.red & 0xffffff | 0x38000000))
            .text_color(rgb(palette.red))
            .text_xs()
            .font_weight(FontWeight::SEMIBOLD)
            .child(format!("Overdue · {d}"))
            .into_any_element(),
        DueStatus::Upcoming(d) => div()
            .px_2()
            .py(px(1.5))
            .rounded(px(4.))
            .bg(rgba(palette.raised))
            .border_1()
            .border_color(rgba(palette.border))
            .text_color(rgb(palette.muted))
            .text_xs()
            .child(d)
            .into_any_element(),
        DueStatus::None => div()
            .text_sm()
            .text_color(rgb(palette.muted))
            .child("—")
            .into_any_element(),
    }
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
    palette: Palette,
) -> AnyElement {
    div()
        .id("back")
        .text_sm()
        .text_color(rgb(palette.accent))
        .cursor_pointer()
        .hover(move |s| s.text_color(rgb(palette.text)))
        .on_click(on_click)
        .child(label.into())
        .into_any_element()
}

/// Badge showing sync and offline mutation queue status.
pub fn sync_status_badge(pending: usize, syncing: bool, palette: Palette) -> AnyElement {
    let (text, color, bg) = if syncing {
        (
            "Syncing…",
            palette.accent,
            palette.accent & 0xffffff | 0x20000000,
        )
    } else if pending > 0 {
        (
            "Offline queue",
            palette.amber,
            palette.amber & 0xffffff | 0x20000000,
        )
    } else {
        (
            "Synced",
            palette.green,
            palette.green & 0xffffff | 0x20000000,
        )
    };
    div()
        .flex()
        .items_center()
        .gap_1()
        .px_2()
        .py(px(2.))
        .rounded(px(4.))
        .bg(rgba(bg))
        .text_xs()
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(color))
        .child(dot(color))
        .child(if pending > 0 && !syncing {
            format!("{text} ({pending})")
        } else {
            text.to_string()
        })
        .into_any_element()
}

/// Renders an interactive panel to control window background, transparency and glass effects.
pub fn appearance_panel<F, C>(
    settings: AppearanceSettings,
    palette: Palette,
    on_change: F,
    on_close: C,
) -> AnyElement
where
    F: Fn(AppearanceSettings, &mut Window, &mut App) + 'static + Clone,
    C: Fn(&mut Window, &mut App) + 'static + Clone,
{
    let mut panel = div()
        .id("appearance-panel")
        .flex()
        .flex_col()
        .gap_3()
        .p_4()
        .w(px(360.))
        .rounded(px(12.))
        .bg(rgba(palette.surface))
        .border_1()
        .border_color(rgba(palette.border));

    // Header with Close
    let close = on_close.clone();
    let header = div()
        .flex()
        .items_center()
        .justify_between()
        .pb_1()
        .border_b_1()
        .border_color(rgba(palette.border))
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(palette.text))
                .child("Appearance & Display"),
        )
        .child(
            div()
                .id("close-appearance")
                .text_xs()
                .cursor_pointer()
                .px_2()
                .py(px(2.))
                .rounded(px(4.))
                .bg(rgba(palette.raised))
                .text_color(rgb(palette.muted))
                .hover(|s| s.text_color(rgb(palette.text)))
                .on_click(move |_, window, app| close(window, app))
                .child("✕"),
        );
    panel = panel.child(header);

    // Section 1: Theme Mode
    let mut theme_row = div().flex().items_center().gap_1();
    for mode in ThemeMode::ALL {
        let is_active = settings.theme_mode == mode;
        let change = on_change.clone();
        let new_settings = AppearanceSettings {
            theme_mode: mode,
            ..settings
        };
        theme_row = theme_row.child(
            div()
                .id(SharedString::from(format!("theme-mode-{}", mode.label())))
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .py(px(4.))
                .px_2()
                .rounded(px(6.))
                .text_xs()
                .cursor_pointer()
                .when(is_active, |el| {
                    el.bg(rgba(palette.accent & 0xffffff | 0x24000000))
                        .text_color(rgb(palette.accent))
                        .font_weight(FontWeight::SEMIBOLD)
                        .border_1()
                        .border_color(rgba(palette.accent & 0xffffff | 0x48000000))
                })
                .when(!is_active, |el| {
                    el.bg(rgba(palette.raised))
                        .text_color(rgb(palette.muted))
                        .hover(|s| s.text_color(rgb(palette.text)))
                })
                .on_click(move |_, window, app| change(new_settings, window, app))
                .child(mode.label()),
        );
    }
    panel = panel.child(section("THEME MODE", theme_row.into_any_element(), palette));

    // Section 2: Accent Color Swatches
    let mut accent_row = div().flex().items_center().justify_between().gap_2();
    for accent in AccentColor::ALL {
        let is_active = settings.accent == accent;
        let change = on_change.clone();
        let new_settings = AppearanceSettings { accent, ..settings };
        let color = accent.swatch_rgb();
        accent_row = accent_row.child(
            div()
                .id(SharedString::from(format!(
                    "accent-swatch-{}",
                    accent.label()
                )))
                .flex()
                .flex_col()
                .items_center()
                .gap_1()
                .cursor_pointer()
                .child(
                    div()
                        .size(px(22.))
                        .rounded_full()
                        .bg(rgb(color))
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(is_active, |el| {
                            el.border_2().border_color(rgb(palette.text)).shadow_sm()
                        })
                        .child(
                            div()
                                .size(px(6.))
                                .rounded_full()
                                .when(is_active, |dot| dot.bg(rgb(0xffffff))),
                        ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(if is_active {
                            rgb(palette.accent)
                        } else {
                            rgb(palette.muted)
                        })
                        .font_weight(if is_active {
                            FontWeight::SEMIBOLD
                        } else {
                            FontWeight::NORMAL
                        })
                        .child(accent.label()),
                )
                .on_click(move |_, window, app| change(new_settings, window, app)),
        );
    }
    panel = panel.child(section(
        "ACCENT COLOR",
        accent_row.into_any_element(),
        palette,
    ));

    // Section 3: Opacity / Transparency
    let mut opacity_row = div().flex().items_center().gap_1();
    for op in WindowOpacity::ALL {
        let is_active = settings.opacity == op;
        let change = on_change.clone();
        let new_settings = AppearanceSettings {
            opacity: op,
            ..settings
        };
        opacity_row = opacity_row.child(
            div()
                .id(SharedString::from(format!("op-pill-{}", op.short_label())))
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .py(px(4.))
                .px_2()
                .rounded(px(6.))
                .text_xs()
                .cursor_pointer()
                .when(is_active, |el| {
                    el.bg(rgba(palette.accent & 0xffffff | 0x24000000))
                        .text_color(rgb(palette.accent))
                        .font_weight(FontWeight::SEMIBOLD)
                        .border_1()
                        .border_color(rgba(palette.accent & 0xffffff | 0x48000000))
                })
                .when(!is_active, |el| {
                    el.bg(rgba(palette.raised))
                        .text_color(rgb(palette.muted))
                        .hover(|s| s.text_color(rgb(palette.text)))
                })
                .on_click(move |_, window, app| change(new_settings, window, app))
                .child(op.short_label()),
        );
    }
    panel = panel.child(section(
        "TRANSPARENCY",
        opacity_row.into_any_element(),
        palette,
    ));

    // Section 4: Sidebar Glass
    let mut glass_row = div().flex().items_center().gap_1();
    for style in GlassStyle::ALL {
        let is_active = settings.glass == style;
        let change = on_change.clone();
        let new_settings = AppearanceSettings {
            glass: style,
            ..settings
        };
        glass_row = glass_row.child(
            div()
                .id(SharedString::from(format!("glass-pill-{}", style.label())))
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .py(px(4.))
                .px_2()
                .rounded(px(6.))
                .text_xs()
                .cursor_pointer()
                .when(is_active, |el| {
                    el.bg(rgba(palette.accent & 0xffffff | 0x24000000))
                        .text_color(rgb(palette.accent))
                        .font_weight(FontWeight::SEMIBOLD)
                        .border_1()
                        .border_color(rgba(palette.accent & 0xffffff | 0x48000000))
                })
                .when(!is_active, |el| {
                    el.bg(rgba(palette.raised))
                        .text_color(rgb(palette.muted))
                        .hover(|s| s.text_color(rgb(palette.text)))
                })
                .on_click(move |_, window, app| change(new_settings, window, app))
                .child(style.label()),
        );
    }
    panel = panel.child(section(
        "SIDEBAR GLASS",
        glass_row.into_any_element(),
        palette,
    ));

    // Section 5: Background Tone with Preview Dots
    let mut tone_row = div().flex().items_center().gap_1();
    for tone in BackgroundTone::ALL {
        let is_active = settings.tone == tone;
        let change = on_change.clone();
        let new_settings = AppearanceSettings { tone, ..settings };
        let preview_rgb = tone.tint_rgb(true).unwrap_or(0x10131a);
        tone_row = tone_row.child(
            div()
                .id(SharedString::from(format!("tone-pill-{}", tone.label())))
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .gap_1_5()
                .py(px(4.))
                .px_2()
                .rounded(px(6.))
                .text_xs()
                .cursor_pointer()
                .when(is_active, |el| {
                    el.bg(rgba(palette.accent & 0xffffff | 0x24000000))
                        .text_color(rgb(palette.accent))
                        .font_weight(FontWeight::SEMIBOLD)
                        .border_1()
                        .border_color(rgba(palette.accent & 0xffffff | 0x48000000))
                })
                .when(!is_active, |el| {
                    el.bg(rgba(palette.raised))
                        .text_color(rgb(palette.muted))
                        .hover(|s| s.text_color(rgb(palette.text)))
                })
                .child(
                    div()
                        .size(px(8.))
                        .rounded_full()
                        .bg(rgb(preview_rgb))
                        .border_1()
                        .border_color(rgba(palette.border)),
                )
                .child(tone.label())
                .on_click(move |_, window, app| change(new_settings, window, app)),
        );
    }
    panel = panel.child(section(
        "BACKGROUND TONE",
        tone_row.into_any_element(),
        palette,
    ));

    // Section 6: UI Scale & Density
    let mut density_row = div().flex().items_center().gap_1();
    for den in Density::DESKTOP_ALL {
        let is_active = settings.density == den;
        let change = on_change.clone();
        let new_settings = AppearanceSettings {
            density: den,
            ..settings
        };
        density_row = density_row.child(
            div()
                .id(SharedString::from(format!("density-pill-{}", den.label())))
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .py(px(4.))
                .px_2()
                .rounded(px(6.))
                .text_xs()
                .cursor_pointer()
                .when(is_active, |el| {
                    el.bg(rgba(palette.accent & 0xffffff | 0x24000000))
                        .text_color(rgb(palette.accent))
                        .font_weight(FontWeight::SEMIBOLD)
                        .border_1()
                        .border_color(rgba(palette.accent & 0xffffff | 0x48000000))
                })
                .when(!is_active, |el| {
                    el.bg(rgba(palette.raised))
                        .text_color(rgb(palette.muted))
                        .hover(|s| s.text_color(rgb(palette.text)))
                })
                .on_click(move |_, window, app| change(new_settings, window, app))
                .child(den.label()),
        );
    }
    panel = panel.child(section(
        "LAYOUT DENSITY",
        density_row.into_any_element(),
        palette,
    ));

    panel.into_any_element()
}
