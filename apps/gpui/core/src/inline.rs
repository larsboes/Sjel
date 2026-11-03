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
    /// Link targets paired with the displayed character ranges they cover.
    pub links: Vec<(Range<usize>, String)>,
    /// The source offset of each byte of `text`, then the source length.
    source: Vec<usize>,
}

impl Inline {
    pub fn parse(src: &str) -> Self {
        let mut out = Self::default();
        let flow = !src.trim_start().starts_with('|');
        parse(src, 0, flow, None, &mut out);
        out.source.push(src.len());
        out
    }

    /// Projects markdown for editing, revealing delimiters only in spans containing the source
    /// caret. The source remains unchanged; offsets and marks describe the projected text.
    pub fn preview(src: &str, caret: usize) -> Self {
        let mut out = Self::default();
        let flow = !src.trim_start().starts_with('|');
        parse(src, 0, flow, Some(caret), &mut out);
        out.source.push(src.len());
        out
    }

    /// The source offset for a position in `text`.
    pub fn source_offset(&self, shown: usize) -> usize {
        self.source[shown.min(self.source.len() - 1)]
    }

    /// The shown offset for a source offset. A position inside a marker collects to the first
    /// shown byte after it, which is where a caret at the start of `**bold**` belongs.
    pub fn shown_offset(&self, source: usize) -> usize {
        // `source` is monotone, so the first entry at or past `source` is the answer.
        let mut at = self.source.partition_point(|mapped| *mapped < source);
        at = at.min(self.text.len());
        // A source offset inside a multi-byte character lands before that character, never in it.
        while at > 0 && !self.text.is_char_boundary(at) {
            at -= 1;
        }
        at
    }

    /// The source ranges the marks hide: the `**`, the backticks, and a link's `](url)`. What a
    /// rendered view leaves out, and what a live-preview editor conceals everywhere but under the
    /// caret.
    pub fn concealed(&self) -> Vec<Range<usize>> {
        let mut ranges: Vec<Range<usize>> = Vec::new();
        let mut expected = 0;
        for mapped in &self.source {
            if *mapped > expected {
                match ranges.last_mut() {
                    Some(range) if range.end == expected => range.end = *mapped,
                    _ => ranges.push(expected..*mapped),
                }
            }
            expected = *mapped + 1;
        }
        ranges
    }

    fn push(&mut self, src: &str, at: usize, base: usize) {
        let ch = src[at..].chars().next().expect("a char at a char boundary");
        for byte in 0..ch.len_utf8() {
            self.source.push(base + at + byte);
        }
        self.text.push(ch);
    }

    /// `inner` (at `base + start` in the source), parsed, under `mark`.
    fn wrap(
        &mut self,
        src: &str,
        inner: Range<usize>,
        base: usize,
        mark: Mark,
        caret: Option<usize>,
    ) {
        let shown = self.text.len();
        if mark == Mark::Code {
            let mut at = inner.start;
            while at < inner.end {
                self.push(src, at, base);
                at += src[at..].chars().next().map_or(1, char::len_utf8);
            }
        } else {
            parse(&src[inner.clone()], base + inner.start, true, caret, self);
        }
        if self.text.len() > shown {
            self.marks.push((shown..self.text.len(), mark));
        }
    }
}

fn parse(src: &str, base: usize, flow: bool, caret: Option<usize>, out: &mut Inline) {
    let mut at = 0;
    while at < src.len() {
        let rest = &src[at..];
        if flow && rest.starts_with('\n') {
            out.text.push(' ');
            out.source.push(base + at);
            at += 1 + rest[1..].len() - rest[1..].trim_start_matches([' ', '\t']).len();
            continue;
        }
        if let Some((inner, end, mark, target)) = span(src, at, rest) {
            let reveal = caret.is_some_and(|caret| (base + at..base + end).contains(&caret));
            if reveal {
                for (offset, _) in src[at..inner.start].char_indices() {
                    out.push(src, at + offset, base);
                }
            }
            let shown = out.text.len();
            let inner_end = inner.end;
            out.wrap(src, inner, base, mark, caret);
            if let Some(target) = target {
                out.links.push((shown..out.text.len(), target));
            }
            if reveal {
                for (offset, _) in src[inner_end..end].char_indices() {
                    out.push(src, inner_end + offset, base);
                }
            }
            at = end;
            continue;
        }
        out.push(src, at, base);
        at += rest.chars().next().map_or(1, char::len_utf8);
    }
}

/// A span opening at `at`: its inner range, the offset after it, and its mark.
fn span(src: &str, at: usize, rest: &str) -> Option<(Range<usize>, usize, Mark, Option<String>)> {
    let closed = |open: &str, close: &str| -> Option<(Range<usize>, usize)> {
        let start = at + open.len();
        let len = src[start..].find(close)?;
        (len > 0).then(|| (start..start + len, start + len + close.len()))
    };
    if rest.starts_with('`') {
        let (inner, end) = closed("`", "`")?;
        return Some((inner, end, Mark::Code, None));
    }
    if rest.starts_with("[[") {
        let (inner, end) = closed("[[", "]]")?;
        // `[[target|alias]]` shows the alias.
        let target_end = src[inner.clone()]
            .find('|')
            .map_or(inner.end, |bar| inner.start + bar);
        let target = src[inner.start..target_end].to_owned();
        let shown = match src[inner.clone()].find('|') {
            Some(bar) => inner.start + bar + 1..inner.end,
            None => inner,
        };
        return Some((shown, end, Mark::Link, Some(target)));
    }
    if rest.starts_with('[') {
        let (label, close) = closed("[", "](")?;
        if src[label.clone()].contains('[') || src[label.clone()].contains(']') {
            return None;
        }
        let url_end = src[close..].find(')')?;
        let target = src[close..close + url_end].trim().to_owned();
        return Some((label, close + url_end + 1, Mark::Link, Some(target)));
    }
    for (marker, mark) in [
        ("**", Mark::Bold),
        ("__", Mark::Bold),
        ("~~", Mark::Strike),
        ("==", Mark::Highlight),
    ] {
        if rest.starts_with(marker) {
            if let Some((inner, end)) = closed(marker, marker) {
                return Some((inner, end, mark, None));
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
            return Some((inner, end, Mark::Italic, None));
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
                    Mark::Highlight => "highlight",
                };
                (name, inline.text[range.clone()].to_owned())
            })
            .collect();
        (inline.text, marks)
    }

    #[test]
    fn markers_hide_and_marks_cover_their_text() {
        assert_eq!(
            shown("a **b** *c* `d*e` ~~f~~ ==g=="),
            (
                "a b c d*e f g".into(),
                vec![
                    ("bold", "b".into()),
                    ("italic", "c".into()),
                    ("code", "d*e".into()),
                    ("strike", "f".into()),
                    ("highlight", "g".into())
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
    fn preview_reveals_only_the_carets_span_and_keeps_its_style() {
        let src = "a **bold** and *other* end";
        let preview = Inline::preview(src, 5);
        assert_eq!(preview.text, "a **bold** and other end");
        assert_eq!(
            preview.marks,
            vec![(4..8, Mark::Bold), (15..20, Mark::Italic)]
        );
        assert_eq!(Inline::preview(src, 10).text, Inline::parse(src).text);
        assert_eq!(
            Inline::preview(src, src.len()).text,
            Inline::parse(src).text
        );
        assert_eq!(Inline::preview("**bold**", 0).text, "**bold**");
    }

    #[test]
    fn preview_reveals_nested_delimiters_but_not_a_sibling() {
        let src = "**one *two* [three](url)**";
        assert_eq!(Inline::preview(src, 8).text, "**one *two* three**");
        let link = Inline::preview(src, src.find("url").unwrap());
        assert_eq!(link.text, "**one two [three](url)**");
        assert_eq!(link.links[0].1, "url");
        assert_eq!(
            Inline::preview("[[Plan|名前]] `code`", 8).text,
            "[[Plan|名前]] code"
        );
    }

    #[test]
    fn preview_offsets_round_trip_every_visible_character() {
        for src in [
            "é **名前** 🦀 [label](url)",
            "**a *b* c**",
            "",
            "one\n  two",
        ] {
            for caret in src.char_indices().map(|(at, _)| at).chain([src.len()]) {
                let preview = Inline::preview(src, caret);
                for (shown, ch) in preview.text.char_indices() {
                    let source = preview.source_offset(shown);
                    assert_eq!(preview.shown_offset(source), shown, "{src:?} at {caret}");
                    assert!(src.is_char_boundary(source));
                    if ch != ' ' || src.as_bytes()[source] != b'\n' {
                        assert!(src[source..].starts_with(ch));
                    }
                }
                assert!(preview.text.is_char_boundary(preview.shown_offset(caret)));
                assert_eq!(preview.source_offset(preview.text.len()), src.len());
            }
        }
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
    fn concealment_names_every_marker_and_maps_the_caret_back() {
        let src = "a **bold** and `code` [[Inbox|there]]";
        let inline = Inline::parse(src);
        let hidden: String = inline
            .concealed()
            .iter()
            .map(|range| &src[range.clone()])
            .collect();
        assert_eq!(hidden, "****``[[Inbox|]]");
        // A caret anywhere in a marker lands on the text after it, never inside.
        assert_eq!(inline.shown_offset(2), 2);
        assert_eq!(inline.shown_offset(3), 2);
        assert_eq!(inline.shown_offset(4), 2);
        assert_eq!((src.as_bytes()[4] as char), 'b');
        // Every shown byte maps back to the byte it came from.
        for shown in 0..inline.text.len() {
            assert_eq!(
                inline.source_offset(inline.shown_offset(inline.source_offset(shown))),
                inline.source_offset(shown)
            );
        }
        assert_eq!(Inline::parse("plain").concealed(), vec![]);
        assert_eq!(inline.shown_offset(src.len()), inline.text.len());
        // An offset inside a multi-byte character lands before it, on a boundary.
        let accented = Inline::parse("é **bold**");
        for source in 0..=accented_source_len() {
            assert!(accented
                .text
                .is_char_boundary(accented.shown_offset(source)));
        }
        assert_eq!(accented.shown_offset(1), 0);
        fn accented_source_len() -> usize {
            "é **bold**".len()
        }
    }

    /// `SJEL_NOTES_DIR=<dir> cargo test --lib concealment_cost -- --ignored`: what live preview
    /// would add to a note, in microseconds. Parses each block, names its concealed ranges and
    /// maps every source byte to the shown one. Prints timings and counts only, never note text.
    #[test]
    #[ignore]
    fn concealment_cost() {
        use std::time::Instant;
        let dir = std::env::var("SJEL_NOTES_DIR").expect("SJEL_NOTES_DIR");
        let mut stack = vec![std::path::PathBuf::from(dir)];
        let (mut notes, mut blocks, mut bytes) = (0usize, 0usize, 0usize);
        let mut slowest = (0f64, 0usize);
        let mut total = 0f64;
        while let Some(path) = stack.pop() {
            for entry in std::fs::read_dir(&path).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "md") {
                    let content = std::fs::read_to_string(&path).unwrap();
                    let doc = crate::blocks::Doc::parse(&content);
                    let start = Instant::now();
                    let mut plan = 0usize;
                    for block in &doc.blocks {
                        let inline = Inline::parse(&block.text);
                        plan += inline.concealed().len();
                        // Every caret the block can hold, mapped to the shown text.
                        for source in 0..=block.text.len() {
                            plan += inline.shown_offset(source);
                        }
                    }
                    std::hint::black_box(plan);
                    let micros = start.elapsed().as_secs_f64() * 1e6;
                    notes += 1;
                    blocks += doc.blocks.len();
                    bytes += content.len();
                    total += micros;
                    if micros > slowest.0 {
                        slowest = (micros, doc.blocks.len());
                    }
                }
            }
        }
        println!(
            "{notes} notes, {blocks} blocks, {bytes} bytes: {:.1} µs a note, {:.2} µs a block; \
             slowest note {:.1} µs over {} blocks",
            total / notes.max(1) as f64,
            total / blocks.max(1) as f64,
            slowest.0,
            slowest.1
        );
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
