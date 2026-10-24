//! The places register and the people it links to: `places_places` and `places_person_places`
//! (`capabilities/places/src/store.rs`).
//!
//! Two tables, one question — "who is this, and where have we been together" — so this module owns
//! both the place row and the link, and the rules that join them. The link stores a person's *name*,
//! not an id (docs/capabilities/links.md D7–D11): `places_people` holds the operator's decision for
//! a name that means somebody, and `sjel_links::name_key` is how the two are compared. A dismissed
//! row is not a link and is never shown.
//!
//! Nothing here writes. The register's writes are the operator's review (`/api/people/decide`), and
//! a device that holds only the Mac's copy has no way to make one.

use serde::{Deserialize, Serialize};

pub use sjel_links::name_key;

/// One place, as the register holds it. `kind` is the register's own word
/// (`venue`, `city`, `station`, `address`, `region`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Place {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub city: Option<String>,
    pub country_code: Option<String>,
    pub address: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
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
}

/// One `places_person_places` row: a name the register has a place for, and what the operator
/// decided about it. `state` is `proposed` or `confirmed`; a dismissed row never reaches here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

/// The places linked to a person named `person`, in name order, confirmed links first.
///
/// A dismissed row is not a link, whatever the caller passed in: the operator already said that
/// place does not belong to that person. The copy does not carry dismissed rows at all
/// ([`crate::mirror::Mirror::people`]), so this is the rule stated once for both readers.
///
/// A person with no link is an empty answer, not an error: most of the register has none, and a
/// card that says nothing is true.
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

/// Filters places by a typed query against name, kind, city and country.
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
        // Confirmed first, then by name; the dismissed row is not a link at all.
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
        // Another place's people are not this place's.
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
