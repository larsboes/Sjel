//! Capture input field, quick tag chips, and save dispatch.

use gpui::{
    div, prelude::*, px, rgb, rgba, AnyElement, Context, FontWeight, MouseButton,
    SharedString, Window,
};
use gpui_kit::component::input::{InputEvent, InputState};
use sjel_gpui_core::capture::{Draft, TAGS};
use sjel_gpui_core::shell;

use super::capture::preview_label;
use super::types::InboxEvent;
use super::Inbox;

impl Inbox {
    /// Keeps what is in the field: written into the replica first, queued for the Mac second.
    pub(crate) fn keep(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        let Some(input) = self.input.as_ref().map(|(input, _)| input.clone()) else {
            return;
        };
        let typed = input.read(cx).value().to_string();
        let draft = match Draft::parse(&typed, self.tag) {
            Ok(draft) => draft,
            Err(why) => {
                self.error = Some(SharedString::from(why));
                cx.notify();
                return;
            }
        };
        let Some(store) = self.store.clone() else {
            self.error = Some(SharedString::from(
                "Captures cannot be saved on this phone: its store did not open.",
            ));
            cx.notify();
            return;
        };
        self.saving = true;
        self.error = None;
        let work = shell::run(async move {
            store
                .save_capture(
                    draft.text.as_deref(),
                    draft.url.as_deref(),
                    draft.tag.as_deref(),
                )
                .await
                .map_err(|e| e.to_string())
        });
        cx.spawn_in(window, async move |this, cx| {
            let answer = work
                .await
                .unwrap_or_else(|| Err("The capture was not written.".to_owned()));
            let _ = this.update_in(cx, |view, window, cx| {
                view.saving = false;
                match answer {
                    Ok(_) => {
                        // The field empties and lets go of the chip: the next capture is a new
                        // thought, not a correction of the last one.
                        view.tag = None;
                        if let Some((input, _)) = view.input.as_ref() {
                            let input = input.clone();
                            input.update(cx, |input, cx| input.set_value("", window, cx));
                        }
                        view.load(cx);
                        cx.emit(InboxEvent::Wrote);
                    }
                    Err(error) => view.error = Some(SharedString::from(error)),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// The field, created once. Enter keeps the capture, and the keyboard follows the focus.
    pub(crate) fn field(&mut self, window: &mut Window, cx: &mut Context<Self>) -> gpui::Entity<InputState> {
        if let Some((input, _)) = &self.input {
            return input.clone();
        }
        let input =
            cx.new(|cx| InputState::new(window, cx).placeholder("A thought, a link, or #buy …"));
        let subscription =
            cx.subscribe_in(&input, window, |view, _, event, window, cx| match event {
                InputEvent::PressEnter { .. } => view.keep(window, cx),
                InputEvent::Focus => cx.emit(InboxEvent::Editing(true)),
                InputEvent::Blur => cx.emit(InboxEvent::Editing(false)),
                _ => {}
            });
        self.input = Some((input.clone(), subscription));
        input
    }

    /// The preview under the field: where this will land, or that it waits.
    pub(crate) fn preview(&self, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        let typed = self
            .input
            .as_ref()
            .map(|(input, _)| input.read(cx).value().to_string())
            .unwrap_or_default();
        div()
            .text_xs()
            .text_color(rgb(palette.muted))
            .child(preview_label(&typed, self.tag))
            .into_any_element()
    }

    /// The four tags, one at a time, and none for the rules to decide.
    pub(crate) fn chips(&self, cx: &mut Context<Self>) -> AnyElement {
        let palette = self.palette;
        let mut row = div().flex().items_center().gap_2();
        for tag in TAGS {
            let active = self.tag == Some(tag);
            row = row.child(
                div()
                    .id(SharedString::from(format!("inbox-tag-{tag}")))
                    .flex()
                    .items_center()
                    .justify_center()
                    .min_h(px(34.))
                    .px_3()
                    .rounded(px(8.))
                    .text_xs()
                    .cursor_pointer()
                    .when(active, |chip| {
                        chip.bg(rgb(palette.accent))
                            .text_color(rgb(0xffffff))
                            .font_weight(FontWeight::SEMIBOLD)
                    })
                    .when(!active, |chip| {
                        chip.bg(rgba(palette.raised))
                            .text_color(rgb(palette.muted))
                            .border_1()
                            .border_color(rgba(palette.border))
                    })
                    .child(format!("#{tag}"))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |view, _, _, cx| {
                            view.tag = if view.tag == Some(tag) {
                                None
                            } else {
                                Some(tag)
                            };
                            cx.notify();
                        }),
                    ),
            );
        }
        row.into_any_element()
    }
}
