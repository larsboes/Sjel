//! Rung 0 of the redaction ladder (PRD §6.2, §6.2c): a dictionary of names the operator
//! already knows, matched in linear time.
//!
//! Matching runs over a *folded* copy of the text, so `JÖRG MÜLLER`, `Jörg Müller` and
//! `Joerg Mueller` are one entry. [`fold`] is the only definition of "the same spelling".

use crate::types::EntityType;
use aho_corasick::{AhoCorasick, MatchKind};

/// One dictionary or pattern hit, as a byte range of the original text.
///
/// `Debug` is written by hand and prints the range and kind only: `original_matched` is the
/// personal value itself, and a `{:?}` in a log line must not print it.
#[derive(Clone)]
pub struct MatchSpan {
    pub start: usize,
    pub end: usize,
    pub entity_type: EntityType,
    pub original_matched: String,
}

impl std::fmt::Debug for MatchSpan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MatchSpan")
            .field("start", &self.start)
            .field("end", &self.end)
            .field("entity_type", &self.entity_type)
            .field("original_matched", &"<redacted>")
            .finish()
    }
}

/// Case and spelling folding for dictionary matching.
///
/// Unicode lowercase (not ASCII-only, which left `MÜNCHEN` unmatched against `München`),
/// then the German transliterations a sender types when the keyboard has no umlaut:
/// `ß`→`ss`, `ä`→`ae`, `ö`→`oe`, `ü`→`ue`. Decomposed umlauts (`u` + U+0308) fold the
/// same way. Both dictionary entries and text pass through this function.
pub fn fold(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(c) = chars.next() {
        let decomposed_umlaut = is_umlaut_base(c) && chars.peek() == Some(&'\u{0308}');
        if decomposed_umlaut {
            chars.next();
        }
        push_folded_sequence(&mut out, c, decomposed_umlaut);
    }
    out
}

fn is_umlaut_base(c: char) -> bool {
    matches!(c.to_lowercase().next(), Some('a' | 'o' | 'u'))
}

fn push_folded_sequence(out: &mut String, c: char, decomposed_umlaut: bool) {
    if decomposed_umlaut {
        match c.to_lowercase().next() {
            Some('a') => out.push_str("ae"),
            Some('o') => out.push_str("oe"),
            Some('u') => out.push_str("ue"),
            _ => push_folded(out, c),
        }
    } else {
        push_folded(out, c);
    }
}

fn push_folded(out: &mut String, c: char) {
    match c {
        'ß' | 'ẞ' => out.push_str("ss"),
        'ä' | 'Ä' => out.push_str("ae"),
        'ö' | 'Ö' => out.push_str("oe"),
        'ü' | 'Ü' => out.push_str("ue"),
        _ => out.extend(c.to_lowercase()),
    }
}

/// The folded text plus a map from folded byte offsets back to original byte offsets.
/// `boundary[i]` is `Some(original)` when folded offset `i` starts the expansion of one
/// original character (or is the end of the text), and `None` inside an expansion — a match
/// that starts or ends inside `ss` from one `ß` is not a match of whole characters.
struct Folded {
    text: String,
    boundary: Vec<Option<usize>>,
}

fn fold_with_offsets(value: &str) -> Folded {
    let mut text = String::with_capacity(value.len());
    let mut boundary = Vec::with_capacity(value.len() + 1);
    let mut chars = value.char_indices().peekable();
    while let Some((index, c)) = chars.next() {
        let decomposed_umlaut =
            is_umlaut_base(c) && chars.peek().is_some_and(|(_, next)| *next == '\u{0308}');
        if decomposed_umlaut {
            chars.next();
        }
        let before = text.len();
        push_folded_sequence(&mut text, c, decomposed_umlaut);
        boundary.push(Some(index));
        boundary.extend(std::iter::repeat_n(None, text.len() - before - 1));
    }
    boundary.push(Some(value.len()));
    Folded { text, boundary }
}

#[derive(Default)]
pub struct EntityRegistryBuilder {
    entries: Vec<(String, EntityType)>,
}

impl EntityRegistryBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_person(mut self, name: impl Into<String>) -> Self {
        let n = name.into().trim().to_string();
        if !n.is_empty() {
            self.entries.push((n, EntityType::Person));
        }
        self
    }

    pub fn add_people<I, S>(mut self, names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        for name in names {
            self = self.add_person(name);
        }
        self
    }

    pub fn add_place(mut self, place: impl Into<String>) -> Self {
        let p = place.into().trim().to_string();
        if !p.is_empty() {
            self.entries.push((p, EntityType::Place));
        }
        self
    }

    pub fn add_places<I, S>(mut self, places: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        for place in places {
            self = self.add_place(place);
        }
        self
    }

    pub fn build(self) -> EntityRegistry {
        // First entry wins on a folded duplicate, so the kind a caller added first is kept.
        let mut patterns: Vec<String> = Vec::with_capacity(self.entries.len());
        let mut kinds = Vec::with_capacity(self.entries.len());
        for (term, kind) in self.entries {
            let folded = fold(&term);
            if folded.is_empty() || patterns.contains(&folded) {
                continue;
            }
            patterns.push(folded);
            kinds.push(kind);
        }

        // `Standard` plus overlapping search, not `LeftmostLongest`: a leftmost match that
        // then fails the word-boundary test must not hide an overlapping one that passes.
        // The leftmost-longest choice is made below, over the matches that survive.
        let aut = if patterns.is_empty() {
            None
        } else {
            AhoCorasick::builder()
                .match_kind(MatchKind::Standard)
                .build(&patterns)
                .ok()
        };

        EntityRegistry {
            aut,
            patterns,
            kinds,
        }
    }
}

pub struct EntityRegistry {
    aut: Option<AhoCorasick>,
    patterns: Vec<String>,
    kinds: Vec<EntityType>,
}

impl EntityRegistry {
    pub fn builder() -> EntityRegistryBuilder {
        EntityRegistryBuilder::new()
    }

    pub fn is_empty(&self) -> bool {
        self.patterns.is_empty()
    }

    pub fn len(&self) -> usize {
        self.patterns.len()
    }

    /// Finds non-overlapping, leftmost-longest whole-word dictionary matches in `text`.
    ///
    /// Word rules, each chosen for a case that went wrong or could:
    /// - A letter or digit on either side is no match: `Anna` is not in `Hannah`.
    /// - A hyphen joins a compound, and the match grows to the whole compound: `Anna-Lena`
    ///   becomes one entity, never `<TRAVELER_01>-Lena`, which would leak half a name and
    ///   claim Anna-Lena is Anna.
    /// - A person followed by one `s` still matches, and the `s` stays outside the span:
    ///   German genitive `Annas` becomes `<TRAVELER_01>s`, the same token as `Anna`. Only for
    ///   people, because `Hbfs` is not a word and a place list holds common nouns more often.
    pub fn find_matches(&self, text: &str) -> Vec<MatchSpan> {
        let Some(ref aut) = self.aut else {
            return Vec::new();
        };
        let folded = fold_with_offsets(text);

        let mut candidates: Vec<MatchSpan> = Vec::new();
        for mat in aut.find_overlapping_iter(&folded.text) {
            let (Some(start), Some(end)) =
                (folded.boundary[mat.start()], folded.boundary[mat.end()])
            else {
                continue;
            };
            let kind = self.kinds[mat.pattern().as_usize()];
            if let Some((start, end)) = word_span(text, start, end, kind) {
                candidates.push(MatchSpan {
                    start,
                    end,
                    entity_type: kind,
                    original_matched: text[start..end].to_string(),
                });
            }
        }

        candidates.sort_by(|a, b| a.start.cmp(&b.start).then(b.end.cmp(&a.end)));
        let mut results: Vec<MatchSpan> = Vec::with_capacity(candidates.len());
        for candidate in candidates {
            if results
                .last()
                .is_some_and(|last| candidate.start < last.end)
            {
                continue;
            }
            results.push(candidate);
        }
        results
    }
}

/// Applies the word rules of [`EntityRegistry::find_matches`] to one raw hit, returning the
/// span to replace or `None` when the hit sits inside a longer word.
fn word_span(text: &str, start: usize, end: usize, kind: EntityType) -> Option<(usize, usize)> {
    let before = text[..start].chars().next_back();
    let after = text[end..].chars().next();
    let mut span = (start, end);

    match before {
        Some(c) if c.is_alphanumeric() => return None,
        Some('-') => {
            let head = &text[..start - 1];
            if head.chars().next_back().is_some_and(char::is_alphanumeric) {
                span.0 = compound_start(head);
            }
        }
        _ => {}
    }

    match after {
        Some('-') => {
            let tail = &text[end + 1..];
            if tail.chars().next().is_some_and(char::is_alphanumeric) {
                span.1 = end + 1 + compound_len(tail);
            }
        }
        Some(c) if c.is_alphanumeric() => {
            let possessive = kind == EntityType::Person
                && c == 's'
                && !text[end + 1..]
                    .chars()
                    .next()
                    .is_some_and(|n| n.is_alphanumeric() || n == '-');
            if !possessive {
                return None;
            }
        }
        _ => {}
    }
    Some(span)
}

/// Byte offset where the hyphenated compound ending at `head`'s end begins.
fn compound_start(head: &str) -> usize {
    let mut start = head.len();
    for (index, c) in head.char_indices().rev() {
        if c.is_alphanumeric() || c == '-' {
            start = index;
        } else {
            break;
        }
    }
    // A compound does not begin with a hyphen.
    start + head[start..].len() - head[start..].trim_start_matches('-').len()
}

/// Byte length of the hyphenated compound at the start of `tail`.
fn compound_len(tail: &str) -> usize {
    let mut len = 0;
    for (index, c) in tail.char_indices() {
        if c.is_alphanumeric() || c == '-' {
            len = index + c.len_utf8();
        } else {
            break;
        }
    }
    // A compound does not end with a hyphen.
    tail[..len].trim_end_matches('-').len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn people(names: &[&str]) -> EntityRegistry {
        EntityRegistry::builder()
            .add_people(names.iter().copied())
            .build()
    }

    fn matched<'a>(registry: &EntityRegistry, text: &'a str) -> Vec<&'a str> {
        registry
            .find_matches(text)
            .into_iter()
            .map(|m| &text[m.start..m.end])
            .collect()
    }

    #[test]
    fn unicode_case_and_umlaut_spellings_match_one_entry() {
        let registry = EntityRegistry::builder()
            .add_person("Jörg Müller")
            .add_place("München Hbf")
            .add_place("Großstraße")
            .build();
        assert_eq!(matched(&registry, "JÖRG MÜLLER kommt"), ["JÖRG MÜLLER"]);
        assert_eq!(matched(&registry, "ab MÜNCHEN HBF"), ["MÜNCHEN HBF"]);
        assert_eq!(
            matched(&registry, "Joerg Mueller, Muenchen Hbf"),
            ["Joerg Mueller", "Muenchen Hbf"]
        );
        assert_eq!(matched(&registry, "in der Grossstrasse"), ["Grossstrasse"]);
    }

    #[test]
    fn composed_and_decomposed_umlauts_match_and_preserve_original_spans() {
        let registry = EntityRegistry::builder()
            .add_person("Jörg Müller")
            .add_place("München Hbf")
            .add_person("Jo\u{0308}rg Mu\u{0308}ller")
            .build();
        let text = "JÖRG MU\u{0308}LLER kommt aus Mu\u{0308}nchen Hbf";
        assert_eq!(
            matched(&registry, text),
            ["JÖRG MU\u{0308}LLER", "Mu\u{0308}nchen Hbf"]
        );
        assert_eq!(fold("Jörg Jo\u{0308}rg"), "joerg joerg");
    }

    #[test]
    fn a_hyphenated_compound_is_one_entity_not_half_a_name() {
        let registry = people(&["Anna"]);
        assert_eq!(matched(&registry, "Anna-Lena kommt"), ["Anna-Lena"]);
        assert_eq!(matched(&registry, "Marie-Anna kommt"), ["Marie-Anna"]);
        assert_eq!(matched(&registry, "Anna - Lena"), ["Anna"]);
    }

    #[test]
    fn a_genitive_s_keeps_the_person_and_leaves_the_s_outside() {
        let registry = people(&["Anna"]);
        let text = "Annas Wohnung";
        let spans = registry.find_matches(text);
        assert_eq!(spans.len(), 1);
        assert_eq!(&text[spans[0].start..spans[0].end], "Anna");
        assert!(matched(&registry, "Annasee").is_empty());
        assert!(matched(&registry, "Hannah").is_empty());
    }

    #[test]
    fn a_failed_leftmost_hit_does_not_hide_a_valid_overlapping_one() {
        let registry = EntityRegistry::builder()
            .add_places(["Bonn", "Bonnie"])
            .build();
        assert_eq!(matched(&registry, "Bonnies Bonn"), ["Bonn"]);
    }

    #[test]
    fn debug_output_does_not_print_the_matched_value() {
        let registry = people(&["Anna"]);
        let spans = registry.find_matches("Anna");
        let printed = format!("{:?}", spans);
        assert!(!printed.contains("Anna"), "{printed}");
    }
}
