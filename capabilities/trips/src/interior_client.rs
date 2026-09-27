//! The one call trips makes to interior: `POST /api/items`, once per gear proposal.
//!
//! Written the way `finance_client` is written, and for the same reason — the failure has a
//! type carrying a reason, and nothing here can turn a refused write into a silent success.
//!
//! **Why HTTP and not SQL.** Both capabilities resolve to the same database file, so a direct
//! `INSERT` would work and would be wrong: Q73's write-path rule says a capability's rows are
//! written by its own handler. Interior's handler is also what makes this idempotent for free
//! — it answers 409 for an id that already exists, so a second `--apply` over the same notes
//! changes nothing and says so per row.

use std::time::Duration;

use serde::Serialize;

/// Where interior listens. `capabilities/interior/service.toml` declares 8092. The fourth
/// capability to hardcode a sibling's port, which is the same argument `finance_client`
/// already records for the spine mechanism that would end it.
pub fn interior_base_url() -> String {
    axon_config::env_var("SJEL_INTERIOR_URL").unwrap_or_else(|_| "http://127.0.0.1:8092".to_string())
}

/// Long enough for a loopback write, short enough that a stopped interior fails the import
/// rather than hanging a terminal.
const TIMEOUT: Duration = Duration::from_secs(5);

/// What one proposal became.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Written {
    /// Interior created the row.
    Created,
    /// The id was already there. Not an error: the import is meant to be re-runnable, and
    /// this is the answer that makes it so.
    AlreadyThere,
    /// Interior answered, and refused. The message is its own.
    Refused(String),
    /// Interior did not answer at all.
    Unreachable(String),
}

/// The request body, ONE FLAT OBJECT.
///
/// Interior reads it into `NewItem { #[serde(flatten)] item: Item, state, note }`, so
/// `state` and `note` are siblings of the item's own keys and not a wrapper around them. A
/// body that nested the item under a key of its own would fail to deserialize and interior
/// would answer 422 for every proposal — a run that writes nothing while each line reports a
/// refusal it cannot explain.
///
/// Separate from the request so that shape is testable without a server. Nothing else here
/// can be: `create_item`'s remaining work is one HTTP call.
fn item_body<T: Serialize>(item: &T, state: &str, note: &str) -> Result<serde_json::Value, String> {
    match serde_json::to_value(item) {
        Ok(serde_json::Value::Object(mut map)) => {
            map.insert("state".into(), serde_json::Value::String(state.to_string()));
            map.insert("note".into(), serde_json::Value::String(note.to_string()));
            Ok(serde_json::Value::Object(map))
        }
        // A proposal that does not serialise to an object is a programming error here, not a
        // condition of the world, so it is reported and never guessed past.
        Ok(other) => Err(format!("proposal is not an object: {other}")),
        Err(error) => Err(error.to_string()),
    }
}

/// `POST /api/items` for one gear proposal.
///
/// `state` is `wanted` for gear the operator does not own yet and `owned` otherwise; the
/// caller decides, because the note knows and this file does not. Interior requires the
/// field — a row with no state joins to nothing and appears in no list.
pub fn create_item<T: Serialize>(item: &T, state: &str, note: &str) -> Written {
    let body = match item_body(item, state, note) {
        Ok(body) => body,
        Err(reason) => return Written::Refused(reason),
    };

    let client = match axon_http::client(axon_http::Purpose::new("trips-interior"), TIMEOUT) {
        Ok(client) => client,
        Err(error) => return Written::Unreachable(format!("interior client: {error}")),
    };
    let mut request = client
        .post(format!("{}/api/items", interior_base_url()))
        .json(&body);
    // The inbound gate covers every route but /health and /ready. Without the token a gated
    // interior reads as "not running", which would turn a refused write into a false
    // unreachable — the same rule `finance_client` states, resolved per request so a rotated
    // token needs no restart.
    if let Some(bearer) = axon_server::InboundAuth::from_deployment().bearer_header() {
        request = request.header(reqwest::header::AUTHORIZATION, bearer);
    }
    let response = match request.send() {
        Ok(response) => response,
        // `without_url` for the same reason finance_client gives: the URL is already known
        // to the reader and reqwest prints it whole inside the message.
        Err(error) => return Written::Unreachable(error.without_url().to_string()),
    };

    match response.status().as_u16() {
        201 => Written::Created,
        409 => Written::AlreadyThere,
        status => Written::Refused(format!(
            "{status}: {}",
            response
                .text()
                .unwrap_or_else(|error| format!("(body unreadable: {error})"))
                .trim()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gear::{proposal_from, REQUIRED_INTERIOR_COLUMNS};
    use std::collections::BTreeMap;

    fn zelt() -> crate::gear::GearProposal {
        proposal_from(
            "Zelt",
            &BTreeMap::from([
                ("category".to_string(), "schlafen".to_string()),
                ("weight_g".to_string(), "1850".to_string()),
                ("packable".to_string(), "true".to_string()),
                ("waterproof".to_string(), "true".to_string()),
                ("quick_dry".to_string(), "false".to_string()),
                ("pack_location".to_string(), "rucksack".to_string()),
                ("trip_types".to_string(), "[hiking]".to_string()),
            ]),
        )
        .expect("a note with all seven fields is a proposal")
    }

    /// The seam trips owns, and the only part of `--apply` that can be checked without
    /// interior answering.
    ///
    /// `gear.rs` already pins the ten keys the payload carries. What that test cannot see is
    /// what this file puts AROUND them: interior flattens the item, so `state` and `note`
    /// have to arrive as siblings of `weight_g`, not as a wrapper. Nest them and every POST
    /// is a 422 — the import run that writes nothing while reporting a refusal per line.
    #[test]
    fn the_body_is_flat_and_carries_the_item_beside_its_state() {
        let proposal = zelt();
        let body = item_body(&proposal.item_payload(), &proposal.state, "imported")
            .expect("a payload object makes a body");
        let map = body.as_object().expect("the body is one object");

        assert_eq!(body["id"], "gear:zelt");
        assert_eq!(body["kind"], "gear");
        assert_eq!(body["label"], "Zelt");
        assert_eq!(body["state"], "owned", "interior requires a state");
        assert_eq!(body["note"], "imported");
        for column in REQUIRED_INTERIOR_COLUMNS {
            assert!(
                map.contains_key(*column),
                "{column} is not a top-level key of the body"
            );
        }
        assert_eq!(
            map.len(),
            REQUIRED_INTERIOR_COLUMNS.len() + 5,
            "the body carries id, kind, label, state, note and the seven columns — nothing \
             nested and nothing else: {map:?}"
        );
    }

    /// The refusal branch, which exists so a shape error is reported rather than posted.
    #[test]
    fn a_payload_that_is_not_an_object_is_refused_before_the_request() {
        let error =
            item_body(&"just a string", "owned", "imported").expect_err("a string is not an item");
        assert!(
            error.contains("not an object"),
            "the reason names the shape: {error}"
        );
    }
}
