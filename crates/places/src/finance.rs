//! Reads Finance projections without opening Finance's database.

use std::time::Duration;

use finance_api::places;
use serde::de::DeserializeOwned;
use sjel_server::InboundAuth;

use crate::store::Fallible;

pub fn expenses(description: Option<&str>) -> Fallible<Vec<places::Expense>> {
    match description {
        Some(description) => read(places::EXPENSES, &[("description", description)]),
        None => read(places::EXPENSES, &[]),
    }
}

pub fn candidates(source_account: &str) -> Fallible<Vec<places::Candidate>> {
    read(places::CANDIDATES, &[("source_account", source_account)])
}

fn read<T: DeserializeOwned>(route: &str, query: &[(&str, &str)]) -> Fallible<T> {
    let client = reqwest::blocking::Client::builder()
        .unix_socket(sjel_server::owner_snapshot::socket_path("finance")?)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(15))
        .build()?;
    let url = format!("http://127.0.0.1{route}");
    let send = || InboundAuth::with_loopback_auth(client.get(&url).query(query), &url).send();
    let reply = match send() {
        Err(error) if error.is_connect() => {
            start_finance()?;
            send()
        }
        reply => reply,
    };
    Ok(reply
        .map_err(|_| "Finance owner did not answer")?
        .error_for_status()
        .map_err(|_| "Finance owner rejected the Places read")?
        .json()?)
}

fn start_finance() -> Fallible<()> {
    let port = sjel_config::env_var("SJEL_STATUS_PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(8082);
    let url = format!("http://127.0.0.1:{port}/api/sjel-status/capabilities/finance/start");
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(15))
        .build()?;
    InboundAuth::with_loopback_auth(client.post(&url), &url)
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|_| "Finance owner could not start")?;
    Ok(())
}
