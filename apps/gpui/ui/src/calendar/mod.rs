//! Calendar pages and event adapters for the Mac and iPhone.

pub mod actions;
mod adapter;
mod details;
mod editor;
pub mod grid;
pub mod page;
#[cfg(target_os = "ios")]
mod phone;
#[cfg(target_os = "ios")]
pub use phone::PhoneCalendar;
pub mod proposals;
pub mod types;

pub use actions::*;
pub use page::*;
pub use types::*;

use chrono::NaiveDate;
use gpui::{div, prelude::*, px, rgb, rgba, App, Div, FontWeight, SharedString, Window};
use sjel_gpui_core::{calendar, shell::CalendarEntry};

use crate::{
    ely::{
        buttons::{Button, ButtonVariant},
        theme::ControlSize,
    },
    theme::Palette,
};

/// Mobile week strip showing previous/next week chevrons and 7 compact two-line day pills
/// (weekday initial above day number and activity dot) without single-line button ellipsis.
pub fn mobile_week_strip(
    day: NaiveDate,
    week: &[NaiveDate],
    entries: &[CalendarEntry],
    palette: Palette,
    on_prev_week: impl Fn(&mut Window, &mut App) + 'static,
    on_next_week: impl Fn(&mut Window, &mut App) + 'static,
    on_pick_day: impl Fn(NaiveDate, &mut Window, &mut App) + 'static,
) -> Div {
    let on_pick = std::rc::Rc::new(on_pick_day);
    div()
        .debug_selector(|| "calendar-mobile-week-strip".into())
        .w_full()
        .flex()
        .items_center()
        .justify_between()
        .gap_1()
        .child(
            Button::new("previous-week", "‹")
                .size(ControlSize::Sm)
                .variant(ButtonVariant::Ghost)
                .on_click(move |_, window, cx| on_prev_week(window, cx)),
        )
        .children(week.iter().copied().enumerate().map(|(idx, date)| {
            let selected = date == day;
            let has_entries = !calendar::entries_on(entries, date).is_empty();
            let weekday_initial = date
                .format("%a")
                .to_string()
                .chars()
                .next()
                .unwrap_or(' ')
                .to_string();
            let day_num = if has_entries {
                format!("{}•", date.format("%-d"))
            } else {
                date.format("%-d").to_string()
            };
            let pick = on_pick.clone();
            let date_str = date.to_string();
            div()
                .id(SharedString::from(date_str))
                .debug_selector(move || format!("week-day-{idx}"))
                .flex_1()
                .min_w_0()
                .min_h(px(44.))
                .py_1()
                .px_0p5()
                .rounded(px(8.))
                .cursor_pointer()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_0p5()
                .bg(if selected {
                    rgb(palette.accent)
                } else {
                    rgba(0x00000000)
                })
                .text_color(if selected {
                    rgb(0xffffff)
                } else {
                    rgb(palette.text)
                })
                .on_click(move |_, window, cx| pick(date, window, cx))
                .child(
                    div()
                        .debug_selector(move || format!("week-day-label-{idx}"))
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(if selected {
                            rgb(0xffffff)
                        } else {
                            rgb(palette.muted)
                        })
                        .child(weekday_initial),
                )
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(day_num),
                )
        }))
        .child(
            Button::new("next-week", "›")
                .size(ControlSize::Sm)
                .variant(ButtonVariant::Ghost)
                .on_click(move |_, window, cx| on_next_week(window, cx)),
        )
}

fn minute_clock<V: 'static>(cx: &mut gpui::Context<V>) -> gpui::Task<()> {
    cx.spawn(async move |view, cx| loop {
        cx.background_executor()
            .timer(std::time::Duration::from_secs(60))
            .await;
        if view.update(cx, |_, cx| cx.notify()).is_err() {
            break;
        }
    })
}

/// Backwards-compatibility alias for callers expecting `sjel_gpui_ui::calendar::page::*`.
pub mod page_compat {
    pub use super::*;
}
pub use page_compat as page_alias;

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

    #[gpui::test]
    fn mobile_week_strip_renders_all_seven_day_labels_within_phone_width(cx: &mut TestAppContext) {
        crate::test_support::init(cx);
        let day = NaiveDate::from_ymd_opt(2026, 11, 21).expect("valid date");
        let week = sjel_gpui_core::calendar::week_dates(day);
        assert_eq!(week.len(), 7);
        let dates = week.clone();
        let (_view, cx) = cx.add_window_view(move |_, _| {
            struct Host {
                day: NaiveDate,
                week: Vec<NaiveDate>,
            }
            impl gpui_kit::Render for Host {
                fn render(
                    &mut self,
                    _: &mut gpui_kit::Window,
                    _: &mut gpui_kit::Context<Self>,
                ) -> impl IntoElement {
                    div().w(px(366.)).child(mobile_week_strip(
                        self.day,
                        &self.week,
                        &[],
                        Palette::dark(),
                        |_, _| {},
                        |_, _| {},
                        |_, _, _| {},
                    ))
                }
            }
            Host {
                day,
                week: dates.clone(),
            }
        });
        cx.run_until_parked();
        let strip = cx
            .debug_bounds("calendar-mobile-week-strip")
            .expect("mobile week strip rendered");
        assert!(strip.size.width <= px(366.));
        for (cell_sel, label_sel) in [
            ("week-day-0", "week-day-label-0"),
            ("week-day-1", "week-day-label-1"),
            ("week-day-2", "week-day-label-2"),
            ("week-day-3", "week-day-label-3"),
            ("week-day-4", "week-day-label-4"),
            ("week-day-5", "week-day-label-5"),
            ("week-day-6", "week-day-label-6"),
        ] {
            let cell = cx.debug_bounds(cell_sel).expect("day cell rendered");
            let label = cx
                .debug_bounds(label_sel)
                .expect("day label rendered without ellipsis");
            assert!(cell.size.width >= px(32.), "day pill has readable width");
            assert!(label.size.width > px(0.), "weekday label is visible");
            assert!(cell.right() <= strip.right());
        }
    }
}
