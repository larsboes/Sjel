//! A terminal on the Mac, drawn as the grid the program printed.
//!
//! The wire is `sjel_gpui_core::terminal`; this file is the other half. The pty's bytes are fed to
//! `alacritty_terminal`'s parser, which keeps the grid, the colours and the cursor, and GPUI draws
//! what it holds. The parser is a dependency for the same reason the pty is a dependency: a VT
//! emulator is years of escape sequences, and a wrong one shows as a screen that is almost right.
//!
//! Three decisions are worth naming.
//!
//! **Rows of runs, not a cell per element.** A row is split into runs of cells that share a
//! foreground, a background and a weight, and each run is one text element. A cell-per-element grid
//! would be a thousand elements for a shell prompt; one element per row would lose the colour that
//! `LS_COLORS` and a compiler's red are for.
//!
//! **Keys are bytes.** The pty wants what a keyboard sends, so a keystroke becomes bytes here
//! ([`key_bytes`]). That is what lets the Mac's keyboard and the phone's key row share one path into
//! the terminal.
//!
//! **The grid is the client's.** The Mac keeps a ring so a reattached window catches up, and the
//! parser keeps history for scrolling back, but a client that drew history would have to agree with
//! the Mac about offsets. This draws the visible screen, which both screens can always agree on.

use futures::StreamExt;
use gpui::prelude::*;
use gpui::{
    div, px, rgb, rgba, AnyElement, Context, FocusHandle, FontWeight, KeyDownEvent, Keystroke,
    MouseButton, Render, ScrollDelta, ScrollWheelEvent, SharedString, Window,
};
use gpui_kit::component::ActiveTheme;

use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::vte::ansi::{Color, NamedColor, Processor};

use sjel_gpui_core::shell;
use sjel_gpui_core::terminal::{Host, Position, Terminal as Peer, Update};

use crate::theme::Palette;

/// The history the parser keeps, for the scroll-back the wheel already moves through.
const HISTORY: usize = 10_000;
/// The size a view assumes before it has measured itself.
pub const START_COLS: usize = 100;
pub const START_ROWS: usize = 30;
/// A monospace cell at this size is about 7 by 15 points at the default UI font.
const FONT_SIZE: f32 = 12.0;
const LINE_HEIGHT: f32 = 1.4;
const CELL_WIDTH: f32 = FONT_SIZE * 0.6;
/// How far one notch of the wheel scrolls.
const WHEEL_LINES: i32 = 3;
/// What a phone's key row offers, and the bytes each key sends. `Ctrl-C` is here rather than a
/// modifier chip because interrupting is the one control a shell cannot be used without.
const TOUCH_KEYS: [(&str, &[u8]); 11] = [
    ("esc", b"\x1b"),
    ("tab", b"\t"),
    ("\u{2190}", b"\x1b[D"),
    ("\u{2191}", b"\x1b[A"),
    ("\u{2193}", b"\x1b[B"),
    ("\u{2192}", b"\x1b[C"),
    ("|", b"|"),
    ("~", b"~"),
    ("/", b"/"),
    ("\u{23ce}", b"\r"),
    ("ctrl-c", &[0x03]),
];

/// The sixteen ANSI tones, then their bright pair. Fixed rather than themed: a program that asks
/// for red means red, and a palette that renamed it would break the one thing these colours do.
const ANSI: [(u8, u8, u8); 16] = [
    (0x1d, 0x1f, 0x21),
    (0xcc, 0x66, 0x66),
    (0xb5, 0xbd, 0x68),
    (0xf0, 0xc6, 0x74),
    (0x81, 0xa2, 0xbe),
    (0xb2, 0x94, 0xbb),
    (0x8a, 0xbe, 0xb7),
    (0xc5, 0xc8, 0xc6),
    (0x66, 0x66, 0x66),
    (0xd5, 0x4e, 0x53),
    (0xe0, 0xdc, 0x6c),
    (0xff, 0xd0, 0x7a),
    (0x90, 0xb4, 0xd4),
    (0xc9, 0xa7, 0xd4),
    (0x9c, 0xdc, 0xd3),
    (0xf0, 0xf0, 0xf0),
];

/// What the parser reports to. A title, a colour query and a bell are events this client does not
/// act on yet, and an unhandled event is not an error.
struct Quiet;

impl EventListener for Quiet {}

/// The visible size, as the parser wants it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Size {
    cols: usize,
    rows: usize,
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.rows
    }

    fn screen_lines(&self) -> usize {
        self.rows
    }

    fn columns(&self) -> usize {
        self.cols
    }
}

/// The grid: the parser, the size the view asked for, and where the reader has scrolled to.
struct Screen {
    term: Term<Quiet>,
    parser: Processor,
    size: Size,
}

impl Screen {
    fn new(cols: usize, rows: usize) -> Self {
        let size = Size {
            cols: cols.max(1),
            rows: rows.max(1),
        };
        let config = Config {
            scrolling_history: HISTORY,
            ..Config::default()
        };
        Self {
            term: Term::new(config, &size, Quiet),
            parser: Processor::new(),
            size,
        }
    }

    /// Feed what the Mac printed. Escape sequences arrive split across frames, which is why the
    /// parser is kept rather than restarted per frame.
    fn feed(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
    }

    fn resize(&mut self, cols: usize, rows: usize) {
        let size = Size {
            cols: cols.max(1),
            rows: rows.max(1),
        };
        if size == self.size {
            return;
        }
        self.term.resize(size);
        self.size = size;
    }

    fn cols(&self) -> usize {
        self.size.cols
    }

    fn rows(&self) -> usize {
        self.size.rows
    }

    /// How many lines above the live screen the reader has scrolled.
    fn scrolled(&self) -> usize {
        self.term.grid().display_offset()
    }

    fn scroll(&mut self, lines: i32) {
        // The parser owns the offset: it clamps at both ends and repaints what it exposes.
        self.term.scroll_display(Scroll::Delta(lines));
    }

    /// The screen as the view draws it: one entry per line, each a run per shared look.
    fn lines(&self) -> Vec<ScreenLine> {
        let content = self.term.renderable_content();
        let cursor = content.cursor;
        let show_cursor = content.mode.contains(TermMode::SHOW_CURSOR) && self.scrolled() == 0;
        let mut lines: Vec<ScreenLine> = Vec::with_capacity(self.size.rows);
        for _ in 0..self.size.rows {
            lines.push(ScreenLine {
                runs: Vec::new(),
                cursor: None,
            });
        }
        for indexed in content.display_iter {
            let row = indexed.point.line.0.max(0) as usize;
            let cell = indexed.cell;
            let Some(line) = lines.get_mut(row) else {
                continue;
            };
            let style = Style {
                fg: cell.fg,
                bg: cell.bg,
                bold: cell.flags.contains(Flags::BOLD),
            };
            match line.runs.last_mut() {
                Some(run) if run.style == style => run.text.push(cell.c),
                _ => line.runs.push(Run {
                    style,
                    text: cell.c.to_string(),
                }),
            }
        }
        // A screen is full of cells a program never wrote, and a run of them is a text element
        // that draws nothing. Blank cells with the terminal's own background are dropped; a
        // painted background is kept, because that is a bar or a highlight on screen.
        for line in &mut lines {
            while let Some(last) = line.runs.last() {
                let blank = last.text.trim_end().is_empty()
                    && last.style.bg == Color::Named(NamedColor::Background);
                if !blank {
                    break;
                }
                line.runs.pop();
            }
            if let Some(last) = line.runs.last_mut() {
                if last.style.bg == Color::Named(NamedColor::Background) {
                    let trimmed = last.text.trim_end().len();
                    last.text.truncate(trimmed);
                }
            }
            line.runs.retain(|run| !run.text.is_empty());
        }
        if show_cursor {
            let row = cursor.point.line.0.max(0) as usize;
            if let Some(line) = lines.get_mut(row) {
                line.cursor = Some(cursor.point.column.0);
            }
        }
        lines
    }
}

/// One drawn line.
struct ScreenLine {
    runs: Vec<Run>,
    /// The column the cursor sits at, when it sits on this line.
    cursor: Option<usize>,
}

/// Cells that share a look, drawn as one text element.
struct Run {
    style: Style,
    text: String,
}

impl Run {
    /// The run's background (absent means the window's own) and foreground, as tones.
    fn tones(&self, palette: &Palette) -> (Option<u32>, u32) {
        (
            tone_of(self.style.bg, palette),
            tone_of(self.style.fg, palette).unwrap_or(palette.text),
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Style {
    fg: Color,
    bg: Color,
    bold: bool,
}

/// A colour as `0xRRGGBB`, or `None` for "the window's own", which the parent draws.
fn tone_of(colour: Color, palette: &Palette) -> Option<u32> {
    match colour {
        Color::Spec(rgb) => Some(hex(rgb.r, rgb.g, rgb.b)),
        Color::Indexed(index) => {
            let (r, g, b) = ansi_256(index);
            Some(hex(r, g, b))
        }
        Color::Named(NamedColor::Background) | Color::Named(NamedColor::Cursor) => None,
        Color::Named(NamedColor::Foreground) => Some(palette.text),
        Color::Named(named) => {
            let (r, g, b) = ansi_256(named_index(named));
            Some(hex(r, g, b))
        }
    }
}

fn hex(r: u8, g: u8, b: u8) -> u32 {
    (r as u32) << 16 | (g as u32) << 8 | b as u32
}

/// The xterm 256-colour palette: the sixteen ANSI tones, then a 6×6×6 cube, then 24 greys.
fn ansi_256(index: u8) -> (u8, u8, u8) {
    match index {
        0..=15 => ANSI[index as usize],
        16..=231 => {
            let cube = index - 16;
            let level = |value: u8| if value == 0 { 0 } else { 55 + value * 40 };
            (level(cube / 36), level((cube % 36) / 6), level(cube % 6))
        }
        _ => {
            let grey = 8 + (index - 232) * 10;
            (grey, grey, grey)
        }
    }
}

/// The ANSI slot a named colour has.
fn named_index(named: NamedColor) -> u8 {
    match named {
        NamedColor::Black => 0,
        NamedColor::Red => 1,
        NamedColor::Green => 2,
        NamedColor::Yellow => 3,
        NamedColor::Blue => 4,
        NamedColor::Magenta => 5,
        NamedColor::Cyan => 6,
        NamedColor::White => 7,
        NamedColor::BrightBlack | NamedColor::DimBlack => 8,
        NamedColor::BrightRed | NamedColor::DimRed => 9,
        NamedColor::BrightGreen | NamedColor::DimGreen => 10,
        NamedColor::BrightYellow | NamedColor::DimYellow => 11,
        NamedColor::BrightBlue | NamedColor::DimBlue => 12,
        NamedColor::BrightMagenta | NamedColor::DimMagenta => 13,
        NamedColor::BrightCyan | NamedColor::DimCyan => 14,
        NamedColor::BrightWhite | NamedColor::DimWhite => 15,
        _ => 7,
    }
}

/// The bytes a keystroke sends to a pty.
///
/// `None` means the view keeps the key: a `⌘`-combination belongs to the window (copy, close), and
/// a terminal that swallowed it would make the app unusable around it.
pub fn key_bytes(keystroke: &Keystroke) -> Option<Vec<u8>> {
    let key = keystroke.key.as_str();
    if keystroke.modifiers.platform {
        return None;
    }
    if keystroke.modifiers.control {
        if let Some(bytes) = control_bytes(key) {
            return Some(bytes);
        }
    }
    if keystroke.modifiers.alt {
        // Option is Meta: ESC then the character, which is how readline binds a word.
        let mut bytes = vec![0x1b];
        bytes.extend_from_slice(&plain_bytes(key, keystroke.key_char.as_deref())?);
        return Some(bytes);
    }
    match key {
        "enter" => Some(b"\r".to_vec()),
        "tab" => Some(b"\t".to_vec()),
        "backspace" => Some(vec![0x7f]),
        "escape" => Some(vec![0x1b]),
        "delete" => Some(b"\x1b[3~".to_vec()),
        "up" => Some(b"\x1b[A".to_vec()),
        "down" => Some(b"\x1b[B".to_vec()),
        "right" => Some(b"\x1b[C".to_vec()),
        "left" => Some(b"\x1b[D".to_vec()),
        "home" => Some(b"\x1b[H".to_vec()),
        "end" => Some(b"\x1b[F".to_vec()),
        "pageup" => Some(b"\x1b[5~".to_vec()),
        "pagedown" => Some(b"\x1b[6~".to_vec()),
        "insert" => Some(b"\x1b[2~".to_vec()),
        "space" => Some(b" ".to_vec()),
        _ => plain_bytes(key, keystroke.key_char.as_deref()),
    }
}

/// The character a key typed, or the key's own name when the layout has no character for it.
fn plain_bytes(key: &str, key_char: Option<&str>) -> Option<Vec<u8>> {
    let typed = key_char
        .filter(|character| !character.is_empty())
        .unwrap_or(key);
    if typed.is_empty() || typed.chars().count() > 1 {
        return None;
    }
    Some(typed.as_bytes().to_vec())
}

/// `Ctrl` plus a key, as the single byte a shell receives for it.
fn control_bytes(key: &str) -> Option<Vec<u8>> {
    if key.len() == 1 {
        let byte = key.as_bytes()[0];
        if byte.is_ascii_lowercase() {
            return Some(vec![byte - b'a' + 1]);
        }
        if byte.is_ascii_uppercase() {
            return Some(vec![byte - b'A' + 1]);
        }
    }
    let byte = match key {
        "@" | "space" => 0x00,
        "[" => 0x1b,
        "\\" => 0x1c,
        "]" => 0x1d,
        "^" => 0x1e,
        "_" => 0x1f,
        "?" => 0x7f,
        _ => return None,
    };
    Some(vec![byte])
}

/// What a request says when the runtime dropped it before the Mac answered.
const UNANSWERED: &str = "The Mac did not answer.";

/// One terminal: the peer's row, the grid it is drawn from, and where the reader has read to.
pub struct TerminalView {
    peer: Peer,
    host: Host,
    screen: Screen,
    position: Position,
    focus: FocusHandle,
    /// Why a write or the stream failed, in the words the header shows.
    problem: Option<SharedString>,
    palette: Palette,
    closing: bool,
}

impl TerminalView {
    /// A view of a terminal the capability already holds, attached from byte `from`.
    pub fn new(
        peer: Peer,
        host: Host,
        from: u64,
        cols: usize,
        rows: usize,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut screen = Screen::new(cols, rows);
        // A terminal that died while nobody was watching still says how it ended.
        screen.scroll(0);
        Self {
            screen,
            host,
            position: Position::new(from),
            focus: cx.focus_handle(),
            problem: None,
            palette,
            closing: false,
            peer,
        }
    }

    pub fn peer(&self) -> &Peer {
        &self.peer
    }

    pub fn id(&self) -> &str {
        &self.peer.id
    }

    /// The last byte offset this view has drawn, which is what a reconnect asks from.
    pub fn position(&self) -> u64 {
        self.position.offset()
    }

    /// Feed one frame of the stream into the grid. Answers whether the grid changed.
    pub fn apply(&mut self, update: Update) -> bool {
        match update {
            Update::Output { offset, data } => {
                if !self.position.accept(offset, data.len()) {
                    return false;
                }
                self.screen.feed(&data);
                true
            }
            Update::Truncated { first_offset } => {
                // The bytes this grid was built from are gone from the Mac's ring, so the grid
                // starts again rather than continuing from a hole.
                self.position.restart(first_offset);
                self.screen = Screen::new(self.screen.cols(), self.screen.rows());
                true
            }
            Update::Exit { code, signal } => {
                self.peer.alive = false;
                self.peer.exit_code = Some(code);
                self.peer.exit_signal = signal;
                true
            }
        }
    }

    /// Follow the terminal's stream until the program ends or this view goes away.
    ///
    /// A `truncated` frame ends the stream by the capability's own contract, so it is the one
    /// answer that reconnects instead of stopping: the position has already moved to the oldest
    /// byte the Mac still holds, and the next stream is asked from there. Every other ending —
    /// `exit`, or a channel that closes — stops here.
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
                let ended = !matches!(update, Update::Output { .. });
                if weak
                    .update(cx, |view, cx| {
                        view.apply(update);
                        cx.notify();
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

    /// Type bytes at the program. A refused write is reported where a stream failure is.
    pub fn send(&mut self, bytes: Vec<u8>, cx: &mut Context<Self>) {
        if bytes.is_empty() || !self.peer.alive {
            return;
        }
        let host = self.host.clone();
        let id = self.peer.id.clone();
        let work = shell::run(async move { host.input(&id, &bytes).await });
        cx.spawn(async move |weak, cx| {
            if let Err(error) = work.await.unwrap_or_else(|| Err(UNANSWERED.into())) {
                let _ = weak.update(cx, |view, cx| {
                    view.problem = Some(error.into());
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// Tell the parser and the pty the same size, so the program redraws to it.
    pub fn resize(&mut self, cols: usize, rows: usize, cx: &mut Context<Self>) {
        if (cols.max(1), rows.max(1)) == (self.screen.cols(), self.screen.rows()) {
            return;
        }
        self.screen.resize(cols, rows);
        let host = self.host.clone();
        let id = self.peer.id.clone();
        let cols = self.screen.cols() as u16;
        let rows = self.screen.rows() as u16;
        let work = shell::run(async move { host.resize(&id, cols, rows).await });
        cx.spawn(async move |_weak, _cx| {
            // A resize the Mac refuses leaves the program wrapping at the old width. Nothing here
            // can fix that, and a card in the middle of typing would be worse than the wrap.
            let _ = work.await;
        })
        .detach();
        cx.notify();
    }

    /// End the program on the Mac.
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

    fn type_keystroke(&mut self, keystroke: &Keystroke, cx: &mut Context<Self>) {
        if self.peer.alive {
            if let Some(bytes) = key_bytes(keystroke) {
                self.send(bytes, cx);
            }
        }
    }

    /// The keys a phone cannot otherwise send, in one row above the grid.
    ///
    /// A touch keyboard produces text, not `Esc` or an arrow, and a shell is unusable without
    /// both. The bytes are the same ones [`key_bytes`] sends for the same keys on a Mac, which is
    /// what keeps the two front ends one implementation.
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
            .into_any_element()
    }

    /// The wheel moves through the parser's history; the program is not told about it, because a
    /// terminal's scroll-back is the emulator's and not something a program can see.
    fn scroll_by(&mut self, lines: i32, cx: &mut Context<Self>) {
        self.screen.scroll(lines);
        cx.notify();
    }

    fn header(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let status: SharedString = match (self.problem.as_ref(), self.peer.ending()) {
            (Some(problem), _) => problem.clone(),
            (None, Some(ending)) => ending.into(),
            (None, None) => {
                if self.screen.scrolled() == 0 {
                    "live".into()
                } else {
                    format!("{} lines back", self.screen.scrolled()).into()
                }
            }
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The appearance follows the window, so a terminal in light mode is not a black rectangle.
        self.palette = Palette::for_appearance(window.appearance());
        let palette = self.palette;
        let mono = cx.theme().mono_font_family.clone();
        let line_height = px(FONT_SIZE * LINE_HEIGHT);
        let lines = self.screen.lines();
        let header = self.header(palette, cx);

        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .bg(rgba(palette.panel()))
            .text_color(rgb(palette.text))
            .child(header)
            .child(
                div()
                    .id("terminal-grid")
                    .flex_1()
                    .overflow_y_scroll()
                    .track_focus(&self.focus)
                    .bg(rgba(palette.surface))
                    .rounded(px(8.))
                    .border_1()
                    .border_color(rgba(palette.border))
                    .p_2()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|view, _event, window, cx| {
                            window.focus(&view.focus, cx);
                        }),
                    )
                    .on_key_down(cx.listener(|view, event: &KeyDownEvent, _window, cx| {
                        if !event.is_held {
                            view.type_keystroke(&event.keystroke, cx);
                        }
                    }))
                    .on_scroll_wheel(cx.listener(|view, event: &ScrollWheelEvent, _window, cx| {
                        let steps = match event.delta {
                            ScrollDelta::Lines(delta) => delta.y,
                            ScrollDelta::Pixels(delta) => delta.y / px(12.),
                        };
                        let lines = (steps.signum() * WHEEL_LINES as f32).round() as i32;
                        if lines != 0 {
                            view.scroll_by(lines, cx);
                        }
                    }))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .children(lines.into_iter().map(|line| {
                                let cursor = line.cursor;
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .h(line_height)
                                    .whitespace_nowrap()
                                    .children(runs_of(line, cursor, &palette, mono.clone()))
                            })),
                    ),
            )
            .into_any_element()
    }
}

/// One line's runs as elements, with the cursor's cell drawn inverted where it sits.
fn runs_of(
    line: ScreenLine,
    cursor: Option<usize>,
    palette: &Palette,
    mono: SharedString,
) -> Vec<AnyElement> {
    let mut elements: Vec<AnyElement> = Vec::new();
    let mut column = 0usize;
    for run in line.runs {
        let (bg, fg) = run.tones(palette);
        let characters: Vec<char> = run.text.chars().collect();
        let end = column + characters.len();
        match cursor {
            Some(at) if at >= column && at < end => {
                let offset = at - column;
                let before: String = characters[..offset].iter().collect();
                let under: String = characters[offset..offset + 1].iter().collect();
                let after: String = characters[offset + 1..].iter().collect();
                if !before.is_empty() {
                    elements.push(cell(before, bg, fg, run.style.bold, &mono));
                }
                // The cursor is the cell's own look inverted, which is what every terminal does.
                elements.push(
                    div()
                        .flex_none()
                        .h(px(FONT_SIZE * LINE_HEIGHT))
                        .bg(rgb(fg))
                        .text_color(rgb(bg.unwrap_or(palette.surface)))
                        .font_family(mono.clone())
                        .text_size(px(FONT_SIZE))
                        .child(SharedString::from(under))
                        .into_any_element(),
                );
                if !after.is_empty() {
                    elements.push(cell(after, bg, fg, run.style.bold, &mono));
                }
            }
            _ => elements.push(cell(run.text, bg, fg, run.style.bold, &mono)),
        }
        column = end;
    }
    // A cursor past the last character of the line still needs somewhere to be drawn.
    if let Some(at) = cursor.filter(|at| *at >= column) {
        let padding = " ".repeat(at.saturating_sub(column));
        if !padding.is_empty() {
            elements.push(cell(padding, None, palette.text, false, &mono));
        }
        elements.push(
            div()
                .flex_none()
                .w(px(CELL_WIDTH))
                .h(px(FONT_SIZE * LINE_HEIGHT))
                .bg(rgb(palette.text))
                .into_any_element(),
        );
    }
    elements
}

fn cell(text: String, bg: Option<u32>, fg: u32, bold: bool, mono: &SharedString) -> AnyElement {
    let mut element = div()
        .flex_none()
        .font_family(mono.clone())
        .text_size(px(FONT_SIZE))
        .text_color(rgb(fg))
        .child(SharedString::from(text));
    if let Some(bg) = bg {
        element = element.bg(rgb(bg));
    }
    if bold {
        element = element.font_weight(FontWeight::BOLD);
    }
    element.into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::Modifiers;

    fn typed(key: &str, key_char: Option<&str>) -> Option<Vec<u8>> {
        key_bytes(&Keystroke {
            key: key.to_owned(),
            key_char: key_char.map(str::to_owned),
            modifiers: Modifiers::default(),
        })
    }

    fn with(modifiers: Modifiers, key: &str, key_char: Option<&str>) -> Option<Vec<u8>> {
        key_bytes(&Keystroke {
            key: key.to_owned(),
            key_char: key_char.map(str::to_owned),
            modifiers,
        })
    }

    fn text_of(line: &ScreenLine) -> String {
        line.runs.iter().map(|run| run.text.as_str()).collect()
    }

    #[test]
    fn a_letter_is_the_letter_it_printed() {
        assert_eq!(typed("a", Some("a")), Some(b"a".to_vec()));
        assert_eq!(typed("space", Some(" ")), Some(b" ".to_vec()));
        // A key with no character of its own — a dead key — sends nothing rather than a guess.
        assert_eq!(typed("dead_acute", None), None);
    }

    #[test]
    fn the_editing_keys_are_the_bytes_a_pty_expects() {
        assert_eq!(typed("enter", None), Some(b"\r".to_vec()));
        assert_eq!(typed("tab", None), Some(b"\t".to_vec()));
        assert_eq!(typed("backspace", None), Some(vec![0x7f]));
        assert_eq!(typed("escape", None), Some(vec![0x1b]));
        assert_eq!(typed("up", None), Some(b"\x1b[A".to_vec()));
        assert_eq!(typed("down", None), Some(b"\x1b[B".to_vec()));
        assert_eq!(typed("delete", None), Some(b"\x1b[3~".to_vec()));
        assert_eq!(typed("home", None), Some(b"\x1b[H".to_vec()));
        assert_eq!(typed("pageup", None), Some(b"\x1b[5~".to_vec()));
    }

    #[test]
    fn control_and_option_are_what_a_shell_reads() {
        let control = Modifiers {
            control: true,
            ..Modifiers::default()
        };
        assert_eq!(with(control, "c", None), Some(vec![0x03]));
        assert_eq!(with(control, "d", None), Some(vec![0x04]));
        assert_eq!(with(control, "l", Some("l")), Some(vec![0x0c]));
        assert_eq!(with(control, "[", None), Some(vec![0x1b]));

        let alt = Modifiers {
            alt: true,
            ..Modifiers::default()
        };
        // Option-b is "back one word" to readline: ESC then the letter.
        assert_eq!(with(alt, "b", Some("b")), Some(b"\x1bb".to_vec()));
    }

    #[test]
    fn a_window_shortcut_stays_with_the_window() {
        let platform = Modifiers {
            platform: true,
            ..Modifiers::default()
        };
        // ⌘C copies; a terminal that swallowed it would make the app unusable around it.
        assert_eq!(with(platform, "c", Some("c")), None);
    }

    #[test]
    fn the_touch_key_row_sends_what_a_keyboard_would() {
        let bytes = |label: &str| {
            TOUCH_KEYS
                .iter()
                .find(|(name, _)| *name == label)
                .map(|(_, bytes)| *bytes)
                .unwrap_or(&[][..])
        };
        assert_eq!(bytes("esc"), &[0x1b]);
        assert_eq!(bytes("tab"), &[0x09]);
        assert_eq!(bytes("ctrl-c"), &[0x03]);
        assert_eq!(bytes("\u{2191}"), &b"\x1b[A"[..]);
        // The row offers the three characters a phone keyboard hides.
        for character in ["|", "~", "/"] {
            assert_eq!(bytes(character), character.as_bytes());
        }
    }

    #[test]
    fn the_ansi_palette_is_the_tones_a_program_asks_for() {
        assert_eq!(ansi_256(1), ANSI[1]);
        assert_eq!(ansi_256(9), ANSI[9]);
        // The cube: 196 is full red, 16 is black, 231 white.
        assert_eq!(ansi_256(196), (255, 0, 0));
        assert_eq!(ansi_256(16), (0, 0, 0));
        assert_eq!(ansi_256(231), (255, 255, 255));
        // The greys run from 8 to 238.
        assert_eq!(ansi_256(232), (8, 8, 8));
        assert_eq!(ansi_256(255), (238, 238, 238));
    }

    #[test]
    fn the_screen_keeps_what_the_program_printed() {
        let mut screen = Screen::new(20, 4);
        screen.feed(b"hello");
        let lines = screen.lines();
        assert_eq!(lines.len(), 4);
        assert_eq!(text_of(&lines[0]), "hello");
    }

    #[test]
    fn an_escape_sequence_split_across_frames_still_lands() {
        let mut screen = Screen::new(20, 3);
        // A red x, split mid-sequence: the parser is kept between frames.
        screen.feed(b"\x1b[3");
        screen.feed(b"1mx");
        let lines = screen.lines();
        let red = lines[0]
            .runs
            .iter()
            .find(|run| run.text.contains('x'))
            .expect("the x was drawn");
        assert_eq!(
            red.style.fg,
            Color::Named(NamedColor::Red),
            "the colour was lost when the sequence was split"
        );
    }

    #[test]
    fn a_resize_reaches_the_parser_and_keeps_its_content() {
        let mut screen = Screen::new(40, 10);
        screen.feed(b"one\r\ntwo");
        screen.resize(20, 5);
        assert_eq!((screen.cols(), screen.rows()), (20, 5));
        assert_eq!(text_of(&screen.lines()[0]), "one");
    }

    #[test]
    fn the_cursor_is_on_the_line_the_program_stopped_at() {
        let mut screen = Screen::new(20, 5);
        screen.feed(b"first\r\nsecond");
        let lines = screen.lines();
        assert_eq!(lines[1].cursor, Some(6));
        assert_eq!(lines[0].cursor, None);
    }

    #[test]
    fn the_wheel_moves_through_history_and_stops_at_the_live_screen() {
        let mut screen = Screen::new(20, 3);
        for index in 0..10 {
            screen.feed(format!("line{index}\r\n").as_bytes());
        }
        assert_eq!(screen.scrolled(), 0);
        screen.scroll(WHEEL_LINES);
        assert_eq!(screen.scrolled(), WHEEL_LINES as usize);
        // The live screen is where a reader cannot scroll past.
        screen.scroll(10_000);
        assert_eq!(screen.scrolled(), screen.term.grid().history_size());
    }

    #[test]
    fn the_cursor_cell_is_inverted_and_the_rest_of_the_line_survives() {
        let mut screen = Screen::new(20, 3);
        screen.feed(b"abc");
        let lines = screen.lines();
        let elements = runs_of(
            ScreenLine {
                runs: lines[0]
                    .runs
                    .iter()
                    .map(|run| Run {
                        style: run.style,
                        text: run.text.clone(),
                    })
                    .collect(),
                cursor: Some(1),
            },
            Some(1),
            &Palette::dark(),
            SharedString::from("monospace"),
        );
        // "a", the inverted "b", then "c": the cursor splits the run rather than replacing it.
        assert_eq!(elements.len(), 3);
    }
}
