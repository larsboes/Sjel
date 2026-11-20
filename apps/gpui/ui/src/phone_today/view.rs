//! GPUI layout for the phone's Today route.

use gpui::{div, prelude::*, rgb, AnyElement, Context, FontWeight, Task, Window};

use super::{due_label, emit, rest_line, spot_label, subscribe, toggle, Snapshot, TodayEvent};
use crate::{
    ely::{
        buttons::{Button, ButtonVariant},
        forms::Checkbox,
        theme::ControlSize,
    },
    theme::{Palette, ThemeContext},
};

#[derive(Clone, Copy, Default, PartialEq)]
enum Section {
    #[default]
    Overview,
    Calendar,
    Tasks,
    Projects,
}

impl Section {
    const TABS: [(Self, &'static str); 4] = [
        (Self::Overview, "Overview"),
        (Self::Calendar, "The Day"),
        (Self::Tasks, "Up Next"),
        (Self::Projects, "Projects"),
    ];

    fn shows(self, section: Self) -> bool {
        self == Self::Overview || self == section
    }
}

/// The GPUI Today page observes the existing phone replica and copy state.
pub struct PhoneToday {
    snapshot: Snapshot,
    section: Section,
    _updates: Task<()>,
}

impl PhoneToday {
    /// Connects the view to the host's current Today state.
    pub fn new(cx: &mut Context<Self>) -> Self {
        let mut source = subscribe();
        let snapshot = source.borrow_and_update().clone();
        let updates = cx.spawn(async move |this, cx| {
            while source.changed().await.is_ok() {
                let snapshot = source.borrow_and_update().clone();
                if this
                    .update(cx, |view, cx| {
                        view.snapshot = snapshot;
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
            section: Section::Overview,
            _updates: updates,
        }
    }
}

fn action(label: &'static str, event: TodayEvent) -> Button {
    Button::new(label, label)
        .size(ControlSize::Lg)
        .variant(ButtonVariant::Ghost)
        .on_click(move |_, _, _| emit(event.clone()))
}

fn heading(label: &'static str) -> gpui::Div {
    div()
        .text_sm()
        .font_weight(FontWeight::SEMIBOLD)
        .child(label)
}

fn section(
    title: &'static str,
    empty: &'static str,
    rows: Vec<AnyElement>,
    palette: Palette,
) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(heading(title))
        .when(rows.is_empty(), |section| {
            section.child(div().text_sm().text_color(rgb(palette.muted)).child(empty))
        })
        .children(rows)
}

impl Render for PhoneToday {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = ThemeContext::of(window, cx).palette;
        let snap = &self.snapshot;
        let day = snap.view.as_ref().map(|today| today.day);
        let mut content = div()
            .flex()
            .flex_col()
            .gap_3()
            .p_4()
            .child(
                div().flex().h(gpui::px(48.)).items_center().children(
                    [
                        ("Soundscape", TodayEvent::Soundscape),
                        ("Activity", TodayEvent::Activity),
                        ("Settings", TodayEvent::Settings),
                    ]
                    .map(|(label, event)| div().flex_1().child(action(label, event).full_width())),
                ),
            )
            .child(heading("TODAY"))
            .child(
                Button::new("today-calendar", snap.headline.clone())
                    .size(ControlSize::Lg)
                    .disabled(day.is_none())
                    .on_click(move |_, _, _| {
                        if let Some(day) = day {
                            emit(TodayEvent::OpenDay(day));
                        }
                    }),
            )
            .when_some(
                snap.view.as_ref().map(|day| day.summary()),
                |body, summary| body.child(div().text_sm().child(summary)),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(palette.muted))
                    .child(snap.source.clone()),
            )
            .when_some(snap.note.clone(), |body, note| {
                body.child(div().text_sm().child(note))
            })
            .when_some(snap.travel.clone(), |body, (message, pending)| {
                body.child(
                    div()
                        .text_sm()
                        .text_color(rgb(if pending {
                            palette.amber
                        } else {
                            palette.muted
                        }))
                        .child(message),
                )
            })
            .when(snap.show_pair, |body| {
                body.child(action(
                    "Pair this phone with your Mac",
                    TodayEvent::Settings,
                ))
            })
            .child(action("Capture note, link or task", TodayEvent::Capture))
            .child(div().flex().flex_wrap().gap_1().children([
                action("People & Places", TodayEvent::People),
                action("Documents", TodayEvent::Documents),
                action("Project hubs", TodayEvent::Projects),
                action("Interior", TodayEvent::Interior),
                action("Finance", TodayEvent::Finance),
                action("Wishlist", TodayEvent::Inventory),
            ]))
            .child(div().flex().flex_wrap().gap_1().children(Section::TABS.map(
                |(section, label)| {
                    Button::new(format!("today-section-{label}"), label)
                        .variant(if section == self.section {
                            ButtonVariant::Primary
                        } else {
                            ButtonVariant::Ghost
                        })
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.section = section;
                            cx.notify();
                        }))
                },
            )))
            .when(snap.busy.is_some(), |body| {
                body.child(div().text_sm().child("Saving…"))
            });

        if let Some(today) = &snap.view {
            let day = today.day;
            content = content
                .when(self.section.shows(Section::Calendar), |body| {
                    let rows = today
                        .entries
                        .iter()
                        .map(|entry| {
                            let mut label =
                                format!("{} · {} · {}", entry.when, entry.title, entry.where_);
                            if let Some(spot) = spot_label(entry.spot) {
                                label.push_str(" · ");
                                label.push_str(spot);
                            }
                            Button::new(format!("today-entry-{}", entry.id), label)
                                .size(ControlSize::Lg)
                                .full_width()
                                .on_click(move |_, _, _| emit(TodayEvent::OpenDay(day)))
                                .into_any_element()
                        })
                        .collect();
                    body.child(section(
                        "THE DAY",
                        "Nothing on the calendar today.",
                        rows,
                        palette,
                    ))
                })
                .when(self.section.shows(Section::Tasks), |body| {
                    let rows = today
                        .focus
                        .iter()
                        .map(|task| {
                            let path = task.path.clone();
                            let label = due_label(task).map_or_else(
                                || format!("{} · {}", task.title, task.project),
                                |due| format!("{} · {} · {due}", task.title, task.project),
                            );
                            div()
                                .min_h(gpui::px(44.))
                                .flex()
                                .items_center()
                                .child(
                                    Checkbox::new(format!("today-task-{path}"), false)
                                        .label(label)
                                        .disabled(!snap.writable || snap.busy.is_some())
                                        .on_change(move |done, _, _| toggle(path.clone(), done)),
                                )
                                .into_any_element()
                        })
                        .collect();
                    body.child(section("UP NEXT", "Nothing left to do.", rows, palette))
                        .when_some(rest_line(today.rest), |body, rest| {
                            body.child(div().text_sm().child(rest))
                        })
                        .child(action("Refresh", TodayEvent::Refresh))
                })
                .when(self.section.shows(Section::Projects), |body| {
                    let rows = today
                        .projects
                        .iter()
                        .map(|project| {
                            let name = project.name.clone();
                            Button::new(
                                format!("today-project-{name}"),
                                format!("{} · {}", project.title, project.line()),
                            )
                            .size(ControlSize::Lg)
                            .full_width()
                            .on_click(move |_, _, _| emit(TodayEvent::OpenProject(name.clone())))
                            .into_any_element()
                        })
                        .collect();
                    body.child(section("PROJECTS", "No projects yet.", rows, palette))
                });
        }
        div()
            .id("phone-today")
            .size_full()
            .overflow_y_scroll()
            .text_color(rgb(palette.text))
            .bg(gpui::rgba(palette.surface))
            .child(content)
    }
}
