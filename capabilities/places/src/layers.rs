//! The three map layers, assembled with read-only SELECTs over `finance_*`,
//! `trips_*` and `transit_*`. One database with a namespace per capability was
//! chosen exactly for these joins; PRD Q45 (2026-08-27) made that namespace a
//! table prefix instead of a schema, and the joins are the same joins.
//! The wire shapes here are the dashboard map's contract; every collection is
//! GeoJSON with `[longitude, latitude]` coordinate order.

use rusqlite::params;
use sjel_store::QueryAll;

use crate::geocode::{GeocodeQuery, Geocoder};
use crate::store::{validate_prefix, Fallible, PlacesStore};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};

fn feature(longitude: f64, latitude: f64, properties: Value) -> Value {
    json!({
        "type": "Feature",
        "geometry": { "type": "Point", "coordinates": [longitude, latitude] },
        "properties": properties,
    })
}

fn line_feature(coordinates: Vec<[f64; 2]>, properties: Value) -> Value {
    json!({
        "type": "Feature",
        "geometry": { "type": "LineString", "coordinates": coordinates },
        "properties": properties,
    })
}

fn collection(features: Vec<Value>) -> Value {
    json!({ "type": "FeatureCollection", "features": features })
}

/// A station reference as transit persists it: either a bare EVA code
/// (`transit.trips`) or a full HAFAS location id whose `L=` names the EVA and
/// whose `X=`/`Y=` carry longitude/latitude in microdegrees
/// (`transit.trip_legs`).
#[derive(Debug, Clone, PartialEq)]
pub struct StationRef {
    pub eva: String,
    pub name: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
}

/// EVA codes appear zero-padded to eight digits in `punctuality.stations` and
/// unpadded in `transit.trips`; one spelling, so the registry key is stable.
pub fn normalize_eva(raw: &str) -> String {
    let trimmed = raw.trim().trim_start_matches('0');
    if trimmed.is_empty() {
        raw.trim().to_string()
    } else {
        trimmed.to_string()
    }
}

pub fn parse_station_ref(raw: &str) -> Option<StationRef> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    if !raw.contains('@') {
        return Some(StationRef {
            eva: normalize_eva(raw),
            name: None,
            latitude: None,
            longitude: None,
        });
    }
    let field = |key: &str| {
        raw.split('@')
            .find_map(|part| part.strip_prefix(key))
            .map(str::to_string)
    };
    let micro = |key: &str| {
        field(key)
            .and_then(|value| value.parse::<f64>().ok())
            .map(|value| value / 1_000_000.0)
    };
    Some(StationRef {
        eva: normalize_eva(&field("L=")?),
        name: field("O="),
        latitude: micro("Y="),
        longitude: micro("X="),
    })
}

/// One linked expense row: a `places.transaction_places` link joined to the
/// finance projection on the journal-stable `source_id` (README D2).
struct LinkedSpendRow {
    place_id: String,
    precision: String,
    place_name: String,
    place_city: Option<String>,
    country_code: Option<String>,
    latitude: Option<f64>,
    longitude: Option<f64>,
    amount_cents: i64,
    booked_at: String,
    category: String,
}

fn linked_spend_rows(store: &PlacesStore, finance_prefix: &str) -> Fallible<Vec<LinkedSpendRow>> {
    validate_prefix(finance_prefix)?;
    let prefix = store.prefix();
    let conn = store.conn()?;
    Ok(conn.query_all(
        &format!(
            "SELECT tp.place_id, tp.precision, pl.name, pl.city, pl.country_code,
                    pl.latitude, pl.longitude, p.amount_cents, p.booked_at, p.category
             FROM {prefix}_transaction_places tp
             JOIN {prefix}_places pl ON pl.id = tp.place_id
             JOIN {finance_prefix}_transaction_projection p ON p.source_id = tp.source_id
             WHERE p.kind = 'expense' AND p.currency = 'EUR'
             ORDER BY p.booked_at, tp.source_id"
        ),
        [],
        |row| {
            Ok(LinkedSpendRow {
                place_id: row.get(0)?,
                precision: row.get(1)?,
                place_name: row.get(2)?,
                place_city: row.get(3)?,
                country_code: row.get(4)?,
                latitude: row.get(5)?,
                longitude: row.get(6)?,
                amount_cents: row.get(7)?,
                booked_at: row.get(8)?,
                category: row.get(9)?,
            })
        },
    )?)
}

/// Coordinates for city names, from city-kind registry rows.
fn city_coordinates(store: &PlacesStore) -> Fallible<HashMap<String, (f64, f64)>> {
    Ok(store
        .places_with_coordinates()?
        .into_iter()
        .filter(|place| place.kind == "city")
        .filter_map(|place| {
            Some((
                place.name.to_lowercase(),
                (place.latitude?, place.longitude?),
            ))
        })
        .collect())
}

#[derive(Default)]
struct Aggregate {
    total_cents: i64,
    transactions: i64,
    first: Option<String>,
    last: Option<String>,
    categories: BTreeMap<String, i64>,
}

impl Aggregate {
    fn add(&mut self, row: &LinkedSpendRow) {
        self.total_cents += row.amount_cents;
        self.transactions += 1;
        if self
            .first
            .as_deref()
            .is_none_or(|d| row.booked_at.as_str() < d)
        {
            self.first = Some(row.booked_at.clone());
        }
        if self
            .last
            .as_deref()
            .is_none_or(|d| row.booked_at.as_str() > d)
        {
            self.last = Some(row.booked_at.clone());
        }
        *self.categories.entry(row.category.clone()).or_default() += row.amount_cents;
    }

    fn top_category(&self) -> Option<String> {
        self.categories
            .iter()
            .max_by_key(|(_, cents)| **cents)
            .map(|(category, _)| category.clone())
    }
}

/// `GET /api/layers/spend`.
///
/// Reconciliation (ISA PLC-5): `summary.total_cents` is the sum of
/// `finance.transaction_projection.amount_cents` over exactly the linked EUR
/// expense rows, computed from the same join the features come from — the two
/// cannot disagree because they are one query. `summary.transactions` counts
/// every EUR expense row in the projection; `summary.linked` counts the ones a
/// link exists for.
pub fn spend_layer(store: &PlacesStore) -> Fallible<Value> {
    spend_layer_in(store, "finance")
}

/// The finance prefix is a parameter so a test can point the same SQL at a scratch
/// projection table in its own file; production passes `finance`.
pub fn spend_layer_in(store: &PlacesStore, finance_prefix: &str) -> Fallible<Value> {
    validate_prefix(finance_prefix)?;
    let rows = linked_spend_rows(store, finance_prefix)?;
    let city_coords = city_coordinates(store)?;
    let conn = store.conn()?;
    let projected_expenses: i64 = conn.query_row(
        &format!(
            "SELECT COUNT(*) FROM {finance_prefix}_transaction_projection
             WHERE kind = 'expense' AND currency = 'EUR'"
        ),
        [],
        |row| row.get(0),
    )?;

    struct PlaceInfo {
        name: String,
        city: Option<String>,
        country_code: Option<String>,
        latitude: Option<f64>,
        longitude: Option<f64>,
    }
    let mut venue_info: HashMap<String, PlaceInfo> = HashMap::new();
    let mut venue_agg: HashMap<String, Aggregate> = HashMap::new();
    let mut city_place_info: HashMap<String, PlaceInfo> = HashMap::new();
    let mut city_place_agg: HashMap<String, Aggregate> = HashMap::new();
    // City ranking spans both precisions: a venue purchase counts toward its
    // place's city, a city link toward the city place's own name.
    let mut ranking: HashMap<(String, Option<String>), Aggregate> = HashMap::new();

    let mut total_cents = 0_i64;
    for row in &rows {
        total_cents += row.amount_cents;
        let (bucket_info, bucket_agg) = if row.precision == "venue" {
            (&mut venue_info, &mut venue_agg)
        } else {
            (&mut city_place_info, &mut city_place_agg)
        };
        bucket_info
            .entry(row.place_id.clone())
            .or_insert(PlaceInfo {
                name: row.place_name.clone(),
                city: row.place_city.clone(),
                country_code: row.country_code.clone(),
                latitude: row.latitude,
                longitude: row.longitude,
            });
        bucket_agg.entry(row.place_id.clone()).or_default().add(row);

        let city_name = if row.precision == "venue" {
            row.place_city.clone()
        } else {
            Some(row.place_name.clone())
        };
        if let Some(city_name) = city_name.filter(|name| !name.is_empty()) {
            ranking
                .entry((city_name, row.country_code.clone()))
                .or_default()
                .add(row);
        }
    }

    let mut venue_features: Vec<(i64, Value)> = venue_agg
        .iter()
        .filter_map(|(place_id, agg)| {
            let info = venue_info.get(place_id)?;
            let (latitude, longitude) = (info.latitude?, info.longitude?);
            let avg = (agg.total_cents + agg.transactions / 2) / agg.transactions.max(1);
            Some((
                agg.total_cents,
                feature(
                    longitude,
                    latitude,
                    json!({
                        "place_id": place_id,
                        "name": info.name,
                        "city": info.city,
                        "precision": "venue",
                        "total_cents": agg.total_cents,
                        "transactions": agg.transactions,
                        "avg_cents": avg,
                        "first": agg.first,
                        "last": agg.last,
                        "top_category": agg.top_category(),
                    }),
                ),
            ))
        })
        .collect();
    venue_features.sort_by_key(|(total, _)| std::cmp::Reverse(*total));

    let mut city_features: Vec<(i64, Value)> = city_place_agg
        .iter()
        .filter_map(|(place_id, agg)| {
            let info = city_place_info.get(place_id)?;
            let (latitude, longitude) = (info.latitude?, info.longitude?);
            Some((
                agg.total_cents,
                feature(
                    longitude,
                    latitude,
                    json!({
                        "place_id": place_id,
                        "city": info.name,
                        "country_code": info.country_code,
                        "precision": "city",
                        "total_cents": agg.total_cents,
                        "transactions": agg.transactions,
                    }),
                ),
            ))
        })
        .collect();
    city_features.sort_by_key(|(total, _)| std::cmp::Reverse(*total));

    let mut ranked: Vec<_> = ranking.into_iter().collect();
    ranked.sort_by(|a, b| b.1.total_cents.cmp(&a.1.total_cents).then(a.0.cmp(&b.0)));
    let cities: Vec<Value> = ranked
        .into_iter()
        .map(|((city, country_code), agg)| {
            let coords = city_coords.get(&city.to_lowercase());
            json!({
                "city": city,
                "country_code": country_code,
                "total_cents": agg.total_cents,
                "transactions": agg.transactions,
                "latitude": coords.map(|c| c.0),
                "longitude": coords.map(|c| c.1),
            })
        })
        .collect();

    Ok(json!({
        "summary": {
            "total_cents": total_cents,
            "transactions": projected_expenses,
            "linked": rows.len(),
            "venues": venue_features.len(),
            "cities": cities,
        },
        "venues": collection(venue_features.into_iter().map(|(_, f)| f).collect()),
        "cities": collection(city_features.into_iter().map(|(_, f)| f).collect()),
    }))
}

fn phase(date: Option<&str>, today: &str) -> Value {
    match date {
        Some(date) if date.get(..10).unwrap_or(date) < today => json!("past"),
        Some(_) => json!("upcoming"),
        None => Value::Null,
    }
}

/// `GET /api/layers/travel`: trip destinations from `trips_plans`, transit legs
/// as LineStrings between station coordinates, station points from the
/// registry, and city-presence evidence derived from linked spend.
pub fn travel_layer(store: &PlacesStore, today: &str) -> Fallible<Value> {
    travel_layer_in(store, "trips", "transit", today)
}

/// The neighbour prefixes are parameters so a test can build scratch `plans` and
/// `trip_legs` tables in its own file; production passes `trips` and `transit`.
pub fn travel_layer_in(
    store: &PlacesStore,
    trips_prefix: &str,
    transit_prefix: &str,
    today: &str,
) -> Fallible<Value> {
    validate_prefix(trips_prefix)?;
    validate_prefix(transit_prefix)?;
    let mut points: Vec<Value> = Vec::new();
    let mut routes: Vec<Value> = Vec::new();

    // Trip destinations: PlaceRef JSON serialized by trips' store
    // (capabilities/trips/src/store.rs); only refs that carry a coordinate can
    // be drawn.
    {
        let conn = store.conn()?;
        let plans: Vec<(String, String, String, String)> = conn.query_all(
            &format!(
                "SELECT id, destinations, date_start, date_end
                 FROM {trips_prefix}_plans WHERE status != 'archived'
                 ORDER BY date_start, id"
            ),
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
        for (plan_id, destinations, date_start, date_end) in plans {
            let Ok(refs) = serde_json::from_str::<Value>(&destinations) else {
                continue;
            };
            for place_ref in refs.as_array().into_iter().flatten() {
                let (Some(latitude), Some(longitude)) = (
                    place_ref.get("latitude").and_then(Value::as_f64),
                    place_ref.get("longitude").and_then(Value::as_f64),
                ) else {
                    continue;
                };
                let name = place_ref
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("unnamed destination");
                points.push(feature(
                    longitude,
                    latitude,
                    json!({
                        "kind": "trip-destination",
                        "name": name,
                        "phase": phase(Some(&date_end), today),
                        "plan_id": plan_id,
                        "first": date_start,
                        "last": date_end,
                        "visits": Value::Null,
                    }),
                ));
            }
        }
    }

    // Stations and legs. Station coordinates come from the registry
    // (`backfill stations` fills it, ISA PLC-6); a leg whose HAFAS ref embeds
    // its own coordinates can be drawn even before the registry has the row.
    let station_places: HashMap<String, (String, f64, f64)> = store
        .places_with_coordinates()?
        .into_iter()
        .filter(|place| place.kind == "station")
        .filter_map(|place| {
            let eva = place.external_ref?.strip_prefix("eva:")?.to_string();
            Some((eva, (place.name, place.latitude?, place.longitude?)))
        })
        .collect();

    struct LegRow {
        origin: String,
        origin_name: String,
        destination: String,
        destination_name: String,
        departure_time: String,
        train_name: String,
    }
    let legs: Vec<LegRow> = {
        let conn = store.conn()?;
        conn.query_all(
            &format!(
                "SELECT origin_eva, origin_name, destination_eva, destination_name,
                        departure_time, train_name
                 FROM {transit_prefix}_trip_legs
                 ORDER BY departure_time, trip_id, leg_index"
            ),
            [],
            |row| {
                Ok(LegRow {
                    origin: row.get(0)?,
                    origin_name: row.get(1)?,
                    destination: row.get(2)?,
                    destination_name: row.get(3)?,
                    departure_time: row.get(4)?,
                    train_name: row.get(5)?,
                })
            },
        )?
    };

    let mut visits: HashMap<String, i64> = HashMap::new();
    let mut seen_routes: HashSet<String> = HashSet::new();
    for leg in &legs {
        let Some(origin) = parse_station_ref(&leg.origin) else {
            continue;
        };
        let Some(destination) = parse_station_ref(&leg.destination) else {
            continue;
        };
        *visits.entry(origin.eva.clone()).or_default() += 1;
        *visits.entry(destination.eva.clone()).or_default() += 1;
        let resolve = |station: &StationRef| -> Option<[f64; 2]> {
            match (station.longitude, station.latitude) {
                (Some(lon), Some(lat)) => Some([lon, lat]),
                _ => station_places
                    .get(&station.eva)
                    .map(|(_, lat, lon)| [*lon, *lat]),
            }
        };
        let (Some(from), Some(to)) = (resolve(&origin), resolve(&destination)) else {
            continue;
        };
        let label = if leg.train_name.is_empty() {
            format!("{} → {}", leg.origin_name, leg.destination_name)
        } else {
            format!(
                "{} → {} · {}",
                leg.origin_name, leg.destination_name, leg.train_name
            )
        };
        let leg_phase = phase(Some(&leg.departure_time), today);
        let identity = format!("{}|{}|{label}|{leg_phase}", origin.eva, destination.eva);
        if !seen_routes.insert(identity) {
            continue;
        }
        routes.push(line_feature(
            vec![from, to],
            json!({ "kind": "transit-leg", "label": label, "phase": leg_phase }),
        ));
    }

    for (eva, (name, latitude, longitude)) in &station_places {
        points.push(feature(
            *longitude,
            *latitude,
            json!({
                "kind": "station",
                "name": name,
                "phase": Value::Null,
                "plan_id": Value::Null,
                "first": Value::Null,
                "last": Value::Null,
                "visits": visits.get(eva).copied().unwrap_or(0),
            }),
        ));
    }

    // Spend presence: cities where linked purchases prove the operator was.
    let spend_rows = linked_spend_rows(store, "finance")?;
    let city_coords = city_coordinates(store)?;
    let mut presence: BTreeMap<String, Aggregate> = BTreeMap::new();
    for row in &spend_rows {
        let city_name = if row.precision == "venue" {
            row.place_city.clone()
        } else {
            Some(row.place_name.clone())
        };
        if let Some(city_name) = city_name.filter(|name| !name.is_empty()) {
            presence.entry(city_name).or_default().add(row);
        }
    }
    for (city, agg) in presence {
        let Some((latitude, longitude)) = city_coords.get(&city.to_lowercase()) else {
            continue;
        };
        points.push(feature(
            *longitude,
            *latitude,
            json!({
                "kind": "spend-presence",
                "name": city,
                "phase": "past",
                "plan_id": Value::Null,
                "first": agg.first,
                "last": agg.last,
                "visits": agg.transactions,
            }),
        ));
    }

    Ok(json!({
        "points": collection(points),
        "routes": collection(routes),
    }))
}

/// `GET /api/unplaced` (the dashboard's review contract): expense-kind EUR
/// projection rows with no `transaction_places` link, grouped by exact
/// description, ranked by total spend, capped at 200 groups. Cents are
/// integers, EUR implied — the same units as the spend layer.
pub fn unplaced_groups(store: &PlacesStore) -> Fallible<Value> {
    unplaced_groups_in(store, "finance")
}

/// The finance prefix is a parameter so a test can point the same SQL at a scratch
/// copy of the projection table; production passes `finance`.
pub fn unplaced_groups_in(store: &PlacesStore, finance_prefix: &str) -> Fallible<Value> {
    validate_prefix(finance_prefix)?;
    let prefix = store.prefix();
    let conn = store.conn()?;
    // `COUNT(*)::bigint` and `SUM(...)::bigint` lose their casts: SQLite integers
    // are already 64-bit. `left(x, 10)` becomes `substr(x, 1, 10)`.
    let groups: Vec<Value> = conn.query_all(
        &format!(
            "SELECT p.description, COUNT(*), SUM(p.amount_cents),
                    substr(MIN(p.booked_at), 1, 10), substr(MAX(p.booked_at), 1, 10)
             FROM {finance_prefix}_transaction_projection p
             WHERE p.kind = 'expense' AND p.currency = 'EUR'
               AND p.source_id IS NOT NULL
               AND NOT EXISTS (
                   SELECT 1 FROM {prefix}_transaction_places tp
                   WHERE tp.source_id = p.source_id
               )
             GROUP BY p.description
             ORDER BY SUM(p.amount_cents) DESC, p.description
             LIMIT 200"
        ),
        [],
        |row| {
            Ok(json!({
                "description": row.get::<_, String>(0)?,
                "transactions": row.get::<_, i64>(1)?,
                "total_cents": row.get::<_, i64>(2)?,
                "first": row.get::<_, String>(3)?,
                "last": row.get::<_, String>(4)?,
            }))
        },
    )?;
    Ok(json!({ "groups": groups }))
}

/// `POST /api/unplaced/assign` body. Exactly one of `place_id` /
/// `geocode_query`; the server handler enforces that and the precision
/// vocabulary before any store work.
#[derive(Debug, Deserialize)]
pub struct AssignUnplaced {
    pub description: String,
    pub place_id: Option<String>,
    pub geocode_query: Option<String>,
    pub precision: String,
}

/// Resolve the target place (an existing registry row by id, or the cached
/// geocoder), then link every currently-unlinked source_id whose projection
/// description matches exactly — `source='manual'`, ON CONFLICT DO NOTHING.
/// `Ok(None)` means no place resolved (the caller's 404). A city-kind place is
/// linked at city precision whatever the request said (README D1); the
/// response reports the precision actually written. The write stays inside the
/// places schema (ISA A2); the description match is a read-only SELECT over
/// finance.
pub fn assign_unplaced(
    store: &PlacesStore,
    geocoder: &Geocoder,
    finance_prefix: &str,
    request: &AssignUnplaced,
    today: &str,
) -> Fallible<Option<Value>> {
    validate_prefix(finance_prefix)?;
    if !matches!(request.precision.as_str(), "venue" | "city") {
        return Err("precision must be venue or city".into());
    }
    let place = match (&request.place_id, &request.geocode_query) {
        (Some(id), None) => store.place(id)?,
        (None, Some(query)) => {
            // No kind_override: the registry row keeps the kind DERIVED from
            // the provider response, so a venue-precision request cannot
            // register a city relation as a venue-kind row (README D1). The
            // precision guard below then works off that honest kind.
            geocoder
                .geocode(&GeocodeQuery::Free(query.clone()), None, today)?
                .place
        }
        _ => return Err("send exactly one of place_id or geocode_query".into()),
    };
    let Some(place) = place else {
        return Ok(None);
    };
    // The server-side twin of the dashboard's pickPrecision rule
    // (dashboard/src/routes/map/+page.svelte): a city-kind place is linked at
    // city precision whatever was requested — a city bubble never pretends to
    // be a venue (README D1). Enforced here too because the API is a caller
    // surface of its own, not only the dashboard's backend.
    let precision = if place.kind == "city" {
        "city"
    } else {
        request.precision.as_str()
    };
    let prefix = store.prefix();
    let conn = store.conn()?;
    // 9500: operator-asserted, above any derived link (amex venue 9000,
    // city fallback 6000). The `::text`/`::smallint` casts on the SELECT list are
    // gone: they told Postgres what type a bare parameter was, and SQLite binds a
    // value rather than inferring a type.
    let linked = conn.execute(
        &format!(
            "INSERT INTO {prefix}_transaction_places
                (source_id, place_id, precision, confidence_bp, source, created_at)
             SELECT DISTINCT p.source_id, ?2, ?3, ?4, 'manual', ?5
             FROM {finance_prefix}_transaction_projection p
             WHERE p.kind = 'expense' AND p.currency = 'EUR'
               AND p.source_id IS NOT NULL AND p.description = ?1
             ON CONFLICT (source_id) DO NOTHING"
        ),
        params![&request.description, &place.id, precision, 9500_i16, &today],
    )?;
    Ok(Some(json!({
        "ok": true,
        "linked": linked,
        "precision": precision,
        "place": {
            "id": place.id,
            "name": place.name,
            "kind": place.kind,
            "city": place.city,
            "country_code": place.country_code,
            "latitude": place.latitude,
            "longitude": place.longitude,
        },
    })))
}

/// `GET /api/layers/people`: confirmed, currently-valid register rows only —
/// proposals and dismissals never reach the map (README D4).
pub fn people_layer(store: &PlacesStore, today: &str) -> Fallible<Value> {
    let prefix = store.prefix();
    let conn = store.conn()?;
    let features = conn
        .query_all(
            &format!(
                "SELECT pp.id, pp.person, pp.date_start, pp.confidence_bp, pp.source,
                        pl.name, pl.latitude, pl.longitude, pp.date_end
                 FROM {prefix}_person_places pp
                 JOIN {prefix}_places pl ON pl.id = pp.place_id
                 WHERE pp.state = 'confirmed'
                   AND (pp.date_end IS NULL OR pp.date_end >= ?1)
                 ORDER BY pp.person, pp.id"
            ),
            params![&today],
            |row| {
                // A tuple, then the coordinate filter below: a row without one is
                // not an error, it is a confirmed relation to a place nobody has
                // geocoded yet, and it simply cannot be drawn.
                Ok((
                    json!({
                        "id": row.get::<_, String>(0)?,
                        "person": row.get::<_, String>(1)?,
                        "place_name": row.get::<_, String>(5)?,
                        "since": row.get::<_, Option<String>>(2)?,
                        // Null means "still there". A planner asking about a day
                        // after today needs the end as much as the start.
                        "until": row.get::<_, Option<String>>(8)?,
                        "confidence_bp": i64::from(row.get::<_, i16>(3)?),
                        "source": row.get::<_, String>(4)?,
                    }),
                    row.get::<_, Option<f64>>(6)?,
                    row.get::<_, Option<f64>>(7)?,
                ))
            },
        )?
        .into_iter()
        .filter_map(|(properties, latitude, longitude)| {
            Some(feature(longitude?, latitude?, properties))
        })
        .collect();
    Ok(collection(features))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_hafas_ref_yields_eva_name_and_coordinates() {
        let station =
            parse_station_ref("A=1@O=Köln Hbf@X=6958730@Y=50943029@U=80@L=8000207@i=U×008015458@")
                .unwrap();
        assert_eq!(station.eva, "8000207");
        assert_eq!(station.name.as_deref(), Some("Köln Hbf"));
        assert_eq!(station.longitude, Some(6.958_73));
        assert_eq!(station.latitude, Some(50.943_029));
    }

    #[test]
    fn a_bare_eva_normalizes_the_punctuality_zero_padding_away() {
        let station = parse_station_ref("8000207").unwrap();
        assert_eq!(station.eva, "8000207");
        assert_eq!(station.latitude, None);
        assert_eq!(parse_station_ref("08000207").unwrap().eva, "8000207");
        assert_eq!(normalize_eva("08000207"), normalize_eva("8000207"));
        assert!(parse_station_ref("  ").is_none());
    }

    #[test]
    fn phases_compare_the_date_part_against_today() {
        assert_eq!(
            phase(Some("2026-08-08T08:31:00"), "2026-08-25"),
            json!("past")
        );
        assert_eq!(phase(Some("2026-09-01"), "2026-08-25"), json!("upcoming"));
        assert_eq!(phase(None, "2026-08-25"), Value::Null);
    }
}

/// Layer queries over the neighbour tables, in this test's own file. Named `db_tests`
/// like every other database-backed module — see `store.rs`'s note on the selector.
#[cfg(test)]
mod db_tests {
    use super::*;
    use crate::store::db_tests::open_test_store;
    use rusqlite::Connection;

    /// The neighbour tables the layers read, created in the test's own file under the
    /// prefixes production uses. It restates their shape rather than linking their
    /// crates, because a capability depends on another's surface and never its code
    /// (README.md#schemas-and-dependency-direction) — the columns named here ARE the
    /// coupling the cross-capability join creates, so writing them down is the point.
    /// Kept to the columns these queries project.
    fn create_neighbour_tables(path: &std::path::Path) -> Connection {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS finance_transaction_projection (
                 source_id TEXT,
                 description TEXT NOT NULL,
                 amount_cents INTEGER NOT NULL,
                 booked_at TEXT NOT NULL,
                 kind TEXT NOT NULL,
                 category TEXT NOT NULL DEFAULT 'uncategorized',
                 currency TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS trips_plans (
                 id TEXT PRIMARY KEY,
                 destinations TEXT NOT NULL,
                 date_start TEXT NOT NULL,
                 date_end TEXT NOT NULL,
                 status TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS transit_trip_legs (
                 trip_id TEXT NOT NULL,
                 leg_index INTEGER NOT NULL,
                 origin_eva TEXT NOT NULL,
                 origin_name TEXT NOT NULL,
                 destination_eva TEXT NOT NULL,
                 destination_name TEXT NOT NULL,
                 departure_time TEXT NOT NULL,
                 train_name TEXT NOT NULL
             );",
        )
        .unwrap();
        conn
    }

    /// Was `#[ignore]`d: `spend_layer` and `travel_layer` named
    /// `finance.transaction_projection` and `trips.plans` in another *database schema*,
    /// so an empty CI database failed at parse time (measured 2026-08-26, `relation
    /// "finance.transaction_projection" does not exist`). One file with table prefixes
    /// removes that: the neighbour tables are ordinary tables this test can create, so
    /// the layer SQL runs hermetically and the ignore is gone.
    #[test]
    fn the_three_layers_assemble_and_reconcile_against_an_empty_registry() {
        let (store, path) = open_test_store("layers");
        create_neighbour_tables(&path);

        // No links yet: the spend layer must reconcile to zero (PLC-5's shape:
        // total is the sum over linked rows, and none are linked here).
        let spend = spend_layer(&store).expect("spend layer SQL is valid");
        assert_eq!(spend["summary"]["total_cents"], 0);
        assert_eq!(spend["summary"]["linked"], 0);
        assert_eq!(spend["venues"]["type"], "FeatureCollection");
        assert_eq!(spend["cities"]["type"], "FeatureCollection");
        assert_eq!(spend["summary"]["transactions"], 0);

        let travel = travel_layer(&store, "2026-08-25").expect("travel layer SQL is valid");
        assert_eq!(travel["points"]["type"], "FeatureCollection");
        assert_eq!(travel["routes"]["type"], "FeatureCollection");

        let people = people_layer(&store, "2026-08-25").expect("people layer SQL is valid");
        assert_eq!(people["type"], "FeatureCollection");
        assert_eq!(people["features"].as_array().map(Vec::len), Some(0));
    }

    /// The cross-capability joins are the reason one database was chosen, so they are
    /// tested as joins and not only as valid SQL: a trip destination and a transit leg
    /// have to reach the map through the same file the registry lives in.
    #[test]
    fn the_travel_layer_joins_trips_and_transit_out_of_the_same_file() {
        let (store, path) = open_test_store("travel_join");
        let conn = create_neighbour_tables(&path);
        conn.execute(
            "INSERT INTO trips_plans VALUES ('plan-1',
                 '[{\"name\":\"Musterstadt\",\"latitude\":50.0,\"longitude\":7.0}]',
                 '2026-09-01', '2026-09-05', 'saved')",
            [],
        )
        .unwrap();
        // Archived plans are excluded, which is a predicate and not a filter in code.
        conn.execute(
            "INSERT INTO trips_plans VALUES ('plan-2',
                 '[{\"name\":\"Nowhere\",\"latitude\":1.0,\"longitude\":1.0}]',
                 '2026-09-01', '2026-09-05', 'archived')",
            [],
        )
        .unwrap();
        // Full HAFAS refs, so the leg carries its own coordinates and can be drawn
        // before the registry has either station.
        conn.execute(
            "INSERT INTO transit_trip_legs VALUES ('trip-1', 0,
                 'A=1@O=Koeln Hbf@X=6958730@Y=50943029@L=8000207@',  'Koeln Hbf',
                 'A=1@O=Bonn Hbf@X=7097000@Y=50732000@L=8000044@',   'Bonn Hbf',
                 '2026-09-01T09:00:00', 'ICE 1')",
            [],
        )
        .unwrap();

        let travel = travel_layer(&store, "2026-08-25").unwrap();
        let names: Vec<&str> = travel["points"]["features"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|f| f["properties"]["name"].as_str())
            .collect();
        assert!(names.contains(&"Musterstadt"), "got {names:?}");
        assert!(
            !names.contains(&"Nowhere"),
            "an archived plan reached the map"
        );
        assert_eq!(
            travel["routes"]["features"].as_array().map(Vec::len),
            Some(1),
            "the leg between two coordinate-carrying refs is one LineString"
        );
    }

    /// The B<->C review contract: groups over a scratch copy of the finance
    /// projection (synthetic rows only), then both assign paths.
    #[test]
    fn unplaced_groups_rank_by_total_and_assign_links_by_exact_description() {
        use crate::store::{stable_id, Place};

        let (store, path) = open_test_store("unplaced");
        let neighbours = create_neighbour_tables(&path);
        neighbours
            .execute_batch(
                "INSERT INTO finance_transaction_projection
                     (source_id, description, amount_cents, booked_at, kind, currency) VALUES
                 ('src-1', 'SYNTHETIC MARKET MUSTERSTADT', 300, '2026-08-01', 'expense', 'EUR'),
                 ('src-2', 'SYNTHETIC MARKET MUSTERSTADT', 700, '2026-08-03', 'expense', 'EUR'),
                 ('src-3', 'SYNTHETIC CAFE', 500, '2026-08-02', 'expense', 'EUR'),
                 ('src-4', 'SYNTHETIC SALARY', 9999, '2026-08-01', 'income', 'EUR'),
                 ('src-5', 'SYNTHETIC MARKET MUSTERSTADT', 400, '2026-08-04', 'expense', 'USD'),
                 (NULL, 'SYNTHETIC MARKET MUSTERSTADT', 100, '2026-08-05', 'expense', 'EUR')",
            )
            .unwrap();

        let groups = unplaced_groups(&store).unwrap();
        let groups = groups["groups"].as_array().unwrap().clone();
        assert_eq!(groups.len(), 2, "income, USD and NULL source_id stay out");
        assert_eq!(groups[0]["description"], "SYNTHETIC MARKET MUSTERSTADT");
        assert_eq!(groups[0]["transactions"], 2);
        assert_eq!(groups[0]["total_cents"], 1000);
        assert_eq!(groups[0]["first"], "2026-08-01");
        assert_eq!(groups[0]["last"], "2026-08-03");
        assert_eq!(groups[1]["description"], "SYNTHETIC CAFE");

        // The geocode_query path. Venue precision is requested, but the stub
        // resolves a city relation, so the guard must derive kind city and
        // write city-precision links — a city bubble never pretends to be a
        // venue (README D1).
        let (url, _hits) = crate::geocode::db_tests::stub(
            r#"[{"osm_type":"relation","osm_id":7002,"lat":"50.0","lon":"7.0","name":"Musterstadt","display_name":"Musterstadt, Germany","addresstype":"city","address":{"city":"Musterstadt","country_code":"de"}}]"#,
        );
        let geocoder = Geocoder::with_url(&store, url);
        let request = AssignUnplaced {
            description: "SYNTHETIC MARKET MUSTERSTADT".into(),
            place_id: None,
            geocode_query: Some("Musterstadt".into()),
            precision: "venue".into(),
        };
        let body = assign_unplaced(&store, &geocoder, "finance", &request, "2026-08-25")
            .unwrap()
            .expect("the stub resolves the city");
        assert_eq!(body["ok"], true);
        assert_eq!(body["linked"], 2, "both unlinked EUR expenses, exactly");
        assert_eq!(body["place"]["name"], "Musterstadt");
        assert_eq!(body["place"]["kind"], "city");
        assert_eq!(
            body["precision"], "city",
            "the venue request is downgraded to the place's own level (D1)"
        );

        // Idempotent: the same assignment again links nothing new.
        let again = assign_unplaced(&store, &geocoder, "finance", &request, "2026-08-25")
            .unwrap()
            .unwrap();
        assert_eq!(again["linked"], 0);

        // The place_id path against an existing registry row.
        let venue = Place {
            id: stable_id("place", "test:cafe"),
            name: "Synthetic Cafe".into(),
            kind: "venue".into(),
            address: None,
            city: Some("Musterstadt".into()),
            country_code: Some("DE".into()),
            latitude: Some(50.0),
            longitude: Some(7.0),
            source: "test".into(),
            external_ref: None,
        };
        store.upsert_place(&venue, "2026-08-25").unwrap();
        let request = AssignUnplaced {
            description: "SYNTHETIC CAFE".into(),
            place_id: Some(venue.id.clone()),
            geocode_query: None,
            precision: "venue".into(),
        };
        let body = assign_unplaced(&store, &geocoder, "finance", &request, "2026-08-25")
            .unwrap()
            .unwrap();
        assert_eq!(body["linked"], 1);
        assert_eq!(body["place"]["id"], venue.id.as_str());
        assert_eq!(body["precision"], "venue", "a venue-kind place keeps it");

        // Everything assigned: the review queue is empty.
        let groups = unplaced_groups(&store).unwrap();
        assert_eq!(groups["groups"].as_array().map(Vec::len), Some(0));
        {
            let conn = store.conn().unwrap();
            let manual: i64 = conn
                .query_row(
                    &format!(
                        "SELECT COUNT(*) FROM {}_transaction_places WHERE source = 'manual'",
                        store.prefix()
                    ),
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(manual, 3);
            let city_precision: i64 = conn
                .query_row(
                    &format!(
                        "SELECT COUNT(*) FROM {}_transaction_places WHERE precision = 'city'",
                        store.prefix()
                    ),
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(city_precision, 2, "the downgraded links are city rows");
        }

        // The spend layer reconciles over the same links, out of the same file.
        let spend = spend_layer(&store).unwrap();
        assert_eq!(spend["summary"]["linked"], 3);
        assert_eq!(spend["summary"]["total_cents"], 1500);

        // An unknown place id resolves nothing: the caller's 404, no write.
        let request = AssignUnplaced {
            description: "SYNTHETIC CAFE".into(),
            place_id: Some("place_missing".into()),
            geocode_query: None,
            precision: "venue".into(),
        };
        assert!(
            assign_unplaced(&store, &geocoder, "finance", &request, "2026-08-25")
                .unwrap()
                .is_none()
        );
    }
}
