//! Guided Morning / Weekly Review flow for Projects and Tasks.

use chrono::{Duration, Local};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::{div, prelude::*, px, rgb, rgba, AnyElement, Context, FontWeight, SharedString};
use sjel_gpui_core::shell::{GlobalTask, ProjectSummary};

use crate::ely::buttons::{Button, ButtonVariant};
use crate::ely::charts::{BarChart, Series};
use crate::ely::theme::ControlSize;
use crate::primitives::{chart_card, section};
use crate::theme::{tint, Density, Palette};

use super::charts::due_counts_window;
use super::{Projects, ProjectsTab};

impl Projects {
    pub(super) fn review_view(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let today_date = Local::now().date_naive();
        let today = today_date.format("%Y-%m-%d").to_string();
        let tasks: &[GlobalTask] = self.all_tasks.value.as_deref().unwrap_or(&[]);
        let projects: &[ProjectSummary] = self.projects.value.as_deref().unwrap_or(&[]);

        let counts = due_counts_window(tasks, today_date, 7);
        let labels: Vec<String> = (0..7)
            .map(|offset| match offset {
                0 => "Today".to_string(),
                1 => "Tomorrow".to_string(),
                _ => (today_date + Duration::days(offset))
                    .format("%a %d")
                    .to_string(),
            })
            .collect();

        let day_dates: Vec<String> = (0..7)
            .map(|offset| (today_date + Duration::days(offset)).format("%Y-%m-%d").to_string())
            .collect();
        let chart_owner = cx.entity();

        let week_chart = chart_card(
            "NEXT 7 DAYS LOAD",
            Some("Active tasks due per day · click bar to filter All Tasks".into()),
            BarChart::new("projects-review-week", labels)
                .series(Series::new("Due tasks", counts))
                .format(|v| format!("{v:.0}"))
                .on_select(move |ix, _, cx| {
                    if let Some(date) = day_dates.get(ix).cloned() {
                        chart_owner.update(cx, |view, cx| {
                            view.due_date_filter = Some(date);
                            view.review_mode = false;
                            view.tab = ProjectsTab::AllTasks;
                            cx.notify();
                        });
                    }
                })
                .h(px(140.))
                .w_full()
                .into_any_element(),
            palette,
        );

        let overdue: Vec<&GlobalTask> = tasks
            .iter()
            .filter(|t| {
                let st = t.status();
                st != "done"
                    && st != "cancelled"
                    && t.due
                        .as_deref()
                        .is_some_and(|d| !d.is_empty() && d < today.as_str())
            })
            .collect();

        let blocked: Vec<&GlobalTask> = tasks
            .iter()
            .filter(|t| {
                let st = t.status();
                st != "done"
                    && st != "cancelled"
                    && (st == "blocked"
                        || st == "waiting"
                        || t.fields
                            .get("blocked_by")
                            .is_some_and(|v| !v.trim().is_empty()))
            })
            .collect();

        let pinned_next: Vec<&GlobalTask> = tasks
            .iter()
            .filter(|t| {
                let st = t.status();
                st != "done"
                    && st != "cancelled"
                    && t.fields
                        .get("next")
                        .is_some_and(|v| v.eq_ignore_ascii_case("true"))
            })
            .collect();

        let mut active_projects: Vec<&ProjectSummary> = projects
            .iter()
            .filter(|p| !p.archived && p.open > 0)
            .collect();
        active_projects.sort_by(|a, b| b.open.cmp(&a.open));

        let overdue_rows: Vec<AnyElement> = overdue
            .iter()
            .take(8)
            .enumerate()
            .map(|(ix, task)| {
                let t_done = (*task).clone();
                let t_today = (*task).clone();
                let today_str = today.clone();
                let due_label = task.due.clone().unwrap_or_default();
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .rounded(px(6.))
                    .bg(rgba(palette.raised))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_sm()
                                    .truncate()
                                    .text_color(rgb(palette.text))
                                    .child(task.title.clone()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(palette.muted))
                                    .child(task.project.clone()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(palette.red))
                                    .child(due_label),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(
                                Button::new(
                                    SharedString::from(format!("review-today-{ix}")),
                                    "Today",
                                )
                                .size(ControlSize::Sm)
                                .variant(ButtonVariant::Ghost)
                                .on_click(cx.listener(move |view, _, window, cx| {
                                    view.set_task_property(
                                        t_today.id.clone(),
                                        "due",
                                        today_str.clone(),
                                        window,
                                        cx,
                                    );
                                })),
                            )
                            .child(
                                Button::new(
                                    SharedString::from(format!("review-done-{ix}")),
                                    "Done ✓",
                                )
                                .size(ControlSize::Sm)
                                .variant(ButtonVariant::Primary)
                                .on_click(cx.listener(move |view, _, window, cx| {
                                    view.toggle_global_task(
                                        t_done.id.clone(),
                                        !t_done.done,
                                        window,
                                        cx,
                                    );
                                })),
                            ),
                    )
                    .into_any_element()
            })
            .collect();

        let blocked_rows: Vec<AnyElement> = blocked
            .iter()
            .take(6)
            .enumerate()
            .map(|(ix, task)| {
                let target = (*task).clone();
                let blocker = task
                    .fields
                    .get("blocked_by")
                    .cloned()
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or_else(|| task.status().to_string());
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .rounded(px(6.))
                    .bg(rgba(palette.raised))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(div().text_sm().truncate().child(task.title.clone()))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(palette.amber))
                                    .child(blocker),
                            ),
                    )
                    .child(
                        Button::new(SharedString::from(format!("review-blocked-{ix}")), "Inspect")
                            .size(ControlSize::Sm)
                            .variant(ButtonVariant::Ghost)
                            .on_click(cx.listener(move |view, _, window, cx| {
                                view.review_mode = false;
                                view.tab = ProjectsTab::AllTasks;
                                view.select_task(&target, window, cx);
                            })),
                    )
                    .into_any_element()
            })
            .collect();

        let next_rows: Vec<AnyElement> = pinned_next
            .iter()
            .take(6)
            .enumerate()
            .map(|(ix, task)| {
                let target = (*task).clone();
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .rounded(px(6.))
                    .bg(rgba(tint(palette.accent, 0x18)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(div().text_sm().truncate().child(task.title.clone()))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(palette.muted))
                                    .child(task.project.clone()),
                            ),
                    )
                    .child(
                        Button::new(SharedString::from(format!("review-next-{ix}")), "Open")
                            .size(ControlSize::Sm)
                            .variant(ButtonVariant::Ghost)
                            .on_click(cx.listener(move |view, _, window, cx| {
                                view.review_mode = false;
                                view.tab = ProjectsTab::AllTasks;
                                view.select_task(&target, window, cx);
                            })),
                    )
                    .into_any_element()
            })
            .collect();

        let project_rows: Vec<AnyElement> = active_projects
            .iter()
            .take(6)
            .enumerate()
            .map(|(ix, project)| {
                let name = project.name.clone();
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .rounded(px(6.))
                    .bg(rgba(palette.raised))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(div().text_sm().truncate().child(project.name.clone()))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(palette.muted))
                                    .child(format!("{} open · {} done", project.open, project.done)),
                            ),
                    )
                    .child(
                        Button::new(SharedString::from(format!("review-proj-{ix}")), "Open")
                            .size(ControlSize::Sm)
                            .variant(ButtonVariant::Ghost)
                            .on_click(cx.listener(move |view, _, _, cx| {
                                view.review_mode = false;
                                view.open_project(name.clone(), cx);
                            })),
                    )
                    .into_any_element()
            })
            .collect();

        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .mb_3()
            .child(
                div()
                    .text_lg()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(palette.text))
                    .child("Morning / Weekly Review"),
            )
            .child(
                Button::new("review-exit", "Exit Review")
                    .size(ControlSize::Sm)
                    .variant(ButtonVariant::Ghost)
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.review_mode = false;
                        cx.notify();
                    })),
            );

        let columns = if self.density == Density::Mobile {
            div().flex().flex_col().gap_3()
        } else {
            div().flex().items_start().gap_3()
        };

        div()
            .id("projects-review")
            .size_full()
            .track_scroll(&self.scroll)
            .overflow_y_scroll()
            .vertical_scrollbar(&self.scroll)
            .flex()
            .flex_col()
            .gap_3()
            .child(header)
            .child(week_chart)
            .child(
                columns
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .child(section(
                                "Overdue / Drifting",
                                if overdue_rows.is_empty() {
                                    div()
                                        .text_xs()
                                        .text_color(rgb(palette.muted))
                                        .child("No overdue tasks.")
                                        .into_any_element()
                                } else {
                                    div().flex().flex_col().gap_1p5().children(overdue_rows).into_any_element()
                                },
                                palette,
                            ))
                            .child(section(
                                "Pinned Next",
                                if next_rows.is_empty() {
                                    div()
                                        .text_xs()
                                        .text_color(rgb(palette.muted))
                                        .child("Pin a task with ☆ Pin Next in its panel.")
                                        .into_any_element()
                                } else {
                                    div().flex().flex_col().gap_1p5().children(next_rows).into_any_element()
                                },
                                palette,
                            )),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_3()
                            .child(section(
                                "Blocked on Others",
                                if blocked_rows.is_empty() {
                                    div()
                                        .text_xs()
                                        .text_color(rgb(palette.muted))
                                        .child("Nothing blocked.")
                                        .into_any_element()
                                } else {
                                    div().flex().flex_col().gap_1p5().children(blocked_rows).into_any_element()
                                },
                                palette,
                            ))
                            .child(section(
                                "Active Projects",
                                if project_rows.is_empty() {
                                    div()
                                        .text_xs()
                                        .text_color(rgb(palette.muted))
                                        .child("No active projects with open tasks.")
                                        .into_any_element()
                                } else {
                                    div().flex().flex_col().gap_1p5().children(project_rows).into_any_element()
                                },
                                palette,
                            )),
                    ),
            )
            .into_any_element()
    }
}
