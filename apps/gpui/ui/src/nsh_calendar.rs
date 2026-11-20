//! The NSH Calendar view used by non-phone hosts.

use chrono::{Duration, Local, NaiveDate, Timelike};
use nsh_base::{button, input};
use nsh_core::prelude::*;
use sjel_gpui_core::calendar::{self, timeline, Draft};
use sjel_gpui_core::shell::CalendarEntry;

use crate::phone_calendar::{
    change_time, delete_entry, emit, has_store, load, nudge, open_day, save_entry, subscribe,
    today, CalendarEvent, Origin, Snapshot,
};

fn muted() -> AdaptiveColor {
    nsh_base::theme().muted_foreground
}

const HOUR_PX: f32 = 52.0;

/// The day's new-entry form, while it is open.
#[derive(Clone, Debug, PartialEq)]
struct Form {
    start: String,
    end: String,
    all_day: bool,
    commitment: &'static str,
}

fn form_at(start_minute: u16, end_minute: u16) -> Form {
    let clock = |m: u16| format!("{:02}:{:02}", (m / 60) % 24, m % 60);
    Form {
        start: clock(start_minute),
        end: clock(end_minute),
        all_day: false,
        commitment: "planned",
    }
}

#[derive(Clone, Copy)]
struct Ui {
    snap: Signal<Snapshot>,
    grid: Signal<bool>,
    free: Signal<bool>,
    selected: Signal<Option<String>>,
    confirm_delete: Signal<bool>,
    form: Signal<Option<Form>>,
    title: Signal<String>,
    form_error: Signal<Option<String>>,
    note: Signal<Option<String>>,
}

impl Ui {
    fn writable(&self) -> bool {
        self.snap.with(|s| s.writable(has_store()))
    }

    fn open_form(&self, form: Form) {
        if self.writable() {
            self.title.set(String::new());
            self.form_error.set(None);
            self.selected.set(None);
            self.form.set(Some(form));
        }
    }
}

fn chip(label: impl Into<String>, on: bool, press: impl Fn() + 'static) -> impl IntoElement {
    crate::nsh_ui::pill_tab(label, on, press)
}

fn week_strip(ui: Ui) -> impl IntoElement {
    let snap = ui.snap.get();
    let mut strip = view().flex_row().items_center().gap_1().child(
        button()
            .px_2()
            .py_2()
            .on_press(move || open_day(snap_day(ui) - Duration::days(7)))
            .child(text("‹").text_base()),
    );
    for day in snap.week.clone() {
        let on = day == snap.day;
        let dotted = !calendar::entries_on(&snap.entries, day).is_empty();
        strip = strip.child(
            button()
                .flex_1()
                .py_2()
                .items_center()
                .rounded_lg()
                .bg(move || {
                    if on {
                        nsh_base::theme().primary
                    } else {
                        nsh_base::theme().background
                    }
                })
                .on_press(move || {
                    ui.selected.set(None);
                    open_day(day);
                })
                .child(
                    text(calendar_weekday(day))
                        .text_xs()
                        .text_color(move || weekday_color(on)),
                )
                .child(
                    text(day.format("%-d").to_string())
                        .text_base()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || weekday_color(on)),
                )
                .child(
                    text(if dotted { "•" } else { " " })
                        .text_xs()
                        .text_color(move || weekday_color(on)),
                ),
        );
    }
    strip.child(
        button()
            .px_2()
            .py_2()
            .on_press(move || open_day(snap_day(ui) + Duration::days(7)))
            .child(text("›").text_base()),
    )
}

fn snap_day(ui: Ui) -> NaiveDate {
    ui.snap.with(|s| s.day)
}

fn calendar_weekday(day: NaiveDate) -> String {
    day.format("%a").to_string().chars().take(1).collect()
}

fn weekday_color(on: bool) -> AdaptiveColor {
    if on {
        nsh_base::theme().primary_foreground
    } else {
        nsh_base::theme().foreground
    }
}

fn entry_button(entry: &CalendarEntry, day: NaiveDate, ui: Ui) -> impl IntoElement {
    let id = entry.id.clone();
    let commitment = entry.commitment.clone();
    let place = entry
        .location
        .clone()
        .filter(|l| !l.is_empty())
        .unwrap_or_else(|| calendar::kind_label(&entry.kind).to_owned());
    button()
        .px_4()
        .py_3()
        .gap_1()
        .border_b(px(1.0))
        .border_b_color(move || nsh_base::theme().border)
        .on_press(move || {
            ui.form.set(None);
            ui.confirm_delete.set(false);
            ui.selected.set(Some(id.clone()));
        })
        .child(text(entry.title.clone()).text_base())
        .child(
            text(format!(
                "{} · {place} · {commitment}",
                calendar::time_label(entry, day)
            ))
            .text_xs()
            .text_color(move || muted()),
        )
}

fn list(ui: Ui) -> impl IntoElement {
    let snap = ui.snap.get();
    let day = snap.day;
    let entries = calendar::entries_on(&snap.entries, day);
    let mut out = view();
    if entries.is_empty() {
        let line = match (snap.origin, snap.loaded) {
            (_, false) if snap.loading => "Reading the day…",
            (Origin::Copy, true) => "No entries on this day in the Mac's copy.",
            (Origin::Store, true) => "Nothing on this day.",
            _ => "",
        };
        return out.child(
            text(line)
                .text_sm()
                .px_4()
                .py_3()
                .text_color(move || muted()),
        );
    }
    for entry in entries {
        out = out.child(entry_button(entry, day, ui));
    }
    out
}

/// The 24-hour grid: hour rows a tap opens the form on, and the day's timed entries placed by
/// `timeline::blocks`, side by side where they overlap.
fn grid(ui: Ui) -> impl IntoElement {
    let snap = ui.snap.get();
    let day = snap.day;
    let mut canvas = view().relative().h(px(HOUR_PX * 24.0));
    for hour in 0..24u16 {
        canvas = canvas.child(
            button()
                .absolute()
                .top(px(HOUR_PX * f32::from(hour)))
                .left(px(0.0))
                .right(px(0.0))
                .h(px(HOUR_PX))
                .border_b(px(1.0))
                .border_b_color(move || nsh_base::theme().border)
                .on_press(move || ui.open_form(form_at(hour * 60, hour * 60 + 60)))
                .child(
                    text(format!("{hour:02}"))
                        .text_xs()
                        .text_color(move || muted()),
                ),
        );
    }
    let lane_left = 40.0;
    for block in timeline::blocks(&snap.entries, day) {
        let Some(entry) = snap.entries.get(block.entry_index).cloned() else {
            continue;
        };
        let id = entry.id.clone();
        let width = 1.0 / block.columns.max(1) as f32;
        let top = HOUR_PX * f32::from(block.start) / 60.0;
        let height = (HOUR_PX * f32::from(block.end.saturating_sub(block.start)) / 60.0).max(20.0);
        canvas = canvas.child(
            view()
                .absolute()
                .top(px(top))
                .left(px(lane_left))
                .right(px(4.0))
                .h(px(height))
                .child(
                    button()
                        .absolute()
                        .top(px(0.0))
                        .h(px(height))
                        .left(relative(width * block.column as f32))
                        .w(relative(width))
                        .px_2()
                        .rounded_md()
                        .bg(move || nsh_base::theme().primary)
                        .on_press(move || {
                            ui.form.set(None);
                            ui.confirm_delete.set(false);
                            ui.selected.set(Some(id.clone()));
                        })
                        .child(
                            text(entry.title.clone())
                                .text_xs()
                                .number_of_lines(2u32)
                                .text_color(move || nsh_base::theme().primary_foreground),
                        ),
                ),
        );
    }
    if day == today() {
        let now = Local::now();
        let minute = now.hour() * 60 + now.minute();
        canvas = canvas.child(
            view()
                .absolute()
                .top(px(HOUR_PX * minute as f32 / 60.0))
                .left(px(lane_left))
                .right(px(0.0))
                .h(px(2.0))
                .bg(move || nsh_base::theme().destructive),
        );
    }
    let all_day: Vec<_> = calendar::entries_on(&snap.entries, day)
        .into_iter()
        .filter(|e| e.all_day)
        .cloned()
        .collect();
    let mut top = view();
    for entry in &all_day {
        top = top.child(entry_button(entry, day, ui));
    }
    view().child(top).child(canvas)
}

fn free_time(ui: Ui) -> impl IntoElement {
    let snap = ui.snap.get();
    let writable = ui.writable();
    let mut row = view().flex_row().flex_wrap().gap_2().px_4();
    if !(snap.loaded && !snap.loading && snap.error.is_none()) {
        return row;
    }
    let gaps = timeline::gaps(
        &snap.entries,
        snap.day,
        timeline::Interval {
            start: 480,
            end: 1200,
        },
    );
    if gaps.is_empty() {
        return row.child(text("No free time between 08:00 and 20:00.").text_sm());
    }
    for gap in gaps {
        let label = format!(
            "{:02}:{:02}–{:02}:{:02} free",
            gap.start / 60,
            gap.start % 60,
            gap.end / 60,
            gap.end % 60
        );
        row = row.child(chip(label, false, move || {
            if writable {
                ui.open_form(form_at(gap.start, gap.end.min(gap.start + 60)));
            }
        }));
    }
    row
}

fn form_card(ui: Ui, form: Form) -> impl IntoElement {
    let mut commitments = view().flex_row().gap_2();
    for (key, label) in calendar::COMMITMENTS {
        let on = form.commitment == key;
        commitments = commitments.child(chip(label, on, move || {
            ui.form.update(|f| {
                if let Some(f) = f {
                    f.commitment = key;
                }
            })
        }));
    }
    let timed = !form.all_day;
    let times = form.clone();
    view()
        .mx_4()
        .p_4()
        .gap_3()
        .rounded_lg()
        .border(px(1.0))
        .border_color(move || nsh_base::theme().border)
        .bg(move || nsh_base::theme().card)
        .child(
            input()
                .value(move || ui.title.get())
                .on_change(move |value: &str| ui.title.set(value.to_owned()))
                .on_submit(move |_: &str| submit(ui))
                .placeholder("New entry")
                .px_3()
                .py_2()
                .border(px(1.0))
                .border_color(move || nsh_base::theme().input)
                .rounded_lg(),
        )
        .child(
            view()
                .flex_row()
                .gap_2()
                .child(chip("Timed", timed, move || set_all_day(ui, false)))
                .child(chip("All day", !timed, move || set_all_day(ui, true)))
                .child(
                    text(if timed {
                        format!("{}–{}", times.start, times.end)
                    } else {
                        "all day".to_owned()
                    })
                    .text_sm()
                    .text_color(move || muted()),
                ),
        )
        .child(commitments)
        .child(dynamic(move || {
            ui.form_error.get().map(|e| {
                text(e)
                    .text_xs()
                    .text_color(move || nsh_base::theme().destructive)
                    .into_element()
            })
        }))
        .child(
            view()
                .flex_row()
                .gap_2()
                .child(chip("Save", true, move || submit(ui)))
                .child(chip("Cancel", false, move || ui.form.set(None))),
        )
}

fn set_all_day(ui: Ui, all_day: bool) {
    ui.form.update(|f| {
        if let Some(f) = f {
            f.all_day = all_day;
        }
    });
}

fn submit(ui: Ui) {
    let Some(form) = ui.form.get() else {
        return;
    };
    let draft = Draft {
        title: ui.title.get(),
        day: snap_day(ui),
        start: form.start,
        end: form.end,
        all_day: form.all_day,
        kind: "event".to_owned(),
        commitment: form.commitment.to_owned(),
    };
    if let Err(why) = save_entry(draft) {
        ui.form_error.set(Some(why));
    }
}

/// The sheet for one entry: what it is, quarter-hour moves, undo and a two-tap delete.
fn sheet(ui: Ui, entry: CalendarEntry) -> impl IntoElement {
    let snap = ui.snap.get();
    let writable = ui.writable();
    let id = entry.id.clone();
    let day = snap.day;
    let mut card = view()
        .mx_4()
        .p_4()
        .gap_3()
        .rounded_lg()
        .border(px(1.0))
        .border_color(move || nsh_base::theme().border)
        .bg(move || nsh_base::theme().card)
        .child(
            view()
                .flex_row()
                .justify_between()
                .child(
                    text(entry.title.clone())
                        .text_lg()
                        .font_weight(FontWeight::SEMIBOLD),
                )
                .child(chip("Close", false, move || ui.selected.set(None))),
        )
        .child(
            text(format!(
                "{} · {} · {}",
                calendar::time_label(&entry, day),
                calendar::kind_label(&entry.kind),
                entry.commitment
            ))
            .text_sm()
            .text_color(move || muted()),
        )
        .child(
            entry
                .location
                .clone()
                .filter(|l| !l.is_empty())
                .map(|l| text(l).text_sm()),
        )
        .child(
            entry
                .notes
                .clone()
                .filter(|n| !n.is_empty())
                .map(|n| text(n).text_sm()),
        );
    if !writable {
        return card.child(
            text("Read-only: this day comes from the Mac's copy.")
                .text_xs()
                .text_color(move || muted()),
        );
    }
    if !entry.all_day {
        let moves: [(&'static str, i64, bool); 4] = [
            ("−15 min", -15, false),
            ("+15 min", 15, false),
            ("Shorter", -15, true),
            ("Longer", 15, true),
        ];
        let mut row = view().flex_row().gap_2();
        for (label, minutes, resize) in moves {
            let entry = entry.clone();
            row = row.child(chip(label, false, move || {
                if let Err(why) = nudge(&entry, minutes, resize) {
                    ui.note.set(Some(why));
                }
            }));
        }
        card = card.child(row);
    }
    if let Some((before, after)) = snap.undo.clone().filter(|(_, after)| after.id == id) {
        card = card.child(chip("Undo time change", false, move || {
            change_time(after.clone(), before.clone(), true)
        }));
    }
    let delete_id = id.clone();
    card.child(dynamic(move || {
        if ui.confirm_delete.get() {
            let delete_id = delete_id.clone();
            view()
                .flex_row()
                .gap_2()
                .child(chip("Delete", true, move || {
                    delete_entry(delete_id.clone());
                    ui.selected.set(None);
                    ui.confirm_delete.set(false);
                }))
                .child(chip("Keep", false, move || ui.confirm_delete.set(false)))
                .into_element()
        } else {
            chip("Delete…", false, move || ui.confirm_delete.set(true)).into_element()
        }
    }))
}

#[component]
pub fn CalendarScreen() -> impl IntoElement {
    let mut rx = subscribe();
    let initial = rx.borrow_and_update().clone();
    let seen = initial.saved;
    let ui = Ui {
        snap: signal(initial),
        grid: signal(false),
        free: signal(false),
        selected: signal(None::<String>),
        confirm_delete: signal(false),
        form: signal(None::<Form>),
        title: signal(String::new()),
        form_error: signal(None::<String>),
        note: signal(None::<String>),
    };
    let last_saved = signal(seen);
    spawn(async move {
        while rx.changed().await.is_ok() {
            let next = rx.borrow_and_update().clone();
            if next.saved != last_saved.get() {
                last_saved.set(next.saved);
                ui.form.set(None);
            }
            ui.snap.set(next);
        }
    });
    load();
    let header = view()
        .px_4()
        .pt_4()
        .pb_2()
        .gap_2()
        .child(
            view()
                .flex_row()
                .justify_between()
                .items_center()
                .child(
                    text(move || ui.snap.with(|s| s.day.format("%A, %-d %B").to_string()))
                        .text_xl()
                        .font_weight(FontWeight::SEMIBOLD),
                )
                .child(chip("Today", false, || open_day(today()))),
        )
        .child(dynamic(move || {
            ui.snap.with(|s| s.source.clone()).map(|line| {
                let error = ui.snap.with(|s| s.sync_error);
                text(line)
                    .text_xs()
                    .text_color(move || {
                        if error {
                            nsh_base::theme().destructive
                        } else {
                            muted()
                        }
                    })
                    .into_element()
            })
        }))
        .child(
            view()
                .flex_row()
                .gap_2()
                .child(dynamic(move || {
                    chip(
                        if ui.grid.get() { "List" } else { "Grid" },
                        false,
                        move || ui.grid.update(|g| *g = !*g),
                    )
                    .into_element()
                }))
                .child(chip("Refresh", false, || emit(CalendarEvent::Refresh)))
                .child(dynamic(move || {
                    let on = ui.free.get();
                    chip(
                        if on {
                            "Hide free time"
                        } else {
                            "Show free time"
                        },
                        on,
                        move || ui.free.update(|f| *f = !*f),
                    )
                    .into_element()
                }))
                .child(dynamic(move || {
                    ui.writable().then(|| {
                        chip("+ New", true, move || ui.open_form(form_at(540, 600))).into_element()
                    })
                })),
        )
        .child(dynamic(move || week_strip(ui).into_element()))
        .child(dynamic(move || {
            let (event_count, all_day_count) = ui.snap.with(|s| {
                let all_day = s.entries.iter().filter(|e| e.all_day).count();
                (s.entries.len(), all_day)
            });
            view()
                .flex_row()
                .gap_2()
                .child(crate::nsh_ui::metric_tile(
                    "EVENTS",
                    event_count.to_string(),
                    if event_count == 0 {
                        "Calendar clear"
                    } else {
                        "Scheduled"
                    },
                ))
                .child(crate::nsh_ui::metric_tile(
                    "ALL DAY",
                    all_day_count.to_string(),
                    "Blocks",
                ))
                .into_element()
        }))
        .child(dynamic(move || {
            let error = ui.snap.with(|s| s.error.clone()).or_else(|| ui.note.get());
            error.map(|e| {
                text(e)
                    .text_xs()
                    .text_color(move || nsh_base::theme().destructive)
                    .into_element()
            })
        }));
    view().flex_1().child(header).child(
        scroll().flex_1().child(
            view()
                .gap_3()
                .pb_6()
                .child(dynamic(move || {
                    ui.free.get().then(|| free_time(ui).into_element())
                }))
                .child(dynamic(move || {
                    ui.form.get().map(|form| form_card(ui, form).into_element())
                }))
                .child(dynamic(move || {
                    let id = ui.selected.get()?;
                    let entry = ui
                        .snap
                        .with(|s| s.entries.iter().find(|e| e.id == id).cloned())?;
                    Some(sheet(ui, entry).into_element())
                }))
                .child(dynamic(move || {
                    if ui.grid.get() {
                        grid(ui).into_element()
                    } else {
                        list(ui).into_element()
                    }
                })),
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::phone_calendar::test_snapshot;

    fn entry(id: &str, title: &str, starts: &str, ends: &str) -> CalendarEntry {
        CalendarEntry {
            title: title.to_owned(),
            starts_at: starts.to_owned(),
            ends_at: ends.to_owned(),
            all_day: false,
            commitment: "planned".to_owned(),
            id: id.to_owned(),
            kind: "event".to_owned(),
            location: None,
            notes: None,
            source: "manual".to_owned(),
        }
    }

    #[test]
    fn a_week_from_the_copy_is_read_only_and_says_so() {
        let day = NaiveDate::from_ymd_opt(2026, 11, 9).unwrap();
        let mut snap = subscribe().borrow().clone();
        snap.day = day;
        snap.loaded = true;
        snap.origin = Origin::Copy;
        assert!(!snap.writable(true), "the copy is never written");
        snap.origin = Origin::Store;
        assert!(snap.writable(true));
        assert!(!snap.writable(false));
        assert_eq!(form_at(480, 540).start, "08:00");
        assert_eq!(form_at(1410, 1500).end, "01:00");
    }

    #[test]
    fn the_screen_draws_a_day_opens_a_sheet_and_switches_to_the_grid() {
        use nsh_core::element::Root;
        use nsh_engine::EngineConfig;
        use nsh_testing::Headless;

        let day = NaiveDate::from_ymd_opt(2026, 11, 10).unwrap();
        test_snapshot(|snap| {
            snap.day = day;
            snap.week = calendar::week_dates(day);
            snap.origin = Origin::Copy;
            snap.loaded = true;
            snap.entries = vec![entry(
                "a",
                "Dentist",
                "2026-11-10T09:00:00",
                "2026-11-10T09:30:00",
            )];
        });
        let mut app = Headless::start(EngineConfig {
            ui_sets: vec![nsh_base::registry()],
            ..EngineConfig::new(Root::of::<CalendarScreen>())
        })
        .expect("the Calendar screen starts");
        app.wait_for("text:Tuesday, 10 November").unwrap();
        app.wait_for("text:Read-only copy from Mac").unwrap();
        app.press("text:Dentist").unwrap();
        app.wait_for("text:Read-only: this day comes from the Mac's copy.")
            .unwrap();
        app.press("text:Close").unwrap();
        app.press("text:Grid").unwrap();
        app.wait_for("text:List").unwrap();
        app.press("text:Show free time").unwrap();
        app.wait_for("text:08:00–09:00 free").unwrap();
    }
}
