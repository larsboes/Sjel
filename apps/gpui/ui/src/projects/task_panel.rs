//! The side panel of the cross-project task table: one task's status, priority, due date,
//! context and blockers, each written through the capability as it changes.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::Sizable;
use gpui_kit::{
    div, prelude::*, px, rgb, rgba, AnyElement, Context, Entity, FontWeight, SharedString,
    Subscription, Window,
};

use crate::primitives::{due_badge, priority_pill};
use crate::theme::{tint, Palette};
use sjel_gpui_core::shell::GlobalTask;

use super::{Projects, ProjectsTab};

/// Frontmatter keys the panel edits or derives, so the "Other fields" list leaves them out.
const SHOWN_KEYS: [&str; 9] = [
    "type",
    "done",
    "status",
    "priority",
    "due",
    "context",
    "blocked_by",
    "summary",
    "projects",
];

/// The text fields a task's panel edits on Enter or on leaving the field.
const TEXT_KEYS: [(&str, &str); 2] = [("context", "Context"), ("blocked_by", "Blocked by")];

/// The task the panel shows, and one input per [`TEXT_KEYS`] entry.
pub struct TaskPanel {
    pub(in crate::projects) path: String,
    inputs: Vec<(&'static str, Entity<InputState>)>,
    _subscriptions: Vec<Subscription>,
}

impl TaskPanel {
    /// The field that edits `key`, if the panel has one.
    #[cfg(test)]
    pub(in crate::projects) fn input(&self, key: &str) -> Option<&Entity<InputState>> {
        self.inputs
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, input)| input)
    }
}

impl Projects {
    /// Opens the panel on `task`, or closes it when it already shows that task.
    pub(super) fn select_task(
        &mut self,
        task: &GlobalTask,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.task_panel.as_ref().is_some_and(|p| p.path == task.id) {
            self.task_panel = None;
            cx.notify();
            return;
        }
        let mut inputs = Vec::new();
        let mut subscriptions = Vec::new();
        for (key, label) in TEXT_KEYS {
            let value = task.fields.get(key).cloned().unwrap_or_default();
            let input = cx.new(|cx| {
                let mut state = InputState::new(window, cx).placeholder(label);
                state.set_value(value.clone(), window, cx);
                state
            });
            let path = task.id.clone();
            subscriptions.push(cx.subscribe_in(
                &input,
                window,
                move |view, input, event, window, cx| {
                    if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                        let typed = input.read(cx).value().trim().to_owned();
                        if typed != value {
                            view.set_task_property(path.clone(), key, typed, window, cx);
                        }
                    }
                },
            ));
            inputs.push((key, input));
        }
        self.task_panel = Some(TaskPanel {
            path: task.id.clone(),
            inputs,
            _subscriptions: subscriptions,
        });
        cx.notify();
    }

    pub(super) fn task_panel_view(
        &self,
        panel: &TaskPanel,
        task: &GlobalTask,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let label = |text: &'static str| {
            div()
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(palette.muted))
                .child(text)
        };
        let field = |name: &'static str, body: AnyElement| {
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(label(name))
                .child(body)
        };

        let priorities = (1..=3u8)
            .map(|priority| {
                let path = task.id.clone();
                let active = task.priority.clamp(1, 3) == priority;
                div()
                    .id(SharedString::from(format!("panel-priority-{priority}")))
                    .cursor_pointer()
                    .when(!active, |chip| chip.opacity(0.5).hover(|s| s.opacity(1.)))
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.set_task_property(
                            path.clone(),
                            "priority",
                            priority.to_string(),
                            window,
                            cx,
                        )
                    }))
                    .child(priority_pill(priority, palette))
            })
            .collect::<Vec<_>>();

        let due = match self
            .due_edit
            .as_ref()
            .filter(|(path, _, _)| *path == task.id)
        {
            Some((_, input, _)) => {
                self.due_editor(task.id.clone(), input, usize::MAX, 200., palette, cx)
            }
            None => {
                let (path, current) = (task.id.clone(), task.due.clone());
                div()
                    .id("panel-due")
                    .cursor_pointer()
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.edit_due(path.clone(), current.clone(), window, cx)
                    }))
                    .child(due_badge(task.due.as_deref(), task.done, &today, palette))
                    .into_any_element()
            }
        };

        let texts = panel.inputs.iter().map(|(key, input)| {
            let name = TEXT_KEYS
                .iter()
                .find(|(k, _)| k == key)
                .map_or(*key, |(_, label)| *label);
            field(name, Input::new(input).small().into_any_element())
        });

        let others: Vec<_> = task
            .fields
            .iter()
            .filter(|(key, value)| !SHOWN_KEYS.contains(&key.as_str()) && !value.is_empty())
            .map(|(key, value)| {
                div()
                    .flex()
                    .gap_2()
                    .text_xs()
                    .child(
                        div()
                            .w(px(96.))
                            .text_color(rgb(palette.muted))
                            .child(key.clone()),
                    )
                    .child(div().flex_1().min_w_0().truncate().child(value.clone()))
            })
            .collect();

        let (project, path) = (task.project.clone(), task.id.clone());
        div()
            .w(px(340.))
            .flex_none()
            .flex()
            .flex_col()
            .gap_4()
            .p_4()
            .rounded(px(8.))
            .border_1()
            .border_color(rgba(palette.border))
            .bg(rgba(tint(palette.raised, 0x40)))
            .child(
                div()
                    .flex()
                    .items_start()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(task.title.clone()),
                    )
                    .child(
                        div()
                            .id("panel-close")
                            .cursor_pointer()
                            .text_color(rgb(palette.muted))
                            .hover(move |s| s.text_color(rgb(palette.text)))
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.task_panel = None;
                                cx.notify();
                            }))
                            .child("×"),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.accent))
                    .child(task.project.clone()),
            )
            .child(field(
                "Status",
                self.status_chips(&task.id, task.status(), palette, cx),
            ))
            .child(field(
                "Priority",
                div().flex().gap_1().children(priorities).into_any_element(),
            ))
            .child(field("Due", due))
            .children(texts)
            .children(
                task.summary
                    .clone()
                    .filter(|s| !s.is_empty())
                    .map(|summary| {
                        field("Summary", div().text_sm().child(summary).into_any_element())
                    }),
            )
            .when(!others.is_empty(), |panel| {
                panel.child(field(
                    "Other fields",
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .children(others)
                        .into_any_element(),
                ))
            })
            .child(
                Button::new("panel-open-note")
                    .small()
                    .ghost()
                    .label("Open note")
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.task_panel = None;
                        view.tab = ProjectsTab::Hubs;
                        view.open_record(project.clone(), path.clone(), window, cx);
                    })),
            )
            .into_any_element()
    }
}
