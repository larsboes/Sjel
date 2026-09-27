//! Rung 1 and rung 2 of the redaction ladder (PRD §6.2): the shape detectors and the
//! contextual person cues, as word-level predicates.
//!
//! **This is the only copy.** `capabilities/comms/src/cloud_derivative.rs` (the destructive
//! `prepare` path, `REDACTION_VERSION`) and [`crate::session`] (the reversible path,
//! `PSEUDONYMIZE_VERSION`, PRD §6.2c) both call these functions. Two copies drifted once
//! already: the reversible path's number rule accepted only digits, dashes and spaces and
//! leaked `Tel. 0721/1234567`, `Konto 12.345.678` and `geb. 01.02.1990`, which the
//! destructive path caught. A rule change here changes both paths, and the frozen-corpus
//! gate (`comms-redaction-eval`, both modes) measures both.
//!
//! Each predicate takes one whitespace-separated word, as the caller split it.

/// The entity decoder, re-exported rather than copied. `libs/extraction` owns the table and
/// the ordering argument (`&amp;` last); a second table is how the two drift apart.
pub use sjel_extraction::decode_basic_entities;

pub fn looks_like_url(value: &str) -> bool {
    value.starts_with("http://") || value.starts_with("https://") || value.starts_with("www.")
}

pub fn looks_like_email(value: &str) -> bool {
    let trimmed = value.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '@' && c != '.');
    let Some((local, domain)) = trimmed.split_once('@') else {
        return false;
    };
    !local.is_empty() && domain.contains('.')
}

pub fn looks_like_iban(value: &str) -> bool {
    let cleaned = value
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>();
    (15..=34).contains(&cleaned.len())
        && cleaned.chars().take(2).all(|c| c.is_ascii_alphabetic())
        && cleaned.chars().skip(2).take(2).all(|c| c.is_ascii_digit())
        && cleaned.chars().skip(4).all(|c| c.is_ascii_alphanumeric())
}

pub fn looks_like_phone(value: &str) -> bool {
    let digit_count = value.chars().filter(char::is_ascii_digit).count();
    digit_count >= 7
        && (value.trim_start().starts_with('+')
            || value.contains('-')
            || value.contains('(')
            || value.contains(')'))
}

/// Any word that carries six or more digits, whatever else it carries.
///
/// Deliberately wide. A reference (`#1234567`), a landline written with a slash
/// (`0721/1234567`), an account number with dots (`12.345.678`) and a birth date
/// (`01.02.1990`) are all identifiers, and none of them is digits-only. Over-redaction costs
/// summary quality; a leak costs the privacy claim (PRD §6.2).
pub fn looks_like_sensitive_number(value: &str) -> bool {
    value.chars().filter(char::is_ascii_digit).count() >= 6
}

pub fn looks_like_token(value: &str) -> bool {
    let cleaned: String = value.chars().filter(char::is_ascii_alphanumeric).collect();
    cleaned.len() >= 16
        && cleaned.chars().any(|c| c.is_ascii_alphabetic())
        && cleaned.chars().any(|c| c.is_ascii_digit())
}

/// A login handle: letters and digits fused into one opaque word.
///
/// PRD D14's second gap. `labo2764` identifies its owner as surely as the name on the
/// account, and it passes every other recognizer here — too short for [`looks_like_token`],
/// too few digits for [`looks_like_sensitive_number`], and lowercase, so
/// [`looks_like_person_name`] refuses it. Only reachable behind a person cue; see the call
/// sites.
pub fn looks_like_handle(value: &str) -> bool {
    let cleaned = value.trim_matches(|c: char| !c.is_ascii_alphanumeric());
    let letters = cleaned.chars().filter(char::is_ascii_alphabetic).count();
    let digits = cleaned.chars().filter(char::is_ascii_digit).count();
    (4..=32).contains(&cleaned.len())
        && cleaned.chars().all(|c| c.is_ascii_alphanumeric())
        && letters >= 2
        && digits >= 2
}

pub fn looks_like_person_name(value: &str) -> bool {
    let cleaned = value.trim_matches(|c: char| !c.is_alphabetic() && c != '-' && c != '\'');
    let mut chars = cleaned.chars();
    cleaned.chars().filter(|c| c.is_alphabetic()).count() >= 2
        && chars.next().is_some_and(char::is_uppercase)
        && chars.all(|c| c.is_alphabetic() || c == '-' || c == '\'')
}

/// One word reduced to the form a cue test compares against: entities decoded, lowercased,
/// surrounding punctuation dropped.
///
/// The decode is not decoration. Stored mail still holds `&#39;`, so the word that carries
/// the "I'm X" cue is literally `I&#39;m`, and a matcher written against `i'm` would never
/// fire on the corpus it was written for.
///
/// The apostrophe survives the trim on purpose. It is the only thing separating the English
/// cue `i'm` from the German preposition `im`, which precedes a capitalised noun in a
/// language that capitalises every noun.
pub fn cue_word(value: &str) -> String {
    decode_basic_entities(value)
        .to_lowercase()
        .trim_matches(|c: char| !c.is_alphanumeric() && c != '\'')
        .to_string()
}

/// Whether the words before `index` announce that the next word names a person.
///
/// Two families, and the second one is PRD D14. A salutation gate sees `Dear X` and nothing
/// else; the shadow evaluation (`<overlay>/config/comms-redaction-shadow.md`, 2026-08-30)
/// found three of Presidio's four unique catches were English self-introductions mid-body.
///
/// German self-introduction (`ich bin X`) is deliberately absent: German capitalises every
/// noun, so `ich bin Ihr Ansprechpartner` would redact a pronoun. The rule is added when a
/// labelled miss asks for it, not before.
pub fn introduces_person(tokens: &[&str], index: usize) -> bool {
    let word = |back: usize| index.checked_sub(back).map(|i| cue_word(tokens[i]));
    let Some(previous) = word(1) else {
        return false;
    };
    match previous.as_str() {
        "dear" | "hello" | "hi" | "hallo" | "liebe" | "lieber" => true,
        "i'm" => true,
        "am" => word(2).as_deref() == Some("i"),
        "is" => matches!(word(2).as_deref(), Some("this") | Some("name")),
        _ => false,
    }
}

/// Whether the word at `index` is a person named by the organisation they are from.
///
/// PRD D14's first gap. `X from Y` is a person only when `Y` is a proper noun too — the
/// shape of "co-hosted by Rayn from Scriptbee", not of "Regards from Berlin". German is
/// untouched by construction: `from` is not a German word.
pub fn names_a_person_in_apposition(tokens: &[&str], index: usize) -> bool {
    looks_like_person_name(tokens[index])
        && tokens
            .get(index + 1)
            .is_some_and(|next| cue_word(next) == "from")
        && tokens
            .get(index + 2)
            .is_some_and(|after| looks_like_person_name(after))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The four probes the reversible path leaked before the two copies were merged. Each is
    /// an identifier, and none of them is digits-and-dashes only.
    #[test]
    fn numbers_with_separators_other_than_dashes_are_sensitive() {
        for word in ["#1234567", "0721/1234567", "12.345.678", "01.02.1990"] {
            assert!(looks_like_sensitive_number(word), "{word}");
        }
        for word in ["2026", "12345", "iPhone15", "v1.2.3"] {
            assert!(!looks_like_sensitive_number(word), "{word}");
        }
    }

    #[test]
    fn a_handle_up_to_32_characters_is_still_a_handle() {
        assert!(looks_like_handle("labo2764"));
        assert!(looks_like_handle("abcdefghijklmnopqrstuvwxyz12345"));
        assert!(!looks_like_handle("abcdefghijklmnopqrstuvwxyz1234567"));
    }

    #[test]
    fn the_cue_word_decodes_the_entity_mail_actually_holds() {
        assert_eq!(cue_word("I&#39;m"), "i'm");
        assert_eq!(cue_word("im"), "im");
    }
}
