//! Typed ids, and the answer one capability gives about rows of its own that reference another's.
//!
//! The contract is in `libs/links/ISA.md`. In short: an id is `<kind>:<rest>`; a capability that
//! holds references declares the kinds in `service.toml` as `links_to`, and answers
//! `GET /api/links?to=<id>` with a [`Links`] body. The shell asks only the capabilities that
//! declare the id's kind, so it holds no code per pair of capabilities.
//!
//! No axum here on purpose (ISA D4): each capability writes its own handler around [`target`].

use serde::Serialize;

/// A typed id and its kind. The kind is every segment but the last: `trip:plan:18c7` is kind
/// `trip:plan`, which is the string trips' rows carry and the one `links_to` declares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypedId<'a> {
    pub kind: &'a str,
    pub id: &'a str,
}

impl<'a> TypedId<'a> {
    /// The kind is everything before the last `:`, so `trip:plan:18c7` is kind `trip:plan` and
    /// `ent:18d8` is kind `ent`. Refused: no `:`, an empty kind, an empty last segment.
    pub fn parse(id: &'a str) -> Result<Self, String> {
        let Some((kind, rest)) = id.rsplit_once(':') else {
            return Err(format!(
                "`{id}` is not a typed id: it has no `<kind>:` prefix"
            ));
        };
        if kind.is_empty() || kind.split(':').any(str::is_empty) {
            return Err(format!("`{id}` has an empty kind"));
        }
        if rest.is_empty() {
            return Err(format!("`{id}` has nothing after its kind"));
        }
        Ok(Self { kind, id })
    }
}

/// Checks the `to` query parameter. An absent or untyped `to` is a 400 with this reason, never
/// an empty list: an empty list would read as "nothing references it" (ISA LNK-3).
pub fn target(to: Option<&str>) -> Result<TypedId<'_>, String> {
    match to {
        None | Some("") => Err("`to` is required: the typed id to find references to".into()),
        Some(to) => TypedId::parse(to),
    }
}

/// One row that references the target. Rendered the same for every kind (ISA D5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Link {
    /// The row's own typed id, so the shell can follow it.
    pub id: String,
    /// The row's kind, the same string its id starts with.
    pub kind: String,
    pub title: String,
    /// A civil date or an RFC 3339 instant: when the row happened.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
    /// One short line the shell prints beside the title, such as an amount.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<String>,
    /// The field that holds the reference: `trip_id`, `payload.plan_id`.
    pub via: &'static str,
}

/// The body of `GET /api/links`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Links {
    pub to: String,
    pub links: Vec<Link>,
    /// Rows that reference the target but have no linkable id, so a short list is never a silent
    /// one (ISA D3).
    pub unlinkable: usize,
}

/// `<kind>:<16 hex>` from the first 8 bytes of SHA-256 over `identity`. Deterministic, so the
/// same identity always gives the same id. Places used this with a `_` separator until
/// 2026-10-06; the hex is unchanged, which is what makes its migration a rename (ISA LNK-11).
pub fn stable_id(kind: &str, identity: &str) -> String {
    use sha2::{Digest, Sha256};
    use std::fmt::Write;
    let digest = Sha256::digest(identity.as_bytes());
    let mut id = String::with_capacity(kind.len() + 17);
    id.push_str(kind);
    id.push(':');
    for byte in &digest[..8] {
        write!(&mut id, "{byte:02x}").expect("writing to a String cannot fail");
    }
    id
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_is_everything_before_the_last_segment() {
        assert_eq!(TypedId::parse("trip:plan:18c7").unwrap().kind, "trip:plan");
        assert_eq!(TypedId::parse("ent:18d8").unwrap().kind, "ent");
        assert_eq!(TypedId::parse("fin:tx:ab12").unwrap().kind, "fin:tx");
    }

    #[test]
    fn untyped_ids_are_refused() {
        for bad in ["place_0bd5", ":x", "a::b", "trip:", "transaction_16_1_eur"] {
            assert!(TypedId::parse(bad).is_err(), "{bad} parsed");
        }
    }

    #[test]
    fn a_missing_target_is_an_error_not_an_empty_answer() {
        assert!(target(None).is_err());
        assert!(target(Some("")).is_err());
        assert!(target(Some("trip:plan:1")).is_ok());
    }

    #[test]
    fn link_omits_absent_fields() {
        let link = Link {
            id: "cal:entry:1".into(),
            kind: "cal:entry".into(),
            title: "Bonn → Stuttgart".into(),
            at: None,
            meta: None,
            via: "payload.plan_id",
        };
        let json = serde_json::to_value(&link).unwrap();
        assert!(json.get("at").is_none() && json.get("meta").is_none());
        assert_eq!(json["via"], "payload.plan_id");
    }

    #[test]
    fn stable_id_keeps_the_hex_places_minted() {
        // `shasum -a 256` over "eva:8000207", first 16 hex: what places minted as `place_…`.
        assert_eq!(stable_id("place", "eva:8000207"), "place:32b1af1b5a9b8869");
        assert_ne!(
            stable_id("place", "eva:8000207"),
            stable_id("place", "eva:1234567")
        );
    }
}
