//! Normalization: the stage between extraction and everything that reads an
//! item. Extraction's job ends at "faithful bytes to text"; this module owns
//! "clean". Both outputs are kept — `feed_items.raw_content` holds what the
//! extractor emitted, `transcript` holds what this produced — so changing a
//! rule re-runs over stored raw content instead of re-fetching the web.
//!
//! Every rule is a line predicate with a `drops` sentence attached. That
//! sentence is the inspectable part: `comms normalize --explain` prints the
//! table, and a re-run reports which rule fired how often. A rule that cannot
//! say what it throws away does not belong here.
//!
//! No model runs in this stage. Paraphrase is summarization and has its own
//! stage; a model that rewrites the body destroys the thing being stored.

/// Lines longer than this are never boilerplate candidates. Cookie banners and
/// share widgets are short labels; a paragraph that happens to discuss cookies
/// is prose, and dropping it would be the failure mode that matters here.
const BOILERPLATE_LINE_MAX: usize = 120;

/// A run of this many consecutive link-only lines reads as a navigation block
/// rather than as content. Two links in a row are still a citation pair.
const LINK_RUN_MIN: usize = 3;

/// One boilerplate rule. `drops` is not documentation — it is what the item's
/// own normalization report shows a human when they ask why text is missing.
pub struct Rule {
    pub name: &'static str,
    pub drops: &'static str,
    matches: fn(&str) -> bool,
}

/// The rule table, in application order. Order matters only in that earlier
/// rules see more text; none of them depend on another having run.
pub const RULES: &[Rule] = &[
    Rule {
        name: "cookie-notice",
        drops: "short lines offering or describing cookie consent",
        matches: is_cookie_notice,
    },
    Rule {
        name: "newsletter-interstitial",
        drops: "short lines asking the reader to subscribe or sign up",
        matches: is_newsletter_prompt,
    },
    Rule {
        name: "share-widget",
        drops: "short lines that are social-share or copy-link affordances",
        matches: is_share_widget,
    },
    Rule {
        name: "navigation",
        drops: "short lines that are bare navigation labels, not sentences",
        matches: is_navigation,
    },
];

/// What normalization did, per rule. Counts, not the dropped text: the raw
/// content is retained, so the dropped text is always one diff away.
#[derive(Debug, Clone, PartialEq)]
pub struct DropCount {
    pub rule: &'static str,
    pub lines: usize,
}

/// The canonical markdown plus the record of how it got that way.
#[derive(Debug, Clone, PartialEq)]
pub struct Normalized {
    pub text: String,
    pub dropped: Vec<DropCount>,
}

impl Normalized {
    /// One line per rule that fired, for the CLI and the re-run report.
    pub fn report(&self) -> String {
        if self.dropped.is_empty() {
            return "nothing dropped".to_string();
        }
        self.dropped
            .iter()
            .map(|d| format!("{}: {} line(s)", d.rule, d.lines))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// Turn extractor output into the canonical markdown every downstream consumer
/// reads. Idempotent: normalizing an already-normalized body is a no-op, which
/// is what makes a re-run over stored content safe to repeat.
pub fn normalize(raw: &str) -> Normalized {
    let mut counts: Vec<DropCount> = Vec::new();
    let mut kept: Vec<String> = Vec::new();

    for line in raw.lines() {
        // Order matters: inline noise is removed while its markup is still
        // intact, then the leftover tags go, then entities decode last so a
        // literal `&lt;` survives as text rather than being re-read as a tag.
        let line = decode_entities(&strip_markup(&drop_inline_noise(line.trim_end())));
        let probe = line.trim();

        if probe.is_empty() {
            kept.push(String::new());
            continue;
        }

        match RULES
            .iter()
            .find(|rule| probe.chars().count() <= BOILERPLATE_LINE_MAX && (rule.matches)(probe))
        {
            Some(rule) => bump(&mut counts, rule.name),
            None => kept.push(probe.to_string()),
        }
    }

    let (kept, link_runs) = drop_link_runs(kept);
    if link_runs > 0 {
        bump_by(&mut counts, "link-list-run", link_runs);
    }

    let (text, blanks) = collapse_blank_runs(kept);
    if blanks > 0 {
        bump_by(&mut counts, "blank-run", blanks);
    }

    Normalized {
        text,
        dropped: counts,
    }
}

/// The two rules that need more than a single line of context, documented here
/// because they are not in `RULES` and a reader would otherwise go looking.
///
/// - `link-list-run`: drops runs of ≥3 consecutive link-only lines
/// - `blank-run`: collapses runs of blank lines to one
pub fn structural_rules() -> [(&'static str, &'static str); 2] {
    [
        (
            "link-list-run",
            "runs of three or more consecutive lines that are nothing but a link",
        ),
        ("blank-run", "runs of blank lines, collapsed to one"),
    ]
}

fn bump(counts: &mut Vec<DropCount>, rule: &'static str) {
    bump_by(counts, rule, 1);
}

fn bump_by(counts: &mut Vec<DropCount>, rule: &'static str, n: usize) {
    match counts.iter_mut().find(|c| c.rule == rule) {
        Some(existing) => existing.lines += n,
        None => counts.push(DropCount { rule, lines: n }),
    }
}

/// Inline noise, dropped mid-line rather than by the line rules above:
///
/// - `<!-- ... -->` — HTML comments the extractor left behind
/// - `![alt](url)` — image markdown, which the reader has no image to show
/// - `[](url)` — links whose text is empty, i.e. an icon that lost its icon
fn drop_inline_noise(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;

    loop {
        let next = ["<!--", "![", "]("]
            .iter()
            .filter_map(|pat| rest.find(pat).map(|i| (i, *pat)))
            .min_by_key(|(i, _)| *i);

        let (at, pat) = match next {
            Some(hit) => hit,
            None => break,
        };

        match pat {
            "<!--" => {
                out.push_str(&rest[..at]);
                rest = match rest[at..].find("-->") {
                    Some(end) => &rest[at + end + 3..],
                    None => "",
                };
            }
            "![" => {
                out.push_str(&rest[..at]);
                rest = skip_link_tail(&rest[at + 2..]);
            }
            // An empty-text link: `[](url)`. Anything else is a real link.
            _ if at >= 1 && rest[..at].ends_with('[') => {
                out.push_str(&rest[..at - 1]);
                rest = skip_link_tail(&rest[at..]);
            }
            _ => {
                out.push_str(&rest[..at + 2]);
                rest = &rest[at + 2..];
            }
        }
    }

    out.push_str(rest);
    out
}

/// Consume up to and including the `)` that closes a markdown link target.
fn skip_link_tail(s: &str) -> &str {
    match s.find(')') {
        Some(end) => &s[end + 1..],
        None => "",
    }
}

/// Decode the entities an extractor leaves in text. This lives here rather
/// than in the reader because the summarizer and the embedder read the stored
/// text too, and a `&amp;` they see is a `&amp;` they score on (#86).
fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = s.to_string();
    for (entity, replacement) in [
        ("&amp;", "&"),
        ("&apos;", "'"),
        ("&quot;", "\""),
        ("&nbsp;", " "),
        ("&lt;", "<"),
        ("&gt;", ">"),
        ("&#39;", "'"),
        ("&hellip;", "…"),
        ("&mdash;", "—"),
        ("&ndash;", "–"),
    ] {
        if out.contains(entity) {
            out = out.replace(entity, replacement);
        }
    }
    out
}

/// Remove leftover `<...>` spans. The extractor already strips markup; what
/// survives to here is the unclosed and malformed remainder, which is exactly
/// what a markdown renderer would otherwise show verbatim.
fn strip_markup(s: &str) -> String {
    if !(s.contains('<') && s.contains('>')) {
        return s.to_string();
    }
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

fn is_cookie_notice(line: &str) -> bool {
    let low = line.to_lowercase();
    (low.contains("cookie") || low.contains("consent"))
        && (low.contains("accept")
            || low.contains("agree")
            || low.contains("settings")
            || low.contains("preferences")
            || low.contains("we use")
            || low.contains("akzeptieren")
            || low.contains("einstellungen"))
}

fn is_newsletter_prompt(line: &str) -> bool {
    let low = line.to_lowercase();
    low.contains("newsletter")
        || low.starts_with("subscribe")
        || low.contains("subscribe to our")
        || low.contains("sign up for")
        || low.contains("jetzt abonnieren")
}

fn is_share_widget(line: &str) -> bool {
    let low = line.to_lowercase();
    low.starts_with("share")
        || low.contains("share on ")
        || low.contains("share this")
        || low == "copy link"
        || low.contains("teilen auf")
}

/// A navigation label is a short line with no sentence in it. The sentence test
/// does the work: "Skip to content" is nav, "Skip the tutorial and read the
/// source." is prose, and length alone cannot tell them apart.
fn is_navigation(line: &str) -> bool {
    const LABELS: &[&str] = &[
        "home",
        "menu",
        "search",
        "skip to content",
        "skip to main content",
        "toggle navigation",
        "back to top",
        "main menu",
        "close",
        "log in",
        "sign in",
        "startseite",
        "zum inhalt springen",
    ];
    let low = line.to_lowercase();
    let low = low.trim_matches(|c: char| !c.is_alphanumeric());
    LABELS.contains(&low)
}

/// A line whose entire content is one markdown link or one bare URL.
fn is_link_only(line: &str) -> bool {
    let t = line.trim().trim_start_matches(['-', '*', '+']).trim();
    if t.starts_with("http://") || t.starts_with("https://") {
        return t.split_whitespace().nth(1).is_none();
    }
    t.starts_with('[') && t.ends_with(')') && t.contains("](") && t.matches('[').count() == 1
}

fn drop_link_runs(lines: Vec<String>) -> (Vec<String>, usize) {
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut dropped = 0usize;
    let mut run: Vec<String> = Vec::new();

    for line in lines {
        if is_link_only(&line) {
            run.push(line);
            continue;
        }
        flush_run(&mut run, &mut out, &mut dropped);
        out.push(line);
    }
    flush_run(&mut run, &mut out, &mut dropped);
    (out, dropped)
}

fn flush_run(run: &mut Vec<String>, out: &mut Vec<String>, dropped: &mut usize) {
    if run.len() >= LINK_RUN_MIN {
        *dropped += run.len();
    } else {
        out.append(run);
    }
    run.clear();
}

fn collapse_blank_runs(lines: Vec<String>) -> (String, usize) {
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut blanks = 0usize;
    let mut collapsed = 0usize;

    for line in lines {
        if line.trim().is_empty() {
            blanks += 1;
            if blanks == 1 && !out.is_empty() {
                out.push(String::new());
            } else {
                collapsed += 1;
            }
        } else {
            blanks = 0;
            out.push(line);
        }
    }

    let text = out.join("\n").trim().to_string();
    (text, collapsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fired(n: &Normalized, rule: &str) -> usize {
        n.dropped
            .iter()
            .find(|d| d.rule == rule)
            .map(|d| d.lines)
            .unwrap_or(0)
    }

    #[test]
    fn html_extraction_hands_the_normalizer_line_structure() {
        // The one test that crosses the extraction/normalize seam, so it stays
        // on this side of it after the readers moved to `libs/extraction`:
        // what it asserts is that this module's input still arrives shaped the
        // way its rules need.
        //
        // The normalizer is a line predicate table, so extraction owes it
        // lines. Collapsing a page to one paragraph made every rule
        // unreachable and shipped LinkedIn's cookie banner as article text.
        use crate::extraction::{Builtin, Document, Extractor};

        let html = "<html><head><title>T</title></head><body>\
            <nav><a href=\"/a\">Home</a><a href=\"/b\">Jobs</a></nav>\
            <div id=\"consent\"><p>Accept all cookies</p>\
            <p>Reject non-essential cookies</p></div>\
            <article><p>Transit data should remain inspectable, which is the \
            single claim this article exists to make.</p></article>\
            </body></html>";

        let out = Builtin.extract(&Document::html(html.as_bytes())).unwrap();
        let clean = normalize(&out.text);

        assert!(
            !clean.text.contains("Accept all cookies"),
            "consent banner survived normalization: {}",
            clean.text
        );
        assert!(
            clean
                .text
                .contains("Transit data should remain inspectable"),
            "the article body must survive: {}",
            clean.text
        );
        assert_eq!(out.title.as_deref(), Some("T"));
        assert_eq!(out.producer, "builtin");
    }

    #[test]
    fn cookie_notice_goes_and_prose_about_cookies_stays() {
        let out = normalize(
            "We use cookies to improve your experience. Accept all\n\
             The cookie consent industry is worth billions, and this article argues that the \
             entire apparatus exists to manufacture the appearance of choice rather than to \
             give anyone real control over what is collected.",
        );
        assert_eq!(fired(&out, "cookie-notice"), 1);
        assert!(out.text.starts_with("The cookie consent industry"));
    }

    #[test]
    fn newsletter_prompt_goes() {
        let out = normalize("Real paragraph.\nSubscribe to our newsletter\nAnother paragraph.");
        assert_eq!(fired(&out, "newsletter-interstitial"), 1);
        assert_eq!(out.text, "Real paragraph.\nAnother paragraph.");
    }

    #[test]
    fn share_widget_goes() {
        let out = normalize("Body text.\nShare on Twitter\nCopy link\nMore body text.");
        assert_eq!(fired(&out, "share-widget"), 2);
        assert_eq!(out.text, "Body text.\nMore body text.");
    }

    #[test]
    fn navigation_label_goes_but_a_sentence_starting_with_one_stays() {
        let out = normalize("Home\nHome automation is the subject of this piece.");
        assert_eq!(fired(&out, "navigation"), 1);
        assert_eq!(out.text, "Home automation is the subject of this piece.");
    }

    #[test]
    fn link_run_goes_and_a_pair_stays() {
        let run = normalize("Intro.\n[a](https://a.example)\n[b](https://b.example)\n[c](https://c.example)\nOutro.");
        assert_eq!(fired(&run, "link-list-run"), 3);
        assert_eq!(run.text, "Intro.\nOutro.");

        let pair = normalize("Intro.\n[a](https://a.example)\n[b](https://b.example)\nOutro.");
        assert_eq!(fired(&pair, "link-list-run"), 0);
        assert!(pair.text.contains("[a](https://a.example)"));
    }

    #[test]
    fn blank_runs_collapse_and_residual_markup_is_stripped() {
        let out =
            normalize("\n\n# Header\n\n\n\nParagraph with <span class=\"x\">tag</span>.\n   \n\n");
        assert_eq!(out.text, "# Header\n\nParagraph with tag.");
        assert!(fired(&out, "blank-run") > 0);
    }

    #[test]
    fn inline_noise_and_entities_are_handled_here_not_in_the_reader() {
        let out = normalize(
            "Tom &amp; Jerry <!-- tracking pixel --> went ![logo](https://x.example/l.png) home[](https://x.example/i).",
        );
        assert_eq!(out.text, "Tom & Jerry  went  home.");
    }

    #[test]
    fn a_real_link_survives_the_inline_pass() {
        let out = normalize("See [the paper](https://arxiv.org/abs/1234.5678) for details.");
        assert_eq!(
            out.text,
            "See [the paper](https://arxiv.org/abs/1234.5678) for details."
        );
    }

    #[test]
    fn normalizing_twice_changes_nothing() {
        let once = normalize(
            "Menu\nWe use cookies. Accept all\n\n\nReal content here.\n[a](https://a.example)\n[b](https://b.example)\n[c](https://c.example)\n",
        );
        let twice = normalize(&once.text);
        assert_eq!(once.text, twice.text);
        assert!(twice.dropped.is_empty(), "second pass should find nothing");
    }

    #[test]
    fn every_rule_states_what_it_drops() {
        for rule in RULES {
            assert!(
                !rule.drops.is_empty(),
                "{} has no drops sentence",
                rule.name
            );
        }
        for (name, drops) in structural_rules() {
            assert!(!drops.is_empty(), "{name} has no drops sentence");
        }
    }

    #[test]
    fn report_reads_as_an_explanation() {
        let out = normalize("Menu\nBody.");
        assert_eq!(out.report(), "navigation: 1 line(s)");
        assert_eq!(normalize("Body.").report(), "nothing dropped");
    }
}
