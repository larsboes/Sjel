//! The detail HTTP contract and its local read cache. No trip records are written here.
use super::Plan;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Item {
    pub id: String,
    pub item_type: String,
    pub day: Option<String>,
    #[serde(default)]
    pub external_id: String,
    pub title: String,
    #[serde(default)]
    pub payload: serde_json::Value,
}

impl Item {
    /// Only fields understood by the itinerary are presented as item properties.
    pub fn fields(&self) -> Vec<(&'static str, String)> {
        [
            ("status", "Status"),
            ("time", "Time"),
            ("check_in", "Check-in"),
            ("check_out", "Check-out"),
            ("address", "Address"),
            ("provider", "Provider"),
            ("order_ref", "Reference"),
            ("free_cancellation_until", "Free cancellation until"),
            ("notes", "Notes"),
        ]
        .into_iter()
        .filter_map(|(key, label)| {
            self.payload
                .get(key)?
                .as_str()
                .filter(|s| !s.is_empty())
                .map(|s| (label, s.to_owned()))
        })
        .collect()
    }

    pub fn booking_code(&self) -> Option<&str> {
        self.payload
            .get("order_ref")
            .or_else(|| self.payload.get("booking_code"))
            .or_else(|| self.payload.get("reference"))
            .or_else(|| self.payload.get("pnr"))
            .and_then(serde_json::Value::as_str)
            .filter(|s| !s.is_empty())
    }

    pub fn source_url(&self) -> Option<reqwest::Url> {
        let url = self.payload.get("url")?.as_str()?;
        let url = reqwest::Url::parse(url).ok()?;
        (matches!(url.scheme(), "http" | "https")
            && url.username().is_empty()
            && url.password().is_none())
        .then_some(url)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Details {
    #[serde(flatten)]
    pub plan: Plan,
    pub items: Vec<Item>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SavedDetails {
    pub saved_at: u64,
    pub details: Details,
}

fn path(id: &str) -> Result<std::path::PathBuf, String> {
    super::details_path(id)?;
    // Hex encoding gives case-sensitive identifiers distinct files on a case-insensitive disk.
    let name: String = id.as_bytes().iter().map(|b| format!("{b:02x}")).collect();
    Ok(super::cache_dir()
        .ok_or("This device has no travel storage directory.")?
        .join("details")
        .join(format!("{name}.json")))
}

pub async fn saved_details(id: &str) -> Result<Option<SavedDetails>, String> {
    let path = path(id)?;
    let expected = id.to_owned();
    tokio::task::spawn_blocking(move || match std::fs::read(path) {
        Ok(bytes) => {
            let saved: SavedDetails = serde_json::from_slice(&bytes)
                .map_err(|e| format!("Read saved trip details: {e}"))?;
            if saved.details.plan.id != expected {
                return Err("The saved details belong to another trip.".into());
            }
            Ok(Some(saved))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("Read saved trip details: {e}")),
    })
    .await
    .map_err(|e| e.to_string())?
}

pub async fn save_details(details: Details) -> Result<SavedDetails, String> {
    let path = path(&details.plan.id)?;
    let saved_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_secs();
    let saved = SavedDetails { saved_at, details };
    let bytes = serde_json::to_vec(&saved).map_err(|e| e.to_string())?;
    tokio::task::spawn_blocking(move || -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let temporary = path.with_extension("tmp");
        std::fs::write(&temporary, bytes)?;
        std::fs::rename(temporary, path)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| format!("Save trip details: {e}"))?;
    Ok(saved)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(payload: serde_json::Value) -> Item {
        Item {
            id: "item_1".into(),
            item_type: "stay".into(),
            day: None,
            external_id: "stay:1".into(),
            title: "Stay".into(),
            payload,
        }
    }

    #[test]
    fn item_links_require_web_urls() {
        for url in [
            "javascript:alert(1)",
            "file:///tmp/example",
            "data:text/plain,test",
            "https://user:pass@example.com",
            "not a url",
        ] {
            assert!(item(serde_json::json!({"url": url})).source_url().is_none());
        }
        assert_eq!(
            item(serde_json::json!({"url": "https://example.com/stay"}))
                .source_url()
                .unwrap()
                .path(),
            "/stay"
        );
    }

    #[test]
    fn properties_use_known_string_fields() {
        let item = item(
            serde_json::json!({"status": "booked", "check_in": "2026-11-01", "address": 7, "notes": "", "unrecognized": "hidden"}),
        );
        assert_eq!(
            item.fields(),
            vec![
                ("Status", "booked".into()),
                ("Check-in", "2026-11-01".into())
            ]
        );
    }

    #[test]
    fn detail_contract_and_snapshot_round_trip() {
        let details: Details = serde_json::from_value(serde_json::json!({
            "id": "plan_1", "title": "Trip", "date_start": "2026-11-01", "date_end": "2026-11-04", "status": "planning",
            "origin": {"name": "Origin", "latitude": null, "longitude": null},
            "destinations": [], "stages": [],
            "items": [{"id": "item_1", "item_type": "stay", "day": "2026-11-02", "title": "Stay", "payload": {"status": "booked"}}],
            "retrospective": null
        })).unwrap();
        let saved = SavedDetails {
            saved_at: 123,
            details,
        };
        let restored: SavedDetails =
            serde_json::from_slice(&serde_json::to_vec(&saved).unwrap()).unwrap();
        assert_eq!(restored.saved_at, 123);
        assert_eq!(restored.details.plan.id, "plan_1");
        assert_eq!(restored.details.items[0].day.as_deref(), Some("2026-11-02"));
        assert!(super::super::details_path("../other").is_err());
        assert_eq!(
            super::super::details_path("plan_1").unwrap(),
            "/trips/api/plans/plan_1"
        );
        assert_eq!(
            super::super::details_path("trip:plan:1234abcd").unwrap(),
            "/trips/api/plans/trip:plan:1234abcd"
        );
    }
}
