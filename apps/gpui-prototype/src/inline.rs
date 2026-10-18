//! Inline markdown for a block that is not being edited: the text a reader sees, the marks on
//! it, and where each shown byte came from in the source, so a click lands at the same place in
//! the markdown.
//!
//! Covers `**bold**`, `__bold__`, `*italic*`, `_italic_`, `` `code` ``, `~~strike~~`,
//! `[label](url)` and `[[target|alias]]`. An unclosed marker stays literal text. A single
//! newline and the indent after it show as one space, as CommonMark reads a soft line break;
//! a table keeps its rows.

use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Bold,
    Italic,
    Code,
    Strike,
    Link,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Inline {
    pub text: String,
    /// Ranges in `text`.
    pub marks: Vec<(Range<usize>, Mark)>,
    /// The source offset of each byte of `text`, then the source length.
    source: Vec<usize>,
}

impl Inline {
    pub fn parse(src: &str) -> Self {
        let mut out = Self::default();
        let flow = !src.trim_start().starts_with('|');
        parse(src, 0, flow, &mut out);
        out.source.push(src.len());
        out
    }

    /// The source offset for a position in `text`.
    pub fn source_offset(&self, shown: usize) -> usize {
        self.source[shown.min(self.source.len() - 1)]
    }

    fn push(&mut self, src: &str, at: usize, base: usize) {
        let ch = src[at..].chars().next().expect("a char at a char boundary");
        for byte in 0..ch.len_utf8() {
            self.source.push(base + at + byte);
        }
        self.text.push(ch);
    }

    /// `inner` (at `base + start` in the source), parsed, under `mark`.
    fn wrap(&mut self, src: &str, inner: Range<usize>, base: usize, mark: Mark) {
        let shown = self.text.len();
        if mark == Mark::Code {
            let mut at = inner.start;
            while at < inner.end {
                self.push(src, at, base);
                at += src[at..].chars().next().map_or(1, char::len_utf8);
            }
        } else {
            parse(&src[inner.clone()], base + inner.start, true, self);
        }
        if self.text.len() > shown {
            self.marks.push((shown..self.text.len(), mark));
        }
    }
}

fn parse(src: &str, base: usize, flow: bool, out: &mut Inline) {
    let mut at = 0;
    while at < src.len() {
        let rest = &src[at..];
        if flow && rest.starts_with('\n') {
            out.text.push(' ');
            out.source.push(base + at);
            at += 1 + rest[1..].len() - rest[1..].trim_start_matches([' ', '\t']).len();
            continue;
        }
        if let Some((inner, end, mark)) = span(src, at, rest) {
            out.wrap(src, inner, base, mark);
            at = end;
            continue;
        }
        out.push(src, at, base);
        at += rest.chars().next().map_or(1, char::len_utf8);
    }
}

/// A span opening at `at`: its inner range, the offset after it, and its mark.
fn span(src: &str, at: usize, rest: &str) -> Option<(Range<usize>, usize, Mark)> {
    let closed = |open: &str, close: &str| -> Option<(Range<usize>, usize)> {
        let start = at + open.len();
        let len = src[start..].find(close)?;
        (len > 0).then(|| (start..start + len, start + len + close.len()))
    };
    if rest.starts_with('`') {
        let (inner, end) = closed("`", "`")?;
        return Some((inner, end, Mark::Code));
    }
    if rest.starts_with("[[") {
        let (inner, end) = closed("[[", "]]")?;
        // `[[target|alias]]` shows the alias.
        let shown = match src[inner.clone()].find('|') {
            Some(bar) => inner.start + bar + 1..inner.end,
            None => inner,
        };
        return Some((shown, end, Mark::Link));
    }
    if rest.starts_with('[') {
        let (label, close) = closed("[", "](")?;
        let url_end = src[close..].find(')')?;
        return Some((label, close + url_end + 1, Mark::Link));
    }
    for (marker, mark) in [("**", Mark::Bold), ("__", Mark::Bold), ("~~", Mark::Strike)] {
        if rest.starts_with(marker) {
            if let Some((inner, end)) = closed(marker, marker) {
                return Some((inner, end, mark));
            }
        }
    }
    for marker in ["*", "_"] {
        if !rest.starts_with(marker) || rest[1..].starts_with(marker) || rest[1..].starts_with(' ')
        {
            continue;
        }
        // `snake_case` is a word, not emphasis.
        let after_word = src[..at]
            .chars()
            .next_back()
            .is_some_and(char::is_alphanumeric);
        if marker == "_" && after_word {
            continue;
        }
        if let Some((inner, end)) = closed(marker, marker) {
            return Some((inner, end, Mark::Italic));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shown(src: &str) -> (String, Vec<(&'static str, String)>) {
        let inline = Inline::parse(src);
        let marks = inline
            .marks
            .iter()
            .map(|(range, mark)| {
                let name = match mark {
                    Mark::Bold => "bold",
                    Mark::Italic => "italic",
                    Mark::Code => "code",
                    Mark::Strike => "strike",
                    Mark::Link => "link",
                };
                (name, inline.text[range.clone()].to_owned())
            })
            .collect();
        (inline.text, marks)
    }

    #[test]
    fn markers_hide_and_marks_cover_their_text() {
        assert_eq!(
            shown("a **b** *c* `d*e` ~~f~~"),
            (
                "a b c d*e f".into(),
                vec![
                    ("bold", "b".into()),
                    ("italic", "c".into()),
                    ("code", "d*e".into()),
                    ("strike", "f".into())
                ]
            )
        );
        assert_eq!(
            shown("see [docs](https://x.y) and [[Notes/Plan|the plan]] or [[Inbox]]"),
            (
                "see docs and the plan or Inbox".into(),
                vec![
                    ("link", "docs".into()),
                    ("link", "the plan".into()),
                    ("link", "Inbox".into())
                ]
            )
        );
        assert_eq!(
            shown("**bold [link](u)**").1,
            vec![("link", "link".into()), ("bold", "bold link".into())]
        );
    }

    #[test]
    fn unclosed_and_word_internal_markers_stay_literal() {
        for src in [
            "2 * 3",
            "snake_case_name",
            "**open",
            "`tick",
            "[no link]",
            "a ** b",
        ] {
            assert_eq!(shown(src), (src.to_owned(), vec![]), "{src}");
        }
    }

    #[test]
    fn soft_breaks_flow_and_tables_keep_their_rows() {
        let inline = Inline::parse("one\n  two");
        assert_eq!(inline.text, "one two");
        // `t` sits after the newline and its indent.
        assert_eq!(inline.source_offset(4), 6);
        assert_eq!(Inline::parse("| a |\n| b |").text, "| a |\n| b |");
    }

    #[test]
    fn a_shown_position_maps_back_to_the_source() {
        let inline = Inline::parse("é **bold** end");
        // "é bold end": `b` at shown 3 is source 5, after the opening marker.
        assert_eq!(inline.text, "é bold end");
        assert_eq!(inline.source_offset(3), 5);
        // The end of the text is the end of the source.
        assert_eq!(
            inline.source_offset(inline.text.len()),
            "é **bold** end".len()
        );
        assert_eq!(inline.source_offset(0), 0);
    }
}
