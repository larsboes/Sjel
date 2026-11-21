//! GPUI grid and controls for the signed, locally authenticated phone terminal.

use std::{cell::Cell, rc::Rc};

use alacritty_terminal::{index::Side, selection::SelectionType};
use gpui::{
    canvas, div, point, prelude::*, px, rgb, rgba, ClipboardItem, Context, Div, Entity, Focusable,
    FontWeight, MouseButton, Pixels, Point, SharedString, Subscription, Task, Window,
};

use super::{
    cell_size, close, edit_bytes, feed, open, publish, refresh, resize, select, send, state,
    unlock, zoom, Screen, Snapshot,
};
use crate::{
    ely::{
        buttons::{Button, ButtonVariant},
        forms::{Input, InputEvent, TextInput},
        theme::ControlSize,
    },
    theme::{ThemeContext, MONOKAI},
};

pub struct PhoneTerminal {
    snap: Snapshot,
    input: Entity<TextInput>,
    cwd: Entity<TextInput>,
    typed: String,
    tools: bool,
    confirm: bool,
    origin: Rc<Cell<Point<Pixels>>>,
    _input: Vec<Subscription>,
    _updates: Task<()>,
}

impl PhoneTerminal {
    pub fn new(window: &mut Window, keyboard: fn(bool), cx: &mut Context<Self>) -> Self {
        let mut updates = feed().tx.subscribe();
        let snap = updates.borrow_and_update().clone();
        let input = cx.new(|cx| {
            TextInput::new(window, cx)
                .placeholder("Terminal keyboard")
                .masked()
        });
        let cwd =
            cx.new(|cx| TextInput::new(window, cx).placeholder("Working directory (optional)"));
        let mut subscriptions =
            vec![
                cx.subscribe_in(&input, window, |view, input, event, _, cx| match event {
                    InputEvent::Changed if view.snap.unlocked => {
                        let next = input.read(cx).text();
                        let bytes = edit_bytes(&view.typed, next);
                        view.typed = next.to_owned();
                        if !bytes.is_empty() {
                            send(bytes);
                        }
                    }
                    InputEvent::Submit if view.snap.unlocked => view.raw(b"\r", cx),
                    _ => {}
                }),
            ];
        for field in [&input, &cwd] {
            let focus = field.read(cx).focus_handle(cx);
            subscriptions.push(cx.on_focus(&focus, window, move |_, _, _| keyboard(true)));
            subscriptions.push(cx.on_blur(&focus, window, move |_, _, _| keyboard(false)));
        }
        let task = cx.spawn(async move |view, cx| {
            while updates.changed().await.is_ok() {
                let snap = updates.borrow_and_update().clone();
                if view
                    .update(cx, |view, cx| {
                        if !snap.unlocked || snap.selected != view.snap.selected {
                            view.clear_input(cx);
                            view.confirm = false;
                        }
                        view.snap = snap;
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            snap,
            input,
            cwd,
            typed: String::new(),
            tools: false,
            confirm: false,
            origin: Rc::new(Cell::new(point(px(0.), px(0.)))),
            _input: subscriptions,
            _updates: task,
        }
    }

    fn clear_input(&mut self, cx: &mut Context<Self>) {
        self.typed.clear();
        self.input.update(cx, |input, cx| input.set_text("", cx));
    }

    fn raw(&mut self, bytes: &[u8], cx: &mut Context<Self>) {
        self.clear_input(cx);
        send(bytes.to_vec());
    }

    fn tools(&self, cx: &Context<Self>) -> Div {
        let mut tools = column()
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .child(button("Refresh terminals").on_click(|_, _, _| refresh()))
                    .child(
                        button("Scroll back").on_click(|_, _, _| with_grid(|grid| grid.scroll(10))),
                    )
                    .child(
                        button("Live output")
                            .on_click(|_, _, _| with_grid(|grid| grid.scroll(-10000))),
                    )
                    .child(button("Select text").on_click(|_, _, _| {
                        let mut session = state();
                        session.select = !session.select;
                        let clear = !session.select;
                        drop(session);
                        if clear {
                            with_grid(Screen::clear_selection);
                        }
                    }))
                    .child(button("Select all").on_click(|_, _, _| with_grid(Screen::select_all)))
                    .child(button("Copy").on_click(|_, _, cx| {
                        let session = state();
                        if let Some(text) = session
                            .snap
                            .selected
                            .as_ref()
                            .and_then(|id| session.screens.get(id))
                            .and_then(Screen::selected_text)
                        {
                            cx.write_to_clipboard(ClipboardItem::new_string(text));
                        }
                    }))
                    .child(button("Paste").on_click(|_, _, cx| {
                        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                            let bracketed = {
                                let session = state();
                                session
                                    .snap
                                    .selected
                                    .as_ref()
                                    .and_then(|id| session.screens.get(id))
                                    .is_some_and(Screen::bracketed_paste)
                            };
                            let text = text.replace("\r\n", "\r").replace('\n', "\r");
                            send(if bracketed {
                                format!("\x1b[200~{}\x1b[201~", text.replace("\x1b[201~", ""))
                                    .into_bytes()
                            } else {
                                text.into_bytes()
                            });
                        }
                    }))
                    .child(button("Smaller text").on_click(|_, _, _| zoom(-1)))
                    .child(button("Larger text").on_click(|_, _, _| zoom(1))),
            )
            .child(Input::new(&self.cwd))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .children(self.snap.options.presets.iter().map(|preset| {
                        let preset = preset.clone();
                        button(format!("New {preset}")).on_click(cx.listener(
                            move |view, _, _, cx| {
                                let cwd = view.cwd.read(cx).text().trim();
                                let size = { state().size };
                                open(
                                    preset.clone(),
                                    (!cwd.is_empty()).then(|| cwd.to_owned()),
                                    size,
                                );
                            },
                        ))
                    })),
            );
        if self.confirm {
            tools = tools.child(
                div()
                    .flex()
                    .child(button("Confirm terminate terminal").on_click(cx.listener(
                        |view, _, _, cx| {
                            close();
                            view.confirm = false;
                            cx.notify();
                        },
                    )))
                    .child(button("Cancel").on_click(cx.listener(|view, _, _, cx| {
                        view.confirm = false;
                        cx.notify();
                    }))),
            );
        } else {
            tools = tools.child(
                button("Terminate terminal")
                    .disabled(self.snap.selected.is_none())
                    .on_click(cx.listener(|view, _, _, cx| {
                        view.confirm = true;
                        cx.notify();
                    })),
            );
        }
        tools
    }
}

impl Render for PhoneTerminal {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = ThemeContext::of(window, cx);
        let palette = theme.palette;
        if !self.snap.unlocked {
            return column().p_4().child(label("Terminal locked"))
                .child(div().child("Confirm with Face ID or the device passcode before opening a terminal on the Mac."))
                .child(button("Unlock terminal").variant(ButtonVariant::Primary).on_click(|_, _, _| unlock()));
        }
        let (cell, line) = cell_size(&self.snap);
        let font = f32::from(self.snap.font_size);
        let family = self
            .snap
            .appearance
            .terminal_font
            .resolve(&["Menlo".into()], "Menlo");
        let origin = self.origin.clone();
        column()
            .size_full()
            .p_2()
            .min_h_0()
            .child(label("Terminal"))
            .child(
                div()
                    .id("terminal-sessions")
                    .overflow_x_scroll()
                    .child(div().flex().children(self.snap.sessions.iter().map(|term| {
                        let id = term.id.clone();
                        div().flex_none().child(
                            button(term.label())
                                .variant(if self.snap.selected.as_ref() == Some(&id) {
                                    ButtonVariant::Primary
                                } else {
                                    ButtonVariant::Ghost
                                })
                                .on_click(move |_, _, _| select(id.clone())),
                        )
                    }))),
            )
            .child(
                div()
                    .id("terminal-grid")
                    .relative()
                    .flex_1()
                    .min_h(px(70.))
                    .overflow_hidden()
                    .bg(rgba(palette.terminal_surface))
                    .font_family(family)
                    .text_size(px(font))
                    .line_height(px(line))
                    .child(
                        canvas(
                            move |bounds, _, _| {
                                origin.set(bounds.origin);
                                resize(bounds.size.width.into(), bounds.size.height.into());
                            },
                            |_, (), _, _| {},
                        )
                        .absolute()
                        .size_full(),
                    )
                    .children(self.snap.lines.iter().enumerate().flat_map(|(row, spans)| {
                        spans.iter().map(move |span| {
                            div()
                                .absolute()
                                .left(px(span.column as f32 * cell))
                                .top(px(row as f32 * line))
                                .h(px(line))
                                .text_color(rgb(span.fg))
                                .when(span.bold, |el| el.font_weight(FontWeight::BOLD))
                                .when_some(span.bg, |el, bg| el.bg(rgb(bg)))
                                .child(span.text.clone())
                        })
                    }))
                    .children(self.snap.cursors.iter().map(|&(row, col)| {
                        div()
                            .absolute()
                            .left(px(col as f32 * cell))
                            .top(px((row + 1) as f32 * line - 2.))
                            .w(px(cell))
                            .h(px(2.))
                            .bg(rgb(MONOKAI.cursor))
                    }))
                    .on_scroll(move |event, _, cx| {
                        let delta = event.delta.pixel_delta(px(line));
                        let rows = (f32::from(delta.y) / line).round() as i32;
                        with_grid(|grid| grid.scroll(rows));
                        cx.stop_propagation();
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|view, event: &gpui::MouseDownEvent, _, _| {
                            drag(view.origin.get(), event.position, true);
                        }),
                    )
                    .on_mouse_move(cx.listener(|view, event: &gpui::MouseMoveEvent, _, _| {
                        if event.dragging() {
                            drag(view.origin.get(), event.position, false);
                        }
                    }))
                    .on_mouse_up(MouseButton::Left, |_, _, _| {
                        state().drag = None;
                        with_grid(Screen::drop_empty_selection);
                    }),
            )
            .child(
                div()
                    .id("terminal-input")
                    .capture_key_down(cx.listener(|view, event: &gpui::KeyDownEvent, _, cx| {
                        let key = event.keystroke.key.as_str();
                        let bytes: &[u8] = match key {
                            "up" => b"\x1b[A",
                            "down" => b"\x1b[B",
                            "left" => b"\x1b[D",
                            "right" => b"\x1b[C",
                            "escape" => b"\x1b",
                            "tab" => b"\t",
                            "backspace" if view.typed.is_empty() => &[0x7f],
                            "c" if event.keystroke.modifiers.control => &[3],
                            _ => return,
                        };
                        cx.stop_propagation();
                        view.raw(bytes, cx);
                    }))
                    .child(Input::new(&self.input)),
            )
            .child(
                div().flex().flex_wrap().children(
                    [
                        ("Esc", b"\x1b".as_slice()),
                        ("Tab", b"\t".as_slice()),
                        ("←", b"\x1b[D".as_slice()),
                        ("↑", b"\x1b[A".as_slice()),
                        ("↓", b"\x1b[B".as_slice()),
                        ("→", b"\x1b[C".as_slice()),
                        ("Ctrl-C", &[3]),
                        ("Return", b"\r".as_slice()),
                    ]
                    .map(|(label, bytes)| {
                        button(label)
                            .on_click(cx.listener(move |view, _, _, cx| view.raw(bytes, cx)))
                    }),
                ),
            )
            .child(
                button("Terminal tools").on_click(cx.listener(|view, _, _, cx| {
                    view.tools = !view.tools;
                    cx.notify();
                })),
            )
            .when(self.tools, |panel| {
                panel.child(
                    div()
                        .id("terminal-tools")
                        .max_h(px(132.))
                        .overflow_y_scroll()
                        .child(self.tools(cx)),
                )
            })
            .child(
                div().text_sm().child(
                    self.snap
                        .error
                        .clone()
                        .unwrap_or_else(|| self.snap.status.clone()),
                ),
            )
    }
}

fn with_grid(change: impl FnOnce(&mut Screen)) {
    let mut session = state();
    if !session.snap.unlocked {
        return;
    }
    if let Some(grid) = session
        .snap
        .selected
        .clone()
        .and_then(|id| session.screens.get_mut(&id))
    {
        change(grid);
        publish(&mut session);
    }
}

fn drag(origin: Point<Pixels>, position: Point<Pixels>, start: bool) {
    let mut session = state();
    if !session.snap.unlocked {
        return;
    }
    let relative = position - origin;
    let at = (f32::from(relative.x), f32::from(relative.y));
    let (cell, line) = cell_size(&session.snap);
    let before = session.drag.replace(at);
    let selecting = session.select;
    if let Some(grid) = session
        .snap
        .selected
        .clone()
        .and_then(|id| session.screens.get_mut(&id))
    {
        let point = grid.point_at(
            (at.0 / cell).max(0.) as usize,
            (at.1 / line).max(0.) as usize,
        );
        if selecting && start {
            grid.select(SelectionType::Simple, point, Side::Left);
        } else if selecting {
            grid.extend_selection(point, Side::Right);
        } else if let Some(before) = before.filter(|_| !start) {
            grid.scroll(((at.1 - before.1) / line).round() as i32);
        }
        publish(&mut session);
    }
}

fn column() -> Div {
    div().flex().flex_col().gap_1()
}
fn label(text: &'static str) -> Div {
    div()
        .text_xl()
        .font_weight(FontWeight::SEMIBOLD)
        .child(text)
}
fn button(label: impl Into<SharedString>) -> Button {
    let label = label.into();
    Button::new(label.clone(), label)
        .size(ControlSize::Sm)
        .variant(ButtonVariant::Ghost)
}
