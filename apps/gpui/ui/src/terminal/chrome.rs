use super::{keys::TOUCH_KEYS, Copy, Paste, TerminalEvent, TerminalView, PINCH_STEP};
use crate::{
    ely::terminal::Appearance,
    theme::{Palette, MONOKAI},
};
use gpui::{
    div, font, prelude::*, px, rgb, rgba, AnyElement, Context, FontWeight, MouseButton, PinchEvent,
    Render, SharedString, TouchPhase, Window,
};

impl TerminalView {
    pub fn key_row(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let ended = !self.peer.alive;
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap_1()
            .flex_wrap()
            .w_full()
            .children(TOUCH_KEYS.iter().map(|(label, bytes)| {
                div()
                    .id(SharedString::from(format!("terminal-key-{label}")))
                    .px_2()
                    .py(px(3.))
                    .rounded(px(4.))
                    .bg(rgba(palette.raised))
                    .text_color(rgb(palette.muted))
                    .text_xs()
                    .cursor_pointer()
                    .when(ended, |this| this.opacity(0.4))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |view, _event, _window, cx| {
                            if view.peer.alive {
                                view.send(bytes.to_vec(), cx);
                            }
                        }),
                    )
                    .child(SharedString::from(*label))
            }))
            // A touch screen has no ⌘C or ⌘V, so the row carries them.
            .child(
                div()
                    .id("terminal-key-copy")
                    .px_2()
                    .py(px(3.))
                    .rounded(px(4.))
                    .bg(rgba(palette.raised))
                    .text_color(rgb(palette.muted))
                    .text_xs()
                    .cursor_pointer()
                    .when(self.terminal.read(cx).selected_text().is_none(), |this| {
                        this.opacity(0.4)
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|view, _event, window, cx| view.copy(&Copy, window, cx)),
                    )
                    .child("copy"),
            )
            .child(
                div()
                    .id("terminal-key-paste")
                    .px_2()
                    .py(px(3.))
                    .rounded(px(4.))
                    .bg(rgba(palette.raised))
                    .text_color(rgb(palette.muted))
                    .text_xs()
                    .cursor_pointer()
                    .when(ended, |this| this.opacity(0.4))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|view, _event, window, cx| view.paste(&Paste, window, cx)),
                    )
                    .child("paste"),
            )
            .into_any_element()
    }

    fn header(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let status: SharedString = match (self.problem.as_ref(), self.peer.ending()) {
            (Some(problem), _) => problem.clone(),
            (None, Some(ending)) => ending.into(),
            (None, None) => match (self.terminal.read(cx).scroll_offset(), self.peer.command()) {
                (0, Some(command)) => format!("live · {command}").into(),
                (scrolled @ 1.., _) => format!("{scrolled} lines back").into(),
                _ => "live".into(),
            },
        };
        let ended = !self.peer.alive || self.closing;
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .w_full()
            .text_xs()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .min_w_0()
                    .child(
                        div()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(palette.text))
                            .child(SharedString::from(self.peer.label())),
                    )
                    .child(
                        div()
                            .text_color(rgb(palette.muted))
                            .child(SharedString::from(self.peer.directory())),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .text_color(if self.peer.alive {
                                rgb(palette.muted)
                            } else {
                                rgb(palette.amber)
                            })
                            .child(status),
                    )
                    .child(
                        div()
                            .id("terminal-end")
                            .px_2()
                            .py(px(2.))
                            .rounded(px(4.))
                            .bg(rgba(palette.raised))
                            .text_color(rgb(palette.muted))
                            .cursor_pointer()
                            .when(ended, |this| this.opacity(0.4))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|view, _event, _window, cx| {
                                    if view.peer.alive && !view.closing {
                                        view.close(cx);
                                    }
                                }),
                            )
                            .child(if self.peer.alive { "End" } else { "Ended" }),
                    ),
            )
            .into_any_element()
    }
}

impl Render for TerminalView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self.palette;
        let font_size = px(f32::from(self.font_size));
        let appearance = Appearance {
            font: font(self.font_family.clone()),
            font_size,
            line_height: font_size * 1.4,
            foreground: rgb(MONOKAI.foreground).into(),
            background: rgba(palette.terminal_surface).into(),
            ansi: MONOKAI.ansi.map(|color| rgb(color).into()),
            cursor: rgb(MONOKAI.cursor).into(),
            selection: rgb(MONOKAI.selection).into(),
        };
        if let Err(error) = self
            .terminal
            .update(cx, |terminal, cx| terminal.set_appearance(appearance, cx))
        {
            self.problem = Some(error.to_string().into());
        }
        let header = self.header(palette, cx);
        let keys = self.touch.then(|| self.key_row(palette, cx));
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .text_color(rgb(palette.text))
            .child(header)
            .children(keys)
            .child(
                div()
                    .id("terminal-grid")
                    .key_context("Terminal")
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(rgba(palette.border))
                    .overflow_hidden()
                    .on_action(cx.listener(Self::copy))
                    .on_action(cx.listener(Self::paste))
                    .on_action(cx.listener(Self::select_all))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|view, _, _, cx| {
                            if view.touch {
                                cx.emit(TerminalEvent::Editing(true));
                            }
                        }),
                    )
                    .on_pinch(cx.listener(|view, event: &PinchEvent, _, cx| {
                        cx.stop_propagation();
                        view.pinch += event.delta;
                        if view.pinch.abs() >= PINCH_STEP {
                            cx.emit(TerminalEvent::Zoom(view.pinch.signum() as i32));
                            view.pinch = 0.;
                        }
                        if event.phase == TouchPhase::Ended {
                            view.pinch = 0.;
                        }
                    }))
                    .child(self.terminal.clone()),
            )
    }
}
