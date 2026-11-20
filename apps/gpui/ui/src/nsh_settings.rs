//! The NSH Settings view used by non-phone hosts.

use nsh_base::button;
use nsh_core::prelude::*;

use crate::phone_settings::{send, snapshots, Choice, SettingsAction, SettingsState};

fn muted() -> AdaptiveColor {
    nsh_base::theme().muted_foreground
}

fn card() -> View {
    crate::nsh_ui::surface_card().p_4().gap_3()
}

fn pill(label: impl Into<String>, enabled: bool, action: SettingsAction) -> impl IntoElement {
    button()
        .px_4()
        .py_2()
        .rounded_full()
        .border(px(1.0))
        .border_color(move || {
            if enabled {
                nsh_base::theme().primary
            } else {
                nsh_base::theme().border
            }
        })
        .bg(move || {
            if enabled {
                nsh_base::theme().primary
            } else {
                nsh_base::theme().card
            }
        })
        .on_press(move || {
            if enabled {
                send(action.clone());
            }
        })
        .child(
            text(label.into())
                .text_sm()
                .text_color(move || nsh_base::theme().primary_foreground),
        )
}

fn choice_row(choice: Choice) -> impl IntoElement {
    let mut row = view().flex_row().gap_1();
    for (ix, (label, active)) in choice.options.into_iter().enumerate() {
        let id = choice.id;
        row = row.child(
            button()
                .flex_1()
                .py_2()
                .items_center()
                .rounded_md()
                .bg(move || {
                    if active {
                        nsh_base::theme().primary
                    } else {
                        nsh_base::theme().muted
                    }
                })
                .on_press(move || send(SettingsAction::Choose { id, option: ix }))
                .child(
                    text(label)
                        .text_xs()
                        .number_of_lines(1u32)
                        .text_color(move || {
                            if active {
                                nsh_base::theme().primary_foreground
                            } else {
                                nsh_base::theme().foreground
                            }
                        }),
                ),
        );
    }
    view()
        .gap_1()
        .child(text(choice.title).text_xs().text_color(move || muted()))
        .child(row)
}

fn title(label: &'static str) -> impl IntoElement {
    text(label).text_base().font_weight(FontWeight::SEMIBOLD)
}

fn body(state: SettingsState) -> impl IntoElement {
    let pairing = card()
        .flex_row()
        .items_center()
        .gap_3()
        .child(
            view()
                .flex_1()
                .gap_1()
                .child(title(if state.paired {
                    "Pair again"
                } else {
                    "Scan your Mac’s code"
                }))
                .child(
                    text(if state.paired {
                        "Scan a new code from Setup on the Mac. The phone keeps its pairing; \
                         only the address changes."
                    } else {
                        "Setup on the Mac shows a square code. Point this phone at it."
                    })
                    .text_sm()
                    .text_color(move || muted()),
                ),
        )
        .child(pill(
            if state.joining { "Working…" } else { "Scan" },
            !state.joining,
            SettingsAction::Scan,
        ));

    let mut page = view()
        .px_4()
        .pt_2()
        .pb_6()
        .gap_4()
        .child(
            button()
                .py_2()
                .on_press(|| send(SettingsAction::Back))
                .child(text("‹ Today").text_base()),
        )
        .child(
            text("Settings")
                .text_2xl()
                .font_weight(FontWeight::SEMIBOLD),
        )
        .child(
            text(state.status.clone())
                .text_sm()
                .text_color(move || muted()),
        )
        .child(
            state
                .synced
                .clone()
                .map(|line| text(line).text_xs().text_color(move || muted())),
        )
        .child(pairing);

    if !state.found.is_empty() && state.capabilities.is_empty() {
        let mut found = card();
        for (ix, mac) in state.found.iter().enumerate() {
            found = found.child(
                view()
                    .flex_row()
                    .items_center()
                    .gap_3()
                    .child(
                        view()
                            .flex_1()
                            .gap_1()
                            .child(text(mac.name.clone()).text_base())
                            .child(
                                text(mac.address.clone())
                                    .text_sm()
                                    .text_color(move || muted()),
                            ),
                    )
                    .child(pill(
                        if state.joining { "Asking…" } else { "Join" },
                        !state.joining,
                        SettingsAction::Join(ix),
                    )),
            );
        }
        page = page.child(found);
    }

    page = page.child(
        card()
            .child(title("Health sync"))
            .child(
                text(state.health_status)
                    .text_sm()
                    .text_color(move || muted()),
            )
            .child(
                state
                    .health_last_synced
                    .map(|at| text(format!("Last synced: {at}")).text_sm()),
            )
            .child(pill(
                "Sync now",
                state.health_can_sync,
                SettingsAction::SyncHealth,
            ))
            .child(
                text("Check read access in Apple Health › profile › Apps (under Privacy) › Sjel.")
                    .text_xs()
                    .text_color(move || muted()),
            ),
    );

    if !state.capabilities.is_empty() {
        let mut list = card().gap_2();
        for (name, up) in state.capabilities.iter().cloned() {
            list = list.child(
                view()
                    .flex_row()
                    .items_center()
                    .gap_3()
                    .child(view().w(px(8.0)).h(px(8.0)).rounded_full().bg(move || {
                        if up {
                            nsh_base::theme().primary
                        } else {
                            muted()
                        }
                    }))
                    .child(text(name).text_sm()),
            );
        }
        page = page.child(list);
    }

    let mut room = card()
        .child(title("Room scan"))
        .child(
            text(state.room_status.clone())
                .text_sm()
                .text_color(move || muted()),
        )
        .child(pill("Capture a room", true, SettingsAction::CaptureRoom));
    if state.room_can_sync {
        room = room.child(pill(
            "Sync pending scans to Mac",
            true,
            SettingsAction::SyncRoomScans,
        ));
    }
    page = page.child(room);

    let mut reminders = card()
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .child(
                    view().flex_1().gap_1().child(title("Local alerts")).child(
                        text(state.reminders_sentence.clone())
                            .text_sm()
                            .text_color(move || muted()),
                    ),
                )
                .child(pill(
                    if state.reminders_on { "On" } else { "Off" },
                    true,
                    SettingsAction::ToggleReminders,
                )),
        )
        .child(
            text("Calendar and task reminders, plus new low-fare notes in upcoming trips.")
                .text_xs()
                .text_color(move || muted()),
        );
    if state.reminders_denied {
        reminders = reminders.child(pill(
            "Open iOS Settings",
            true,
            SettingsAction::OpenSystemSettings,
        ));
    }
    if let Some(lead) = state.lead.clone() {
        reminders = reminders.child(choice_row(lead));
    }
    if let Some(dropped) = state.reminders_dropped.clone() {
        reminders = reminders.child(text(dropped).text_xs().text_color(move || muted()));
    }
    page = page.child(reminders);

    let mut appearance = card().child(title("Appearance & Theme"));
    for choice in state.appearance.iter().cloned() {
        appearance = appearance.child(choice_row(choice));
    }
    page = page.child(appearance);

    if !state.terminal.is_empty() {
        let mut terminal = card().child(title("Terminal"));
        for choice in state.terminal.iter().cloned() {
            terminal = terminal.child(choice_row(choice));
        }
        page = page.child(terminal);
    }
    scroll().flex_1().child(page)
}

#[component]
pub fn SettingsScreen() -> impl IntoElement {
    let state = crate::nsh_ui::snapshot(snapshots());
    view()
        .flex_1()
        .child(dynamic(move || body(state.get()).into_element()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::phone_settings::{set_host, set_state, FoundMac};
    use std::sync::{Arc, Mutex};

    #[test]
    fn the_screen_draws_the_shells_state_and_reports_presses() {
        use nsh_core::element::Root;
        use nsh_engine::EngineConfig;
        use nsh_testing::Headless;

        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();
        set_host(move |action| sink.lock().unwrap().push(action));
        set_state(SettingsState {
            status: "Paired with Studio".to_owned(),
            found: vec![FoundMac {
                name: "Studio".to_owned(),
                address: "192.0.2.4:8443".to_owned(),
            }],
            room_status: "No scans yet".to_owned(),
            health_status: "No upload confirmed yet.".to_owned(),
            health_can_sync: true,
            reminders_sentence: "Off · nothing is announced".to_owned(),
            appearance: vec![Choice {
                id: "theme",
                title: "THEME MODE",
                options: vec![("System".to_owned(), true), ("Dark".to_owned(), false)],
            }],
            ..SettingsState::default()
        });
        let mut app = Headless::start(EngineConfig {
            ui_sets: vec![nsh_base::registry()],
            ..EngineConfig::new(Root::of::<SettingsScreen>())
        })
        .expect("the Settings screen starts");
        app.wait_for("text:Paired with Studio").unwrap();
        app.press("text:Join").unwrap();
        app.wait_for("text:Health sync").unwrap();
        app.wait_for("text:No upload confirmed yet.").unwrap();
        app.press("text:Sync now").unwrap();
        app.press("text:Dark").unwrap();
        app.press("text:Scan").unwrap();
        let current = snapshots().borrow().clone();
        set_state(SettingsState {
            status: "Point the camera at the code on your Mac.".to_owned(),
            health_status: "Upload confirmed by your Mac.".to_owned(),
            health_last_synced: Some("Nov 20, 4:50 PM".to_owned()),
            ..current
        });
        app.wait_for("text:Point the camera at the code on your Mac.")
            .unwrap();
        app.wait_for("text:Last synced: Nov 20, 4:50 PM").unwrap();
        assert_eq!(
            *seen.lock().unwrap(),
            vec![
                SettingsAction::Join(0),
                SettingsAction::SyncHealth,
                SettingsAction::Choose {
                    id: "theme",
                    option: 1
                },
                SettingsAction::Scan,
            ]
        );
    }
}
