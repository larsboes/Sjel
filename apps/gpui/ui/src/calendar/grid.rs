//! Month and Week grid layout views for CalendarPage.
use chrono::{Datelike, NaiveDate};
use gpui::{
    div, prelude::*, px, rgb, rgba, AnyElement, Context, FontWeight, SharedString,
};
use sjel_gpui_core::calendar::{entries_on, time_label};
use sjel_gpui_core::shell::CalendarEntry;

use crate::theme::tint;
use crate::Palette;

use super::page::CalendarPage;
use super::types::chip;

impl CalendarPage {
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
            div().flex().gap_1().children(week.iter().map(|&day| {
                let on = entries_on(entries, day);
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
                                .pt(px(2.))
                                .text_xs()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(rgb(palette.muted))
                                .child(format!("+{more} more")),
                        )
                    })
            }))
        });
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(weekdays)
            .children(rows)
            .into_any_element()
    }

    pub(super) fn week(
        &self,
        dates: &[NaiveDate],
        entries: &[CalendarEntry],
        today: NaiveDate,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .flex()
            .gap_1()
            .children(dates.iter().map(|&day| {
                let selected = day == self.day;
                let is_today = day == today;
                let on = entries_on(entries, day);
                div()
                    .id(SharedString::from(format!("calendar-col-{day}")))
                    .flex_1()
                    .min_w_0()
                    .min_h(px(360.))
                    .p_2()
                    .rounded(px(6.))
                    .border_1()
                    .border_color(if selected {
                        rgb(palette.accent)
                    } else {
                        rgba(palette.border)
                    })
                    .bg(if selected {
                        rgba(palette.raised)
                    } else {
                        rgba(palette.surface)
                    })
                    .cursor_pointer()
                    .on_click(cx.listener(move |page, _, _, cx| page.select(day, cx)))
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .pb_1()
                            .border_b_1()
                            .border_color(rgba(palette.border))
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(palette.muted))
                                    .child(day.format("%a").to_string().to_uppercase()),
                            )
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
                                    } else {
                                        FontWeight::MEDIUM
                                    })
                                    .text_color(rgb(if is_today { 0xffffff } else { palette.text }))
                                    .child(day.day().to_string()),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .flex_1()
                            .children(on.into_iter().map(|e| {
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(rgb(palette.muted))
                                            .child(time_label(e, day)),
                                    )
                                    .child(chip(e, palette))
                            })),
                    )
            }))
            .into_any_element()
    }
}
