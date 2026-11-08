//! Context cards, day navigation header, and filter summary action bars.

use chrono::{Duration, Local, NaiveDate};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::Sizable;
use gpui_kit::{div, prelude::*, px, rgb, rgba, AnyElement, Context, FontWeight};
use sjel_gpui_core::context::{
    format_day_header, format_money, needs_attention, ContextDay, TimelineKind,
};
use sjel_gpui_core::shell::GlobalTask;

use super::types::ContextEvent;
use super::ContextPage;
use crate::primitives::{stat_card_interactive, stat_strip, InteractiveStatCard};
use crate::theme::Palette;

pub fn render_day_header(
    day: NaiveDate,
    is_today: bool,
    palette: Palette,
    cx: &mut Context<ContextPage>,
) -> AnyElement {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap_4()
        .child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(
                    div()
                        .text_lg()
                        .font_weight(FontWeight::BOLD)
                        .text_color(rgb(palette.text))
                        .child(format_day_header(day)),
                )
                .when(is_today, |el| {
                    el.child(
                        div()
                            .px_2()
                            .py(px(2.))
                            .rounded_full()
                            .bg(rgba(crate::theme::tint(palette.accent, 0x22)))
                            .border_1()
                            .border_color(rgba(crate::theme::tint(palette.accent, 0x44)))
                            .text_xs()
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgb(palette.accent))
                            .child("TODAY"),
                    )
                }),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_1()
                .child(
                    Button::new("context-prev")
                        .small()
                        .ghost()
                        .label("‹")
                        .tooltip("Previous day ([)")
                        .on_click(cx.listener(move |page, _, _, cx| {
                            page.select(page.day - Duration::days(1), cx)
                        })),
                )
                .child(
                    Button::new("context-today")
                        .small()
                        .ghost()
                        .label("Today")
                        .tooltip("Jump to today (t)")
                        .on_click(
                            cx.listener(|page, _, _, cx| {
                                page.select(Local::now().date_naive(), cx)
                            }),
                        ),
                )
                .child(
                    Button::new("context-next")
                        .small()
                        .ghost()
                        .label("›")
                        .tooltip("Next day (])")
                        .on_click(cx.listener(move |page, _, _, cx| {
                            page.select(page.day + Duration::days(1), cx)
                        })),
                )
                .child(
                    Button::new("context-refresh")
                        .small()
                        .ghost()
                        .label("↻")
                        .tooltip("Refresh (⌘R)")
                        .on_click(cx.listener(|page, _, _, cx| page.refresh(cx))),
                ),
        )
        .into_any_element()
}

pub fn render_metric_cards(
    context: &ContextDay,
    tasks: &[GlobalTask],
    is_today: bool,
    filter_kind: Option<TimelineKind>,
    palette: Palette,
    cx: &mut Context<ContextPage>,
) -> AnyElement {
    let event_count = context.events.len();
    let today_str = Local::now().format("%Y-%m-%d").to_string();
    let attention = needs_attention(tasks, &today_str);
    let mut open_ids: Vec<&str> = context
        .tasks
        .iter()
        .filter(|task| !task.done)
        .map(|task| task.id.as_str())
        .collect();
    if is_today {
        for task in &attention {
            if !open_ids.contains(&task.id.as_str()) {
                open_ids.push(task.id.as_str());
            }
        }
    }
    let open_task_count = open_ids.len();
    let done_task_count = context.tasks.iter().filter(|t| t.done).count();
    let total_spend_str = format_money(context.total_spent_cents, &context.currency);

    stat_strip(vec![
        stat_card_interactive(
            InteractiveStatCard {
                id: "context-stat-events".into(),
                label: "EVENTS".into(),
                value: format!("{event_count} scheduled").into(),
                detail: Some(if event_count == 0 {
                    "Calendar clear".into()
                } else {
                    "Click to filter · Meetings & blocks".into()
                }),
                tone: palette.text,
                active: filter_kind == Some(TimelineKind::Event),
                palette,
            },
            cx.listener(|page, _, _, cx| page.toggle_filter(TimelineKind::Event, cx)),
        ),
        stat_card_interactive(
            InteractiveStatCard {
                id: "context-stat-tasks".into(),
                label: "TASKS".into(),
                value: format!("{open_task_count} open").into(),
                detail: Some(if is_today && !attention.is_empty() {
                    format!("{} urgent / due today", attention.len()).into()
                } else if done_task_count > 0 {
                    format!("{done_task_count} completed").into()
                } else {
                    "Due today".into()
                }),
                tone: palette.text,
                active: filter_kind == Some(TimelineKind::Task),
                palette,
            },
            cx.listener(|page, _, _, cx| page.toggle_filter(TimelineKind::Task, cx)),
        ),
        stat_card_interactive(
            InteractiveStatCard {
                id: "context-stat-spend".into(),
                label: "SPENDING".into(),
                value: total_spend_str.into(),
                detail: Some(if context.spend.is_empty() {
                    "No transactions".into()
                } else {
                    format!("{} transactions", context.spend.len()).into()
                }),
                tone: palette.text,
                active: filter_kind == Some(TimelineKind::Spend),
                palette,
            },
            cx.listener(|page, _, _, cx| page.toggle_filter(TimelineKind::Spend, cx)),
        ),
    ])
}

pub fn render_filter_action(
    filter_kind: Option<TimelineKind>,
    day: NaiveDate,
    cx: &mut Context<ContextPage>,
) -> Option<AnyElement> {
    match filter_kind {
        Some(TimelineKind::Event) => Some(
            Button::new("context-open-calendar")
                .xsmall()
                .ghost()
                .label("Open Calendar →")
                .on_click(cx.listener(move |_, _, _, cx| {
                    cx.emit(ContextEvent::OpenCalendar {
                        day,
                        entry_id: None,
                    });
                }))
                .into_any_element(),
        ),
        Some(TimelineKind::Task) => Some(
            Button::new("context-open-projects")
                .xsmall()
                .ghost()
                .label("Open Projects →")
                .on_click(cx.listener(|_, _, _, cx| {
                    cx.emit(ContextEvent::OpenProject(String::new()));
                }))
                .into_any_element(),
        ),
        Some(TimelineKind::Spend) => Some(
            Button::new("context-open-finance")
                .xsmall()
                .ghost()
                .label("Open Finance →")
                .on_click(cx.listener(move |_, _, _, cx| {
                    cx.emit(ContextEvent::OpenFinance { day });
                }))
                .into_any_element(),
        ),
        None => None,
    }
}
