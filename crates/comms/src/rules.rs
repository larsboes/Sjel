//! Pure, deterministic, config-driven classifier. No network, no LLM in this
//! build. Given the headers of a mail thread, it assigns exactly one stream and
//! a one-sentence English rationale.
//!
//! Order: config `rules` (first match wins) → built-in heuristics → the
//! conservative default (`aktiv`). The built-in heuristics are intentionally
//! generic (no personal senders); anything personal belongs in the overlay's
//! `rules` list.

use serde::Deserialize;

/// A config rule. `r#match` is `match` in JSON (a Rust keyword, hence raw).
/// All match conditions present must hold (AND); absent conditions are ignored.
/// A rule with an entirely empty match spec never fires (guards against an
/// accidental catch-all).
#[derive(Debug, Clone, Deserialize)]
pub struct Rule {
    pub r#match: MatchSpec,
    pub stream: String,
    pub note: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct MatchSpec {
    /// Any of these substrings present (case-insensitive) in the From header.
    pub from_contains: Option<Vec<String>>,
    /// Any of these substrings present (case-insensitive) in the Subject.
    pub subject_contains: Option<Vec<String>>,
    /// Require the List-Unsubscribe header present (true) or absent (false).
    pub has_list_unsubscribe: Option<bool>,
}

/// The header facts a single thread's classification depends on. Borrowed so
/// the classifier stays allocation-free and trivially testable.
#[derive(Debug, Clone, Copy)]
pub struct MailFacts<'a> {
    pub from: &'a str,
    pub subject: &'a str,
    pub has_list_unsubscribe: bool,
}

/// The seven ratified triage streams. Kept in sync with the CHECK constraint
/// in store.rs manually (single-user tool, not worth generating).
pub const STREAMS: [&str; 7] = [
    "aktiv",
    "issue",
    "feed",
    "werbung",
    "belege",
    "steuern",
    "sonstiges",
];

/// The classifier version every deterministic mail row carries.
///
/// A bare literal in five Rust places before this existed, next to a
/// `content_item::MAIL_CLASSIFIER_VERSION` that had a constant for the class
/// axis. The two DDL DEFAULTs in `store/migrations.rs` stay literal because
/// they are inside a SQL string.
pub const MAIL_RULES_VERSION: &str = "mail-rules-v1";

/// One English line per stream, in [`STREAMS`] order.
///
/// This is the vocabulary block a prompt is built from (`crate::mail_model`),
/// and it lives beside `STREAMS` so a stream added to one and not the other
/// fails a test rather than reaching a model that has never heard of it.
pub const STREAM_DEFINITIONS: [(&str, &str); 7] = [
    (
        "aktiv",
        "A real message from a person or a service that still concerns the reader, with nothing \
         specific asked of them yet.",
    ),
    (
        "issue",
        "The message asks the reader to do something: answer a question, confirm, pay, appear, \
         or decide by a date.",
    ),
    (
        "feed",
        "A newsletter, digest or release announcement the reader subscribed to. Reading material, \
         never an obligation.",
    ),
    (
        "werbung",
        "Advertising, a promotion, a discount or a sales approach the reader did not ask for.",
    ),
    (
        "belege",
        "A receipt, an invoice, an order confirmation or a payment record worth keeping.",
    ),
    (
        "steuern",
        "Tax material: an assessment, a tax office letter, or a document filed with a tax return.",
    ),
    (
        "sonstiges",
        "None of the six above. A notification with nothing to do and nothing to keep.",
    ),
];

/// Which rung of the deterministic ladder decided a thread.
///
/// `DecidedBy` rather than `Rung`: `crate::quiet::Rung` already owns that word
/// in this crate for the inference ladder, and `crate::mail_model` holds both
/// in scope.
///
/// The values are the `CHECK` vocabulary of `{prefix}_triage_rules`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecidedBy {
    /// A rule from the overlay's `rules` list fired.
    ConfigRule,
    /// One of the four built-in heuristics fired.
    Heuristic,
    /// Nothing fired and the conservative `aktiv` default was kept. These are
    /// the rows the model rung is allowed to look at.
    Fallback,
}

impl DecidedBy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ConfigRule => "config_rule",
            Self::Heuristic => "heuristic",
            Self::Fallback => "fallback",
        }
    }
}

impl std::str::FromStr for DecidedBy {
    type Err = String;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        match value {
            "config_rule" => Ok(Self::ConfigRule),
            "heuristic" => Ok(Self::Heuristic),
            "fallback" => Ok(Self::Fallback),
            other => Err(format!("unknown decided_by '{other}'")),
        }
    }
}

/// What the deterministic classifier decided, and which rung decided it.
///
/// The rung is not derivable afterwards: `classify` reads
/// `has_list_unsubscribe`, a header no row stores, and string-matching the
/// fallback rationale breaks the moment an overlay rule declares stream
/// `aktiv`. So it is returned here and persisted beside the row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    pub stream: String,
    pub rationale: String,
    pub decided_by: DecidedBy,
}

const SHOPPING_KEYWORDS: [&str; 6] = ["sale", "rabatt", "%", "deal", "shop", "angebot"];
const TECH_KEYWORDS: [&str; 6] = [
    "release",
    "changelog",
    "engineering",
    "ai",
    "newsletter",
    "digest",
];
const RECEIPT_KEYWORDS: [&str; 6] = [
    "rechnung", "receipt", "invoice", "zahlung", "payment", "beleg",
];

fn contains_ci(haystack: &str, needle: &str) -> bool {
    haystack.to_lowercase().contains(&needle.to_lowercase())
}

fn any_contains(haystack: &str, needles: &[String]) -> bool {
    needles.iter().any(|n| contains_ci(haystack, n))
}

fn any_contains_static(haystack: &str, needles: &[&str]) -> bool {
    let low = haystack.to_lowercase();
    needles.iter().any(|n| low.contains(n))
}

/// Evaluate a single config rule against the facts. Returns the matched
/// condition description if it fires, else None.
fn eval_rule(rule: &Rule, f: &MailFacts) -> Option<String> {
    let m = &rule.r#match;
    let mut conditions = 0;
    let mut matched: Vec<String> = Vec::new();

    if let Some(subs) = &m.from_contains {
        conditions += 1;
        if any_contains(f.from, subs) {
            matched.push("sender".into());
        } else {
            return None;
        }
    }
    if let Some(subs) = &m.subject_contains {
        conditions += 1;
        if any_contains(f.subject, subs) {
            matched.push("subject".into());
        } else {
            return None;
        }
    }
    if let Some(want) = m.has_list_unsubscribe {
        conditions += 1;
        if f.has_list_unsubscribe == want {
            matched.push(if want {
                "List-Unsubscribe present".into()
            } else {
                "no List-Unsubscribe".into()
            });
        } else {
            return None;
        }
    }

    if conditions == 0 {
        // Empty match spec: never a catch-all.
        return None;
    }
    Some(matched.join(" + "))
}

/// Classify a thread. Config rules first (first match wins), then the built-in
/// heuristics, then the conservative `aktiv` default.
///
/// Returns a [`Verdict`] rather than a `(stream, rationale)` pair because the
/// rung that fired is a fact only this function holds, and the model rung runs
/// on exactly the rows where nothing fired.
pub fn classify(f: &MailFacts, rules: &[Rule]) -> Verdict {
    // 1. Config rules — first match wins.
    for rule in rules {
        if let Some(cond) = eval_rule(rule, f) {
            return Verdict {
                stream: rule.stream.clone(),
                rationale: format!("{} ({})", rule.note, cond),
                decided_by: DecidedBy::ConfigRule,
            };
        }
    }

    // 2. Built-in heuristics (generic, no personal facts). The four rationale
    // literals below are also embedded in the `{prefix}_triage_rules` backfill
    // in `store/migrations.rs`; `the_backfill_literals_match_the_classifier`
    // asserts the two lists still agree.
    if f.has_list_unsubscribe && any_contains_static(f.subject, &SHOPPING_KEYWORDS) {
        return Verdict {
            stream: "werbung".into(),
            rationale:
                "List-Unsubscribe plus a shopping signal in the subject; classified as advertising."
                    .into(),
            decided_by: DecidedBy::Heuristic,
        };
    }
    if f.has_list_unsubscribe && any_contains_static(f.subject, &TECH_KEYWORDS) {
        return Verdict {
            stream: "feed".into(),
            rationale: "List-Unsubscribe plus a development or technology signal in the subject; classified as a Feed newsletter.".into(),
            decided_by: DecidedBy::Heuristic,
        };
    }
    let noreply = contains_ci(f.from, "noreply") || contains_ci(f.from, "no-reply");
    if noreply && any_contains_static(f.subject, &RECEIPT_KEYWORDS) {
        return Verdict {
            stream: "belege".into(),
            rationale: "A no-reply sender plus a receipt or invoice signal in the subject; classified as a receipt.".into(),
            decided_by: DecidedBy::Heuristic,
        };
    }
    if f.has_list_unsubscribe {
        return Verdict {
            stream: "sonstiges".into(),
            rationale:
                "List-Unsubscribe is present, but no specific rule matched; classified as other."
                    .into(),
            decided_by: DecidedBy::Heuristic,
        };
    }

    // 3. Conservative default.
    Verdict {
        stream: "aktiv".into(),
        rationale: "No rule matched; kept active as the conservative default.".into(),
        decided_by: DecidedBy::Fallback,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts<'a>(from: &'a str, subject: &'a str, lu: bool) -> MailFacts<'a> {
        MailFacts {
            from,
            subject,
            has_list_unsubscribe: lu,
        }
    }

    #[test]
    fn builtin_shopping_promo_is_werbung() {
        let verdict = classify(
            &facts("news@shop.example", "Winter SALE -50% Rabatt", true),
            &[],
        );
        assert_eq!(verdict.stream, "werbung");
        assert!(!verdict.rationale.is_empty());
    }

    #[test]
    fn builtin_tech_newsletter_is_feed() {
        let verdict = classify(
            &facts("hello@bytes.dev", "This week in AI: new release", true),
            &[],
        );
        assert_eq!(verdict.stream, "feed");
    }

    #[test]
    fn builtin_noreply_invoice_is_belege() {
        let verdict = classify(
            &facts("noreply@vendor.example", "Ihre Rechnung 2026-07", false),
            &[],
        );
        assert_eq!(verdict.stream, "belege");
    }

    #[test]
    fn builtin_bare_list_unsubscribe_is_sonstiges() {
        let verdict = classify(
            &facts("info@social.example", "Weekly community update", true),
            &[],
        );
        assert_eq!(verdict.stream, "sonstiges");
    }

    #[test]
    fn no_signal_is_conservative_aktiv() {
        let verdict = classify(
            &facts("a.person@gmail.com", "Re: lunch tomorrow?", false),
            &[],
        );
        assert_eq!(verdict.stream, "aktiv");
        assert!(verdict.rationale.contains("conservative default"));
    }

    #[test]
    fn config_rule_wins_over_builtin() {
        // Without the rule this would be a tech `feed`; the config rule reroutes it.
        let rules = vec![Rule {
            r#match: MatchSpec {
                from_contains: Some(vec!["bytes.dev".into()]),
                subject_contains: None,
                has_list_unsubscribe: None,
            },
            stream: "feed".into(),
            note: "curated development newsletter".into(),
        }];
        let verdict = classify(&facts("hello@bytes.dev", "random subject", true), &rules);
        assert_eq!(verdict.stream, "feed");
        assert!(verdict.rationale.contains("curated development newsletter"));
        assert!(verdict.rationale.contains("sender"));
    }

    #[test]
    fn config_rule_can_produce_steuern() {
        let rules = vec![Rule {
            r#match: MatchSpec {
                from_contains: None,
                subject_contains: Some(vec!["Steuerbescheid".into()]),
                has_list_unsubscribe: None,
            },
            stream: "steuern".into(),
            note: "steuerrelevant".into(),
        }];
        let verdict = classify(
            &facts("amt@example.gov", "Ihr Steuerbescheid 2025", false),
            &rules,
        );
        assert_eq!(verdict.stream, "steuern");
    }

    #[test]
    fn empty_match_spec_never_fires() {
        let rules = vec![Rule {
            r#match: MatchSpec::default(),
            stream: "werbung".into(),
            note: "should never match".into(),
        }];
        let verdict = classify(&facts("a@b.com", "hello", false), &rules);
        assert_eq!(
            verdict.stream, "aktiv",
            "empty match spec must not act as a catch-all"
        );
    }

    /// Which rung fired is the eligibility rule for the model rung, and it
    /// replaces the only signal that existed before: a rationale string
    /// nothing read. One input per value.
    #[test]
    fn classify_names_who_decided() {
        let rules = vec![Rule {
            r#match: MatchSpec {
                from_contains: Some(vec!["bytes.dev".into()]),
                subject_contains: None,
                has_list_unsubscribe: None,
            },
            stream: "feed".into(),
            note: "curated development newsletter".into(),
        }];
        assert_eq!(
            classify(&facts("hello@bytes.dev", "anything", true), &rules).decided_by,
            DecidedBy::ConfigRule
        );
        assert_eq!(
            classify(&facts("news@shop.example", "Winter SALE", true), &[]).decided_by,
            DecidedBy::Heuristic
        );
        assert_eq!(
            classify(&facts("a.person@example.com", "Re: lunch?", false), &[]).decided_by,
            DecidedBy::Fallback
        );
    }

    /// A round trip over the stored vocabulary, because the strings are a
    /// `CHECK` constraint in `{prefix}_triage_rules` and a mismatch would only
    /// show up as a write failure against the live file.
    #[test]
    fn decided_by_round_trips_through_its_stored_form() {
        use std::str::FromStr;
        for value in [
            DecidedBy::ConfigRule,
            DecidedBy::Heuristic,
            DecidedBy::Fallback,
        ] {
            assert_eq!(DecidedBy::from_str(value.as_str()), Ok(value));
        }
        assert!(DecidedBy::from_str("model").is_err());
    }

    /// A stream added to the `CHECK` constraint without a prompt definition
    /// would reach the model as a bare word it has never been told the meaning
    /// of. It fails here instead.
    #[test]
    fn stream_definitions_cover_every_stream() {
        let named: Vec<&str> = STREAM_DEFINITIONS.iter().map(|(name, _)| *name).collect();
        assert_eq!(named, STREAMS.to_vec());
        for (name, definition) in STREAM_DEFINITIONS {
            assert!(
                definition.len() > 30,
                "{name} has no usable one-line definition"
            );
        }
    }
}
