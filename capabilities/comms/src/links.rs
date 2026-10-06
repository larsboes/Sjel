//! What comms answers on `GET /api/links` (libs/links/ISA.md D10, LNK-17).
//!
//! Comms stores no person ids. A person's mail is the triage rows whose sender address is one of
//! that person's emails, as entities holds them. The match is exact on the address, so mail never
//! goes on the review list: an address belongs to the person or it does not.

use std::collections::HashSet;
use std::time::Duration;

use sjel_links::{Link, Links};

use crate::store::TriageItem;

/// The id kinds comms answers for. Declared again as `links_to` in `service.toml`; a test holds
/// the two equal.
pub const LINKS_TO: &[&str] = &["ent"];

/// The address in a `From` value, lower case: `Lucia <lucia@example.org>` and `lucia@example.org`
/// both give `lucia@example.org`.
pub fn address_of(from: &str) -> String {
    let inner = match (from.rfind('<'), from.rfind('>')) {
        (Some(open), Some(close)) if open < close => &from[open + 1..close],
        _ => from,
    };
    inner.trim().to_lowercase()
}

/// The triage rows sent from one of `emails`, newest first as the store lists them.
pub fn mail_from(entity_id: &str, items: &[TriageItem], emails: &HashSet<String>) -> Links {
    let links = items
        .iter()
        .filter(|item| {
            item.from_addr
                .as_deref()
                .is_some_and(|from| emails.contains(&address_of(from)))
        })
        .map(|item| Link {
            id: format!("mail:{}", item.id),
            kind: "mail".into(),
            title: item
                .subject
                .clone()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| "(no subject)".into()),
            at: item.internal_date_text.clone(),
            meta: item.from_addr.as_deref().map(address_of),
            via: "from_addr",
        })
        .collect();
    Links {
        to: entity_id.to_owned(),
        links,
        unlinkable: 0,
    }
}

/// Where entities-server listens. The same sibling-port convention trips and places use.
fn entities_base_url() -> String {
    sjel_config::env_var("SJEL_ENTITIES_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:8097".to_string())
}

const TIMEOUT: Duration = Duration::from_secs(3);

/// The person's emails, lower case, as entities holds them. The error is a sentence a reader can
/// act on; an unknown person is an error, not an empty set.
pub fn emails_of(entity_id: &str) -> Result<HashSet<String>, String> {
    let url = format!(
        "{}/api/entities/{}",
        entities_base_url(),
        entity_id.replace('/', "%2F")
    );
    let client = sjel_http::client(sjel_http::Purpose::new("comms-entities"), TIMEOUT)
        .map_err(|error| format!("entities client: {error}"))?;
    let response = sjel_server::InboundAuth::with_loopback_auth(client.get(&url), &url)
        .send()
        .map_err(|error| format!("entities is not reachable: {}", error.without_url()))?;
    if !response.status().is_success() {
        return Err(format!(
            "entities answered {} for that person",
            response.status()
        ));
    }
    let entity: serde_json::Value = response
        .json()
        .map_err(|_| "entities answered an unexpected shape for that person".to_string())?;
    Ok(entity["values"]["emails"]["value"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str())
        .map(|e| e.trim().to_lowercase())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, from: Option<&str>) -> TriageItem {
        TriageItem {
            from_addr: from.map(Into::into),
            ..crate::store::db_tests::mk_triage(id, "personal")
        }
    }

    #[test]
    fn the_address_is_read_from_either_form() {
        assert_eq!(
            address_of("Lucia García <Lucia@Example.org>"),
            "lucia@example.org"
        );
        assert_eq!(address_of(" lucia@example.org "), "lucia@example.org");
    }

    #[test]
    fn only_mail_from_the_persons_addresses_is_linked() {
        let items = [
            item("1", Some("Lucia <lucia@example.org>")),
            item("2", Some("news@shop.example")),
            item("3", None),
            item("4", Some("LUCIA@example.org")),
        ];
        let emails = HashSet::from(["lucia@example.org".to_string()]);
        let ids: Vec<_> = mail_from("ent:1", &items, &emails)
            .links
            .into_iter()
            .map(|l| l.id)
            .collect();
        assert_eq!(ids, ["mail:1", "mail:4"]);
        assert!(mail_from("ent:1", &items, &HashSet::new()).links.is_empty());
    }

    #[test]
    fn links_to_matches_the_manifest() {
        let manifest = include_str!("../service.toml");
        let line = manifest
            .lines()
            .find(|line| line.starts_with("links_to"))
            .expect("service.toml declares links_to");
        for kind in LINKS_TO {
            assert!(
                line.contains(&format!("\"{kind}\"")),
                "{kind} missing from {line}"
            );
        }
    }
}
