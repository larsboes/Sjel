//! Timeline rows, kind pills, and empty quiet day view.

use chrono::NaiveDate;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::Sizable;
use gpui_kit::{
    div, prelude::*, px, rgb, rgba, AnyElement, Context, FontWeight, SharedString,
};
use sjel_gpui_core::context::{format_money, TimelineItem, TimelineKind};

use super::types::ContextEvent;
use super::ContextPage;
use crate::primitives::{mono_num, tone_chip};
use crate::theme::Palette;

pub fn render_timeline_content(
    filtered_timeline: Vec<TimelineItem>,
    filter_kind: Option<TimelineKind>,
    day: NaiveDate,
    palette: Palette,
    cx: &mut Context<ContextPage>,
) -> AnyElement {
    if filtered_timeline.is_empty() {
        div()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .py_6()
            .px_4()
            .gap_1()
            .rounded(px(10.))
            .border_1()
            .border_color(rgba(palette.border))
            .bg(rgba(palette.surface))
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(palette.muted))
                    .child(match filter_kind {
                        Some(TimelineKind::Event) => "No events on this day",
                        Some(TimelineKind::Task) => "No due tasks on this day",
                        Some(TimelineKind::Spend) => "No transactions on this day",
                        None => "Quiet day",
                    }),
            )
            .into_any_element()
    } else {
        let mut list = div().flex().flex_col().gap_1p5();
        for item in filtered_timeline {
            list = list.child(render_timeline_row(item, day, palette, cx));
        }
        list.into_any_element()
    }
}

pub fn render_timeline_row(
    item: TimelineItem,
    day: NaiveDate,
    palette: Palette,
    cx: &mut Context<ContextPage>,
) -> AnyElement {
    let (kind_color, kind_name) = match item.kind {
        TimelineKind::Event => (palette.accent, "EVENT"),
        TimelineKind::Task => (palette.amber, "TASK"),
        TimelineKind::Spend => (palette.green, "SPEND"),
    };

    let time_pill = match &item.time {
        Some(t) => div()
            .w(px(56.))
            .text_xs()
            .font_weight(FontWeight::SEMIBOLD)
            .child(mono_num(t.clone(), palette.text)),
        None => div()
            .w(px(56.))
            .text_xs()
            .font_weight(FontWeight::MEDIUM)
            .text_color(rgb(palette.muted))
            .child("ALL DAY"),
    };

    let kind_pill = tone_chip(kind_name, kind_color);

    let right_element: AnyElement = match item.kind {
        TimelineKind::Task => {
            let task_id = item
                .id
                .strip_prefix("task:")
                .unwrap_or(&item.id)
                .to_string();
            let is_done = item.completed;
            Button::new(SharedString::from(format!("toggle-task-{}", item.id)))
                .xsmall()
                .ghost()
                .label(if is_done { "✓ Done" } else { "○ Mark Done" })
                .on_click(cx.listener(move |page, _, _, cx| {
                    page.toggle_task(task_id.clone(), is_done, cx);
                }))
                .into_any_element()
        }
        TimelineKind::Spend => {
            if let Some(cents) = item.amount_cents {
                let curr = item.currency.as_deref().unwrap_or("EUR");
                let is_expense = cents < 0;
                let text = format_money(cents, curr);
                tone_chip(
                    text,
                    if is_expense {
                        palette.red
                    } else {
                        palette.green
                    },
                )
            } else {
                div().into_any_element()
            }
        }
        TimelineKind::Event => {
            if let Some(badge) = item.badge {
                div()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child(badge)
                    .into_any_element()
            } else {
                div().into_any_element()
            }
        }
    };

    let row_id = SharedString::from(format!("timeline-row-{}", item.id));
    let item_kind = item.kind;
    let raw_id = item.id.clone();
    let subtitle_project = item.subtitle.clone();

    div()
        .id(row_id)
        .flex()
        .items_center()
        .justify_between()
        .px_3()
        .py_2()
        .rounded(px(8.))
        .border_1()
        .border_color(rgba(palette.border))
        .bg(rgba(palette.surface))
        .cursor_pointer()
        .hover(move |s| s.bg(rgba(palette.raised)))
        .on_click(cx.listener(move |_, _, _, cx| match item_kind {
            TimelineKind::Event => {
                let entry_id = raw_id.strip_prefix("event:").map(str::to_owned);
                cx.emit(ContextEvent::OpenCalendar { day, entry_id });
            }
            TimelineKind::Task => {
                cx.emit(ContextEvent::OpenProject(subtitle_project.clone()));
            }
            TimelineKind::Spend => {
                cx.emit(ContextEvent::OpenFinance { day });
            }
        }))
        .child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .min_w_0()
                .child(time_pill)
                .child(kind_pill)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_0p5()
                        .min_w_0()
                        .child(
                            div()
                                .truncate()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(rgb(palette.text))
                                .child(item.title),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_xs()
                                .text_color(rgb(palette.muted))
                                .child(item.subtitle),
                        ),
                ),
        )
        .child(right_element)
        .into_any_element()
}
