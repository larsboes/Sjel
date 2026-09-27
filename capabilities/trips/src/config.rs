//! Public, zero-personal-data config for the Trips store.
//!
//! The store path comes from `sjel_config::database_path` — `SJEL_DB_PATH`, else
//! `<overlay>/data/axon/axon.db`. One file for every capability (PRD Q45), so
//! there is no per-capability database to resolve any more.

use serde::Deserialize;
use sjel_config::{database_path, expand_tilde, resolve_port};
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize)]
pub struct ObsidianConfig {
    pub root: PathBuf,
    pub trips_dir: PathBuf,
}

/// A city where sleeping costs nothing and staying is wanted -- a friend, family.
/// The pivot-routing search (PRD F4) enumerates itineraries THROUGH these, which
/// is the one thing no commercial engine can offer: it does not know where you
/// are welcome. Personal by nature, so it lives in the overlay's trips.json,
/// never in this repository.
#[derive(Debug, Clone, Deserialize)]
pub struct PivotConfig {
    pub name: String,
    pub iata: String,
    /// How many nights staying there is welcome, which becomes the offset range
    /// for the onward leg. Defaults to 2.
    #[serde(default = "default_pivot_nights")]
    pub max_nights: u8,
}

fn default_pivot_nights() -> u8 {
    2
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct TravelPrefs {
    /// Cities where sleeping is free or wanted, and how many nights.
    ///
    /// `home_airport` used to sit here too. It moved to `capabilities/traveler`'s
    /// profile on 2026-09-23, because the default origin is a fact about the
    /// traveller rather than about this capability — and because two homes for it
    /// meant the profile's airports were read by nothing. A pivot stays: its
    /// `max_nights` is a property of the couch, not of the person.
    #[serde(default)]
    pub pivots: Vec<PivotConfig>,
}

/// Where `trips gear import` reads item notes from.
///
/// A directory under the overlay, never in this repository: the notes are the
/// operator's own wardrobe and kit. `items_dir` is relative to
/// `SJEL_PERSONAL_ROOT` unless it is absolute.
#[derive(Debug, Clone, Deserialize)]
pub struct GearConfig {
    #[serde(default = "default_items_dir")]
    pub items_dir: String,
}

fn default_items_dir() -> String {
    "data/items/vault-notes".into()
}

#[derive(Debug, Clone)]
pub struct Config {
    pub database_path: PathBuf,
    pub port: u16,
    pub obsidian: Option<ObsidianConfig>,
    pub travel: TravelPrefs,
    /// `None` when no overlay is configured, which is what `gear import` reports
    /// rather than scanning a guessed path.
    pub gear_items_dir: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
struct TripsFileConfig {
    obsidian: Option<TripsFileObsidian>,
    #[serde(default)]
    travel: Option<TravelPrefs>,
    #[serde(default)]
    gear: Option<GearConfig>,
}

#[derive(Debug, Deserialize)]
struct TripsFileObsidian {
    root: String,
    #[serde(default = "default_trips_dir")]
    trips_dir: String,
}

fn default_trips_dir() -> String {
    "Atlas/Events".into()
}

fn file_config() -> Option<TripsFileConfig> {
    let overlay = sjel_config::env_var("SJEL_PERSONAL_ROOT").ok()?;
    let path = expand_tilde(&overlay).join("config").join("trips.json");
    let body = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&body).ok()
}

fn obsidian_from_personal_config() -> Option<ObsidianConfig> {
    let obsidian = file_config()?.obsidian?;
    Some(ObsidianConfig {
        root: expand_tilde(&obsidian.root),
        trips_dir: PathBuf::from(obsidian.trips_dir),
    })
}

/// The gear notes directory, from `SJEL_TRIPS_GEAR_DIR` or the overlay's
/// `config/trips.json`. Absent when there is no overlay to resolve against.
fn gear_items_dir() -> Option<PathBuf> {
    if let Ok(explicit) = sjel_config::env_var("SJEL_TRIPS_GEAR_DIR") {
        return Some(expand_tilde(&explicit));
    }
    let overlay = expand_tilde(&sjel_config::env_var("SJEL_PERSONAL_ROOT").ok()?);
    let configured = file_config()
        .and_then(|c| c.gear)
        .map(|gear| gear.items_dir)
        .unwrap_or_else(default_items_dir);
    let relative = PathBuf::from(&configured);
    Some(if relative.is_absolute() {
        relative
    } else {
        overlay.join(relative)
    })
}

impl Config {
    pub fn load() -> Self {
        let port = resolve_port(None, None, 8086);
        let obsidian = match sjel_config::env_var("SJEL_TRIPS_OBSIDIAN_ROOT") {
            Ok(root) => Some(ObsidianConfig {
                root: expand_tilde(&root),
                trips_dir: PathBuf::from(
                    sjel_config::env_var("SJEL_TRIPS_OBSIDIAN_DIR")
                        .unwrap_or_else(|_| default_trips_dir()),
                ),
            }),
            Err(_) => obsidian_from_personal_config(),
        };
        let travel = file_config().and_then(|c| c.travel).unwrap_or_default();
        Self {
            database_path: database_path(),
            port,
            obsidian,
            travel,
            gear_items_dir: gear_items_dir(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn obsidian_default_is_atlas_events() {
        assert_eq!(default_trips_dir(), "Atlas/Events");
    }

    /// A shape, not a value: the default is a directory layout this repository
    /// may name, and the notes inside it are never read from here.
    #[test]
    fn the_gear_notes_default_is_a_relative_overlay_path() {
        assert_eq!(default_items_dir(), "data/items/vault-notes");
        assert!(!PathBuf::from(default_items_dir()).is_absolute());
    }
}
