//! Loopback reads from Inventory and Finance.

use std::collections::BTreeMap;
use std::io::Read;
use std::time::Duration;

use axum::http::Method;
use inventory_api::{
    GroupComparison, PriceRecorded, RecordPriceRequest, SetAlternativesRequest, SetNeedsRequest,
    SetSpecsRequest, Spec,
};
use serde::Deserialize;

use crate::store::{Item, State};

type Error = Box<dyn std::error::Error>;
pub type Catalogue = BTreeMap<String, (Item, Option<State>)>;

fn inventory_url() -> String {
    sjel_config::env_var("SJEL_INVENTORY_URL").unwrap_or_else(|_| "http://127.0.0.1:8101".into())
}

/// Read the item catalogue and current states. Call from a CLI or blocking worker.
pub fn catalogue() -> Result<Catalogue, Error> {
    #[derive(Deserialize)]
    struct Entry {
        item: Item,
        state: Option<State>,
    }

    let rows: Vec<Entry> = read_json(
        &inventory_url(),
        inventory_api::ROUTE_INVENTORY,
        "inventory-catalogue",
    )?;
    Ok(rows
        .into_iter()
        .map(|row| (row.item.id.clone(), (row.item, row.state)))
        .collect())
}

pub fn comparisons() -> Result<Vec<GroupComparison>, Error> {
    #[derive(Deserialize)]
    struct Comparison {
        groups: Vec<GroupComparison>,
    }
    let comparison: Comparison = read_json(
        &inventory_url(),
        inventory_api::ROUTE_WISH_COMPARISON,
        "inventory-comparison",
    )?;
    Ok(comparison.groups)
}

pub fn record_price(id: &str, price: &RecordPriceRequest) -> Result<PriceRecorded, Error> {
    let response = request(
        &inventory_url(),
        inventory_api::ROUTE_ITEM_PRICES,
        "inventory-price",
        Method::POST,
        Some(id),
        Some(serde_json::to_vec(price)?),
    )?;
    Ok(serde_json::from_reader(response)?)
}

pub fn create_wish(item: Item) -> Result<(), Error> {
    let body = crate::api::NewItem {
        item,
        state: State::Wanted,
        note: Some("created with `inventory add`".into()),
    };
    request(
        &inventory_url(),
        inventory_api::ROUTE_ITEMS,
        "inventory-intake",
        Method::POST,
        None,
        Some(serde_json::to_vec(&body)?),
    )?;
    Ok(())
}

pub fn import_overlay(force: bool) -> Result<crate::import::Report, Error> {
    let response = request(
        &inventory_url(),
        inventory_api::ROUTE_IMPORT_OVERLAY,
        "inventory-import",
        Method::POST,
        None,
        Some(serde_json::to_vec(&inventory_api::ImportOverlayRequest {
            force,
        })?),
    )?;
    Ok(serde_json::from_reader(response)?)
}

pub fn set_specs(id: &str, specs: Vec<Spec>) -> Result<(), Error> {
    update(
        id,
        inventory_api::ROUTE_ITEM_SPECS,
        &SetSpecsRequest { specs },
    )
}

pub fn set_needs(id: &str, needs: Vec<Spec>) -> Result<(), Error> {
    update(
        id,
        inventory_api::ROUTE_WISH_GROUP_NEEDS,
        &SetNeedsRequest { needs },
    )
}

pub fn set_alternatives(id: &str, alternatives: bool) -> Result<(), Error> {
    update(
        id,
        inventory_api::ROUTE_WISH_GROUP_ALTERNATIVES,
        &SetAlternativesRequest { alternatives },
    )
}

fn update(id: &str, path: &str, body: &impl serde::Serialize) -> Result<(), Error> {
    request(
        &inventory_url(),
        path,
        "inventory-comparison",
        Method::PUT,
        Some(id),
        Some(serde_json::to_vec(body)?),
    )?;
    Ok(())
}

pub(crate) fn read_json<T: serde::de::DeserializeOwned>(
    base_url: &str,
    path: &str,
    purpose: &'static str,
) -> Result<T, Error> {
    let response = request(base_url, path, purpose, Method::GET, None, None)?;
    Ok(serde_json::from_reader(response)?)
}

fn request(
    base_url: &str,
    path: &str,
    purpose: &'static str,
    method: Method,
    id: Option<&str>,
    body: Option<Vec<u8>>,
) -> Result<impl Read, Error> {
    let client =
        sjel_http::loopback_builder(sjel_http::Purpose::new(purpose), Duration::from_secs(1))
            .build()?;
    let mut url = client.get(base_url).build()?.url().clone();
    if url.host_str() == Some("localhost") {
        url.set_host(Some("127.0.0.1"))?;
    }
    if !matches!(url.host_str(), Some("127.0.0.1" | "[::1]"))
        || !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("Capability URL must use a loopback address".into());
    }
    url.set_query(None);
    url.set_fragment(None);
    if let Some(id) = id {
        url.path_segments_mut()
            .map_err(|_| "Invalid capability URL")?
            .clear()
            .extend(path.trim_start_matches('/').split('/').map(|segment| {
                if segment == "{id}" {
                    id
                } else {
                    segment
                }
            }));
    } else {
        url.set_path(path);
    }
    let mut request = client.request(method, url.clone());
    if let Some(body) = body {
        request = request
            .header("Content-Type", "application/json")
            .body(body);
    }
    let request = sjel_server::InboundAuth::with_loopback_auth(request, url.as_str());
    let response = request.send()?;
    if !response.status().is_success() {
        let status = response.status();
        if status.is_client_error() {
            let mut body = Vec::new();
            response.take(1024).read_to_end(&mut body)?;
            let detail: String = String::from_utf8_lossy(&body)
                .chars()
                .filter(|c| !c.is_control())
                .collect();
            if !detail.is_empty() {
                return Err(format!("owner request failed: {status}: {detail}").into());
            }
        }
        return Err(format!("owner request failed: {status}").into());
    }
    Ok(response)
}
