use places_api::{GeocodeResponse, Place};
use serde_json::json;

#[test]
fn geocoder_projection_reads_the_existing_place_id_without_registry_only_fields() {
    let response: GeocodeResponse = serde_json::from_value(json!({
        "status": "ok", "cached": true,
        "place": {"place_id": "p1", "name": "Berlin", "kind": "city",
            "latitude": 52.52, "longitude": 13.405,
            "city": "Berlin", "country_code": "DE"}
    }))
    .unwrap();
    let place = response.place.unwrap();
    assert_eq!(place.id, "p1");
    assert_eq!(place.latitude, Some(52.52));
    assert_eq!(place.source, "");
    assert!(place.address.is_none());
}

#[test]
fn registry_id_stays_canonical_and_a_geocode_miss_stays_absent() {
    let place: Place = serde_json::from_value(json!({
        "id": "p1", "name": "Berlin", "kind": "city"
    }))
    .unwrap();
    let wire = serde_json::to_value(place).unwrap();
    assert_eq!(wire["id"], "p1");
    assert!(wire.get("place_id").is_none());
    let miss: GeocodeResponse =
        serde_json::from_value(json!({"status": "not_found", "place": null})).unwrap();
    assert!(miss.place.is_none());
}
