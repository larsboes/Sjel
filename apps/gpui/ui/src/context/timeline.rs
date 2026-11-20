//! Timeline rows, kind pills, and empty quiet day view.

use std::rc::Rc;

use crate::ely::data_display::Tone;
use crate::ely::tables::{Cell, Column, DataTable, Row, RowAction};
use crate::ely::theme::Density;
use chrono::NaiveDate;
use gpui_kit::{div, prelude::*, px, rems, rgb, rgba, AnyElement, Context};
use sjel_gpui_core::context::{format_money, TimelineItem, TimelineKind};

use super::types::ContextEvent;
use super::ContextPage;
use crate::theme::Palette;

fn timeline_columns() -> [Column; 5] {
    [
        Column::new("time", "TIME").width(rems(5.5)).muted(),
        Column::new("kind", "KIND").width(rems(5.5)),
        Column::new("item", "ITEM"),
        Column::new("detail", "AMOUNT / STATUS").width(rems(8.)).end(),
        Column::new("actions", "").width(rems(8.)).actions([
            RowAction::new("done", "Mark Done").primary(),
            RowAction::new("undo", "✓ Done"),
            RowAction::new("open", "Open"),
        ]),
    ]
}

fn timeline_table_row(item: &TimelineItem) -> Row {
    let time_cell = Cell::Text(
        item.time
            .clone()
            .unwrap_or_else(|| "ALL DAY".to_owned())
            .into(),
    );
    let kind_cell = match item.kind {
        TimelineKind::Event => Cell::Tag("EVENT".into(), Tone::Accent),
        TimelineKind::Task => Cell::Tag("TASK".into(), Tone::Warning),
        TimelineKind::Spend => Cell::Tag("SPEND".into(), Tone::Success),
    };
    let item_cell = if item.subtitle.trim().is_empty() {
        Cell::Text(item.title.clone().into())
    } else {
        Cell::Lines(item.title.clone().into(), item.subtitle.clone().into())
    };
    let detail_cell = match item.kind {
        TimelineKind::Spend => {
            if let Some(cents) = item.amount_cents {
                let curr = item.currency.as_deref().unwrap_or("EUR");
                let tone = if cents < 0 {
                    Tone::Danger
                } else {
                    Tone::Success
                };
                Cell::Figure(format_money(cents, curr).into(), cents as f64, tone)
            } else {
                Cell::Empty
            }
        }
        TimelineKind::Event => item
            .badge
            .as_ref()
            .map(|b| Cell::Tag(b.clone().into(), Tone::Neutral))
            .unwrap_or(Cell::Empty),
        TimelineKind::Task => {
            if item.completed {
                Cell::Tag("Done".into(), Tone::Success)
            } else {
                Cell::Tag("Open".into(), Tone::Warning)
            }
        }
    };
    let actions = match item.kind {
        TimelineKind::Task if item.completed => {
            Cell::Actions(vec!["undo".into(), "open".into()])
        }
        TimelineKind::Task => Cell::Actions(vec!["done".into(), "open".into()]),
        _ => Cell::Actions(vec!["open".into()]),
    };
    Row::new(
        item.id.clone(),
        [time_cell, kind_cell, item_cell, detail_cell, actions],
    )
}

pub fn render_timeline_content(
    filtered_timeline: Vec<TimelineItem>,
    filter_kind: Option<TimelineKind>,
    day: NaiveDate,
    palette: Palette,
    cx: &mut Context<ContextPage>,
) -> AnyElement {
    let empty_msg = match filter_kind {
        Some(TimelineKind::Event) => "No events on this day",
        Some(TimelineKind::Task) => "No due tasks on this day",
        Some(TimelineKind::Spend) => "No transactions on this day",
        None => "Quiet day — no scheduled events, due tasks, or spending",
    };
    if filtered_timeline.is_empty() {
        return div()
            .flex()
            .items_center()
            .py_2()
            .px_3()
            .rounded(px(8.))
            .border_1()
            .border_color(rgba(palette.border))
            .bg(rgba(palette.surface))
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child(empty_msg),
            )
            .into_any_element();
    }
    let rows: Vec<Row> = filtered_timeline.iter().map(timeline_table_row).collect();
    let items_for_action = filtered_timeline.clone();
    let items_for_open = filtered_timeline;
    let entity_action = cx.entity();
    let entity_open = cx.entity();

    let open_item = move |page_entity: &gpui_kit::Entity<ContextPage>,
                          items: &[TimelineItem],
                          key: &str,
                          cx: &mut gpui_kit::App| {
        if let Some(item) = items.iter().find(|i| i.id == key) {
            let kind = item.kind;
            let raw_id = item.id.clone();
            let subtitle_project = item.subtitle.clone();
            page_entity.update(cx, |_, cx| match kind {
                TimelineKind::Event => {
                    let entry_id = raw_id.strip_prefix("event:").map(str::to_owned);
                    cx.emit(ContextEvent::OpenCalendar { day, entry_id });
                }
                TimelineKind::Task => {
                    cx.emit(ContextEvent::OpenProject(subtitle_project));
                }
                TimelineKind::Spend => {
                    cx.emit(ContextEvent::OpenFinance { day });
                }
            });
        }
    };

    DataTable::new("context-timeline-table", timeline_columns())
        .label("Daily timeline")
        .rows(Rc::new(rows))
        .density(Density::Comfortable)
        .empty(empty_msg)
        .on_open(move |key, _, cx| {
            open_item(&entity_open, &items_for_open, key.as_ref(), cx);
        })
        .on_action(move |key, action, _, cx| {
            match action.as_ref() {
                "done" | "undo" => {
                    if let Some(item) = items_for_action.iter().find(|i| i.id == key.as_ref()) {
                        let task_id = item
                            .id
                            .strip_prefix("task:")
                            .unwrap_or(&item.id)
                            .to_string();
                        let is_done = item.completed;
                        entity_action.update(cx, |page, cx| {
                            page.toggle_task(task_id, is_done, cx);
                        });
                    }
                }
                "open" => {
                    open_item(&entity_action, &items_for_action, key.as_ref(), cx);
                }
                _ => {}
            }
        })
        .into_any_element()
}
