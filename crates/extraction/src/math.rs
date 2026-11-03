//! Does this look like a page of notation the reader above could not express?
//!
//! Rung 2 of the ladder does not fail on a formula sheet. It succeeds, and
//! returns something wrong. `upstreams.toml [auge]` records the second
//! measurement of that engine, 2026-08-31: Apple Vision is excellent on printed
//! German prose and useless on mathematical notation, reading `=` as `-`
//! (`q- 10nC`, `d-2am`) and expressing no fraction at all. A page of formulas
//! "comes back as plausible-looking wrong text rather than as a failure --
//! which is worse than an error". PRD Q63 -> B30 gives the ladder a third rung
//! for exactly that, and this module is the only thing that reaches it.
//!
//! So this is a detector over rung 2's TEXT. No image, no model, no engine: it
//! reads what came back and asks whether the shape of the answer is the shape
//! of that recorded failure. It is hermetic and it is cheap, which is what lets
//! it run on every page rung 2 produces.
//!
//! ## Asymmetric on purpose
//!
//! A false positive costs one rung-3 call. A false negative stores plausible
//! wrong text under a `producer` that claims it was read correctly, and nothing
//! downstream can tell. The signals below are therefore tuned to fire, and the
//! fire rule combines a *notation* signal with a *not-prose* signal rather than
//! trusting either alone.
//!
//! ## Unverified
//!
//! **These thresholds are not measured.** They are derived from one recorded
//! failure signature and from the six pages of `eval/ocr-corpus.json`, which is
//! six pages and not a labelled corpus. The false-positive rate on German prose
//! carrying hyphenated compounds (`Nord-Süd`) or numeric ranges (`2-3 Tage`) is
//! unknown; both shapes are excluded by construction below and neither is
//! measured at scale. `cargo run --bin extraction-gate` is what turns this
//! paragraph into a number, and until it does this sentence stays.

/// How many signals exist, so [`MathSuspicion::score`] is a share and not a
/// count that quietly changes meaning when a signal is added.
const SIGNALS: usize = 5;

/// Which signal fired, so a test asserts WHY and a later threshold change is
/// reviewable rather than a diff of one float.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// A relation was read as a hyphen: `q- 10nC` for `q = 10 nC`. The recorded
    /// signature, and the one that turns an equation into a subtraction that
    /// still parses.
    RelationReadAsHyphen,
    /// Mathematical symbols or unit-bearing quantities are present and not one
    /// relation sign is. A page that states quantities and asserts nothing
    /// about them lost its relations somewhere.
    NoRelationInNotation,
    /// A line ends on a relation sign with nothing after it: `E=`, `SE =`. The
    /// equation was recognized as far as its verb and its body did not survive.
    ///
    /// Added 2026-09-02 from `eval/results/2026-09-02-apple-vision-baseline.md`,
    /// which is the first run of this corpus and found a failure the recorded
    /// 2026-08-31 signature does not cover: on PRINTED notation Apple Vision
    /// does not corrupt a displayed formula, it DELETES it, and returns the
    /// surrounding prose as though the page had none. Nothing above catches
    /// that, because everything above looks for a corruption and there is none
    /// to find.
    RelationWithNoRightHandSide,
    /// Mathematical content came back with no division, grouping or exponent
    /// mark anywhere in it — no `/`, `÷`, `(`, `)` or `^`. That is what "cannot
    /// express a fraction at all" looks like from the text side.
    NoFractionStructure,
    /// Too few function words and too many short lines for prose. Vision's good
    /// case is prose, so this separates a formula sheet from a German article.
    LowProseShare,
}

/// What the detector found, and how much of it.
#[derive(Debug, Clone, PartialEq)]
pub struct MathSuspicion {
    /// Share of the [`SIGNALS`] signals present, `0.0..=1.0`. Reported, never
    /// compared: [`MathSuspicion::fires`] is the rule, and a threshold on this
    /// number would quietly replace it.
    pub score: f32,
    pub reasons: Vec<Reason>,
}

impl MathSuspicion {
    /// Whether this output should be re-read by rung 3.
    ///
    /// A notation signal AND a not-prose signal. Either alone is a known false
    /// positive: a prose page listing measurements trips
    /// [`Reason::NoRelationInNotation`], and a table of departure times trips
    /// [`Reason::LowProseShare`]. Neither is a page of notation.
    pub fn fires(&self) -> bool {
        let notation = self.has(Reason::RelationReadAsHyphen)
            || self.has(Reason::NoRelationInNotation)
            || self.has(Reason::RelationWithNoRightHandSide);
        let not_prose = self.has(Reason::NoFractionStructure) || self.has(Reason::LowProseShare);
        notation && not_prose
    }

    pub fn has(&self, reason: Reason) -> bool {
        self.reasons.contains(&reason)
    }
}

/// Relation signs an equation keeps when it survives OCR.
const RELATIONS: [char; 7] = ['=', '≈', '≤', '≥', '≠', '<', '>'];

/// Symbols that only appear on a page of notation. Deliberately symbols and not
/// words: a German article about `Gleichungen` must not be treated as one, and
/// a word list is how that mistake gets made.
const MATH_SYMBOLS: [char; 14] = [
    '∫', '∑', '√', '±', '∞', '∂', '∆', 'π', 'Σ', 'Π', 'µ', 'θ', 'α', 'ε',
];

/// Marks a fraction, a group or an exponent. Their total absence from a page
/// that carries notation is [`Reason::NoFractionStructure`].
const STRUCTURE_MARKS: [char; 6] = ['/', '÷', '(', ')', '^', '|'];

/// Units that make a numeral a quantity rather than a page number. Short and
/// SI-shaped on purpose; a longer list buys nothing and misfires on prose.
const UNITS: [&str; 16] = [
    "nc", "pc", "µc", "cm", "mm", "km", "kg", "mg", "hz", "khz", "mhz", "ms", "kv", "mv", "ohm",
    "rad",
];

/// German and English function words. Their share is the prose measure; a
/// formula sheet has almost none and a paragraph is full of them.
///
/// One list, not two, and `in` and `an` appear once although both languages own
/// them: this is a membership test, and a duplicate entry would weight a word
/// that happens to be shared.
const STOPWORDS: [&str; 82] = [
    // de
    "der", "die", "das", "den", "dem", "des", "ein", "eine", "einer", "einem", "eines", "und",
    "oder", "aber", "nicht", "ist", "sind", "war", "waren", "wird", "werden", "mit", "von", "zu",
    "im", "in", "an", "auf", "für", "bei", "aus", "nach", "über", "um", "als", "auch", "noch",
    "nur", "wenn", "dass", "sich", "man", "es", "sie", "wir", "ihr", "beim", "vom",
    // en
    "the", "a", "and", "or", "but", "not", "is", "are", "was", "were", "will", "would", "be",
    "been", "of", "to", "on", "for", "with", "at", "by", "from", "as", "that", "this", "it",
    "they", "we", "you", "have", "has", "had", "its", "their",
];

/// Below this share of function words, a body is not prose.
const PROSE_STOPWORD_FLOOR: f32 = 0.08;
/// Above this share of short lines, a body is a list of fragments.
const SHORT_LINE_CEILING: f32 = 0.5;
/// A line of this many whitespace tokens or fewer counts as short.
const SHORT_LINE_TOKENS: usize = 4;

/// Read rung 2's output and say whether rung 3 should re-read the page.
///
/// Pure over the text. Nothing here opens a file, spawns a process or looks at
/// an image, which is what keeps every consumer's `cargo test` hermetic.
pub fn inspect(text: &str) -> MathSuspicion {
    let mut reasons = Vec::new();
    let trimmed = text.trim();

    // An empty body is not a math page. It is an empty body, and the rung that
    // produced it already reports that as its own failure rather than as text
    // (see `ExtractionError`'s doc comment).
    if trimmed.is_empty() {
        return MathSuspicion {
            score: 0.0,
            reasons,
        };
    }

    let has_notation =
        trimmed.chars().any(|c| MATH_SYMBOLS.contains(&c)) || carries_quantity(trimmed);
    let has_relation = trimmed.chars().any(|c| RELATIONS.contains(&c));

    if trimmed.lines().any(relation_read_as_hyphen) {
        reasons.push(Reason::RelationReadAsHyphen);
    }
    if has_notation && !has_relation {
        reasons.push(Reason::NoRelationInNotation);
    }
    if trimmed.lines().any(relation_with_no_right_hand_side) {
        reasons.push(Reason::RelationWithNoRightHandSide);
    }
    if has_notation && !trimmed.chars().any(|c| STRUCTURE_MARKS.contains(&c)) {
        reasons.push(Reason::NoFractionStructure);
    }
    if stopword_share(trimmed) < PROSE_STOPWORD_FLOOR
        && short_line_share(trimmed) > SHORT_LINE_CEILING
    {
        reasons.push(Reason::LowProseShare);
    }

    MathSuspicion {
        score: reasons.len() as f32 / SIGNALS as f32,
        reasons,
    }
}

/// A line whose last non-space character is a relation sign.
///
/// The equation kept its verb and lost its object. Deliberately not applied to a
/// line ending in a comma or a colon, which is ordinary prose introducing
/// something; a sentence that ends on `=` is not.
fn relation_with_no_right_hand_side(line: &str) -> bool {
    line.trim_end()
        .chars()
        .next_back()
        .is_some_and(|c| RELATIONS.contains(&c))
}

/// One line of the recorded signature: a short identifier, a hyphen, a number,
/// on a line with no function word.
///
/// The three exclusions are each a false positive that would otherwise be
/// certain in German prose:
///
/// * the left side must be an IDENTIFIER, not a numeral, so a range (`2-3
///   Tage`) never matches;
/// * the right side must be a DIGIT, so a compound (`Nord-Süd`) never matches;
/// * the identifier must be at most three characters, because a variable is
///   short and a word is not.
fn relation_read_as_hyphen(line: &str) -> bool {
    let line = line.trim();
    if line.is_empty() || line.split_whitespace().count() > 8 {
        return false;
    }
    if line.split(|c: char| !c.is_alphanumeric()).any(is_stopword) {
        return false;
    }

    let chars: Vec<char> = line.chars().collect();
    for (index, character) in chars.iter().enumerate() {
        if *character != '-' {
            continue;
        }
        // Right: the next non-space character is a digit.
        let right_is_number = chars[index + 1..]
            .iter()
            .find(|c| !c.is_whitespace())
            .is_some_and(|c| c.is_ascii_digit());
        if !right_is_number {
            continue;
        }
        // Left: the preceding run of alphanumerics is a short identifier that
        // starts with a letter.
        let left: String = chars[..index]
            .iter()
            .rev()
            .take_while(|c| c.is_alphanumeric())
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        if !left.is_empty()
            && left.len() <= 3
            && left.chars().next().is_some_and(|c| c.is_alphabetic())
        {
            return true;
        }
    }
    false
}

/// A numeral immediately followed by a unit — `10 nC`, `2cm`. This is what makes
/// a page of measurements notation rather than prose, without a word list.
fn carries_quantity(text: &str) -> bool {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    for (index, token) in tokens.iter().enumerate() {
        let cleaned = token.trim_matches(|c: char| !c.is_alphanumeric());
        if cleaned.is_empty() {
            continue;
        }
        // "10nC": digits then letters in one token.
        let digits: String = cleaned.chars().take_while(char::is_ascii_digit).collect();
        if !digits.is_empty() {
            let suffix = cleaned[digits.len()..].to_lowercase();
            if !suffix.is_empty() && UNITS.contains(&suffix.as_str()) {
                return true;
            }
            // "10 nC": the number and the unit are separate tokens.
            if digits.len() == cleaned.len() {
                if let Some(next) = tokens.get(index + 1) {
                    let unit = next
                        .trim_matches(|c: char| !c.is_alphanumeric())
                        .to_lowercase();
                    if UNITS.contains(&unit.as_str()) {
                        return true;
                    }
                }
            }
        }
    }
    false
}

fn is_stopword(token: &str) -> bool {
    !token.is_empty() && STOPWORDS.contains(&token.to_lowercase().as_str())
}

fn stopword_share(text: &str) -> f32 {
    let tokens: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_lowercase())
        .collect();
    if tokens.is_empty() {
        return 0.0;
    }
    let hits = tokens
        .iter()
        .filter(|t| STOPWORDS.contains(&t.as_str()))
        .count();
    hits as f32 / tokens.len() as f32
}

fn short_line_share(text: &str) -> f32 {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.is_empty() {
        return 0.0;
    }
    let short = lines
        .iter()
        .filter(|l| l.split_whitespace().count() <= SHORT_LINE_TOKENS)
        .count();
    short as f32 / lines.len() as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_relation_read_as_a_hyphen_is_the_signature_that_fires_rung_three() {
        // The literal strings upstreams.toml [auge] recorded on 2026-08-31.
        let recorded = "Elektrisches Feld\nq- 10nC\nd-2am\nE F q\nW F ds";
        let suspicion = inspect(recorded);
        assert!(suspicion.has(Reason::RelationReadAsHyphen), "{suspicion:?}");
        assert!(suspicion.fires(), "{suspicion:?}");
    }

    #[test]
    fn a_page_of_german_prose_never_trips_the_detector() {
        // Vision's excellent case, and the one regression that matters: a false
        // negative costs a rung-3 call, a false positive here would send every
        // article to an engine that does not exist.
        let prose = "Fahrplanänderung im Fernverkehr\n\
             Ab dem kommenden Fahrplanwechsel fahren die Züge zwischen Köln Hbf und\n\
             München Hbf über eine geänderte Streckenführung. Die Fahrzeit verkürzt\n\
             sich um durchschnittlich achtzehn Minuten, weil der Umweg über das\n\
             Ausweichgleis entfällt.";
        assert!(!inspect(prose).fires(), "{:?}", inspect(prose));
    }

    #[test]
    fn german_compounds_and_numeric_ranges_are_not_the_signature() {
        // The two false positives the hyphen signal is constructed to exclude.
        // Both are ordinary German and both would otherwise match.
        assert!(!relation_read_as_hyphen("Nord-Süd-Verbindung"));
        assert!(!relation_read_as_hyphen("2-3 Tage"));
        assert!(!relation_read_as_hyphen(
            "Fahrzeit 2-3 Stunden im Regelbetrieb"
        ));
    }

    #[test]
    fn an_equation_that_kept_its_equals_sign_does_not_fire() {
        // The point of the ladder: an engine that read the notation correctly
        // must not be second-guessed, or rung 3 runs on every page.
        let read_correctly = "Elektrisches Feld einer Punktladung\n\
             q = 10 nC\n\
             d = 2 cm\n\
             E = F / q\n\
             E = 1 / (4 π ε₀) · q / d²";
        assert!(
            !inspect(read_correctly).fires(),
            "{:?}",
            inspect(read_correctly)
        );
    }

    #[test]
    fn an_empty_body_is_not_a_math_page() {
        assert!(inspect("").reasons.is_empty());
        assert!(!inspect("   \n\n  ").fires());
    }

    #[test]
    fn a_table_of_departure_times_is_short_lined_without_being_notation() {
        // Column-major table output is exactly the shape LowProseShare reports,
        // and on its own it must not reach rung 3.
        let table = "Datum\nAb\nBahnhof\n29.08.\n08:14\nKöln Hbf\n11:05\nFrankfurt(M) Hbf\nICE 622";
        let suspicion = inspect(table);
        assert!(suspicion.has(Reason::LowProseShare), "{suspicion:?}");
        assert!(!suspicion.fires(), "{suspicion:?}");
    }

    #[test]
    fn quantities_without_a_relation_are_notation_that_lost_its_relations() {
        let stripped = "Gegeben\nq 10 nC\nd 2 cm\nE F q";
        let suspicion = inspect(stripped);
        assert!(suspicion.has(Reason::NoRelationInNotation), "{suspicion:?}");
        assert!(suspicion.fires(), "{suspicion:?}");
    }

    #[test]
    fn a_formula_that_was_deleted_rather_than_corrupted_still_fires() {
        // The failure the first corpus run found, which the 2026-08-31
        // signature does not cover: nothing is misread, the displayed equations
        // are simply not there, and what is left is a relation with no body.
        // See eval/results/2026-09-02-apple-vision-baseline.md.
        let deleted = "Elektrisches Feld einer Punktladung\n\
             Gegeben:\n9 = 10 nC\nd = 2 cm\n\
             Die Feldstärke im Abstand d folgt aus der Definition\n\
             und mit dem Coulombgesetz\n1\nE=\nATTEO\nd2\n\
             Die Arbeit längs eines Weges ist";
        let suspicion = inspect(deleted);
        assert!(
            suspicion.has(Reason::RelationWithNoRightHandSide),
            "{suspicion:?}"
        );
        assert!(suspicion.fires(), "{suspicion:?}");
    }

    #[test]
    fn prose_that_merely_mentions_a_measurement_is_not_a_stranded_equation() {
        // The signal above must key on the LINE ENDING, not on the presence of
        // a relation. A sentence that contains one is ordinary text.
        assert!(!relation_with_no_right_hand_side(
            "Die Feldstärke ist E = 5 kV/m und damit unkritisch."
        ));
        assert!(relation_with_no_right_hand_side("SE ="));
    }

    #[test]
    fn the_score_reports_how_many_signals_fired_and_decides_nothing() {
        let suspicion = inspect("q- 10nC\nd-2am");
        assert!(suspicion.score > 0.0);
        assert_eq!(
            suspicion.score,
            suspicion.reasons.len() as f32 / SIGNALS as f32,
            "score is a report of the reasons, never a second rule"
        );
    }
}
