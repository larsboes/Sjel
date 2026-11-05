//! The places register and the people it links to: `places_places` and `places_person_places`
//! (`capabilities/places/src/store.rs`).
//!
//! Two tables, one question — "who is this, and where have we been together" — so this module owns
//! both the place row and the link, and the rules that join them. The link stores a person's *name*,
//! not an id (website/content/capabilities/links.md D7–D11): `places_people` holds the operator's decision for
//! a name that means somebody, and `sjel_links::name_key` is how the two are compared. A dismissed
//! row is not a link and is never shown.
//!
//! Nothing here writes. The register's writes are the operator's review (`/api/people/decide`), and
//! a device that holds only the Mac's copy has no way to make one.

pub use places_api::*;

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
