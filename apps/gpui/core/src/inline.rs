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
    Highlight,
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
    for (marker, mark) in [
        ("**", Mark::Bold),
        ("__", Mark::Bold),
        ("~~", Mark::Strike),
        ("==", Mark::Highlight),
    ] {
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
