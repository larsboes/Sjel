//! Nominatim client with a permanent local cache (README decision D3,
//! `upstreams.toml` [nominatim-api]).
//!
//! Three constraints are load-bearing and enforced here rather than remembered:
//!
//! - **A cache hit never egresses** (ISA PLC-2): the cache lookup happens
//!   before a client even exists for the request.
//! - **At most 1 request/s** (Nominatim usage policy): a process-global
//!   throttle spaces provider requests, whatever thread asks.
//! - **A query carries place text or a bare coordinate pair only** (PRD 6.1,
//!   ISA PLC-3/A1): the query type below has fields for street, postal code,
//!   city, country, free place text and a reverse-lookup coordinate pair —
//!   there is nowhere to put a person, an amount or a date.

use crate::store::{stable_id, Place, PlacesStore};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant};

type Fallible<T> = Result<T, Box<dyn std::error::Error>>;

/// The one place the provider endpoint is named. Everything else goes through
/// `nominatim_url()`, which honours the test/stub override.
pub const NOMINATIM_URL: &str = "https://nominatim.openstreetmap.org/search";

const PROVIDER: &str = "nominatim";
const MIN_REQUEST_SPACING: Duration = Duration::from_secs(1);

/// `SJEL_PLACES_NOMINATIM_URL` overrides the endpoint — the same
/// env-overridable-URL seam transit's hafas.rs uses, so tests point the real
/// client at a local stub instead of mocking it.
pub fn nominatim_url() -> String {
    sjel_config::env_var("SJEL_PLACES_NOMINATIM_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| NOMINATIM_URL.to_string())
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct StructuredQuery {
    pub street: Option<String>,
    pub postalcode: Option<String>,
    pub city: Option<String>,
    pub country: Option<String>,
}

/// What can be asked. Place text or a bare coordinate pair, by construction —
/// the two shapes README D3 permits on the wire.
#[derive(Debug, Clone, PartialEq)]
pub enum GeocodeQuery {
    Free(String),
    Structured(StructuredQuery),
    /// Reverse lookup: which locality is at this coordinate pair? The request
    /// carries the two numbers only.
    Reverse {
        latitude: f64,
        longitude: f64,
    },
}

impl GeocodeQuery {
    /// The canonical cache key text: trimmed, lower-cased, field-tagged. Two
    /// spellings of the same query hash to the same row, which is what keeps
    /// the at-most-once egress promise cheap.
    pub fn normalized(&self) -> String {
        fn norm(value: &str) -> String {
            value
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase()
        }
        match self {
            Self::Free(q) => format!("q={}", norm(q)),
            Self::Structured(s) => format!(
                "street={}|postalcode={}|city={}|country={}",
                norm(s.street.as_deref().unwrap_or("")),
                norm(s.postalcode.as_deref().unwrap_or("")),
                norm(s.city.as_deref().unwrap_or("")),
                norm(s.country.as_deref().unwrap_or("")),
            ),
            // Four decimals (~11 m): two near-identical coordinate spellings
            // share a cache row, the same purpose as the text normalization.
            Self::Reverse {
                latitude,
                longitude,
            } => format!("reverse={latitude:.4},{longitude:.4}"),
        }
    }

    pub fn hash(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(PROVIDER.as_bytes());
        hasher.update([0xff]);
        hasher.update(self.normalized().as_bytes());
        let digest = hasher.finalize();
        let mut encoded = String::with_capacity(64);
        for byte in digest {
            use std::fmt::Write;
            write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
        }
        encoded
    }

    /// Nothing to ask. Public so the HTTP handler can reject a blank query as
    /// the client's 400 before it ever reaches the geocoder.
    pub fn is_empty(&self) -> bool {
        match self {
            Self::Free(q) => q.trim().is_empty(),
            Self::Structured(s) => [&s.street, &s.postalcode, &s.city, &s.country]
                .iter()
                .all(|value| value.as_deref().is_none_or(|v| v.trim().is_empty())),
            Self::Reverse {
                latitude,
                longitude,
            } => !latitude.is_finite() || !longitude.is_finite(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct GeocodeOutcome {
    pub found: bool,
    pub cached: bool,
    pub place: Option<Place>,
}

pub struct Geocoder<'a> {
    store: &'a PlacesStore,
    url: String,
}

/// Last provider-request instant, process-global: two geocoders in one process
/// share the same public endpoint, so they share the same budget.
fn last_request() -> &'static std::sync::Mutex<Option<Instant>> {
    static LAST: std::sync::OnceLock<std::sync::Mutex<Option<Instant>>> =
        std::sync::OnceLock::new();
    LAST.get_or_init(|| std::sync::Mutex::new(None))
}

fn throttle() {
    let mut last = last_request()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(previous) = *last {
        let elapsed = previous.elapsed();
        if elapsed < MIN_REQUEST_SPACING {
            std::thread::sleep(MIN_REQUEST_SPACING - elapsed);
        }
    }
    *last = Some(Instant::now());
}

impl<'a> Geocoder<'a> {
    pub fn new(store: &'a PlacesStore) -> Self {
        Self::with_url(store, nominatim_url())
    }

    pub fn with_url(store: &'a PlacesStore, url: String) -> Self {
        Self { store, url }
    }

    /// Resolve a query. `kind_override` names the registry kind the caller
    /// already knows (`venue` for Amex addresses, `city` for city tokens);
    /// `None` derives it from the response.
    pub fn geocode(
        &self,
        query: &GeocodeQuery,
        kind_override: Option<&str>,
        today: &str,
    ) -> Fallible<GeocodeOutcome> {
        if query.is_empty() {
            return Err("geocode query must not be empty".into());
        }
        let hash = query.hash();

        // Cache first: a hit of any status answers without a network client
        // ever being built (PLC-2).
        if let Some(entry) = self.store.cache_get(&hash)? {
            if entry.status != "hit" {
                return Ok(GeocodeOutcome {
                    found: false,
                    cached: true,
                    place: None,
                });
            }
            let response: Value = entry
                .response
                .as_deref()
                .map(serde_json::from_str)
                .transpose()?
                .ok_or("cache row with status hit has no response")?;
            let place = self.register_place(&response, query, kind_override, today)?;
            return Ok(GeocodeOutcome {
                found: true,
                cached: true,
                place: Some(place),
            });
        }

        throttle();
        // The Nominatim usage policy requires an identifying User-Agent, and
        // sjel_http's is exactly the shape it asks for: a product name and a
        // contact URL, not a spoofed browser. (Contrast transit's hafas.rs,
        // which records why it does the opposite for an undocumented endpoint.)
        let client = sjel_http::client(
            sjel_http::Purpose::new("places-geocode"),
            Duration::from_secs(20),
        )?;
        let url = match query {
            GeocodeQuery::Reverse { .. } => reverse_endpoint(&self.url),
            _ => self.url.clone(),
        };
        let mut request = client
            .get(url)
            .query(&[("format", "jsonv2"), ("addressdetails", "1")]);
        match query {
            GeocodeQuery::Free(q) => request = request.query(&[("limit", "1"), ("q", q.trim())]),
            GeocodeQuery::Structured(s) => {
                request = request.query(&[("limit", "1")]);
                for (key, value) in [
                    ("street", &s.street),
                    ("postalcode", &s.postalcode),
                    ("city", &s.city),
                    ("country", &s.country),
                ] {
                    if let Some(value) = value.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
                        request = request.query(&[(key, value)]);
                    }
                }
            }
            // Coordinates only, rounded like the cache key. zoom=10 asks for a
            // city-level answer (the Nominatim reverse zoom table), which is
            // the locality-level name a coordinate-only proposal needs.
            GeocodeQuery::Reverse {
                latitude,
                longitude,
            } => {
                request = request.query(&[
                    ("zoom", "10"),
                    ("lat", format!("{latitude:.4}").as_str()),
                    ("lon", format!("{longitude:.4}").as_str()),
                ]);
            }
        }
        // `without_url`: reqwest's error Display appends the request URL, and
        // for a structured query that URL is the full street address. Backfill
        // errors reach stderr verbatim (main.rs), and an address in a log line
        // is exactly what D3's place-text-only constraint exists to prevent.
        let response = request.send().map_err(reqwest::Error::without_url)?;
        let status = response.status();
        if !status.is_success() {
            // Transport and provider errors are NOT cached: the cache is
            // permanent (D3), and a permanently cached 500 would turn one bad
            // minute into a place that can never be resolved.
            return Err(format!("nominatim answered {status}").into());
        }
        let body: Value = response.json().map_err(reqwest::Error::without_url)?;
        // Forward answers are an array; a reverse answer is one object, or
        // `{"error": ...}` when the provider knows nothing at the coordinate.
        let first = match query {
            GeocodeQuery::Reverse { .. } => (body.get("error").is_none()).then_some(body),
            _ => body.as_array().and_then(|items| items.first()).cloned(),
        };

        let Some(item) = first else {
            self.store.cache_put(
                &hash,
                PROVIDER,
                &query.normalized(),
                None,
                None,
                "miss",
                today,
            )?;
            return Ok(GeocodeOutcome {
                found: false,
                cached: false,
                place: None,
            });
        };
        let place = self.register_place(&item, query, kind_override, today)?;
        self.store.cache_put(
            &hash,
            PROVIDER,
            &query.normalized(),
            Some(&item.to_string()),
            Some(&place.id),
            "hit",
            today,
        )?;
        Ok(GeocodeOutcome {
            found: true,
            cached: false,
            place: Some(place),
        })
    }

    /// Cached reverse geocode: the locality name at a coordinate pair. Same
    /// cache, throttle and registry path as a forward query; the provider
    /// request carries the two numbers only (README D3).
    pub fn reverse(&self, latitude: f64, longitude: f64, today: &str) -> Fallible<GeocodeOutcome> {
        self.geocode(
            &GeocodeQuery::Reverse {
                latitude,
                longitude,
            },
            None,
            today,
        )
    }

    /// Turn one provider item into a registry row and make sure it exists.
    /// Rebuilt from the stored response on cache hits too, so a place deleted
    /// or a fresh database self-heals without egress.
    fn register_place(
        &self,
        item: &Value,
        query: &GeocodeQuery,
        kind_override: Option<&str>,
        today: &str,
    ) -> Fallible<Place> {
        let derived = derive_place(item, query, kind_override)
            .ok_or("nominatim response carries no usable coordinates")?;
        self.store.upsert_place(&derived, today)?;
        Ok(self.store.place(&derived.id)?.unwrap_or(derived))
    }
}

/// The reverse endpoint beside the configured search endpoint. The test stubs
/// answer every path, so a URL without the `/search` suffix simply gains
/// `/reverse`.
fn reverse_endpoint(search_url: &str) -> String {
    let base = search_url.trim_end_matches('/');
    match base.strip_suffix("/search") {
        Some(root) => format!("{root}/reverse"),
        None => format!("{base}/reverse"),
    }
}

fn as_f64(value: Option<&Value>) -> Option<f64> {
    match value? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

/// Pure response-to-place mapping, testable without a network or a database.
pub fn derive_place(
    item: &Value,
    query: &GeocodeQuery,
    kind_override: Option<&str>,
) -> Option<Place> {
    let latitude = as_f64(item.get("lat"))?;
    let longitude = as_f64(item.get("lon"))?;
    let display_name = item
        .get("display_name")
        .and_then(Value::as_str)
        .unwrap_or("");
    let name = item
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.trim().is_empty())
        .map(str::to_string)
        .or_else(|| {
            display_name
                .split(',')
                .next()
                .map(str::trim)
                .filter(|part| !part.is_empty())
                .map(str::to_string)
        })?;
    let address = item.get("address");
    let city = ["city", "town", "village", "municipality", "hamlet"]
        .iter()
        .find_map(|key| address.and_then(|a| a.get(key)).and_then(Value::as_str))
        .map(str::to_string);
    let country_code = address
        .and_then(|a| a.get("country_code"))
        .and_then(Value::as_str)
        .map(str::to_uppercase);
    let addresstype = item
        .get("addresstype")
        .and_then(Value::as_str)
        .unwrap_or("");
    let kind = kind_override
        .map(str::to_string)
        .unwrap_or_else(|| match addresstype {
            "city" | "town" | "village" | "municipality" | "hamlet" => "city".into(),
            _ => match query {
                GeocodeQuery::Structured(s) if s.street.is_some() => "address".into(),
                // zoom=10 answers are locality-level whatever OSM calls the
                // boundary (`administrative` is common), so city, not venue.
                GeocodeQuery::Reverse { .. } => "city".into(),
                _ => "venue".into(),
            },
        });
    let external_ref = match (
        item.get("osm_type").and_then(Value::as_str),
        item.get("osm_id"),
    ) {
        (Some(osm_type), Some(osm_id)) if !osm_type.is_empty() => {
            Some(format!("osm:{osm_type}/{osm_id}"))
        }
        _ => None,
    };
    let identity = external_ref
        .clone()
        .unwrap_or_else(|| format!("geocode:{}", query.normalized()));
    let street = match query {
        GeocodeQuery::Structured(s) => s.street.clone(),
        _ => None,
    };
    Some(Place {
        id: stable_id("place", &identity),
        name,
        kind,
        address: street,
        city,
        country_code,
        latitude: Some(latitude),
        longitude: Some(longitude),
        source: "nominatim".into(),
        external_ref,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn two_spellings_of_one_query_share_a_cache_row() {
        let a = GeocodeQuery::Free("  MARKT   Musterstadt ".into());
        let b = GeocodeQuery::Free("markt musterstadt".into());
        assert_eq!(a.hash(), b.hash());
        let c = GeocodeQuery::Structured(StructuredQuery {
            street: Some("Beispielstr. 1".into()),
            postalcode: Some("12345".into()),
            city: Some("Musterstadt".into()),
            country: Some("Germany".into()),
        });
        assert_ne!(a.hash(), c.hash());
        assert!(GeocodeQuery::Free("   ".into()).is_empty());
        assert!(!c.is_empty());
    }

    #[test]
    fn nearby_reverse_queries_share_a_cache_row_and_the_endpoint_swaps_cleanly() {
        let a = GeocodeQuery::Reverse {
            latitude: 50.123_44,
            longitude: 7.123_39,
        };
        let b = GeocodeQuery::Reverse {
            latitude: 50.123_41,
            longitude: 7.123_42,
        };
        // Both round to the same four decimals, so one provider request serves
        // every re-run over the same note.
        assert_eq!(a.hash(), b.hash());
        let elsewhere = GeocodeQuery::Reverse {
            latitude: 51.0,
            longitude: 7.0,
        };
        assert_ne!(a.hash(), elsewhere.hash());
        assert!(!a.is_empty());
        assert!(GeocodeQuery::Reverse {
            latitude: f64::NAN,
            longitude: 7.0,
        }
        .is_empty());

        assert_eq!(
            reverse_endpoint("https://nominatim.openstreetmap.org/search"),
            "https://nominatim.openstreetmap.org/reverse"
        );
        assert_eq!(
            reverse_endpoint("http://127.0.0.1:9/search"),
            "http://127.0.0.1:9/reverse"
        );
        assert_eq!(
            reverse_endpoint("http://127.0.0.1:9"),
            "http://127.0.0.1:9/reverse"
        );
    }

    #[test]
    fn a_reverse_item_derives_a_city_kind_place() {
        // zoom=10 boundaries usually come back as `administrative`, which must
        // still land as a city-kind registry row, never a venue.
        let item = json!({
            "osm_type": "relation",
            "osm_id": 4242,
            "lat": "50.1",
            "lon": "7.1",
            "name": "Musterstadt",
            "display_name": "Musterstadt, Germany",
            "addresstype": "administrative",
            "address": {"country_code": "de"}
        });
        let query = GeocodeQuery::Reverse {
            latitude: 50.1234,
            longitude: 7.1234,
        };
        let place = derive_place(&item, &query, None).unwrap();
        assert_eq!(place.kind, "city");
        assert_eq!(place.name, "Musterstadt");
        assert_eq!(place.external_ref.as_deref(), Some("osm:relation/4242"));
        assert_eq!(place.address, None);
    }

    #[test]
    fn a_response_item_becomes_a_stable_registry_row() {
        let item = json!({
            "osm_type": "node",
            "osm_id": 123456,
            "lat": "50.0001",
            "lon": "8.0001",
            "name": "Synthetic Market",
            "display_name": "Synthetic Market, Musterstadt, Germany",
            "addresstype": "shop",
            "address": {"town": "Musterstadt", "country_code": "de"}
        });
        let query = GeocodeQuery::Structured(StructuredQuery {
            street: Some("Beispielstr. 1".into()),
            postalcode: Some("12345".into()),
            city: Some("Musterstadt".into()),
            country: Some("Germany".into()),
        });
        let place = derive_place(&item, &query, Some("venue")).unwrap();
        assert_eq!(place.kind, "venue");
        assert_eq!(place.city.as_deref(), Some("Musterstadt"));
        assert_eq!(place.country_code.as_deref(), Some("DE"));
        assert_eq!(place.external_ref.as_deref(), Some("osm:node/123456"));
        // Same OSM identity, same row — that is what makes backfills idempotent.
        let again =
            derive_place(&item, &GeocodeQuery::Free("synthetic market".into()), None).unwrap();
        assert_eq!(place.id, again.id);

        let city_item = json!({
            "lat": 48.2085, "lon": 16.3721, "name": "Vienna",
            "addresstype": "city", "address": {"city": "Vienna", "country_code": "at"}
        });
        let derived = derive_place(&city_item, &GeocodeQuery::Free("vienna".into()), None).unwrap();
        assert_eq!(derived.kind, "city");
        assert!(derived.external_ref.is_none());
    }
}

/// Cache behaviour against a real store and a local stub endpoint — the
/// env-overridable-URL pattern from transit, with the URL passed explicitly so
/// parallel tests never race on process env.
#[cfg(test)]
pub(crate) mod db_tests {
    use super::*;
    use crate::store::db_tests::open_test_store;
    use std::io::{Read, Write};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// Minimal HTTP stub: answers every request with what `router` returns for
    /// the raw request text, counts requests. `pub(crate)` so the backfill and
    /// layer tests reuse it instead of growing their own.
    pub(crate) fn stub_with(
        router: impl Fn(&str) -> String + Send + 'static,
    ) -> (String, Arc<AtomicUsize>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind stub");
        let addr = listener.local_addr().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = hits.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                counter.fetch_add(1, Ordering::SeqCst);
                let mut buffer = [0_u8; 4096];
                let read = stream.read(&mut buffer).unwrap_or(0);
                let request = String::from_utf8_lossy(&buffer[..read]);
                let body = router(&request);
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len(),
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        (format!("http://{addr}/search"), hits)
    }

    /// The common case: every request gets the same answer.
    pub(crate) fn stub(body: &'static str) -> (String, Arc<AtomicUsize>) {
        stub_with(move |_| body.to_string())
    }

    #[test]
    fn a_repeated_query_is_served_from_the_cache_without_egress() {
        let (store, _path) = open_test_store("geocache");
        let (url, hits) = stub(
            r#"[{"osm_type":"node","osm_id":42,"lat":"50.0","lon":"7.0","name":"Synthetic Market","display_name":"Synthetic Market, Synthetic Town","addresstype":"shop","address":{"town":"Synthetic Town","country_code":"de"}}]"#,
        );
        let geocoder = Geocoder::with_url(&store, url);
        let query = GeocodeQuery::Free("synthetic market".into());

        let first = geocoder
            .geocode(&query, Some("venue"), "2026-08-25")
            .unwrap();
        assert!(first.found);
        assert!(!first.cached);
        assert_eq!(hits.load(Ordering::SeqCst), 1);

        let second = geocoder
            .geocode(&query, Some("venue"), "2026-08-25")
            .unwrap();
        assert!(second.found);
        assert!(second.cached, "second identical query must be a cache hit");
        assert_eq!(
            hits.load(Ordering::SeqCst),
            1,
            "a cache hit must not produce a provider request (PLC-2)"
        );
        assert_eq!(first.place.as_ref().unwrap().id, second.place.unwrap().id);
    }

    /// A transport failure's message reaches backfill stderr verbatim
    /// (main.rs), so it must not carry the request URL — for a structured
    /// query that URL is the full street address (D3: place text stays out of
    /// logs too).
    #[test]
    fn a_transport_error_never_carries_the_query_address() {
        let (store, _path) = open_test_store("geoerr");
        // Port 1 on loopback: nothing listens, the connection is refused.
        let geocoder = Geocoder::with_url(&store, "http://127.0.0.1:1/search".into());
        let query = GeocodeQuery::Structured(StructuredQuery {
            street: Some("Synthetic Street 7".into()),
            postalcode: Some("99999".into()),
            city: Some("Synthetic Town".into()),
            country: Some("Germany".into()),
        });
        let error = geocoder
            .geocode(&query, Some("venue"), "2026-08-25")
            .expect_err("a refused connection must surface as an error")
            .to_string();
        assert!(
            !error.contains("Synthetic") && !error.contains("99999"),
            "transport error must not leak the address query: {error}"
        );
    }

    #[test]
    fn a_reverse_lookup_is_cached_like_a_forward_one() {
        let (store, _path) = open_test_store("georev");
        // A reverse answer is one object, not an array.
        let (url, hits) = stub(
            r#"{"osm_type":"relation","osm_id":4242,"lat":"50.10","lon":"7.10","name":"Musterstadt","display_name":"Musterstadt, Germany","addresstype":"city","address":{"city":"Musterstadt","country_code":"de"}}"#,
        );
        let geocoder = Geocoder::with_url(&store, url);

        let first = geocoder.reverse(50.123_44, 7.123_39, "2026-08-25").unwrap();
        assert!(first.found);
        assert!(!first.cached);
        let place = first.place.expect("a locality answer registers a place");
        assert_eq!(place.name, "Musterstadt");
        assert_eq!(place.kind, "city");
        assert_eq!(hits.load(Ordering::SeqCst), 1);

        // A nearby coordinate rounds to the same cache row: no second egress.
        let second = geocoder.reverse(50.123_41, 7.123_42, "2026-08-25").unwrap();
        assert!(second.cached, "the rounded repeat must be a cache hit");
        assert_eq!(second.place.unwrap().id, place.id);
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_reverse_error_answer_is_a_cached_miss() {
        let (store, _path) = open_test_store("georevmiss");
        let (url, hits) = stub(r#"{"error":"Unable to geocode"}"#);
        let geocoder = Geocoder::with_url(&store, url);
        let first = geocoder.reverse(0.1234, 0.1234, "2026-08-25").unwrap();
        assert!(!first.found);
        assert!(first.place.is_none());
        let second = geocoder.reverse(0.1234, 0.1234, "2026-08-25").unwrap();
        assert!(second.cached);
        assert_eq!(
            hits.load(Ordering::SeqCst),
            1,
            "a cached miss never re-asks"
        );
    }

    #[test]
    fn an_empty_provider_answer_is_cached_as_a_miss() {
        let (store, _path) = open_test_store("geomiss");
        let (url, hits) = stub("[]");
        let geocoder = Geocoder::with_url(&store, url);
        let query = GeocodeQuery::Free("synthetic nowhere 4029357733".into());

        let first = geocoder.geocode(&query, None, "2026-08-25").unwrap();
        assert!(!first.found);
        assert!(!first.cached);
        let second = geocoder.geocode(&query, None, "2026-08-25").unwrap();
        assert!(!second.found);
        assert!(second.cached);
        assert_eq!(
            hits.load(Ordering::SeqCst),
            1,
            "a cached miss never re-asks"
        );
    }
}
