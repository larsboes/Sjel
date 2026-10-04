//! Entry, rhythm and export-ledger types plus their validation. The database stores kinds
//! and sources as free TEXT (no CHECK constraint) — the reasoning lives in
//! the README's "Why this shape: kinds are data, not a constraint"; this
//! module validates shape (token-safe, non-empty), not membership.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::date;

/// Well-known entry kinds. Not exhaustive and not enforced — new kinds (e.g.
/// day-planning blocks, Phase F) land without a migration. The correlation
/// layer (Phase C) maps kinds onto feasibility verdicts; anything unknown
/// there is treated as neutral.
pub const KNOWN_KINDS: &[&str] = &[
    "busy",        // hard block, whatever the reason
    "work_onsite", // on-site work; `location` says where (e.g. the office city)
    "work_remote", // working, but location-flexible
    "away",        // not at home, not necessarily work
    "event",       // attending something concrete (a saved Luma event lands here)
    "nightlife",   // party, club or open-air event with a concrete time/place
    "deadline",    // a dated action or due date; visible evidence, never a time block
    "travel_ok",   // explicit "up for trips in this window" signal — a boost,
                   // never a requirement: open-by-default means an empty day
                   // already counts as feasible
];
// `draft` used to sit in this list. It was never a kind: it answered "how sure
// is this", not "what is this", and it only ever covered the Google case. That
// question is `Commitment` now, so an unadopted import keeps a real kind and
// carries `Commitment::Possible`; the same neutrality falls out of the matrix
// instead of a named special case.

/// How binding an entry is. Orthogonal to `kind`: a holiday can be an idea or
/// a booked flight, and an event can be a bookmark or a paid ticket. `kind`
/// says what it is; this says whether it is happening.
///
/// Unlike kinds, this is a *closed* set. Kinds stay open because an unknown
/// kind has a safe reading (neutral); an unknown commitment has none — the
/// entire job of the field is to decide how hard a day is blocked, so a value
/// nobody defined cannot be waved through. `Ord` runs least- to most-binding,
/// which is what makes "worst commitment on a day" a `max()`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default, Hash,
)]
#[serde(rename_all = "lowercase")]
pub enum Commitment {
    /// On the radar. Scouting found it, a friend mentioned it, you bookmarked
    /// it. Never blocks a day; always shows up as evidence.
    #[default]
    Possible,
    /// You decided, but nothing is booked. Blocks softly, with the cost named.
    Planned,
    /// Ticket bought, registration in, leave approved. Only here does `kind`
    /// get to make a day hard.
    Committed,
}

pub const COMMITMENTS: &[&str] = &["possible", "planned", "committed"];

impl Commitment {
    pub fn as_str(self) -> &'static str {
        match self {
            Commitment::Possible => "possible",
            Commitment::Planned => "planned",
            Commitment::Committed => "committed",
        }
    }

    /// Reading a stored value. Total, and lenient in exactly one direction: a
    /// row written by an older binary reads as `Possible`. That is the safe
    /// failure direction for a planning system — a bug hands you a free day
    /// you can check yourself, never a silently blocked one you never see.
    /// Rejecting bad *input* is the API boundary's job (serde), not the
    /// reader's.
    pub fn from_db(text: &str) -> Self {
        match text {
            "committed" => Commitment::Committed,
            "planned" => Commitment::Planned,
            _ => Commitment::Possible,
        }
    }
}

/// Well-known entry sources. Same free-TEXT reasoning as kinds.
pub const KNOWN_SOURCES: &[&str] = &[
    "manual",   // painted or typed in the dashboard
    "rhythm",   // materialized from a rhythm rule
    "feed",     // explicitly promoted from a Comms Feed item
    "comms",    // explicitly proposed from a reviewed Comms content analysis
    "scouting", // explicitly promoted from a Scouting opportunity
    "luma",     // imported from a Luma calendar (Phase A/C)
    "web",      // one reviewed public event page, URL retained in payload
    "google",   // imported from Google Calendar (Phase E)
];

fn default_source() -> String {
    "manual".into()
}

/// A kind/source token: short, lowercase, machine-safe. Membership in
/// KNOWN_* is deliberately not required.
fn valid_token(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 40
        && text
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    pub id: String,
    pub kind: String,
    pub commitment: Commitment,
    pub title: String,
    pub starts_at: String,
    /// Exclusive end, always: an all-day entry covering only 2026-08-14 has
    /// `starts_at = "2026-08-14"`, `ends_at = "2026-08-15"`.
    pub ends_at: String,
    pub all_day: bool,
    pub location: Option<String>,
    pub notes: Option<String>,
    pub source: String,
    pub external_id: Option<String>,
    pub rhythm_id: Option<String>,
    /// Inert provider evidence (the Luma event JSON, later a Google event).
    /// Never executed, never part of the durable contract — same pattern as
    /// trips.plan_items.payload.
    pub payload: Value,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NewEntry {
    pub kind: String,
    /// Omitted means `possible`. A write that does not say how binding it is
    /// is under-specified, and under-specified must not block a day — every
    /// caller that knows better (the dashboard form, the Google import) says
    /// so explicitly.
    #[serde(default)]
    pub commitment: Commitment,
    pub title: String,
    pub starts_at: String,
    pub ends_at: String,
    #[serde(default)]
    pub all_day: bool,
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default = "default_source")]
    pub source: String,
    #[serde(default)]
    pub external_id: Option<String>,
    #[serde(default)]
    pub rhythm_id: Option<String>,
    #[serde(default)]
    pub payload: Value,
}

fn present_nullable<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

/// PATCH shape. Nullable fields distinguish omission ("leave it alone") from
/// an explicit JSON null ("clear it"). This matters once the dashboard can
/// edit imported entries without accidentally retaining an old place/note.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct UpdateEntry {
    pub kind: Option<String>,
    /// Raising this is how "I might" becomes "I'm going". Promotion never
    /// touches it — see the upsert's DO UPDATE list in `store`.
    pub commitment: Option<Commitment>,
    pub title: Option<String>,
    pub starts_at: Option<String>,
    pub ends_at: Option<String>,
    pub all_day: Option<bool>,
    #[serde(
        default,
        deserialize_with = "present_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub location: Option<Option<String>>,
    #[serde(
        default,
        deserialize_with = "present_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub notes: Option<Option<String>>,
}

// Override semantics: ANY patch to a rhythm-linked entry detaches it
// (`rhythm_id` → NULL in store::update_entry) — re-materialization deletes and
// regenerates linked instances, so even a notes edit would otherwise be lost.

impl NewEntry {
    pub fn validate(&self) -> Result<(), String> {
        if !valid_token(&self.kind) {
            return Err("kind must be 1-40 chars of [a-z0-9_]".into());
        }
        if self.title.trim().is_empty() {
            return Err("title is required".into());
        }
        if !valid_token(&self.source) {
            return Err("source must be 1-40 chars of [a-z0-9_]".into());
        }
        let (start_day, start_time) =
            date::parse_instant(&self.starts_at).ok_or("starts_at must be a date or local time")?;
        let (end_day, end_time) =
            date::parse_instant(&self.ends_at).ok_or("ends_at must be a date or local time")?;
        if self.all_day && (start_time.is_some() || end_time.is_some()) {
            return Err("all_day entries take date-only starts_at/ends_at".into());
        }
        if !self.all_day && (start_time.is_none() || end_time.is_none()) {
            return Err("timed entries need HH:MM in starts_at/ends_at".into());
        }
        if (end_day, end_time) <= (start_day, start_time) {
            return Err("ends_at must be after starts_at (ends are exclusive)".into());
        }
        if self.source == "comms" {
            if self.commitment != Commitment::Possible {
                return Err("Comms contributions must remain possible until reviewed".into());
            }
            if !self.all_day || end_day != start_day + 1 {
                return Err("Comms contributions must be one-day all-day proposals".into());
            }
            if !matches!(self.kind.as_str(), "event" | "deadline") {
                return Err("Comms proposals must be an event or deadline".into());
            }
            if self
                .external_id
                .as_deref()
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .is_none()
            {
                return Err("Comms proposals require an external_id".into());
            }
            let payload = self
                .payload
                .as_object()
                .filter(|payload| {
                    payload.get("schema_version").and_then(Value::as_str)
                        == Some("calendar-proposal-provenance-v1")
                        && matches!(
                            payload.get("data_class").and_then(Value::as_str),
                            Some("c0" | "c1" | "c2" | "c3")
                        )
                        && matches!(
                            payload.get("importance").and_then(Value::as_str),
                            Some("low" | "medium" | "high")
                        )
                        && matches!(
                            payload
                                .get("analysis_schema_version")
                                .and_then(Value::as_str),
                            Some("cloud-content-analysis-v1" | "local-mail-event-analysis-v1")
                        )
                        && payload
                            .get("importance_rationale")
                            .and_then(Value::as_str)
                            .is_some_and(|value| {
                                !value.trim().is_empty() && value.chars().count() <= 600
                            })
                })
                .ok_or("Comms proposal provenance is invalid")?;
            payload
                .get("origin")
                .and_then(Value::as_object)
                .filter(|origin| {
                    origin.get("capability").and_then(Value::as_str) == Some("comms")
                        && matches!(
                            origin.get("source").and_then(Value::as_str),
                            Some("feed" | "mail")
                        )
                        && origin
                            .get("item_id")
                            .and_then(Value::as_str)
                            .is_some_and(|value| !value.trim().is_empty())
                        && match (
                            payload
                                .get("analysis_schema_version")
                                .and_then(Value::as_str),
                            origin.get("job_id"),
                        ) {
                            (Some("local-mail-event-analysis-v1"), Some(Value::Null)) => true,
                            (Some("cloud-content-analysis-v1"), Some(Value::String(value))) => {
                                !value.trim().is_empty()
                            }
                            _ => false,
                        }
                        && matches!(
                            (
                                payload
                                    .get("analysis_schema_version")
                                    .and_then(Value::as_str),
                                origin.get("field").and_then(Value::as_str),
                            ),
                            (Some("local-mail-event-analysis-v1"), Some("mail_events"))
                                | (
                                    Some("cloud-content-analysis-v1"),
                                    Some("important_dates" | "action_items")
                                )
                        )
                        && origin
                            .get("index")
                            .and_then(Value::as_u64)
                            .is_some_and(|value| value < 10)
                })
                .ok_or("Comms proposal origin is invalid")?;
            if let Some(evidence) = payload.get("evidence") {
                match evidence {
                    Value::Null => {}
                    Value::String(value) if value.chars().count() <= 300 => {}
                    _ => return Err("Comms proposal evidence is invalid".into()),
                }
            }
        }
        Ok(())
    }
}

/// A soft fact that matters only during a bounded date range.
///
/// Contexts deliberately live beside entries rather than inside them. They
/// inform planning ("the colloquium can land in this window") without
/// occupying time or changing a feasibility verdict.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Context {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub details: String,
    pub valid_from: String,
    pub valid_until: String,
    pub source: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NewContext {
    pub kind: String,
    pub title: String,
    #[serde(default)]
    pub details: String,
    pub valid_from: String,
    pub valid_until: String,
    #[serde(default = "default_source")]
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct UpdateContext {
    pub kind: Option<String>,
    pub title: Option<String>,
    pub details: Option<String>,
    pub valid_from: Option<String>,
    pub valid_until: Option<String>,
}

pub fn validate_context_fields(
    kind: &str,
    title: &str,
    valid_from: &str,
    valid_until: &str,
    source: &str,
) -> Result<(), String> {
    if !valid_token(kind) {
        return Err("kind must be 1-40 chars of [a-z0-9_]".into());
    }
    if title.trim().is_empty() {
        return Err("title is required".into());
    }
    if !valid_token(source) {
        return Err("source must be 1-40 chars of [a-z0-9_]".into());
    }
    let from = date::parse_date(valid_from).ok_or("valid_from must be a date")?;
    let until = date::parse_date(valid_until).ok_or("valid_until must be a date")?;
    if until < from {
        return Err("valid_until must be on or after valid_from".into());
    }
    Ok(())
}

impl NewContext {
    pub fn validate(&self) -> Result<(), String> {
        validate_context_fields(
            &self.kind,
            &self.title,
            &self.valid_from,
            &self.valid_until,
            &self.source,
        )
    }
}

/// One row of the Google export ledger (`calendar.google_exports`).
///
/// The row's existence is the per-entry opt-in — nothing exports by default,
/// and opting out deletes it. `google_event_id` is `None` until the first push
/// succeeds; after that it is what makes the next push an update rather than a
/// second event on the Google side.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExportOptIn {
    pub entry_id: String,
    pub google_calendar_id: String,
    pub google_event_id: Option<String>,
    pub pushed_at: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Rhythm {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub location: Option<String>,
    /// Weekday tokens ("mo".."su"), at least one.
    pub byweekday: Vec<String>,
    /// "HH:MM" wall times; both unset means all-day instances.
    pub start_time: Option<String>,
    pub end_time: Option<String>,
    pub valid_from: String,
    pub valid_until: String,
    pub active: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NewRhythm {
    pub kind: String,
    pub title: String,
    #[serde(default)]
    pub location: Option<String>,
    pub byweekday: Vec<String>,
    #[serde(default)]
    pub start_time: Option<String>,
    #[serde(default)]
    pub end_time: Option<String>,
    pub valid_from: String,
    pub valid_until: String,
    #[serde(default = "default_active")]
    pub active: bool,
}

fn default_active() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct UpdateRhythm {
    pub kind: Option<String>,
    pub title: Option<String>,
    pub location: Option<String>,
    pub byweekday: Option<Vec<String>>,
    pub start_time: Option<String>,
    pub end_time: Option<String>,
    pub valid_from: Option<String>,
    pub valid_until: Option<String>,
    pub active: Option<bool>,
}

pub fn validate_rhythm_fields(
    kind: &str,
    title: &str,
    byweekday: &[String],
    start_time: Option<&str>,
    end_time: Option<&str>,
    valid_from: &str,
    valid_until: &str,
) -> Result<(), String> {
    if !valid_token(kind) {
        return Err("kind must be 1-40 chars of [a-z0-9_]".into());
    }
    if title.trim().is_empty() {
        return Err("title is required".into());
    }
    if byweekday.is_empty() {
        return Err("byweekday needs at least one weekday".into());
    }
    for token in byweekday {
        if date::parse_weekday(token).is_none() {
            return Err(format!(
                "byweekday token must be one of mo,tu,we,th,fr,sa,su: {token}"
            ));
        }
    }
    match (start_time, end_time) {
        (Some(start), Some(end)) => {
            let start = date::parse_time(start).ok_or("start_time must be HH:MM")?;
            let end = date::parse_time(end).ok_or("end_time must be HH:MM")?;
            if end <= start {
                return Err("end_time must be after start_time".into());
            }
        }
        (None, None) => {}
        _ => return Err("start_time and end_time come as a pair, or neither (all-day)".into()),
    }
    let from = date::parse_date(valid_from).ok_or("valid_from must be a date")?;
    let until = date::parse_date(valid_until).ok_or("valid_until must be a date")?;
    if until < from {
        return Err("valid_until must be on or after valid_from".into());
    }
    Ok(())
}

impl NewRhythm {
    pub fn validate(&self) -> Result<(), String> {
        validate_rhythm_fields(
            &self.kind,
            &self.title,
            &self.byweekday,
            self.start_time.as_deref(),
            self.end_time.as_deref(),
            &self.valid_from,
            &self.valid_until,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry() -> NewEntry {
        NewEntry {
            kind: "busy".into(),
            commitment: Commitment::Committed,
            title: "Dentist".into(),
            starts_at: "2026-08-14T09:00".into(),
            ends_at: "2026-08-14T10:00".into(),
            all_day: false,
            location: None,
            notes: None,
            source: "manual".into(),
            external_id: None,
            rhythm_id: None,
            payload: Value::Null,
        }
    }

    #[test]
    fn all_day_entry_needs_exclusive_end_date() {
        let mut all_day = entry();
        all_day.all_day = true;
        all_day.starts_at = "2026-08-14".into();
        all_day.ends_at = "2026-08-14".into();
        assert!(
            all_day.validate().is_err(),
            "same-day exclusive end is empty"
        );
        all_day.ends_at = "2026-08-15".into();
        assert!(all_day.validate().is_ok());
    }

    #[test]
    fn kind_and_time_shape_are_checked_not_membership() {
        let mut future_kind = entry();
        future_kind.kind = "deep_work".into(); // unknown but well-formed: fine
        assert!(future_kind.validate().is_ok());
        future_kind.kind = "Deep Work!".into();
        assert!(future_kind.validate().is_err());
        let mut bad_time = entry();
        bad_time.ends_at = "2026-08-14T08:00".into();
        assert!(bad_time.validate().is_err());
    }

    #[test]
    fn all_day_flag_must_match_string_shape() {
        let mut mismatched = entry();
        mismatched.all_day = true; // but starts_at still carries T09:00
        assert!(mismatched.validate().is_err());
        let mut date_only_timed = entry();
        date_only_timed.starts_at = "2026-08-14".into();
        date_only_timed.ends_at = "2026-08-15".into();
        assert!(date_only_timed.validate().is_err());
    }

    #[test]
    fn comms_contributions_are_bounded_review_proposals() {
        let mut proposal = entry();
        proposal.kind = "deadline".into();
        proposal.commitment = Commitment::Possible;
        proposal.starts_at = "2026-08-10".into();
        proposal.ends_at = "2026-08-11".into();
        proposal.all_day = true;
        proposal.source = "comms".into();
        proposal.external_id = Some("content-analysis:mail:thread-1:action:2026-08-10".into());
        proposal.payload = serde_json::json!({
            "schema_version": "calendar-proposal-provenance-v1",
            "origin": {
                "capability": "comms",
                "source": "mail",
                "item_id": "thread-1",
                "job_id": "job-1",
                "field": "action_items",
                "index": 0
            },
            "data_class": "c1",
            "analysis_schema_version": "cloud-content-analysis-v1",
            "importance": "high",
            "importance_rationale": "A dated action needs review.",
            "evidence": null
        });
        assert!(proposal.validate().is_ok());

        proposal.payload["origin"]["job_id"] = Value::Null;
        proposal.payload["origin"]["field"] = Value::String("mail_events".into());
        proposal.payload["analysis_schema_version"] =
            Value::String("local-mail-event-analysis-v1".into());
        assert!(proposal.validate().is_ok());
        proposal.payload["origin"]["field"] = Value::String("important_dates".into());
        assert!(proposal.validate().is_err());
        proposal.payload["origin"]["field"] = Value::String("mail_events".into());
        proposal.payload["origin"]["job_id"] = Value::String("invented-job".into());
        assert!(proposal.validate().is_err());

        proposal.commitment = Commitment::Planned;
        assert_eq!(
            proposal.validate().unwrap_err(),
            "Comms contributions must remain possible until reviewed"
        );
    }

    #[test]
    fn rhythm_validation() {
        let rhythm = NewRhythm {
            kind: "work_onsite".into(),
            title: "Office days".into(),
            location: Some("Office".into()),
            byweekday: vec!["tu".into(), "we".into(), "th".into()],
            start_time: None,
            end_time: None,
            valid_from: "2026-09-01".into(),
            valid_until: "2026-10-31".into(),
            active: true,
        };
        assert!(rhythm.validate().is_ok());

        let mut half_timed = rhythm.clone();
        half_timed.start_time = Some("09:00".into());
        assert!(half_timed.validate().is_err(), "time must come as a pair");

        let mut backwards = rhythm.clone();
        backwards.valid_until = "2026-08-01".into();
        assert!(backwards.validate().is_err());

        let mut no_days = rhythm.clone();
        no_days.byweekday = vec![];
        assert!(no_days.validate().is_err());
    }

    #[test]
    fn update_entry_default_is_empty() {
        assert_eq!(UpdateEntry::default(), UpdateEntry::default());
    }

    #[test]
    fn update_entry_distinguishes_missing_from_null() {
        let missing: UpdateEntry = serde_json::from_str("{}").unwrap();
        assert_eq!(missing.location, None);
        assert_eq!(missing.notes, None);

        let cleared: UpdateEntry =
            serde_json::from_str(r#"{"location":null,"notes":null}"#).unwrap();
        assert_eq!(cleared.location, Some(None));
        assert_eq!(cleared.notes, Some(None));

        let set: UpdateEntry =
            serde_json::from_str(r#"{"location":"Example City","notes":"Bring ID"}"#).unwrap();
        assert_eq!(set.location, Some(Some("Example City".into())));
        assert_eq!(set.notes, Some(Some("Bring ID".into())));
    }

    #[test]
    fn bounded_context_is_not_a_calendar_entry() {
        let context = NewContext {
            kind: "uncertainty".into(),
            title: "Kolloquium".into(),
            details: "Termin noch offen".into(),
            valid_from: "2026-08-24".into(),
            valid_until: "2026-09-30".into(),
            source: "manual".into(),
        };
        assert!(context.validate().is_ok());

        let mut backwards = context.clone();
        backwards.valid_until = "2026-08-01".into();
        assert!(backwards.validate().is_err());
    }
}
