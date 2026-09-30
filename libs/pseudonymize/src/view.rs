//! The agent view of a JSON response: what a caller outside this machine may read (ISA F9).
//!
//! Three rules, by field name, because a response carries no other signal:
//!
//! - **Structural** fields stay verbatim: ids, timestamps, enums and classes. Tokenizing them
//!   breaks the API: a Gmail id is 16 hex characters and reads as a secret to the text ladder,
//!   and a follow-up call needs it exactly.
//! - **Identity** fields (a sender, a recipient) become one token each, through
//!   [`PseudonymizerSession::tokenize_whole`]. Part by part, `Alice <a@x.de>` would keep `Alice`.
//!   An organisation's domain stays beside the token, `<SENDER_k3x9qa> (dhl.de)`, because
//!   without it an agent cannot tell a parcel service from a bank. It is dropped for a freemail
//!   domain, a domain that contains a known name, and every sender on a c2 object: a medical
//!   practice's domain would say what the c2 mail is about.
//! - Every other string goes through the text ladder ([`PseudonymizerSession::tokenize_text`]).
//!
//! An object whose `data_class` is `c3` is removed and counted. Secret content never leaves
//! the machine in any form, pseudonymized or not (Q27, amended for c2 only on 2026-09-30).

use serde_json::Value;

use crate::registry::EntityRegistry;
use crate::session::PseudonymizerSession;
use crate::types::EntityType;

/// What the view did beyond tokenizing, for the receipt.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ViewReport {
    /// Objects removed because their `data_class` was `c3`.
    pub withheld_secret: usize,
}

const STRUCTURAL_KEYS: &[&str] = &[
    "id",
    "status",
    "stream",
    "state",
    "kind",
    "type",
    "category",
    "method",
    "version",
    "tier",
    "data_class",
    "date",
    "first_seen",
    "last_seen",
    "purge_after",
    "source",
    "unit",
    "currency",
    "lang",
    "language",
    // comms' relevance profile id. Exact, not a `_key` suffix: `api_key` must never pass.
    "profile_key",
];

const STRUCTURAL_SUFFIXES: &[&str] = &[
    "_id",
    "_ids",
    "_at",
    "_date",
    "_since",
    "_version",
    "_method",
    "_status",
    "_state",
    "_kind",
    "_type",
    "_class",
    "_bp",
    "_count",
    "_hours",
    "_days",
    "_minutes",
    "_seconds",
    "_revision",
    "_hash",
    "_digest",
];

const IDENTITY_KEYS: &[&str] = &[
    "from",
    "from_addr",
    "sender",
    "author",
    "to",
    "cc",
    "bcc",
    "reply_to",
    "recipient",
    "recipients",
    "organizer",
    "attendees",
];

fn is_structural(key: &str) -> bool {
    STRUCTURAL_KEYS.contains(&key) || STRUCTURAL_SUFFIXES.iter().any(|s| key.ends_with(s))
}

fn is_secret_object(value: &Value) -> bool {
    value.get("data_class").and_then(Value::as_str) == Some("c3")
}

/// Rewrites `value` in place into the agent view. The caller decides what to do when the
/// top-level value is itself a c3 object; see [`is_withheld`].
pub fn agent_view(
    value: &mut Value,
    session: &mut PseudonymizerSession,
    registry: &EntityRegistry,
) -> ViewReport {
    let mut report = ViewReport::default();
    walk(value, None, None, session, registry, &mut report);
    report
}

/// One identity field: a token, plus the sender's domain where the domain is an organisation's.
fn identity(
    value: &str,
    class: Option<&str>,
    session: &mut PseudonymizerSession,
    registry: &EntityRegistry,
) -> String {
    let token = session.tokenize_whole(value, EntityType::Identity);
    let domain = crate::pattern::sender_domain(value).filter(|d| {
        class != Some("c2")
            && !crate::pattern::is_freemail_domain(d)
            && registry.find_matches(d).is_empty()
    });
    match domain {
        Some(d) if token != value => {
            let rendered = format!("{token} ({d})");
            session.alias(&rendered, value.trim());
            rendered
        }
        _ => token,
    }
}

/// Whether the whole response is one c3 object and must not be sent at all.
pub fn is_withheld(value: &Value) -> bool {
    is_secret_object(value)
}

fn walk(
    value: &mut Value,
    key: Option<&str>,
    // The nearest enclosing `data_class`, so a field knows which class of row it sits in.
    class: Option<&str>,
    session: &mut PseudonymizerSession,
    registry: &EntityRegistry,
    report: &mut ViewReport,
) {
    match value {
        Value::String(s) => match key {
            Some(k) if is_structural(k) => {}
            Some(k) if IDENTITY_KEYS.contains(&k) => *s = identity(s, class, session, registry),
            _ => *s = session.tokenize_text(s, registry),
        },
        Value::Array(items) => {
            let before = items.len();
            items.retain(|item| !is_secret_object(item));
            report.withheld_secret += before - items.len();
            for item in items {
                // An array inherits its key: `"to": ["a@x.de"]` is still identities.
                walk(item, key, class, session, registry, report);
            }
        }
        Value::Object(map) => {
            let own = map
                .get("data_class")
                .and_then(Value::as_str)
                .map(str::to_string);
            let class = own.as_deref().or(class);
            for (k, v) in map.iter_mut() {
                if is_secret_object(v) {
                    *v = Value::Null;
                    report.withheld_secret += 1;
                    continue;
                }
                walk(v, Some(k.as_str()), class, session, registry, report);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn session() -> PseudonymizerSession {
        PseudonymizerSession::keyed(crate::keyed::session_key(b"test", "s"))
    }

    fn registry() -> EntityRegistry {
        EntityRegistry::builder().add_people(["Katrin"]).build()
    }

    #[test]
    fn ids_and_timestamps_survive_and_a_sender_is_one_token() {
        let mut value = json!([{
            "id": "18f3a9c2b7d41e05",
            "thread_id": "18f3a9c2b7d41e05",
            "internal_date": "2026-09-26T10:00:00Z",
            "data_class": "c1",
            "stream": "aktiv",
            "from_addr": "Katrin Wissem <katrin@example.com>",
            "subject": "616685 is your UNiDAYS passcode",
            "score_bp": 4200
        }]);
        let report = agent_view(&mut value, &mut session(), &registry());
        let row = &value[0];
        assert_eq!(row["id"], "18f3a9c2b7d41e05");
        assert_eq!(row["thread_id"], "18f3a9c2b7d41e05");
        assert_eq!(row["internal_date"], "2026-09-26T10:00:00Z");
        assert_eq!(row["stream"], "aktiv");
        assert_eq!(row["score_bp"], 4200);
        let from = row["from_addr"].as_str().unwrap();
        assert!(
            from.starts_with("<SENDER_") && !from.contains('@'),
            "{from}"
        );
        let subject = row["subject"].as_str().unwrap();
        assert!(!subject.contains("616685"), "{subject}");
        assert_eq!(report.withheld_secret, 0);
    }

    #[test]
    fn a_secret_row_is_removed_and_counted_wherever_it_sits() {
        let mut value = json!({
            "items": [
                {"data_class": "c3", "subject": "reset code"},
                {"data_class": "c2", "subject": "Katrin's receipt"}
            ],
            "pinned": {"data_class": "c3", "subject": "another code"}
        });
        let report = agent_view(&mut value, &mut session(), &registry());
        assert_eq!(report.withheld_secret, 2);
        assert_eq!(value["items"].as_array().unwrap().len(), 1);
        assert!(value["pinned"].is_null());
        // c2 stays, pseudonymized (ruling 2026-09-30).
        let subject = value["items"][0]["subject"].as_str().unwrap();
        assert!(!subject.contains("Katrin"), "{subject}");
    }

    #[test]
    fn a_key_named_like_a_secret_is_not_structural() {
        let mut value = json!({"api_key": "sk_live_4f9a8b7c6d5e4f3a2b1c", "profile_key": "p1"});
        agent_view(&mut value, &mut session(), &registry());
        assert!(
            !value["api_key"].as_str().unwrap().contains("sk_live"),
            "{value}"
        );
        assert_eq!(value["profile_key"], "p1");
    }

    #[test]
    fn a_top_level_secret_is_reported_as_withheld() {
        assert!(is_withheld(&json!({"data_class": "c3"})));
        assert!(!is_withheld(&json!([{"data_class": "c3"}])));
    }

    #[test]
    fn an_organisation_keeps_its_domain_and_a_person_does_not() {
        let mut value = json!([
            {"data_class": "c1", "from_addr": "DHL Paket <noreply@dhl.de>"},
            {"data_class": "c1", "from_addr": "Anna <anna.b@gmail.com>"},
            {"data_class": "c1", "from_addr": "x <hi@mail.gmx.net>"},
            {"data_class": "c1", "from_addr": "K <k@katrin-design.de>"},
            {"data_class": "c2", "from_addr": "Praxis <termine@praxis-am-see.de>"}
        ]);
        agent_view(&mut value, &mut session(), &registry());
        let from = |i: usize| value[i]["from_addr"].as_str().unwrap().to_string();
        assert!(
            from(0).starts_with("<SENDER_") && from(0).ends_with(" (dhl.de)"),
            "{}",
            from(0)
        );
        assert!(!from(0).contains("noreply"), "{}", from(0));
        let mut s = session();
        let mut again = json!({"from": "DHL Paket <noreply@dhl.de>"});
        agent_view(&mut again, &mut s, &registry());
        assert_eq!(
            s.rehydrate_text(again["from"].as_str().unwrap()),
            "DHL Paket <noreply@dhl.de>"
        );
        for i in 1..5 {
            assert!(
                !from(i).contains('(') && !from(i).contains('.'),
                "{}",
                from(i)
            );
        }
    }

    #[test]
    fn recipients_in_an_array_are_identities() {
        let mut value = json!({"to": ["Anna <anna@example.com>", "bob@example.com"]});
        agent_view(&mut value, &mut session(), &registry());
        for item in value["to"].as_array().unwrap() {
            assert!(item.as_str().unwrap().starts_with("<SENDER_"), "{item}");
        }
    }
}
