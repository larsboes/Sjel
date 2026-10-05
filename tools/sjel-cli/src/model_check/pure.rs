//! The pure half of `tools/model-check`: family and version comparison, catalogue and refusal
//! parsing, and the decisions that turn those into a status. No I/O, so every rule the tool
//! applies can be asserted directly.
//!
//! The TypeScript this replaces had no test file, so the cases at the bottom of this file are
//! written from the behaviours its own comments record rather than ported from a suite.

use regex::Regex;
use serde_json::Value;
use std::sync::OnceLock;

/// `[0-9]+(\.[0-9]+)*` — one version run: `3`, `3.6`, `3.6.1`.
fn version_run() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"[0-9]+(?:\.[0-9]+)*").expect("a literal pattern"))
}

/// The id with its version numbers removed — the model's LINE rather than its version.
///
/// `gemini-3.6-flash` and `gemini-3.7-flash` are the same line at different versions;
/// `nemotron-3-nano` and `nemotron-3-super` are NOT — they are size tiers, and calling a bigger
/// one "newer" would advise an upgrade nobody asked for onto a model with different economics.
/// So only an id that matches the family exactly, differing in those numbers, can be newer.
pub fn family(id: &str) -> String {
    version_run().replace_all(id, "#").into_owned()
}

/// Every number in an id, flattened: `gemini-3.6-flash` becomes `[3, 6]`.
pub fn version_key(id: &str) -> Vec<u64> {
    version_run()
        .find_iter(id)
        .flat_map(|m| {
            m.as_str()
                .split('.')
                .map(|part| part.parse::<u64>().unwrap_or(0))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Whether `candidate` is a later version than `current`, compared element by element with a
/// missing component read as zero.
pub fn newer(candidate: &str, current: &str) -> bool {
    let a = version_key(candidate);
    let b = version_key(current);
    for i in 0..a.len().max(b.len()) {
        let left = a.get(i).copied().unwrap_or(0);
        let right = b.get(i).copied().unwrap_or(0);
        if left != right {
            return left > right;
        }
    }
    false
}

/// The names a catalogue entry is allowed to satisfy.
///
/// Ollama resolves an untagged name to its `:latest` tag: `ollama run bge-m3` and the `bge-m3:latest`
/// its catalogue reports are one model. A role naming the untagged form names it the way the CLI
/// does, so an exact-string test called an installed, answering model missing — and a check that
/// cries wolf is the one the next person learns to ignore.
pub fn declared_as(is_ollama: bool, id: &str) -> Vec<String> {
    if is_ollama {
        if let Some(stripped) = id.strip_suffix(":latest") {
            return vec![id.to_owned(), stripped.to_owned()];
        }
    }
    vec![id.to_owned()]
}

/// The model ids a catalogue body lists, or the reason it is not one.
///
/// `data[].id` first, then `models[].name` — the order the TypeScript tested them in. A leading
/// `models/` is stripped from an OpenAi id, because some shims prefix it.
pub fn catalogue_ids(body: &Value) -> Result<Vec<String>, String> {
    if let Some(entries) = body.get("data").and_then(Value::as_array) {
        return Ok(entries
            .iter()
            .map(|m| {
                let raw = js_string(m.get("id").unwrap_or(&Value::Null));
                raw.strip_prefix("models/")
                    .map(str::to_owned)
                    .unwrap_or(raw)
            })
            .collect());
    }
    if let Some(entries) = body.get("models").and_then(Value::as_array) {
        return Ok(entries
            .iter()
            .map(|m| js_string(m.get("name").unwrap_or(&Value::Null)))
            .collect());
    }
    Err("catalogue in an unrecognised shape".to_owned())
}

/// What a refusal says about itself, when it says anything useful.
///
/// A 429 is where a provider states its real limit, and reading it is how `max_requests_per_day`
/// stops being a guess. On 2026-08-30 this deployment declared 1000/day for a Gemini role whose
/// free-tier quota is 20 — the provider had been naming that number in every rejection for as long
/// as the role had been wrong, in `error.details[].violations[].quotaValue`. Cloudflare meters
/// something else entirely and says so differently, hence the fallback to a short excerpt rather
/// than a second provider-shaped parser.
pub fn refusal_detail(body: &str) -> (String, Option<f64>) {
    if let Ok(parsed) = serde_json::from_str::<Value>(body) {
        let root = parsed
            .as_array()
            .and_then(|a| a.first())
            .cloned()
            .unwrap_or(parsed);
        let error = root.get("error");
        if let Some(details) = error
            .and_then(|e| e.get("details"))
            .and_then(Value::as_array)
        {
            for detail in details {
                let Some(violations) = detail.get("violations").and_then(Value::as_array) else {
                    continue;
                };
                for violation in violations {
                    // `!== undefined`, so a JSON null counts as stated — and `Number(null)` is 0,
                    // which is what the TypeScript would have compared against.
                    let Some(quota) = violation.get("quotaValue") else {
                        continue;
                    };
                    let id = violation
                        .get("quotaId")
                        .and_then(Value::as_str)
                        .map(|q| format!(" ({q})"))
                        .unwrap_or_default();
                    let stated = js_number(quota);
                    return (
                        format!("provider states a quota of {}{id}", js_string(quota)),
                        Some(stated),
                    );
                }
            }
        }
        if let Some(message) = error.and_then(|e| e.get("message")).and_then(Value::as_str) {
            return (slice_chars(message, 120), None);
        }
    }
    (slice_chars(squeeze_whitespace(body).trim(), 120), None)
}

/// Whether a probe's reply counts as the model answering. A probe that came back as an HTTP status
/// or a timeout is not an answer.
pub fn answered(reply: &str) -> bool {
    reply == "answers" || reply.starts_with("answers (")
}

/// `libs/inference` resolves rungs by name — `role_on("embedding", …)` is how a caller asks for the
/// retrieval rung — so the name is the existing contract for what a role is, and the right probe
/// follows from that rather than from a new field. Asking `bge-m3` to reply "OK" gets an HTTP 4xx
/// and reports a model that is installed, served and correct as broken.
pub fn asks_for_a_vector(role_name: &str) -> bool {
    let bytes = role_name.as_bytes();
    for (i, _) in role_name.match_indices("embedding") {
        let before_ok = i == 0 || bytes[i - 1] == b'_';
        let after = i + "embedding".len();
        let after_ok = after == role_name.len() || bytes[after] == b'_';
        if before_ok && after_ok {
            return true;
        }
    }
    false
}

/// A JSON object as its declaration order, which `serde_json`'s own `Map` does not give: it is a
/// `BTreeMap` unless the `preserve_order` feature is on, and this crate deliberately does not turn
/// it on (it would change `serde_json::Map` for every consumer of this binary — the same reason
/// `updates/parse.rs` carries its own order-preserving map).
///
/// The order matters here: `inference.json` lists the local rungs first and the cloud rungs by
/// failover priority, and that is the order the report has always printed. Sorting would be
/// deterministic but would throw away something a person wrote.
#[derive(Debug, Default)]
pub struct OrderedMap(pub Vec<(String, Value)>);

impl<'de> serde::Deserialize<'de> for OrderedMap {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = OrderedMap;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a JSON object")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut access: A,
            ) -> Result<OrderedMap, A::Error> {
                let mut entries = Vec::new();
                while let Some((key, value)) = access.next_entry::<String, Value>()? {
                    entries.push((key, value));
                }
                Ok(OrderedMap(entries))
            }
        }
        deserializer.deserialize_map(Visitor)
    }
}

/// `s.slice(0, n)`, counting characters rather than bytes so a multi-byte body cannot split.
pub fn slice_chars(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// `s.replace(/\s+/g, " ")` — every run of whitespace becomes one space.
pub fn squeeze_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_space = false;
    for c in s.chars() {
        if c.is_whitespace() {
            if !in_space {
                out.push(' ');
                in_space = true;
            }
        } else {
            out.push(c);
            in_space = false;
        }
    }
    out
}

/// `String(value)` for the values a provider body holds. An absent or null field is `"null"`,
/// which is what `String(undefined)` and `String(null)` both produce once read off an object.
pub fn js_string(value: &Value) -> String {
    match value {
        Value::Null => "null".to_owned(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        Value::Array(_) => String::new(),
        Value::Object(_) => "[object Object]".to_owned(),
    }
}

/// `Number(value)`: a numeric string parses, a null is 0, anything else unparseable is `NaN`.
pub fn js_number(value: &Value) -> f64 {
    match value {
        Value::Null => 0.0,
        Value::Bool(b) => {
            if *b {
                1.0
            } else {
                0.0
            }
        }
        Value::Number(n) => n.as_f64().unwrap_or(f64::NAN),
        Value::String(s) => s.trim().parse::<f64>().unwrap_or(f64::NAN),
        _ => f64::NAN,
    }
}

/// The status a role's answer produces, and whether it counts as a failure.
///
/// Kept here rather than in the loop because the two call sites that decide it — a catalogue that
/// could not be read and one that was — disagree about what an unanswered probe means, and that
/// disagreement is the whole reason the two questions are separate.
pub fn status_for(answer: bool) -> &'static str {
    if answer {
        "ok"
    } else {
        "unreachable"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_family_is_the_id_with_its_versions_removed() {
        assert_eq!(family("gemini-3.6-flash"), "gemini-#-flash");
        assert_eq!(family("gemini-3.7-flash"), "gemini-#-flash");
        assert_eq!(family("nemotron-3-nano-30b-a3b"), "nemotron-#-nano-#b-a#b");
        assert_eq!(family("bge-m3"), "bge-m#");
    }

    /// A size tier is not a version: `nano` and `super` are different models with different
    /// economics, so a bigger one must not be offered as "newer".
    #[test]
    fn a_size_tier_is_not_a_newer_version() {
        assert!(newer("gemini-3.7-flash", "gemini-3.6-flash"));
        assert!(!newer("gemini-3.6-flash", "gemini-3.7-flash"));
        assert!(!newer("nemotron-3-super", "nemotron-3-nano"));
        assert_eq!(version_key("gemini-3.6-flash"), vec![3, 6]);
        assert_eq!(version_key("bge-m3"), vec![3]);
    }

    #[test]
    fn a_missing_version_component_reads_as_zero() {
        assert!(newer("qwen3.8-27b", "qwen3.8"));
        assert!(!newer("qwen3", "qwen3.0.0"));
        assert!(!newer("bge-m3", "bge-m3"));
    }

    /// `ollama run bge-m3` and the `bge-m3:latest` its catalogue reports are one model.
    #[test]
    fn an_ollama_latest_tag_is_the_same_model_as_the_untagged_name() {
        assert_eq!(
            declared_as(true, "bge-m3:latest"),
            vec!["bge-m3:latest".to_owned(), "bge-m3".to_owned()]
        );
        assert_eq!(declared_as(true, "bge-m3"), vec!["bge-m3".to_owned()]);
        // An OpenAI-style backend has no tagging convention, so nothing is stripped.
        assert_eq!(
            declared_as(false, "bge-m3:latest"),
            vec!["bge-m3:latest".to_owned()]
        );
    }

    #[test]
    fn a_catalogue_is_read_from_data_then_models() {
        assert_eq!(
            catalogue_ids(
                &json!({"data": [{"id": "gemini-3.7-flash"}, {"id": "models/gemini-3.6-flash"}]})
            )
            .unwrap(),
            vec!["gemini-3.7-flash".to_owned(), "gemini-3.6-flash".to_owned()]
        );
        assert_eq!(
            catalogue_ids(&json!({"models": [{"name": "bge-m3:latest"}]})).unwrap(),
            vec!["bge-m3:latest".to_owned()]
        );
        assert_eq!(
            catalogue_ids(&json!({"data": {}})).unwrap_err(),
            "catalogue in an unrecognised shape"
        );
        assert_eq!(
            catalogue_ids(&json!({"error": "nope"})).unwrap_err(),
            "catalogue in an unrecognised shape"
        );
    }

    /// The provider's own number, which is what stops `max_requests_per_day` being a community
    /// directory's guess.
    #[test]
    fn a_quota_is_read_out_of_a_refusal() {
        let (text, quota) = refusal_detail(
            &json!({"error": {"details": [{"violations": [{"quotaValue": "20", "quotaId": "GenerateRequestsPerDayPerProjectPerModel"}]}]}})
                .to_string(),
        );
        assert_eq!(
            text,
            "provider states a quota of 20 (GenerateRequestsPerDayPerProjectPerModel)"
        );
        assert_eq!(quota, Some(20.0));
    }

    #[test]
    fn a_refusal_with_no_quota_falls_back_to_its_message_then_to_an_excerpt() {
        let (text, quota) =
            refusal_detail(&json!({"error": {"message": "high demand"}}).to_string());
        assert_eq!(text, "high demand");
        assert_eq!(quota, None);

        let (text, quota) = refusal_detail("  not   json\n\nat all  ");
        assert_eq!(text, "not json at all");
        assert_eq!(quota, None);
    }

    /// A JSON null counts as stated, and `Number(null)` is 0 — the TypeScript's own reading.
    #[test]
    fn a_null_quota_value_is_still_a_stated_quota() {
        let (text, quota) = refusal_detail(
            &json!({"error": {"details": [{"violations": [{"quotaValue": null}]}]}}).to_string(),
        );
        assert_eq!(text, "provider states a quota of null");
        assert_eq!(quota, Some(0.0));
    }

    #[test]
    fn only_an_answer_counts_as_an_answer() {
        assert!(answered("answers"));
        assert!(answered("answers (1024-dimensional)"));
        assert!(!answered("answers HTTP 503 — high demand"));
        assert!(!answered("no answer within 60s"));
        assert!(!answered(""));
    }

    #[test]
    fn a_role_asks_for_a_vector_by_name() {
        assert!(asks_for_a_vector("embedding"));
        assert!(asks_for_a_vector("cloud_embedding_light"));
        assert!(asks_for_a_vector("local_embedding"));
        assert!(!asks_for_a_vector("summarization"));
        assert!(!asks_for_a_vector("embeddings_extra"));
        assert!(!asks_for_a_vector("cloud_embeddingix"));
    }

    #[test]
    fn an_object_keeps_its_declaration_order() {
        let parsed: OrderedMap = serde_json::from_str(r#"{"z": 1, "a": 2, "m": 3}"#).unwrap();
        let names: Vec<&str> = parsed.0.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(names, vec!["z", "a", "m"]);
        assert_eq!(parsed.0[0].1, json!(1));
        assert!(serde_json::from_str::<OrderedMap>("[1, 2]").is_err());
    }

    #[test]
    fn an_excerpt_is_squeezed_and_cut_at_a_character_boundary() {
        assert_eq!(squeeze_whitespace("  a  b\n\nc  "), " a b c ");
        assert_eq!(slice_chars("äöü", 2), "äö");
        assert_eq!(slice_chars("abc", 9), "abc");
    }
}
