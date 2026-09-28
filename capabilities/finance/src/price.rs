//! Market prices: what was observed, by whom, and what every attempt did.
//!
//! Two rules govern every provider in this file, and both were bought with a
//! measurement rather than reasoned from taste.
//!
//! **A response is validated by the shape of its body, never by its status
//! code.** Measured 2026-09-05: `GET https://stooq.com/q/d/l/?s=spy.us&i=d`
//! answers **HTTP 200** with a 796-byte HTML body carrying a JavaScript
//! proof-of-work loop. A client that trusts the 200 parses HTML as CSV and
//! writes garbage into an append-only table nothing can correct. So a CSV body
//! whose first line is not the expected header is a `refused` fetch with a named
//! detail, whatever the status was. `upstreams.toml` carries the reject rows for
//! stooq and tradegate so the measurement outlives the memory of it.
//!
//! **A per-instrument failure is a recorded row, never a fatal run.** Every
//! attempt writes a `finance_price_fetches` row with a status, modelled on
//! `capabilities/places/src/store.rs`'s geocode cache. One instrument refusing
//! must not cost the other twelve their prices.
//!
//! A fetched price is **never** written back into the reviewed holdings
//! snapshot. `investment::validate_source_snapshot` recomputes the content hash
//! over `latest_unit_price`, so a quote written there would make the file refuse
//! itself on the next read.
//!
//! These tables are NOT `finance_price_points`, which is Axon's own subscription
//! pricing history (PRD §9.2 dogfooding). Market data and what Axon pays for a
//! streaming service are two different series that happen to share a word.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::Serialize;

use crate::clock;
use crate::config::InstrumentProfile;
use crate::investment::{parse_decimal, Quantity};
use crate::store::FinanceStore;

/// Every provider name that may reach `finance_prices.source`.
///
/// This const is the enumeration the column deliberately does not carry as a
/// CHECK: the provider set grows, SQLite cannot alter a CHECK, and this crate
/// has no table-rebuild path at all (`store.rs`'s migration doc comment).
/// `every_registered_provider_name_round_trips` is the test that replaces it.
pub const PROVIDERS: &[&str] = &["broker", "yahoo", "ecb"];

/// What the FX providers publish rates against.
pub const FX_BASE: &str = "EUR";

/// One observed price. An observation, never a correction: two sources may hold
/// one instrument-day and the reader prefers the newest `fetched_at` and reports
/// which source it used.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PriceObservation {
    pub instrument: String,
    pub observed_on: String,
    pub price: Quantity,
    pub currency: String,
    pub source: String,
    pub fetched_at: String,
}

/// One published FX reference rate, in quote units per one base unit, stored as
/// published. Never inverted at write time: a division is where an exact decimal
/// stops being exact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FxObservation {
    pub base: String,
    pub quote: String,
    pub observed_on: String,
    pub rate: Quantity,
    pub source: String,
    pub fetched_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FetchStatus {
    /// Rows were written.
    Ok,
    /// The provider answered correctly and had nothing to give.
    Empty,
    /// The provider answered something that is not the contract: a gate page, a
    /// body that is not CSV, an instrument with no configured ticker.
    Refused,
    /// The request itself failed: no route, a timeout, a torn body.
    Error,
}

impl FetchStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Empty => "empty",
            Self::Refused => "refused",
            Self::Error => "error",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "ok" => Some(Self::Ok),
            "empty" => Some(Self::Empty),
            "refused" => Some(Self::Refused),
            "error" => Some(Self::Error),
            _ => None,
        }
    }
}

/// One attempt against one target, successful or not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FetchAttempt {
    pub provider: String,
    pub target: String,
    pub requested_on: String,
    pub status: FetchStatus,
    /// A bounded reason -- an HTTP status, a note that the body was not CSV.
    /// Never a response body: a provider page can contain anything and this
    /// table is inside the backup contract.
    pub detail: String,
    pub rows_written: i64,
    pub fetched_at: String,
}

/// What one provider produced for one target.
pub struct ProviderResult {
    pub prices: Vec<PriceObservation>,
    pub rates: Vec<FxObservation>,
    pub status: FetchStatus,
    pub detail: String,
}

impl ProviderResult {
    pub fn refused(detail: impl Into<String>) -> Self {
        Self {
            prices: Vec::new(),
            rates: Vec::new(),
            status: FetchStatus::Refused,
            detail: detail.into(),
        }
    }

    pub fn error(detail: impl Into<String>) -> Self {
        Self {
            prices: Vec::new(),
            rates: Vec::new(),
            status: FetchStatus::Error,
            detail: detail.into(),
        }
    }
}

/// What a provider needs to know to fetch. Assembled by the caller so a provider
/// opens no store and reads no config file of its own.
pub struct FetchContext<'a> {
    pub instruments: &'a [InstrumentProfile],
    /// The reviewed broker prices, by instrument. The `broker` provider's whole
    /// input; the networked providers ignore it.
    pub broker_prices: &'a BTreeMap<String, (Quantity, String, String)>,
    /// The currencies held or priced that are not [`FX_BASE`]. The FX provider's
    /// targets; [`fx_targets`] builds the list.
    pub quote_currencies: &'a [String],
    pub as_of: &'a str,
    pub fetched_at: &'a str,
    /// How many days of history a networked provider should ask for.
    pub history_days: u32,
}

/// A source of prices.
///
/// `targets` and `fetch` are separate so orchestration can write one
/// `finance_price_fetches` row per target even when the provider itself never
/// runs -- an instrument with no configured ticker is a recorded `refused`, not
/// a silence.
pub trait PriceProvider {
    fn name(&self) -> &'static str;

    /// The targets this provider would attempt, in order. A target is opaque to
    /// the caller and meaningful to the provider: an instrument symbol, a
    /// currency pair.
    fn targets(&self, context: &FetchContext<'_>) -> Vec<String>;

    /// Fetch one target. Must never panic and must never abort the run: a
    /// failure is a `ProviderResult` with a status.
    fn fetch(&self, target: &str, context: &FetchContext<'_>) -> ProviderResult;
}

// ---------------------------------------------------------------------------
// broker
// ---------------------------------------------------------------------------

/// Replays the reviewed activity prices already in `finance_holding_projection`.
///
/// No network, so it always works, and it is what makes the Investments tab
/// useful on a machine with no internet. It writes one observation per priced
/// instrument per run, which is also its limit: a history built this way grows
/// one point per fetch, so it is a floor under the price series and never a
/// substitute for a market source.
pub struct BrokerProvider;

impl PriceProvider for BrokerProvider {
    fn name(&self) -> &'static str {
        "broker"
    }

    fn targets(&self, context: &FetchContext<'_>) -> Vec<String> {
        context.broker_prices.keys().cloned().collect()
    }

    fn fetch(&self, target: &str, context: &FetchContext<'_>) -> ProviderResult {
        let Some((price, currency, observed_on)) = context.broker_prices.get(target) else {
            return ProviderResult::refused("instrument is not in the reviewed projection");
        };
        ProviderResult {
            prices: vec![PriceObservation {
                instrument: target.to_string(),
                observed_on: observed_on.clone(),
                price: price.clone(),
                currency: currency.clone(),
                source: "broker".into(),
                fetched_at: context.fetched_at.to_string(),
            }],
            rates: Vec::new(),
            status: FetchStatus::Ok,
            detail: String::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// ecb -- the FX reference rates
// ---------------------------------------------------------------------------

/// Daily euro foreign-exchange reference rates from the ECB Data Portal, read as
/// CSV from the SDMX data endpoint.
///
/// Measured 2026-09-05:
/// `GET https://data-api.ecb.europa.eu/service/data/EXR/D.USD.EUR.SP00.A?format=csvdata&lastNObservations=2`
/// answered HTTP 200 with a 31-column header and the rows TIME_PERIOD=2026-09-03
/// OBS_VALUE=1.1615 and TIME_PERIOD=2026-09-04 OBS_VALUE=1.1622.
///
/// Read **by column name**, so an inserted column does not shift the parse.
/// `lastNObservations` bounds the request. Rates are stored exactly as published
/// -- quote units per one base unit -- and never inverted at write time, because
/// a division is where an exact decimal stops being exact.
pub struct EcbProvider {
    pub client: reqwest::blocking::Client,
    /// The fallback when the ECB endpoint is unreachable: Frankfurter, an
    /// MIT-licensed server of the same ECB reference rates. A second path to one
    /// fact rather than a second fact -- rows carry `source = "ecb"` either way
    /// and the detail names which door answered.
    pub allow_fallback: bool,
}

const ECB_BASE: &str = "https://data-api.ecb.europa.eu/service/data/EXR";
const FRANKFURTER_BASE: &str = "https://api.frankfurter.dev/v1";

impl PriceProvider for EcbProvider {
    fn name(&self) -> &'static str {
        "ecb"
    }

    fn targets(&self, context: &FetchContext<'_>) -> Vec<String> {
        context.quote_currencies.to_vec()
    }

    fn fetch(&self, target: &str, context: &FetchContext<'_>) -> ProviderResult {
        if target.len() != 3 || !target.chars().all(|c| c.is_ascii_uppercase()) {
            return ProviderResult::refused("a currency code is three uppercase letters");
        }
        let url = format!(
            "{ECB_BASE}/D.{target}.{FX_BASE}.SP00.A?format=csvdata&lastNObservations={}",
            context.history_days.min(400)
        );
        match self.client.get(&url).send() {
            Ok(response) => {
                let status = response.status();
                let Ok(body) = response.text() else {
                    return ProviderResult::error(format!("HTTP {status}: the body was torn"));
                };
                match parse_ecb_csv(&body, target, context.fetched_at) {
                    Ok(rates) if rates.is_empty() => ProviderResult {
                        prices: Vec::new(),
                        rates,
                        status: FetchStatus::Empty,
                        detail: format!("HTTP {status}: the series carried no observation"),
                    },
                    Ok(rates) => ProviderResult {
                        prices: Vec::new(),
                        rates,
                        status: FetchStatus::Ok,
                        detail: String::new(),
                    },
                    Err(reason) if self.allow_fallback => {
                        self.frankfurter(target, context, &reason)
                    }
                    Err(reason) => ProviderResult::refused(format!("HTTP {status}: {reason}")),
                }
            }
            Err(error) if self.allow_fallback => {
                self.frankfurter(target, context, &format!("request failed: {error}"))
            }
            Err(error) => ProviderResult::error(format!("request failed: {error}")),
        }
    }
}

impl EcbProvider {
    /// The same ECB reference rates over a simpler JSON shape.
    ///
    /// Measured 2026-09-05: `GET https://api.frankfurter.dev/v1/latest?base=EUR`
    /// answered HTTP 200 with date 2026-09-04 and thirty-odd rates. Only the
    /// latest, not a history: this door exists so a stale FX table has one more
    /// chance to update, not so it becomes the primary series.
    fn frankfurter(&self, target: &str, context: &FetchContext<'_>, why: &str) -> ProviderResult {
        let url = format!("{FRANKFURTER_BASE}/latest?base={FX_BASE}&symbols={target}");
        let Ok(response) = self.client.get(&url).send() else {
            return ProviderResult::error(format!("{why}; the fallback did not answer either"));
        };
        let status = response.status();
        let Ok(body) = response.text() else {
            return ProviderResult::error(format!("{why}; the fallback body was torn"));
        };
        match parse_frankfurter_json(&body, target, context.fetched_at) {
            Ok(rates) if rates.is_empty() => ProviderResult {
                prices: Vec::new(),
                rates,
                status: FetchStatus::Empty,
                detail: format!("{why}; the fallback answered HTTP {status} with no rate"),
            },
            Ok(rates) => ProviderResult {
                prices: Vec::new(),
                rates,
                status: FetchStatus::Ok,
                detail: format!("{why}; served by the frankfurter fallback"),
            },
            Err(reason) => {
                ProviderResult::refused(format!("{why}; the fallback answered {reason}"))
            }
        }
    }
}

/// Parse the SDMX CSV by column name.
///
/// The header check is the shape rule: a body whose first line does not carry
/// `TIME_PERIOD` and `OBS_VALUE` is refused whatever the status code said.
pub fn parse_ecb_csv(
    body: &str,
    quote: &str,
    fetched_at: &str,
) -> Result<Vec<FxObservation>, String> {
    if looks_like_html(body) {
        return Err("the body is HTML, not CSV".into());
    }
    if !csv_header_matches(body, &["TIME_PERIOD", "OBS_VALUE"]) {
        return Err("the first line is not the expected CSV header".into());
    }
    let mut reader = csv::ReaderBuilder::new()
        .flexible(true)
        .from_reader(body.as_bytes());
    let headers = reader
        .headers()
        .map_err(|_| "the CSV header could not be read".to_string())?
        .clone();
    let period = header_index(&headers, "TIME_PERIOD")?;
    let value = header_index(&headers, "OBS_VALUE")?;
    let mut rates = Vec::new();
    for record in reader.records() {
        let Ok(record) = record else { continue };
        let (Some(day), Some(raw)) = (record.get(period), record.get(value)) else {
            continue;
        };
        if raw.trim().is_empty() || !crate::clock::valid_iso_date(day.trim()) {
            continue;
        }
        let rate = parse_provider_decimal(raw)?;
        rates.push(FxObservation {
            base: FX_BASE.into(),
            quote: quote.to_string(),
            observed_on: day.trim().to_string(),
            rate,
            source: "ecb".into(),
            fetched_at: fetched_at.to_string(),
        });
    }
    Ok(rates)
}

fn header_index(headers: &csv::StringRecord, name: &str) -> Result<usize, String> {
    headers
        .iter()
        .position(|column| column.trim() == name)
        .ok_or_else(|| format!("the CSV has no {name} column"))
}

/// Parse Frankfurter's `{ "date": "...", "rates": { "USD": 1.1622 } }`.
pub fn parse_frankfurter_json(
    body: &str,
    quote: &str,
    fetched_at: &str,
) -> Result<Vec<FxObservation>, String> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|_| "a body that is not JSON".to_string())?;
    let Some(day) = value.get("date").and_then(|day| day.as_str()) else {
        return Err("JSON with no date".into());
    };
    if !crate::clock::valid_iso_date(day) {
        return Err("JSON whose date is not a date".into());
    }
    let Some(raw) = value.get("rates").and_then(|rates| rates.get(quote)) else {
        return Ok(Vec::new());
    };
    // Serialised back through `to_string` rather than read as f64: the number is
    // exact in the JSON text and only becomes lossy once it is a float.
    let rate = parse_provider_decimal(&raw.to_string())?;
    Ok(vec![FxObservation {
        base: FX_BASE.into(),
        quote: quote.to_string(),
        observed_on: day.to_string(),
        rate,
        source: "ecb".into(),
        fetched_at: fetched_at.to_string(),
    }])
}

// ---------------------------------------------------------------------------
// yahoo -- daily close history
// ---------------------------------------------------------------------------

/// Daily close history per instrument, from Yahoo's v8 chart endpoint.
///
/// Unofficial and undocumented, and the flip condition is written into
/// `upstreams.toml` rather than remembered: retired the day two consecutive
/// manual runs get a non-200 that a fresh consent-plus-crumb handshake does not
/// fix, or the day Yahoo publishes terms forbidding this use. On retirement the
/// `broker` provider still prices every holding.
///
/// Measured 2026-09-05 from this network:
/// `GET https://query1.finance.yahoo.com/v8/finance/chart/SPY?range=5d&interval=1d`
/// answered **HTTP 200** with five daily closes and `chart.error = null` -- no
/// cookie and no crumb needed. `GET https://query1.finance.yahoo.com/v1/test/getcrumb`
/// answered **HTTP 429** both with and without the `A3` consent cookie that
/// `GET https://fc.yahoo.com` sets (that endpoint answers 404 and sets the cookie
/// anyway). So the bare call is the primary path and the handshake is the retry:
/// a non-200 or an error body triggers consent, crumb and one retry.
///
/// **Re-measured 2026-09-08, and both readings repeated exactly** -- 200 on the
/// chart, 429 on the crumb with and without `A3`. The full numbers are in the
/// `[yahoo-finance-chart]` row of `upstreams.toml` and not duplicated here; a
/// measurement lives in one place or it drifts. Nothing below changed because of
/// it: two readings from one network three days apart are two readings, not a
/// property of the endpoint, which is the whole lesson the row records. The
/// handshake is still the retry path and still **unverified live**.
pub struct YahooProvider {
    pub client: reqwest::blocking::Client,
}

const YAHOO_CHART: &str = "https://query1.finance.yahoo.com/v8/finance/chart";
const YAHOO_CONSENT: &str = "https://fc.yahoo.com";
const YAHOO_CRUMB: &str = "https://query1.finance.yahoo.com/v1/test/getcrumb";

impl PriceProvider for YahooProvider {
    fn name(&self) -> &'static str {
        "yahoo"
    }

    /// Only instruments with a configured ticker. An instrument without one is
    /// still a target, so the run records a `refused` row naming the missing key
    /// rather than passing over it in silence.
    fn targets(&self, context: &FetchContext<'_>) -> Vec<String> {
        context
            .instruments
            .iter()
            .map(|profile| profile.instrument.clone())
            .collect()
    }

    fn fetch(&self, target: &str, context: &FetchContext<'_>) -> ProviderResult {
        let Some(profile) = context
            .instruments
            .iter()
            .find(|profile| profile.instrument == target)
        else {
            return ProviderResult::refused("the instrument is not in the overlay's instruments");
        };
        let Some(ticker) = profile.ticker.as_deref().filter(|t| !t.trim().is_empty()) else {
            return ProviderResult::refused(
                "no ticker configured for this instrument; add `ticker` to its instruments entry",
            );
        };
        let range = if context.history_days > 365 {
            "2y"
        } else {
            "1y"
        };
        let url = format!(
            "{YAHOO_CHART}/{}?range={range}&interval=1d",
            urlencode(ticker)
        );
        match self.attempt(&url, target, context, None) {
            Ok(result) => result,
            Err(first) => match self.crumb() {
                Some(crumb) => {
                    let retried = format!("{url}&crumb={}", urlencode(&crumb));
                    match self.attempt(&retried, target, context, Some(&first)) {
                        Ok(result) => result,
                        Err(second) => ProviderResult::refused(format!(
                            "{first}; a fresh consent-plus-crumb handshake answered {second}"
                        )),
                    }
                }
                None => ProviderResult::refused(format!(
                    "{first}; the crumb handshake did not produce a crumb"
                )),
            },
        }
    }
}

impl YahooProvider {
    fn attempt(
        &self,
        url: &str,
        instrument: &str,
        context: &FetchContext<'_>,
        after: Option<&str>,
    ) -> Result<ProviderResult, String> {
        let response = self
            .client
            .get(url)
            .send()
            .map_err(|error| format!("request failed: {error}"))?;
        let status = response.status();
        let body = response
            .text()
            .map_err(|_| format!("HTTP {status}: the body was torn"))?;
        // The shape rule again, and it is what a status check would have missed:
        // a gate page can arrive with any status at all.
        if looks_like_html(&body) {
            return Err(format!(
                "HTTP {status}: the body is HTML, not the chart JSON"
            ));
        }
        let observations = parse_yahoo_chart(&body, instrument, context.fetched_at)
            .map_err(|reason| format!("HTTP {status}: {reason}"))?;
        let detail = match after {
            Some(first) => format!("{first}; recovered after the consent-plus-crumb handshake"),
            None => String::new(),
        };
        Ok(ProviderResult {
            status: if observations.is_empty() {
                FetchStatus::Empty
            } else {
                FetchStatus::Ok
            },
            prices: observations,
            rates: Vec::new(),
            detail,
        })
    }

    /// The EU consent cookie, then the crumb.
    ///
    /// `fc.yahoo.com` answers 404 and sets the `A3` cookie anyway, which is why
    /// the status is ignored here and only the cookie jar matters. Measured
    /// 2026-09-05: the crumb endpoint answered 429 from this network, so this
    /// path is present and unverified live -- the bare chart call was the one
    /// that worked.
    fn crumb(&self) -> Option<String> {
        let _ = self.client.get(YAHOO_CONSENT).send();
        let response = self.client.get(YAHOO_CRUMB).send().ok()?;
        if !response.status().is_success() {
            return None;
        }
        let crumb = response.text().ok()?.trim().to_string();
        // A crumb is a short opaque token. An HTML page or a sentence is the
        // refusal wearing a 200, which is the whole reason for a shape check.
        (!crumb.is_empty() && crumb.len() <= 32 && !crumb.contains(char::is_whitespace))
            .then_some(crumb)
    }
}

/// Turn the chart JSON into exact-decimal observations.
///
/// The wire carries IEEE doubles (`767.0499877929688` for a close of 767.05), so
/// an exact decimal has to be CHOSEN at this boundary rather than recovered. The
/// choice is four decimal places, which is more precision than any quoted equity
/// price carries and is stated here rather than left for a reader to infer from a
/// stored mantissa.
pub fn parse_yahoo_chart(
    body: &str,
    instrument: &str,
    fetched_at: &str,
) -> Result<Vec<PriceObservation>, String> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|_| "a body that is not JSON".to_string())?;
    let chart = value
        .get("chart")
        .ok_or_else(|| "JSON with no chart object".to_string())?;
    if let Some(error) = chart.get("error").filter(|error| !error.is_null()) {
        // The provider's `code` and never its `description`. The description is
        // free text from the response body, and `{prefix}_price_fetches.detail`
        // states that it never holds a response body (store.rs, the DDL comment
        // above the table) because a provider page can contain anything and this
        // table is backed up. A code is a short token from a closed set; it is
        // still provider-supplied, so it is bounded here as well as at the write.
        let code = error
            .get("code")
            .and_then(|value| value.as_str())
            .filter(|code| {
                code.len() <= 40
                    && code.chars().all(|character| {
                        character.is_ascii_alphanumeric() || " -_.".contains(character)
                    })
            })
            .unwrap_or("unnamed");
        return Err(format!("the chart carried an error, code {code}"));
    }
    let result = chart
        .get("result")
        .and_then(|result| result.as_array())
        .and_then(|results| results.first())
        .ok_or_else(|| "JSON with no chart result".to_string())?;
    let currency = result
        .get("meta")
        .and_then(|meta| meta.get("currency"))
        .and_then(|currency| currency.as_str())
        .ok_or_else(|| "a chart result with no currency".to_string())?;
    let timestamps = result
        .get("timestamp")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    // `adjclose` when it is there, `close` otherwise. Adjusted closes are what a
    // return series has to be computed from -- an unadjusted series records a
    // dividend or a split as a price move that never happened.
    let closes = result
        .get("indicators")
        .and_then(|indicators| {
            indicators
                .get("adjclose")
                .and_then(|series| series.as_array())
                .and_then(|series| series.first())
                .and_then(|entry| entry.get("adjclose"))
                .or_else(|| {
                    indicators
                        .get("quote")
                        .and_then(|series| series.as_array())
                        .and_then(|series| series.first())
                        .and_then(|entry| entry.get("close"))
                })
        })
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    if timestamps.len() != closes.len() {
        return Err("the timestamp and close series are different lengths".into());
    }
    let mut observations = Vec::new();
    for (timestamp, close) in timestamps.iter().zip(&closes) {
        let (Some(seconds), Some(price)) = (timestamp.as_i64(), close.as_f64()) else {
            continue;
        };
        if !price.is_finite() || price <= 0.0 {
            continue;
        }
        let day = crate::clock::civil_from_days(seconds.div_euclid(86_400));
        let quantity = parse_provider_decimal(&format!("{price:.4}"))?;
        observations.push(PriceObservation {
            instrument: instrument.to_string(),
            observed_on: day,
            price: quantity,
            currency: currency.to_string(),
            source: "yahoo".into(),
            fetched_at: fetched_at.to_string(),
        });
    }
    Ok(observations)
}

/// The three characters a ticker or a crumb could carry that a query string
/// reads as structure. Not a general encoder: a ticker is `[A-Z0-9.^-]` and a
/// crumb is base64-ish, so this covers the set and refuses to pretend otherwise.
fn urlencode(value: &str) -> String {
    value
        .chars()
        .map(|c| match c {
            '&' => "%26".to_string(),
            '?' => "%3F".to_string(),
            '=' => "%3D".to_string(),
            '/' => "%2F".to_string(),
            ' ' => "%20".to_string(),
            other => other.to_string(),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// The shape rule, shared by every networked provider
// ---------------------------------------------------------------------------

/// The header a CSV body must start with, or the body is not the contract.
///
/// Named rather than inlined because it is the rule the Stooq measurement bought:
/// a 200 with an HTML gate page fails here and nowhere else.
pub fn csv_header_matches(body: &str, required: &[&str]) -> bool {
    let Some(first) = body.lines().next() else {
        return false;
    };
    let columns: Vec<&str> = first
        .split(',')
        .map(|column| column.trim().trim_matches('"'))
        .collect();
    required
        .iter()
        .all(|name| columns.iter().any(|column| column == name))
}

/// A body that is HTML rather than the data contract, whatever the status said.
pub fn looks_like_html(body: &str) -> bool {
    let head = body.trim_start().get(..512).unwrap_or(body.trim_start());
    let lowered = head.to_ascii_lowercase();
    lowered.starts_with("<!doctype")
        || lowered.starts_with("<html")
        || lowered.contains("<script")
        || lowered.contains("<noscript")
}

/// One shared blocking client. Built by the caller inside `spawn_blocking`,
/// never on the async runtime: a blocking reqwest client driven from a Tokio
/// worker panics at run time rather than failing to compile.
pub fn blocking_client() -> Result<reqwest::blocking::Client, String> {
    sjel_http::builder(
        sjel_http::Purpose::new("finance-price"),
        Duration::from_secs(15),
    )
    .cookie_store(true)
    .build()
    .map_err(|error| format!("client could not be built: {error}"))
}

// ---------------------------------------------------------------------------
// Orchestration
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FetchRun {
    pub provider: String,
    pub attempted: usize,
    pub written: i64,
    pub refused: usize,
    pub errored: usize,
    pub attempts: Vec<FetchAttempt>,
}

impl FetchRun {
    /// True when at least one attempt reached its provider. The CLI's exit
    /// status, and it counts ATTEMPTS rather than rows on purpose: every write
    /// here is idempotent (`UNIQUE (instrument, observed_on, source)`), so the
    /// normal nightly outcome is a run that succeeds and writes nothing --
    /// `broker` re-reads the same reviewed date, a market provider re-reads a
    /// weekend, an offline host writes nothing at all. Keyed on `written > 0`
    /// this made `tools/service-runner.sh` return non-zero and systemd mark the
    /// oneshot unit failed on a run in which nothing was wrong.
    ///
    /// A run with no targets is a success: nothing was asked of it. A host with
    /// no instruments configured has no fetch to fail, and its staleness is
    /// visible on `GET /api/prices/status`, not in an exit code.
    pub fn succeeded(&self) -> bool {
        if self.attempted == 0 {
            return true;
        }
        self.refused + self.errored < self.attempted
    }
}

/// Run one provider over every target it names, writing a fetch row per attempt.
///
/// `dry_run` writes nothing at all -- not the prices and not the attempt rows --
/// because a recorded attempt that never happened is a lie in the one table that
/// exists to answer why an instrument is stale.
pub fn run_provider(
    store: &FinanceStore,
    provider: &dyn PriceProvider,
    context: &FetchContext<'_>,
    dry_run: bool,
) -> Result<FetchRun, String> {
    let mut run = FetchRun {
        provider: provider.name().to_string(),
        attempted: 0,
        written: 0,
        refused: 0,
        errored: 0,
        attempts: Vec::new(),
    };
    for target in provider.targets(context) {
        run.attempted += 1;
        let result = provider.fetch(&target, context);
        let mut written = 0i64;
        if !dry_run {
            for observation in &result.prices {
                match store.append_market_price(observation) {
                    Ok(true) => written += 1,
                    Ok(false) => {}
                    Err(error) => return Err(error.to_string()),
                }
            }
            for rate in &result.rates {
                match store.append_fx_rate(rate) {
                    Ok(true) => written += 1,
                    Ok(false) => {}
                    Err(error) => return Err(error.to_string()),
                }
            }
        } else {
            written = (result.prices.len() + result.rates.len()) as i64;
        }
        let status = match result.status {
            // A provider that answered correctly and produced nothing new is
            // `empty`, not `ok`: an idempotent re-fetch and a source with no
            // data are different facts.
            FetchStatus::Ok if result.prices.is_empty() && result.rates.is_empty() => {
                FetchStatus::Empty
            }
            other => other,
        };
        match status {
            FetchStatus::Refused => run.refused += 1,
            FetchStatus::Error => run.errored += 1,
            _ => {}
        }
        run.written += written;
        let attempt = FetchAttempt {
            provider: provider.name().to_string(),
            target: target.clone(),
            requested_on: context.as_of.to_string(),
            status,
            detail: truncate_detail(&result.detail),
            rows_written: written,
            fetched_at: context.fetched_at.to_string(),
        };
        if !dry_run {
            store
                .record_fetch(&attempt)
                .map_err(|error| error.to_string())?;
        }
        run.attempts.push(attempt);
    }
    Ok(run)
}

/// Every provider named, built and run against one store.
///
/// One assembly point, so `finance-cli prices fetch` and any future caller read
/// the same context rather than two that drift. `names` is the provider list to
/// run; an unknown name is an error rather than a silent skip.
pub fn run_named(
    store: &FinanceStore,
    config: &crate::config::Config,
    names: &[String],
    as_of: &str,
    fetched_at: &str,
    dry_run: bool,
) -> Result<Vec<FetchRun>, String> {
    let snapshot = store
        .holding_projection()
        .map_err(|error| error.to_string())?
        .ok_or("no reviewed holdings snapshot is in the projection; import one first")?;
    let broker_prices = broker_prices_from_snapshot(&snapshot);
    let subscriptions = store.list().map_err(|error| error.to_string())?;
    let quote_currencies = fx_targets(
        snapshot
            .holdings
            .iter()
            .map(|holding| holding.currency.as_str()),
        &subscriptions,
    );
    let context = FetchContext {
        instruments: &config.instruments,
        broker_prices: &broker_prices,
        quote_currencies: &quote_currencies,
        as_of,
        fetched_at,
        history_days: 400,
    };
    let mut runs = Vec::new();
    for name in names {
        // Built here, once per named provider, and never on an async runtime: a
        // blocking reqwest client driven from a Tokio worker panics at run time.
        let provider: Box<dyn PriceProvider> = match name.as_str() {
            "broker" => Box::new(BrokerProvider),
            "ecb" => Box::new(EcbProvider {
                client: blocking_client()?,
                allow_fallback: true,
            }),
            "yahoo" => Box::new(YahooProvider {
                client: blocking_client()?,
            }),
            other => {
                return Err(format!(
                    "unknown provider {other:?}; the registered set is {}",
                    PROVIDERS.join(", ")
                ))
            }
        };
        runs.push(run_provider(store, provider.as_ref(), &context, dry_run)?);
    }
    Ok(runs)
}

/// The currencies the FX provider fetches against [`FX_BASE`]: every currency a
/// holding is in, and every currency a subscription is priced in.
///
/// Holdings alone are not enough. `money::to_eur` needs a rate for each subscription
/// currency (PRD Q103), and a store whose holdings are all EUR used to hand the `ecb`
/// provider an empty list, so a USD subscription stayed `not_convertible` and
/// `finance-cli subscriptions audit` could never pass. Every point in a price series
/// counts, not only the one in force, because a scheduled price is rendered too.
/// Codes are trimmed and upper-cased, as `money::to_eur` reads them.
pub fn fx_targets<'a>(
    holding_currencies: impl IntoIterator<Item = &'a str>,
    subscriptions: &'a [crate::subscription::Subscription],
) -> Vec<String> {
    let subscription_currencies = subscriptions
        .iter()
        .flat_map(|subscription| subscription.prices.iter())
        .map(|price| price.currency.as_str());
    holding_currencies
        .into_iter()
        .chain(subscription_currencies)
        .map(|currency| currency.trim().to_ascii_uppercase())
        .filter(|currency| !currency.is_empty() && currency != FX_BASE)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// A reason a human reads, bounded so no provider can grow this column.
fn truncate_detail(detail: &str) -> String {
    let cleaned: String = detail
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let cleaned = cleaned.trim();
    if cleaned.chars().count() <= 200 {
        return cleaned.to_string();
    }
    cleaned.chars().take(197).collect::<String>() + "..."
}

/// Build the `broker_prices` map a [`FetchContext`] needs from the reviewed
/// snapshot. The observation date is the snapshot's own review date, so a broker
/// point never claims to have been observed later than the file it came from.
pub fn broker_prices_from_snapshot(
    snapshot: &crate::investment::ReviewedHoldingsSnapshot,
) -> BTreeMap<String, (Quantity, String, String)> {
    let mut map = BTreeMap::new();
    for holding in &snapshot.holdings {
        let Some(price) = holding.latest_unit_price.clone() else {
            continue;
        };
        let observed_on = holding_review_date(snapshot, &holding.instrument);
        map.insert(
            holding.instrument.clone(),
            (price, holding.currency.clone(), observed_on),
        );
    }
    map
}

fn holding_review_date(
    snapshot: &crate::investment::ReviewedHoldingsSnapshot,
    _instrument: &str,
) -> String {
    // The snapshot's own review date. A per-source date would be more precise,
    // but `finance_holding_projection` does not record which source a holding
    // came from, and inventing that link here would be a provenance claim this
    // module cannot back.
    if clock::valid_iso_date(&snapshot.reviewed_at) {
        snapshot.reviewed_at.clone()
    } else {
        snapshot
            .reviewed_at
            .get(..10)
            .filter(|head| clock::valid_iso_date(head))
            .map(str::to_string)
            .unwrap_or_else(clock::today)
    }
}

/// Parse a provider decimal through the reader the CSV import already uses, which
/// handles both decimal marks, grouping separators and a trailing minus.
pub fn parse_provider_decimal(value: &str) -> Result<Quantity, String> {
    parse_decimal(value.trim(), '.', "price").map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The body Stooq served on 2026-09-05, trimmed to its shape. Served with
    /// HTTP 200. It is a test fixture because the 200 is the trap: a client that
    /// checks the status parses this as CSV.
    const STOOQ_PROOF_OF_WORK_BODY: &str = concat!(
        "<!DOCTYPE html><html><head><meta charset=\"utf-8\"></head><body>\n",
        "<script>async function s(){for(let n=0;;n++){",
        "const h=await crypto.subtle.digest(\"SHA-256\",new TextEncoder().encode(n));",
        "if(new Uint8Array(h)[0]===0&&new Uint8Array(h)[1]===0)return n}}</script>\n",
        "<noscript>Please enable JavaScript to continue.</noscript>\n",
        "</body></html>\n"
    );

    fn priced(name: &str, currencies: &[&str]) -> crate::subscription::Subscription {
        use crate::subscription::{BillingCycle, PricePoint, Subscription};
        Subscription {
            id: format!("sub_{name}"),
            name: name.into(),
            source_path: format!("Subscriptions/{name}.md"),
            category: None,
            value_rating: None,
            prices: currencies
                .iter()
                .enumerate()
                .map(|(i, currency)| PricePoint {
                    valid_from: format!("2026-0{}-01", i + 1),
                    amount_cents: 1500,
                    currency: (*currency).into(),
                    cycle: BillingCycle::Monthly,
                    plan: None,
                    reason: "fixture".into(),
                })
                .collect(),
            states: Vec::new(),
        }
    }

    #[test]
    fn a_subscription_currency_is_an_fx_target_when_every_holding_is_eur() {
        // PRD Q103: with EUR-only holdings the targets used to be empty, so no EUR/USD
        // rate was ever fetched and a USD subscription could never be stated in EUR.
        let subs = vec![
            priced("Studio Lite", &["USD"]),
            priced("Vault Storage", &["EUR"]),
            priced("Transit Card", &["EUR", " chf "]),
        ];
        assert_eq!(fx_targets(["EUR", "EUR"], &subs), vec!["CHF", "USD"]);
    }

    #[test]
    fn fx_targets_merge_holdings_and_subscriptions_without_duplicates() {
        let subs = vec![priced("Studio Lite", &["USD"])];
        assert_eq!(fx_targets(["USD", "GBP", "EUR"], &subs), vec!["GBP", "USD"]);
        assert!(fx_targets(["EUR"], &[]).is_empty());
    }

    #[test]
    fn a_proof_of_work_gate_is_not_a_csv() {
        // The measurement, not the taste: this body arrived with HTTP 200.
        assert!(looks_like_html(STOOQ_PROOF_OF_WORK_BODY));
        assert!(!csv_header_matches(
            STOOQ_PROOF_OF_WORK_BODY,
            &["Date", "Close"]
        ));
        assert!(csv_header_matches(
            "Date,Open,High,Low,Close,Volume\n2026-09-04,1,2,0,1.5,10\n",
            &["Date", "Close"]
        ));
    }

    #[test]
    fn every_registered_provider_name_is_a_distinct_lowercase_token() {
        for name in PROVIDERS {
            assert!(!name.is_empty());
            assert!(name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()));
        }
        let mut sorted = PROVIDERS.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), PROVIDERS.len());
    }

    #[test]
    fn a_price_never_becomes_a_float() {
        let parsed = parse_provider_decimal("128.42").expect("a decimal");
        assert_eq!(parsed.mantissa, 12_842);
        assert_eq!(parsed.scale, 2);
    }

    #[test]
    fn a_detail_is_bounded_and_carries_no_control_characters() {
        let detail = truncate_detail(&format!("a\nb{}", "x".repeat(500)));
        assert_eq!(detail.chars().count(), 200);
        assert!(!detail.contains('\n'));
    }

    /// The body the ECB Data Portal served on 2026-09-05, trimmed to the columns
    /// the parser reads. Kept because the parse is by column NAME: this proves an
    /// inserted column does not shift it.
    const ECB_CSV: &str = concat!(
        "KEY,FREQ,CURRENCY,CURRENCY_DENOM,EXR_TYPE,EXR_SUFFIX,TIME_PERIOD,OBS_VALUE,OBS_STATUS\n",
        "EXR.D.USD.EUR.SP00.A,D,USD,EUR,SP00,A,2026-09-03,1.1615,A\n",
        "EXR.D.USD.EUR.SP00.A,D,USD,EUR,SP00,A,2026-09-04,1.1622,A\n"
    );

    #[test]
    fn the_ecb_csv_is_read_by_column_name_and_stays_exact() {
        let rates = parse_ecb_csv(ECB_CSV, "USD", "2026-09-05T00:00:00Z").expect("rates");
        assert_eq!(rates.len(), 2);
        assert_eq!(rates[0].observed_on, "2026-09-03");
        assert_eq!(rates[0].rate.mantissa, 11_615);
        assert_eq!(rates[0].rate.scale, 4);
        assert_eq!(rates[1].rate.mantissa, 11_622);
        // Stored as published: quote units per one base unit, never inverted.
        assert_eq!(rates[0].base, "EUR");
        assert_eq!(rates[0].quote, "USD");
    }

    #[test]
    fn a_gate_page_is_refused_by_every_networked_parser_whatever_the_status_said() {
        assert!(parse_ecb_csv(STOOQ_PROOF_OF_WORK_BODY, "USD", "now").is_err());
        assert!(parse_yahoo_chart(STOOQ_PROOF_OF_WORK_BODY, "SYN-A", "now").is_err());
        assert!(parse_frankfurter_json(STOOQ_PROOF_OF_WORK_BODY, "USD", "now").is_err());
        // A CSV with the wrong header is refused too, not parsed positionally.
        assert!(parse_ecb_csv("A,B,C\n1,2,3\n", "USD", "now").is_err());
    }

    #[test]
    fn the_frankfurter_fallback_reads_the_same_fact_and_labels_it_ecb() {
        let rates = parse_frankfurter_json(
            r#"{"amount":1.0,"base":"EUR","date":"2026-09-04","rates":{"USD":1.1622}}"#,
            "USD",
            "2026-09-05T00:00:00Z",
        )
        .expect("a rate");
        assert_eq!(rates.len(), 1);
        assert_eq!(rates[0].observed_on, "2026-09-04");
        assert_eq!(rates[0].rate.mantissa, 11_622);
        assert_eq!(rates[0].source, "ecb");
    }

    /// The shape Yahoo's v8 chart endpoint answered with on 2026-09-05, cut to
    /// two days. `adjclose` is preferred over `close`: an unadjusted series
    /// records a dividend or a split as a price move that never happened.
    #[test]
    fn the_yahoo_chart_becomes_exact_decimals_from_ieee_doubles() {
        let body = r#"{"chart":{"result":[{"meta":{"currency":"USD","symbol":"SPY"},
            "timestamp":[1788442200,1788528600],
            "indicators":{"quote":[{"close":[773.1699829101562,770.1900024414062]}],
            "adjclose":[{"adjclose":[773.1699829101562,770.1900024414062]}]}}],"error":null}}"#;
        let observations =
            parse_yahoo_chart(body, "SYN-A", "2026-09-05T00:00:00Z").expect("prices");
        assert_eq!(observations.len(), 2);
        assert_eq!(observations[0].currency, "USD");
        assert_eq!(observations[0].source, "yahoo");
        // 773.1699829101562 at four places is 773.1700, stored as an exact pair.
        assert_eq!(observations[0].price.mantissa, 7_731_700);
        assert_eq!(observations[0].price.scale, 4);
        assert_eq!(observations[1].price.mantissa, 7_701_900);
    }

    #[test]
    fn a_yahoo_error_body_is_named_rather_than_parsed_as_an_empty_series() {
        let body = r#"{"chart":{"result":null,"error":{"code":"Not Found",
            "description":"No data found, symbol may be delisted"}}}"#;
        let error = parse_yahoo_chart(body, "SYN-A", "now").expect_err("an error");
        // The provider's short CODE reaches `{prefix}_price_fetches.detail`, and
        // its free-text description does not: that column's own DDL comment says
        // it never holds a response body, and the table is backed up.
        assert!(error.contains("Not Found"), "{error}");
        assert!(!error.contains("delisted"), "{error}");
    }

    #[test]
    fn a_yahoo_error_code_that_is_free_text_is_not_quoted_at_all() {
        let body = r#"{"chart":{"result":null,"error":{"code":"<script>alert(1)</script>",
            "description":"anything at all"}}}"#;
        let error = parse_yahoo_chart(body, "SYN-A", "now").expect_err("an error");
        assert_eq!(error, "the chart carried an error, code unnamed");
    }

    #[test]
    fn a_broker_target_missing_from_the_projection_is_refused_not_skipped() {
        let broker = BrokerProvider;
        let empty = BTreeMap::new();
        let context = FetchContext {
            instruments: &[],
            broker_prices: &empty,
            quote_currencies: &[],
            as_of: "2026-09-05",
            fetched_at: "2026-09-05T00:00:00Z",
            history_days: 400,
        };
        let result = broker.fetch("SYNTH", &context);
        assert_eq!(result.status, FetchStatus::Refused);
        assert!(result.prices.is_empty());
    }
}
