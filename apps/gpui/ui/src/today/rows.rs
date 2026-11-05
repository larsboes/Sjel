//! Entry rows, task rows, project rows, and section layout cards.
use gpui::{
    div, prelude::*, px, rgb, rgba, AnyElement, Context, FontWeight, MouseButton,
    SharedString,
};
use gpui_kit::component::checkbox::Checkbox;
use sjel_gpui_core::calendar;
use sjel_gpui_core::today::{Entry, Project, Spot, Task};

use crate::primitives::{priority_pill, progress_bar};

use super::types::{due_mark, TodayEvent};
use super::Today;

impl Today {
    pub(super) fn entry_row(&self, entry: &Entry, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        let day = self.day();
        let rail = calendar::kind_color(&entry.kind);
        let mark = match entry.spot {
            Spot::Now => Some("NOW"),
            Spot::Next => Some("NEXT"),
            Spot::Past | Spot::Later => None,
        };
        let title_color = match entry.spot {
            Spot::Past => palette.muted,
            _ => palette.text,
        };
        div()
            .id(SharedString::from(format!("entry-{}", entry.id)))
            .flex()
            .items_center()
            .gap_3()
            .px(self.density.pad_x())
            .py(self.density.pad_y())
            .min_h(self.density.row_height())
            .cursor_pointer()
            .child(
                div()
                    .flex_none()
                    .w(px(3.))
                    .h(px(28.))
                    .rounded_full()
                    .bg(rgb(rail)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(1.))
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(title_color))
                            .when(entry.spot == Spot::Past, |title| title.line_through())
                            .child(entry.title.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(format!("{} · {}", entry.when, entry.where_)),
                    ),
            )
            .when_some(mark, |row, mark| {
                row.child(
                    div()
                        .flex_none()
                        .px_2()
                        .py(px(2.))
                        .rounded(px(6.))
                        .bg(rgb(palette.accent))
                        .text_color(rgb(0xffffff))
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(mark),
                )
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |_, _, _, cx| cx.emit(TodayEvent::OpenDay(day))),
            )
            .into_any_element()
    }

    pub(super) fn task_row(&self, task: &Task, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        let writable = self.store.is_some();
        let path = task.path.clone();
        let second = task.project.clone();
        div()
            .id(("today-task", index))
            .flex()
            .items_center()
            .gap_3()
            .px(self.density.pad_x())
            .py(self.density.pad_y())
            .min_h(self.density.row_height())
            .when(!writable, |row| row.opacity(0.6))
            .child(
                div().flex_none().child(
                    Checkbox::new(("today-task-done", index))
                        .checked(false)
                        .on_click(cx.listener(move |view, done: &bool, window, cx| {
                            if view.store.is_none() {
                                return;
                            }
                            view.toggle(path.clone(), *done, window, cx)
                        })),
                ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(1.))
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(palette.text))
                            .truncate()
                            .child(task.title.clone()),
                    )
                    .when(!second.is_empty(), |column| {
                        column.child(div().text_xs().text_color(rgb(palette.muted)).child(second))
                    }),
            )
            .when_some(
                task.due
                    .as_deref()
                    .map(|due| due_mark(due, task.overdue, palette)),
                |row, mark| row.child(mark),
            )
            .when(task.priority < 2, |row| {
                row.child(priority_pill(task.priority, palette))
            })
            .into_any_element()
    }

    pub(super) fn project_row(&self, project: &Project, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        let name = project.name.clone();
        div()
            .id(("today-project", index))
            .flex()
            .items_center()
            .gap_3()
            .px(self.density.pad_x())
            .py(self.density.pad_y())
            .min_h(self.density.row_height())
            .cursor_pointer()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap(px(1.))
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(palette.text))
                            .truncate()
                            .child(project.title.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(palette.muted))
                            .child(project.line()),
                    ),
            )
            .child(div().flex_none().child(progress_bar(
                project.progress(),
                palette.accent,
                palette,
            )))
            .child(
                div()
                    .flex_none()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child("→"),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |_, _, _, cx| cx.emit(TodayEvent::OpenProject(name.clone()))),
            )
            .into_any_element()
    }

    pub(super) fn section(
        &self,
        title: &'static str,
        empty: &'static str,
        rows: Vec<AnyElement>,
    ) -> AnyElement {
        let palette = self.palette;
        let body = if rows.is_empty() {
            div()
                .rounded(px(12.))
                .bg(rgba(palette.surface))
                .border_1()
                .border_color(rgba(palette.border))
                .px_4()
                .py_3()
                .text_sm()
                .text_color(rgb(palette.muted))
                .child(empty)
                .into_any_element()
        } else {
            crate::list(rows, palette)
        };
        crate::section(title, body, palette).into_any_element()
    }

    pub(super) fn section_content(
        &self,
        empty: &'static str,
        rows: Vec<AnyElement>,
        rest: usize,
    ) -> AnyElement {
        let palette = self.palette;
        let body = if rows.is_empty() {
            div()
                .rounded(px(12.))
                .bg(rgba(palette.surface))
                .border_1()
                .border_color(rgba(palette.border))
                .px_4()
                .py_3()
                .text_sm()
                .text_color(rgb(palette.muted))
                .child(empty)
                .into_any_element()
        } else {
            crate::list(rows, palette)
        };
        match rest {
            0 => body,
            more => div()
                .flex()
                .flex_col()
                .gap_2()
                .child(body)
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(palette.muted))
                        .child(format!("and {more} more in Projects")),
                )
                .into_any_element(),
        }
    }

    pub(super) fn refresh(&self, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        div()
            .id("today-refresh")
            .text_xs()
            .text_color(rgb(palette.accent))
            .cursor_pointer()
            .child(if self.loading {
                "Reading…"
            } else {
                "Refresh"
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, _, cx| cx.emit(TodayEvent::Refresh)),
            )
            .into_any_element()
    }
}
