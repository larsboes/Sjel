//! Loopback reads from Inventory and Finance.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::Deserialize;

use crate::store::{Item, State};

type Error = Box<dyn std::error::Error>;
pub type Catalogue = BTreeMap<String, (Item, Option<State>)>;

/// Read the item catalogue and current states. Call from a CLI or blocking worker.
pub fn catalogue() -> Result<Catalogue, Error> {
    #[derive(Deserialize)]
    struct Entry {
        item: Item,
        state: Option<State>,
    }

    let base = sjel_config::env_var("SJEL_INVENTORY_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:8101".into());
    let rows: Vec<Entry> = read_json(&base, inventory_api::ROUTE_INVENTORY, "inventory-catalogue")?;
    Ok(rows
        .into_iter()
        .map(|row| (row.item.id.clone(), (row.item, row.state)))
        .collect())
}

pub(crate) fn read_json<T: serde::de::DeserializeOwned>(
    base_url: &str,
    path: &str,
    purpose: &'static str,
) -> Result<T, Error> {
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
    url.set_path(path);
    url.set_query(None);
    url.set_fragment(None);
    let request =
        sjel_server::InboundAuth::with_loopback_auth(client.get(url.clone()), url.as_str());
    let response = request.send()?.error_for_status()?;
    if !response.status().is_success() {
        return Err("unexpected capability response status".into());
    }
    Ok(serde_json::from_reader(response)?)
}
