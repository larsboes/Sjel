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
        today: NaiveDate,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let spans = sjel_gpui_core::calendar::timeline::spans(entries, dates);
        let lanes = spans.iter().map(|span| span.lane + 1).max().unwrap_or(0);
        let bg = div()
            .absolute()
            .inset_0()
            .flex()
            .children(dates.iter().enumerate().map(|(idx, &day)| {
                let is_today = day == today;
                div()
                    .flex_1()
                    .min_w_0()
                    .when(idx > 0, |el| el.border_l_1().border_color(rgba(palette.border)))
                    .when(is_today, |el| el.bg(rgba(tint(palette.accent, 0x06))))
            }));
        let mut strip = div().relative().h(px((lanes as f32 * 24.).max(8.))).child(bg);
        for span in spans {
            let entry = &entries[span.entry_index];
            let id = entry.id.clone();
            let day = dates[span.start];
            strip = strip.child(
                div()
                    .id(SharedString::from(format!("span-{day}-{id}")))
                    .absolute()
                    .top(px(span.lane as f32 * 24. + 1.))
                    .left(gpui::relative(span.start as f32 / dates.len() as f32))
                    .w(gpui::relative(
                        (span.end - span.start) as f32 / dates.len() as f32,
                    ))
                    .px_0p5()
                    .h(px(22.))
                    .overflow_hidden()
                    .cursor_pointer()
                    .child(chip(entry, palette))
                    .on_click(cx.listener(move |page, _, _, cx| {
                        page.select(day, cx);
                        page.select_entry(id.clone(), cx);
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
        let headings = div()
            .flex()
            .border_b_1()
            .border_color(rgba(palette.border))
            .child(
                div()
                    .w(px(48.))
                    .flex_none()
                    .border_r_1()
                    .border_color(rgba(palette.border)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .children(dates.iter().enumerate().map(|(idx, &day)| {
                        let is_today = day == today;
                        let is_selected = day == self.day;
                        div()
                            .id(SharedString::from(format!("calendar-heading-{day}")))
                            .flex_1()
                            .min_w_0()
                            .py_2()
                            .px_1()
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap_1()
                            .cursor_pointer()
                            .when(idx > 0, |el| el.border_l_1().border_color(rgba(palette.border)))
                            .on_click(cx.listener(move |page, _, _, cx| {
                                page.select(day, cx);
                            }))
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(if is_today {
                                        palette.accent
                                    } else {
                                        palette.muted
                                    }))
                                    .child(day.format("%a").to_string().to_uppercase()),
                            )
                            .child(
                                div()
                                    .w(px(26.))
                                    .h(px(26.))
                                    .rounded_full()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .when(is_today, |el| el.bg(rgb(palette.accent)))
                                    .when(!is_today && is_selected, |el| {
                                        el.border_1().border_color(rgba(palette.border))
                                    })
                                    .text_sm()
                                    .font_weight(if is_today || is_selected {
                                        FontWeight::BOLD
                                    } else {
                                        FontWeight::MEDIUM
                                    })
                                    .text_color(rgb(if is_today {
                                        0xffffff
                                    } else {
                                        palette.text
                                    }))
                                    .child(day.day().to_string()),
                            )
                    })),
            );
        let all_day = div()
            .flex()
            .items_center()
            .py_1()
            .border_b_1()
            .border_color(rgba(palette.border))
            .child(
                div()
                    .w(px(48.))
                    .flex_none()
                    .border_r_1()
                    .border_color(rgba(palette.border))
                    .px_1()
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(palette.muted))
                    .child("all-day"),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(self.span_strip(dates, entries, today, palette, cx)),
            );
        let rail = div()
            .w(px(48.))
            .flex_none()
            .border_r_1()
            .border_color(rgba(palette.border))
            .children((0..24).map(|hour| {
                div()
                    .h(px(hours))
                    .pr_2()
                    .flex()
                    .justify_end()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child(format!("{hour:02}:00"))
            }));
        let columns = dates.iter().enumerate().map(|(idx, &day)| {
            let is_today = day == today;
            let mut column = div()
                .id(SharedString::from(format!("calendar-time-column-{day}")))
                .relative()
                .flex_1()
                .min_w_0()
                .h(px(hours * 24.))
                .when(idx > 0, |el| el.border_l_1().border_color(rgba(palette.border)))
                .when(is_today, |el| el.bg(rgba(tint(palette.accent, 0x06))))
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
                        .when(quarter % 4 == 0, |slot| {
                            slot.border_t_1().border_color(rgba(palette.border))
                        })
                        .when(quarter % 4 == 2, |slot| {
                            slot.border_t_1()
                                .border_color(rgba(tint(palette.border, 0x40)))
                        })
                        .cursor_pointer()
                        .hover(|slot| slot.bg(rgba(tint(palette.accent, 0x0c))))
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
                            .top(px(gap.start as f32 / 60. * hours + 1.))
                            .left_0()
                            .right_0()
                            .h(px((gap.end - gap.start) as f32 / 60. * hours - 2.))
                            .overflow_hidden()
                            .rounded(px(4.))
                            .border_1()
                            .border_color(rgba(tint(palette.green, 0x30)))
                            .bg(rgba(tint(palette.green, 0x14)))
                            .hover(|g| g.bg(rgba(tint(palette.green, 0x24))))
                            .px_2()
                            .py(px(1.5))
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
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
                let is_selected = self.selected_entry.as_deref() == Some(entry.id.as_str());
                column = column.child(
                    div()
                        .id(SharedString::from(format!("block-{day}-{}", entry.id)))
                        .debug_selector(|| "calendar-time-block".into())
                        .absolute()
                        .top(px(block.start as f32 / 60. * hours))
                        .left(gpui::relative(block.column as f32 / block.columns as f32))
                        .w(gpui::relative(1. / block.columns as f32))
                        .h(px(
                            ((block.end - block.start) as f32 / 60. * hours).max(18.),
                        ))
                        .overflow_hidden()
                        .rounded(px(4.))
                        .border_l_3()
                        .border_color(if is_selected {
                            rgb(palette.accent)
                        } else {
                            rgb(color)
                        })
                        .bg(rgba(tint(
                            color,
                            if entry.commitment == "possible" {
                                0x18
                            } else {
                                0x38
                            },
                        )))
                        .hover(|el| el.bg(rgba(tint(color, 0x50))))
                        .px_1p5()
                        .py(px(1.5))
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
                        .child(
                            div()
                                .font_weight(FontWeight::SEMIBOLD)
                                .truncate()
                                .child(entry.title.clone()),
                        )
                        .child(
                            div()
                                .text_color(rgb(palette.muted))
                                .text_xs()
                                .truncate()
                                .child(time_label(entry, day)),
                        )
                        .when_some(
                            entry
                                .location
                                .clone()
                                .filter(|loc| !loc.is_empty() && block.end - block.start >= 45),
                            |el, loc| {
                                el.child(
                                    div()
                                        .text_color(rgb(palette.muted))
                                        .text_xs()
                                        .truncate()
                                        .child(loc),
                                )
                            },
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
                                .hover(|grip| grip.bg(rgba(tint(color, 0x60))))
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
                            page.select_entry(id.clone(), cx);
                        })),
                );
            }
            if day == today {
                let now = Local::now().time();
                let minute = now.hour() * 60 + now.minute();
                let y = px(minute as f32 / 60. * hours);
                column = column.child(
                    div()
                        .absolute()
                        .top(y)
                        .left_0()
                        .right_0()
                        .h(px(2.))
                        .bg(rgb(palette.red))
                        .child(
                            div()
                                .absolute()
                                .top(px(-3.))
                                .left(px(-4.))
                                .w(px(8.))
                                .h(px(8.))
                                .rounded_full()
                                .bg(rgb(palette.red)),
                        ),
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
                    .hover(|el| {
                        el.bg(rgba(tint(palette.raised, 0x80)))
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
                                .mt(px(2.))
                                .px_1p5()
                                .py(px(1.))
                                .rounded(px(4.))
                                .bg(rgba(tint(palette.accent, 0x18)))
                                .hover(|b| b.bg(rgba(tint(palette.accent, 0x30))))
                                .text_xs()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(rgb(palette.accent))
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
                .child(self.span_strip(week, entries, today, palette, cx))
        });
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_1()
            .child(weekdays)
            .child(
                div()
                    .id("calendar-month-scroll")
                    .debug_selector(|| "calendar-month-scroll".into())
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.month_scroll)
                    .vertical_scrollbar(&self.month_scroll)
                    .children(rows),
            )
            .into_any_element()
    }
}
