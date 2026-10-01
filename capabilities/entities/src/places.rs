//! A coordinate for a place name, from `capabilities/places`.
//!
//! Places owns geocoding: the provider, its rate limit and the permanent cache. So this asks
//! places over HTTP rather than calling the provider itself. Only the place text is sent,
//! never the entity's name (places README D3).
//!
//! Best effort by design. A fact is written without a coordinate when places is down or
//! finds nothing, and says so in the reply; a stored fact with no coordinate is still a
//! fact, and refusing it would lose what the operator typed.

use std::time::Duration;

use serde_json::{json, Value};

/// A resolved coordinate, or the reason there is none.
#[derive(Debug, Clone, PartialEq)]
pub enum Resolved {
    At {
        latitude: f64,
        longitude: f64,
        name: String,
    },
    NotFound,
    Unavailable(String),
}

pub fn resolve(places_url: &str, place: &str) -> Resolved {
    let client = match sjel_http::client(
        sjel_http::Purpose::new("entities-geocode"),
        Duration::from_secs(20),
    ) {
        Ok(client) => client,
        Err(error) => return Resolved::Unavailable(format!("client build: {error}")),
    };
    let url = format!("{}/api/geocode", places_url.trim_end_matches('/'));
    let request = sjel_server::InboundAuth::with_loopback_auth(
        client.post(&url).json(&json!({ "query": place })),
        &url,
    );
    let body: Value = match request
        .send()
        .and_then(|r| r.error_for_status())
        .and_then(|r| r.json())
    {
        Ok(body) => body,
        Err(error) => return Resolved::Unavailable(format!("places geocode: {error}")),
    };
    parse(&body)
}

/// The geocode reply as `capabilities/places/src/server.rs` writes it.
pub fn parse(body: &Value) -> Resolved {
    let place = &body["place"];
    match (place["latitude"].as_f64(), place["longitude"].as_f64()) {
        (Some(latitude), Some(longitude)) => Resolved::At {
            latitude,
            longitude,
            name: place["name"].as_str().unwrap_or_default().to_string(),
        },
        _ => Resolved::NotFound,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_found_place_carries_its_coordinate() {
        let body = json!({ "status": "ok", "place": { "name": "Bonn", "latitude": 50.73, "longitude": 7.1 } });
        assert_eq!(
            parse(&body),
            Resolved::At {
                latitude: 50.73,
                longitude: 7.1,
                name: "Bonn".into()
            }
        );
    }

    #[test]
    fn not_found_and_a_place_without_coordinates_are_not_found() {
        assert_eq!(
            parse(&json!({ "status": "not_found", "place": null })),
            Resolved::NotFound
        );
        assert_eq!(
            parse(&json!({ "place": { "name": "X" } })),
            Resolved::NotFound
        );
    }
}
