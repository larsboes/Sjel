//! Today header view, source indicator line, quick capture bar, and device pairing banner.
use gpui::{
    div, prelude::*, px, rgb, rgba, AnyElement, Context, FontWeight, MouseButton,
};
use gpui_kit::component::{Icon, IconName, Sizable};
use sjel_gpui_core::today::Today as View;

use super::types::TodayEvent;
use super::Today;

impl Today {
    pub(super) fn header_button(
        &self,
        id: &'static str,
        glyph: &'static str,
        event: TodayEvent,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let palette = self.palette;
        div()
            .id(id)
            .flex_none()
            .size(px(36.))
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .text_sm()
            .text_color(rgb(palette.muted))
            .bg(rgba(palette.surface))
            .border_1()
            .border_color(rgba(palette.border))
            .child(glyph)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |_, _, _, cx| cx.emit(event.clone())),
            )
            .into_any_element()
    }

    pub(super) fn source_line(&self, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        let source = match (&self.error, &self.copy, &self.missing) {
            (Some(error), _, _) => format!("The day could not be read: {error}"),
            (_, Some(copy), _) => format!(
                "Calendar from the Mac's copy, {}",
                copy.manifest().built_label()
            ),
            (_, None, Some(missing)) => missing.to_string(),
            (_, None, None) => "Looking for the Mac's copy…".to_owned(),
        };
        let mut column = div()
            .flex()
            .flex_col()
            .gap_1()
            .child(div().text_xs().text_color(rgb(palette.muted)).child(source));
        if let Some(note) = self.note.clone() {
            column = column.child(div().text_xs().text_color(rgb(palette.amber)).child(note));
        }
        let mut travel: Vec<String> = Vec::new();
        if self.pending > 0 {
            travel.push(format!("{} waiting for the Mac", self.pending));
        }
        if !self.paired {
            travel.push("not connected, so changes stay here".to_owned());
        }
        if !travel.is_empty() {
            column = column.child(
                div()
                    .text_xs()
                    .text_color(rgb(if self.pending > 0 {
                        palette.amber
                    } else {
                        palette.muted
                    }))
                    .child(travel.join(" · ")),
            );
        }
        let mut block = div().flex().flex_col().gap_2().child(column);
        if !self.paired && self.copy.is_none() {
            block = block.child(
                div()
                    .id("today-pair")
                    .min_h(px(44.))
                    .rounded(px(10.))
                    .bg(rgb(palette.accent))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(0xffffff))
                    .child("Pair this phone with your Mac")
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|_, _, _, cx| cx.emit(TodayEvent::Settings)),
                    ),
            );
        }
        block.into_any_element()
    }

    pub(super) fn header(&self, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        let view = self.view.as_ref();
        let headline = view
            .map(View::headline)
            .unwrap_or_else(|| self.day().format("%A, %-d %B").to_string());
        let summary = view.map(View::summary).unwrap_or_default();
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_start()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(palette.muted))
                                    .child("TODAY"),
                            )
                            .child(
                                div()
                                    .text_xl()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(palette.text))
                                    .child(headline),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(palette.muted))
                                    .child(summary),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(self.header_button(
                                "today-soundscape",
                                "♪",
                                TodayEvent::Soundscape,
                                cx,
                            ))
                            .child(self.header_button(
                                "today-activity",
                                "≡",
                                TodayEvent::Activity,
                                cx,
                            ))
                            .child(self.header_button(
                                "today-settings",
                                "⚙",
                                TodayEvent::Settings,
                                cx,
                            )),
                    ),
            )
            .child(self.source_line(cx))
            .into_any_element()
    }

    pub(super) fn quick_capture_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .id("today-quick-capture")
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_4()
                    .py_2()
                    .min_h(self.density.min_touch_target())
                    .rounded(px(10.))
                    .bg(rgba(crate::theme::tint(palette.raised, 0x90)))
                    .border_1()
                    .border_color(rgba(crate::theme::tint(palette.border, 0x40)))
                    .cursor_pointer()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|_, _, _, cx| cx.emit(TodayEvent::Capture)),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_sm()
                            .text_color(rgb(palette.muted))
                            .child("✎")
                            .child("Capture note, link or task…"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .px_2()
                            .py(px(2.))
                            .rounded(px(6.))
                            .bg(rgb(palette.accent))
                            .text_color(rgb(0xffffff))
                            .child("+ New"),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .id("today-open-people")
                            .flex_1()
                            .flex()
                            .items_center()
                            .justify_center()
                            .gap_1()
                            .py(px(6.))
                            .min_h(self.density.min_touch_target())
                            .rounded(px(8.))
                            .bg(rgba(palette.surface))
                            .border_1()
                            .border_color(rgba(crate::theme::tint(palette.border, 0x40)))
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(palette.text))
                            .cursor_pointer()
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|_, _, _, cx| cx.emit(TodayEvent::People)),
                            )
                            .child(
                                Icon::new(IconName::User)
                                    .xsmall()
                                    .text_color(rgb(palette.muted)),
                            )
                            .child("People & Places"),
                    )
                    .child(
                        div()
                            .id("today-open-documents")
                            .flex_1()
                            .flex()
                            .items_center()
                            .justify_center()
                            .gap_1()
                            .py(px(6.))
                            .min_h(self.density.min_touch_target())
                            .rounded(px(8.))
                            .bg(rgba(palette.surface))
                            .border_1()
                            .border_color(rgba(crate::theme::tint(palette.border, 0x40)))
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(palette.text))
                            .cursor_pointer()
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|_, _, _, cx| cx.emit(TodayEvent::Documents)),
                            )
                            .child(
                                Icon::new(IconName::File)
                                    .xsmall()
                                    .text_color(rgb(palette.muted)),
                            )
                            .child("Documents"),
                    ),
            )
            .into_any_element()
    }
}
