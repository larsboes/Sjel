//! Inspectable routing for physical and online Scouting events.
//!
//! Interest score answers whether an event matters. This module answers where
//! it belongs. It is pure: no opportunity is dropped or promoted here, and no
//! Trips or Calendar service is called.

use serde::Serialize;
use serde_json::Value;

use crate::config::GeoPolicy;
use crate::opportunity::{Opportunity, OpportunityType};
use crate::store::RankedRow;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EventRouteKind {
    Local,
    TravelCandidate,
    Online,
    Unresolved,
}

impl EventRouteKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::TravelCandidate => "travel_candidate",
            Self::Online => "online",
            Self::Unresolved => "unresolved",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EventRouteBasis {
    SourceMetadata,
    LocationText,
    Coordinates,
    Country,
    Timezone,
    OperatorOverride,
    MissingPolicy,
    MissingEvidence,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EventRoute {
    pub route: EventRouteKind,
    pub basis: EventRouteBasis,
    pub reason: String,
    pub distance_km: Option<f64>,
}

struct Evidence<'a> {
    location: Option<&'a str>,
    city: Option<&'a str>,
    country: Option<&'a str>,
    latitude: Option<f64>,
    longitude: Option<f64>,
    raw: Option<&'a Value>,
}

pub fn classify_opportunity(
    opportunity: &Opportunity,
    policy: Option<&GeoPolicy>,
) -> Option<EventRoute> {
    (opportunity.opportunity_type == OpportunityType::Event).then(|| {
        classify(
            Evidence {
                location: opportunity.location.as_deref().and_then(non_empty),
                city: opportunity.city.as_deref().and_then(non_empty),
                country: opportunity.country_code.as_deref().and_then(non_empty),
                latitude: opportunity.latitude,
                longitude: opportunity.longitude,
                raw: Some(&opportunity.raw),
            },
            policy,
        )
    })
}

pub fn classify_ranked(row: &RankedRow, policy: Option<&GeoPolicy>) -> Option<EventRoute> {
    if row.opportunity_type != "event" {
        return None;
    }
    let raw = row
        .raw
        .as_deref()
        .and_then(|value| serde_json::from_str::<Value>(value).ok());
    Some(classify(
        Evidence {
            location: non_empty(&row.location),
            city: non_empty(&row.city),
            country: row.country_code.as_deref().and_then(non_empty),
            latitude: row.latitude,
            longitude: row.longitude,
            raw: raw.as_ref(),
        },
        policy,
    ))
}

fn classify(evidence: Evidence<'_>, policy: Option<&GeoPolicy>) -> EventRoute {
    if let Some(location_type) = evidence
        .raw
        .and_then(|raw| raw.get("location_type"))
        .and_then(Value::as_str)
        .filter(|value| is_online_text(value))
    {
        return route(
            EventRouteKind::Online,
            EventRouteBasis::SourceMetadata,
            format!("source metadata says location_type={location_type}"),
            None,
        );
    }
    if let Some(text) = [evidence.location, evidence.city]
        .into_iter()
        .flatten()
        .find(|value| is_online_text(value))
    {
        return route(
            EventRouteKind::Online,
            EventRouteBasis::LocationText,
            format!("location text identifies an online event: {text}"),
            None,
        );
    }

    let Some(policy) = policy else {
        return route(
            EventRouteKind::Unresolved,
            EventRouteBasis::MissingPolicy,
            "no geo policy is configured",
            None,
        );
    };

    if let (Some(home_lat), Some(home_lng), Some(radius), Some(lat), Some(lng)) = (
        policy.home_latitude,
        policy.home_longitude,
        policy.local_radius_km,
        evidence.latitude,
        evidence.longitude,
    ) {
        if valid_coordinate(home_lat, home_lng)
            && valid_coordinate(lat, lng)
            && radius.is_finite()
            && radius > 0.0
        {
            let distance = haversine_km(home_lat, home_lng, lat, lng);
            let kind = if distance <= radius {
                EventRouteKind::Local
            } else {
                EventRouteKind::TravelCandidate
            };
            return route(
                kind,
                EventRouteBasis::Coordinates,
                format!("{distance:.1} km from configured home; local radius is {radius:.1} km"),
                Some(round_one(distance)),
            );
        }
    }

    if let Some(country) = evidence.country {
        if let Some(local) = policy.country_is_local(country) {
            return route(
                if local {
                    EventRouteKind::Local
                } else {
                    EventRouteKind::TravelCandidate
                },
                EventRouteBasis::Country,
                if local {
                    format!("country {country} matches a configured local token")
                } else {
                    format!("country {country} does not match any configured local token")
                },
                None,
            );
        }
    }

    if let Some(timezone) = evidence
        .raw
        .and_then(|raw| raw.get("timezone"))
        .and_then(Value::as_str)
        .and_then(non_empty)
    {
        if let Some(local) = policy.timezone_is_local(timezone) {
            return route(
                if local {
                    EventRouteKind::Local
                } else {
                    EventRouteKind::TravelCandidate
                },
                EventRouteBasis::Timezone,
                if local {
                    format!("timezone {timezone} matches a configured local prefix")
                } else {
                    format!("timezone {timezone} does not match any configured local prefix")
                },
                None,
            );
        }
    }

    if policy.allow_unknown {
        return route(
            EventRouteKind::Local,
            EventRouteBasis::OperatorOverride,
            "allow_unknown=true explicitly routes missing geography locally",
            None,
        );
    }

    route(
        EventRouteKind::Unresolved,
        EventRouteBasis::MissingEvidence,
        "coordinates, country, and timezone cannot establish a route",
        None,
    )
}

fn route(
    route: EventRouteKind,
    basis: EventRouteBasis,
    reason: impl Into<String>,
    distance_km: Option<f64>,
) -> EventRoute {
    EventRoute {
        route,
        basis,
        reason: reason.into(),
        distance_km,
    }
}

fn non_empty(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

fn is_online_text(value: &str) -> bool {
    value
        .split(|character: char| !character.is_alphanumeric())
        .any(|token| {
            matches!(
                token.to_ascii_lowercase().as_str(),
                "online" | "virtual" | "remote"
            )
        })
}

fn valid_coordinate(latitude: f64, longitude: f64) -> bool {
    latitude.is_finite()
        && longitude.is_finite()
        && (-90.0..=90.0).contains(&latitude)
        && (-180.0..=180.0).contains(&longitude)
}

fn haversine_km(from_lat: f64, from_lng: f64, to_lat: f64, to_lng: f64) -> f64 {
    const EARTH_RADIUS_KM: f64 = 6_371.008_8;
    let from_lat = from_lat.to_radians();
    let to_lat = to_lat.to_radians();
    let delta_lat = to_lat - from_lat;
    let delta_lng = (to_lng - from_lng).to_radians();
    let a = (delta_lat / 2.0).sin().powi(2)
        + from_lat.cos() * to_lat.cos() * (delta_lng / 2.0).sin().powi(2);
    2.0 * EARTH_RADIUS_KM * a.sqrt().atan2((1.0 - a).sqrt())
}

fn round_one(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opportunity::SourceKind;

    fn policy() -> GeoPolicy {
        GeoPolicy {
            // Deliberately synthetic: public tests must not encode an
            // operator's real home coordinate.
            home_latitude: Some(10.0),
            home_longitude: Some(10.0),
            local_radius_km: Some(120.0),
            allow_countries: vec!["DE".into(), "Germany".into()],
            allow_unknown: false,
            allow_timezone_prefixes: vec!["Europe/".into()],
        }
    }

    fn event() -> Opportunity {
        Opportunity {
            id: "evt:test".into(),
            opportunity_type: OpportunityType::Event,
            source: "fixture".into(),
            source_kind: SourceKind::Api,
            url: "https://example.test/event".into(),
            title: "Synthetic event".into(),
            starts_at: None,
            ends_at: None,
            location: Some("Venue".into()),
            city: Some("City".into()),
            country_code: Some("DE".into()),
            latitude: Some(10.5),
            longitude: Some(10.0),
            raw: serde_json::json!({"timezone": "Europe/Berlin"}),
            fetched_at: "fixture".into(),
        }
    }

    #[test]
    fn real_distance_splits_same_day_from_travel() {
        let local = classify_opportunity(&event(), Some(&policy())).unwrap();
        assert_eq!(local.route, EventRouteKind::Local);
        assert_eq!(local.basis, EventRouteBasis::Coordinates);
        assert!(local.distance_km.is_some_and(|distance| distance < 120.0));

        let mut far = event();
        far.latitude = Some(60.0);
        far.longitude = Some(60.0);
        let far = classify_opportunity(&far, Some(&policy())).unwrap();
        assert_eq!(far.route, EventRouteKind::TravelCandidate);
        assert!(far.distance_km.is_some_and(|distance| distance > 6_000.0));
    }

    #[test]
    fn sparse_coordinates_fall_back_to_country_without_zero_guess() {
        let mut same_country = event();
        same_country.latitude = None;
        same_country.longitude = None;
        let same_country = classify_opportunity(&same_country, Some(&policy())).unwrap();
        assert_eq!(same_country.route, EventRouteKind::Local);
        assert_eq!(same_country.basis, EventRouteBasis::Country);
        assert_eq!(same_country.distance_km, None);

        let mut other_country = event();
        other_country.latitude = None;
        other_country.longitude = Some(0.0);
        other_country.country_code = Some("CA".into());
        let other_country = classify_opportunity(&other_country, Some(&policy())).unwrap();
        assert_eq!(other_country.route, EventRouteKind::TravelCandidate);
        assert_eq!(other_country.basis, EventRouteBasis::Country);
        assert_eq!(other_country.distance_km, None);
    }

    #[test]
    fn timezone_is_the_bounded_fallback_after_country() {
        let mut event = event();
        event.latitude = None;
        event.longitude = None;
        event.country_code = None;
        event.raw = serde_json::json!({"timezone": "America/New_York"});
        let route = classify_opportunity(&event, Some(&policy())).unwrap();
        assert_eq!(route.route, EventRouteKind::TravelCandidate);
        assert_eq!(route.basis, EventRouteBasis::Timezone);
    }

    #[test]
    fn online_metadata_wins_before_physical_distance() {
        let mut event = event();
        event.raw = serde_json::json!({"location_type": "online"});
        let route = classify_opportunity(&event, Some(&policy())).unwrap();
        assert_eq!(route.route, EventRouteKind::Online);
        assert_eq!(route.basis, EventRouteBasis::SourceMetadata);
    }

    #[test]
    fn missing_policy_and_missing_evidence_are_explicit() {
        let without_policy = classify_opportunity(&event(), None).unwrap();
        assert_eq!(without_policy.route, EventRouteKind::Unresolved);
        assert_eq!(without_policy.basis, EventRouteBasis::MissingPolicy);

        let mut unknown = event();
        unknown.latitude = None;
        unknown.longitude = None;
        unknown.country_code = None;
        unknown.raw = Value::Null;
        let unknown = classify_opportunity(&unknown, Some(&policy())).unwrap();
        assert_eq!(unknown.route, EventRouteKind::Unresolved);
        assert_eq!(unknown.basis, EventRouteBasis::MissingEvidence);
    }

    #[test]
    fn explicit_unknown_override_names_itself() {
        let mut unknown = event();
        unknown.latitude = None;
        unknown.longitude = None;
        unknown.country_code = None;
        unknown.raw = Value::Null;
        let mut policy = policy();
        policy.allow_unknown = true;
        let route = classify_opportunity(&unknown, Some(&policy)).unwrap();
        assert_eq!(route.route, EventRouteKind::Local);
        assert_eq!(route.basis, EventRouteBasis::OperatorOverride);
        assert!(route.reason.contains("allow_unknown=true"));
    }
}
