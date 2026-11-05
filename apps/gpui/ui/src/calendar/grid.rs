//! Month and Week grid layout views for CalendarPage.
use chrono::{Datelike, Local, NaiveDate, Timelike};
use gpui::{div, prelude::*, px, rgb, rgba, AnyElement, Context, FontWeight, SharedString};
use sjel_gpui_core::calendar::{entries_on, time_label};
use sjel_gpui_core::shell::CalendarEntry;

use crate::theme::tint;
use crate::Palette;

use super::page::CalendarPage;
use super::types::chip;
use gpui_kit::component::scroll::ScrollableElement;

#[derive(Clone)]
pub(super) struct CalendarDrag {
    pub id: String,
    pub resize: bool,
}

impl Render for CalendarDrag {
    fn render(&mut self, _: &mut gpui::Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .p_2()
            .bg(rgb(0x334155))
            .text_color(rgb(0xffffff))
            .child(if self.resize {
                "Drop to set end time"
            } else {
                "Drop to move · 15 min"
            })
    }
}

impl CalendarPage {
    fn span_strip(
        &self,
        dates: &[NaiveDate],
        entries: &[CalendarEntry],
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let spans = sjel_gpui_core::calendar::timeline::spans(entries, dates);
        let lanes = spans.iter().map(|span| span.lane + 1).max().unwrap_or(0);
        let mut strip = div().relative().h(px((lanes as f32 * 23.).max(8.)));
        for span in spans {
            let entry = &entries[span.entry_index];
            let id = entry.id.clone();
            let day = dates[span.start];
            strip = strip.child(
                div()
                    .id(SharedString::from(format!("span-{day}-{id}")))
                    .absolute()
                    .top(px(span.lane as f32 * 23.))
                    .left(gpui::relative(span.start as f32 / dates.len() as f32))
                    .w(gpui::relative(
                        (span.end - span.start) as f32 / dates.len() as f32,
                    ))
                    .h(px(22.))
                    .overflow_hidden()
                    .cursor_pointer()
                    .child(chip(entry, palette))
                    .on_click(cx.listener(move |page, _, _, cx| {
                        page.select(day, cx);
                        page.selected_entry = Some(id.clone());
                        cx.notify();
                    })),
            );
        }
        strip.into_any_element()
    }

    pub(super) fn time_grid(
        &self,
        dates: &[NaiveDate],
        entries: &[CalendarEntry],
        today: NaiveDate,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let hours = 48.0;
        let headings = div().flex().ml(px(48.)).children(dates.iter().map(|&day| {
            div()
                .flex_1()
                .min_w_0()
                .p_2()
                .text_sm()
                .text_color(rgb(if day == today {
                    palette.accent
                } else {
                    palette.text
                }))
                .child(day.format("%a %-d").to_string())
        }));
        let all_day = div()
            .ml(px(48.))
            .child(self.span_strip(dates, entries, palette, cx));
        let rail = div().w(px(48.)).flex_none().children((0..24).map(|hour| {
            div()
                .h(px(hours))
                .text_xs()
                .text_color(rgb(palette.muted))
                .child(format!("{hour:02}:00"))
        }));
        let columns = dates.iter().map(|&day| {
            let mut column = div()
                .id(SharedString::from(format!("calendar-time-column-{day}")))
                .relative()
                .flex_1()
                .min_w_0()
                .h(px(hours * 24.))
                .on_drag_move(cx.listener(
                    move |page, event: &gpui::DragMoveEvent<CalendarDrag>, _, cx| {
                        let y = f32::from(event.event.position.y - event.bounds.origin.y);
                        let minute = ((y / hours * 60.).clamp(0., 1439.) as u16) / 15 * 15;
                        page.drop_time = Some((day, minute));
                        cx.notify();
                    },
                ))
                .on_drop(cx.listener(move |page, drag: &CalendarDrag, _, cx| {
                    if let Some((day, minute)) = page.drop_time.take() {
                        page.change_time(&drag.id, day, minute, drag.resize, cx);
                    }
                }));
            for quarter in 0..96 {
                let minute = quarter * 15;
                column = column.child(
                    div()
                        .id(SharedString::from(format!("slot-{day}-{minute}")))
                        .absolute()
                        .top(px(minute as f32 / 60. * hours))
                        .left_0()
                        .right_0()
                        .h(px(hours / 4.))
                        .when(quarter % 4 == 0, |slot| slot.border_t_1())
                        .border_color(rgba(palette.border))
                        .cursor_pointer()
                        .on_click(cx.listener(move |page, _, window, cx| {
                            page.select(day, cx);
                            page.open_form_at(minute, window, cx);
                        }))
                        .drag_over::<CalendarDrag>(move |style, _, _, _| {
                            style.bg(rgba(tint(palette.accent, 0x44)))
                        }),
                );
            }
            if self.show_availability
                && self.entries.error.is_none()
                && self.loaded.is_some()
                && !self.entries.loading
            {
                for gap in sjel_gpui_core::calendar::timeline::gaps(
                    entries,
                    day,
                    sjel_gpui_core::calendar::timeline::Interval {
                        start: 480,
                        end: 1200,
                    },
                ) {
                    let minute = u32::from(gap.start);
                    column = column.child(
                        div()
                            .id(SharedString::from(format!("gap-{day}-{minute}")))
                            .absolute()
                            .top(px(gap.start as f32 / 60. * hours))
                            .left_0()
                            .right_0()
                            .h(px((gap.end - gap.start) as f32 / 60. * hours))
                            .overflow_hidden()
                            .bg(rgba(tint(palette.green, 0x18)))
                            .text_xs()
                            .text_color(rgb(palette.green))
                            .child(format!("Free · {} min", gap.end - gap.start))
                            .cursor_pointer()
                            .on_click(cx.listener(move |page, _, window, cx| {
                                page.select(day, cx);
                                page.open_form_at(minute, window, cx);
                                if let Some(form) = &page.form {
                                    let end = u32::from(gap.end).min(minute + 60);
                                    let end = format!("{:02}:{:02}", end / 60, end % 60);
                                    form.end
                                        .update(cx, |input, cx| input.set_value(end, window, cx));
                                }
                            })),
                    );
                }
            }
            for block in sjel_gpui_core::calendar::timeline::blocks(entries, day) {
                let entry = &entries[block.entry_index];
                let id = entry.id.clone();
                let color = sjel_gpui_core::calendar::kind_color(&entry.kind);
                column = column.child(
                    div()
                        .id(SharedString::from(format!("block-{day}-{}", entry.id)))
                        .debug_selector(|| "calendar-time-block".into())
                        .absolute()
                        .top(px(block.start as f32 / 60. * hours))
                        .left(gpui::relative(block.column as f32 / block.columns as f32))
                        .w(gpui::relative(1. / block.columns as f32))
                        .h(px((block.end - block.start) as f32 / 60. * hours))
                        .overflow_hidden()
                        .px_1()
                        .border_l_2()
                        .border_color(rgb(color))
                        .bg(rgba(tint(
                            color,
                            if entry.commitment == "possible" {
                                0x14
                            } else {
                                0x35
                            },
                        )))
                        .text_xs()
                        .text_color(rgb(palette.text))
                        .cursor_pointer()
                        .on_drag(
                            CalendarDrag {
                                id: entry.id.clone(),
                                resize: false,
                            },
                            |drag, _, _, cx| cx.new(|_| drag.clone()),
                        )
                        .child(entry.title.clone())
                        .child(
                            div()
                                .text_color(rgb(palette.muted))
                                .child(time_label(entry, day)),
                        )
                        .child(
                            div()
                                .id(SharedString::from(format!("resize-{day}-{}", entry.id)))
                                .absolute()
                                .bottom_0()
                                .left_0()
                                .right_0()
                                .h(px(8.))
                                .cursor_n_resize()
                                .on_drag(
                                    CalendarDrag {
                                        id: entry.id.clone(),
                                        resize: true,
                                    },
                                    |drag, _, _, cx| {
                                        cx.stop_propagation();
                                        cx.new(|_| drag.clone())
                                    },
                                ),
                        )
                        .on_click(cx.listener(move |page, _, _, cx| {
                            page.select(day, cx);
                            page.selected_entry = Some(id.clone());
                            cx.notify();
                        })),
                );
            }
            if day == today {
                let now = Local::now().time();
                let minute = now.hour() * 60 + now.minute();
                column = column.child(
                    div()
                        .absolute()
                        .top(px(minute as f32 / 60. * hours))
                        .left_0()
                        .right_0()
                        .h(px(2.))
                        .bg(rgb(palette.red)),
                );
            }
            column
        });
        div()
            .flex()
            .flex_col()
            .child(headings)
            .child(all_day)
            .child(
                div()
                    .id("calendar-time-scroll")
                    .debug_selector(|| "calendar-time-scroll".into())
                    .track_scroll(&self.timeline_scroll)
                    .overflow_y_scroll()
                    .vertical_scrollbar(&self.timeline_scroll)
                    .flex_1()
                    .min_h_0()
                    .child(div().flex().child(rail).children(columns)),
            )
            .size_full()
            .into_any_element()
    }

    pub(super) fn month(
        &self,
        dates: &[NaiveDate],
        entries: &[CalendarEntry],
        today: NaiveDate,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let weekdays = div().flex().gap_1().mb_1().children(
            ["MON", "TUE", "WED", "THU", "FRI", "SAT", "SUN"].map(|d| {
                div()
                    .flex_1()
                    .px_2()
                    .py_1()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(palette.muted))
                    .child(d)
            }),
        );
        let month = self.day.month();
        let rows = dates.chunks(7).map(|week| {
            let cells = div().flex().gap_1().children(week.iter().map(|&day| {
                let on: Vec<_> = entries_on(entries, day)
                    .into_iter()
                    .filter(|entry| !entry.all_day)
                    .collect();
                let more = on.len().saturating_sub(3);
                let selected = day == self.day;
                let is_today = day == today;
                let in_current_month = day.month() == month;
                div()
                    .id(SharedString::from(format!("calendar-day-{day}")))
                    .flex_1()
                    .min_w_0()
                    .min_h(px(104.))
                    .p(px(6.))
                    .rounded(px(6.))
                    .border_1()
                    .border_color(if selected {
                        rgb(palette.accent)
                    } else {
                        rgba(palette.border)
                    })
                    .bg(if selected {
                        rgba(palette.raised)
                    } else if !in_current_month {
                        rgba(tint(palette.surface, 0x44))
                    } else {
                        rgba(palette.surface)
                    })
                    .cursor_pointer()
                    .on_click(cx.listener(move |page, _, _, cx| page.select(day, cx)))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .mb_1()
                            .child(
                                div()
                                    .w(px(22.))
                                    .h(px(22.))
                                    .rounded_full()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .when(is_today, |el| el.bg(rgb(palette.accent)))
                                    .text_xs()
                                    .font_weight(if is_today {
                                        FontWeight::BOLD
                                    } else if in_current_month {
                                        FontWeight::MEDIUM
                                    } else {
                                        FontWeight::NORMAL
                                    })
                                    .text_color(rgb(if is_today {
                                        0xffffff
                                    } else if in_current_month {
                                        palette.text
                                    } else {
                                        palette.muted
                                    }))
                                    .child(day.day().to_string()),
                            )
                            .when(day.day() == 1, |el| {
                                el.child(
                                    div()
                                        .text_xs()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(rgb(if in_current_month {
                                            palette.text
                                        } else {
                                            palette.muted
                                        }))
                                        .child(day.format("%b").to_string()),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .children(on.into_iter().take(3).map(|e| chip(e, palette))),
                    )
                    .when(more > 0, |el| {
                        el.child(
                            div()
                                .id(SharedString::from(format!("calendar-overflow-{day}")))
                                .pt(px(2.))
                                .text_xs()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(rgb(palette.muted))
                                .child(format!("+{more} more"))
                                .on_click(cx.listener(move |page, _, _, cx| {
                                    cx.stop_propagation();
                                    page.select(day, cx);
                                    page.set_view(super::types::View::Day, cx);
                                })),
                        )
                    })
            }));
            div()
                .flex()
                .flex_col()
                .child(cells)
                .child(self.span_strip(week, entries, palette, cx))
        });
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(weekdays)
            .children(rows)
            .into_any_element()
    }
}
