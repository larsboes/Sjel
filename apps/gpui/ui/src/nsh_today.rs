//! NSH renderer for the phone Today state.

use chrono::NaiveDate;
use nsh_base::{button, checkbox};
use nsh_core::prelude::*;
use sjel_gpui_core::today::{Entry, Project, Task, Today as DayView};

use crate::phone_today::{
    due_label, emit, rest_line, spot_label, subscribe, toggle, Snapshot, TodayEvent,
};

fn muted() -> AdaptiveColor {
    nsh_base::theme().muted_foreground
}

fn heading(label: &'static str) -> impl IntoElement {
    text(label)
        .text_xs()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(move || muted())
}

fn card() -> View {
    crate::nsh_ui::surface_card()
}

fn pill(label: &'static str, event: TodayEvent) -> impl IntoElement {
    button()
        .px_3()
        .py_2()
        .rounded_full()
        .border(px(1.0))
        .border_color(move || nsh_base::theme().border)
        .accessibility_label(label)
        .on_press(move || emit(event.clone()))
        .child(text(label).text_sm())
}

fn entry_row(entry: Entry, day: NaiveDate) -> impl IntoElement {
    let mark = spot_label(entry.spot);
    button()
        .px_4()
        .py_3()
        .gap_1()
        .on_press(move || emit(TodayEvent::OpenDay(day)))
        .child(
            view()
                .flex_row()
                .justify_between()
                .child(
                    text(entry.title.clone())
                        .text_base()
                        .font_weight(FontWeight::MEDIUM),
                )
                .child(mark.map(|m| text(m).text_xs().font_weight(FontWeight::SEMIBOLD))),
        )
        .child(
            text(format!("{} · {}", entry.when, entry.where_))
                .text_xs()
                .text_color(move || muted()),
        )
}

fn task_row(task: Task, writable: bool) -> impl IntoElement {
    let path = task.path.clone();
    let due = due_label(&task);
    let overdue = task.overdue;
    let second = (!task.project.is_empty()).then(|| task.project.clone());
    view()
        .flex_row()
        .items_center()
        .px_4()
        .py_3()
        .gap_3()
        .child(
            checkbox()
                .checked(false)
                .indicator_class("w-6 h-6 rounded-md border-2 border-border disabled:opacity-40")
                .disabled(!writable)
                .accessibility_label(format!("Done: {}", task.title))
                .hit_slop(EdgeInsets::all(px(10.0)))
                .on_change(move |done: &bool| toggle(path.clone(), *done)),
        )
        .child(
            view()
                .flex_1()
                .gap_1()
                .child(text(task.title.clone()).text_base())
                .child(second.map(|s| text(s).text_xs().text_color(move || muted()))),
        )
        .child(due.map(move |label| {
            text(label).text_xs().text_color(move || {
                if overdue {
                    nsh_base::theme().destructive
                } else {
                    muted()
                }
            })
        }))
        .child(text(format!("P{}", task.priority)).text_xs())
}

fn project_row(project: Project) -> impl IntoElement {
    let name = project.name.clone();
    button()
        .px_4()
        .py_3()
        .gap_1()
        .on_press(move || emit(TodayEvent::OpenProject(name.clone())))
        .child(
            view()
                .flex_row()
                .justify_between()
                .child(text(project.title.clone()).text_base())
                .child(text("→").text_color(move || muted())),
        )
        .child(text(project.line()).text_xs().text_color(move || muted()))
}

fn section<T: IntoElement>(
    label: &'static str,
    empty: &'static str,
    rows: Vec<T>,
    extra: Option<String>,
) -> impl IntoElement {
    let body = if rows.is_empty() {
        card()
            .px_4()
            .py_3()
            .child(text(empty).text_sm().text_color(move || muted()))
    } else {
        let mut list = card();
        for row in rows {
            list = list.child(row);
        }
        list
    };
    view()
        .gap_2()
        .child(heading(label))
        .child(body)
        .child(extra.map(|line| text(line).text_xs().text_color(move || muted())))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum NshTodayTab {
    #[default]
    Overview,
    Day,
    Tasks,
    Projects,
}

fn metric_tile(label: &'static str, value: usize, caption: &'static str) -> impl IntoElement {
    crate::nsh_ui::metric_tile(label, value.to_string(), caption)
}

fn tab_pill(
    label: &'static str,
    tab: NshTodayTab,
    active_tab: Signal<NshTodayTab>,
) -> impl IntoElement {
    let is_active = active_tab.get() == tab;
    crate::nsh_ui::pill_tab(label, is_active, move || active_tab.set(tab))
}

fn body(snap: Snapshot, active_tab: Signal<NshTodayTab>) -> impl IntoElement {
    let Some(view_model) = snap.view.clone() else {
        return view().child(text("Reading today…").text_sm());
    };
    let day = view_model.day;
    let event_count = view_model.entries.len();
    let task_count = view_model.focus.len();
    let entries: Vec<_> = view_model
        .entries
        .iter()
        .cloned()
        .map(|e| entry_row(e, day))
        .collect();
    let tasks: Vec<_> = view_model
        .focus
        .iter()
        .cloned()
        .map(|t| task_row(t, snap.writable))
        .collect();
    let projects: Vec<_> = view_model
        .projects
        .iter()
        .cloned()
        .map(project_row)
        .collect();

    let tab = active_tab.get();
    let show_day = matches!(tab, NshTodayTab::Overview | NshTodayTab::Day);
    let show_tasks = matches!(tab, NshTodayTab::Overview | NshTodayTab::Tasks);
    let show_projects = matches!(tab, NshTodayTab::Overview | NshTodayTab::Projects);

    let tiles = view()
        .flex_row()
        .gap_3()
        .child(metric_tile(
            "EVENTS",
            event_count,
            if event_count == 0 {
                "Calendar clear"
            } else {
                "Scheduled today"
            },
        ))
        .child(metric_tile(
            "TASKS",
            task_count,
            if task_count == 0 {
                "Nothing left"
            } else {
                "Up next"
            },
        ));

    let tabs = view()
        .flex_row()
        .flex_wrap()
        .gap_2()
        .child(tab_pill("Overview", NshTodayTab::Overview, active_tab))
        .child(tab_pill("The Day", NshTodayTab::Day, active_tab))
        .child(tab_pill("Up Next", NshTodayTab::Tasks, active_tab))
        .child(tab_pill("Projects", NshTodayTab::Projects, active_tab));

    view()
        .gap_4()
        .child(tiles)
        .child(tabs)
        .child(
            show_day.then(|| section("THE DAY", "Nothing on the calendar today.", entries, None)),
        )
        .child(show_tasks.then(|| {
            view()
                .gap_2()
                .child(
                    view()
                        .flex_row()
                        .justify_between()
                        .items_center()
                        .child(heading("UP NEXT"))
                        .child(
                            button()
                                .px_2()
                                .py_1()
                                .on_press(|| emit(TodayEvent::Refresh))
                                .child(text("Refresh").text_sm()),
                        ),
                )
                .child(section(
                    "",
                    "Nothing left to do.",
                    tasks,
                    rest_line(view_model.rest),
                ))
        }))
        .child(show_projects.then(|| section("PROJECTS", "No projects yet.", projects, None)))
}

fn header(snap: Signal<Snapshot>) -> impl IntoElement {
    view()
        .gap_2()
        .child(
            view()
                .flex_row()
                .justify_between()
                .items_start()
                .child(
                    view()
                        .flex_1()
                        .gap_1()
                        .child(heading("TODAY"))
                        .child(
                            button()
                                .on_press(move || {
                                    if let Some(v) = snap.with(|s| s.view.clone()) {
                                        emit(TodayEvent::OpenDay(v.day));
                                    }
                                })
                                .child(
                                    text(move || snap.with(|s| s.headline.clone()))
                                        .text_2xl()
                                        .font_weight(FontWeight::SEMIBOLD),
                                ),
                        )
                        .child(
                            text(move || {
                                snap.with(|s| {
                                    s.view.as_ref().map(DayView::summary).unwrap_or_default()
                                })
                            })
                            .text_sm()
                            .text_color(move || muted()),
                        ),
                )
                .child(
                    view()
                        .flex_row()
                        .gap_2()
                        .child(pill("♪", TodayEvent::Soundscape))
                        .child(pill("≡", TodayEvent::Activity))
                        .child(pill("⚙", TodayEvent::Settings)),
                ),
        )
        .child(
            crate::nsh_ui::surface_card()
                .px_3()
                .py_2()
                .gap_1()
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(
                            text(move || snap.with(|s| s.source.clone()))
                                .text_xs()
                                .text_color(move || muted())
                                .flex_1(),
                        )
                        .child(
                            button()
                                .px_2()
                                .py_0p5()
                                .rounded_full()
                                .border(px(1.0))
                                .border_color(move || nsh_base::theme().border)
                                .on_press(|| emit(TodayEvent::Activity))
                                .child(
                                    text(move || {
                                        snap.with(|s| {
                                            if s.travel
                                                .as_ref()
                                                .is_some_and(|(_, waiting)| *waiting)
                                            {
                                                "Queued · Sync"
                                            } else if s.show_pair {
                                                "Offline"
                                            } else {
                                                "Sync status"
                                            }
                                        })
                                    })
                                    .text_xs()
                                    .font_weight(FontWeight::MEDIUM),
                                ),
                        ),
                )
                .child(dynamic(move || {
                    snap.with(|s| s.note.clone())
                        .map(|note| text(note).text_xs().into_element())
                }))
                .child(dynamic(move || {
                    snap.with(|s| s.travel.clone()).map(|(line, waiting)| {
                        text(line)
                            .text_xs()
                            .text_color(move || {
                                if waiting {
                                    nsh_base::theme().destructive
                                } else {
                                    muted()
                                }
                            })
                            .into_element()
                    })
                })),
        )
        .child(dynamic(move || {
            snap.with(|s| s.show_pair).then(|| {
                button()
                    .py_3()
                    .rounded_lg()
                    .items_center()
                    .bg(move || nsh_base::theme().primary)
                    .on_press(|| emit(TodayEvent::Settings))
                    .child(
                        text("Pair this phone with your Mac")
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(move || nsh_base::theme().primary_foreground),
                    )
                    .into_element()
            })
        }))
}

fn capture_bar() -> impl IntoElement {
    view()
        .gap_3()
        .child(
            button()
                .flex_row()
                .justify_between()
                .items_center()
                .px_4()
                .py_3()
                .rounded_xl()
                .border(px(1.0))
                .border_color(move || nsh_base::theme().border)
                .bg(move || nsh_base::theme().card)
                .on_press(|| emit(TodayEvent::Capture))
                .child(
                    text("Capture note, link or task…")
                        .text_sm()
                        .text_color(move || muted()),
                )
                .child(text("+ New").text_xs().font_weight(FontWeight::SEMIBOLD)),
        )
        .child(
            view()
                .flex_row()
                .gap_2()
                .child(crate::nsh_ui::action_circle(
                    "@",
                    "People & Places",
                    false,
                    || emit(TodayEvent::People),
                ))
                .child(crate::nsh_ui::action_circle(
                    "D",
                    "Documents",
                    false,
                    || emit(TodayEvent::Documents),
                ))
                .child(crate::nsh_ui::action_circle("P", "Projects", false, || {
                    emit(TodayEvent::Projects)
                }))
                .child(crate::nsh_ui::action_circle("I", "Interior", false, || {
                    emit(TodayEvent::Interior)
                }))
                .child(crate::nsh_ui::action_circle(
                    "€",
                    "Finance",
                    false,
                    || emit(TodayEvent::Finance),
                ))
                .child(crate::nsh_ui::action_circle("W", "Wishlist", false, || {
                    emit(TodayEvent::Inventory)
                })),
        )
}

#[component]
pub fn TodayScreen() -> impl IntoElement {
    let mut source = subscribe();
    let snap = signal(source.borrow_and_update().clone());
    let active_tab = signal(NshTodayTab::Overview);
    spawn(async move {
        while source.changed().await.is_ok() {
            let next = source.borrow_and_update().clone();
            snap.set(next);
        }
    });
    scroll().flex_1().child(
        view()
            .px_4()
            .pt_4()
            .pb_6()
            .gap_4()
            .child(header(snap))
            .child(capture_bar())
            .child(dynamic(move || {
                snap.with(|s| s.busy.clone())
                    .map(|_| text("Saving…").text_xs().into_element())
            }))
            .child(dynamic(move || body(snap.get(), active_tab).into_element())),
    )
}
