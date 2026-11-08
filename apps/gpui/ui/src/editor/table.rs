//! Markdown table parsing and rendering for NoteEditor and doc viewer.

use gpui_kit::prelude::*;
use gpui_kit::{
    div, px, relative, rgb, rgba, AnyElement, Context, FontWeight, MouseButton, MouseDownEvent,
    SharedString, StyledText,
};
use sjel_gpui_core::inline::Inline;

use super::block_view::runs;
use super::NoteEditor;
use crate::theme::{tint, Palette};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColumnAlign {
    Left,
    Center,
    Right,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedTable {
    pub headers: Vec<String>,
    pub alignments: Vec<ColumnAlign>,
    pub rows: Vec<Vec<String>>,
}

/// Splits a single line of a markdown table into cell strings.
fn split_table_row(line: &str) -> Vec<String> {
    let trimmed = line.trim();
    let content = match (trimmed.starts_with('|'), trimmed.ends_with('|')) {
        (true, true) => {
            if trimmed.len() >= 2 {
                &trimmed[1..trimmed.len() - 1]
            } else {
                ""
            }
        }
        (true, false) => &trimmed[1..],
        (false, true) => &trimmed[..trimmed.len() - 1],
        (false, false) => trimmed,
    };
    content.split('|').map(|s| s.trim().to_string()).collect()
}

/// Parses the separator/delimiter row (e.g. `|---|---|` or `|:---|:---:|---:|`).
fn parse_delimiter_row(line: &str) -> Option<Vec<ColumnAlign>> {
    let cells = split_table_row(line);
    if cells.is_empty() {
        return None;
    }
    let mut alignments = Vec::with_capacity(cells.len());
    for cell in &cells {
        let trimmed = cell.trim();
        if !trimmed.contains('-')
            || !trimmed
                .chars()
                .all(|c| c == '-' || c == ':' || c.is_whitespace())
        {
            return None;
        }
        let starts_colon = trimmed.starts_with(':');
        let ends_colon = trimmed.ends_with(':');
        let align = match (starts_colon, ends_colon) {
            (true, true) => ColumnAlign::Center,
            (false, true) => ColumnAlign::Right,
            _ => ColumnAlign::Left,
        };
        alignments.push(align);
    }
    Some(alignments)
}

/// Parses a multi-line string into a [`ParsedTable`] if it forms a valid GFM table.
pub fn parse_markdown_table(text: &str) -> Option<ParsedTable> {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    if lines.len() < 2 {
        return None;
    }
    let alignments = parse_delimiter_row(lines[1])?;
    let headers = split_table_row(lines[0]);
    if headers.is_empty() || headers.len() != alignments.len() {
        return None;
    }
    let mut rows = Vec::new();
    for line in &lines[2..] {
        let mut row_cells = split_table_row(line);
        if row_cells.len() < headers.len() {
            row_cells.resize(headers.len(), String::new());
        } else if row_cells.len() > headers.len() {
            row_cells.truncate(headers.len());
        }
        rows.push(row_cells);
    }
    Some(ParsedTable {
        headers,
        alignments,
        rows,
    })
}

/// Renders a single cell's text with inline formatting (bold, italic, code, links).
fn render_cell_text(text: &str, palette: Palette, is_header: bool) -> AnyElement {
    if text.is_empty() {
        return div().child(" ").into_any_element();
    }
    let inline = Inline::parse(text);
    let label: SharedString = if inline.text.is_empty() {
        " ".into()
    } else {
        inline.text.into()
    };
    let mut highlights = runs(&inline.marks, palette);
    if is_header {
        for (_, hl) in &mut highlights {
            hl.font_weight = Some(FontWeight::BOLD);
        }
    }
    StyledText::new(label)
        .with_highlights(highlights)
        .into_any_element()
}

/// Calculates column widths based on table content.
fn calculate_col_widths(table: &ParsedTable) -> Vec<Option<f32>> {
    let col_count = table.headers.len();
    if col_count == 0 {
        return Vec::new();
    }
    let col_lens: Vec<usize> = (0..col_count)
        .map(|c| {
            let h_len = table.headers[c].chars().count();
            let r_max = table
                .rows
                .iter()
                .map(|r| r.get(c).map_or(0, |cell| cell.chars().count()))
                .max()
                .unwrap_or(0);
            h_len.max(r_max)
        })
        .collect();

    let mut widths = vec![None; col_count];
    if col_count == 2 {
        // In 2-column key/value tables, fix the first column if it is significantly shorter.
        if col_lens[0] <= 24 && col_lens[0] * 2 <= col_lens[1].max(20) {
            let w = (col_lens[0] as f32 * 11.0).clamp(160.0, 260.0);
            widths[0] = Some(w);
        }
    } else {
        // In multi-column tables, fix short metadata columns (dates, status, priority).
        for c in 0..col_count {
            if col_lens[c] <= 12 {
                let w = (col_lens[c] as f32 * 10.0).clamp(64.0, 140.0);
                widths[c] = Some(w);
            }
        }
    }
    widths
}

/// Renders a table inside NoteEditor: clicking it focuses the block to edit raw markdown.
pub fn render_editor_table(
    table: &ParsedTable,
    palette: Palette,
    index: usize,
    cx: &mut Context<NoteEditor>,
) -> AnyElement {
    let col_count = table.headers.len();
    if col_count == 0 {
        return div().into_any_element();
    }
    let widths = calculate_col_widths(table);

    // Header row
    let mut header_row = div()
        .flex()
        .items_center()
        .w_full()
        .bg(rgba(palette.raised))
        .border_b_1()
        .border_color(rgba(palette.border));

    for (c, header) in table.headers.iter().enumerate() {
        let align = table
            .alignments
            .get(c)
            .copied()
            .unwrap_or(ColumnAlign::Left);
        let mut cell = div()
            .px_3()
            .py(px(7.))
            .text_xs()
            .font_weight(FontWeight::BOLD)
            .text_color(rgb(palette.text));

        match widths.get(c).copied().flatten() {
            Some(w) => cell = cell.w(px(w)).flex_none(),
            None => cell = cell.flex_1().min_w_0(),
        }
        if c + 1 < col_count {
            cell = cell.border_r_1().border_color(rgba(palette.border));
        }
        match align {
            ColumnAlign::Center => cell = cell.flex().justify_center(),
            ColumnAlign::Right => cell = cell.flex().justify_end(),
            ColumnAlign::Left => {}
        }
        header_row = header_row.child(cell.child(render_cell_text(header, palette, true)));
    }

    // Body rows
    let mut body = div().flex().flex_col().w_full();
    for (r_idx, row) in table.rows.iter().enumerate() {
        let mut row_el = div()
            .flex()
            .items_start()
            .w_full()
            .when(r_idx % 2 == 1, |el| el.bg(rgba(tint(palette.raised, 0x40))))
            .hover(move |s| s.bg(rgba(tint(palette.surface, 0x88))));

        if r_idx + 1 < table.rows.len() {
            row_el = row_el.border_b_1().border_color(rgba(palette.border));
        }

        for (c, cell_text) in row.iter().enumerate() {
            let align = table
                .alignments
                .get(c)
                .copied()
                .unwrap_or(ColumnAlign::Left);
            let mut cell = div()
                .px_3()
                .py(px(7.))
                .text_sm()
                .line_height(relative(1.35))
                .text_color(rgb(palette.text));

            match widths.get(c).copied().flatten() {
                Some(w) => cell = cell.w(px(w)).flex_none(),
                None => cell = cell.flex_1().min_w_0(),
            }
            if c + 1 < col_count {
                cell = cell.border_r_1().border_color(rgba(palette.border));
            }
            match align {
                ColumnAlign::Center => cell = cell.flex().justify_center(),
                ColumnAlign::Right => cell = cell.flex().justify_end(),
                ColumnAlign::Left => {}
            }
            row_el = row_el.child(cell.child(render_cell_text(cell_text, palette, false)));
        }
        body = body.child(row_el);
    }

    div()
        .id(("formatted-table", index))
        .w_full()
        .my_2()
        .overflow_hidden()
        .rounded(px(6.))
        .border_1()
        .border_color(rgba(palette.border))
        .bg(rgba(palette.surface))
        .cursor_text()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |editor, _: &MouseDownEvent, window, cx| {
                editor.focus(index, 0, window, cx);
                cx.stop_propagation();
            }),
        )
        .child(header_row)
        .child(body)
        .into_any_element()
}

/// Renders a read-only table without editor click bindings (for `doc_view.rs`).
pub fn render_static_table(table: &ParsedTable, palette: Palette) -> AnyElement {
    let col_count = table.headers.len();
    if col_count == 0 {
        return div().into_any_element();
    }
    let widths = calculate_col_widths(table);

    let mut header_row = div()
        .flex()
        .items_center()
        .w_full()
        .bg(rgba(palette.raised))
        .border_b_1()
        .border_color(rgba(palette.border));

    for (c, header) in table.headers.iter().enumerate() {
        let align = table
            .alignments
            .get(c)
            .copied()
            .unwrap_or(ColumnAlign::Left);
        let mut cell = div()
            .px_3()
            .py(px(7.))
            .text_xs()
            .font_weight(FontWeight::BOLD)
            .text_color(rgb(palette.text));

        match widths.get(c).copied().flatten() {
            Some(w) => cell = cell.w(px(w)).flex_none(),
            None => cell = cell.flex_1().min_w_0(),
        }
        if c + 1 < col_count {
            cell = cell.border_r_1().border_color(rgba(palette.border));
        }
        match align {
            ColumnAlign::Center => cell = cell.flex().justify_center(),
            ColumnAlign::Right => cell = cell.flex().justify_end(),
            ColumnAlign::Left => {}
        }
        header_row = header_row.child(cell.child(render_cell_text(header, palette, true)));
    }

    let mut body = div().flex().flex_col().w_full();
    for (r_idx, row) in table.rows.iter().enumerate() {
        let mut row_el = div()
            .flex()
            .items_start()
            .w_full()
            .when(r_idx % 2 == 1, |el| el.bg(rgba(tint(palette.raised, 0x40))))
            .hover(move |s| s.bg(rgba(tint(palette.surface, 0x88))));

        if r_idx + 1 < table.rows.len() {
            row_el = row_el.border_b_1().border_color(rgba(palette.border));
        }

        for (c, cell_text) in row.iter().enumerate() {
            let align = table
                .alignments
                .get(c)
                .copied()
                .unwrap_or(ColumnAlign::Left);
            let mut cell = div()
                .px_3()
                .py(px(7.))
                .text_sm()
                .line_height(relative(1.35))
                .text_color(rgb(palette.text));

            match widths.get(c).copied().flatten() {
                Some(w) => cell = cell.w(px(w)).flex_none(),
                None => cell = cell.flex_1().min_w_0(),
            }
            if c + 1 < col_count {
                cell = cell.border_r_1().border_color(rgba(palette.border));
            }
            match align {
                ColumnAlign::Center => cell = cell.flex().justify_center(),
                ColumnAlign::Right => cell = cell.flex().justify_end(),
                ColumnAlign::Left => {}
            }
            row_el = row_el.child(cell.child(render_cell_text(cell_text, palette, false)));
        }
        body = body.child(row_el);
    }

    div()
        .w_full()
        .my_2()
        .overflow_hidden()
        .rounded(px(6.))
        .border_1()
        .border_color(rgba(palette.border))
        .bg(rgba(palette.surface))
        .child(header_row)
        .child(body)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_standard_markdown_table_with_pipes() {
        let input = "\
| Frage | Stand |
|---|---|
| Welcher Master | Sieben geprüft, drei mit Frist. |
| Promotion | Route geklärt, Zahlen unfreundlich. |";

        let table = parse_markdown_table(input).expect("table parses");
        assert_eq!(table.headers, vec!["Frage", "Stand"]);
        assert_eq!(table.alignments, vec![ColumnAlign::Left, ColumnAlign::Left]);
        assert_eq!(table.rows.len(), 2);
        assert_eq!(
            table.rows[0],
            vec!["Welcher Master", "Sieben geprüft, drei mit Frist."]
        );
        assert_eq!(
            table.rows[1],
            vec!["Promotion", "Route geklärt, Zahlen unfreundlich."]
        );
    }

    #[test]
    fn parses_table_without_outer_pipes() {
        let input = "\
Asset | Detail
---|---
NAS | Device-Model-A
Network | FTTH 1Gbit";

        let table = parse_markdown_table(input).expect("table without outer pipes parses");
        assert_eq!(table.headers, vec!["Asset", "Detail"]);
        assert_eq!(table.rows.len(), 2);
        assert_eq!(table.rows[0], vec!["NAS", "Device-Model-A"]);
    }

    #[test]
    fn parses_column_alignments() {
        let input = "\
| Left | Center | Right |
| :--- | :---: | ---: |
| L | C | R |";

        let table = parse_markdown_table(input).expect("aligned table parses");
        assert_eq!(
            table.alignments,
            vec![ColumnAlign::Left, ColumnAlign::Center, ColumnAlign::Right]
        );
    }

    #[test]
    fn refuses_plain_paragraph_and_non_table_text() {
        assert!(parse_markdown_table("Just a regular paragraph with no table.").is_none());
        assert!(parse_markdown_table("Option A | Option B without delimiter").is_none());
        assert!(parse_markdown_table("| only | one | line |").is_none());
    }
}
