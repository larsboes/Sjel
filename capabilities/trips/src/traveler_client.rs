//! The one call trips makes to traveler, and why it degrades rather than fails.
//!
//! The default flight origin used to live in this capability's own overlay config
//! (`trips.json` → `travel.home_airport`). That made it the **second** home for a
//! fact `capabilities/traveler` now owns, and the practical consequence was worse
//! than the duplication: the airports the operator actually named were invisible
//! to every flight route, because nothing read them.
//!
//! One home, and it is the profile. `travel.pivots` stays where it is — the
//! profile has no pivot shape, and a pivot carries a `max_nights` that is a
//! property of the couch rather than of the traveller.
//!
//! Degrades to `None`, never to a default. An unreachable traveler means "no
//! origin configured", which the caller reports as a 400 naming both fixes —
//! exactly what it did when the value was missing from its own config. A default
//! airport invented here would send a search somewhere nobody asked for.

use std::time::Duration;

use serde::Deserialize;

/// Where traveler-server listens.
///
/// The fourth capability hardcoding a sibling's port (`punctuality.rs` in transit,
/// `finance_client.rs` and `calendar_base_url()` here are the others). The spine
/// mechanism those comments defer — service-runner exporting a declared sibling's
/// port the way it exports `SJEL_PORT` — is now well past justified by the count.
pub fn traveler_base_url() -> String {
    sjel_config::env_var("SJEL_TRAVELER_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:8096".to_string())
}

/// Long enough for a loopback query, short enough that a stopped traveler never
/// makes a flight search feel broken.
const TIMEOUT: Duration = Duration::from_secs(3);

use crate::plan_search::SoftWeights;

#[derive(Debug, Deserialize)]
struct ProfileEnvelope {
    profile: ProfileBody,
}

#[derive(Debug, Deserialize)]
struct ProfileBody {
    hard: HardBody,
    #[serde(default)]
    soft: Option<SoftWeights>,
}

#[derive(Debug, Deserialize)]
struct HardBody {
    #[serde(default)]
    home_airports: Vec<String>,
}

/// The airports the traveller named, best first. `None` when the profile cannot be
/// read at all — a different fact from an empty list, which means they named none.
pub fn home_airports() -> Option<Vec<String>> {
    let client = sjel_http::client(sjel_http::Purpose::new("trips-traveler"), TIMEOUT).ok()?;
    let url = format!("{}/api/profile", traveler_base_url());
    let envelope: ProfileEnvelope =
        sjel_server::InboundAuth::with_loopback_auth(client.get(&url), &url)
            .send()
            .and_then(|response| response.error_for_status())
            .and_then(|response| response.json())
            .ok()?;
    Some(envelope.profile.hard.home_airports)
}

/// The soft ranking weights stated or derived in traveler profile, if available.
pub fn soft_weights() -> Option<SoftWeights> {
    let client = sjel_http::client(sjel_http::Purpose::new("trips-traveler"), TIMEOUT).ok()?;
    let url = format!("{}/api/profile", traveler_base_url());
    let envelope: ProfileEnvelope =
        sjel_server::InboundAuth::with_loopback_auth(client.get(&url), &url)
            .send()
            .and_then(|response| response.error_for_status())
            .and_then(|response| response.json())
            .ok()?;
    envelope.profile.soft
}

/// The default flight origin: the first airport the traveller named.
pub fn first_home_airport() -> Option<String> {
    home_airports()?.into_iter().next()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_body_shape_is_what_the_profile_serves() {
        // The envelope and the nested keys, pinned so a change on traveler's
        // side fails here rather than as an absent origin nobody can explain.
        let body = r#"{"profile":{"hard":{"home_airports":["AAA","BBB"]},"soft":{"budget_fit":0.3,"feasibility":0.25,"season":0.15,"events":0.2,"retrospective":0.1}},"stored":true}"#;
        let envelope: ProfileEnvelope = serde_json::from_str(body).unwrap();
        assert_eq!(envelope.profile.hard.home_airports, vec!["AAA", "BBB"]);
        assert_eq!(
            envelope.profile.soft,
            Some(SoftWeights {
                budget_fit: 0.3,
                feasibility: 0.25,
                season: 0.15,
                events: 0.2,
                retrospective: 0.1,
            })
        );
    }

    #[test]
    fn a_profile_with_no_airports_parses_to_an_empty_list() {
        // `#[serde(default)]` on the field: an unstated profile serves the key
        // absent rather than null, and a strict parse would turn "nothing named"
        // into "traveler is broken".
        let body = r#"{"profile":{"hard":{}}}"#;
        let envelope: ProfileEnvelope = serde_json::from_str(body).unwrap();
        assert!(envelope.profile.hard.home_airports.is_empty());
        assert!(envelope.profile.soft.is_none());
    }
}
