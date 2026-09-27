//! Flight search against Kiwi.com's open MCP endpoint.
//!
//! `mcp.kiwi.com` answers unauthenticated JSON-RPC over HTTPS with SSE framing
//! (verified live 2026-08-12: `serverInfo` = kiwicom-flight-search 1.28.1, no
//! session state required). No key, no SDK: one POST per search, one `data:`
//! line back. It publishes no rate limit, quota, or acceptable-use policy, so
//! this module enforces its own restraint (a fixed pause before every request)
//! and callers should treat withdrawal of the endpoint as a matter of when.
//!
//! Two verified response hazards shape the types here:
//! - Timestamps are **naive local with no UTC offset** (a CGN 08:15 -> STN
//!   08:35 hop reads as 20 minutes while `durationSeconds` says 4800). Every
//!   segment therefore carries `departure_utc`/`arrival_utc`, resolved via
//!   station-time from the country name the API serves next to each airport --
//!   absent when the zone is unknown, never guessed.
//! - `route[]` is **lossy**: an itinerary with segments CGN->STN then LGW->BCN
//!   reports `["CGN","STN","BCN"]`, silently hiding a 75 km ground transfer.
//!   Segments are the truth; `hidden_ground_transfers` surfaces every airport
//!   change `route[]` would have swallowed.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const MCP_URL: &str = "https://mcp.kiwi.com/";
/// Self-imposed pause before every request. The endpoint publishes no limit;
/// this is the restraint that keeps a flexible-date sweep from looking like an
/// attack.
const REQUEST_PAUSE_MS: u64 = 500;

#[derive(Debug, thiserror::Error)]
pub enum KiwiError {
    #[error("kiwi request failed: {0}")]
    Request(String),
    #[error("kiwi returned HTTP {0}")]
    BadStatus(u16),
    #[error("kiwi response unparseable: {0}")]
    Parse(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FlightSegment {
    pub from: String,
    pub to: String,
    pub from_city: String,
    pub to_city: String,
    pub from_country: String,
    pub to_country: String,
    /// Naive airport-local wall clock, exactly as the API serves it.
    pub departure_time: String,
    pub arrival_time: String,
    /// The same instants in UTC ("...Z"), via station-time's country lookup
    /// with island-airport exceptions. `None` when the zone is unknown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub departure_utc: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arrival_utc: Option<String>,
    pub duration_seconds: u64,
    pub carrier: String,
    pub carrier_name: String,
    pub flight_number: String,
    pub cabin_class: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FlightLeg {
    /// The API's own airport chain. Kept for display only: it is lossy, see
    /// the module doc. Never do connection logic on it.
    pub route: Vec<String>,
    pub departure_time: String,
    pub arrival_time: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub departure_utc: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arrival_utc: Option<String>,
    pub duration_seconds: u64,
    pub stops: u64,
    pub cabin_class: String,
    pub segments: Vec<FlightSegment>,
    /// Airport changes between consecutive segments that `route[]` hides:
    /// `("STN", "LGW")` means you land at one London airport and depart from
    /// another on your own feet and your own risk.
    pub hidden_ground_transfers: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FlightOption {
    pub id: String,
    pub price: f64,
    pub price_formatted: String,
    pub booking_url: String,
    pub total_duration_seconds: u64,
    /// Included baggage counts as served (`personalItem`/`cabinBag`/
    /// `checkedBag`); kept as provider evidence rather than modeled.
    pub baggage: Value,
    pub outbound: FlightLeg,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inbound: Option<FlightLeg>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FlightSearchResult {
    pub currency: String,
    pub results_count: u64,
    pub options: Vec<FlightOption>,
}

/// One day of a flexible-date grid: the cheapest option departing that day.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GridDay {
    pub date: String,
    pub price: f64,
    pub price_formatted: String,
    pub option_id: String,
    pub booking_url: String,
    /// True when this option's legs hide a ground transfer -- cheap days won
    /// by a self-transfer should say so before anyone books them.
    pub has_hidden_ground_transfer: bool,
}

/// Cheapest-per-departure-day over a search result. This is the 40-54% axis
/// from the PRD measurements: date flexibility moved a CGN->LIS fare from
/// €223 fixed to €103 across a month, while provider breadth moved it 0.3%.
pub fn cheapest_per_day(result: &FlightSearchResult) -> Vec<GridDay> {
    let mut by_day: std::collections::BTreeMap<String, GridDay> = std::collections::BTreeMap::new();
    for option in &result.options {
        let Some(date) = option.outbound.departure_time.get(..10) else {
            continue;
        };
        let candidate = GridDay {
            date: date.to_string(),
            price: option.price,
            price_formatted: option.price_formatted.clone(),
            option_id: option.id.clone(),
            booking_url: option.booking_url.clone(),
            has_hidden_ground_transfer: !option.outbound.hidden_ground_transfers.is_empty()
                || option
                    .inbound
                    .as_ref()
                    .is_some_and(|leg| !leg.hidden_ground_transfers.is_empty()),
        };
        match by_day.get(date) {
            Some(existing) if existing.price <= candidate.price => {}
            _ => {
                by_day.insert(date.to_string(), candidate);
            }
        }
    }
    by_day.into_values().collect()
}

pub struct KiwiClient {
    client: reqwest::blocking::Client,
}

impl Default for KiwiClient {
    fn default() -> Self {
        Self::new()
    }
}

impl KiwiClient {
    pub fn new() -> Self {
        let client = sjel_http::client(
            sjel_http::Purpose::new("trips-kiwi"),
            std::time::Duration::from_secs(30),
        )
        .expect("reqwest client with a fixed timeout should always build");
        Self { client }
    }

    /// One flight search. `date` is ISO (YYYY-MM-DD); `flex_days` widens the
    /// search +/- that many days (the 40-54% price axis); `return_date` makes
    /// it a return search.
    pub fn search(
        &self,
        from: &str,
        to: &str,
        date: &str,
        flex_days: u8,
        return_date: Option<&str>,
    ) -> Result<FlightSearchResult, KiwiError> {
        let mut arguments = json!({
            "flyFrom": from,
            "flyTo": to,
            "departureDate": iso_to_ddmmyyyy(date)
                .ok_or_else(|| KiwiError::Parse(format!("not an ISO date: {date}")))?,
        });
        if flex_days > 0 {
            arguments["departureDateFlexDays"] = json!(flex_days.min(10));
        }
        if let Some(rd) = return_date {
            arguments["returnDate"] = json!(iso_to_ddmmyyyy(rd)
                .ok_or_else(|| KiwiError::Parse(format!("not an ISO date: {rd}")))?);
        }
        let rpc = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": "search-flight", "arguments": arguments }
        });

        std::thread::sleep(std::time::Duration::from_millis(REQUEST_PAUSE_MS));
        let response = self
            .client
            .post(MCP_URL)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .json(&rpc)
            .send()
            .map_err(|e| KiwiError::Request(e.to_string()))?;
        let status = response.status();
        let body = response
            .text()
            .map_err(|e| KiwiError::Request(e.to_string()))?;
        if !status.is_success() {
            return Err(KiwiError::BadStatus(status.as_u16()));
        }
        parse_search_response(&body)
    }
}

/// "2026-08-20" -> "20/08/2026", the format the tool schema demands.
fn iso_to_ddmmyyyy(iso: &str) -> Option<String> {
    let mut parts = iso.splitn(3, '-');
    let (y, m, d) = (parts.next()?, parts.next()?, parts.next()?);
    if y.len() != 4 || m.len() != 2 || d.len() != 2 {
        return None;
    }
    Some(format!("{d}/{m}/{y}"))
}

/// The SSE body carries exactly one `data:` line with the JSON-RPC envelope;
/// the search result itself sits in `result.structuredContent`, with
/// `result.content[0].text` carrying the same JSON as a string fallback.
fn parse_search_response(sse_body: &str) -> Result<FlightSearchResult, KiwiError> {
    let data_line = sse_body
        .lines()
        .find_map(|line| line.strip_prefix("data: "))
        .ok_or_else(|| KiwiError::Parse("no `data:` line in SSE response".into()))?;
    let envelope: Value = serde_json::from_str(data_line)
        .map_err(|e| KiwiError::Parse(format!("data line is not JSON: {e}")))?;
    if let Some(error) = envelope.get("error") {
        return Err(KiwiError::Parse(format!("JSON-RPC error: {error}")));
    }
    let result = envelope
        .get("result")
        .ok_or_else(|| KiwiError::Parse("no result in envelope".into()))?;
    let payload: Value = match result.get("structuredContent") {
        Some(sc) if !sc.is_null() => sc.clone(),
        _ => {
            let text = result
                .pointer("/content/0/text")
                .and_then(|t| t.as_str())
                .ok_or_else(|| {
                    KiwiError::Parse("neither structuredContent nor content text".into())
                })?;
            serde_json::from_str(text)
                .map_err(|e| KiwiError::Parse(format!("content text is not JSON: {e}")))?
        }
    };
    Ok(FlightSearchResult {
        currency: str_of(&payload, "currency"),
        results_count: payload
            .get("resultsCount")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        options: payload
            .get("itineraries")
            .and_then(|i| i.as_array())
            .map(|arr| arr.iter().map(parse_option).collect())
            .unwrap_or_default(),
    })
}

fn parse_option(v: &Value) -> FlightOption {
    FlightOption {
        id: str_of(v, "id"),
        price: v.get("price").and_then(|p| p.as_f64()).unwrap_or(0.0),
        price_formatted: str_of(v, "priceFormatted"),
        booking_url: str_of(v, "bookingUrl"),
        total_duration_seconds: v
            .get("totalDurationSeconds")
            .and_then(|d| d.as_u64())
            .unwrap_or(0),
        baggage: v.get("baggage").cloned().unwrap_or(Value::Null),
        outbound: v.get("outbound").map(parse_leg).unwrap_or_else(empty_leg),
        inbound: v.get("inbound").filter(|i| !i.is_null()).map(parse_leg),
    }
}

fn parse_leg(v: &Value) -> FlightLeg {
    let segments: Vec<FlightSegment> = v
        .get("segments")
        .and_then(|s| s.as_array())
        .map(|arr| arr.iter().map(parse_segment).collect())
        .unwrap_or_default();
    let hidden_ground_transfers = segments
        .windows(2)
        .filter(|w| w[0].to != w[1].from)
        .map(|w| (w[0].to.clone(), w[1].from.clone()))
        .collect();
    let (departure_utc, arrival_utc) = (
        segments.first().and_then(|s| s.departure_utc.clone()),
        segments.last().and_then(|s| s.arrival_utc.clone()),
    );
    FlightLeg {
        route: v
            .get("route")
            .and_then(|r| r.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|x| x.as_str())
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default(),
        departure_time: str_of(v, "departureTime"),
        arrival_time: str_of(v, "arrivalTime"),
        departure_utc,
        arrival_utc,
        duration_seconds: v
            .get("durationSeconds")
            .and_then(|d| d.as_u64())
            .unwrap_or(0),
        stops: v.get("stops").and_then(|s| s.as_u64()).unwrap_or(0),
        cabin_class: str_of(v, "cabinClass"),
        segments,
        hidden_ground_transfers,
    }
}

fn parse_segment(v: &Value) -> FlightSegment {
    let from = str_of(v, "from");
    let to = str_of(v, "to");
    let from_country = str_of(v, "fromCountry");
    let to_country = str_of(v, "toCountry");
    let departure_time = str_of(v, "departureTime");
    let arrival_time = str_of(v, "arrivalTime");
    let departure_utc = station_time::rfc3339_utc_airport(&departure_time, &from, &from_country);
    let arrival_utc = station_time::rfc3339_utc_airport(&arrival_time, &to, &to_country);
    FlightSegment {
        from,
        to,
        from_city: str_of(v, "fromCity"),
        to_city: str_of(v, "toCity"),
        from_country,
        to_country,
        departure_time,
        arrival_time,
        departure_utc,
        arrival_utc,
        duration_seconds: v
            .get("durationSeconds")
            .and_then(|d| d.as_u64())
            .unwrap_or(0),
        carrier: str_of(v, "carrier"),
        carrier_name: str_of(v, "carrierName"),
        flight_number: str_of(v, "flightNumber"),
        cabin_class: str_of(v, "cabinClass"),
    }
}

fn empty_leg() -> FlightLeg {
    FlightLeg {
        route: Vec::new(),
        departure_time: String::new(),
        arrival_time: String::new(),
        departure_utc: None,
        arrival_utc: None,
        duration_seconds: 0,
        stops: 0,
        cabin_class: String::new(),
        segments: Vec::new(),
        hidden_ground_transfers: Vec::new(),
    }
}

fn str_of(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Trimmed from a real `mcp.kiwi.com` response captured 2026-08-12
    /// (CGN->STN, Ryanair FR2353). The naive 08:15 -> 08:35 pair with
    /// durationSeconds 4800 is the verified timestamp hazard.
    fn fixture_envelope(itinerary: Value) -> String {
        let envelope = json!({
            "jsonrpc": "2.0", "id": 3,
            "result": { "structuredContent": {
                "currency": "EUR",
                "resultsCount": 1,
                "itineraries": [itinerary]
            }}
        });
        format!("event: message\ndata: {envelope}\n\n")
    }

    fn cgn_stn_itinerary() -> Value {
        json!({
            "id": "it-1",
            "price": 50.0,
            "priceFormatted": "50 EUR",
            "bookingUrl": "https://www.kiwi.com/deep?booking=1",
            "totalDurationSeconds": 4800,
            "baggage": { "personalItem": 1, "cabinBag": 0, "checkedBag": 0 },
            "outbound": {
                "route": ["CGN", "STN"],
                "departureTime": "2026-08-20T08:15:00",
                "arrivalTime": "2026-08-20T08:35:00",
                "durationSeconds": 4800,
                "stops": 0,
                "cabinClass": "Economy",
                "segments": [{
                    "from": "CGN", "to": "STN",
                    "fromCity": "Cologne", "toCity": "London",
                    "fromName": "Cologne Bonn Airport", "toName": "London Stansted",
                    "fromCountry": "Germany", "toCountry": "United Kingdom",
                    "departureTime": "2026-08-20T08:15:00",
                    "arrivalTime": "2026-08-20T08:35:00",
                    "durationSeconds": 4800,
                    "carrier": "FR", "carrierName": "Ryanair",
                    "flightNumber": "FR2353", "cabinClass": "Economy"
                }]
            }
        })
    }

    #[test]
    fn a_naive_20_minute_hop_resolves_to_its_true_80_utc_minutes() {
        let parsed = parse_search_response(&fixture_envelope(cgn_stn_itinerary())).unwrap();
        assert_eq!(parsed.results_count, 1);
        let leg = &parsed.options[0].outbound;
        // Naive strings survive untouched for display.
        assert_eq!(leg.departure_time, "2026-08-20T08:15:00");
        assert_eq!(leg.arrival_time, "2026-08-20T08:35:00");
        // The UTC pair carries the truth: 06:15Z -> 07:35Z is the API's own 4800s.
        assert_eq!(leg.departure_utc.as_deref(), Some("2026-08-20T06:15:00Z"));
        assert_eq!(leg.arrival_utc.as_deref(), Some("2026-08-20T07:35:00Z"));
        assert!(leg.hidden_ground_transfers.is_empty());
    }

    /// The verified `route[]` lossiness: segments CGN->STN then LGW->BCN come
    /// back as route ["CGN","STN","BCN"], hiding the Stansted-to-Gatwick
    /// ground transfer. The parser surfaces it from segments.
    #[test]
    fn a_hidden_ground_transfer_is_surfaced_from_segments_not_route() {
        let mut itinerary = cgn_stn_itinerary();
        itinerary["outbound"]["route"] = json!(["CGN", "STN", "BCN"]);
        itinerary["outbound"]["segments"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "from": "LGW", "to": "BCN",
                "fromCity": "London", "toCity": "Barcelona",
                "fromCountry": "United Kingdom", "toCountry": "Spain",
                "departureTime": "2026-08-20T14:00:00",
                "arrivalTime": "2026-08-20T17:10:00",
                "durationSeconds": 7800,
                "carrier": "VY", "carrierName": "Vueling",
                "flightNumber": "VY7821", "cabinClass": "Economy"
            }));
        let parsed = parse_search_response(&fixture_envelope(itinerary)).unwrap();
        let leg = &parsed.options[0].outbound;
        assert_eq!(
            leg.hidden_ground_transfers,
            vec![("STN".to_string(), "LGW".to_string())]
        );
    }

    #[test]
    fn the_grid_keeps_the_cheapest_option_per_day_and_flags_self_transfers() {
        let mut day_one = cgn_stn_itinerary();
        day_one["id"] = json!("a");
        day_one["price"] = json!(80.0);
        let mut day_one_cheaper = cgn_stn_itinerary();
        day_one_cheaper["id"] = json!("b");
        day_one_cheaper["price"] = json!(50.0);
        let mut day_two = cgn_stn_itinerary();
        day_two["id"] = json!("c");
        day_two["price"] = json!(65.0);
        day_two["outbound"]["departureTime"] = json!("2026-08-21T08:15:00");
        // The cheap day-two option rides a hidden self-transfer; the grid says so.
        day_two["outbound"]["segments"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "from": "LGW", "to": "BCN",
                "fromCountry": "United Kingdom", "toCountry": "Spain",
                "departureTime": "2026-08-21T14:00:00",
                "arrivalTime": "2026-08-21T17:10:00",
                "durationSeconds": 7800,
                "carrier": "VY", "carrierName": "Vueling",
                "flightNumber": "VY7821", "cabinClass": "Economy"
            }));

        let envelope = json!({
            "jsonrpc": "2.0", "id": 3,
            "result": { "structuredContent": {
                "currency": "EUR", "resultsCount": 3,
                "itineraries": [day_one, day_one_cheaper, day_two]
            }}
        });
        let parsed = parse_search_response(&format!("data: {envelope}\n")).unwrap();
        let grid = cheapest_per_day(&parsed);
        assert_eq!(grid.len(), 2);
        assert_eq!((grid[0].date.as_str(), grid[0].price), ("2026-08-20", 50.0));
        assert!(!grid[0].has_hidden_ground_transfer);
        assert_eq!((grid[1].date.as_str(), grid[1].price), ("2026-08-21", 65.0));
        assert!(grid[1].has_hidden_ground_transfer);
    }

    #[test]
    fn dates_convert_to_the_schema_format_or_refuse() {
        assert_eq!(iso_to_ddmmyyyy("2026-08-20").as_deref(), Some("20/08/2026"));
        assert_eq!(iso_to_ddmmyyyy("20/08/2026"), None);
        assert_eq!(iso_to_ddmmyyyy("2026-8-20"), None);
    }

    #[test]
    fn a_json_rpc_error_is_an_error_not_an_empty_result() {
        let body = "data: {\"jsonrpc\":\"2.0\",\"id\":1,\"error\":{\"code\":-32000,\"message\":\"boom\"}}\n";
        assert!(matches!(
            parse_search_response(body),
            Err(KiwiError::Parse(_))
        ));
    }
}
