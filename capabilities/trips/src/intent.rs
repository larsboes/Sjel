//! A sentence to a plan draft that resolves nothing.
//!
//! There is exactly one way to start a trip today: a form that needs an origin
//! picked from `transit.suggest`, destinations, dates and modes typed field by
//! field. "Somewhere warm in October, under 300 euro, by train, long weekend"
//! has no entry point at all.
//!
//! What this produces is the body of that form, filled in, plus a list of what
//! it could not settle. It is deliberately the smallest possible use of a model:
//!
//! - It persists nothing. The output is a `CreatePlan` nobody has submitted.
//! - It emits no EVA code, no price, no feasibility judgement and no plan id.
//!   Resolving a name to a station is `transit`'s job and stays there.
//! - Every destination comes back as a `place:<slug>` with null coordinates,
//!   which is bit-identical to what the dashboard already mints from typed text.
//!   So the operator still has to pick a real station before anything can be
//!   searched, exactly as before.
//!
//! The model's whole job is turning prose into fields. If it invents a station,
//! the worst case is a slug the operator has to correct in a form, which is the
//! same thing that happens when they mistype one.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A request to turn a natural language sentence into a draft trip form.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IntentDraftRequest {
    pub sentence: String,
}

/// A filled-in form, and what it could not settle.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
pub struct IntentDraft {
    /// The `CreatePlan` body, ready for a human to review and submit.
    pub draft: Value,
    /// Fields the sentence did not determine, by name. A caller shows these as
    /// blanks rather than letting a default stand in silently.
    pub unresolved: Vec<String>,
    /// What was inferred rather than stated, in plain words, so the operator can
    /// disagree with a guess instead of discovering it later.
    pub assumptions: Vec<String>,
    /// The sentence this came from, so a draft is traceable to its input.
    pub source_text: String,
}

pub const SYSTEM_PROMPT: &str = "\
You turn a travel sentence into JSON for a trip-planning form.

Reply with ONLY a JSON object, no prose and no code fence:
{\"title\": str, \"destinations\": [str], \"date_start\": \"YYYY-MM-DD\"|null,
 \"date_end\": \"YYYY-MM-DD\"|null, \"interests\": str,
 \"transport_modes\": [\"train\"|\"flight\"|\"bus\"|\"car\"|\"ferry\"|\"bike\"|\"walk\"],
 \"travelers\": [str], \"unresolved\": [str], \"assumptions\": [str]}

Rules:
- Destinations are place names as a person would say them. Never a station code.
- Never invent a date. If the sentence gives no dates, use null and add \"dates\" to unresolved.
- Put anything you inferred rather than read into assumptions.
- At most four destinations.";

/// Builds the request body for the chat-completions endpoint.
///
/// Kept separate from the call so the prompt can be tested without a model
/// running, which is most of what is worth testing here.
pub fn request_body(model: &str, sentence: &str) -> Value {
    serde_json::json!({
        "model": model,
        "messages": [
            { "role": "system", "content": SYSTEM_PROMPT },
            { "role": "user", "content": sentence }
        ],
        "max_tokens": 700,
        "temperature": 0.2
    })
}

/// Slugifies typed text into a place id, exactly as the dashboard's place field
/// does, so a drafted destination is indistinguishable from a typed one and
/// carries the same null coordinates.
pub fn place_slug(name: &str) -> String {
    let slug: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    let trimmed = slug.trim_matches('-').to_string();
    let collapsed = trimmed
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    format!("place:{collapsed}")
}

/// Turns the model's JSON into a `CreatePlan`-shaped body.
///
/// Every field is re-derived here rather than trusted: the model supplies words,
/// this supplies the shape. A model that returns an EVA code, a price or a plan
/// id has those fields dropped on the floor, because nothing reads them.
pub fn draft_from_model_json(sentence: &str, raw: &str) -> Result<IntentDraft, String> {
    draft_from_model_json_on(sentence, raw, &civil_date::today())
}

/// A date the model produced is only kept if it could actually be travelled.
///
/// Measured on 2026-08-11 against the on-device model: asked for "somewhere warm
/// in October" with no year, it returned 2023 both times, and it invented a whole
/// date range for a sentence that named none while simultaneously listing
/// "dates" in its own `unresolved`. A plan silently created for a date three
/// years past is worse than a blank field, so the year is checked here rather
/// than asked for in the prompt. Prompts are not a validation layer.
fn usable_date(value: Option<String>, today: &str) -> Option<String> {
    let value = value?;
    let looks_like_a_date = value.len() == 10
        && value.as_bytes().iter().enumerate().all(|(i, b)| {
            if i == 4 || i == 7 {
                *b == b'-'
            } else {
                b.is_ascii_digit()
            }
        });
    // String comparison is correct for ISO dates and needs no date library.
    (looks_like_a_date && value.as_str() >= today).then_some(value)
}

pub fn draft_from_model_json_on(
    sentence: &str,
    raw: &str,
    today: &str,
) -> Result<IntentDraft, String> {
    let cleaned = strip_fence(raw);
    let value: Value = serde_json::from_str(&cleaned)
        .map_err(|e| format!("the model did not return JSON ({e}): {}", preview(raw)))?;

    let strings = |key: &str| -> Vec<String> {
        value
            .get(key)
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|i| i.as_str())
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    };
    let text = |key: &str| -> Option<String> {
        value
            .get(key)
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty() && *s != "null")
            .map(str::to_string)
    };

    let names: Vec<String> = strings("destinations")
        .into_iter()
        .filter(|name| !is_pseudonym_token(name))
        .take(4)
        .collect();
    let mut unresolved = strings("unresolved");
    if names.is_empty() && !unresolved.iter().any(|u| u == "destinations") {
        unresolved.push("destinations".into());
    }
    // The check decides, not the model's self-report.
    //
    // It reports "dates" as unresolved almost every time, including when it has
    // just returned correct ones: asked for "Munich the 14th to the 16th of
    // September 2026" it answered 2026-09-14/16 and listed "dates" as unresolved
    // in the same object. Trusting that claim threw away dates the sentence
    // plainly gave. Trusting the dates blindly accepts the 2023 it invents for a
    // sentence with no year. So neither is trusted: the date is kept when it is
    // well-formed and not in the past, and `unresolved` is corrected either way.
    let date_start = usable_date(text("date_start"), today);
    let date_end = usable_date(text("date_end"), today);
    let dates_missing = date_start.is_none() || date_end.is_none();
    unresolved.retain(|u| u != "dates");
    if dates_missing {
        unresolved.push("dates".into());
    }

    let destinations: Vec<Value> = names
        .iter()
        .map(|name| {
            serde_json::json!({
                "id": place_slug(name),
                "name": name,
                "kind": "city",
                "latitude": Value::Null,
                "longitude": Value::Null
            })
        })
        .collect();

    // Modes are checked against the enum rather than passed through: an
    // unknown mode would be rejected by the API later, and a draft that cannot
    // be submitted is worse than one with a blank field.
    let modes: Vec<String> = strings("transport_modes")
        .into_iter()
        .map(|m| m.to_lowercase())
        .filter(|m| {
            ["bike", "bus", "car", "ferry", "flight", "train", "walk"].contains(&m.as_str())
        })
        .collect();

    let draft = serde_json::json!({
        "title": text("title").unwrap_or_else(|| sentence.trim().to_string()),
        // Never guessed. The operator's own starting point is a personal fact
        // and belongs to them, not to a sentence parser.
        "origin": Value::Null,
        "destinations": destinations,
        "date_start": date_start,
        "date_end": date_end,
        "interests": text("interests").unwrap_or_default(),
        "transport_modes": modes,
        "travelers": strings("travelers"),
    });

    if !unresolved.iter().any(|u| u == "origin") {
        unresolved.push("origin".into());
    }

    Ok(IntentDraft {
        draft,
        unresolved,
        assumptions: strings("assumptions"),
        source_text: sentence.to_string(),
    })
}

/// Small models fence their JSON however they feel like. Strip it rather than
/// failing a parse over punctuation.
fn strip_fence(raw: &str) -> String {
    let trimmed = raw.trim();
    let without = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .unwrap_or(trimmed);
    without
        .strip_suffix("```")
        .unwrap_or(without)
        .trim()
        .to_string()
}

fn preview(raw: &str) -> String {
    raw.chars().take(160).collect()
}

/// The local model's chat-completions endpoint, `SJEL_INTENT_URL` first.
fn model_url() -> String {
    axon_config::env_var("SJEL_INTENT_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:8091/v1/chat/completions".into())
}

/// How long the server waits for the model before it drafts heuristically.
pub const SERVER_MODEL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Queries the local foundation model with the server's timeout.
pub fn query_model(sentence: &str) -> Result<IntentDraft, String> {
    query_model_within(sentence, SERVER_MODEL_TIMEOUT)
}

/// Queries the local foundation model, waiting at most `timeout`.
///
/// An unreachable model names the command that starts it, because the usual cause
/// is that `foundation-models` is not running.
pub fn query_model_within(
    sentence: &str,
    timeout: std::time::Duration,
) -> Result<IntentDraft, String> {
    let url = model_url();
    let model = axon_config::env_var("SJEL_INTENT_MODEL")
        .unwrap_or_else(|_| "apple-foundationmodel".into());

    let client = axon_http::client(axon_http::Purpose::new("trips-intent"), timeout)
        .map_err(|e| format!("client build: {e}"))?;

    let response = client
        .post(&url)
        .json(&request_body(&model, sentence))
        .send()
        .map_err(|e| {
            format!(
                "could not reach the local model at {url} ({e}). Start it with \
                 `tools/service-runner.sh start foundation-models`, or point \
                 SJEL_INTENT_URL somewhere else."
            )
        })?;

    if !response.status().is_success() {
        return Err(format!("{url} answered {}", response.status()));
    }

    let body: serde_json::Value = response
        .json()
        .map_err(|e| format!("unreadable reply: {e}"))?;
    let content = body["choices"][0]["message"]["content"]
        .as_str()
        .ok_or_else(|| "the reply carried no message content".to_string())?;

    draft_from_model_json(sentence, content)
}

/// Whether a word is a pseudonym token such as `<EMAIL_01>` or `<PERSON_02>`.
///
/// `axon_pseudonymize` mints tokens as `<PREFIX_NN>`. Trimmed like a place name, a
/// token becomes `EMAIL`, a capitalised word that the heuristic would take as a
/// destination. So a token is never a destination.
pub fn is_pseudonym_token(word: &str) -> bool {
    let word = word.trim_matches(|c: char| !(c.is_alphanumeric() || matches!(c, '<' | '>' | '_')));
    let Some(inner) = word.strip_prefix('<').and_then(|w| w.strip_suffix('>')) else {
        return false;
    };
    let Some((prefix, number)) = inner.rsplit_once('_') else {
        return false;
    };
    !prefix.is_empty()
        && prefix.chars().all(|c| c.is_ascii_uppercase() || c == '_')
        && !number.is_empty()
        && number.chars().all(|c| c.is_ascii_digit())
}

/// Fallback heuristic parser when the local model is offline or unconfigured.
pub fn heuristic_draft(sentence: &str) -> IntentDraft {
    heuristic_draft_on(sentence, &civil_date::today())
}

/// Transport modes and the whole words that name them.
///
/// Whole words, not substrings: "ice" is inside Nice and Venice, "car" inside
/// Oscar and Caribbean, "bus" inside business and "rail" inside trail.
const MODE_WORDS: &[(&str, &[&str])] = &[
    (
        "train",
        &["train", "trains", "bahn", "ice", "rail", "railway", "zug"],
    ),
    (
        "flight",
        &["flight", "flights", "fly", "flying", "plane", "flug"],
    ),
    ("bus", &["bus", "buses", "coach"]),
    ("car", &["car", "drive", "driving", "auto"]),
    ("ferry", &["ferry", "boat", "fähre"]),
    ("bike", &["bike", "cycle", "cycling", "fahrrad"]),
    ("walk", &["walk", "walking", "hike", "hiking", "wandern"]),
];

pub fn heuristic_draft_on(sentence: &str, today: &str) -> IntentDraft {
    let lower = sentence.to_lowercase();
    let lower_words: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();

    let modes: Vec<String> = MODE_WORDS
        .iter()
        .filter(|(_, words)| words.iter().any(|w| lower_words.contains(w)))
        .map(|(mode, _)| mode.to_string())
        .collect();

    // Look for ISO dates YYYY-MM-DD
    let mut dates = Vec::new();
    for word in sentence.split_whitespace() {
        let cleaned = word.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-');
        if usable_date(Some(cleaned.to_string()), today).is_some() {
            dates.push(cleaned.to_string());
        }
    }
    let date_start = dates.first().cloned();
    let date_end = dates.get(1).cloned();

    // Destinations: capitalised words after a cue, never a pseudonym token.
    let words: Vec<&str> = sentence.split_whitespace().collect();
    let skip_words = [
        "the",
        "a",
        "an",
        "my",
        "our",
        "to",
        "in",
        "and",
        "or",
        "for",
        "with",
        "by",
        "of",
        "october",
        "november",
        "december",
        "january",
        "february",
        "march",
        "april",
        "may",
        "june",
        "july",
        "august",
        "september",
        "spring",
        "summer",
        "autumn",
        "fall",
        "winter",
        "weekend",
        "monday",
        "tuesday",
        "wednesday",
        "thursday",
        "friday",
        "saturday",
        "sunday",
        "train",
        "flight",
        "bus",
        "car",
        "euro",
        "eur",
        "day",
        "days",
        "trip",
    ];
    let place_word = |raw: &str| -> Option<String> {
        if is_pseudonym_token(raw) {
            return None;
        }
        let cleaned = raw.trim_matches(|c: char| !c.is_alphabetic());
        let capitalised = cleaned.chars().next().is_some_and(char::is_uppercase);
        (capitalised && !skip_words.contains(&cleaned.to_lowercase().as_str()))
            .then(|| cleaned.to_string())
    };
    let mut destinations: Vec<String> = Vec::new();
    let add = |name: String, destinations: &mut Vec<String>| {
        if !destinations.contains(&name) && destinations.len() < 4 {
            destinations.push(name);
        }
    };

    for pair in words.windows(2) {
        let cue = pair[0]
            .trim_matches(|c: char| !c.is_alphabetic())
            .to_lowercase();
        if matches!(
            cue.as_str(),
            "to" | "in" | "visit" | "visiting" | "nach" | "and"
        ) {
            if let Some(name) = place_word(pair[1]) {
                add(name, &mut destinations);
            }
        }
    }

    if destinations.is_empty() {
        for word in words.iter().skip(1) {
            if let Some(name) = place_word(word).filter(|n| n.chars().count() > 2) {
                add(name, &mut destinations);
            }
        }
    }

    let mut unresolved = vec!["origin".to_string()];
    if destinations.is_empty() {
        unresolved.push("destinations".to_string());
    }
    if date_start.is_none() || date_end.is_none() {
        unresolved.push("dates".to_string());
    }

    let dest_values: Vec<Value> = destinations
        .iter()
        .map(|name| {
            serde_json::json!({
                "id": place_slug(name),
                "name": name,
                "kind": "city",
                "latitude": Value::Null,
                "longitude": Value::Null
            })
        })
        .collect();

    let interests: Vec<&str> = [
        "warm",
        "beach",
        "hiking",
        "culture",
        "museum",
        "food",
        "relaxation",
        "nature",
        "mountains",
    ]
    .into_iter()
    .filter(|term| lower.contains(term))
    .collect();

    let draft = serde_json::json!({
        "title": sentence.trim(),
        "origin": Value::Null,
        "destinations": dest_values,
        "date_start": date_start,
        "date_end": date_end,
        "interests": interests.join(", "),
        "transport_modes": modes,
        "travelers": Vec::<String>::new(),
    });

    IntentDraft {
        draft,
        unresolved,
        assumptions: vec![
            "Heuristic draft: local model was unavailable or offline, extracted from text patterns."
                .to_string(),
        ],
        source_text: sentence.to_string(),
    }
}

/// Resolves an intent sentence to a draft, masking what `registry` and the
/// pattern recognisers find before any model sees the sentence.
///
/// What is masked depends on `registry`. The session always masks pattern-shaped
/// values (e-mail addresses, phone numbers, IBANs, URLs, secrets, long numbers) and
/// names that a cue introduces ("Hi Anna", "Anna from Bonn"). A traveler's bare name
/// is masked only when `registry` knows it. Tokens are rehydrated on the way out
/// (PRD §6.2c).
pub fn resolve_draft_pseudonymized(
    sentence: &str,
    registry: &axon_pseudonymize::EntityRegistry,
    session: &mut axon_pseudonymize::PseudonymizerSession,
) -> Result<IntentDraft, String> {
    resolve_draft_pseudonymized_with(sentence, registry, session, query_model)
}

/// [`resolve_draft_pseudonymized`] with the model call injected, so a test does
/// not depend on a live model on 127.0.0.1:8091.
pub fn resolve_draft_pseudonymized_with(
    sentence: &str,
    registry: &axon_pseudonymize::EntityRegistry,
    session: &mut axon_pseudonymize::PseudonymizerSession,
    model: impl FnOnce(&str) -> Result<IntentDraft, String>,
) -> Result<IntentDraft, String> {
    let sentence = sentence.trim();
    if sentence.is_empty() {
        return Err("sentence cannot be empty".to_string());
    }
    let pseudo_text = session.tokenize_text(sentence, registry);
    let mut draft = match model(&pseudo_text) {
        Ok(d) => d,
        Err(_) => heuristic_draft(&pseudo_text),
    };

    // Rehydrate any masked tokens in the JSON draft and string fields
    session.rehydrate_json(&mut draft.draft);
    draft.source_text = sentence.to_string();
    draft.assumptions = draft
        .assumptions
        .into_iter()
        .map(|a| session.rehydrate_text(&a))
        .collect();
    draft.unresolved = draft
        .unresolved
        .into_iter()
        .map(|u| session.rehydrate_text(&u))
        .collect();
    Ok(draft)
}

/// Resolves an intent sentence to a draft by querying the model if reachable,
/// or falling back to heuristic parsing.
///
/// The registry here is EMPTY: trips does not read the people registry. So only
/// pattern-shaped values and cue-introduced names are masked, and a traveler's
/// bare name reaches the model as typed. See [`resolve_draft_pseudonymized`].
pub fn resolve_draft_or_heuristic(sentence: &str) -> Result<IntentDraft, String> {
    resolve_draft_or_heuristic_with(sentence, query_model)
}

/// [`resolve_draft_or_heuristic`] with the model call injected.
pub fn resolve_draft_or_heuristic_with(
    sentence: &str,
    model: impl FnOnce(&str) -> Result<IntentDraft, String>,
) -> Result<IntentDraft, String> {
    let registry = axon_pseudonymize::Pseudonymizer::builder().build();
    let mut session = axon_pseudonymize::PseudonymizerSession::new();
    resolve_draft_pseudonymized_with(sentence, &registry, &mut session, model)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sentence_becomes_a_submittable_form_that_resolves_nothing() {
        let raw = r#"{"title":"Warm long weekend","destinations":["Valencia","Lisbon"],
            "date_start":null,"date_end":null,"interests":"warm, under 300 euro",
            "transport_modes":["train"],"travelers":[],
            "unresolved":["dates"],"assumptions":["read 'warm' as southern Europe"]}"#;
        let drafted =
            draft_from_model_json("somewhere warm in October under 300 by train", raw).unwrap();

        let destinations = drafted.draft["destinations"].as_array().unwrap();
        assert_eq!(destinations.len(), 2);
        // A drafted destination is bit-identical to a typed one: a slug and no
        // coordinates, so nothing downstream can mistake it for a resolved place.
        assert_eq!(destinations[0]["id"], "place:valencia");
        assert!(destinations[0]["latitude"].is_null());
        // The origin is never guessed.
        assert!(drafted.draft["origin"].is_null());
        assert!(drafted.unresolved.contains(&"origin".to_string()));
        assert!(drafted.unresolved.contains(&"dates".to_string()));
        assert_eq!(drafted.assumptions.len(), 1);
        assert_eq!(drafted.draft["transport_modes"][0], "train");
    }

    /// The model supplies words; this supplies the shape. Anything it invents
    /// outside the contract has to fall on the floor.
    #[test]
    fn invented_fields_are_dropped_rather_than_carried() {
        let raw = r#"{"title":"Munich","destinations":["Munich"],
            "date_start":"2026-10-02","date_end":"2026-10-05",
            "transport_modes":["train","teleport"],
            "eva":"8000261","price_eur":210,"plan_id":"trip:plan:made-up",
            "unresolved":[],"assumptions":[]}"#;
        let drafted = draft_from_model_json("munich in october", raw).unwrap();
        let draft = &drafted.draft;
        for invented in ["eva", "price_eur", "plan_id", "id", "status"] {
            assert!(
                draft.get(invented).is_none(),
                "{invented} must not survive into the draft"
            );
        }
        // An unknown transport mode is dropped, because the API would reject the
        // whole write and a draft that cannot be submitted is worse than a blank.
        let modes = draft["transport_modes"].as_array().unwrap();
        assert_eq!(modes.len(), 1);
        assert_eq!(modes[0], "train");
    }

    /// The measured failure, pinned. Asked for "somewhere warm in October" with
    /// no year, the on-device model answered 2023 -- twice. A plan quietly
    /// created for a date three years past is worse than a blank field.
    #[test]
    fn a_hallucinated_past_date_is_dropped_rather_than_drafted() {
        let raw = r#"{"destinations":["Valencia"],"date_start":"2023-10-11",
            "date_end":"2023-10-18","unresolved":[],"assumptions":[]}"#;
        let drafted = draft_from_model_json_on("warm in october", raw, "2026-08-11").unwrap();
        assert!(drafted.draft["date_start"].is_null());
        assert!(drafted.draft["date_end"].is_null());
        assert!(drafted.unresolved.contains(&"dates".to_string()));

        // A future date the sentence really did give is kept.
        let good = r#"{"destinations":["Munich"],"date_start":"2026-09-14",
            "date_end":"2026-09-16","unresolved":[],"assumptions":[]}"#;
        let drafted = draft_from_model_json_on("munich 14-16 sept", good, "2026-08-11").unwrap();
        assert_eq!(drafted.draft["date_start"], "2026-09-14");
        assert!(!drafted.unresolved.contains(&"dates".to_string()));

        // Garbage that is not a date at all is also dropped.
        let bad = r#"{"destinations":["X"],"date_start":"next tuesday","unresolved":[]}"#;
        assert!(draft_from_model_json_on("x", bad, "2026-08-11")
            .unwrap()
            .draft["date_start"]
            .is_null());
    }

    /// The model lists "dates" as unresolved almost every time, including when
    /// it has just returned correct ones. Measured: asked for "Munich the 14th
    /// to the 16th of September 2026" it answered 2026-09-14/16 and called dates
    /// unresolved in the same object. The check decides; the self-report does not.
    #[test]
    fn a_wrong_self_report_does_not_discard_good_dates() {
        let raw = r#"{"destinations":["Munich"],"date_start":"2026-09-14",
            "date_end":"2026-09-16","unresolved":["dates"],"assumptions":[]}"#;
        let drafted = draft_from_model_json_on("munich sept 2026", raw, "2026-08-11").unwrap();
        assert_eq!(drafted.draft["date_start"], "2026-09-14");
        assert_eq!(drafted.draft["date_end"], "2026-09-16");
        assert!(
            !drafted.unresolved.contains(&"dates".to_string()),
            "the claim is corrected when the dates are usable"
        );

        // And the reverse: a confident model with a past date still loses it.
        let stale = r#"{"destinations":["Nice"],"date_start":"2023-10-11",
            "date_end":"2023-10-18","unresolved":[],"assumptions":[]}"#;
        let drafted = draft_from_model_json_on("warm", stale, "2026-08-11").unwrap();
        assert!(drafted.draft["date_start"].is_null());
        assert!(drafted.unresolved.contains(&"dates".to_string()));
    }

    #[test]
    fn a_fenced_or_broken_reply_is_handled_rather_than_panicking() {
        let fenced = "```json\n{\"destinations\":[\"Porto\"],\"unresolved\":[]}\n```";
        let drafted = draft_from_model_json("porto", fenced).unwrap();
        assert_eq!(drafted.draft["destinations"][0]["name"], "Porto");

        let error = draft_from_model_json("x", "I think you should go to Porto!")
            .expect_err("prose is not a draft");
        assert!(error.contains("did not return JSON"), "got: {error}");
    }

    #[test]
    fn place_slugs_match_what_the_dashboard_mints_from_typed_text() {
        assert_eq!(place_slug("Valencia"), "place:valencia");
        assert_eq!(place_slug("Frankfurt(Main)Hbf"), "place:frankfurt-main-hbf");
        assert_eq!(place_slug("  Bonn  Hbf "), "place:bonn-hbf");
    }

    #[test]
    fn the_prompt_forbids_the_things_the_model_must_not_decide() {
        assert!(SYSTEM_PROMPT.contains("Never invent a date"));
        assert!(SYSTEM_PROMPT.contains("Never a station code"));
        assert!(request_body("apple-on-device", "hi")["messages"][1]["content"] == "hi");
    }

    #[test]
    fn heuristic_draft_extracts_places_modes_and_dates() {
        let sentence = "Weekend trip to Munich and Salzburg by train from 2026-10-05 to 2026-10-10";
        let draft = heuristic_draft_on(sentence, "2026-09-01");
        let dests = draft.draft["destinations"].as_array().unwrap();
        assert_eq!(dests.len(), 2);
        assert_eq!(dests[0]["name"], "Munich");
        assert_eq!(dests[0]["id"], "place:munich");
        assert_eq!(dests[1]["name"], "Salzburg");
        assert_eq!(dests[1]["id"], "place:salzburg");
        assert_eq!(draft.draft["transport_modes"][0], "train");
        assert_eq!(draft.draft["date_start"], "2026-10-05");
        assert_eq!(draft.draft["date_end"], "2026-10-10");
        assert!(!draft.unresolved.contains(&"dates".to_string()));
        assert!(draft.unresolved.contains(&"origin".to_string()));
    }

    #[test]
    fn heuristic_draft_handles_missing_dates_and_interests() {
        let sentence = "Somewhere warm with hiking by flight";
        let draft = heuristic_draft_on(sentence, "2026-09-01");
        assert!(draft.draft["date_start"].is_null());
        assert!(draft.unresolved.contains(&"dates".to_string()));
        assert!(draft.unresolved.contains(&"destinations".to_string()));
        assert_eq!(draft.draft["transport_modes"][0], "flight");
        let interests = draft.draft["interests"].as_str().unwrap();
        assert!(interests.contains("warm"));
        assert!(interests.contains("hiking"));
    }

    fn offline(_: &str) -> Result<IntentDraft, String> {
        Err("model offline (test stub)".into())
    }

    #[test]
    fn resolve_draft_or_heuristic_handles_fallback() {
        let draft = resolve_draft_or_heuristic_with("Trip to Vienna by train", offline).unwrap();
        assert_eq!(draft.draft["destinations"][0]["name"], "Vienna");
        assert_eq!(draft.draft["transport_modes"][0], "train");
        assert!(draft.assumptions[0].starts_with("Heuristic draft"));
    }

    #[test]
    fn intent_pseudonymization_protects_entities_and_rehydrates_draft() {
        let registry = axon_pseudonymize::Pseudonymizer::builder().build();
        let mut session = axon_pseudonymize::PseudonymizerSession::new();
        let sentence = "Trip to Berlin by train with traveler contact user@axon.local";
        let seen = std::cell::RefCell::new(String::new());
        let draft = resolve_draft_pseudonymized_with(sentence, &registry, &mut session, |text| {
            *seen.borrow_mut() = text.to_string();
            offline(text)
        })
        .unwrap();
        // The model was handed the masked sentence, never the address.
        assert!(
            !seen.borrow().contains("user@axon.local"),
            "{}",
            seen.borrow()
        );
        assert!(seen.borrow().contains("<EMAIL_"), "{}", seen.borrow());
        assert_eq!(draft.draft["destinations"][0]["name"], "Berlin");
        assert_eq!(
            draft.draft["destinations"].as_array().unwrap().len(),
            1,
            "the e-mail token is not a destination"
        );
        assert_eq!(draft.draft["transport_modes"][0], "train");
        // Ensure source_text retains original sentence
        assert_eq!(draft.source_text, sentence);
    }

    /// A model answer is used when the injected call succeeds, and its tokens rehydrate.
    #[test]
    fn a_stubbed_model_answer_is_used_and_rehydrated() {
        let draft = resolve_draft_or_heuristic_with("Porto, mail me at a@b.example", |text| {
            let token = text
                .split_whitespace()
                .find(|w| is_pseudonym_token(w))
                .expect("the address was masked");
            let raw = format!(
                r#"{{"title":"Porto","destinations":["Porto"],"interests":"{token}","unresolved":[],"assumptions":[]}}"#
            );
            draft_from_model_json(text, &raw)
        })
        .unwrap();
        assert_eq!(draft.draft["destinations"][0]["name"], "Porto");
        assert_eq!(draft.draft["interests"], "a@b.example");
    }

    /// Whole words only: "ice" is inside Venice and Nice, "car" inside Oscar and
    /// Caribbean, "bus" inside business, "rail" inside trail.
    #[test]
    fn transport_modes_match_whole_words_only() {
        let modes = |sentence: &str| {
            heuristic_draft_on(sentence, "2026-09-01").draft["transport_modes"].clone()
        };
        assert_eq!(modes("Trip to Venice"), serde_json::json!([]));
        assert_eq!(modes("Nice with Oscar"), serde_json::json!([]));
        assert_eq!(modes("Caribbean business trail"), serde_json::json!([]));
        assert_eq!(modes("Venice by ICE"), serde_json::json!(["train"]));
        assert_eq!(
            modes("bus or car to Bonn"),
            serde_json::json!(["bus", "car"])
        );
    }

    /// `<EMAIL_01>` trimmed like a place name is `EMAIL`, which looked like a city.
    #[test]
    fn pseudonym_tokens_are_never_destinations() {
        assert!(is_pseudonym_token("<EMAIL_01>"));
        assert!(is_pseudonym_token("(<PERSON_12>),"));
        assert!(!is_pseudonym_token("Porto"));
        assert!(!is_pseudonym_token("<b>"));

        let draft = heuristic_draft_on("Trip to <EMAIL_01> and Porto", "2026-09-01");
        let names: Vec<_> = draft.draft["destinations"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d["name"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(names, vec!["Porto"]);

        // The fallback scan skips tokens too.
        let draft = heuristic_draft_on("Ask <PERSON_01> about <PHONE_01>", "2026-09-01");
        assert!(draft.unresolved.contains(&"destinations".to_string()));

        // And a model that echoes a token as a destination loses it.
        let raw = r#"{"destinations":["<EMAIL_01>","Lisbon"],"unresolved":[]}"#;
        let drafted = draft_from_model_json_on("x", raw, "2026-09-01").unwrap();
        assert_eq!(drafted.draft["destinations"].as_array().unwrap().len(), 1);
        assert_eq!(drafted.draft["destinations"][0]["name"], "Lisbon");
    }
}
