use gpui_kit::component::kbd::Kbd;
use gpui_kit::{div, prelude::*, px, rgb, rgba, Action, AnyElement, Context, IntoElement, Window};
use sjel_gpui_core::area::Area;

use crate::route::Page;
use crate::workspace::{OpenPalette, OpenSettings, ToggleSoundscapePanel, Workspace};
use sjel_gpui_core::soundscape::Preset;
use sjel_gpui_ui::ely::buttons::{Button, ButtonVariant};
use sjel_gpui_ui::ely::motion::ProgressBar;
use sjel_gpui_ui::ely::primitives::IconName;
use sjel_gpui_ui::ely::theme::ControlSize;
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

/// The optional sidebar player for an available soundscape capability.
pub fn soundscape(
    view: &Workspace,
    palette: Palette,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
    let state = view.soundscape.value.as_ref();
    let available = view
        .health
        .value
        .as_ref()
        .and_then(|health| health.capabilities.get("soundscape"))
        .is_some_and(|capability| capability.up);
    if !view.appearance_settings.soundscape_mini_player || (!available && state.is_none()) {
        return div().into_any_element();
    }

    let playing = state.is_some_and(|state| state.playing);
    let preset = state
        .and_then(|state| Preset::from_key(&state.preset))
        .unwrap_or(Preset::Edm);
    let timing = state
        .and_then(|state| state.session.as_ref())
        .map(|session| {
            let now = chrono::Utc::now().timestamp_millis().max(0) as u64;
            let remaining = session.remaining_at(now);
            let progress = session.duration_ms.saturating_sub(remaining) as f32
                / session.duration_ms.max(1) as f32;
            (remaining.div_ceil(60_000), (progress * 100.).floor() / 100.)
        });

    div()
        .id("soundscape-mini-player")
        .debug_selector(|| "soundscape-mini-player".into())
        .mx_1()
        .mb_1()
        .p_2()
        .rounded(px(8.))
        .border_1()
        .border_color(rgba(palette.border))
        .bg(rgba(palette.raised))
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_1()
                .child(
                    Button::new("soundscape-open-player", preset.label())
                        .variant(ButtonVariant::Ghost)
                        .size(ControlSize::Sm)
                        .trailing_icon(IconName::ArrowUpRight)
                        .on_click(
                            cx.listener(|view, _, window, cx| view.open_soundscape(window, cx)),
                        ),
                )
                .child(
                    Button::new(
                        "soundscape-mini-transport",
                        if playing { "Pause" } else { "Play" },
                    )
                    .size(ControlSize::Sm)
                    .variant(if playing {
                        ButtonVariant::Subtle
                    } else {
                        ButtonVariant::Secondary
                    })
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.toggle_soundscape(!playing, window, cx);
                    })),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .px_2()
                .text_xs()
                .text_color(rgb(palette.muted))
                .child(equalizer_indicator(playing, palette))
                .child(match (playing, timing) {
                    (true, Some((minutes, _))) => format!("{minutes} min left"),
                    (false, Some((minutes, _))) => format!("Paused · {minutes} min left"),
                    (true, None) => "Playing".into(),
                    (false, None) => "Paused".into(),
                })
                .child(div().flex_1())
                .children(
                    Kbd::binding_for_action(&ToggleSoundscapePanel, None, window)
                        .map(|kbd| kbd.appearance(false)),
                ),
        )
        .when_some(timing, |player, (_, progress)| {
            player.child(ProgressBar::new("soundscape-mini-progress", progress))
        })
        .when(window.viewport_size().height >= px(640.), |player| {
            player.child(div().flex().justify_between().gap_1().children(
                [Preset::Focus, Preset::Relax, Preset::Sleep].map(|mode| {
                    Button::new(mode.key(), mode.label())
                        .size(ControlSize::Sm)
                        .variant(if mode == preset {
                            ButtonVariant::Subtle
                        } else {
                            ButtonVariant::Ghost
                        })
                        .on_click(cx.listener(move |view, _, window, cx| {
                            view.choose_soundscape_preset(mode, window, cx);
                        }))
                }),
            ))
        })
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
