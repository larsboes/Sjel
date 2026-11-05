//! Grid rendering, shaped line layout, and monospace cell measurement for the terminal.

use gpui::{
    fill, font, point, rgb, Bounds, ContentMask, Font, FontWeight, Pixels, SharedString, TextAlign,
    TextRun, Window,
};

use super::screen::Span;

/// The font the grid draws in, and the cell it measures.
#[derive(Clone)]
pub struct Metrics {
    pub(crate) family: SharedString,
    pub(crate) size: Pixels,
    pub(crate) line: Pixels,
}

impl Metrics {
    pub(crate) fn new(family: SharedString, size: Pixels, line: Pixels) -> Self {
        Self { family, size, line }
    }
}

/// Paints the grid: one quad per painted background and one shaped line per row. The text system
/// places every glyph on the cell's advance (`force_width`), so an icon from a fallback font or a
/// wide character keeps the columns after it where they belong.
pub(crate) fn paint_grid(
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

pub(crate) fn text_run(len: usize, font: &Font, fg: u32) -> TextRun {
    TextRun {
        len,
        font: font.clone(),
        color: rgb(fg).into(),
        background_color: None,
        underline: None,
        strikethrough: None,
    }
}
