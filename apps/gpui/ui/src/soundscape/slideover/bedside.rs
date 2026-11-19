//! Settings and low-light controls for the Mac Bedside player.

use gpui_kit::{
    div, prelude::*, px, rgb, rgba, Context, FocusHandle, IntoElement, SharedString, Window,
};
use sjel_gpui_core::soundscape::{BedsideOptions, SleepEnding, SleepRoom};

use super::{quiet_button, SessionStart, SoundscapeSlideover};
use crate::ely::buttons::{Button, ButtonVariant, ToggleGroup, ToggleItem};
use crate::ely::forms::Switch;
use crate::ely::theme::ControlSize;
use crate::theme::Palette;

impl SoundscapeSlideover {
    pub(super) fn dark_player(
        &self,
        room: SleepRoom,
        playing: bool,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Div {
        let session = self.current().is_some_and(|state| state.session.is_some());
        let reveal = |view: &mut Self, window: &mut Window, cx: &mut Context<Self>| {
            view.dark_room = None;
            window.focus(&view.focus, cx);
            cx.notify();
        };
        let transport = if playing {
            "Pause"
        } else if session {
            "Resume"
        } else {
            "Play again"
        };
        div()
            .relative()
            .flex_1()
            .w_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_3()
            .child(
                div()
                    .id("bedside-dark-surface")
                    .debug_selector(|| "bedside-dark-surface".into())
                    .absolute()
                    .inset_0()
                    .on_click(cx.listener(move |view, _, window, cx| reveal(view, window, cx))),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(0x98a2af))
                    .child(room.label()),
            )
            .child(dark_button(
                "bedside-dark-transport",
                transport,
                &self.dark_focus[0],
                cx,
                move |view, window, cx| {
                    if session {
                        view.toggle(window, cx);
                    } else {
                        view.repeat_session(window, cx);
                    }
                },
            ))
            .child(dark_button(
                "bedside-show-controls",
                "Show controls",
                &self.dark_focus[1],
                cx,
                reveal,
            ))
    }

    fn bedside_options(&self) -> BedsideOptions {
        self.bedside_edit
            .or_else(|| self.current().and_then(|state| state.bedside_options))
            .unwrap_or_default()
    }

    fn edit_bedside(&mut self, edit: impl FnOnce(&mut BedsideOptions), cx: &mut Context<Self>) {
        let mut options = self.bedside_options();
        edit(&mut options);
        self.bedside_edit = Some(options);
        cx.notify();
    }

    pub(super) fn bedside_controls(
        &self,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Div {
        let options = self.bedside_options();
        let rooms = SleepRoom::ALL.into_iter().fold(
            ToggleGroup::new("bedside-rooms").selected([options.room.label()]),
            |group, room| group.item(ToggleItem::new(room.label()).label(room.label())),
        );
        let endings = SleepEnding::ALL.into_iter().fold(
            ToggleGroup::new("bedside-endings").selected([options.ending.label()]),
            |group, ending| group.item(ToggleItem::new(ending.label()).label(ending.label())),
        );
        div()
            .flex()
            .flex_col()
            .gap_2()
            .pt_3()
            .border_t_1()
            .border_color(rgba(palette.border))
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(palette.text))
                    .child("Bedside"),
            )
            .child(
                rooms.on_change(cx.listener(|view, keys: &[SharedString], _, cx| {
                    if let Some(room) = SleepRoom::ALL
                        .into_iter()
                        .find(|room| keys.first().is_some_and(|key| key.as_ref() == room.label()))
                    {
                        view.edit_bedside(|options| options.room = room, cx);
                    }
                })),
            )
            .child(
                endings.on_change(cx.listener(|view, keys: &[SharedString], _, cx| {
                    if let Some(ending) = SleepEnding::ALL.into_iter().find(|ending| {
                        keys.first()
                            .is_some_and(|key| key.as_ref() == ending.label())
                    }) {
                        view.edit_bedside(|options| options.ending = ending, cx);
                    }
                })),
            )
            .when(options.ending == SleepEnding::UntilMorning, |section| {
                let change_dawn = cx.listener(|view, dawn: &bool, _, cx| {
                    view.edit_bedside(|options| options.dawn = *dawn, cx);
                });
                section
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .child(quiet_button(
                                "bedside-earlier",
                                "Earlier",
                                cx,
                                |view, _, cx| {
                                    view.edit_bedside(
                                        |options| {
                                            options.morning_minute =
                                                (options.morning_minute + 1425) % 1440
                                        },
                                        cx,
                                    );
                                },
                            ))
                            .child(div().text_sm().text_color(rgb(palette.text)).child(format!(
                                "{:02}:{:02}",
                                options.morning_minute / 60,
                                options.morning_minute % 60
                            )))
                            .child(quiet_button("bedside-later", "Later", cx, |view, _, cx| {
                                view.edit_bedside(
                                    |options| {
                                        options.morning_minute =
                                            (options.morning_minute + 15) % 1440
                                    },
                                    cx,
                                );
                            })),
                    )
                    .child(
                        Switch::new("bedside-dawn", options.dawn)
                            .label("Gentle dawn ending")
                            .on_change(move |on, window, cx| change_dawn(&on, window, cx)),
                    )
            })
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("bedside-start", "Start Bedside")
                            .variant(ButtonVariant::Primary)
                            .size(ControlSize::Sm)
                            .disabled(self.device.is_none() || self.session_write.contains("start"))
                            .on_click(cx.listener(move |view, _, window, cx| {
                                view.start_session(
                                    SessionStart::Bedside {
                                        options,
                                        back_to_sleep: false,
                                    },
                                    window,
                                    cx,
                                );
                            })),
                    )
                    .child(self.back_to_sleep(options, cx)),
            )
    }

    pub(super) fn back_to_sleep(&self, options: BedsideOptions, cx: &mut Context<Self>) -> Button {
        Button::new("bedside-back-to-sleep", "Back to sleep · 20 min")
            .variant(ButtonVariant::Secondary)
            .size(ControlSize::Sm)
            .disabled(self.device.is_none() || self.session_write.contains("start"))
            .on_click(cx.listener(move |view, _, window, cx| {
                view.start_session(
                    SessionStart::Bedside {
                        options,
                        back_to_sleep: true,
                    },
                    window,
                    cx,
                );
            }))
    }
}

fn dark_button(
    id: &'static str,
    label: &'static str,
    focus: &FocusHandle,
    cx: &mut Context<SoundscapeSlideover>,
    activate: impl Fn(&mut SoundscapeSlideover, &mut Window, &mut Context<SoundscapeSlideover>)
        + Clone
        + 'static,
) -> impl IntoElement {
    let click = activate.clone();
    div()
        .id(id)
        .role(gpui_kit::Role::Button)
        .track_focus(focus)
        .rounded(px(6.))
        .border_1()
        .border_color(rgba(0))
        .px_3()
        .py_1()
        .text_xs()
        .text_color(rgb(0xa1adb9))
        .cursor_pointer()
        .hover(|style| style.bg(rgb(0x151a22)))
        .focus(|style| style.border_color(rgb(0x576473)))
        .on_click(cx.listener(move |view, _, window, cx| click(view, window, cx)))
        .on_key_down(
            cx.listener(move |view, event: &gpui_kit::KeyDownEvent, window, cx| {
                if event.keystroke.key == "enter" {
                    cx.stop_propagation();
                    activate(view, window, cx);
                }
            }),
        )
        .child(label)
}
