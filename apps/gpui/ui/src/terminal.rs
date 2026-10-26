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
    actions, canvas, div, fill, font, point, px, rgb, rgba, AnyElement, Bounds, ClipboardItem,
    ContentMask, Context, EventEmitter, FocusHandle, Font, FontWeight, KeyBinding, KeyDownEvent,
    Keystroke, MouseButton, MouseDownEvent, MouseMoveEvent, PinchEvent, Pixels, Point, Render,
    ScrollDelta, ScrollWheelEvent, SharedString, TextAlign, TextRun, TouchPhase, Window,
};
use gpui_kit::component::ActiveTheme;

use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point as GridPoint, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::vte::ansi::{Color, NamedColor, Processor};

use sjel_gpui_core::shell;
use sjel_gpui_core::terminal::{Host, Position, Terminal as Peer, Update, MAX_INPUT_BYTES};

use crate::theme::{Palette, TerminalFont, TerminalScheme, MONOKAI};

mod deck;

pub use deck::TerminalDeck;

actions!(
    terminal,
    [
        /// Copy the selection.
        Copy,
        /// Paste into the terminal.
        Paste,
        /// Select the whole scrollback.
        SelectAll,
        /// Start a new terminal: choose the program and the folder.
        NewTerminal,
        /// Show the next terminal tab.
        NextTab,
        /// Show the previous terminal tab.
        PrevTab,
    ]
);

/// The keys the terminal takes from the window while its grid has focus. `Terminal` is deeper than
/// any page context, so these win over a binding of the same keys further out.
pub fn keys() -> [KeyBinding; 8] {
    [
        KeyBinding::new("cmd-c", Copy, Some("Terminal")),
        KeyBinding::new("cmd-v", Paste, Some("Terminal")),
        KeyBinding::new("cmd-a", SelectAll, Some("Terminal")),
        KeyBinding::new("cmd-t", NewTerminal, Some("Terminal")),
        // The keys that step a window's tabs in Safari and Terminal step the terminal's here.
        KeyBinding::new("shift-cmd-]", NextTab, Some("Terminal")),
        KeyBinding::new("shift-cmd-[", PrevTab, Some("Terminal")),
        KeyBinding::new("ctrl-tab", NextTab, Some("Terminal")),
        KeyBinding::new("ctrl-shift-tab", PrevTab, Some("Terminal")),
    ]
}

/// The history the parser keeps, for the scroll-back the wheel already moves through.
const HISTORY: usize = 10_000;
/// A monospace cell at this size is about 7 by 15 points at the default UI font.
const LINE_HEIGHT: f32 = 1.4;
/// The size a view draws at before its host hands it the Appearance setting.
const DEFAULT_FONT_SIZE: f32 = 13.0;
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

    /// The grid cell at a visible `col` and `row`, in the parser's coordinates, which count
    /// history lines as negative.
    fn point_at(&self, col: usize, row: usize) -> GridPoint {
        let row = row.min(self.size.rows - 1) as i32;
        GridPoint::new(
            Line(row - self.scrolled() as i32),
            Column(col.min(self.size.cols - 1)),
        )
    }

    fn select(&mut self, kind: SelectionType, at: GridPoint, side: Side) {
        self.term.selection = Some(Selection::new(kind, at, side));
    }

    fn extend_selection(&mut self, at: GridPoint, side: Side) {
        if let Some(selection) = self.term.selection.as_mut() {
            selection.update(at, side);
        }
    }

    /// Every line the parser still holds, history included.
    fn select_all(&mut self) {
        let top = GridPoint::new(Line(-(self.term.grid().history_size() as i32)), Column(0));
        let bottom = GridPoint::new(Line(self.size.rows as i32 - 1), Column(self.size.cols - 1));
        let mut selection = Selection::new(SelectionType::Lines, top, Side::Left);
        selection.update(bottom, Side::Right);
        self.term.selection = Some(selection);
    }

    /// A click without a drag selects nothing, so it leaves no highlight behind.
    fn drop_empty_selection(&mut self) {
        if self
            .term
            .selection
            .as_ref()
            .is_some_and(Selection::is_empty)
        {
            self.term.selection = None;
        }
    }

    fn clear_selection(&mut self) {
        self.term.selection = None;
    }

    fn selected_text(&self) -> Option<String> {
        self.term
            .selection_to_string()
            .filter(|text| !text.is_empty())
    }

    fn bracketed_paste(&self) -> bool {
        self.term.mode().contains(TermMode::BRACKETED_PASTE)
    }

    /// The screen as the view draws it: one entry per line, each a run per shared look.
    fn lines(&self) -> Vec<ScreenLine> {
        let content = self.term.renderable_content();
        let cursor = content.cursor;
        let selection = content.selection;
        // The iterator counts history lines as negative; the view's rows start at the top of what
        // is on screen, so the scroll offset moves one into the other.
        let offset = content.display_offset as i32;
        let show_cursor = content.mode.contains(TermMode::SHOW_CURSOR) && self.scrolled() == 0;
        let mut lines: Vec<ScreenLine> = Vec::with_capacity(self.size.rows);
        for _ in 0..self.size.rows {
            lines.push(ScreenLine {
                runs: Vec::new(),
                cursor: None,
            });
        }
        for indexed in content.display_iter {
            let Ok(row) = usize::try_from(indexed.point.line.0 + offset) else {
                continue;
            };
            let cell = indexed.cell;
            let Some(line) = lines.get_mut(row) else {
                continue;
            };
            let style = Style {
                fg: cell.fg,
                bg: cell.bg,
                bold: cell.flags.contains(Flags::BOLD),
                reverse: cell.flags.contains(Flags::INVERSE),
                selected: selection.is_some_and(|range| range.contains(indexed.point)),
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
                    && last.style.bg == Color::Named(NamedColor::Background)
                    && !last.style.reverse
                    && !last.style.selected;
                if !blank {
                    break;
                }
                line.runs.pop();
            }
            if let Some(last) = line.runs.last_mut() {
                if last.style.bg == Color::Named(NamedColor::Background)
                    && !last.style.reverse
                    && !last.style.selected
                {
                    let trimmed = last.text.trim_end().len();
                    last.text.truncate(trimmed);
                }
            }
            line.runs.retain(|run| !run.text.is_empty());
        }
        if show_cursor {
            let row = (cursor.point.line.0 + offset).max(0) as usize;
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

/// Cells that share a look. A wide character's second cell is a space in the run, so every
/// character is one cell and the run's length is its width.
struct Run {
    style: Style,
    text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Style {
    fg: Color,
    bg: Color,
    bold: bool,
    /// Foreground and background swapped: a program's reverse video.
    reverse: bool,
    selected: bool,
}

impl Style {
    /// The background (absent means the grid's own) and foreground, as `0xRRGGBB`.
    fn tones(&self, scheme: &TerminalScheme) -> (Option<u32>, u32) {
        let bg = tone_of(self.bg, scheme);
        let fg = tone_of(self.fg, scheme).unwrap_or(scheme.foreground);
        let (bg, fg) = if self.reverse {
            (Some(fg), bg.unwrap_or(scheme.background))
        } else {
            (bg, fg)
        };
        if self.selected {
            (Some(scheme.selection), fg)
        } else {
            (bg, fg)
        }
    }
}

/// A colour as `0xRRGGBB`, or `None` for the terminal's own background, which the grid draws.
fn tone_of(colour: Color, scheme: &TerminalScheme) -> Option<u32> {
    match colour {
        Color::Spec(rgb) => Some(hex(rgb.r, rgb.g, rgb.b)),
        Color::Indexed(index) => Some(ansi_256(index, scheme)),
        Color::Named(NamedColor::Background) => None,
        Color::Named(NamedColor::Cursor) => Some(scheme.cursor),
        Color::Named(
            NamedColor::Foreground | NamedColor::BrightForeground | NamedColor::DimForeground,
        ) => Some(scheme.foreground),
        Color::Named(named) => Some(scheme.ansi[named_index(named) as usize]),
    }
}

fn hex(r: u8, g: u8, b: u8) -> u32 {
    (r as u32) << 16 | (g as u32) << 8 | b as u32
}

/// The xterm 256-colour palette: the scheme's sixteen tones, then a 6×6×6 cube, then 24 greys.
fn ansi_256(index: u8, scheme: &TerminalScheme) -> u32 {
    match index {
        0..=15 => scheme.ansi[index as usize],
        16..=231 => {
            let cube = index - 16;
            let level = |value: u8| if value == 0 { 0 } else { 55 + value * 40 };
            hex(level(cube / 36), level((cube % 36) / 6), level(cube % 6))
        }
        _ => {
            let grey = 8 + (index - 232) * 10;
            hex(grey, grey, grey)
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

/// One stretch of a line in one look, at the column it starts: what the grid paints.
#[derive(Debug, PartialEq, Eq)]
struct Span {
    column: usize,
    text: String,
    bg: Option<u32>,
    fg: u32,
    bold: bool,
}

/// A line as spans. The cursor splits the run it sits in and draws its cell inverted, which is
/// what every terminal does; a cursor past the end of the text gets a cell of its own.
fn spans(line: ScreenLine, scheme: &TerminalScheme) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut column = 0;
    let cursor = line.cursor;
    for run in line.runs {
        let (bg, fg) = run.style.tones(scheme);
        let characters: Vec<char> = run.text.chars().collect();
        let end = column + characters.len();
        let mut push = |from: usize, to: usize, bg: Option<u32>, fg: u32| {
            if from < to {
                spans.push(Span {
                    column: column + from,
                    text: characters[from..to].iter().collect(),
                    bg,
                    fg,
                    bold: run.style.bold,
                });
            }
        };
        match cursor {
            Some(at) if at >= column && at < end => {
                let offset = at - column;
                push(0, offset, bg, fg);
                push(
                    offset,
                    offset + 1,
                    Some(scheme.cursor),
                    bg.unwrap_or(scheme.background),
                );
                push(offset + 1, characters.len(), bg, fg);
            }
            _ => push(0, characters.len(), bg, fg),
        }
        column = end;
    }
    if let Some(at) = cursor.filter(|at| *at >= column) {
        spans.push(Span {
            column: at,
            text: " ".to_owned(),
            bg: Some(scheme.cursor),
            fg: scheme.background,
            bold: false,
        });
    }
    spans
}

/// The font the grid draws in, and the cell it measures.
#[derive(Clone)]
struct Metrics {
    family: SharedString,
    size: Pixels,
    line: Pixels,
}

/// Paints the grid: one quad per painted background and one shaped line per row. The text system
/// places every glyph on the cell's advance (`force_width`), so an icon from a fallback font or a
/// wide character keeps the columns after it where they belong.
fn paint_grid(
    bounds: Bounds<Pixels>,
    rows: Vec<Vec<Span>>,
    metrics: &Metrics,
    cell: Pixels,
    window: &mut Window,
    cx: &mut gpui::App,
) {
    let regular = font(metrics.family.clone());
    let bold = Font {
        weight: FontWeight::BOLD,
        ..regular.clone()
    };
    window.with_content_mask(Some(ContentMask { bounds }), |window| {
        for (row, spans) in rows.into_iter().enumerate() {
            let y = bounds.origin.y + metrics.line * row as f32;
            let mut text = String::new();
            let mut runs = Vec::with_capacity(spans.len());
            let mut column = 0;
            for span in &spans {
                let width = span.text.chars().count();
                if let Some(bg) = span.bg {
                    let origin = point(bounds.origin.x + cell * span.column as f32, y);
                    let size = gpui::size(cell * width as f32, metrics.line);
                    window.paint_quad(fill(Bounds::new(origin, size), rgb(bg)));
                }
                // Spans are contiguous from column 0 except the cursor's own cell past the end,
                // which the gap before it is padded to.
                if span.column > column {
                    let gap = " ".repeat(span.column - column);
                    runs.push(text_run(gap.len(), &regular, span.fg));
                    text.push_str(&gap);
                }
                runs.push(text_run(
                    span.text.len(),
                    if span.bold { &bold } else { &regular },
                    span.fg,
                ));
                text.push_str(&span.text);
                column = span.column + width;
            }
            if text.trim().is_empty() {
                continue;
            }
            let shaped = window.text_system().shape_line(
                SharedString::from(text),
                metrics.size,
                &runs,
                Some(cell),
            );
            let _ = shaped.paint(
                point(bounds.origin.x, y),
                metrics.line,
                TextAlign::Left,
                None,
                window,
                cx,
            );
        }
    });
}

fn text_run(len: usize, font: &Font, fg: u32) -> TextRun {
    TextRun {
        len,
        font: font.clone(),
        color: rgb(fg).into(),
        background_color: None,
        underline: None,
        strikethrough: None,
    }
}

/// What a paste sends. Line breaks become Return, which is what a typed newline is. A program that
/// turned on bracketed paste gets the text between the markers, minus any end marker inside it, so
/// pasted text cannot end the paste early and run as typed commands.
fn paste_bytes(text: &str, bracketed: bool) -> Vec<u8> {
    let text = text.replace("\r\n", "\r").replace('\n', "\r");
    if bracketed {
        let inner = text.replace("\x1b[201~", "");
        format!("\x1b[200~{inner}\x1b[201~").into_bytes()
    } else {
        text.into_bytes()
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

/// What the view tells its owner. A tap on the grid of a touch view asks for the software
/// keyboard, which belongs to the platform and not to this crate.
pub enum TerminalEvent {
    Editing(bool),
    /// A pinch asked for this many font sizes larger (or smaller, below zero). The size is an
    /// Appearance setting, so the host applies and saves it.
    Zoom(i32),
}

/// How long the grid's size holds still before the pty is told it.
const RESIZE_SETTLE: std::time::Duration = std::time::Duration::from_millis(150);

/// How far a trackpad pinch travels before it steps the font size once.
const PINCH_STEP: f32 = 0.15;

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
    /// A phone's view: the key row sits above the grid, and a tap asks for the keyboard.
    touch: bool,
    /// Bytes typed while a write is in flight. One write at a time keeps keystrokes in order.
    outgoing: Vec<u8>,
    sending: bool,
    /// Where the grid's first cell is drawn and how big a cell is, as last measured, so a pointer
    /// position becomes a cell.
    origin: Point<Pixels>,
    cell: gpui::Size<Pixels>,
    /// The font the grid draws in, resolved from the Appearance choice when that changes.
    metrics: Metrics,
    font_choice: Option<(TerminalFont, u8)>,
    /// True from a press on the grid until its release: a move extends the selection.
    selecting: bool,
    /// How far the current pinch has travelled since it last stepped the font size.
    pinch: f32,
    /// Counts resizes, so a pty resize goes out only for the last one in a burst.
    resize_generation: u64,
}

impl EventEmitter<TerminalEvent> for TerminalView {}

impl gpui::Focusable for TerminalView {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
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
            touch: false,
            outgoing: Vec::new(),
            sending: false,
            origin: Point::default(),
            cell: gpui::size(
                px(DEFAULT_FONT_SIZE * 0.6),
                px(DEFAULT_FONT_SIZE * LINE_HEIGHT),
            ),
            metrics: Metrics {
                family: SharedString::from("Menlo"),
                size: px(DEFAULT_FONT_SIZE),
                line: px(DEFAULT_FONT_SIZE * LINE_HEIGHT),
            },
            font_choice: None,
            selecting: false,
            pinch: 0.,
            resize_generation: 0,
            peer,
        }
    }

    /// The app's palette and the terminal's font, Appearance settings applied. The host hands
    /// them in on every frame, so a change in Settings reaches a terminal that is already open.
    /// The font is resolved against the installed families only when the choice changes.
    pub fn set_style(&mut self, palette: Palette, font: TerminalFont, size: u8, cx: &gpui::App) {
        self.palette = palette;
        if self.font_choice == Some((font, size)) {
            return;
        }
        self.font_choice = Some((font, size));
        let families = TerminalFont::families(cx);
        let theme_mono = cx.theme().mono_font_family.clone();
        let size = px(size as f32);
        self.metrics = Metrics {
            family: font.resolve(families, &theme_mono).into(),
            size,
            line: size * LINE_HEIGHT,
        };
    }

    /// The same view for a touch screen.
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
        // Typing answers the live screen, the way every terminal behaves.
        self.screen.clear_selection();
        self.screen.term.scroll_display(Scroll::Bottom);
        self.outgoing.extend_from_slice(&bytes);
        if self.sending {
            return;
        }
        self.sending = true;
        let host = self.host.clone();
        let id = self.peer.id.clone();
        // Concurrent writes can reach the pty out of order, so one task drains the queue: what
        // was typed during a write goes out as the next write.
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

    /// The cell under a pointer position, and which half of it the pointer is on.
    fn cell_at(&self, position: Point<Pixels>) -> (GridPoint, Side) {
        let x = f32::from(position.x - self.origin.x).max(0.);
        let y = f32::from(position.y - self.origin.y).max(0.);
        let columns = x / f32::from(self.cell.width);
        let side = if columns.fract() < 0.5 {
            Side::Left
        } else {
            Side::Right
        };
        let row = (y / f32::from(self.cell.height)) as usize;
        (self.screen.point_at(columns as usize, row), side)
    }

    fn press(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        let (at, side) = self.cell_at(event.position);
        if event.modifiers.shift && self.screen.term.selection.is_some() {
            self.screen.extend_selection(at, side);
        } else {
            let kind = match event.click_count {
                0 | 1 => SelectionType::Simple,
                2 => SelectionType::Semantic,
                _ => SelectionType::Lines,
            };
            self.screen.select(kind, at, side);
        }
        self.selecting = true;
        cx.notify();
    }

    fn drag(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        if !self.selecting || event.pressed_button != Some(MouseButton::Left) {
            return;
        }
        let (at, side) = self.cell_at(event.position);
        self.screen.extend_selection(at, side);
        cx.notify();
    }

    fn release(&mut self, cx: &mut Context<Self>) {
        if self.selecting {
            self.selecting = false;
            self.screen.drop_empty_selection();
            cx.notify();
        }
    }

    pub fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = self.screen.selected_text() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    pub fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        let bytes = paste_bytes(&text, self.screen.bracketed_paste());
        self.send(bytes, cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.screen.select_all();
        cx.notify();
    }

    /// Tell the parser and the pty the same size, so the program redraws to it.
    pub fn resize(&mut self, cols: usize, rows: usize, cx: &mut Context<Self>) {
        if (cols.max(1), rows.max(1)) == (self.screen.cols(), self.screen.rows()) {
            return;
        }
        self.screen.resize(cols, rows);
        // The grid follows at once; the pty hears only the size the grid settles on. A drag or
        // a sliding edge resizes every frame, and each size the shell hears is a prompt it
        // redraws for a width the grid has already left.
        self.resize_generation += 1;
        let generation = self.resize_generation;
        let host = self.host.clone();
        let id = self.peer.id.clone();
        let cols = self.screen.cols() as u16;
        let rows = self.screen.rows() as u16;
        cx.spawn(async move |weak, cx| {
            cx.background_executor().timer(RESIZE_SETTLE).await;
            let settled = weak
                .read_with(cx, |view, _| view.resize_generation == generation)
                .unwrap_or(false);
            if settled {
                // A resize the Mac refuses leaves the program wrapping at the old width. Nothing
                // here can fix that, and a card in the middle of typing would be worse than it.
                let _ = shell::run(async move { host.resize(&id, cols, rows).await }).await;
            }
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
                    .when(self.screen.selected_text().is_none(), |this| {
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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self.palette;
        let scheme = MONOKAI;
        let rows: Vec<Vec<Span>> = self
            .screen
            .lines()
            .into_iter()
            .map(|line| spans(line, &scheme))
            .collect();
        let metrics = self.metrics.clone();
        let view = cx.entity();
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
                    .track_focus(&self.focus)
                    .bg(rgba(palette.terminal_surface))
                    .rounded(px(8.))
                    .border_1()
                    .border_color(rgba(palette.border))
                    .p_2()
                    .on_action(cx.listener(Self::copy))
                    .on_action(cx.listener(Self::paste))
                    .on_action(cx.listener(Self::select_all))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|view, event: &MouseDownEvent, window, cx| {
                            window.focus(&view.focus, cx);
                            if view.touch {
                                cx.emit(TerminalEvent::Editing(true));
                            }
                            view.press(event, cx);
                        }),
                    )
                    .on_mouse_move(cx.listener(|view, event: &MouseMoveEvent, _window, cx| {
                        view.drag(event, cx);
                    }))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|view, _event, _window, cx| view.release(cx)),
                    )
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(|view, _event, _window, cx| view.release(cx)),
                    )
                    .on_key_down(cx.listener(|view, event: &KeyDownEvent, _window, cx| {
                        if !event.is_held {
                            view.type_keystroke(&event.keystroke, cx);
                        }
                    }))
                    .on_pinch(cx.listener(|view, event: &PinchEvent, _window, cx| {
                        view.pinch += event.delta;
                        if view.pinch.abs() >= PINCH_STEP {
                            cx.emit(TerminalEvent::Zoom(view.pinch.signum() as i32));
                            view.pinch = 0.;
                        }
                        if event.phase == TouchPhase::Ended {
                            view.pinch = 0.;
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
                        canvas(
                            {
                                let metrics = metrics.clone();
                                move |bounds, window, app| {
                                    // The cell is the font's own advance, so columns land where
                                    // the glyphs do. The pty is resized to what fits.
                                    let font_id = window
                                        .text_system()
                                        .resolve_font(&font(metrics.family.clone()));
                                    let cell = window
                                        .text_system()
                                        .advance(font_id, metrics.size, 'm')
                                        .map(|advance| advance.width)
                                        .unwrap_or(metrics.size * 0.6);
                                    let cols = (bounds.size.width / cell).floor().max(1.) as usize;
                                    let rows = (bounds.size.height / metrics.line).floor().max(1.)
                                        as usize;
                                    let origin = bounds.origin;
                                    let line = metrics.line;
                                    app.defer(move |app| {
                                        view.update(app, |view, cx| {
                                            view.origin = origin;
                                            view.cell = gpui::size(cell, line);
                                            view.resize(cols, rows, cx);
                                        });
                                    });
                                    cell
                                }
                            },
                            move |bounds, cell, window, app| {
                                paint_grid(bounds, rows, &metrics, cell, window, app);
                            },
                        )
                        .size_full(),
                    ),
            )
            .into_any_element()
    }
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
    fn a_dragged_selection_copies_the_cells_it_covers() {
        let mut screen = Screen::new(20, 3);
        screen.feed(b"hello world");
        let start = screen.point_at(0, 0);
        let end = screen.point_at(4, 0);
        screen.select(SelectionType::Simple, start, Side::Left);
        screen.extend_selection(end, Side::Right);
        assert_eq!(screen.selected_text().as_deref(), Some("hello"));
        // The selected cells draw on the scheme's selection colour, the rest of the line does not.
        let mut lines = screen.lines();
        let spans = spans(lines.remove(0), &MONOKAI);
        assert_eq!(spans[0].text, "hello");
        assert_eq!(spans[0].bg, Some(MONOKAI.selection));
        assert_eq!(spans[1].bg, None);
    }

    #[test]
    fn a_wide_character_keeps_its_second_cell() {
        // The grid draws every character on one cell's advance, so the wide character's second
        // cell must stay in the text as a space or everything after it shifts left by one.
        let mut screen = Screen::new(20, 3);
        screen.feed("漢a".as_bytes());
        let runs = &screen.lines()[0].runs;
        let text: String = runs.iter().map(|run| run.text.as_str()).collect();
        assert_eq!(text, "漢 a");
    }

    #[test]
    fn a_click_without_a_drag_leaves_nothing_selected() {
        let mut screen = Screen::new(20, 3);
        screen.feed(b"hello");
        let at = screen.point_at(2, 0);
        screen.select(SelectionType::Simple, at, Side::Left);
        screen.drop_empty_selection();
        assert_eq!(screen.selected_text(), None);
    }

    #[test]
    fn scrolled_back_history_draws_on_its_own_rows() {
        let mut screen = Screen::new(10, 3);
        screen.feed(b"1\r\n2\r\n3\r\n4\r\n5");
        screen.scroll(2);
        let lines = screen.lines();
        let shown: Vec<String> = lines.iter().map(|line| text_of(line)).collect();
        assert_eq!(shown, ["1", "2", "3"]);
    }

    #[test]
    fn a_paste_types_returns_and_cannot_end_a_bracketed_paste_early() {
        assert_eq!(paste_bytes("ls\nexit\r\n", false), b"ls\rexit\r");
        assert_eq!(
            paste_bytes("a\x1b[201~rm -rf", true),
            b"\x1b[200~arm -rf\x1b[201~"
        );
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
        // The first sixteen are the scheme's: Monokai's red is its pink.
        assert_eq!(ansi_256(1, &MONOKAI), 0xf92672);
        assert_eq!(
            tone_of(Color::Named(NamedColor::Red), &MONOKAI),
            Some(0xf92672)
        );
        assert_eq!(
            tone_of(Color::Named(NamedColor::Foreground), &MONOKAI),
            Some(MONOKAI.foreground)
        );
        assert_eq!(
            tone_of(Color::Named(NamedColor::Background), &MONOKAI),
            None
        );
        // The cube: 196 is full red, 16 is black, 231 white.
        assert_eq!(ansi_256(196, &MONOKAI), 0xff0000);
        assert_eq!(ansi_256(16, &MONOKAI), 0x000000);
        assert_eq!(ansi_256(231, &MONOKAI), 0xffffff);
        // The greys run from 8 to 238.
        assert_eq!(ansi_256(232, &MONOKAI), 0x080808);
        assert_eq!(ansi_256(255, &MONOKAI), 0xeeeeee);
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
        let mut lines = screen.lines();
        let mut line = lines.remove(0);
        line.cursor = Some(1);
        let spans = spans(line, &MONOKAI);
        // "a", the inverted "b", then "c": the cursor splits the run rather than replacing it.
        let texts: Vec<&str> = spans.iter().map(|span| span.text.as_str()).collect();
        assert_eq!(texts, ["a", "b", "c"]);
        assert_eq!(spans[1].bg, Some(MONOKAI.cursor));
        assert_eq!(spans[1].fg, MONOKAI.background);
        // A cursor past the text draws a cell of its own at its column.
        let mut line = screen.lines().remove(0);
        line.cursor = Some(5);
        let spans = super::spans(line, &MONOKAI);
        assert_eq!(spans.last().map(|span| span.column), Some(5));
    }
}
