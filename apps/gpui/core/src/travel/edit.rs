//! Request bodies for `capabilities/trips`, checked before they leave. The capability does not
//! check a date's shape, so that check is here.
use chrono::NaiveDate;
use serde_json::{json, Map, Value};

use crate::capture::cents_from;

/// The trips capability takes one to four destinations (`validate_plan_fields`).
const MAX_DESTINATIONS: usize = 4;

/// The stage states a person sets by hand. `option_selected` and `open` are written by search.
pub const STAGE_STATUSES: &[(&str, &str)] = &[
    ("planning", "Planning"),
    ("booked", "Booked"),
    ("completed", "Done"),
];

/// `TransportMode` in `capabilities/trips/src/store.rs`, in the order a trip uses them most.
pub const TRANSPORT_MODES: &[(&str, &str)] = &[
    ("train", "Train"),
    ("flight", "Flight"),
    ("car", "Car"),
    ("bus", "Bus"),
    ("ferry", "Ferry"),
    ("walk", "Walk"),
    ("bike", "Bike"),
];

/// A typed place. The id rule matches `commitTypedPlace` in the dashboard's `PlaceField.svelte`.
pub fn typed_place(name: &str) -> Value {
    let name = name.trim();
    let mut slug = String::new();
    for c in name.to_lowercase().chars() {
        if c.is_alphanumeric() {
            slug.push(c);
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_matches('-');
    let id = if slug.is_empty() {
        format!("place:{}", urlencoding(name))
    } else {
        format!("place:{slug}")
    };
    json!({ "id": id, "name": name, "kind": "city", "latitude": null, "longitude": null })
}

/// JavaScript's `encodeURIComponent`, which the dashboard's rule falls back to.
fn urlencoding(text: &str) -> String {
    text.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
                char::from(b).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

fn date(label: &str, text: &str) -> Result<String, String> {
    let text = text.trim();
    NaiveDate::parse_from_str(text, "%Y-%m-%d")
        .map(|day| day.format("%Y-%m-%d").to_string())
        .map_err(|_| format!("{label} must be a date like 2026-11-01."))
}

fn optional_date(label: &str, text: &str) -> Result<Option<String>, String> {
    if text.trim().is_empty() {
        Ok(None)
    } else {
        date(label, text).map(Some)
    }
}

fn required(label: &str, text: &str) -> Result<String, String> {
    let text = text.trim();
    if text.is_empty() {
        Err(format!("{label} is required."))
    } else {
        Ok(text.to_owned())
    }
}

/// The new-trip form, as typed.
#[derive(Clone, Debug, Default)]
pub struct TripDraft {
    pub title: String,
    pub origin: String,
    /// Comma-separated, in travel order.
    pub destinations: String,
    pub date_start: String,
    pub date_end: String,
}

impl TripDraft {
    /// `POST /trips/api/plans`. The capability generates one stage per destination.
    pub fn body(&self) -> Result<Value, String> {
        let title = required("A title", &self.title)?;
        let origin = required("Where it starts", &self.origin)?;
        let destinations: Vec<&str> = self
            .destinations
            .split(',')
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .collect();
        if destinations.is_empty() || destinations.len() > MAX_DESTINATIONS {
            return Err("Name one to four destinations, separated by commas.".into());
        }
        let start = date("The start", &self.date_start)?;
        let end = date("The end", &self.date_end)?;
        if start > end {
            return Err("The trip ends before it starts.".into());
        }
        Ok(json!({
            "title": title,
            "origin": typed_place(&origin),
            "destinations": destinations.into_iter().map(typed_place).collect::<Vec<_>>(),
            "date_start": start,
            "date_end": end,
        }))
    }
}

/// A checked booking: its title, its day, and the payload keys the form owns.
type Booking = (String, Option<String>, Map<String, Value>);

/// The booking form, as typed. `provider` and `order_ref` are what the capability requires
/// (`DECLARED_PAYLOADS` in `capabilities/trips/src/store.rs`).
#[derive(Clone, Debug, Default)]
pub struct BookingDraft {
    pub title: String,
    pub provider: String,
    pub order_ref: String,
    pub day: String,
    /// `"49,90"` or `"49.90"`, in `currency`.
    pub amount: String,
    pub currency: String,
    /// The calendar turns this into a deadline entry (`capabilities/calendar/src/server.rs`).
    pub free_cancellation_until: String,
}

impl BookingDraft {
    /// Reads a saved booking back into the form.
    pub fn from_item(item: &super::Item) -> Self {
        let text = |key: &str| {
            item.payload
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        let amount = item
            .payload
            .get("amount_cents")
            .and_then(Value::as_i64)
            .map(|cents| format!("{}.{:02}", cents / 100, cents % 100))
            .unwrap_or_default();
        Self {
            title: item.title.clone(),
            provider: text("provider"),
            order_ref: text("order_ref"),
            day: item.day.clone().unwrap_or_default(),
            amount,
            currency: text("currency"),
            free_cancellation_until: text("free_cancellation_until"),
        }
    }

    /// Every payload key the form owns. An empty optional field is `null`, which
    /// `PATCH …/fields` reads as "remove the key" (`editItem` in `dashboard/src/lib/api/travel.ts`).
    fn payload(&self) -> Result<Booking, String> {
        let provider = required("The provider", &self.provider)?;
        let order_ref = required("The booking reference", &self.order_ref)?;
        let title = if self.title.trim().is_empty() {
            format!("{provider} {order_ref}")
        } else {
            self.title.trim().to_owned()
        };
        let day = optional_date("The day", &self.day)?;
        let cancel = optional_date("Free cancellation", &self.free_cancellation_until)?;
        let amount = match self.amount.trim() {
            "" => None,
            text => Some(cents_from(text).ok_or("The amount must be a number like 49,90.")?),
        };
        let currency = self.currency.trim().to_uppercase();
        let currency = match (amount, currency.as_str()) {
            (_, "") if amount.is_some() => {
                return Err("Name the currency of the amount, like EUR.".into())
            }
            (_, "") => None,
            (_, code) if code.len() == 3 && code.bytes().all(|b| b.is_ascii_uppercase()) => {
                Some(code.to_owned())
            }
            _ => return Err("The currency must be a three-letter code, like EUR.".into()),
        };
        let mut payload = Map::new();
        payload.insert("provider".into(), provider.into());
        payload.insert("order_ref".into(), order_ref.into());
        payload.insert(
            "amount_cents".into(),
            amount.map_or(Value::Null, Value::from),
        );
        payload.insert("currency".into(), currency.map_or(Value::Null, Value::from));
        payload.insert(
            "free_cancellation_until".into(),
            cancel.map_or(Value::Null, Value::from),
        );
        Ok((title, day, payload))
    }

    /// `POST /trips/api/plans/{id}/items`. The external id is the provider's reference, so the
    /// same booking entered twice updates one row (`UNIQUE (plan_id, item_type, external_id)`).
    pub fn create_body(&self) -> Result<Value, String> {
        let (title, day, mut payload) = self.payload()?;
        payload.retain(|_, value| !value.is_null());
        let external_id = format!(
            "booking:{}:{}",
            payload["provider"].as_str().unwrap_or_default(),
            payload["order_ref"].as_str().unwrap_or_default()
        );
        Ok(json!({
            "item_type": "booking",
            "day": day,
            "external_id": external_id,
            "title": title,
            "payload": payload,
        }))
    }

    /// `PATCH /trips/api/plans/{id}/items/{item}/fields`, and the day for the separate
    /// `PATCH …/items/{item}`, which is the only route that moves an item between days.
    pub fn edit_body(&self) -> Result<(Value, Option<String>), String> {
        let (title, day, payload) = self.payload()?;
        Ok((json!({ "title": title, "payload": payload }), day))
    }
}

/// The stage form, as typed.
#[derive(Clone, Debug)]
pub struct StageDraft {
    pub origin: String,
    pub destination: String,
    pub date: String,
    pub mode: &'static str,
    pub status: &'static str,
}

impl Default for StageDraft {
    fn default() -> Self {
        Self {
            origin: String::new(),
            destination: String::new(),
            date: String::new(),
            mode: TRANSPORT_MODES[0].0,
            status: STAGE_STATUSES[0].0,
        }
    }
}

/// What one form submission does to a plan's stage list.
#[derive(Clone, Debug)]
pub enum StageEdit {
    Add(StageDraft),
    Change(String, StageDraft),
    Remove(String),
}

impl StageDraft {
    /// Reads a saved stage back into the form.
    pub fn from_stage(stage: &super::Stage) -> Self {
        let pick = |options: &[(&'static str, &'static str)], value: &str| {
            options
                .iter()
                .find(|(key, _)| *key == value)
                .map_or(options[0].0, |(key, _)| key)
        };
        Self {
            origin: stage.origin.name.clone(),
            destination: stage.destination.name.clone(),
            date: stage.date.clone().unwrap_or_default(),
            mode: pick(
                TRANSPORT_MODES,
                stage.transport_modes.first().map_or("", String::as_str),
            ),
            status: pick(STAGE_STATUSES, &stage.status),
        }
    }

    /// The field checks [`StageEdit::apply`] makes, without a plan to apply them to.
    pub fn check(&self) -> Result<(), String> {
        self.apply(&mut Map::new())
    }

    /// Writes the typed fields into `stage`, leaving every field this form does not own
    /// (travelers, the chosen connection, the branch note) as the record had it. A place whose
    /// name did not change keeps its saved id and coordinates.
    fn apply(&self, stage: &mut Map<String, Value>) -> Result<(), String> {
        let origin = required("Where the stage starts", &self.origin)?;
        let destination = required("Where the stage ends", &self.destination)?;
        let day = optional_date("The date", &self.date)?;
        for (key, name) in [("origin", origin), ("destination", destination)] {
            let same = stage
                .get(key)
                .and_then(|place| place.get("name"))
                .and_then(Value::as_str)
                == Some(name.as_str());
            if !same {
                stage.insert(key.into(), typed_place(&name));
            }
        }
        stage.insert("date".into(), day.map_or(Value::Null, Value::from));
        stage.insert("transport_modes".into(), json!([self.mode]));
        stage.insert("status".into(), self.status.into());
        Ok(())
    }
}

impl StageEdit {
    /// Applies the edit to the plan's `stages` exactly as the capability returned them, so the
    /// array written back differs only where the form changed it. Sequences are renumbered in
    /// order, because the itinerary sorts on them.
    pub fn apply(&self, stages: &mut Vec<Value>) -> Result<(), String> {
        let position = |id: &str, stages: &[Value]| {
            stages
                .iter()
                .position(|stage| stage.get("id").and_then(Value::as_str) == Some(id))
                .ok_or_else(|| {
                    "This stage is no longer on the trip. Refresh and try again.".to_owned()
                })
        };
        match self {
            Self::Add(draft) => {
                let next = stages
                    .iter()
                    .filter_map(|stage| {
                        stage
                            .get("id")?
                            .as_str()?
                            .strip_prefix("stage:")?
                            .parse::<u64>()
                            .ok()
                    })
                    .max()
                    .unwrap_or(0)
                    + 1;
                let mut stage = Map::new();
                stage.insert("id".into(), format!("stage:{next}").into());
                stage.insert("travelers".into(), json!([]));
                draft.apply(&mut stage)?;
                stages.push(Value::Object(stage));
            }
            Self::Change(id, draft) => {
                let at = position(id, stages)?;
                let Some(stage) = stages[at].as_object_mut() else {
                    return Err("This stage cannot be read.".into());
                };
                draft.apply(stage)?;
            }
            Self::Remove(id) => {
                let at = position(id, stages)?;
                stages.remove(at);
            }
        }
        for (sequence, stage) in stages.iter_mut().enumerate() {
            if let Some(stage) = stage.as_object_mut() {
                stage.insert("sequence".into(), sequence.into());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_typed_place_gets_the_dashboard_id() {
        assert_eq!(typed_place(" Köln Hbf ")["id"], "place:köln-hbf");
        assert_eq!(typed_place("São Paulo")["id"], "place:são-paulo");
        assert_eq!(typed_place("!!")["id"], "place:!!");
        assert_eq!(typed_place("@ #")["id"], "place:%40%20%23");
        assert_eq!(typed_place("Bonn")["latitude"], Value::Null);
    }

    #[test]
    fn a_trip_needs_a_title_dates_in_order_and_one_to_four_stops() {
        let draft = TripDraft {
            title: "Hamburg".into(),
            origin: "Bonn".into(),
            destinations: "Hamburg, Lübeck,".into(),
            date_start: "2026-11-01".into(),
            date_end: "2026-11-04".into(),
        };
        let body = draft.body().unwrap();
        assert_eq!(body["destinations"].as_array().unwrap().len(), 2);
        assert_eq!(body["destinations"][1]["id"], "place:lübeck");
        assert_eq!(body["origin"]["name"], "Bonn");

        let late = TripDraft {
            date_end: "2026-10-31".into(),
            ..draft.clone()
        };
        assert_eq!(late.body().unwrap_err(), "The trip ends before it starts.");
        let typed = TripDraft {
            date_start: "1.11.2026".into(),
            ..draft.clone()
        };
        assert!(typed
            .body()
            .unwrap_err()
            .starts_with("The start must be a date"));
        let many = TripDraft {
            destinations: "A, B, C, D, E".into(),
            ..draft.clone()
        };
        assert!(many.body().is_err());
        let none = TripDraft {
            destinations: " , ".into(),
            ..draft
        };
        assert!(none.body().is_err());
    }

    #[test]
    fn a_booking_carries_what_the_capability_requires_and_drops_empty_fields() {
        let draft = BookingDraft {
            provider: "DB".into(),
            order_ref: "ABC123".into(),
            amount: "49,90".into(),
            currency: "eur".into(),
            ..Default::default()
        };
        let body = draft.create_body().unwrap();
        assert_eq!(body["item_type"], "booking");
        assert_eq!(body["external_id"], "booking:DB:ABC123");
        assert_eq!(body["title"], "DB ABC123");
        assert_eq!(body["payload"]["amount_cents"], 4990);
        assert_eq!(body["payload"]["currency"], "EUR");
        assert!(body["payload"].get("free_cancellation_until").is_none());
        assert_eq!(body["day"], Value::Null);

        // An edit sends the empty field as null, which removes the key on the Mac.
        let (edit, day) = draft.edit_body().unwrap();
        assert_eq!(edit["payload"]["free_cancellation_until"], Value::Null);
        assert_eq!(day, None);

        let no_ref = BookingDraft {
            order_ref: " ".into(),
            ..draft.clone()
        };
        assert_eq!(
            no_ref.create_body().unwrap_err(),
            "The booking reference is required."
        );
        let no_currency = BookingDraft {
            currency: String::new(),
            ..draft.clone()
        };
        assert!(no_currency.create_body().unwrap_err().contains("currency"));
        let bad_amount = BookingDraft {
            amount: "lots".into(),
            ..draft
        };
        assert!(bad_amount.create_body().is_err());
    }

    #[test]
    fn a_saved_booking_reads_back_into_the_same_form() {
        let item: super::super::Item = serde_json::from_value(json!({
            "id": "item_1", "item_type": "booking", "day": "2026-11-01", "title": "ICE 578",
            "payload": {"provider": "DB", "order_ref": "ABC123", "amount_cents": 4905, "currency": "EUR"}
        }))
        .unwrap();
        let draft = BookingDraft::from_item(&item);
        assert_eq!(draft.amount, "49.05");
        let body = draft.create_body().unwrap();
        assert_eq!(body["payload"]["amount_cents"], 4905);
        assert_eq!(body["day"], "2026-11-01");
    }

    #[test]
    fn a_stage_edit_keeps_what_the_form_does_not_own() {
        let mut stages = vec![
            json!({"id": "stage:1", "sequence": 0, "status": "booked", "date": "2026-11-01",
                   "origin": {"id": "place:bonn", "name": "Bonn", "latitude": 50.7, "longitude": 7.1},
                   "destination": {"id": "place:hamburg", "name": "Hamburg"},
                   "transport_modes": ["train"], "travelers": ["Ada"], "selected_option_id": "j1"}),
            json!({"id": "stage:2", "sequence": 1, "status": "planning", "date": null,
                   "origin": {"id": "place:hamburg", "name": "Hamburg"},
                   "destination": {"id": "place:lübeck", "name": "Lübeck"},
                   "transport_modes": [], "travelers": []}),
        ];
        let change = StageDraft {
            origin: "Bonn".into(),
            destination: "Kiel".into(),
            date: "2026-11-02".into(),
            mode: "bus",
            status: "completed",
        };
        StageEdit::Change("stage:1".into(), change.clone())
            .apply(&mut stages)
            .unwrap();
        assert_eq!(stages[0]["travelers"], json!(["Ada"]));
        assert_eq!(stages[0]["selected_option_id"], "j1");
        assert_eq!(
            stages[0]["origin"]["latitude"], 50.7,
            "an unchanged place keeps its coordinate"
        );
        assert_eq!(stages[0]["destination"]["id"], "place:kiel");
        assert_eq!(stages[0]["transport_modes"], json!(["bus"]));

        StageEdit::Add(change).apply(&mut stages).unwrap();
        assert_eq!(stages[2]["id"], "stage:3");
        assert_eq!(stages[2]["sequence"], 2);

        StageEdit::Remove("stage:1".into())
            .apply(&mut stages)
            .unwrap();
        assert_eq!(stages.len(), 2);
        assert_eq!(stages[0]["id"], "stage:2");
        assert_eq!(stages[0]["sequence"], 0);
        assert!(StageEdit::Remove("stage:9".into())
            .apply(&mut stages)
            .is_err());
    }
}
