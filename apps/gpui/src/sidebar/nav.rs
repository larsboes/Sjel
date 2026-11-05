use gpui_kit::component::kbd::Kbd;
use gpui_kit::{div, prelude::*, px, rgb, rgba, Action, AnyElement, Context, IntoElement, Window};
use sjel_gpui_core::area::Area;

use crate::route::Page;
use crate::workspace::{OpenPalette, OpenSettings, ToggleSoundscapePanel, Workspace};
use sjel_gpui_ui::theme::{tint, Palette};

/// One row per area and, when it is away from its default tab, the tab it will return to.
pub fn nav(view: &Workspace, palette: Palette, cx: &mut Context<Workspace>) -> Vec<AnyElement> {
    Area::ALL
        .into_iter()
        .map(|area| {
            let selected = Area::of(view.page) == Some(area);
            let tab = view.tabs.current(area);
            div()
                .id(area.label())
                .flex()
                .flex_col()
                .gap(px(2.))
                .px_3()
                .py(px(5.))
                .rounded(px(7.))
                .when(selected, |row| {
                    row.bg(rgba(tint(palette.accent, 0x18)))
                        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                })
                .text_color(rgb(if selected {
                    palette.text
                } else {
                    palette.muted
                }))
                .cursor_pointer()
                .hover(move |style| style.bg(rgba(palette.raised)).text_color(rgb(palette.text)))
                .on_click(cx.listener(move |view, _, _, cx| view.open_area(area, cx)))
                .child(div().flex().items_center().child(area.label()))
                // The tab the area key returns to, shown only when it differs from the open one.
                .when(
                    area.has_tabs() && tab != area.default_page() && !selected,
                    |row| {
                        row.child(
                            div().flex().items_center().child(
                                div()
                                    .px_2()
                                    .py(px(1.))
                                    .rounded(px(4.))
                                    .bg(rgba(tint(palette.accent, 0x1c)))
                                    .text_color(rgb(palette.accent))
                                    .text_xs()
                                    .child(tab.label()),
                            ),
                        )
                    },
                )
                .into_any_element()
        })
        .collect()
}

/// The mini player, shown only while the soundscape capability answers or has answered.
pub fn soundscape(
    view: &Workspace,
    palette: Palette,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
    let state = view.soundscape.value.as_ref();
    let is_up = view
        .health
        .value
        .as_ref()
        .map(|h| {
            h.capabilities
                .get("soundscape")
                .map(|c| c.up)
                .unwrap_or(false)
        })
        .unwrap_or(false);

    if !is_up && state.is_none() {
        return div().into_any_element();
    }

    let playing = state.map(|s| s.playing).unwrap_or(false);
    let preset = state.map(|s| s.preset.as_str()).unwrap_or("edm");
    // Whether the panel is on screen is the one thing this row cannot show by itself, and it is
    // what decides whether a click is "show me the player" or "it is already there".
    let panel_open = view
        .soundscape_panel
        .as_ref()
        .map(|panel| panel.read(cx).is_open())
        .unwrap_or(false);

    div()
        .id("soundscape-mini-player")
        .mx_1()
        .mb_1()
        .px_2()
        .py(px(5.))
        .rounded(px(7.))
        .bg(rgba(if panel_open {
            tint(palette.accent, 0x1c)
        } else {
            palette.raised
        }))
        .flex()
        .items_center()
        .justify_between()
        .gap_2()
        .text_xs()
        .child(
            div()
                .flex()
                .items_center()
                .gap_1()
                .min_w_0()
                .child(equalizer_indicator(playing, palette))
                .child(
                    div()
                        .id("soundscape-preset-btn")
                        .cursor_pointer()
                        .truncate()
                        .text_color(rgb(palette.text))
                        .hover(move |s| s.text_color(rgb(palette.accent)))
                        .on_click(cx.listener(|view, _, window, cx| {
                            // The row is the way into the player: the strip can cycle a preset, but
                            // only the panel shows what is playing.
                            view.open_soundscape(window, cx);
                            view.cycle_soundscape_preset(window, cx);
                        }))
                        .child(format!("♪ {preset}")),
                ),
        )
        .child(
            div()
                .id("soundscape-toggle-btn")
                .flex_none()
                .px_2()
                .py(px(2.))
                .rounded(px(4.))
                .bg(rgba(if playing {
                    tint(palette.accent, 0x24)
                } else {
                    palette.raised
                }))
                .text_color(rgb(if playing {
                    palette.accent
                } else {
                    palette.muted
                }))
                .cursor_pointer()
                .hover(move |s| s.text_color(rgb(palette.text)))
                .on_click(cx.listener(move |view, _, window, cx| {
                    view.open_soundscape(window, cx);
                    view.toggle_soundscape(!playing, window, cx);
                }))
                .child(if playing { "Pause" } else { "Play" }),
        )
        // The player is ⌘0, listed after the nine areas.
        .children(
            Kbd::binding_for_action(&ToggleSoundscapePanel, None, window)
                .map(|kbd| kbd.appearance(false)),
        )
        .into_any_element()
}

pub fn equalizer_indicator(playing: bool, palette: Palette) -> impl IntoElement {
    if !playing {
        return div()
            .flex_none()
            .size(px(6.))
            .rounded_full()
            .bg(rgb(palette.muted))
            .into_any_element();
    }
    div()
        .flex_none()
        .flex()
        .items_end()
        .gap(px(1.5))
        .h(px(8.))
        .w(px(8.))
        .child(
            div()
                .w(px(1.5))
                .h(px(5.))
                .rounded(px(0.5))
                .bg(rgb(palette.green)),
        )
        .child(
            div()
                .w(px(1.5))
                .h(px(8.))
                .rounded(px(0.5))
                .bg(rgb(palette.accent)),
        )
        .child(
            div()
                .w(px(1.5))
                .h(px(4.))
                .rounded(px(0.5))
                .bg(rgb(palette.green)),
        )
        .into_any_element()
}

pub fn palette_hint(palette: Palette, window: &mut Window) -> impl IntoElement {
    div()
        .id("palette-hint")
        .flex()
        .items_center()
        .justify_between()
        .px_3()
        .py(px(6.))
        .rounded(px(7.))
        .text_sm()
        .text_color(rgb(palette.muted))
        .cursor_pointer()
        .hover(move |style| style.bg(rgba(palette.raised)))
        .on_click(|_, window, cx| window.dispatch_action(OpenPalette.boxed_clone(), cx))
        .child("Search or ask")
        .children(Kbd::binding_for_action(&OpenPalette, None, window).map(|k| k.appearance(false)))
}

pub fn settings_row(
    view: &Workspace,
    palette: Palette,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
    let selected = view.page == Page::Settings;
    div()
        .id("settings-row")
        .flex()
        .items_center()
        .justify_between()
        .px_3()
        .py(px(6.))
        .rounded(px(7.))
        .text_sm()
        .when(selected, |row| row.bg(rgba(palette.raised)))
        .text_color(rgb(if selected {
            palette.text
        } else {
            palette.muted
        }))
        .cursor_pointer()
        .hover(move |style| style.bg(rgba(palette.raised)))
        .on_click(cx.listener(|view, _, _, cx| view.open_settings(None, cx)))
        .child("Settings")
        .children(Kbd::binding_for_action(&OpenSettings, None, window).map(|k| k.appearance(false)))
}
