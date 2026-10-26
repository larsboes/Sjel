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
//! - Every other string goes through the text ladder ([`PseudonymizerSession::tokenize_text`]),
//!   unless the capability declared its key public for that response ([`agent_view_public`]).
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
    // A task's own schedule (`capabilities/projects`). The text ladder reads `2026-10-23` as a
    // phone number; free-text dates stay redacted, because there a date can be a birth date.
    "due",
    "scheduled",
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

const PATH_KEYS: &[&str] = &["path", "files", "uri"];

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
    agent_view_public(value, session, registry, &[])
}

/// [`agent_view`], with `public` keys whose strings the capability declared public for this one
/// response (a station name in a station lookup), so they stay verbatim. A capability declares
/// it per response, because the same key can be public in one answer and personal in another: a
/// station searched for is timetable data, a station in the operator's trips is their travel.
/// An identity key and every field of a c2 object stay pseudonymized whatever is declared, and a
/// c3 object is still removed.
pub fn agent_view_public(
    value: &mut Value,
    session: &mut PseudonymizerSession,
    registry: &EntityRegistry,
    public: &[&str],
) -> ViewReport {
    let mut report = ViewReport::default();
    walk(value, None, None, public, session, registry, &mut report);
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

/// Keys that place something exactly. The trusted view drops them, so a name never travels
/// with a coordinate or a street address.
const LOCATION_KEYS: &[&str] = &[
    "latitude",
    "longitude",
    "lat",
    "lng",
    "lon",
    "coordinates",
    "address",
    "street",
    "postal_code",
    "zip",
];

/// The view for an owner-trusted or verified zero-retention model (`Tier::Zdr`,
/// `Tier::Trusted` in `sjel-server`). Names stay readable. A c3 object is still removed, a
/// location key is dropped, and an email address, phone number, IBAN or token-shaped value
/// in any string is replaced. A sender keeps its display name, plus its domain when that is an
/// organisation's.
pub fn trusted_view(value: &mut Value) -> ViewReport {
    let mut report = ViewReport::default();
    trusted_walk(value, None, &mut report);
    report
}

fn trusted_walk(value: &mut Value, key: Option<&str>, report: &mut ViewReport) {
    match value {
        Value::String(s) => match key {
            Some(k) if is_structural(k) => {}
            Some(k) if IDENTITY_KEYS.contains(&k) => *s = trusted_identity(s),
            _ => *s = trusted_text(s),
        },
        Value::Array(items) => {
            let before = items.len();
            items.retain(|item| !is_secret_object(item));
            report.withheld_secret += before - items.len();
            for item in items {
                trusted_walk(item, key, report);
            }
        }
        Value::Object(map) => {
            map.retain(|k, _| !LOCATION_KEYS.contains(&k.as_str()));
            for (k, v) in map.iter_mut() {
                if is_secret_object(v) {
                    *v = Value::Null;
                    report.withheld_secret += 1;
                    continue;
                }
                trusted_walk(v, Some(k.as_str()), report);
            }
        }
        _ => {}
    }
}

fn trusted_identity(value: &str) -> String {
    let name = value
        .split('<')
        .next()
        .unwrap_or_default()
        .trim()
        .trim_matches('"');
    let domain =
        crate::pattern::sender_domain(value).filter(|d| !crate::pattern::is_freemail_domain(d));
    match (
        name.is_empty() || crate::pattern::looks_like_email(name),
        domain,
    ) {
        (false, Some(d)) => format!("{name} ({d})"),
        (false, None) => name.to_string(),
        (true, Some(d)) => format!("[email] ({d})"),
        (true, None) => "[email]".to_string(),
    }
}

fn trusted_text(value: &str) -> String {
    use crate::pattern::{looks_like_email, looks_like_iban, looks_like_phone, looks_like_token};
    // `2026-10-25` is a date, not a number in groups: dates are what a trusted agent plans with.
    let iso_date = |w: &str| {
        let b = w.as_bytes();
        b.len() == 10
            && b[4] == b'-'
            && b[7] == b'-'
            && b.iter()
                .enumerate()
                .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
    };
    let digitish = |w: &str| {
        !w.is_empty()
            && !iso_date(w)
            && w.chars().any(|c| c.is_ascii_digit())
            && w.chars().all(|c| c.is_ascii_digit() || "+()/-".contains(c))
    };
    let pieces: Vec<&str> = value.split_inclusive(char::is_whitespace).collect();
    let mut out = String::with_capacity(value.len());
    let mut i = 0;
    while i < pieces.len() {
        let word = pieces[i].trim_end();
        // A number written in groups ("+49 170 1234567") is judged as one run.
        if digitish(word) {
            let mut end = i;
            while end + 1 < pieces.len() && digitish(pieces[end + 1].trim_end()) {
                end += 1;
            }
            let run: String = pieces[i..=end].concat();
            let digits = run.chars().filter(char::is_ascii_digit).count();
            let phone = digits >= 7 && (run.trim_start().starts_with('+') || end > i);
            if phone || looks_like_phone(run.trim_end()) {
                let last = pieces[end];
                out.push_str("[phone]");
                out.push_str(&last[last.trim_end().len()..]);
            } else {
                out.push_str(&run);
            }
            i = end + 1;
            continue;
        }
        let gap = &pieces[i][word.len()..];
        let replaced = if looks_like_email(word) {
            Some("[email]")
        } else if looks_like_iban(word) {
            Some("[account]")
        } else if looks_like_token(word) {
            Some("[token]")
        } else {
            None
        };
        match replaced {
            Some(r) => {
                out.push_str(r);
                out.push_str(gap);
            }
            None => out.push_str(pieces[i]),
        }
        i += 1;
    }
    out
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
    public: &[&str],
    session: &mut PseudonymizerSession,
    registry: &EntityRegistry,
    report: &mut ViewReport,
) {
    match value {
        Value::String(s) => match key {
            Some(k) if is_structural(k) => {}
            Some(k)
                if public.contains(&k) && !IDENTITY_KEYS.contains(&k) && class != Some("c2") => {}
            Some(k) if IDENTITY_KEYS.contains(&k) => *s = identity(s, class, session, registry),
            Some(k) if PATH_KEYS.contains(&k) => *s = session.tokenize_path(s, registry),
            _ => *s = session.tokenize_text(s, registry),
        },
        Value::Array(items) => {
            let before = items.len();
            items.retain(|item| !is_secret_object(item));
            report.withheld_secret += before - items.len();
            for item in items {
                // An array inherits its key: `"to": ["a@x.de"]` is still identities.
                walk(item, key, class, public, session, registry, report);
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
                walk(
                    v,
                    Some(k.as_str()),
                    class,
                    public,
                    session,
                    registry,
                    report,
                );
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

    #[test]
    fn the_trusted_view_keeps_names_and_drops_contact_and_location() {
        let mut value = json!({
            "data_class": "c2",
            "name": "Erika Beispiel",
            "from_addr": "Erika <erika@gmail.com>",
            "note": "Call +49 170 1234567 or erika@gmail.com, IBAN DE89370400440532013000",
            "latitude": 50.7, "longitude": 7.1, "place": "Bonn",
            "secret": {"data_class": "c3", "code": "123456"},
            "invoice": {"total": "179,80 EUR", "date": "30.10.2026"}
        });
        let report = trusted_view(&mut value);
        assert_eq!(value["name"], "Erika Beispiel");
        assert_eq!(value["from_addr"], "Erika");
        assert_eq!(value["place"], "Bonn");
        assert!(value.get("latitude").is_none() && value.get("longitude").is_none());
        let note = value["note"].as_str().unwrap();
        assert!(!note.contains("erika@") && !note.contains("DE89"), "{note}");
        assert!(
            !note.contains("1234567") && note.contains("[phone]"),
            "{note}"
        );
        assert_eq!(
            trusted_text("Hinfahrt 30.10.2026 um 08:12"),
            "Hinfahrt 30.10.2026 um 08:12"
        );
        assert_eq!(trusted_text("2026-10-25"), "2026-10-25");
        assert_eq!(trusted_text("Ruf 0228-123-4567 an"), "Ruf [phone] an");
        assert_eq!(value["secret"], Value::Null);
        assert_eq!(report.withheld_secret, 1);
        assert_eq!(value["invoice"]["total"], "179,80 EUR");
        assert_eq!(
            trusted_identity("DHL Paket <noreply@dhl.de>"),
            "DHL Paket (dhl.de)"
        );
    }

    fn registry() -> EntityRegistry {
        EntityRegistry::builder().add_people(["Erika"]).build()
    }

    #[test]
    fn a_declared_public_key_stays_and_nothing_else_does() {
        let mut value = json!({
            "name": "Erika Hbf",
            "summary": "Erika",
            "sender": "Erika <k@x.de>",
            "secret": {"data_class": "c3", "name": "Erika"},
            "letter": {"data_class": "c2", "name": "Erika"},
        });
        let report =
            agent_view_public(&mut value, &mut session(), &registry(), &["name", "sender"]);
        assert_eq!(value["name"], "Erika Hbf", "declared public");
        assert_ne!(value["summary"], "Erika", "not declared");
        assert!(
            !value["sender"].as_str().unwrap().contains("Erika"),
            "identity never public"
        );
        assert_eq!(value["secret"], Value::Null, "c3 still withheld");
        assert_ne!(value["letter"]["name"], "Erika", "c2 stays pseudonymized");
        assert_eq!(report.withheld_secret, 1);
    }

    #[test]
    fn ids_and_timestamps_survive_and_a_sender_is_one_token() {
        let mut value = json!([{
            "id": "18f3a9c2b7d41e05",
            "thread_id": "18f3a9c2b7d41e05",
            "internal_date": "2026-09-26T10:00:00Z",
            "data_class": "c1",
            "stream": "aktiv",
            "from_addr": "Erika Mustermann <erika@example.com>",
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
                {"data_class": "c2", "subject": "Erika's receipt"}
            ],
            "pinned": {"data_class": "c3", "subject": "another code"}
        });
        let report = agent_view(&mut value, &mut session(), &registry());
        assert_eq!(report.withheld_secret, 2);
        assert_eq!(value["items"].as_array().unwrap().len(), 1);
        assert!(value["pinned"].is_null());
        // c2 stays, pseudonymized (ruling 2026-09-30).
        let subject = value["items"][0]["subject"].as_str().unwrap();
        assert!(!subject.contains("Erika"), "{subject}");
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
    fn a_task_schedule_survives_and_a_date_in_text_does_not() {
        let mut value = json!({
            "due": "2026-10-23",
            "fields": {"scheduled": "2026-10-20"},
            "summary": "geb. 1990-02-01"
        });
        agent_view(&mut value, &mut session(), &registry());
        assert_eq!(value["due"], "2026-10-23");
        assert_eq!(value["fields"]["scheduled"], "2026-10-20");
        assert!(
            !value["summary"].as_str().unwrap().contains("1990"),
            "{value}"
        );
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
            {"data_class": "c1", "from_addr": "K <k@erika-design.de>"},
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

    #[test]
    fn attachment_paths_without_identities_survive_and_named_paths_redact_the_name() {
        let mut value = json!({
            "files": [
                "Waschmaschine/Assets/IMG_1234.jpeg",
                "Steuer-2025/Belege/2025-11-04-rechnung-1234567.pdf",
                "Reisen/Erika/IMG_1234.jpeg"
            ],
            "path": "Waschmaschine/Assets/IMG_1234.jpeg"
        });
        let mut s = session();
        agent_view(&mut value, &mut s, &registry());
        let files = value["files"].as_array().unwrap();
        assert_eq!(files[0], "Waschmaschine/Assets/IMG_1234.jpeg");
        assert_eq!(
            files[1],
            "Steuer-2025/Belege/2025-11-04-rechnung-1234567.pdf"
        );
        let named = files[2].as_str().unwrap();
        assert!(!named.contains("Erika"), "{named}");
        assert!(named.ends_with("/IMG_1234.jpeg"), "{named}");
        assert_eq!(s.rehydrate_text(named), "Reisen/Erika/IMG_1234.jpeg");
        assert_eq!(value["path"], "Waschmaschine/Assets/IMG_1234.jpeg");
    }
}
