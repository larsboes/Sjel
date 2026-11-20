//! Places reads trip plans and persisted journeys through their owners.

use std::time::Duration;

use serde::Deserialize;
use sjel_server::InboundAuth;

use crate::store::Fallible;

#[derive(Deserialize)]
pub(crate) struct Plan {
    pub id: String,
    pub travelers: serde_json::Value,
    pub destinations: serde_json::Value,
    pub date_start: String,
    pub date_end: String,
    pub status: String,
}

#[derive(Deserialize)]
pub(crate) struct Leg {
    #[serde(rename = "origin_eva")]
    pub origin: String,
    pub origin_name: String,
    #[serde(rename = "destination_eva")]
    pub destination: String,
    pub destination_name: String,
    pub departure_time: String,
    pub train_name: String,
}

#[derive(Deserialize)]
pub(crate) struct Journey {
    pub trip_id: String,
    pub origin_eva: String,
    pub destination_eva: String,
    pub legs: Vec<Leg>,
}

#[derive(Deserialize)]
struct Page {
    trips: Vec<Journey>,
    truncated: bool,
    #[serde(default)]
    offset: usize,
}

fn gateway() -> String {
    let port = sjel_config::env_var("SJEL_STATUS_PORT")
        .ok()
        .and_then(|port| port.parse::<u16>().ok())
        .unwrap_or(8082);
    format!("http://127.0.0.1:{port}")
}

pub(crate) fn legs(journeys: Vec<Journey>) -> Vec<Leg> {
    let mut legs: Vec<_> = journeys
        .into_iter()
        .flat_map(|journey| {
            journey
                .legs
                .into_iter()
                .enumerate()
                .map(move |(index, leg)| (journey.trip_id.clone(), index, leg))
        })
        .collect();
    legs.sort_by(|(a_id, a_index, a), (b_id, b_index, b)| {
        (&a.departure_time, a_id, a_index).cmp(&(&b.departure_time, b_id, b_index))
    });
    legs.into_iter().map(|(_, _, leg)| leg).collect()
}

pub(crate) fn plans() -> Fallible<Vec<Plan>> {
    let client = client()?;
    let url = sjel_config::env_var("SJEL_TRIPS_URL")
        .map(|base| format!("{}/api/plans", base.trim_end_matches('/')))
        .unwrap_or_else(|_| format!("{}/trips/api/plans", gateway()));
    let mut plans: Vec<Plan> = read(&client, &url, &[])?;
    plans.retain(|plan| plan.status != "archived");
    plans.sort_by(|a, b| (&a.date_start, &a.id).cmp(&(&b.date_start, &b.id)));
    Ok(plans)
}

pub(crate) fn journeys() -> Fallible<Vec<Journey>> {
    let client = client()?;
    let url = sjel_config::env_var("SJEL_PLACES_TRANSIT_URL")
        .map(|base| format!("{}/api/trips", base.trim_end_matches('/')))
        .unwrap_or_else(|_| format!("{}/transit/api/trips", gateway()));
    let mut journeys = Vec::new();
    loop {
        let offset = journeys.len();
        let page: Page = read(&client, &url, &[("limit", 500), ("offset", offset)])?;
        if page.offset != offset || (page.truncated && page.trips.is_empty()) {
            return Err("Transit owner returned an invalid journey page".into());
        }
        journeys.extend(page.trips);
        if !page.truncated {
            return Ok(journeys);
        }
    }
}

pub(crate) fn media_geotags() -> Fallible<media_api::GeotagsResponse> {
    let socket = sjel_server::owner_snapshot::socket_path("media")?;
    let private = reqwest::blocking::Client::builder()
        .unix_socket(socket)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(15))
        .build()?;
    let url = format!("http://127.0.0.1{}", media_api::ROUTE_MEDIA_MAP_GEOTAGS);
    let request = || InboundAuth::with_loopback_auth(private.get(&url), &url).send();
    let mut response = request();
    if response.as_ref().is_err_and(reqwest::Error::is_connect) {
        let start = format!("{}/api/sjel-status/capabilities/media/start", gateway());
        InboundAuth::with_loopback_auth(client()?.post(&start), &start)
            .send()?
            .error_for_status()?;
        response = request();
    }
    Ok(response?.error_for_status()?.json()?)
}

fn client() -> Fallible<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(15))
        .build()?)
}

fn read<T: serde::de::DeserializeOwned>(
    client: &reqwest::blocking::Client,
    url: &str,
    query: &[(&str, usize)],
) -> Fallible<T> {
    Ok(
        InboundAuth::with_loopback_auth(client.get(url).query(query), url)
            .send()?
            .error_for_status()?
            .json()?,
    )
}
