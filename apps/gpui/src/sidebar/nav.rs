use gpui_kit::component::kbd::Kbd;
use gpui_kit::{
    div, prelude::*, px, rgb, rgba, Action, AnyElement, Context, FontWeight, IntoElement,
    SharedString, Window,
};
use sjel_gpui_core::area::Area;

use crate::route::Page;
use crate::workspace::{
    OpenPalette, OpenScratchpad, OpenSettings, ToggleSoundscapePanel, Workspace,
};
use sjel_gpui_core::soundscape::Preset;
use sjel_gpui_ui::ely::buttons::{Button, ButtonVariant, IconButton};
use sjel_gpui_ui::ely::motion::ProgressBar;
use sjel_gpui_ui::ely::primitives::IconName;
use sjel_gpui_ui::ely::theme::ControlSize;
use sjel_gpui_ui::sidebar::Seat;
use sjel_gpui_ui::theme::{tint, Palette};

fn area_badge_count(view: &Workspace, area: Area, cx: &Context<Workspace>) -> Option<usize> {
    let count = match area {
        Area::Today => view.urgent_task_count(),
        Area::Inbox => view
            .inbox
            .read(cx)
            .rows
            .value
            .as_ref()
            .map(|rows| rows.iter().filter(|c| c.status == "new").count())
            .unwrap_or(0),
        Area::Calendar => view
            .calendar_page
            .as_ref()
            .map(|cp| cp.read(cx).pending_proposals_count())
            .unwrap_or(0),
        Area::Projects => view
            .focus_tasks
            .value
            .as_ref()
            .map(|tasks| tasks.iter().filter(|t| !t.done).count())
            .unwrap_or(0),
        Area::Travel => view.travel.read(cx).upcoming_count(),
        Area::Life => {
            let fin = view
                .finance_page
                .as_ref()
                .and_then(|fp| fp.read(cx).badge_counts().2)
                .unwrap_or(0);
            let inv = view
                .inventory_page
                .as_ref()
                .map(|ip| ip.read(cx).mail_review_pending_count())
                .unwrap_or(0);
            fin + inv
        }
        Area::Sjel => view
            .health
            .value
            .as_ref()
            .map(|h| h.down().len())
            .unwrap_or(0),
        _ => 0,
    };
    (count > 0).then_some(count)
}

fn page_badge_count(view: &Workspace, page: Page, cx: &Context<Workspace>) -> Option<usize> {
    let count = match page {
        Page::Inbox => view
            .inbox
            .read(cx)
            .rows
            .value
            .as_ref()
            .map(|rows| rows.iter().filter(|c| c.status == "new").count())
            .unwrap_or(0),
        Page::Projects => view
            .focus_tasks
            .value
            .as_ref()
            .map(|tasks| tasks.iter().filter(|t| !t.done).count())
            .unwrap_or(0),
        Page::Finance => view
            .finance_page
            .as_ref()
            .and_then(|fp| fp.read(cx).badge_counts().2)
            .unwrap_or(0),
        Page::Inventory => view
            .inventory_page
            .as_ref()
            .map(|ip| ip.read(cx).mail_review_pending_count())
            .unwrap_or(0),
        Page::Capabilities => view
            .health
            .value
            .as_ref()
            .map(|h| h.down().len())
            .unwrap_or(0),
        _ => 0,
    };
    (count > 0).then_some(count)
}

/// One row per area, with shortcut hints (`⌘1`–`⌘8`), live pending badges, and an expandable
/// sub-menu tree of its child pages when the area is active.
pub fn nav(view: &Workspace, palette: Palette, cx: &mut Context<Workspace>) -> Vec<AnyElement> {
    let active_area = Area::of(view.page);
    Area::ALL
        .into_iter()
        .enumerate()
        .map(|(idx, area)| {
            let selected = active_area == Some(area);
            let tab = view.tabs.current(area);
            let badge = area_badge_count(view, area, cx);
            let shortcut = format!("⌘{}", idx + 1);
            let has_tabs = area.has_tabs();

            let mut item = div().flex().flex_col().gap(px(2.));

            let row = div()
                .id(area.label())
                .flex()
                .items_center()
                .justify_between()
                .gap_2()
                .px_3()
                .py(px(5.))
                .rounded(px(7.))
                .when(selected, |row| {
                    row.bg(rgba(tint(palette.accent, 0x18)))
                        .font_weight(FontWeight::SEMIBOLD)
                })
                .text_color(rgb(if selected {
                    palette.text
                } else {
                    palette.muted
                }))
                .cursor_pointer()
                .hover(move |style| style.bg(rgba(palette.raised)).text_color(rgb(palette.text)))
                .on_click(cx.listener(move |view, _, _, cx| view.open_area(area, cx)))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .min_w_0()
                        .child(div().truncate().child(area.label()))
                        .when(has_tabs && tab != area.default_page() && !selected, |el| {
                            el.child(
                                div()
                                    .px_1p5()
                                    .py(px(0.5))
                                    .rounded(px(4.))
                                    .bg(rgba(tint(palette.accent, 0x1c)))
                                    .text_color(rgb(palette.accent))
                                    .text_xs()
                                    .child(tab.label()),
                            )
                        }),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .flex_none()
                        .when_some(badge, |el, count| {
                            let tone = if area == Area::Sjel {
                                palette.red
                            } else {
                                palette.accent
                            };
                            el.child(
                                div()
                                    .px_1p5()
                                    .py(px(0.5))
                                    .rounded_full()
                                    .bg(rgba(tint(tone, 0x20)))
                                    .text_color(rgb(tone))
                                    .font_family(".AppleSystemUIFontMonospaced")
                                    .text_xs()
                                    .child(count.to_string()),
                            )
                        })
                        .child(
                            div()
                                .text_xs()
                                .font_family(".AppleSystemUIFontMonospaced")
                                .text_color(rgb(palette.muted))
                                .opacity(if selected { 0.85 } else { 0.55 })
                                .child(shortcut),
                        ),
                );

            item = item.child(row);

            if selected && has_tabs {
                let mut sub_list = div()
                    .ml_4()
                    .pl_2()
                    .py(px(2.))
                    .border_l_1()
                    .border_color(rgba(tint(palette.border, 0x55)))
                    .flex()
                    .flex_col()
                    .gap(px(1.));

                for &sub_page in area.pages() {
                    let is_current = sjel_gpui_core::area::canonical(view.page) == sub_page;
                    let sub_badge = page_badge_count(view, sub_page, cx);
                    sub_list = sub_list.child(
                        div()
                            .id(SharedString::from(format!(
                                "sidebar-sub-{}-{}",
                                area.label(),
                                sub_page.label()
                            )))
                            .flex()
                            .items_center()
                            .justify_between()
                            .px_2p5()
                            .py(px(3.5))
                            .rounded(px(5.))
                            .text_xs()
                            .cursor_pointer()
                            .when(is_current, |el| {
                                el.bg(rgba(palette.raised))
                                    .text_color(rgb(palette.accent))
                                    .font_weight(FontWeight::MEDIUM)
                            })
                            .when(!is_current, |el| {
                                el.text_color(rgb(palette.muted)).hover(move |s| {
                                    s.bg(rgba(palette.raised)).text_color(rgb(palette.text))
                                })
                            })
                            .on_click(cx.listener(move |view, _, _, cx| view.go(sub_page, cx)))
                            .child(div().truncate().child(sub_page.label()))
                            .when_some(sub_badge, |el, count| {
                                el.child(
                                    div()
                                        .font_family(".AppleSystemUIFontMonospaced")
                                        .text_color(rgb(palette.muted))
                                        .child(count.to_string()),
                                )
                            }),
                    );
                }
                item = item.child(sub_list);
            } else if selected && area == Area::Calendar {
                use sjel_gpui_ui::calendar::View as CalendarView;
                let cur_view = view
                    .calendar_page
                    .as_ref()
                    .map(|cp| cp.read(cx).view)
                    .unwrap_or(CalendarView::Month);
                let mut sub_list = div()
                    .ml_4()
                    .pl_2()
                    .py(px(2.))
                    .border_l_1()
                    .border_color(rgba(tint(palette.border, 0x55)))
                    .flex()
                    .flex_col()
                    .gap(px(1.));
                for (label, target) in [
                    ("Day", CalendarView::Day),
                    ("Week", CalendarView::Week),
                    ("Month", CalendarView::Month),
                ] {
                    let is_current = cur_view == target;
                    sub_list = sub_list.child(
                        div()
                            .id(SharedString::from(format!("sidebar-sub-cal-{label}")))
                            .flex()
                            .items_center()
                            .justify_between()
                            .px_2p5()
                            .py(px(3.5))
                            .rounded(px(5.))
                            .text_xs()
                            .cursor_pointer()
                            .when(is_current, |el| {
                                el.bg(rgba(palette.raised))
                                    .text_color(rgb(palette.accent))
                                    .font_weight(FontWeight::MEDIUM)
                            })
                            .when(!is_current, |el| {
                                el.text_color(rgb(palette.muted)).hover(move |s| {
                                    s.bg(rgba(palette.raised)).text_color(rgb(palette.text))
                                })
                            })
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.go(Page::Calendar, cx);
                                view.ensure_calendar_page(cx)
                                    .update(cx, |page, cx| page.set_view(target, cx));
                            }))
                            .child(div().truncate().child(label)),
                    );
                }
                item = item.child(sub_list);
            } else if selected && area == Area::Travel {
                use sjel_gpui_core::travel::TravelRoute;
                let cur_route = view.travel.read(cx).route().clone();
                let upcoming = view.travel.read(cx).upcoming_count();
                let mut sub_list = div()
                    .ml_4()
                    .pl_2()
                    .py(px(2.))
                    .border_l_1()
                    .border_color(rgba(tint(palette.border, 0x55)))
                    .flex()
                    .flex_col()
                    .gap(px(1.));
                for (label, target, count) in [
                    ("Map & Places", TravelRoute::Map, None),
                    (
                        "Trips",
                        TravelRoute::Trips,
                        (upcoming > 0).then_some(upcoming),
                    ),
                    ("Connections", TravelRoute::Connections, None),
                ] {
                    let is_current = match (&cur_route, &target) {
                        (TravelRoute::Map, TravelRoute::Map) => true,
                        (TravelRoute::Connections, TravelRoute::Connections) => true,
                        (TravelRoute::Trips | TravelRoute::Trip { .. }, TravelRoute::Trips) => true,
                        _ => false,
                    };
                    sub_list = sub_list.child(
                        div()
                            .id(SharedString::from(format!("sidebar-sub-travel-{label}")))
                            .flex()
                            .items_center()
                            .justify_between()
                            .px_2p5()
                            .py(px(3.5))
                            .rounded(px(5.))
                            .text_xs()
                            .cursor_pointer()
                            .when(is_current, |el| {
                                el.bg(rgba(palette.raised))
                                    .text_color(rgb(palette.accent))
                                    .font_weight(FontWeight::MEDIUM)
                            })
                            .when(!is_current, |el| {
                                el.text_color(rgb(palette.muted)).hover(move |s| {
                                    s.bg(rgba(palette.raised)).text_color(rgb(palette.text))
                                })
                            })
                            .on_click(cx.listener(move |view, _, _, cx| {
                                let route = target.clone();
                                view.go(Page::Travel, cx);
                                view.travel
                                    .update(cx, |travel, cx| travel.open_route(route, cx));
                            }))
                            .child(div().truncate().child(label))
                            .when_some(count, |el, count| {
                                el.child(
                                    div()
                                        .font_family(".AppleSystemUIFontMonospaced")
                                        .text_color(rgb(palette.muted))
                                        .child(count.to_string()),
                                )
                            }),
                    );
                }
                item = item.child(sub_list);
            }

            item.into_any_element()
        })
        .collect()
}

/// Quick sidebar row to toggle the 24-hour Scratchpad seat (`⌘\`) with live staged item count.
pub fn scratchpad_row(
    view: &Workspace,
    palette: Palette,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> impl IntoElement {
    let active = view.right_panel.is_open() && view.right_panel.seat() == Seat::Scratchpad;
    let count = view.scratchpad.read(cx).state.parse_targets().len();
    div()
        .id("sidebar-scratchpad-row")
        .flex()
        .items_center()
        .justify_between()
        .px_3()
        .py(px(6.))
        .rounded(px(7.))
        .text_sm()
        .when(active, |row| {
            row.bg(rgba(tint(palette.accent, 0x18)))
                .text_color(rgb(palette.text))
                .font_weight(FontWeight::MEDIUM)
        })
        .when(!active, |row| row.text_color(rgb(palette.muted)))
        .cursor_pointer()
        .hover(move |style| style.bg(rgba(palette.raised)).text_color(rgb(palette.text)))
        .on_click(cx.listener(|view, _, window, cx| view.toggle_scratchpad(window, cx)))
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child("Scratchpad")
                .when(count > 0, |el| {
                    el.child(
                        div()
                            .px_1p5()
                            .py(px(0.5))
                            .rounded_full()
                            .bg(rgba(tint(palette.amber, 0x22)))
                            .text_color(rgb(palette.amber))
                            .font_family(".AppleSystemUIFontMonospaced")
                            .text_xs()
                            .child(count.to_string()),
                    )
                }),
        )
        .children(
            Kbd::binding_for_action(&OpenScratchpad, None, window).map(|k| k.appearance(false)),
        )
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
    let repeat = state
        .filter(|state| state.session.is_none())
        .and_then(|state| state.last_session.as_ref());

    // The labels carry tempo after a middot ("LO-FI · 78 BPM"); the strip names the scape only.
    let name = preset.label().split(" · ").next().unwrap_or_default();
    let status = match (playing, timing) {
        (true, Some((minutes, _))) => format!("{minutes} min left"),
        (false, Some((minutes, _))) => format!("Paused · {minutes} min left"),
        (true, None) => "Playing".into(),
        (false, None) => "Paused".into(),
    };

    div()
        .id("soundscape-mini-player")
        .debug_selector(|| "soundscape-mini-player".into())
        .mx_1()
        .mb_1()
        .px_2()
        .py(px(6.))
        .rounded(px(8.))
        .border_1()
        .border_color(rgba(palette.border))
        .bg(rgba(palette.raised))
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .id("soundscape-open-player")
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_1()
                        .py(px(2.))
                        .rounded(px(5.))
                        .cursor_pointer()
                        .hover(move |style| style.bg(rgba(tint(palette.accent, 0x14))))
                        .on_click(
                            cx.listener(|view, _, window, cx| view.open_soundscape(window, cx)),
                        )
                        .child(equalizer_indicator(playing, palette))
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_xs()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(palette.text))
                                .child(name),
                        ),
                )
                .child(
                    IconButton::new(
                        "soundscape-mini-transport",
                        if playing {
                            IconName::Pause
                        } else {
                            IconName::Play
                        },
                    )
                    .size(ControlSize::Sm)
                    .variant(if playing {
                        ButtonVariant::Subtle
                    } else {
                        ButtonVariant::Secondary
                    })
                    .tooltip(if playing { "Pause" } else { "Play" })
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.toggle_soundscape(!playing, window, cx);
                    })),
                ),
        )
        .when_some(timing, |player, (_, progress)| {
            player.child(ProgressBar::new("soundscape-mini-progress", progress))
        })
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_2()
                .px_1()
                .text_xs()
                .text_color(rgb(palette.muted))
                .child(div().min_w_0().truncate().child(status))
                .children(
                    Kbd::binding_for_action(&ToggleSoundscapePanel, None, window)
                        .map(|kbd| kbd.appearance(false)),
                ),
        )
        .when_some(repeat, |player, saved| {
            player.child(
                Button::new(
                    "soundscape-repeat-session",
                    format!(
                        "Repeat {}",
                        sjel_gpui_core::soundscape::Scenario::label_for(&saved.scenario)
                    ),
                )
                .icon(IconName::Repeat)
                .variant(ButtonVariant::Ghost)
                .size(ControlSize::Sm)
                .full_width()
                .on_click(cx.listener(|view, _, window, cx| {
                    view.repeat_soundscape_session(window, cx);
                })),
            )
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
