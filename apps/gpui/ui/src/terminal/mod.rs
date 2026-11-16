//! Capability-owned terminal sessions rendered by Ely on the Mac.

use std::time::Duration;

use futures::StreamExt;
use gpui::{Context, Entity, EventEmitter, FocusHandle, SharedString, Subscription, Task};
use gpui_kit::component::ActiveTheme;
use sjel_gpui_core::{
    shell,
    terminal::{Host, Position, Terminal as Peer, Update, MAX_INPUT_BYTES},
};

use crate::{
    ely::terminal::Terminal,
    theme::{Palette, TerminalFont},
};

mod chrome;
mod deck;
mod keys;
#[cfg(test)]
mod tests;
mod transport;

pub use deck::TerminalDeck;
pub use keys::{key_bytes, keys, Copy, NewTerminal, NextTab, Paste, PrevTab, SelectAll};

const RESIZE_SETTLE: Duration = Duration::from_millis(150);
const PINCH_STEP: f32 = 0.15;
pub const UNANSWERED: &str = "The Mac did not answer.";

pub enum TerminalEvent {
    Editing(bool),
    Zoom(i32),
    Find,
}

pub struct TerminalView {
    peer: Peer,
    host: Host,
    terminal: Entity<Terminal>,
    position: Position,
    problem: Option<SharedString>,
    palette: Palette,
    closing: bool,
    touch: bool,
    outgoing: Vec<u8>,
    sending: bool,
    font_family: SharedString,
    font_size: u8,
    font_choice: Option<(TerminalFont, u8)>,
    pinch: f32,
    dimensions: (usize, usize),
    resize_generation: u64,
    _requests: Task<()>,
    _events: Subscription,
    _grid: Subscription,
}

impl EventEmitter<TerminalEvent> for TerminalView {}

impl gpui::Focusable for TerminalView {
    fn focus_handle(&self, cx: &gpui::App) -> FocusHandle {
        gpui::Focusable::focus_handle(self.terminal.read(cx), cx)
    }
}

impl TerminalView {
    pub fn new(
        peer: Peer,
        host: Host,
        from: u64,
        cols: usize,
        rows: usize,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Self {
        let dimensions = (cols.max(2), rows.max(1));
        let (terminal, requests) = transport::connect(dimensions, cx);
        let grid = cx.observe(&terminal, |_, _, cx| cx.notify());
        let events = cx.subscribe(&terminal, |_, _, event, cx| {
            match event {
                crate::ely::terminal::TerminalEvent::Zoom(step) => {
                    cx.emit(TerminalEvent::Zoom(*step))
                }
                crate::ely::terminal::TerminalEvent::Find => cx.emit(TerminalEvent::Find),
                _ => {}
            }
            cx.notify();
        });
        Self {
            peer,
            host,
            terminal,
            position: Position::new(from),
            problem: None,
            palette,
            closing: false,
            touch: false,
            outgoing: Vec::new(),
            sending: false,
            font_family: "Menlo".into(),
            font_size: 13,
            font_choice: None,
            pinch: 0.,
            dimensions,
            resize_generation: 0,
            _requests: requests,
            _events: events,
            _grid: grid,
        }
    }

    pub fn set_style(&mut self, palette: Palette, font: TerminalFont, size: u8, cx: &gpui::App) {
        self.palette = palette;
        if self.font_choice != Some((font, size)) {
            self.font_choice = Some((font, size));
            self.font_family = font
                .resolve(TerminalFont::families(cx), &cx.theme().mono_font_family)
                .into();
            self.font_size = size;
        }
    }

    pub fn touch(mut self) -> Self {
        self.touch = true;
        self
    }

    pub fn peer(&self) -> &Peer {
        &self.peer
    }
    pub fn id(&self) -> &str {
        &self.peer.id
    }
    pub fn position(&self) -> u64 {
        self.position.offset()
    }

    /// Applies capability offsets before handing ordered output to Ely's persistent parser.
    pub fn apply(&mut self, update: Update, cx: &mut Context<Self>) -> bool {
        let result = match update {
            Update::Output { offset, data } => {
                if !self.position.accept(offset, data.len()) {
                    return false;
                }
                self.terminal
                    .update(cx, |terminal, cx| terminal.feed_remote(&data, cx))
            }
            Update::Truncated { first_offset } => {
                self.position.restart(first_offset);
                self.terminal
                    .update(cx, |terminal, cx| terminal.reset_remote(cx))
            }
            Update::Exit { code, signal } => {
                self.peer.alive = false;
                self.peer.exit_code = Some(code);
                self.peer.exit_signal = signal;
                self.terminal
                    .update(cx, |terminal, cx| terminal.finish_remote(cx));
                Ok(())
            }
            Update::Command(command) => {
                self.peer.shell = command;
                Ok(())
            }
        };
        if let Err(error) = result {
            self.problem = Some(error.to_string().into());
        }
        cx.notify();
        true
    }

    /// Reconnects only after truncation; command marks do not end the output stream.
    pub fn attach(&mut self, cx: &mut Context<Self>) {
        let host = self.host.clone();
        let id = self.peer.id.clone();
        cx.spawn(async move |weak, cx| loop {
            let Ok(from) = weak.read_with(cx, |view, _| view.position()) else {
                return;
            };
            let stream = host.clone();
            let name = id.clone();
            let work = shell::run(async move { stream.stream(&name, from).await });
            let mut receiver = match work.await.unwrap_or_else(|| Err(UNANSWERED.into())) {
                Ok(receiver) => receiver,
                Err(error) => {
                    let _ = weak.update(cx, |view, cx| {
                        view.problem = Some(error.into());
                        cx.notify();
                    });
                    return;
                }
            };
            let mut again = false;
            while let Some(update) = receiver.next().await {
                let truncated = matches!(update, Update::Truncated { .. });
                let ended = matches!(update, Update::Exit { .. });
                if weak
                    .update(cx, |view, cx| {
                        view.apply(update, cx);
                    })
                    .is_err()
                {
                    return;
                }
                if truncated {
                    again = true;
                    break;
                }
                if ended {
                    return;
                }
            }
            if !again {
                return;
            }
        })
        .detach();
    }

    /// External key rows use the same selection, scroll and input path as Ely's keyboard.
    pub fn send(&mut self, bytes: Vec<u8>, cx: &mut Context<Self>) {
        if !bytes.is_empty() && self.peer.alive {
            self.terminal
                .update(cx, |terminal, cx| terminal.send_input(bytes, cx));
        }
    }

    /// One network writer drains bounded batches in input order.
    fn queue_input(&mut self, bytes: Vec<u8>, cx: &mut Context<Self>) {
        if bytes.is_empty() || !self.peer.alive {
            return;
        }
        self.outgoing.extend_from_slice(&bytes);
        if self.sending {
            return;
        }
        self.sending = true;
        let host = self.host.clone();
        let id = self.peer.id.clone();
        cx.spawn(async move |weak, cx| loop {
            let Ok(batch) = weak.update(cx, |view, _| {
                let take = view.outgoing.len().min(MAX_INPUT_BYTES);
                let batch: Vec<u8> = view.outgoing.drain(..take).collect();
                if batch.is_empty() {
                    view.sending = false;
                }
                batch
            }) else {
                return;
            };
            if batch.is_empty() {
                return;
            }
            let host = host.clone();
            let id = id.clone();
            let work = shell::run(async move { host.input(&id, &batch).await });
            if let Err(error) = work.await.unwrap_or_else(|| Err(UNANSWERED.into())) {
                let _ = weak.update(cx, |view, cx| {
                    view.outgoing.clear();
                    view.sending = false;
                    view.problem = Some(error.into());
                    cx.notify();
                });
                return;
            }
        })
        .detach();
    }

    pub fn copy(&mut self, _: &Copy, _: &mut gpui::Window, cx: &mut Context<Self>) {
        if let Some(text) = self.terminal.read(cx).selected_text() {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
        }
    }

    pub fn paste(&mut self, _: &Paste, _: &mut gpui::Window, cx: &mut Context<Self>) {
        if !self.peer.alive {
            return;
        }
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.terminal
                .update(cx, |terminal, cx| terminal.paste_text(&text, cx));
        }
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut gpui::Window, cx: &mut Context<Self>) {
        self.terminal
            .update(cx, |terminal, cx| terminal.select_all(cx));
    }

    /// The renderer resizes immediately; only the last settled size reaches the capability.
    pub fn resize(&mut self, cols: usize, rows: usize, cx: &mut Context<Self>) {
        let dimensions = (cols.max(2), rows.max(1));
        if dimensions == self.dimensions {
            return;
        }
        self.dimensions = dimensions;
        self.resize_generation += 1;
        let generation = self.resize_generation;
        let host = self.host.clone();
        let id = self.peer.id.clone();
        let cols = dimensions.0.min(u16::MAX as usize) as u16;
        let rows = dimensions.1.min(u16::MAX as usize) as u16;
        cx.spawn(async move |weak, cx| {
            cx.background_executor().timer(RESIZE_SETTLE).await;
            let settled = weak
                .read_with(cx, |view, _| {
                    view.resize_generation == generation && view.peer.alive
                })
                .unwrap_or(false);
            if settled {
                let result = shell::run(async move { host.resize(&id, cols, rows).await })
                    .await
                    .unwrap_or_else(|| Err(UNANSWERED.into()));
                if let Err(error) = result {
                    let _ = weak.update(cx, |view, cx| {
                        view.problem = Some(error.into());
                        cx.notify();
                    });
                }
            }
        })
        .detach();
        cx.notify();
    }

    pub fn close(&mut self, cx: &mut Context<Self>) {
        self.closing = true;
        let host = self.host.clone();
        let id = self.peer.id.clone();
        let work = shell::run(async move { host.close(&id).await });
        cx.spawn(async move |weak, cx| {
            let result = work.await.unwrap_or_else(|| Err(UNANSWERED.into()));
            let _ = weak.update(cx, |view, cx| {
                if let Err(error) = result {
                    view.problem = Some(error.into());
                }
                view.closing = false;
                cx.notify();
            });
        })
        .detach();
    }
}
