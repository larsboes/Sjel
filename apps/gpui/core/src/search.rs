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

/// How well `query` matches one candidate's fields, or `None` when it does not match at all.
///
/// Every query word has to appear in some field, so adding a word narrows. A word that starts the
/// title scores highest, then one that starts any word of the title, then one anywhere in it, then
/// one only in the detail fields. Shorter titles win ties, so "Inbox" beats "Inbox rules".
pub fn score(query: &str, title: &str, details: &[&str]) -> Option<u32> {
    let title = title.to_lowercase();
    let details: Vec<String> = details.iter().map(|detail| detail.to_lowercase()).collect();
    let mut total = 0u32;
    let mut any = false;
    for word in query.split_whitespace() {
        let word = word.to_lowercase();
        any = true;
        let points = if title.starts_with(&word) {
            100
        } else if title
            .split(|c: char| !c.is_alphanumeric())
            .any(|part| part.starts_with(&word))
        {
            70
        } else if title.contains(&word) {
            40
        } else if details.iter().any(|detail| detail.contains(&word)) {
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
}
