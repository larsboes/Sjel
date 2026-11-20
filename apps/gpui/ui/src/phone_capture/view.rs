//! GPUI layout for entering a capture and checking the phone's outbox.

use gpui::{
    div, prelude::*, rgb, rgba, Context, Entity, Focusable, FontWeight, Subscription, Task, Window,
};
use sjel_gpui_core::capture::TAGS;

use super::{headline, keep, load, preview_label, second_line, subscribe, Snapshot};
use crate::{
    ely::{
        buttons::{Button, ButtonVariant},
        forms::{Input, InputEvent, TextInput},
        theme::ControlSize,
    },
    theme::ThemeContext,
};

/// Phone capture entry and replica outbox.
pub struct PhoneCapture {
    snapshot: Snapshot,
    draft: Entity<TextInput>,
    tag: Option<&'static str>,
    _input: [Subscription; 3],
    _updates: Task<()>,
}

impl PhoneCapture {
    /// Observes the existing local queue and delegates system keyboard visibility to the host.
    pub fn new(window: &mut Window, keyboard: fn(bool), cx: &mut Context<Self>) -> Self {
        let mut source = subscribe();
        let snapshot = source.borrow_and_update().clone();
        let draft = cx.new(|cx| TextInput::new(window, cx).placeholder("Note, link or task…"));
        let input = cx.subscribe_in(&draft, window, |view, input, event, _, cx| match event {
            InputEvent::Changed => cx.notify(),
            InputEvent::Submit => keep(input.read(cx).text().to_owned(), view.tag),
            _ => {}
        });
        let focus = draft.read(cx).focus_handle(cx);
        let on_focus = cx.on_focus(&focus, window, move |_, _, _| keyboard(true));
        let on_blur = cx.on_blur(&focus, window, move |_, _, _| keyboard(false));
        let updates = cx.spawn(async move |this, cx| {
            while source.changed().await.is_ok() {
                let snapshot = source.borrow_and_update().clone();
                if this
                    .update(cx, |view, cx| {
                        if snapshot.saved != view.snapshot.saved {
                            view.draft.update(cx, |input, cx| input.set_text("", cx));
                            view.tag = None;
                        }
                        view.snapshot = snapshot;
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        load();
        Self {
            snapshot,
            draft,
            tag: None,
            _input: [input, on_focus, on_blur],
            _updates: updates,
        }
    }
}

impl Render for PhoneCapture {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = ThemeContext::of(window, cx).palette;
        let waiting = self
            .snapshot
            .captures
            .iter()
            .filter(|capture| !capture.on_the_mac())
            .count();
        let status = match (self.snapshot.captures.is_empty(), waiting) {
            (true, _) => "Nothing captured on this phone yet.".into(),
            (_, 0) => "Everything here is on the Mac.".into(),
            (_, waiting) => format!("{waiting} waiting for the Mac"),
        };
        let preview = preview_label(self.draft.read(cx).text(), self.tag);
        let tags = TAGS.iter().map(|&tag| {
            Button::new(format!("capture-{tag}"), format!("#{tag}"))
                .variant(if self.tag == Some(tag) {
                    ButtonVariant::Primary
                } else {
                    ButtonVariant::Outline
                })
                .on_click(cx.listener(move |view, _, _, cx| {
                    view.tag = if view.tag == Some(tag) {
                        None
                    } else {
                        Some(tag)
                    };
                    cx.notify();
                }))
        });
        let rows = self.snapshot.captures.iter().map(|capture| {
            let refusal = self
                .snapshot
                .refusals
                .iter()
                .find_map(|(id, reason)| (id == &capture.id).then_some(reason));
            div()
                .flex()
                .flex_col()
                .gap_1()
                .py_3()
                .border_b_1()
                .border_color(rgba(palette.border))
                .child(headline(capture))
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(if refusal.is_some() {
                            palette.red
                        } else {
                            palette.muted
                        }))
                        .child(second_line(capture, refusal)),
                )
        });
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_2()
            .p_4()
            .bg(rgba(palette.surface))
            .text_color(rgb(palette.text))
            .child(
                div()
                    .text_xl()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Capture"),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .items_center()
                    .child(div().flex_1().child(Input::new(&self.draft)))
                    .child(
                        Button::new(
                            "capture-keep",
                            if self.snapshot.saving {
                                "Saving…"
                            } else {
                                "Keep"
                            },
                        )
                        .primary()
                        .size(ControlSize::Lg)
                        .disabled(self.snapshot.saving)
                        .on_click(cx.listener(|view, _, _, cx| {
                            keep(view.draft.read(cx).text().to_owned(), view.tag)
                        })),
                    ),
            )
            .child(div().flex().flex_wrap().gap_1().children(tags))
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(palette.muted))
                    .child(preview),
            )
            .when_some(self.snapshot.error.clone(), |body, error| {
                body.child(div().text_sm().text_color(rgb(palette.red)).child(error))
            })
            .child(div().text_sm().text_color(rgb(palette.muted)).child(status))
            .child(
                div()
                    .id("capture-outbox")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .children(rows),
            )
    }
}
