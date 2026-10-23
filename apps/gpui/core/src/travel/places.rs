//! Missing coordinates for a trip's places, resolved through the capability that owns
//! geocoding and kept on this device.
//!
//! A trip stores a place as it was typed. The dashboard's place field slugifies typed text
//! into `place:<slug>` with a null coordinate whenever nobody picks a suggested station, and
//! `trips draft-intent` does the same on purpose (`docs/capabilities/travel.md` §1). Every
//! coordinate-dependent view downstream then degrades without saying so: the map falls back
//! to a world overview and each stop reads "Coordinates not available"
//! (`capabilities/trips/src/store.rs`, `list_places`).
//!
//! Resolution happens in the client that needs the coordinate, not in the stored record: the
//! capability keeps what was typed, and the answer lands in `places.json` beside the plan
//! snapshot. That cache is what makes the map instant — a second open reads one small file
//! instead of asking a geocoder, and it keeps working when the Mac or the network is away.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{cache_dir, Coordinate, Place, Source};
use crate::shell::{self, ShellError};

const FILE: &str = "places.json";

/// One resolved coordinate, as stored. Plain floats so a cache written by an older build
/// still reads, and validated again by [`Coordinate::new`] on the way out.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct Cached {
    pub latitude: f64,
    pub longitude: f64,
}

/// Name key → coordinate. A map rather than a list because the only question asked is
/// "does this place already have an answer".
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct PlaceCoords {
    #[serde(default)]
    places: BTreeMap<String, Cached>,
}

/// The lookup key: the display name alone, so the same city referenced by a plan, a stage and
/// a detail read resolves once. Case and whitespace runs fold; umlauts do not, because
/// München and Munchen are two queries a geocoder answers differently
/// (`capabilities/trips/src/store.rs`, `normalize_place_name`).
pub fn key(place: &Place) -> String {
    place
        .name
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// What is sent to the geocoder. The address disambiguates a station or venue; a bare city
/// name is a fine query on its own.
pub fn query(place: &Place) -> String {
    match place.address.as_deref().map(str::trim) {
        Some(address) if !address.is_empty() => format!("{}, {address}", place.name.trim()),
        _ => place.name.trim().to_owned(),
    }
}

impl PlaceCoords {
    pub fn is_empty(&self) -> bool {
        self.places.is_empty()
    }

    pub fn len(&self) -> usize {
        self.places.len()
    }

    pub fn get(&self, place: &Place) -> Option<Coordinate> {
        let cached = self.places.get(&key(place))?;
        Coordinate::new(cached.latitude, cached.longitude)
    }

    pub fn insert(&mut self, place: &Place, at: Coordinate) {
        self.insert_at(&key(place), at);
    }

    /// Insert by the key [`resolve`] reports, for a caller that no longer holds the place.
    pub fn insert_at(&mut self, key: &str, at: Coordinate) {
        self.places.insert(
            key.to_owned(),
            Cached {
                latitude: at.latitude,
                longitude: at.longitude,
            },
        );
    }
}

fn path() -> Result<std::path::PathBuf, String> {
    Ok(cache_dir()
        .ok_or("This device has no travel storage directory.")?
        .join(FILE))
}

/// Disk work runs on the HTTP runtime's blocking pool, never GPUI's foreground executor.
pub async fn load() -> Result<PlaceCoords, String> {
    let path = path()?;
    tokio::task::spawn_blocking(move || match std::fs::read(path) {
        Ok(bytes) => {
            serde_json::from_slice(&bytes).map_err(|e| format!("Read resolved places: {e}"))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(PlaceCoords::default()),
        Err(e) => Err(format!("Read resolved places: {e}")),
    })
    .await
    .map_err(|e| e.to_string())?
}

pub async fn save(coords: PlaceCoords) -> Result<(), String> {
    let path = path()?;
    let bytes = serde_json::to_vec_pretty(&coords).map_err(|e| e.to_string())?;
    tokio::task::spawn_blocking(move || {
        if let Some(directory) = path.parent() {
            std::fs::create_dir_all(directory)?;
        }
        std::fs::write(path.with_extension("json.tmp"), &bytes)?;
        std::fs::rename(path.with_extension("json.tmp"), &path)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| format!("Save resolved places: {e}"))
}

/// The wire answer shared by both sources: `POST /places/api/geocode` replies
/// `{status, cached, place:{latitude, longitude, …}}` and `place` is null when nothing matched.
#[derive(Debug, Deserialize)]
struct Reply {
    place: Option<Point>,
}

#[derive(Debug, Deserialize)]
struct Point {
    latitude: Option<f64>,
    longitude: Option<f64>,
}

fn coordinate(reply: Reply) -> Option<Coordinate> {
    let point = reply.place?;
    Coordinate::new(point.latitude?, point.longitude?)
}

impl Source {
    /// One geocode, answered as a coordinate when the provider found the place.
    ///
    /// `places` is an on-demand capability: the first resolution after a restart starts it,
    /// which is the same recovery the assistant path already relies on
    /// (`Shell::ask`, and `set_running` beside it).
    pub async fn geocode(&self, query: &str) -> Result<Option<Coordinate>, String> {
        match self {
            Self::Shell(shell) => match shell.places_geocode(query).await {
                Ok(found) => Ok(found),
                Err(ShellError::Refused(502, _)) => {
                    shell
                        .set_running("places", true)
                        .await
                        .map_err(|e| e.to_string())?;
                    shell.places_geocode(query).await.map_err(|e| e.to_string())
                }
                Err(error) => Err(error.to_string()),
            },
            // The phone cannot start a capability; it asks the paired Mac, which proxies
            // the same route.
            Self::Device(client) => {
                let body = serde_json::to_vec(&serde_json::json!({ "query": query }))
                    .map_err(|e| e.to_string())?;
                let reply: Reply = client.post("/places/api/geocode", &body).await?;
                Ok(coordinate(reply))
            }
        }
    }
}

/// One name's answer: `Ok(None)` is the geocoder saying it has nothing, `Err` is the call itself
/// failing. A caller can remember the first and has to retry the second.
pub type Answer = (String, Result<Option<Coordinate>, String>);

/// Resolves pending `(key, query)` pairs one at a time and answers for every one of them.
///
/// Sequential on purpose: `places` throttles its own provider requests to one at a time
/// (`capabilities/places/src/geocode.rs`, `throttle`), so parallelism here would only
/// queue up inside the capability while spending this client's whole HTTP pool on it. A
/// failure answers for one name and does not hide the rest.
pub async fn resolve(source: Source, pending: Vec<(String, String)>) -> Vec<Answer> {
    let mut answers = Vec::with_capacity(pending.len());
    for (key, query) in pending {
        let asking = source.clone();
        let answer = shell::run(async move { asking.geocode(&query).await })
            .await
            .unwrap_or_else(|| Err("The request stopped before the geocoder answered.".into()));
        answers.push((key, answer));
    }
    answers
}

#[cfg(test)]
mod tests {
    use super::*;

    fn place(name: &str, address: Option<&str>) -> Place {
        Place {
            name: name.into(),
            address: address.map(str::to_owned),
            latitude: None,
            longitude: None,
        }
    }

    #[test]
    fn key_folds_case_and_whitespace_but_not_umlauts() {
        assert_eq!(key(&place("Bonn  Hbf", None)), "bonn hbf");
        assert_eq!(key(&place(" BONN HBF ", None)), "bonn hbf");
        assert_ne!(key(&place("München", None)), key(&place("Munchen", None)));
    }

    #[test]
    fn query_uses_the_address_only_when_it_has_one() {
        assert_eq!(query(&place("Bonn", None)), "Bonn");
        assert_eq!(query(&place("Bonn", Some("  "))), "Bonn");
        assert_eq!(
            query(&place("Köln Hbf", Some("Bahnhofplatz 1"))),
            "Köln Hbf, Bahnhofplatz 1"
        );
    }

    #[test]
    fn a_cached_answer_is_validated_before_it_is_used() {
        let mut coords = PlaceCoords::default();
        coords.insert(&place("Bonn", None), Coordinate::new(50.73, 7.1).unwrap());
        assert_eq!(
            coords.get(&place("BONN", None)),
            Coordinate::new(50.73, 7.1)
        );
        assert!(coords.get(&place("Hamburg", None)).is_none());
        coords.places.insert(
            key(&place("Nowhere", None)),
            Cached {
                latitude: 91.,
                longitude: 7.1,
            },
        );
        assert!(coords.get(&place("Nowhere", None)).is_none());
    }

    #[test]
    fn an_answer_without_a_place_or_coordinate_is_no_coordinate() {
        assert!(coordinate(Reply { place: None }).is_none());
        assert!(coordinate(Reply {
            place: Some(Point {
                latitude: None,
                longitude: None
            })
        })
        .is_none());
        assert_eq!(
            coordinate(Reply {
                place: Some(Point {
                    latitude: Some(52.5),
                    longitude: Some(13.4)
                })
            }),
            Coordinate::new(52.5, 13.4)
        );
    }
}
