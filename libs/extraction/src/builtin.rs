//! HTML and plain text, hand-rolled, no dependency.

use crate::{cap, Document, Extraction, ExtractionError, Extractor, InputClass, Result, TEXT_CAP};

/// HTML and plain text, hand-rolled, no dependency.
///
/// It stays after xberg lands rather than being deleted: it is the fallback
/// when a document does not parse, and the reference the corpus gate scores a
/// replacement against.
pub struct Builtin;

impl Extractor for Builtin {
    fn name(&self) -> &'static str {
        "builtin"
    }

    fn handles(&self, class: InputClass) -> bool {
        matches!(class, InputClass::Html | InputClass::PlainText)
    }

    fn extract(&self, doc: &Document<'_>) -> Result<Extraction> {
        let body = String::from_utf8_lossy(doc.bytes);
        match doc.class {
            InputClass::Html => Ok(Extraction {
                title: html_title(&body),
                text: html_to_lines(&body),
                markdown: None,
                producer: self.name(),
            }),
            InputClass::PlainText => Ok(Extraction {
                title: None,
                text: cap(body.trim().to_string()),
                markdown: None,
                producer: self.name(),
            }),
            // Listed rather than wildcarded, so adding an input class is a
            // compile error here and a decision somebody makes on purpose.
            InputClass::Pdf | InputClass::Image => Err(ExtractionError::UnsupportedClass {
                extractor: self.name(),
                class: doc.class,
            }),
        }
    }
}

/// HTML to text, keeping the line structure the normalizer needs.
///
/// What a tag becomes is the whole point. Running every tag through a blanket
/// stripper and then collapsing all whitespace welded the document into one
/// line, which broke both stages downstream: `normalize` is a table of line
/// predicates guarded by a max line length, so a single long line made every
/// rule unreachable and stored consent walls verbatim as article text, and a
/// tag boundary that emitted nothing welded `Home</a><a>Jobs` into one token
/// the embedder then scored on.
pub fn html_to_lines(html: &str) -> String {
    let without_blocks = remove_blocks(html, &["script", "style", "noscript", "head"]);
    let text = tags_to_breaks(&without_blocks);
    let collapsed = collapse_lines(&decode_basic_entities(&text));
    collapsed.chars().take(TEXT_CAP).collect()
}

pub fn html_title(html: &str) -> Option<String> {
    let low = html.to_lowercase();
    let start = low.find("<title")?;
    let gt = low[start..].find('>')? + start + 1;
    let end = low[gt..].find("</title>")? + gt;
    let title = decode_basic_entities(&strip_all_tags(html[gt..end].trim()));
    let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.is_empty() {
        None
    } else {
        Some(title)
    }
}

/// Does this look like markup rather than the text a client already extracted?
pub fn looks_like_html(raw: &str) -> bool {
    let low = raw.to_lowercase();
    low.contains("<html")
        || low.contains("<body")
        || low.contains("<div")
        || low.contains("<p>")
        || low.contains("<span")
        || low.contains("<article")
}

/// HTML elements that end a line of text. Everything not listed is inline and
/// becomes a single space, so two adjacent text nodes stay two words.
const BLOCK_TAGS: &[&str] = &[
    "address",
    "article",
    "aside",
    "blockquote",
    "br",
    "button",
    "dd",
    "div",
    "dl",
    "dt",
    "figcaption",
    "figure",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hr",
    "li",
    "main",
    "nav",
    "ol",
    "option",
    "p",
    "pre",
    "section",
    "table",
    "td",
    "th",
    "tr",
    "ul",
];

/// Replace every tag with the separator its element implies: a newline for a
/// block element, a space for an inline one. An unterminated `<` is markup the
/// page never closed, so the remainder goes.
fn tags_to_breaks(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;

    while let Some(open) = rest.find('<') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let close = match after.find('>') {
            Some(i) => i,
            None => return out,
        };
        out.push(if is_block_tag(&after[..close]) {
            '\n'
        } else {
            ' '
        });
        rest = &after[close + 1..];
    }

    out.push_str(rest);
    out
}

fn is_block_tag(inner: &str) -> bool {
    let name = inner
        .trim_start_matches('/')
        .split(|c: char| c.is_whitespace() || c == '/')
        .next()
        .unwrap_or_default();
    BLOCK_TAGS.iter().any(|tag| tag.eq_ignore_ascii_case(name))
}

/// Collapse whitespace inside each line, keeping the breaks the tag walk
/// produced. Collapsing across newlines is exactly the structure loss this
/// path exists to avoid.
fn collapse_lines(s: &str) -> String {
    s.lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Drop a whole element, opening tag to closing tag.
///
/// The tag NAME has to match, not a prefix of it. `<head` also starts
/// `<header`, and once the real `</head>` had been consumed the search for a
/// second one failed and took the `None` arm below — which truncates. Every
/// page carrying a site `<header>` therefore lost everything from that element
/// onward, silently, as an empty or stub transcript. A real arXiv paper
/// extracted to zero characters, which is how this surfaced.
fn remove_blocks(html: &str, tags: &[&str]) -> String {
    let mut out = html.to_string();
    for tag in tags {
        loop {
            let low = out.to_lowercase();
            let open = match find_open_tag(&low, tag) {
                Some(i) => i,
                None => break,
            };
            let close_tag = format!("</{tag}>");
            let close = match low[open..].find(&close_tag) {
                Some(i) => open + i + close_tag.len(),
                // Unclosed: the rest of the document is inside an element we
                // are removing, so there is nothing after it to keep.
                None => {
                    out.truncate(open);
                    break;
                }
            };
            out.replace_range(open..close, " ");
        }
    }
    out
}

/// Offset of `<tag` where the next character actually ends the tag name.
fn find_open_tag(haystack: &str, tag: &str) -> Option<usize> {
    let needle = format!("<{tag}");
    let mut from = 0usize;
    while let Some(hit) = haystack[from..].find(&needle) {
        let at = from + hit;
        let after = haystack[at + needle.len()..].chars().next();
        match after {
            None | Some('>') | Some('/') => return Some(at),
            Some(c) if c.is_whitespace() => return Some(at),
            // `<header` when looking for `<head`: keep searching.
            _ => from = at + needle.len(),
        }
    }
    None
}

/// Depth-counted tag removal, for a fragment where no separator is wanted --
/// a `<title>` is one line by definition.
fn strip_all_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut depth = 0usize;
    for c in s.chars() {
        match c {
            '<' => depth += 1,
            '>' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

/// Whitespace collapsed across the whole string, newlines included.
///
/// The opposite of what `collapse_lines` does, and correct for the callers
/// that have it: a `<title>`, or one field of machine-generated XML, is one
/// line by definition. Never use it on a document — that is the bug that made
/// every normalization rule unreachable.
pub fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The entities Sjel's sources actually emit.
///
/// `&amp;` is decoded **last**, and that ordering is the whole correctness
/// argument: decoding it first turns `&amp;lt;` into `&lt;`, which the next
/// replacement then turns into a real `<`. A page that escaped its markup for
/// display would have it silently reconstituted as markup, one stage before
/// the tag walk that eats it.
pub fn decode_basic_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = s.to_string();
    for (entity, replacement) in [
        ("&nbsp;", " "),
        ("&lt;", "<"),
        ("&gt;", ">"),
        ("&quot;", "\""),
        ("&apos;", "'"),
        ("&#39;", "'"),
        ("&#x27;", "'"),
        ("&#x2F;", "/"),
        ("&#47;", "/"),
        ("&hellip;", "…"),
        ("&mdash;", "—"),
        ("&ndash;", "–"),
        ("&amp;", "&"),
    ] {
        if out.contains(entity) {
            out = out.replace(entity, replacement);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_site_header_does_not_take_the_rest_of_the_page_with_it() {
        // `<head` is a prefix of `<header`. With prefix matching, the second
        // pass over the document found `<header`, failed to find a closing
        // `</head>` (the real one was already consumed), and truncated
        // everything from there on. A real arXiv paper extracted to zero
        // characters; every page with a site header lost its body silently.
        let html = "<html><head><title>T</title></head><body>\
            <header><p>Site chrome</p></header>\
            <article><p>The body that must survive.</p></article>\
            </body></html>";

        let out = Builtin.extract(&Document::html(html.as_bytes())).unwrap();
        assert!(
            out.text.contains("The body that must survive."),
            "the page was truncated at <header>: {:?}",
            out.text
        );
        assert!(
            out.text.contains("Site chrome"),
            "<header> is page furniture for the normalizer to judge, not for \
             extraction to delete: {:?}",
            out.text
        );
        assert_eq!(out.title.as_deref(), Some("T"));
    }

    #[test]
    fn an_unclosed_removed_element_still_takes_the_rest() {
        // The truncating branch is correct on its own terms: everything after
        // an unclosed <script> is inside it. Kept as a decision, not an
        // accident, now that exact matching stops it firing on `<header>`.
        let out = Builtin
            .extract(&Document::html(
                b"<body><p>Kept.</p><script>var x = 1;<p>Gone.</p></body>",
            ))
            .unwrap();
        assert!(out.text.contains("Kept."));
        assert!(!out.text.contains("Gone."));
    }

    #[test]
    fn script_and_style_bodies_never_reach_the_text() {
        let html = "<html><head><title>My &amp; Page</title><style>.x{}</style></head>\
            <body><script>bad()</script><p>Hello  world</p></body></html>";
        let out = Builtin.extract(&Document::html(html.as_bytes())).unwrap();
        assert_eq!(out.title.as_deref(), Some("My & Page"));
        assert!(out.text.contains("Hello world"), "got: {}", out.text);
        assert!(!out.text.contains("bad()"), "script body must be dropped");
        assert!(!out.text.contains(".x{"), "style body must be dropped");
    }

    #[test]
    fn a_tag_boundary_separates_words_rather_than_welding_them() {
        let out = Builtin
            .extract(&Document::html(b"<a>Home</a><a>Jobs</a>"))
            .unwrap();
        assert_eq!(out.text, "Home Jobs");
    }

    #[test]
    fn plain_text_is_returned_as_it_arrived_apart_from_the_cap() {
        let out = Builtin
            .extract(&Document::text(b"  already extracted  "))
            .unwrap();
        assert_eq!(out.text, "already extracted");
        assert_eq!(out.title, None);
    }

    #[test]
    fn the_builtin_refuses_a_class_it_does_not_own() {
        // It does not return an empty string for a document it cannot read. An
        // empty body is indistinguishable from "this page had nothing", which
        // is what `content_status` means, and would be a lie here.
        let error = Builtin
            .extract(&Document::pdf(b"%PDF-1.7"))
            .expect_err("the built-in extractor does not read PDFs");
        assert_eq!(error.to_string(), "builtin does not read pdf input");
    }

    #[test]
    fn no_reader_here_recovers_layout_so_markdown_is_always_absent() {
        // `None` is the honest answer, and a caller that needs a table has to
        // see the absence rather than be handed a flattened substitute.
        let out = Builtin
            .extract(&Document::html(
                b"<table><tr><td>a</td><td>b</td></tr></table>",
            ))
            .unwrap();
        assert!(out.markdown.is_none());
    }

    #[test]
    fn malformed_input_yields_text_or_nothing_but_never_an_error() {
        // The judgement here is that a broken page is not an exceptional
        // condition: pages are broken constantly, and a caller that had to
        // handle an Err per malformed document would end up swallowing it.
        // Emptiness is the signal instead, and `content_status` carries it.
        for (name, bytes) in [
            (
                "unterminated tag",
                &b"<html><body><p>Text<div class=\"x"[..],
            ),
            ("stray closing tag", &b"</p></div>Loose text</span>"[..]),
            (
                "angle brackets in prose",
                &b"<p>Use a &lt; b and c > d</p>"[..],
            ),
            ("nothing at all", &b""[..]),
            ("not html", &b"\x00\x01\x02 binary"[..]),
            ("unclosed comment", &b"<p>Before<!-- never closed"[..]),
        ] {
            let out = Builtin
                .extract(&Document::html(bytes))
                .unwrap_or_else(|e| panic!("{name} returned an error: {e}"));
            assert!(
                out.text.chars().count() <= TEXT_CAP,
                "{name} exceeded the cap"
            );
        }

        // Invalid UTF-8 is lossy-decoded rather than rejected: a page in the
        // wrong declared encoding is still mostly readable text.
        let out = Builtin
            .extract(&Document::html(b"<p>caf\xFF\xFE latte</p>"))
            .unwrap();
        assert!(out.text.contains("latte"), "got: {:?}", out.text);
    }

    #[test]
    fn escaped_markup_does_not_come_back_as_markup() {
        // Decoding `&amp;` first turns `&amp;lt;` into `&lt;`, which the next
        // replacement turns into a real `<`. A page that escaped its markup
        // for display would have it reconstituted one stage before the tag
        // walk eats it. Ordering is the fix, so ordering is what is tested.
        assert_eq!(
            decode_basic_entities("&amp;lt;script&amp;gt;"),
            "&lt;script&gt;"
        );
        assert_eq!(decode_basic_entities("Rust &amp; Zig"), "Rust & Zig");
        assert_eq!(decode_basic_entities("a &lt; b"), "a < b");
        assert_eq!(decode_basic_entities("path&#x2F;to"), "path/to");
        // Untouched when there is nothing to decode.
        assert_eq!(decode_basic_entities("plain text"), "plain text");
    }

    /// A numeric entity has a decimal form and a hex form for the same
    /// character, and a table that carries one is a table that decodes half
    /// its inputs. mckinsey.org emits `&#x27;` where the decimal `&#39;` was
    /// already covered, and the undecoded title reached a vault file name as
    /// `&-x27;` — the projection's sanitizer had turned the `#` into a dash.
    /// Both spellings of both characters are asserted so the pairing cannot
    /// drift apart again.
    #[test]
    fn decimal_and_hex_spellings_of_one_character_decode_alike() {
        for s in ["McKinsey.org&#39;s", "McKinsey.org&#x27;s"] {
            assert_eq!(decode_basic_entities(s), "McKinsey.org's", "input: {s}");
        }
        for s in ["path&#47;to", "path&#x2F;to"] {
            assert_eq!(decode_basic_entities(s), "path/to", "input: {s}");
        }
    }

    #[test]
    fn a_title_survives_entity_decoding_and_nested_markup() {
        let html = "<title>My &amp; <b>Page</b></title>";
        assert_eq!(html_title(html).as_deref(), Some("My & Page"));
    }
}
