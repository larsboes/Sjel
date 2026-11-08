//! Kanban Board view for project tasks.

use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::Sizable;
use gpui_kit::{
    div, prelude::*, px, rgb, rgba, AnyElement, Context, Entity, FontWeight, Render, SharedString,
    Window,
};

use crate::primitives::{due_badge, priority_pill, status_pill};
use crate::theme::{tint, Palette};
use sjel_gpui_core::shell::ProjectRecord;

use super::tables::{parse_frontmatter_status, project_record_to_global_task};
use super::Projects;

/// Preview for a task being dragged between Kanban columns.
#[derive(Clone)]
pub(super) struct DraggedTask {
    pub(super) path: String,
    pub(super) title: String,
    pub(super) palette: Palette,
}

impl Render for DraggedTask {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let palette = self.palette;
        div()
            .max_w(px(260.))
            .truncate()
            .px_3()
            .py_2()
            .rounded(px(8.))
            .bg(rgba(palette.surface | 0xff))
            .border_1()
            .border_color(rgb(palette.accent))
            .shadow_lg()
            .text_sm()
            .font_weight(FontWeight::MEDIUM)
            .text_color(rgb(palette.text))
            .child(self.title.clone())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoardColumnKind {
    Open,
    Doing,
    WaitingBlocked,
    Done,
}

impl BoardColumnKind {
    pub const ALL: [Self; 4] = [Self::Open, Self::Doing, Self::WaitingBlocked, Self::Done];

    pub fn label(self) -> &'static str {
        match self {
            Self::Open => "Open",
            Self::Doing => "In Progress",
            Self::WaitingBlocked => "Waiting / Blocked",
            Self::Done => "Done",
        }
    }

    pub fn matches(self, status: &str, done: bool) -> bool {
        match self {
            Self::Open => !done && (status == "open" || status.is_empty()),
            Self::Doing => !done && status == "doing",
            Self::WaitingBlocked => !done && (status == "waiting" || status == "blocked"),
            Self::Done => done || status == "done" || status == "cancelled",
        }
    }

    pub fn color(self, palette: Palette) -> u32 {
        match self {
            Self::Open => palette.muted,
            Self::Doing => palette.accent,
            Self::WaitingBlocked => palette.amber,
            Self::Done => palette.green,
        }
    }

    pub fn next_status(self) -> Option<&'static str> {
        match self {
            Self::Open => Some("doing"),
            Self::Doing => Some("done"),
            Self::WaitingBlocked => Some("doing"),
            Self::Done => Some("open"),
        }
    }

    pub fn advance_label(self) -> &'static str {
        match self {
            Self::Open => "Start →",
            Self::Doing => "Done ✓",
            Self::WaitingBlocked => "Resume →",
            Self::Done => "Reopen ↺",
        }
    }
}

impl Projects {
    pub(super) fn task_board(
        &self,
        records: &[ProjectRecord],
        new_task: &Entity<InputState>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let _cols = self.columns();
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let tasks: Vec<&ProjectRecord> = records.iter().filter(|r| r.kind == "task").collect();

        let columns = BoardColumnKind::ALL.into_iter().map(|col_kind| {
            let col_color = col_kind.color(palette);
            let matching: Vec<&ProjectRecord> = tasks
                .iter()
                .copied()
                .filter(|t| {
                    let status = parse_frontmatter_status(&t.content, t.done);
                    col_kind.matches(&status, t.done)
                })
                .collect();
            let count = matching.len();

            let cards: Vec<AnyElement> = matching
                .into_iter()
                .enumerate()
                .map(|(idx, task)| {
                    let task_rec = (*task).clone();
                    let path = task.path.clone();
                    let task_pos = tasks.iter().position(|t| t.path == task.path);
                    let is_keyboard_selected =
                        self.selected_task_index.is_some() && self.selected_task_index == task_pos;
                    let is_panel_selected = self
                        .task_panel
                        .as_ref()
                        .is_some_and(|p| p.path == task.path);
                    let is_selected = is_keyboard_selected || is_panel_selected;
                    let due = task.due.clone().filter(|d| !d.is_empty());
                    let subtask_progress = crate::tasks::parse_subtask_progress(&task.content);
                    let status = parse_frontmatter_status(&task.content, task.done);
                    let next_status = col_kind.next_status();

                    div()
                        .id(SharedString::from(format!(
                            "board-card-{idx}-{}",
                            task.path
                        )))
                        .group("board-card")
                        .p_3()
                        .rounded(px(8.))
                        .bg(rgba(palette.surface))
                        .border_1()
                        .border_color(if is_selected {
                            rgb(palette.accent)
                        } else {
                            rgba(palette.border)
                        })
                        .when(is_selected, |s| s.bg(rgba(tint(palette.accent, 0x14))))
                        .hover(move |s| {
                            if !is_selected {
                                s.border_color(rgba(tint(palette.accent, 0x60)))
                            } else {
                                s
                            }
                        })
                        .cursor_pointer()
                        .on_click(cx.listener({
                            let t = task_rec.clone();
                            move |view, _, window, cx| {
                                view.selected_task_index = task_pos;
                                let global = project_record_to_global_task(&t);
                                view.select_task(&global, window, cx);
                            }
                        }))
                        .on_drag(
                            DraggedTask {
                                path: task.path.clone(),
                                title: task.title.clone(),
                                palette,
                            },
                            |dragged, _, _, cx| cx.new(|_| dragged.clone()),
                        )
                        .child(
                            div()
                                .flex()
                                .items_start()
                                .justify_between()
                                .gap_2()
                                .mb_1p5()
                                .child(
                                    div()
                                        .flex_1()
                                        .text_sm()
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(rgb(palette.text))
                                        .when(task.done, |el| {
                                            el.line_through().text_color(rgb(palette.muted))
                                        })
                                        .child(task.title.clone()),
                                )
                                .when_some(next_status, |el, target_status| {
                                    el.child(
                                        div()
                                            .id(SharedString::from(format!(
                                                "adv-{}-{}",
                                                task.path, target_status
                                            )))
                                            .px_1p5()
                                            .py_0p5()
                                            .rounded(px(4.))
                                            .bg(rgba(tint(col_color, 0x18)))
                                            .text_color(rgb(col_color))
                                            .text_xs()
                                            .font_weight(FontWeight::MEDIUM)
                                            .cursor_pointer()
                                            .hover(move |s| s.bg(rgba(tint(col_color, 0x30))))
                                            .opacity(0.)
                                            .group_hover("board-card", |s| s.opacity(1.))
                                            .on_click(cx.listener({
                                                let p = path.clone();
                                                move |view, _, window, cx| {
                                                    cx.stop_propagation();
                                                    view.set_task_property(
                                                        p.clone(),
                                                        "status",
                                                        target_status.to_string(),
                                                        window,
                                                        cx,
                                                    );
                                                }
                                            }))
                                            .child(col_kind.advance_label()),
                                    )
                                }),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(status_pill(&status, palette))
                                .when(task.priority > 1, |el| {
                                    el.child(priority_pill(task.priority, palette))
                                })
                                .when_some(due, |el, d| {
                                    el.child(due_badge(Some(&d), task.done, &today, palette))
                                })
                                .when_some(subtask_progress, |el, (done, total)| {
                                    el.child(
                                        div()
                                            .px_1p5()
                                            .py_0p5()
                                            .rounded(px(4.))
                                            .bg(rgba(if done == total {
                                                tint(palette.green, 0x22)
                                            } else {
                                                palette.raised
                                            }))
                                            .text_xs()
                                            .text_color(rgb(if done == total {
                                                palette.green
                                            } else {
                                                palette.muted
                                            }))
                                            .child(format!("{done}/{total}")),
                                    )
                                }),
                        )
                        .into_any_element()
                })
                .collect();

            let drop_status = match col_kind {
                BoardColumnKind::Open => "open",
                BoardColumnKind::Doing => "doing",
                BoardColumnKind::WaitingBlocked => "waiting",
                BoardColumnKind::Done => "done",
            };

            div()
                .flex_1()
                .min_w(px(220.))
                .flex()
                .flex_col()
                .gap_2()
                .p_2()
                .rounded(px(10.))
                .bg(rgba(tint(palette.surface, 0x55)))
                .border_1()
                .border_color(rgba(palette.border))
                .drag_over::<DraggedTask>(move |style, _, _, _| {
                    style
                        .bg(rgba(tint(col_color, 0x18)))
                        .border_color(rgb(col_color))
                })
                .on_drop(cx.listener(move |view, dragged: &DraggedTask, window, cx| {
                    view.set_task_property(
                        dragged.path.clone(),
                        "status",
                        drop_status.to_string(),
                        window,
                        cx,
                    );
                }))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .px_1()
                        .py_0p5()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(div().w(px(8.)).h(px(8.)).rounded_full().bg(rgb(col_color)))
                                .child(
                                    div()
                                        .text_xs()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(rgb(palette.text))
                                        .child(col_kind.label()),
                                ),
                        )
                        .child(
                            div()
                                .px_1p5()
                                .py_0p5()
                                .rounded_full()
                                .bg(rgba(palette.surface))
                                .text_xs()
                                .font_family(".AppleSystemUIFontMonospaced")
                                .text_color(rgb(palette.muted))
                                .child(count.to_string()),
                        ),
                )
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .overflow_hidden()
                        .children(cards)
                        .when(col_kind == BoardColumnKind::Open, |col| {
                            col.child(
                                div()
                                    .p_2()
                                    .rounded(px(6.))
                                    .border_1()
                                    .border_dashed()
                                    .border_color(rgba(palette.border))
                                    .bg(rgba(palette.surface))
                                    .child(Input::new(new_task).small().appearance(false)),
                            )
                        }),
                )
        });

        div()
            .flex()
            .gap_3()
            .size_full()
            .min_h(px(400.))
            .overflow_hidden()
            .children(columns)
            .into_any_element()
    }
}
