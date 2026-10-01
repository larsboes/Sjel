//! The bidirectional session symbol table (PRD §6.2c).
//!
//! One session is one conversation with one remote evaluator: every value it replaced, the
//! token it issued, and the way back. A token's number depends on what the session has
//! already seen, so anything that must hash the same way twice — a cloud derivative preview
//! (`capabilities/comms/src/cloud_derivative.rs`, `prepare_pseudonymized`) — starts from a
//! fresh session.

use aho_corasick::{AhoCorasick, MatchKind};
use serde_json::Value;
use std::collections::HashMap;

use crate::pattern;
use crate::registry::{EntityRegistry, MatchSpan};
use crate::types::{compute_digest, format_receipt, EntityType, RedactionFinding};

/// Prefix of the tokens that stand for token-shaped text already present in the source.
const LITERAL_PREFIX: &str = "LITERAL";

/// `Clone` and `Default`, and a hand-written `Debug`: the maps hold the personal values this
/// type exists to keep off the wire, and a derived `Debug` would print every one of them into
/// the first log line that formats a session.
#[derive(Clone, Default)]
pub struct PseudonymizerSession {
    /// Original value, exactly as written → issued token.
    forward: HashMap<String, String>,
    /// Issued token → original value, exactly as written.
    reverse: HashMap<String, String>,
    counters: HashMap<EntityType, usize>,
    literals: usize,
    /// Occurrences replaced since the session began or since the last [`Self::take_findings`].
    findings: Vec<RedactionFinding>,
    /// Set for a keyed session ([`Self::keyed`]): tokens come from the key, not a counter.
    key: Option<[u8; 32]>,
}

impl std::fmt::Debug for PseudonymizerSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PseudonymizerSession")
            .field("tokens", &self.forward.len())
            .field("findings", &self.findings)
            .finish_non_exhaustive()
    }
}

impl PseudonymizerSession {
    pub fn new() -> Self {
        Self::default()
    }

    /// A session whose tokens are a function of `key`, the entity type and the value.
    ///
    /// Two keyed sessions with one key issue the same token for the same value, in any
    /// process and in any order (ISA F9, ISC-41). Use [`crate::keyed::session_key`] to derive
    /// the key; never pass a key an agent can read.
    pub fn keyed(key: [u8; 32]) -> Self {
        Self {
            key: Some(key),
            ..Self::default()
        }
    }

    pub fn forward_map(&self) -> &HashMap<String, String> {
        &self.forward
    }

    pub fn reverse_map(&self) -> &HashMap<String, String> {
        &self.reverse
    }

    /// What was replaced since the session began or since the last [`Self::take_findings`],
    /// counted in occurrences: `Lars … Lars` is two mentions of a person.
    pub fn findings(&self) -> &[RedactionFinding] {
        &self.findings
    }

    /// Returns the findings of the call that just ended and starts counting the next one.
    ///
    /// PRD Q9b asks for a receipt on every reduced *call*. A session outlives a call, so the
    /// caller marks where a call ends; without this, the second call's receipt would repeat
    /// the first call's removals.
    pub fn take_findings(&mut self) -> Vec<RedactionFinding> {
        std::mem::take(&mut self.findings)
    }

    /// Distinct tokens issued, escapes of literal token-shaped text included.
    pub fn token_count(&self) -> usize {
        // Forward, not reverse: an alias adds a way back but issues no token.
        self.forward.len()
    }

    /// Q9b's receipt for the current call (see [`Self::findings`]), or `None` when nothing
    /// was replaced.
    pub fn receipt(&self) -> Option<String> {
        format_receipt(&self.findings)
    }

    pub fn digest(&self) -> String {
        compute_digest(&self.findings)
    }

    fn record_finding(&mut self, entity_type: EntityType) {
        if let Some(finding) = self
            .findings
            .iter_mut()
            .find(|f| f.entity_type == entity_type)
        {
            finding.count += 1;
        } else {
            self.findings.push(RedactionFinding {
                entity_type,
                count: 1,
            });
        }
    }

    /// The token for `original`, issued on first sight.
    ///
    /// Keyed on the exact spelling, so rehydration gives back the text that was there:
    /// `LARS` and `Lars` are two tokens, and each restores its own case. A folded key would
    /// merge them and rewrite one spelling into the other on the way back.
    fn token_for(&mut self, original: &str, entity_type: EntityType) -> String {
        if let Some(existing) = self.forward.get(original) {
            return existing.clone();
        }
        if self.key.is_some() {
            return self.keyed_token_for(original, entity_type.token_prefix());
        }
        let counter = self.counters.entry(entity_type).or_insert(0);
        *counter += 1;
        let token = format!("<{}_{:02}>", entity_type.token_prefix(), *counter);
        self.issue(original, token)
    }

    fn literal_token_for(&mut self, original: &str) -> String {
        if let Some(existing) = self.forward.get(original) {
            return existing.clone();
        }
        if self.key.is_some() {
            return self.keyed_token_for(original, LITERAL_PREFIX);
        }
        self.literals += 1;
        let token = format!("<{LITERAL_PREFIX}_{:02}>", self.literals);
        self.issue(original, token)
    }

    /// Six base32 characters, lengthened only if this session already issued the same
    /// token for another value. The lengthened form is still a function of the value, but
    /// it depends on the collision, so it is the one case where two processes can disagree.
    fn keyed_token_for(&mut self, original: &str, prefix: &str) -> String {
        let key = self
            .key
            .expect("keyed_token_for is called only on a keyed session");
        let mut chars = 6;
        loop {
            let token = format!(
                "<{prefix}_{}>",
                crate::keyed::suffix(&key, prefix, original, chars)
            );
            match self.reverse.get(&token) {
                Some(other) if other != original && chars < 26 => chars += 4,
                _ => return self.issue(original, token),
            }
        }
    }

    fn issue(&mut self, original: &str, token: String) -> String {
        self.forward.insert(original.to_string(), token.clone());
        self.reverse.insert(token.clone(), original.to_string());
        token
    }

    /// Makes `rendered` rehydrate to `original` as a whole.
    ///
    /// For a token shown with context beside it, `<SENDER_k3x9qa> (dhl.de)`: an agent that
    /// copies the whole field back must get the original field, not the original plus
    /// ` (dhl.de)`. Rehydration matches leftmost-longest, so the alias wins over its token.
    pub fn alias(&mut self, rendered: &str, original: &str) {
        self.reverse
            .insert(rendered.to_string(), original.to_string());
    }

    /// Replaces the whole of `value` with one token of `entity_type`, whatever it contains.
    ///
    /// For fields that are an identity as a unit — a mail's author line. Tokenizing such a
    /// field part by part leaves every part no rule recognises: `Alice <alice@example.com>`
    /// became `Alice <EMAIL_01>`. Returns `value` unchanged when it is blank.
    pub fn tokenize_whole(&mut self, value: &str, entity_type: EntityType) -> String {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return value.to_string();
        }
        let token = self.token_for(trimmed, entity_type);
        self.record_finding(entity_type);
        token
    }

    /// Tokenizes `text` with the ladder of PRD §6.2: rung 1 shapes, then rung 0 names from
    /// `registry`, then rung 2 cues on what is left.
    pub fn tokenize_text(&mut self, text: &str, registry: &EntityRegistry) -> String {
        if text.trim().is_empty() {
            return text.to_string();
        }

        let word_spans = word_spans(text);
        let words: Vec<&str> = word_spans.iter().map(|&(s, e)| &text[s..e]).collect();
        let mut spans: Vec<MatchSpan> = Vec::new();

        // Rung 1: word-level shapes. The shared detectors are the ones the destructive path
        // uses (`pattern`), in the same order.
        for &(start, end) in &word_spans {
            let clean = trim_trailing_punctuation(&text[start..end]);
            let lowered = clean.to_ascii_lowercase();
            let detected = if pattern::looks_like_url(&lowered) {
                Some(EntityType::Link)
            } else if pattern::looks_like_email(clean) {
                Some(EntityType::Email)
            } else if pattern::looks_like_iban(clean) {
                Some(EntityType::Iban)
            } else if pattern::looks_like_phone(clean) {
                Some(EntityType::Phone)
            } else if pattern::looks_like_token(clean) {
                Some(EntityType::Secret)
            } else if pattern::looks_like_sensitive_number(clean) {
                Some(EntityType::Number)
            } else {
                None
            };
            if let Some(kind) = detected {
                spans.push(MatchSpan {
                    start,
                    end: start + clean.len(),
                    entity_type: kind,
                    original_matched: clean.to_string(),
                });
            }
        }

        // Rung 0: the dictionary, wherever rung 1 did not already claim the text.
        for m in registry.find_matches(text) {
            if !spans
                .iter()
                .any(|s| overlaps(s.start, s.end, m.start, m.end))
            {
                spans.push(m);
            }
        }

        // Rung 2: cues on the words left over. A person found by any rung cues the next word,
        // so "Erika Mustermann" is caught when only "Erika" is in the dictionary — the same
        // tail rule as the destructive path.
        let mut person_tail = false;
        for (i, &(start, end)) in word_spans.iter().enumerate() {
            if let Some(hit) = spans.iter().find(|s| overlaps(s.start, s.end, start, end)) {
                person_tail = hit.entity_type == EntityType::Person;
                continue;
            }
            let clean = trim_trailing_punctuation(&text[start..end]);
            let cued = person_tail || pattern::introduces_person(&words, i);
            let is_person = pattern::names_a_person_in_apposition(&words, i)
                || (cued
                    && (pattern::looks_like_person_name(clean)
                        || pattern::looks_like_handle(clean)));
            if is_person {
                spans.push(MatchSpan {
                    start,
                    end: start + clean.len(),
                    entity_type: EntityType::Person,
                    original_matched: clean.to_string(),
                });
            }
            person_tail = is_person;
        }

        spans.sort_by(|a, b| a.start.cmp(&b.start).then(b.end.cmp(&a.end)));
        let mut result = String::with_capacity(text.len());
        let mut cursor = 0;
        for span in spans {
            if span.start < cursor {
                continue;
            }
            self.push_escaped(&mut result, &text[cursor..span.start]);
            let token = self.token_for(&span.original_matched, span.entity_type);
            self.record_finding(span.entity_type);
            result.push_str(&token);
            cursor = span.end;
        }
        self.push_escaped(&mut result, &text[cursor..]);
        result
    }

    /// Copies source text that no rule replaced, escaping any token-shaped part of it.
    ///
    /// A mail that already contains `<EMAIL_01>` as text would otherwise travel as-is, and
    /// rehydration would turn it into whichever real address the session issued that token
    /// for. The literal gets a `<LITERAL_nn>` token of its own and comes back as itself.
    /// Not a finding: nothing personal was removed.
    fn push_escaped(&mut self, out: &mut String, segment: &str) {
        let mut cursor = 0;
        while let Some((start, end)) = next_token_shape(segment, cursor) {
            out.push_str(&segment[cursor..start]);
            let token = self.literal_token_for(&segment[start..end]);
            out.push_str(&token);
            cursor = end;
        }
        out.push_str(&segment[cursor..]);
    }

    /// Rehydrates tokenized text back to original values in a single linear pass.
    ///
    /// Tokens match ASCII-case-insensitively, because a model may answer `<traveler_01>`.
    /// The value put back is the original spelling, so the case of the personal value is
    /// the case it arrived in.
    pub fn rehydrate_text(&self, text: &str) -> String {
        if self.reverse.is_empty() || text.is_empty() {
            return text.to_string();
        }
        let (patterns, replacements): (Vec<&str>, Vec<&str>) = self
            .reverse
            .iter()
            .map(|(token, original)| (token.as_str(), original.as_str()))
            .unzip();
        match AhoCorasick::builder()
            .ascii_case_insensitive(true)
            .match_kind(MatchKind::LeftmostLongest)
            .build(&patterns)
        {
            Ok(aut) => aut.replace_all(text, &replacements),
            // Unreachable for a table of short ASCII tokens. Written as a refusal to rehydrate
            // rather than a sequential `replace`, which would rewrite its own output.
            Err(_) => text.to_string(),
        }
    }

    /// Recursively tokenizes all string values within a JSON structure while keeping object
    /// keys intact.
    pub fn tokenize_json(&mut self, value: &mut Value, registry: &EntityRegistry) {
        match value {
            Value::String(s) => *s = self.tokenize_text(s, registry),
            Value::Array(items) => {
                for item in items {
                    self.tokenize_json(item, registry);
                }
            }
            Value::Object(map) => {
                for val in map.values_mut() {
                    self.tokenize_json(val, registry);
                }
            }
            _ => {}
        }
    }

    /// Recursively rehydrates all string values within a JSON structure.
    pub fn rehydrate_json(&self, value: &mut Value) {
        match value {
            Value::String(s) => *s = self.rehydrate_text(s),
            Value::Array(items) => {
                for item in items {
                    self.rehydrate_json(item);
                }
            }
            Value::Object(map) => {
                for val in map.values_mut() {
                    self.rehydrate_json(val);
                }
            }
            _ => {}
        }
    }
}

fn overlaps(a_start: usize, a_end: usize, b_start: usize, b_end: usize) -> bool {
    a_start < b_end && b_start < a_end
}

fn trim_trailing_punctuation(word: &str) -> &str {
    word.trim_end_matches(['.', ',', ';', '!', '?', ':'])
}

/// Byte ranges of the whitespace-separated words of `text`, as `split_whitespace` sees them.
fn word_spans(text: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut start = None;
    for (index, c) in text.char_indices() {
        match (c.is_whitespace(), start) {
            (true, Some(s)) => {
                spans.push((s, index));
                start = None;
            }
            (false, None) => start = Some(index),
            _ => {}
        }
    }
    if let Some(s) = start {
        spans.push((s, text.len()));
    }
    spans
}

/// The next `<LETTERS_ALNUM>` substring at or after `from`: the shape of every token this
/// crate issues, in either case.
fn next_token_shape(text: &str, from: usize) -> Option<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut i = from;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            if let Some(end) = token_shape_end(bytes, i) {
                return Some((i, end));
            }
        }
        i += 1;
    }
    None
}

fn token_shape_end(bytes: &[u8], open: usize) -> Option<usize> {
    let mut i = open + 1;
    let letters = i;
    while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
        i += 1;
    }
    if i == letters || bytes.get(i) != Some(&b'_') {
        return None;
    }
    i += 1;
    let tail = i;
    while i < bytes.len() && bytes[i].is_ascii_alphanumeric() {
        i += 1;
    }
    (i > tail && bytes.get(i) == Some(&b'>')).then_some(i + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> EntityRegistry {
        EntityRegistry::builder()
            .add_people(["Anna", "Jörg Müller"])
            .add_places(["München Hbf"])
            .build()
    }

    #[test]
    fn a_literal_token_in_the_source_cannot_rehydrate_into_a_real_value() {
        let mut session = PseudonymizerSession::new();
        let registry = registry();
        let text = "Reply to anna@example.com, not to <EMAIL_01> or <email_01>.";
        let out = session.tokenize_text(text, &registry);
        assert_eq!(out.matches("<EMAIL_01>").count(), 1, "{out}");
        assert!(!out.contains("<email_01>"), "{out}");
        assert_eq!(session.rehydrate_text(&out), text);
        // The escape is not a removal, so the receipt names the email only.
        assert_eq!(
            session.receipt().as_deref(),
            Some("Reduced 1 detail before this call: 1 email address.")
        );
    }

    #[test]
    fn rehydration_restores_each_spelling_in_its_own_case() {
        let mut session = PseudonymizerSession::new();
        let text = "JÖRG MÜLLER and Jörg Müller, ab MÜNCHEN HBF";
        let out = session.tokenize_text(text, &registry());
        assert!(!out.to_lowercase().contains("jörg"), "{out}");
        assert!(!out.to_lowercase().contains("münchen"), "{out}");
        assert_eq!(session.rehydrate_text(&out), text);
    }

    #[test]
    fn decomposed_umlauts_are_tokenized_and_rehydrated_without_normalizing_source() {
        let registry = EntityRegistry::builder().add_person("Jörg Müller").build();
        let original = "JÖRG MU\u{0308}LLER schreibt";
        let mut session = PseudonymizerSession::new();
        let tokenized = session.tokenize_text(original, &registry);
        assert_eq!(tokenized, "<TRAVELER_01> schreibt");
        assert_eq!(session.rehydrate_text(&tokenized), original);
    }

    #[test]
    fn a_compound_and_a_genitive_do_not_leak_half_a_name() {
        let mut session = PseudonymizerSession::new();
        let out = session.tokenize_text("Anna-Lena und Annas neue Wohnung", &registry());
        assert!(!out.contains("Lena"), "{out}");
        assert!(!out.contains("Anna"), "{out}");
        assert!(out.contains("<TRAVELER_02>s neue"), "{out}");
    }

    #[test]
    fn findings_count_occurrences_per_call() {
        let mut session = PseudonymizerSession::new();
        let registry = registry();
        session.tokenize_text("Anna and Anna", &registry);
        assert_eq!(
            session.receipt().as_deref(),
            Some("Reduced 2 details before this call: 2 mentions of people.")
        );
        let first = session.take_findings();
        assert_eq!(first[0].count, 2);
        session.tokenize_text("Anna again", &registry);
        assert_eq!(
            session.receipt().as_deref(),
            Some("Reduced 1 detail before this call: 1 mention of a person.")
        );
    }

    #[test]
    fn the_number_probes_the_old_detector_leaked_are_replaced() {
        for (text, secret) in [
            ("Ref #1234567", "1234567"),
            ("Tel. 0721/1234567", "0721/1234567"),
            ("Konto 12.345.678", "12.345.678"),
            ("geb. 01.02.1990", "01.02.1990"),
        ] {
            let mut session = PseudonymizerSession::new();
            let out = session.tokenize_text(text, &EntityRegistry::builder().build());
            assert!(!out.contains(secret), "{text} -> {out}");
            assert!(out.contains("<NUM_01>"), "{text} -> {out}");
            assert_eq!(session.rehydrate_text(&out), text);
        }
    }

    #[test]
    fn debug_output_prints_counts_and_no_value() {
        let mut session = PseudonymizerSession::new();
        session.tokenize_text("Anna at anna@example.com", &registry());
        let printed = format!("{session:?}");
        assert!(!printed.to_lowercase().contains("anna"), "{printed}");
        assert!(printed.contains("tokens: 2"), "{printed}");
    }

    #[test]
    fn keyed_sessions_with_one_key_agree_whatever_they_saw_first() {
        let key = crate::keyed::session_key(b"machine-secret", "session-1");
        let mut first = PseudonymizerSession::keyed(key);
        let mut second = PseudonymizerSession::keyed(key);
        let registry = registry();
        let a = first.tokenize_text("Anna wrote to anna@example.com", &registry);
        // The second session sees the values in the opposite order.
        second.tokenize_text("mail anna@example.com", &registry);
        let b = second.tokenize_text("Anna wrote to anna@example.com", &registry);
        assert_eq!(a, b);
        assert!(!a.contains("Anna") && !a.contains("anna@"), "{a}");
        assert_eq!(first.rehydrate_text(&a), "Anna wrote to anna@example.com");

        let other = crate::keyed::session_key(b"machine-secret", "session-2");
        let c = PseudonymizerSession::keyed(other).tokenize_text("Anna", &registry);
        assert_ne!(c, first.tokenize_text("Anna", &registry));
    }

    #[test]
    fn a_keyed_literal_still_escapes_token_shaped_source_text() {
        let key = crate::keyed::session_key(b"k", "s");
        let mut session = PseudonymizerSession::keyed(key);
        let text = "Anna and <TRAVELER_abcdef>";
        let out = session.tokenize_text(text, &registry());
        assert!(!out.contains("<TRAVELER_abcdef>"), "{out}");
        assert_eq!(session.rehydrate_text(&out), text);
    }

    #[test]
    fn a_whole_field_becomes_one_reversible_token() {
        let mut session = PseudonymizerSession::new();
        let out = session.tokenize_whole("Alice <alice@example.com>", EntityType::Identity);
        assert_eq!(out, "<SENDER_01>");
        assert_eq!(session.rehydrate_text(&out), "Alice <alice@example.com>");
    }
}
