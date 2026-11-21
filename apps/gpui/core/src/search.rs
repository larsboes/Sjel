//! One input for a search and a prompt: which of the two a text is, and how candidates rank.
//!
//! The finder (`apps/gpui/src/finder.rs`) offers three modes. *Ask* and *Search* do what they say;
//! *Auto* asks [`intent`] and shows the answer beside the input, so a wrong guess costs one key.
//! The rule is deliberately plain words and punctuation rather than a model: it runs on every
//! keystroke, it has to agree with what the operator can see, and the phone runs it offline.
//!
//! Nothing here knows about GPUI, so the iPhone's finder can rank the same candidates the same way.

/// What the input is set to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Auto,
    Ask,
    Search,
    /// Start a run instead of answering. Never what *Auto* decides: a run edits files and asks
    /// for permission, and that must not be what Enter does by accident.
    Agent,
}

impl Mode {
    pub const ALL: [Mode; 4] = [Mode::Auto, Mode::Ask, Mode::Search, Mode::Agent];

    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::Ask => "Ask",
            Self::Search => "Search",
            Self::Agent => "Agent",
        }
    }

    /// The next mode, for the key that cycles them.
    pub fn next(self) -> Self {
        match self {
            Self::Auto => Self::Ask,
            Self::Ask => Self::Search,
            Self::Search => Self::Agent,
            Self::Agent => Self::Auto,
        }
    }

    /// What a text resolves to in this mode: never `Auto`, and never `Agent` from `Auto`.
    pub fn resolve(self, text: &str) -> Mode {
        match self {
            Self::Auto => intent(text),
            mode => mode,
        }
    }
}

/// Words that open a question or a request, in English and German. A text that starts with one
/// is a prompt; "people berlin" is a search, "who lives in berlin" is a question.
const PROMPT_OPENERS: &[&str] = &[
    // English
    "what",
    "when",
    "where",
    "who",
    "whom",
    "whose",
    "why",
    "how",
    "which",
    "is",
    "are",
    "was",
    "were",
    "do",
    "does",
    "did",
    "can",
    "could",
    "should",
    "would",
    "will",
    "shall",
    "may",
    "might",
    "have",
    "has",
    "explain",
    "summarize",
    "summarise",
    "tell",
    "write",
    "draft",
    "compare",
    "plan",
    "suggest",
    "help",
    "give",
    "make",
    "create",
    "remind",
    "translate",
    "please",
    "list",
    // German
    "was",
    "wann",
    "wo",
    "wer",
    "wem",
    "wen",
    "warum",
    "wieso",
    "weshalb",
    "wie",
    "welche",
    "welcher",
    "welches",
    "ist",
    "sind",
    "kann",
    "kannst",
    "soll",
    "sollte",
    "hast",
    "habe",
    "erkläre",
    "erklär",
    "fasse",
    "schreib",
    "schreibe",
    "zeig",
    "zeige",
    "plane",
    "bitte",
];

/// A prompt or a search, decided from the words alone.
///
/// A question mark, an opening question or request word, or five words and more make a prompt.
/// Everything else -- a name, a title, two keywords -- is a search, because that is what people
/// type when they want to find a thing rather than be told about it.
pub fn intent(text: &str) -> Mode {
    let text = text.trim();
    if text.is_empty() {
        return Mode::Search;
    }
    if text.ends_with('?') {
        return Mode::Ask;
    }
    let mut words = text.split_whitespace();
    let first = words
        .next()
        .map(|word| {
            word.trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase()
        })
        .unwrap_or_default();
    if PROMPT_OPENERS.contains(&first.as_str()) && text.split_whitespace().count() >= 2 {
        return Mode::Ask;
    }
    if text.split_whitespace().count() >= 5 {
        return Mode::Ask;
    }
    Mode::Search
}

/// Folds case, diacritics, and German/Nordic ligatures into a canonical ASCII-friendly search
/// string so non-ASCII titles and document bodies match both accented and plain-ASCII queries
/// (e.g. `"Köln"` matches `"köln"`, `"koln"`, and `"koeln"`; `"Straße"` matches `"strasse"`).
pub fn fold_search_text(input: &str) -> String {
    let mut folded = String::with_capacity(input.len());
    for ch in input.chars() {
        for lower in ch.to_lowercase() {
            match lower {
                'ä' | 'á' | 'à' | 'â' | 'ã' | 'å' | 'ā' | 'ă' | 'ą' => folded.push('a'),
                'ö' | 'ó' | 'ò' | 'ô' | 'õ' | 'ø' | 'ō' | 'ŏ' | 'ő' => folded.push('o'),
                'ü' | 'ú' | 'ù' | 'û' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => folded.push('u'),
                'ß' => folded.push_str("ss"),
                'æ' => folded.push('a'),
                'œ' => folded.push('o'),
                'é' | 'è' | 'ê' | 'ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => folded.push('e'),
                'í' | 'ì' | 'î' | 'ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' => folded.push('i'),
                'ñ' | 'ń' | 'ň' => folded.push('n'),
                'ç' | 'ć' | 'č' => folded.push('c'),
                'ś' | 'š' | 'ş' => folded.push('s'),
                'ź' | 'ż' | 'ž' => folded.push('z'),
                'ł' => folded.push('l'),
                'ý' | 'ÿ' => folded.push('y'),
                other => folded.push(other),
            }
        }
    }
    // Collapse common German/Nordic ASCII digraph transliterations (ae/oe/ue -> a/o/u) when
    // preceded/followed by letters so "koeln" and "koln" fold identically to "Köln".
    folded
        .replace("ae", "a")
        .replace("oe", "o")
        .replace("ue", "u")
}

/// How well `query` matches one candidate's fields, or `None` when it does not match at all.
///
/// Every query word has to appear in some field, so adding a word narrows. A word that starts the
/// title scores highest, then one that starts any word of the title, then one anywhere in it, then
/// one only in the detail fields. Shorter titles win ties, so "Inbox" beats "Inbox rules".
pub fn score(query: &str, title: &str, details: &[&str]) -> Option<u32> {
    let title_folded = fold_search_text(title);
    let details_folded: Vec<String> = details
        .iter()
        .map(|detail| fold_search_text(detail))
        .collect();
    let mut total = 0u32;
    let mut any = false;
    for word in query.split_whitespace() {
        let word = fold_search_text(word);
        if word.is_empty() {
            continue;
        }
        any = true;
        let points = if title_folded.starts_with(&word) {
            100
        } else if title_folded
            .split(|c: char| !c.is_alphanumeric())
            .any(|part| part.starts_with(&word))
        {
            70
        } else if title_folded.contains(&word) {
            40
        } else if details_folded.iter().any(|detail| detail.contains(&word)) {
            15
        } else {
            return None;
        };
        total += points;
    }
    if !any {
        return None;
    }
    // A tiebreak only: up to 40 points for a short title, under the 60 that one more matched word
    // in a detail field is worth.
    Some(total * 4 + 40u32.saturating_sub(title.chars().count() as u32))
}

/// The indices of the matching candidates, best first. `fields` gives a candidate's title and
/// its detail fields; candidates that do not match are dropped, and equal scores keep the order
/// they came in, so a caller can put its most likely kinds first.
pub fn rank<T>(
    query: &str,
    candidates: &[T],
    fields: impl Fn(&T) -> (&str, Vec<&str>),
) -> Vec<usize> {
    let mut scored: Vec<(usize, u32)> = candidates
        .iter()
        .enumerate()
        .filter_map(|(index, candidate)| {
            let (title, details) = fields(candidate);
            score(query, title, &details).map(|points| (index, points))
        })
        .collect();
    scored.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    scored.into_iter().map(|(index, _)| index).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn questions_and_requests_are_prompts() {
        for text in [
            "what is on my calendar today",
            "who is Anna?",
            "summarize the inbox",
            "Wann fährt der nächste Zug nach Köln",
            "trips?",
            "find everything about the Miele request from last week",
        ] {
            assert_eq!(intent(text), Mode::Ask, "{text}");
        }
    }

    #[test]
    fn names_and_keywords_are_searches() {
        for text in [
            "",
            "Anna",
            "miele",
            "packs",
            "tax 2025",
            "Soma base renders",
        ] {
            assert_eq!(intent(text), Mode::Search, "{text}");
        }
        // One opener word alone is still a search: "plan" is a page as much as a request.
        assert_eq!(intent("plan"), Mode::Search);
    }

    #[test]
    fn a_fixed_mode_wins_over_the_words() {
        assert_eq!(Mode::Search.resolve("what is this?"), Mode::Search);
        assert_eq!(Mode::Ask.resolve("anna"), Mode::Ask);
        assert_eq!(Mode::Auto.resolve("anna"), Mode::Search);
        assert_eq!(
            Mode::Auto.next().next().next().next(),
            Mode::Auto,
            "the four-mode cycle returns to Auto"
        );
    }

    #[test]
    fn every_word_has_to_match_and_the_title_counts_most() {
        assert!(score("inb", "Inbox", &[]).is_some());
        assert!(score("inbox zzz", "Inbox", &[]).is_none());
        // Detail-only matches still count, below a title match.
        let detail = score("berlin", "Anna", &["lives in Berlin"]).unwrap();
        let title = score("berlin", "Berlin trip", &[]).unwrap();
        assert!(title > detail);
        assert!(score("", "Inbox", &[]).is_none());
    }

    #[test]
    fn ranking_puts_prefixes_first_and_keeps_order_on_ties() {
        let items = ["Self-model", "Settings", "Setup", "Reset tasks"];
        let ranked = rank("se", &items, |item| (*item, Vec::new()));
        // "Reset" contains "se" but does not start a word with it.
        assert_eq!(ranked.last(), Some(&3));
        assert_eq!(
            &ranked[..3],
            &[2, 1, 0],
            "shorter titles first among prefixes"
        );
    }

    #[test]
    fn non_ascii_case_folding_and_diacritics_match_titles_and_document_bodies() {
        // Reproduces non-ASCII case-folding and German/Nordic diacritic/ligature queries
        // across titles and document body details while keeping local-first prefix priority.
        let candidates = [
            ("Köln Hbf → München Hbf", vec!["ICE 515 · Wagen 12"]),
            (
                "Finanzamt Bescheid 2026",
                vec!["Einkommensteuerbescheid für 2025 · Voßstraße 14, Zürich"],
            ),
            (
                "Søren Kierkegaard",
                vec!["Copenhagen notes · Àda reference"],
            ),
            ("Trip to Koln Archive", vec!["Secondary match"]),
        ];

        // Plain ASCII "koln" and German digraph "koeln" both match "Köln"
        assert!(score("koln", candidates[0].0, &candidates[0].1).is_some());
        assert!(score("koeln", candidates[0].0, &candidates[0].1).is_some());
        assert!(score("munchen", candidates[0].0, &candidates[0].1).is_some());
        assert!(score("muenchen", candidates[0].0, &candidates[0].1).is_some());

        // Document body detail matches "ß" via "ss" ("vossstrasse") and "ü" via "u"/"ue" ("fur", "zurich", "zuerich")
        assert!(score("vossstrasse zurich", candidates[1].0, &candidates[1].1).is_some());
        assert!(score("zuerich fur 2025", candidates[1].0, &candidates[1].1).is_some());

        // Nordic "ø" and accented "Àda" match plain ASCII "soren" and "ada"
        assert!(score("soren ada", candidates[2].0, &candidates[2].1).is_some());

        // Ranking keeps title prefix matches ahead of detail-only matches
        let ranked = rank("koeln", &candidates, |c| (c.0, c.1.clone()));
        assert_eq!(ranked, vec![0, 3]);
    }
}
