//! GPUI surfaces for the phone's Scratchpad and Activity seats.

use gpui::{div, prelude::*, rgb, rgba, Context, Div, FontWeight, Window};

use super::{feed, Activity};
use crate::theme::{Palette, ThemeContext};

pub struct PhoneScratchpad;

impl Render for PhoneScratchpad {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = ThemeContext::of(window, cx).palette;
        panel(palette)
            .child(heading("Daily Scratchpad"))
            .child(detail(
                palette,
                "Quick thoughts, links and tasks live here for 24h before sorting into Sjel.",
            ))
    }
}

pub struct PhoneActivity {
    activity: Activity,
    _updates: gpui::Task<()>,
}

impl PhoneActivity {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let mut source = feed().tx.subscribe();
        let activity = source.borrow_and_update().activity.clone();
        let updates = cx.spawn(async move |view, cx| {
            while source.changed().await.is_ok() {
                let activity = source.borrow_and_update().activity.clone();
                if view
                    .update(cx, |view, cx| {
                        if view.activity != activity {
                            view.activity = activity;
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            activity,
            _updates: updates,
        }
    }
}

impl Render for PhoneActivity {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = ThemeContext::of(window, cx).palette;
        div()
            .id("phone-activity-scroll")
            .size_full()
            .overflow_y_scroll()
            .bg(rgba(palette.surface))
            .child(
                panel(palette)
                    .child(heading("Activity"))
                    .child(group("With the Mac", &self.activity.connection, palette))
                    .child(group("Reminders", &self.activity.reminders, palette)),
            )
    }
}

fn panel(palette: Palette) -> Div {
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap_3()
        .p_4()
        .bg(rgba(palette.surface))
        .text_color(rgb(palette.text))
}

fn heading(title: &'static str) -> Div {
    div()
        .text_xl()
        .font_weight(FontWeight::SEMIBOLD)
        .child(title)
}

fn detail(palette: Palette, line: impl Into<gpui::SharedString>) -> Div {
    div()
        .text_sm()
        .text_color(rgb(palette.muted))
        .child(line.into())
}

fn group(title: &'static str, lines: &[String], palette: Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(div().font_weight(FontWeight::SEMIBOLD).child(title))
        .children(lines.iter().cloned().map(|line| detail(palette, line)))
}
