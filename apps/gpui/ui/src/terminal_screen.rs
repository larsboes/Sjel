//! Screen representation, cell styling, and ANSI color decoding for the terminal emulator.

use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point as GridPoint, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::vte::ansi::{Color, NamedColor, Processor};

use crate::theme::TerminalScheme;

/// The history the parser keeps, for the scroll-back the wheel already moves through.
pub(crate) const HISTORY: usize = 10_000;

/// What the parser reports to. A title, a colour query and a bell are events this client does not
/// act on yet, and an unhandled event is not an error.
pub(crate) struct Quiet;

impl EventListener for Quiet {}

/// The visible size, as the parser wants it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Size {
    pub(crate) cols: usize,
    pub(crate) rows: usize,
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
pub(crate) struct Screen {
    pub(crate) term: Term<Quiet>,
    parser: Processor,
    size: Size,
}

impl Screen {
    pub(crate) fn new(cols: usize, rows: usize) -> Self {
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
    pub(crate) fn feed(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
    }

    pub(crate) fn resize(&mut self, cols: usize, rows: usize) {
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

    pub(crate) fn cols(&self) -> usize {
        self.size.cols
    }

    pub(crate) fn rows(&self) -> usize {
        self.size.rows
    }

    /// How many lines above the live screen the reader has scrolled.
    pub(crate) fn scrolled(&self) -> usize {
        self.term.grid().display_offset()
    }

    pub(crate) fn scroll(&mut self, lines: i32) {
        // The parser owns the offset: it clamps at both ends and repaints what it exposes.
        self.term.scroll_display(Scroll::Delta(lines));
    }

    /// The grid cell at a visible `col` and `row`, in the parser's coordinates, which count
    /// history lines as negative.
    pub(crate) fn point_at(&self, col: usize, row: usize) -> GridPoint {
        let row = row.min(self.size.rows - 1) as i32;
        GridPoint::new(
            Line(row - self.scrolled() as i32),
            Column(col.min(self.size.cols - 1)),
        )
    }

    pub(crate) fn select(&mut self, kind: SelectionType, at: GridPoint, side: Side) {
        self.term.selection = Some(Selection::new(kind, at, side));
    }

    pub(crate) fn extend_selection(&mut self, at: GridPoint, side: Side) {
        if let Some(selection) = self.term.selection.as_mut() {
            selection.update(at, side);
        }
    }

    /// Every line the parser still holds, history included.
    pub(crate) fn select_all(&mut self) {
        let top = GridPoint::new(Line(-(self.term.grid().history_size() as i32)), Column(0));
        let bottom = GridPoint::new(Line(self.size.rows as i32 - 1), Column(self.size.cols - 1));
        let mut selection = Selection::new(SelectionType::Lines, top, Side::Left);
        selection.update(bottom, Side::Right);
        self.term.selection = Some(selection);
    }

    /// A click without a drag selects nothing, so it leaves no highlight behind.
    pub(crate) fn drop_empty_selection(&mut self) {
        if self
            .term
            .selection
            .as_ref()
            .is_some_and(Selection::is_empty)
        {
            self.term.selection = None;
        }
    }

    pub(crate) fn clear_selection(&mut self) {
        self.term.selection = None;
    }

    pub(crate) fn selected_text(&self) -> Option<String> {
        self.term
            .selection_to_string()
            .filter(|text| !text.is_empty())
    }

    pub(crate) fn bracketed_paste(&self) -> bool {
        self.term.mode().contains(TermMode::BRACKETED_PASTE)
    }

    /// The screen as the view draws it: one entry per line, each a run per shared look.
    pub(crate) fn lines(&self) -> Vec<ScreenLine> {
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
pub(crate) struct ScreenLine {
    pub(crate) runs: Vec<Run>,
    /// The column the cursor sits at, when it sits on this line.
    pub(crate) cursor: Option<usize>,
}

/// Cells that share a look. A wide character's second cell is a space in the run, so every
/// character is one cell and the run's length is its width.
pub(crate) struct Run {
    pub(crate) style: Style,
    pub(crate) text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Style {
    pub(crate) fg: Color,
    pub(crate) bg: Color,
    pub(crate) bold: bool,
    /// Foreground and background swapped: a program's reverse video.
    pub(crate) reverse: bool,
    pub(crate) selected: bool,
}

impl Style {
    /// The background (absent means the grid's own) and foreground, as `0xRRGGBB`.
    pub(crate) fn tones(&self, scheme: &TerminalScheme) -> (Option<u32>, u32) {
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
pub fn tone_of(colour: Color, scheme: &TerminalScheme) -> Option<u32> {
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

pub(crate) fn hex(r: u8, g: u8, b: u8) -> u32 {
    (r as u32) << 16 | (g as u32) << 8 | b as u32
}

/// The xterm 256-colour palette: the scheme's sixteen tones, then a 6×6×6 cube, then 24 greys.
pub fn ansi_256(index: u8, scheme: &TerminalScheme) -> u32 {
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
pub(crate) fn named_index(named: NamedColor) -> u8 {
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
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Span {
    pub(crate) column: usize,
    pub(crate) text: String,
    pub(crate) bg: Option<u32>,
    pub(crate) fg: u32,
    pub(crate) bold: bool,
}

/// A line as spans. The cursor splits the run it sits in and draws its cell inverted, which is
/// what every terminal does; a cursor past the end of the text gets a cell of its own.
pub(crate) fn spans(line: ScreenLine, scheme: &TerminalScheme) -> Vec<Span> {
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
