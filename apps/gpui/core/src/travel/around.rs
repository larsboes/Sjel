//! Who the operator knows near each leg of a plan, and the meetups the plan holds.
//!
//! People come from entities' `/api/located`. Names stay on this device: the join runs here,
//! and nothing in it goes to a model.
use serde::Deserialize;

use super::{Item, Stage};

/// The disclosure radius of `crates/places` (README, D4).
pub const AROUND_RADIUS_KM: f64 = 50.0;

/// One person's place on a day (`GET /entities/api/located`).
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Located {
    pub entity_id: String,
    pub name: String,
    pub place: String,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    #[serde(default)]
    pub sleeping_option: Option<String>,
    #[serde(default)]
    pub sleeping_note: Option<String>,
}

/// A person within the radius of a leg's destination.
#[derive(Clone, Debug, PartialEq)]
pub struct Around {
    pub entity_id: String,
    pub person: String,
    pub place: String,
    pub distance_km: f64,
    /// Their sleeping option is `ask` or `yes`.
    pub host: bool,
    pub note: Option<String>,
}

/// An `activity` item whose payload names people in `with`.
#[derive(Clone, Debug, PartialEq)]
pub struct Meetup {
    pub item_id: String,
    pub title: String,
    pub people: Vec<String>,
    pub status: String,
    pub day: Option<String>,
    pub place: Option<String>,
}

pub use places_api::haversine_km;

/// The meetups on a plan, dated ones first.
pub fn meetups(items: &[Item]) -> Vec<Meetup> {
    let text = |item: &Item, key: &str| {
        item.payload
            .get(key)
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    };
    let mut out: Vec<Meetup> = items
        .iter()
        .filter(|item| item.item_type == "activity")
        .filter_map(|item| {
            let people: Vec<String> = item
                .payload
                .get("with")?
                .as_array()?
                .iter()
                .filter_map(|entry| entry.get("person")?.as_str())
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_owned)
                .collect();
            (!people.is_empty()).then(|| Meetup {
                item_id: item.id.clone(),
                title: item.title.clone(),
                people,
                status: text(item, "status").unwrap_or_else(|| "idea".into()),
                day: item.day.clone(),
                place: text(item, "place"),
            })
        })
        .collect();
    out.sort_by(|a, b| {
        a.day
            .as_deref()
            .unwrap_or("9999")
            .cmp(b.day.as_deref().unwrap_or("9999"))
    });
    out
}

/// People within [`AROUND_RADIUS_KM`] of the leg's destination, nearest first.
pub fn around(stage: &Stage, located: &[Located]) -> Vec<Around> {
    let Some(at) = stage.destination.coordinate() else {
        return vec![];
    };
    let mut out: Vec<Around> = located
        .iter()
        .filter_map(|person| {
            let distance_km = haversine_km(
                (at.latitude, at.longitude),
                (person.latitude?, person.longitude?),
            );
            (distance_km <= AROUND_RADIUS_KM).then(|| Around {
                entity_id: person.entity_id.clone(),
                person: person.name.clone(),
                place: person.place.clone(),
                distance_km,
                host: matches!(person.sleeping_option.as_deref(), Some("ask" | "yes")),
                note: person.sleeping_note.clone(),
            })
        })
        .collect();
    out.sort_by(|a, b| a.distance_km.total_cmp(&b.distance_km));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::travel::Place;
    use serde_json::json;

    fn item(id: &str, payload: serde_json::Value, day: Option<&str>) -> Item {
        Item {
            id: id.into(),
            item_type: "activity".into(),
            day: day.map(str::to_owned),
            external_id: id.into(),
            title: id.into(),
            payload,
            ..Default::default()
        }
    }

    #[test]
    fn only_activities_that_name_people_are_meetups_dated_first() {
        let found = meetups(&[
            item("museum", json!({ "status": "proposed" }), None),
            item("later", json!({ "with": [{ "person": "B" }] }), None),
            item(
                "boulder",
                json!({ "with": [{ "person": "A" }], "status": "asked", "place": "Bonn" }),
                Some("2026-10-14"),
            ),
        ]);
        let got: Vec<_> = found
            .iter()
            .map(|m| (m.item_id.as_str(), m.status.as_str(), m.people.clone()))
            .collect();
        assert_eq!(
            got,
            vec![
                ("boulder", "asked", vec!["A".to_owned()]),
                ("later", "idea", vec!["B".to_owned()]),
            ]
        );
    }

    #[test]
    fn people_near_the_leg_nearest_first_with_hosts_by_sleeping_option() {
        let place = |lat: Option<f64>| Place {
            name: "Bonn".into(),
            address: None,
            latitude: lat,
            longitude: lat.map(|_| 7.0982),
        };
        let stage = |lat| Stage {
            id: "stage:1".into(),
            sequence: 1,
            origin: place(None),
            destination: place(lat),
            date: None,
            status: "planned".into(),
            transport_modes: vec![],
        };
        let person = |id: &str, name: &str, at: Option<(f64, f64)>, sleep: &str| Located {
            entity_id: id.into(),
            name: name.into(),
            place: String::new(),
            latitude: at.map(|a| a.0),
            longitude: at.map(|a| a.1),
            sleeping_option: Some(sleep.into()),
            sleeping_note: (sleep == "ask").then(|| "sofa".into()),
        };
        let located = [
            person("ent:3", "Anna", Some((50.9375, 6.9603)), "none"),
            person("2", "Far", Some((38.72, -9.14)), "yes"),
            person("ent:1", "Ron", Some((50.735, 7.1)), "ask"),
            person("4", "Unplaced", None, "yes"),
        ];
        let near = around(&stage(Some(50.7374)), &located);
        let names: Vec<_> = near.iter().map(|p| (p.person.as_str(), p.host)).collect();
        assert_eq!(names, vec![("Ron", true), ("Anna", false)]);
        assert_eq!(near[0].note.as_deref(), Some("sofa"));
        assert!(around(&stage(None), &located).is_empty());
    }
}
