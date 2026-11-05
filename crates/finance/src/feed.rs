//! Reads from the comms feed over its loopback HTTP contract.
//!
//! Only id, title, url, day and the item's own summary or transcript are read, and only for
//! items whose stated class is no stricter than finance's rows (`item_is_quotable`).

use serde_json::Value;

/// Quotable feed items of the last `days` (comms' default when `None`), each with its summary.
pub fn read_feed(
    base_url: &str,
    days: Option<u32>,
) -> Vec<(crate::decision::FeedEvidence, String)> {
    if base_url.is_empty() {
        return Vec::new();
    }
    let Ok(client) = crate::price::blocking_client() else {
        return Vec::new();
    };
    let mut url = format!("{}/feed", base_url.trim_end_matches('/'));
    if let Some(days) = days {
        url.push_str(&format!("?days={days}"));
    }
    let request = sjel_server::InboundAuth::with_capability_loopback_auth(
        client.get(&url),
        &url,
        sjel_server::comms_config_token(),
    );
    let body = match request
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .and_then(reqwest::blocking::Response::text)
    {
        Ok(body) => body,
        Err(error) => {
            eprintln!("finance: comms feed read failed: {error}");
            return Vec::new();
        }
    };
    let items = match serde_json::from_str::<Vec<Value>>(&body) {
        Ok(items) => items,
        Err(error) => {
            eprintln!("finance: comms feed is not a JSON list: {error}");
            return Vec::new();
        }
    };
    items
        .iter()
        .filter(|item| item_is_quotable(item))
        .filter_map(|item| {
            Some((
                crate::decision::FeedEvidence {
                    id: item.get("id")?.as_str()?.to_string(),
                    title: item.get("title")?.as_str()?.to_string(),
                    url: item.get("url")?.as_str()?.to_string(),
                    day: item.get("day")?.as_str()?.to_string(),
                },
                item.get("summary")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            ))
        })
        .collect()
}

/// May this item's title and URL be copied into a `c1` row?
///
/// Only when comms states a class and that class is no stricter than the row's
/// own. `class_rank` is the single home of the ordering
/// (libs/content-item/src/lib.rs:350); comparing the strings here would be a
/// second copy of the vocabulary.
fn item_is_quotable(item: &Value) -> bool {
    let Some(stated) = item.get("data_class").and_then(Value::as_str) else {
        return false;
    };
    let (Some(stated), Some(row)) = (
        content_item::class_rank(stated),
        content_item::class_rank(crate::decision::DATA_CLASS),
    ) else {
        return false;
    };
    stated <= row
}

/// The text of one feed item for a model prompt: its summary, else its transcript, cut to
/// `max_chars`. `None` when comms does not answer or the item has neither.
pub fn read_item_text(base_url: &str, id: &str, max_chars: usize) -> Option<String> {
    if base_url.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    let client = crate::price::blocking_client().ok()?;
    let url = format!("{}/feed/{id}", base_url.trim_end_matches('/'));
    let item: Value = sjel_server::InboundAuth::with_capability_loopback_auth(
        client.get(&url),
        &url,
        sjel_server::comms_config_token(),
    )
    .send()
    .and_then(reqwest::blocking::Response::error_for_status)
    .and_then(reqwest::blocking::Response::json)
    .ok()?;
    let item = item.get("item").unwrap_or(&item);
    if !item_is_quotable(item) {
        return None;
    }
    let text = ["summary", "transcript"]
        .iter()
        .filter_map(|key| item.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .find(|text| !text.is_empty())?;
    Some(text.chars().take(max_chars).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The class bound on feed evidence is on the CLASS, not on the field count.
    /// An item comms does not classify is dropped rather than assumed public, and
    /// an item comms classifies above the row's own `c1` is dropped whatever its
    /// four fields say.
    #[test]
    fn only_an_item_comms_classifies_at_or_below_the_row_is_quotable() {
        let at_class = json!({ "id": "a", "title": "t", "url": "u", "day": "2026-09-05",
                               "data_class": "c1" });
        let public = json!({ "id": "b", "data_class": "c0" });
        let stricter = json!({ "id": "c", "data_class": "c2" });
        let unstated = json!({ "id": "d", "title": "t", "url": "u", "day": "2026-09-05" });
        let unknown = json!({ "id": "e", "data_class": "confidential" });
        assert!(item_is_quotable(&at_class));
        assert!(item_is_quotable(&public));
        assert!(!item_is_quotable(&stricter));
        assert!(!item_is_quotable(&unstated));
        assert!(!item_is_quotable(&unknown));
    }
}
