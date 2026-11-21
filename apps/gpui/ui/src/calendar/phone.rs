//! The phone's week strip and daily agenda over its Calendar replica.

use chrono::Duration;
use gpui::{
    div, prelude::*, rgb, rgba, Context, Div, Entity, Focusable, FontWeight, SharedString,
    Subscription, Task, Window,
};
use sjel_gpui_core::{
    calendar::{self, timeline, Draft},
    shell::CalendarEntry,
};

use super::{adapter, minute_clock};
use crate::{
    ely::{
        buttons::{Button, ButtonVariant},
        calendar::{
            jiff::{tz::TimeZone, Timestamp},
            CalendarDayView, FreeInterval,
        },
        forms::{Input, InputEvent, TextInput},
        theme::ControlSize,
    },
    phone_calendar::{self as data, CalendarEvent, Origin, Snapshot},
    theme::{Palette, ThemeContext},
};

pub struct PhoneCalendar {
    snapshot: Snapshot,
    grid: bool,
    free: bool,
    selected: Option<String>,
    delete: bool,
    draft: Option<Draft>,
    error: Option<String>,
    title: Entity<TextInput>,
    zone: TimeZone,
    _input: [Subscription; 3],
    _updates: Task<()>,
    _clock: Task<()>,
}

impl PhoneCalendar {
    pub fn new(window: &mut Window, keyboard: fn(bool), cx: &mut Context<Self>) -> Self {
        let mut snapshots = data::subscribe();
        let snapshot = snapshots.borrow_and_update().clone();
        let title = cx.new(|cx| TextInput::new(window, cx).placeholder("Entry title"));
        let submit = cx.subscribe_in(&title, window, |view, _, event, window, cx| {
            if matches!(event, InputEvent::Submit) {
                view.submit(cx);
                if view.error.is_none() {
                    window.blur(cx);
                }
            }
        });
        let focus = title.read(cx).focus_handle(cx);
        let on_focus = cx.on_focus(&focus, window, move |_, _, _| keyboard(true));
        let on_blur = cx.on_blur(&focus, window, move |_, _, _| keyboard(false));
        let updates = cx.spawn(async move |this, cx| {
            while snapshots.changed().await.is_ok() {
                let next = snapshots.borrow_and_update().clone();
                if this
                    .update(cx, |view, cx| {
                        if next.day != view.snapshot.day || next.saved != view.snapshot.saved {
                            view.close_editor(cx);
                        }
                        view.snapshot = next;
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            snapshot,
            grid: false,
            free: false,
            selected: None,
            delete: false,
            draft: None,
            error: None,
            title,
            zone: TimeZone::system(),
            _input: [submit, on_focus, on_blur],
            _updates: updates,
            _clock: minute_clock(cx),
        }
    }

    fn writable(&self) -> bool {
        self.snapshot.writable(data::has_store())
    }

    fn close_editor(&mut self, cx: &mut Context<Self>) {
        self.selected = None;
        self.delete = false;
        self.draft = None;
        self.error = None;
        cx.notify();
    }

    fn select(&mut self, id: &str, cx: &mut Context<Self>) {
        self.close_editor(cx);
        self.selected = Some(id.to_owned());
    }

    fn open_draft(&mut self, start: u16, end: u16, cx: &mut Context<Self>) {
        if !self.writable() {
            return;
        }
        self.close_editor(cx);
        self.title.update(cx, |title, cx| title.set_text("", cx));
        self.draft = Some(Draft {
            title: String::new(),
            day: self.snapshot.day,
            start: clock(start),
            end: clock(end),
            kind: "event".into(),
            commitment: "planned".into(),
            all_day: false,
        });
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        if !self.writable() || self.snapshot.saving {
            return;
        }
        if let Some(mut draft) = self.draft.clone() {
            draft.title = self.title.read(cx).text().to_owned();
            self.error = data::save_entry(draft).err();
            cx.notify();
        }
    }

    fn gaps(&self) -> Vec<timeline::Interval> {
        if self.free
            && self.snapshot.loaded
            && !self.snapshot.loading
            && self.snapshot.error.is_none()
        {
            timeline::gaps(
                &self.snapshot.entries,
                self.snapshot.day,
                timeline::Interval {
                    start: 480,
                    end: 1200,
                },
            )
        } else {
            Vec::new()
        }
    }

    fn week(&self, palette: Palette, cx: &Context<Self>) -> Div {
        let day = self.snapshot.day;
        let entity = cx.entity();
        super::mobile_week_strip(
            day,
            &self.snapshot.week,
            &self.snapshot.entries,
            palette,
            move |_, _| data::open_day(day - Duration::days(7)),
            move |_, _| data::open_day(day + Duration::days(7)),
            move |date, window, cx| {
                window.blur(cx);
                entity.update(cx, |view, cx| view.close_editor(cx));
                data::open_day(date);
            },
        )
    }

    fn list(&self, palette: Palette, cx: &Context<Self>) -> impl IntoElement {
        let day = self.snapshot.day;
        let entries = calendar::entries_on(&self.snapshot.entries, day);
        let empty = if self.snapshot.loading {
            "Reading the day…"
        } else if self.snapshot.origin == Origin::Copy {
            "No entries on this day in the Mac’s copy."
        } else {
            "No entries on this day."
        };
        div()
            .id("calendar-agenda")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .when(
                entries.is_empty() && (self.snapshot.loaded || self.snapshot.loading),
                |list| list.child(caption(palette, empty)),
            )
            .children(entries.into_iter().map(|entry| {
                let id = entry.id.clone();
                let place = entry
                    .location
                    .as_deref()
                    .filter(|place| !place.is_empty())
                    .unwrap_or_else(|| calendar::kind_label(&entry.kind));
                div()
                    .id(SharedString::from(id.clone()))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .p_3()
                    .border_b_1()
                    .border_color(rgba(palette.border))
                    .child(
                        div()
                            .font_weight(FontWeight::MEDIUM)
                            .child(entry.title.clone()),
                    )
                    .child(caption(
                        palette,
                        format!(
                            "{} · {place} · {}",
                            calendar::time_label(entry, day),
                            entry.commitment
                        ),
                    ))
                    .on_click(cx.listener(move |view, _, _, cx| view.select(&id, cx)))
            }))
    }

    fn detail(&self, entry: &CalendarEntry, palette: Palette, cx: &Context<Self>) -> Div {
        let id = entry.id.clone();
        let mut card = column()
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(entry.title.clone()),
                    )
                    .child(
                        control("close-entry", "Close", false)
                            .on_click(cx.listener(|view, _, _, cx| view.close_editor(cx))),
                    ),
            )
            .child(caption(
                palette,
                format!(
                    "{} · {} · {}",
                    calendar::time_label(entry, self.snapshot.day),
                    calendar::kind_label(&entry.kind),
                    entry.commitment
                ),
            ))
            .children(
                [&entry.location, &entry.notes]
                    .into_iter()
                    .filter_map(|value| value.as_ref())
                    .filter(|value| !value.is_empty())
                    .cloned()
                    .map(|value| div().child(value)),
            );
        if !self.writable() {
            return card.child(caption(palette, "Read-only · Mac copy"));
        }
        if !entry.all_day {
            card = card.child(
                div().flex().flex_wrap().gap_1().children(
                    [
                        ("−15 min", -15, false),
                        ("+15 min", 15, false),
                        ("Shorter", -15, true),
                        ("Longer", 15, true),
                    ]
                    .map(|(label, minutes, resize)| {
                        let entry = entry.clone();
                        control(label, label, false)
                            .disabled(self.snapshot.saving)
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.error = data::nudge(&entry, minutes, resize).err();
                                cx.notify();
                            }))
                    }),
                ),
            );
        }
        if let Some((before, after)) = self
            .snapshot
            .undo
            .as_ref()
            .filter(|(_, after)| after.id == id)
        {
            let (before, after) = (before.clone(), after.clone());
            card = card.child(
                control("undo-time", "Undo time change", false)
                    .disabled(self.snapshot.saving)
                    .on_click(move |_, _, _| {
                        data::change_time(after.clone(), before.clone(), true)
                    }),
            );
        }
        if self.delete {
            card.child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        control("delete-entry", "Delete", true)
                            .disabled(self.snapshot.saving)
                            .on_click(cx.listener(move |view, _, _, cx| {
                                data::delete_entry(id.clone());
                                view.close_editor(cx);
                            })),
                    )
                    .child(control("keep-entry", "Keep", false).on_click(cx.listener(
                        |view, _, _, cx| {
                            view.delete = false;
                            cx.notify();
                        },
                    ))),
            )
        } else {
            card.child(
                control("confirm-delete", "Delete…", false)
                    .disabled(self.snapshot.saving)
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.delete = true;
                        cx.notify();
                    })),
            )
        }
    }

    fn form(&self, draft: &Draft, palette: Palette, cx: &Context<Self>) -> Div {
        let all_day = draft.all_day;
        column()
            .child(Input::new(&self.title))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_1()
                    .children(
                        [(false, "Timed"), (true, "All day")].map(|(all_day, label)| {
                            control(label, label, draft.all_day == all_day).on_click(cx.listener(
                                move |view, _, _, cx| {
                                    if let Some(draft) = &mut view.draft {
                                        draft.all_day = all_day;
                                        cx.notify();
                                    }
                                },
                            ))
                        }),
                    )
                    .when(!all_day, |row| {
                        row.child(caption(palette, format!("{}–{}", draft.start, draft.end)))
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_1()
                    .children(calendar::COMMITMENTS.iter().map(|&(key, label)| {
                        control(key, label, draft.commitment == key).on_click(cx.listener(
                            move |view, _, _, cx| {
                                if let Some(draft) = &mut view.draft {
                                    draft.commitment = key.to_owned();
                                    cx.notify();
                                }
                            },
                        ))
                    })),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        control("save-calendar-entry", "Save", true)
                            .disabled(self.snapshot.saving || !self.writable())
                            .on_click(cx.listener(|view, _, window, cx| {
                                view.submit(cx);
                                if view.error.is_none() {
                                    window.blur(cx);
                                }
                            })),
                    )
                    .child(control("cancel-calendar-entry", "Cancel", false).on_click(
                        cx.listener(|view, _, window, cx| {
                            window.blur(cx);
                            view.close_editor(cx);
                        }),
                    )),
            )
    }

    fn grid(&self, gaps: &[timeline::Interval], palette: Palette, cx: &Context<Self>) -> Div {
        let day = adapter::date(self.snapshot.day);
        let mut errors = Vec::new();
        let events = self
            .snapshot
            .entries
            .iter()
            .filter_map(
                |entry| match adapter::event(entry, &self.zone, self.selected.as_deref()) {
                    Ok(event) => Some(event),
                    Err(error) => {
                        errors.push(format!("{}: {error}", entry.title));
                        None
                    }
                },
            )
            .collect::<Vec<_>>();
        let view = cx.entity().downgrade();
        let mut timeline = CalendarDayView::new("phone-day-grid", day, events)
            .header(false)
            .zone(self.zone.clone())
            .free_intervals(gaps.iter().map(|gap| FreeInterval {
                day,
                start: gap.start,
                end: gap.end,
            }))
            .on_event(move |id, _, cx| {
                let _ = view.update(cx, |view, cx| view.select(id, cx));
            });
        if self.writable() && !self.snapshot.saving {
            let (slot, make, moved, resized) = (
                cx.entity().downgrade(),
                cx.entity().downgrade(),
                cx.entity().downgrade(),
                cx.entity().downgrade(),
            );
            timeline = timeline
                .on_slot(move |start, end, _, cx| {
                    let _ = slot.update(cx, |view, cx| view.slot(start, end, cx));
                })
                .on_create(move |start, end, _, cx| {
                    let _ = make.update(cx, |view, cx| view.slot(start, end, cx));
                })
                .on_move(move |id, start, end, _, cx| {
                    let _ = moved.update(cx, |view, cx| view.change_time(id, Some(start), end, cx));
                })
                .on_resize(move |id, end, _, cx| {
                    let _ = resized.update(cx, |view, cx| view.change_time(id, None, end, cx));
                });
        }
        column()
            .flex_1()
            .min_h_0()
            .when_some(errors.first().cloned(), |strip, error| {
                strip.child(caption(palette, error))
            })
            .child(div().flex_1().min_h_0().child(timeline))
    }

    fn slot(&mut self, start: Timestamp, end: Timestamp, cx: &mut Context<Self>) {
        let minute = |time: Timestamp| {
            let local = time.to_zoned(self.zone.clone());
            local.hour() as u16 * 60 + local.minute() as u16
        };
        self.open_draft(minute(start), minute(end), cx);
    }

    fn change_time(
        &mut self,
        id: &str,
        start: Option<Timestamp>,
        end: Timestamp,
        cx: &mut Context<Self>,
    ) {
        let Some(before) = self
            .snapshot
            .entries
            .iter()
            .find(|entry| entry.id == id)
            .cloned()
        else {
            return;
        };
        let mut after = before.clone();
        if let Some(start) = start {
            after.starts_at = adapter::local(start, &self.zone);
        }
        after.ends_at = adapter::local(end, &self.zone);
        match adapter::event(&after, &self.zone, None) {
            Ok(_) => data::change_time(before, after, false),
            Err(error) => self.error = Some(error),
        }
        cx.notify();
    }
}

impl Render for PhoneCalendar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = ThemeContext::of(window, cx).palette;
        let selected = self
            .selected
            .as_deref()
            .and_then(|id| self.snapshot.entries.iter().find(|entry| entry.id == id));
        let gaps = self.gaps();
        let content = if let Some(draft) = &self.draft {
            div()
                .id("calendar-draft")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .child(self.form(draft, palette, cx))
                .into_any_element()
        } else if let Some(entry) = selected {
            div()
                .id("calendar-entry")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .child(self.detail(entry, palette, cx))
                .into_any_element()
        } else if self.grid {
            self.grid(&gaps, palette, cx).into_any_element()
        } else {
            self.list(palette, cx).into_any_element()
        };
        column()
            .size_full()
            .p_3()
            .bg(rgba(palette.surface))
            .text_color(rgb(palette.text))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .text_xl()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(self.snapshot.day.format("%A, %-d %B").to_string()),
                    )
                    .child(
                        control("calendar-today", "Today", false)
                            .on_click(|_, _, _| data::open_day(data::today())),
                    ),
            )
            .when_some(self.snapshot.source.clone(), |screen, line| {
                let color = if self.snapshot.sync_error {
                    palette.red
                } else {
                    palette.muted
                };
                screen.child(div().text_sm().text_color(rgb(color)).child(line))
            })
            .child(self.week(palette, cx))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_1()
                    .child(
                        control(
                            "calendar-grid-toggle",
                            if self.grid { "List" } else { "Grid" },
                            self.grid,
                        )
                        .on_click(cx.listener(|view, _, window, cx| {
                            window.blur(cx);
                            view.close_editor(cx);
                            view.grid = !view.grid;
                        })),
                    )
                    .child(
                        control("calendar-availability", "Free time", self.free).on_click(
                            cx.listener(|view, _, _, cx| {
                                view.free = !view.free;
                                cx.notify();
                            }),
                        ),
                    )
                    .child(
                        control("calendar-refresh", "Refresh", false)
                            .on_click(|_, _, _| data::emit(CalendarEvent::Refresh)),
                    )
                    .when(self.writable(), |bar| {
                        bar.child(
                            control("calendar-new", "+ New", true)
                                .disabled(self.snapshot.saving)
                                .on_click(
                                    cx.listener(|view, _, _, cx| view.open_draft(540, 600, cx)),
                                ),
                        )
                    }),
            )
            .when(
                self.free
                    && self.snapshot.loaded
                    && self.snapshot.error.is_none()
                    && !self.snapshot.loading
                    && self.draft.is_none()
                    && selected.is_none(),
                |screen| {
                    if gaps.is_empty() {
                        return screen
                            .child(caption(palette, "No free time between 08:00 and 20:00."));
                    }
                    if self.grid {
                        return screen;
                    }
                    screen.child(div().flex().flex_wrap().gap_1().children(gaps.iter().map(
                        |gap| {
                            let (start, end) = (gap.start, gap.end);
                            control(
                                format!("free-{start}"),
                                format!("{}–{} free", clock(start), clock(end)),
                                false,
                            )
                            .disabled(!self.writable() || self.snapshot.saving)
                            .on_click(cx.listener(
                                move |view, _, _, cx| {
                                    view.open_draft(start, end.min(start + 60), cx)
                                },
                            ))
                        },
                    )))
                },
            )
            .when_some(
                self.error.clone().or_else(|| self.snapshot.error.clone()),
                |screen, error| {
                    screen.child(div().text_sm().text_color(rgb(palette.red)).child(error))
                },
            )
            .child(content)
    }
}

fn clock(minute: u16) -> String {
    format!("{:02}:{:02}", minute / 60 % 24, minute % 60)
}

fn control(
    id: impl Into<gpui::ElementId>,
    label: impl Into<SharedString>,
    selected: bool,
) -> Button {
    Button::new(id, label)
        .size(ControlSize::Md)
        .variant(if selected {
            ButtonVariant::Primary
        } else {
            ButtonVariant::Ghost
        })
}

fn column() -> Div {
    div().flex().flex_col().gap_2()
}

fn caption(palette: Palette, text: impl Into<SharedString>) -> Div {
    div()
        .text_sm()
        .text_color(rgb(palette.muted))
        .child(text.into())
}
