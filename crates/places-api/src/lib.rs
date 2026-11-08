//! Pure wire models, contracts, and route constants for the Sjel places capability.
//!
//! Shared across server capability (`sjel-places`) and client apps (`sjel-gpui-core`).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub use sjel_links::name_key;

// ============================================================================
// Route Constants
// ============================================================================

pub const DEFAULT_PLACES_PORT: u16 = 8093;

pub const ROUTE_PLACES_HEALTH: &str = "/health";
pub const ROUTE_PLACES_READY: &str = "/ready";
pub const ROUTE_PLACES_ROUTES: &str = "/routes";
pub const ROUTE_PLACES_LINKS: &str = "/api/links";
pub const ROUTE_PLACES_PEOPLE_OPEN: &str = "/api/people/open";
pub const ROUTE_PLACES_PEOPLE_DECIDE: &str = "/api/people/decide";
pub const ROUTE_PLACES: &str = "/api/places";
pub const ROUTE_PLACES_VISITS: &str = "/api/visits";
pub const ROUTE_PLACES_GEOCODE: &str = "/api/geocode";
pub const ROUTE_PLACES_LAYER_SPEND: &str = "/api/layers/spend";
pub const ROUTE_PLACES_LAYER_TRAVEL: &str = "/api/layers/travel";
pub const ROUTE_PLACES_LAYER_PEOPLE: &str = "/api/layers/people";
pub const ROUTE_PLACES_LAYER_PHOTOS: &str = "/api/layers/photos";
pub const ROUTE_PLACES_LAYER_COMPANIONS: &str = "/api/layers/companions";
pub const ROUTE_PLACES_LAYER_COMPOSITE: &str = "/api/layers/composite";
pub const ROUTE_PLACES_UNPLACED: &str = "/api/unplaced";
pub const ROUTE_PLACES_UNPLACED_ASSIGN: &str = "/api/unplaced/assign";
pub const ROUTE_PLACES_PEOPLE_PROPOSALS: &str = "/api/people/proposals";
pub const ROUTE_PLACES_PEOPLE_PLACES: &str = "/api/people/places";
pub const ROUTE_PLACES_PEOPLE_PRESENCE: &str = "/api/people/presence";
pub const ROUTE_PLACES_REVIEWS: &str = "/api/reviews";
pub const ROUTE_PLACES_REVIEWS_DECIDE: &str = "/api/reviews/decide";

// ============================================================================
// Core Place & Link Models
// ============================================================================

/// Coordinate pair validated within standard latitude and longitude ranges.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Coordinate {
    pub latitude: f64,
    pub longitude: f64,
}

impl Coordinate {
    pub fn new(latitude: f64, longitude: f64) -> Option<Self> {
        (latitude.is_finite()
            && longitude.is_finite()
            && (-90.0..=90.0).contains(&latitude)
            && (-180.0..=180.0).contains(&longitude))
        .then_some(Self {
            latitude,
            longitude,
        })
    }

    pub fn world(self, zoom: u8) -> (f64, f64) {
        self.world_at(f64::from(zoom))
    }

    /// Web Mercator projection at fractional zoom for fluid map panning and zooming.
    pub fn world_at(self, zoom: f64) -> (f64, f64) {
        let size = 256.0 * 2_f64.powf(zoom);
        let latitude = self.latitude.clamp(-85.05112878, 85.05112878).to_radians();
        (
            (self.longitude + 180.0) / 360.0 * size,
            (1.0 - latitude.tan().asinh() / std::f64::consts::PI) / 2.0 * size,
        )
    }

    pub fn from_world(x: f64, y: f64, zoom: u8) -> Self {
        Self::from_world_at(x, y, f64::from(zoom))
    }

    pub fn from_world_at(x: f64, y: f64, zoom: f64) -> Self {
        let size = 256.0 * 2_f64.powf(zoom);
        Self {
            longitude: x.rem_euclid(size) / size * 360.0 - 180.0,
            latitude: (std::f64::consts::PI * (1.0 - 2.0 * y.clamp(0.0, size) / size))
                .sinh()
                .atan()
                .to_degrees(),
        }
    }
}

/// Great-circle distance between two (latitude, longitude) coordinate pairs in kilometers.
pub fn haversine_km(a: (f64, f64), b: (f64, f64)) -> f64 {
    let (lat_a, lat_b) = (a.0.to_radians(), b.0.to_radians());
    let d_lat = (b.0 - a.0).to_radians();
    let d_lon = (b.1 - a.1).to_radians();
    let h = (d_lat / 2.0).sin().powi(2) + lat_a.cos() * lat_b.cos() * (d_lon / 2.0).sin().powi(2);
    2.0 * 6371.0 * h.clamp(0.0, 1.0).sqrt().asin()
}

/// One place, as the register holds it. `kind` is the register's own word
/// (`venue`, `city`, `station`, `address`, `region`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Place {
    #[serde(alias = "place_id")]
    pub id: String,
    pub name: String,
    pub kind: String,
    #[serde(default)]
    pub address: Option<String>,
    #[serde(default)]
    pub city: Option<String>,
    #[serde(default)]
    pub country_code: Option<String>,
    #[serde(default)]
    pub latitude: Option<f64>,
    #[serde(default)]
    pub longitude: Option<f64>,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub external_ref: Option<String>,
}

impl Place {
    /// Where the place is in one line: `Berlin, DE`, or the address when it is all there is.
    pub fn where_line(&self) -> String {
        let city = self.city.as_deref().unwrap_or_default().trim();
        let country = self.country_code.as_deref().unwrap_or_default().trim();
        match (city.is_empty(), country.is_empty()) {
            (false, false) => format!("{city}, {country}"),
            (false, true) => city.to_owned(),
            (true, false) => country.to_owned(),
            (true, true) => self.address.clone().unwrap_or_default(),
        }
    }

    /// The register's kind as a word a person reads, capitalised where it is a proper one.
    pub fn kind_label(&self) -> String {
        match self.kind.as_str() {
            "venue" => "Venue".to_owned(),
            "city" => "City".to_owned(),
            "station" => "Station".to_owned(),
            "address" => "Address".to_owned(),
            "region" => "Region".to_owned(),
            other => other.to_owned(),
        }
    }

    /// Coordinate pair if latitude and longitude are both present.
    pub fn coordinate(&self) -> Option<(f64, f64)> {
        match (self.latitude, self.longitude) {
            (Some(lat), Some(lon)) => Some((lat, lon)),
            _ => None,
        }
    }
}

/// One `places_person_places` row: a name the register has a place for, and what the operator
/// decided about it. `state` is `proposed` or `confirmed`; a dismissed row never reaches here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PersonLink {
    pub person: String,
    pub place_id: String,
    pub state: String,
}

impl PersonLink {
    /// The person's name in the form the register compares by.
    pub fn key(&self) -> String {
        name_key(&self.person)
    }

    /// True when the operator confirmed the link rather than a backfill proposing it.
    pub fn confirmed(&self) -> bool {
        self.state == "confirmed"
    }

    /// True when the operator said this place does not belong to that person.
    pub fn dismissed(&self) -> bool {
        self.state == "dismissed"
    }
}

/// One recorded visit: the operator was here, then, and rated it so.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PlaceVisit {
    pub id: String,
    pub place_id: String,
    #[serde(default)]
    pub visited_on: Option<String>,
    #[serde(default)]
    pub rating: Option<i64>,
    pub source: String,
    #[serde(default)]
    pub external_ref: Option<String>,
}

/// A visit joined to the place it happened at.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PlaceVisitRow {
    pub place_id: String,
    pub place_name: String,
    #[serde(default)]
    pub city: Option<String>,
    #[serde(default)]
    pub country_code: Option<String>,
    #[serde(default)]
    pub latitude: Option<f64>,
    #[serde(default)]
    pub longitude: Option<f64>,
    #[serde(default)]
    pub visited_on: Option<String>,
    #[serde(default)]
    pub rating: Option<i64>,
    pub source: String,
}

// ============================================================================
// Geocoding and Request Models
// ============================================================================

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct StructuredQuery {
    #[serde(default)]
    pub street: Option<String>,
    #[serde(default)]
    pub postalcode: Option<String>,
    #[serde(default)]
    pub city: Option<String>,
    #[serde(default)]
    pub country: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, JsonSchema)]
pub struct CreatePlaceRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub link: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub address: Option<String>,
    #[serde(default)]
    pub city: Option<String>,
    #[serde(default)]
    pub country_code: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub latitude: Option<f64>,
    #[serde(default)]
    pub longitude: Option<f64>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct GeocodeRequest {
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub structured: Option<StructuredQuery>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, JsonSchema)]
pub struct GeocodeResponse {
    #[serde(default)]
    pub place: Option<Place>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, JsonSchema)]
pub struct PlacesListResponse {
    #[serde(default)]
    pub places: Vec<Place>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, JsonSchema)]
pub struct VisitsListResponse {
    pub count: usize,
    #[serde(default)]
    pub visits: Vec<PlaceVisitRow>,
}

// ============================================================================
// Reductions and Queries
// ============================================================================

/// The places linked to a person named `person`, in name order, confirmed links first.
pub fn places_of<'a>(places: &'a [Place], links: &[PersonLink], person: &str) -> Vec<&'a Place> {
    let key = name_key(person);
    let mut found: Vec<(&Place, bool)> = links
        .iter()
        .filter(|link| link.key() == key && !link.dismissed())
        .filter_map(|link| {
            places
                .iter()
                .find(|place| place.id == link.place_id)
                .map(|place| (place, link.confirmed()))
        })
        .collect();
    found.sort_by(|(left, left_confirmed), (right, right_confirmed)| {
        right_confirmed
            .cmp(left_confirmed)
            .then_with(|| left.name.cmp(&right.name))
    });
    found.into_iter().map(|(place, _)| place).collect()
}

/// The names linked to one place, confirmed first, then alphabetically.
pub fn people_at(links: &[PersonLink], place_id: &str) -> Vec<(String, bool)> {
    let mut found: Vec<(String, bool)> = links
        .iter()
        .filter(|link| link.place_id == place_id && !link.dismissed())
        .map(|link| (link.person.clone(), link.confirmed()))
        .collect();
    found.sort_by(
        |(left_name, left_confirmed), (right_name, right_confirmed)| {
            right_confirmed
                .cmp(left_confirmed)
                .then_with(|| left_name.cmp(right_name))
        },
    );
    found.dedup_by(|left, right| name_key(&left.0) == name_key(&right.0));
    found
}

/// Filters places by a typed query against name, kind, city, country, and address.
pub fn filter_places<'a>(places: &'a [Place], query: &str) -> Vec<&'a Place> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return places.iter().collect();
    }
    places
        .iter()
        .filter(|place| {
            place.name.to_lowercase().contains(&query)
                || place.kind.to_lowercase().contains(&query)
                || place
                    .city
                    .as_deref()
                    .is_some_and(|city| city.to_lowercase().contains(&query))
                || place
                    .country_code
                    .as_deref()
                    .is_some_and(|code| code.to_lowercase().contains(&query))
                || place
                    .address
                    .as_deref()
                    .is_some_and(|address| address.to_lowercase().contains(&query))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn place(id: &str, name: &str, kind: &str, city: Option<&str>) -> Place {
        Place {
            id: id.into(),
            name: name.into(),
            kind: kind.into(),
            city: city.map(str::to_string),
            country_code: Some("DE".into()),
            address: None,
            latitude: None,
            longitude: None,
            source: "test".into(),
            external_ref: None,
        }
    }

    fn link(person: &str, place_id: &str, state: &str) -> PersonLink {
        PersonLink {
            person: person.into(),
            place_id: place_id.into(),
            state: state.into(),
        }
    }

    #[test]
    fn a_name_finds_its_places_whatever_the_spacing_and_case() {
        let places = vec![
            place("p1", "Konzerthaus", "venue", Some("Berlin")),
            place("p2", "Hauptbahnhof", "station", Some("Berlin")),
            place("p3", "Zürich", "city", Some("Zürich")),
        ];
        let links = vec![
            link("Ada Lovelace", "p2", "proposed"),
            link("ada  lovelace", "p1", "confirmed"),
            link("Ada Lovelace", "p3", "dismissed"),
        ];

        let found = places_of(&places, &links, "Ada Lovelace");
        assert_eq!(
            found.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
            vec!["Konzerthaus", "Hauptbahnhof"]
        );
    }

    #[test]
    fn a_place_says_who_the_register_links_to_it() {
        let links = vec![
            link("Ada Lovelace", "p1", "proposed"),
            link("Alan Turing", "p1", "confirmed"),
            link("ada lovelace", "p1", "proposed"),
            link("Grace Hopper", "p2", "confirmed"),
        ];
        assert_eq!(
            people_at(&links, "p1"),
            vec![
                ("Alan Turing".to_string(), true),
                ("Ada Lovelace".to_string(), false)
            ]
        );
        assert_eq!(people_at(&links, "p9"), vec![]);
    }

    #[test]
    fn the_where_line_falls_back_from_city_to_country_to_address() {
        let mut venue = place("p1", "Konzerthaus", "venue", Some("Berlin"));
        assert_eq!(venue.where_line(), "Berlin, DE");
        venue.city = None;
        assert_eq!(venue.where_line(), "DE");
        venue.country_code = None;
        venue.address = Some("Gendarmenmarkt 2".into());
        assert_eq!(venue.where_line(), "Gendarmenmarkt 2");
        assert_eq!(venue.kind_label(), "Venue");
    }

    #[test]
    fn a_place_is_found_by_its_city_as_well_as_its_name() {
        let places = vec![
            place("p1", "Konzerthaus", "venue", Some("Berlin")),
            place("p2", "Zürich", "city", Some("Zürich")),
        ];
        assert_eq!(filter_places(&places, "berlin").len(), 1);
        assert_eq!(filter_places(&places, "station").len(), 0);
        assert_eq!(filter_places(&places, "").len(), 2);
    }
}
