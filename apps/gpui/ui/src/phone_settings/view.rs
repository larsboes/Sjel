//! The phone settings surface over the host's current state.

use gpui::{
    div, prelude::*, rgb, rgba, AnyElement, Context, Div, FontWeight, SharedString, Task, Window,
};

use super::{send, snapshots, Choice, SettingsAction as Action, SettingsState};
use crate::{
    ely::{
        buttons::{Button, ButtonVariant},
        data_display::{Badge, Tone},
        forms::SettingRow,
        theme::ControlSize,
    },
    primitives::{note, surface_card},
    theme::{Palette, ThemeContext},
};

pub struct PhoneSettings {
    settings: SettingsState,
    _updates: Task<()>,
}

impl PhoneSettings {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let mut snapshots = snapshots().subscribe();
        let settings = snapshots.borrow_and_update().clone();
        let updates = cx.spawn(async move |view, cx| {
            while snapshots.changed().await.is_ok() {
                let settings = snapshots.borrow_and_update().clone();
                if view
                    .update(cx, |view, cx| {
                        view.settings = settings;
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            settings,
            _updates: updates,
        }
    }
}

impl Render for PhoneSettings {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = &self.settings;
        let palette = ThemeContext::of(window, cx).palette;
        let pairing = section(
            palette,
            if state.paired {
                "Pair again"
            } else {
                "Scan your Mac’s code"
            },
        )
        .child(caption(
            palette,
            if state.paired {
                "Scan a new code from Setup on the Mac to update its address."
            } else {
                "Open Setup on the Mac and point this phone at the code."
            },
        ))
        .child(
            action(
                "pair-scan",
                if state.joining { "Working…" } else { "Scan" },
                Action::Scan,
            )
            .disabled(state.joining),
        );

        let health = section(palette, "Health sync")
            .child(caption(palette, state.health_status.clone()))
            .when_some(state.health_last_synced.as_ref(), |card, at| {
                card.child(format!("Last synced: {at}"))
            })
            .child(
                action("health-sync", "Sync now", Action::SyncHealth)
                    .disabled(!state.health_can_sync),
            )
            .child(caption(
                palette,
                "Check read access in Apple Health › profile › Apps (under Privacy) › Sjel.",
            ));

        let room = section(palette, "Room scan")
            .child(caption(palette, state.room_status.clone()))
            .child(action(
                "room-capture",
                "Capture a room",
                Action::CaptureRoom,
            ))
            .when(state.room_can_sync, |card| {
                card.child(action(
                    "room-sync",
                    "Sync pending scans to Mac",
                    Action::SyncRoomScans,
                ))
            });

        let reminders = section(palette, "Local alerts")
            .child(
                SettingRow::new(
                    "Notification reminders",
                    action(
                        "reminders-toggle",
                        if state.reminders_on { "On" } else { "Off" },
                        Action::ToggleReminders,
                    ),
                )
                .description(state.reminders_sentence.clone()),
            )
            .child(caption(
                palette,
                "Calendar and task reminders, plus new low-fare notes in upcoming trips.",
            ))
            .when(state.reminders_denied, |card| {
                card.child(action(
                    "open-settings",
                    "Open iOS Settings",
                    Action::OpenSystemSettings,
                ))
            })
            .when_some(state.lead.as_ref(), |card, choice| {
                card.child(choice_row(palette, choice))
            })
            .when_some(state.reminders_dropped.as_ref(), |card, line| {
                card.child(caption(palette, line.clone()))
            });

        div()
            .id("settings-scroll")
            .size_full()
            .overflow_y_scroll()
            .bg(rgba(palette.window_tint))
            .text_color(rgb(palette.text))
            .child(
                column()
                    .p_4()
                    .child(
                        action("settings-back", "‹ Today", Action::Back)
                            .variant(ButtonVariant::Ghost),
                    )
                    .child(
                        div()
                            .text_2xl()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Settings"),
                    )
                    .child(caption(palette, state.status.clone()))
                    .when_some(state.synced.as_ref(), |page, synced| {
                        page.child(caption(palette, synced.clone()))
                    })
                    .child(pairing)
                    .when(
                        !state.found.is_empty() && state.capabilities.is_empty(),
                        |page| {
                            page.child(section(palette, "Nearby Macs").children(
                                state.found.iter().enumerate().map(|(index, mac)| {
                                    SettingRow::new(
                                        mac.name.clone(),
                                        action(
                                            format!("join-mac-{index}"),
                                            if state.joining { "Asking…" } else { "Join" },
                                            Action::Join(index),
                                        )
                                        .disabled(state.joining),
                                    )
                                    .description(mac.address.clone())
                                }),
                            ))
                        },
                    )
                    .child(health)
                    .when(!state.capabilities.is_empty(), |page| {
                        page.child(section(palette, "Capabilities").children(
                            state.capabilities.iter().map(|(name, up)| {
                                SettingRow::new(
                                    name.clone(),
                                    Badge::new(if *up { "Available" } else { "Unavailable" })
                                        .tone(if *up { Tone::Success } else { Tone::Warning }),
                                )
                            }),
                        ))
                    })
                    .child(room)
                    .child(reminders)
                    .child(
                        section(palette, "Appearance & Theme").children(
                            state
                                .appearance
                                .iter()
                                .map(|choice| choice_row(palette, choice)),
                        ),
                    )
                    .when(!state.terminal.is_empty(), |page| {
                        page.child(
                            section(palette, "Terminal").children(
                                state
                                    .terminal
                                    .iter()
                                    .map(|choice| choice_row(palette, choice)),
                            ),
                        )
                    }),
            )
    }
}

fn choice_row(palette: Palette, choice: &Choice) -> Div {
    let id = choice.id;
    column().child(caption(palette, choice.title)).child(
        div()
            .flex()
            .flex_wrap()
            .gap_1()
            .children(
                choice
                    .options
                    .iter()
                    .enumerate()
                    .map(|(option, (label, active))| {
                        action(
                            format!("{id}-{option}"),
                            label.clone(),
                            Action::Choose { id, option },
                        )
                        .variant(if *active {
                            ButtonVariant::Primary
                        } else {
                            ButtonVariant::Outline
                        })
                    }),
            ),
    )
}

fn action(
    id: impl Into<gpui::ElementId>,
    label: impl Into<SharedString>,
    action: Action,
) -> Button {
    Button::new(id, label)
        .size(ControlSize::Lg)
        .on_click(move |_, _, _| send(action.clone()))
}

fn section(palette: Palette, title: &'static str) -> Div {
    surface_card(palette)
        .flex()
        .flex_col()
        .gap_2()
        .p_3()
        .child(div().font_weight(FontWeight::SEMIBOLD).child(title))
}

fn column() -> Div {
    div().flex().flex_col().gap_2()
}

fn caption(palette: Palette, text: impl Into<SharedString>) -> AnyElement {
    note(text, palette.muted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn phone_settings_mounts_at_phone_viewport(cx: &mut gpui::TestAppContext) {
        let (view, cx) = crate::test_support::mount_root(
            cx,
            Some(gpui::size(gpui::px(390.), gpui::px(844.))),
            |_, cx| PhoneSettings::new(cx),
        );
        view.read_with(cx, |view, _| assert!(!view.settings.paired));
    }
}
