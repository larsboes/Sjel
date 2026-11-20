//! places: canonical place registry, cached geocoder, and the map layers.
//!
pub use places_api::*;

pub mod backfill;
pub mod climate;
pub mod config;
pub mod finance;
pub mod geocode;
pub mod layers;
pub mod links;
pub mod people;
pub mod store;
mod travel_reads;

/// Today as an ISO date from the wall clock, UTC.
///
/// The algorithm and its no-date-dependency reasoning are now `libs/civil-date`,
/// which this and the three other copies call. The name stays because the call
/// sites read better as `places::today()`.
pub fn today() -> String {
    civil_date::today()
}

/// An ISO date as days since the Unix epoch, for counting days between two of
/// them. `None` for anything not shaped like `YYYY-MM-DD`; see
/// `civil_date::unix_day_of_iso` for exactly how much shape it checks.
pub fn days_from_civil(iso: &str) -> Option<i64> {
    civil_date::unix_day_of_iso(iso)
}
