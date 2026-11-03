//! Phase E, the half that has no network in it: Google's wire shapes, the
//! mapping onto `NewEntry`, the draft rule, the conflict decision and the
//! export body.
//!
//! Pure over its inputs, the same call `correlate.rs` made — every rule here
//! is unit-testable against a recorded `events.list` payload with no token, no
//! socket and no database. `google_sync.rs` is the part that talks to Google.
//!
//! Three things this module is deliberately strict about:
//!
//! * **Nothing from Google becomes an authoritative block.** An imported event
//!   lands at `Commitment::Possible`, which the correlation layer caps at
//!   `Free`, until the operator adopts it. See [`IMPORT_COMMITMENT`].
//! * **Sjel wins a disagreement.** [`decide`] never overwrites an entry the
//!   operator has already adopted; the divergence is reported, not applied.
//! * **No date is guessed.** An event whose schedule this module cannot read
//!   exactly is skipped with a reason, never imported with an invented time —
//!   the same contract Phase A's Luma promotion states.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::model::{Commitment, Entry, NewEntry};
use crate::zone::{self, HomeTimezone};

/// The `source` every imported event carries. Already in `KNOWN_SOURCES`.
pub const SOURCE: &str = "google";

/// What an unadopted import is, expressed on the axis built for it.
///
/// This used to be `DRAFT_KIND = "draft"`, and the doc here used to name the
/// price of that choice: "a draft cannot simultaneously carry the kind Google
/// suggests." That price is gone. Draft-ness was never a kind — it answered
/// "how sure is this", which is `Commitment` — so an import now keeps the kind
/// Google's event actually implies *and* stays weightless until the operator adopts
/// it. Adoption is raising the commitment, so "has the operator touched this"
/// still needs no extra column and no timestamp comparison; the field they
/// change is still the signal.
pub const IMPORT_COMMITMENT: Commitment = Commitment::Possible;

/// The kind an imported event lands as. Google's own event stays in `payload`
/// either way, so adopting one can always prefill from it.
pub fn kind_for(event: &GoogleEvent) -> &'static str {
    match event.event_type.as_deref() {
        // The one Google type that maps cleanly onto an Sjel kind. Still
        // weightless on import — the commitment is what gates blocking, so
        // saying `away` here no longer costs anything.
        Some("outOfOffice") => "away",
        // Everything else is "there is something on your calendar then".
        _ => "busy",
    }
}

/// Placeholder for an event Google has no `summary` for. Entries require a
/// non-empty title, and an untitled block still has to be visible in the grid.
pub const UNTITLED: &str = "(ohne Titel)";

// ---- wire shapes ----------------------------------------------------------

/// One end of a Google event's schedule. Exactly one of `date` (all-day) or
/// `date_time` (timed) is populated; `time_zone` is advisory metadata that
/// accompanies `date_time` and is kept only as evidence — the offset inside
/// `date_time` is the authority.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct EventTime {
    #[serde(default)]
    pub date: Option<String>,
    #[serde(rename = "dateTime", default)]
    pub date_time: Option<String>,
    #[serde(rename = "timeZone", default)]
    pub time_zone: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct GoogleAttendee {
    #[serde(rename = "displayName", default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(rename = "self", default)]
    pub is_self: bool,
}

/// A `calendar#event` resource, narrowed to the fields this capability reads.
/// Unknown fields are dropped by serde rather than rejected: Google adds
/// fields, and an import that fails on a new one would be a scheduled outage.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct GoogleEvent {
    pub id: String,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default)]
    pub attendees: Vec<GoogleAttendee>,
    #[serde(default)]
    pub start: EventTime,
    #[serde(default)]
    pub end: EventTime,
    #[serde(rename = "htmlLink", default)]
    pub html_link: Option<String>,
    #[serde(rename = "iCalUID", default)]
    pub ical_uid: Option<String>,
    #[serde(rename = "recurringEventId", default)]
    pub recurring_event_id: Option<String>,
    #[serde(default)]
    pub transparency: Option<String>,
    #[serde(rename = "eventType", default)]
    pub event_type: Option<String>,
    #[serde(default)]
    pub updated: Option<String>,
}

/// An `events.list` page.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct EventsPage {
    #[serde(default)]
    pub items: Vec<GoogleEvent>,
    #[serde(rename = "nextPageToken", default)]
    pub next_page_token: Option<String>,
}

impl GoogleEvent {
    /// Google marks a deleted event — and a deleted instance of a recurring
    /// series, which `singleEvents=true` expands into the list — as
    /// `cancelled` rather than omitting it.
    pub fn is_cancelled(&self) -> bool {
        self.status.as_deref() == Some("cancelled")
    }
}

// ---- mapping --------------------------------------------------------------

/// Maps a Google event onto the entry this capability would store.
///
/// The dedupe key is the event `id`. With `singleEvents=true` that is stable
/// per *instance* of a recurring series (`base_20260814T080000Z`), which is
/// what dedupe needs — a series expanded into twelve instances is twelve
/// entries, and re-importing it updates those twelve rather than adding more.
///
/// Timed events are converted from Google's own offset to the operator's home
/// wall clock. All-day events are taken as written: Google's `end.date` is
/// already exclusive, which is this capability's convention too, so an all-day
/// event needs no arithmetic at all.
pub fn map_event(event: &GoogleEvent, tz: &HomeTimezone) -> Result<NewEntry, String> {
    if event.id.trim().is_empty() {
        return Err("event has no id to dedupe on".into());
    }
    if event.is_cancelled() {
        return Err("event is cancelled upstream".into());
    }

    let (starts_at, ends_at, all_day) = match (&event.start.date, &event.start.date_time) {
        (Some(start_date), None) => {
            let end_date = event
                .end
                .date
                .as_deref()
                .ok_or("all-day event has a start date but no end date")?;
            (
                start_date.trim().to_string(),
                end_date.trim().to_string(),
                true,
            )
        }
        (None, Some(start_instant)) => {
            let end_instant = event
                .end
                .date_time
                .as_deref()
                .ok_or("timed event has a start but no end")?;
            let start_wall = tz
                .wall_time(start_instant)
                .map_err(|e| format!("start: {e}"))?;
            let end_wall = tz.wall_time(end_instant).map_err(|e| format!("end: {e}"))?;
            // The one shape naive local storage genuinely cannot hold: an
            // event running through the autumn fall-back covers real time the
            // wall clock repeats, so its two ends can render as the same
            // clock face (02:30 CEST → 02:30 CET) or even backwards. Say so
            // instead of letting the entry contract report a generic
            // "ends_at must be after starts_at" for a perfectly valid event.
            if end_wall <= start_wall
                && zone::parse_rfc3339(end_instant)? > zone::parse_rfc3339(start_instant)?
            {
                return Err(format!(
                    "event spans the autumn DST transition: {start_instant} → {end_instant} is \
                     real time, but the wall clock repeats that hour, so it collapses to \
                     {start_wall} → {end_wall} locally. Naive local storage cannot represent it \
                     (README § Time model); reschedule or split the event in Google."
                ));
            }
            (start_wall, end_wall, false)
        }
        (Some(_), Some(_)) => {
            return Err("event carries both a date and a dateTime; refusing to pick one".into())
        }
        (None, None) => return Err("event has no start".into()),
    };

    let title = event
        .summary
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(UNTITLED)
        .to_string();

    let attendees: Vec<String> = event
        .attendees
        .iter()
        .filter(|a| !a.is_self)
        .filter_map(|a| {
            a.display_name
                .as_deref()
                .or(a.email.as_deref())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        })
        .collect();

    let entry = NewEntry {
        kind: kind_for(event).to_string(),
        commitment: IMPORT_COMMITMENT,
        title,
        starts_at,
        ends_at,
        all_day,
        location: event
            .location
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        notes: event
            .description
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        attendees,
        source: SOURCE.to_string(),
        external_id: Some(event.id.trim().to_string()),
        rhythm_id: None,
        payload: evidence_payload(event, tz),
    };
    // Ends-are-exclusive and the all_day/shape agreement are the store's
    // rules; failing here names the event instead of failing at the INSERT.
    entry.validate()?;
    Ok(entry)
}

/// Inert evidence. This is where the offset survives: `starts_at`/`ends_at` on
/// the row are naive home wall time, and the original offset-bearing strings
/// plus Google's own `timeZone` sit here byte-for-byte as received. A later
/// read path that needs true instants can recover them without a re-import —
/// the README's § Time model plan, made concrete.
///
/// Carries no "imported at" stamp on purpose: the payload has to be identical
/// across runs for a repeat import of an unchanged event to be a real no-op
/// rather than a churning update. Same reasoning as scouting's promotion.
fn evidence_payload(event: &GoogleEvent, tz: &HomeTimezone) -> Value {
    json!({
        "imported_from": "google",
        "google_event_id": event.id,
        "ical_uid": event.ical_uid,
        "recurring_event_id": event.recurring_event_id,
        "html_link": event.html_link,
        "status": event.status,
        "transparency": event.transparency,
        "event_type": event.event_type,
        "google_updated": event.updated,
        "original_start": event.start,
        "original_end": event.end,
        "home_timezone": tz.name(),
    })
}

// ---- conflict policy ------------------------------------------------------

/// What an import should do with one Google event, given whatever this
/// capability already holds under the same `(source, external_id)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Action {
    /// Nothing here yet — land it as a draft.
    Create,
    /// A draft the operator has not adopted; Google's newer version replaces it.
    RefreshDraft,
    /// The operator confirmed or re-kinded this entry. Sjel is the source of
    /// truth, so the import leaves it alone and reports the divergence.
    KeepSjelVersion,
    /// Google withdrew an event the operator never adopted. The draft goes.
    DropDraft,
    /// Google withdrew an event the operator *did* adopt. Sjel wins: the entry
    /// stays, and the operator is told the upstream is gone.
    KeepCancelledSjelVersion,
    /// Google withdrew an event that was never imported. Nothing to do.
    Skip,
}

/// The conflict policy, as a function.
///
/// One rule carries it: **an entry still at `Commitment::Possible` is
/// Google's to update; anything he has raised is the operator's and Sjel
/// wins.** Raising the commitment is the act of adoption, so "has the operator
/// touched this" needs no extra column and no timestamp comparison — the thing
/// the operator changes *is* the signal.
pub fn decide(event: &GoogleEvent, existing: Option<&Entry>) -> Action {
    let is_draft = existing.is_some_and(|entry| entry.commitment == IMPORT_COMMITMENT);
    match (event.is_cancelled(), existing.is_some(), is_draft) {
        (true, false, _) => Action::Skip,
        (true, true, true) => Action::DropDraft,
        (true, true, false) => Action::KeepCancelledSjelVersion,
        (false, false, _) => Action::Create,
        (false, true, true) => Action::RefreshDraft,
        (false, true, false) => Action::KeepSjelVersion,
    }
}

/// Whether a refresh would actually change anything. A Google event that has
/// not moved should produce no write at all, so a nightly import of a stable
/// calendar is silent instead of bumping `updated_at` on every row.
pub fn differs(candidate: &NewEntry, existing: &Entry) -> bool {
    candidate.starts_at != existing.starts_at
        || candidate.ends_at != existing.ends_at
        || candidate.all_day != existing.all_day
        || candidate.title != existing.title
        || candidate.location != existing.location
        || candidate.notes != existing.notes
        || candidate.payload != existing.payload
}

// ---- export ---------------------------------------------------------------

/// Why an entry may not be opted in to export.
///
/// Re-exporting an imported event is the one that matters: it would create a
/// second Google event for something Google already owns, and the next import
/// would pull that copy back in as a third thing.
pub fn export_refusal(entry: &Entry) -> Option<String> {
    if entry.source == SOURCE {
        return Some(format!(
            "entry {} came from Google (source = {SOURCE}); exporting it would duplicate the event Google already holds",
            entry.id
        ));
    }
    if entry.rhythm_id.is_some() {
        return Some(format!(
            "entry {} is a materialized rhythm instance; export the rhythm's meaning to Google as its own event instead of pushing generated instances",
            entry.id
        ));
    }
    None
}

/// The `calendar#event` body for an Sjel entry.
///
/// Timed entries get an explicit offset *and* the zone name: the offset is
/// what pins the instant, the zone is what lets Google render it correctly for
/// anyone else looking at the calendar. All-day entries go out as `date` on
/// both ends — Google's exclusive end matches this capability's, so again no
/// arithmetic.
///
/// `extendedProperties.private.sjel_entry_id` is the back-reference. It is not
/// what the export ledger keys on (that is the returned Google event id), but
/// it makes an event in the Google UI traceable to the entry that produced it.
pub fn export_body(entry: &Entry, tz: &HomeTimezone) -> Result<Value, String> {
    if let Some(reason) = export_refusal(entry) {
        return Err(reason);
    }
    let (start, end) = if entry.all_day {
        (
            json!({ "date": entry.starts_at }),
            json!({ "date": entry.ends_at }),
        )
    } else {
        (
            json!({
                "dateTime": tz.rfc3339(&entry.starts_at).map_err(|e| format!("start: {e}"))?,
                "timeZone": tz.name(),
            }),
            json!({
                "dateTime": tz.rfc3339(&entry.ends_at).map_err(|e| format!("end: {e}"))?,
                "timeZone": tz.name(),
            }),
        )
    };
    Ok(json!({
        "summary": entry.title,
        "description": entry.notes,
        "location": entry.location,
        "start": start,
        "end": end,
        "extendedProperties": {
            "private": { "sjel_entry_id": entry.id, "sjel_kind": entry.kind }
        },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A recorded `events.list` response. Shapes taken from Google Calendar
    /// API v3: a timed event with an offset, an all-day event with an
    /// exclusive `end.date`, an expanded recurring instance, an untitled
    /// event, and a cancelled instance.
    const EVENTS_LIST: &str = r#"{
      "kind": "calendar#events",
      "summary": "operator@example.com",
      "timeZone": "Europe/Berlin",
      "nextPageToken": "CkAKMGo",
      "items": [
        {
          "kind": "calendar#event",
          "id": "3q7l9v1c8m2p4t6y0x5z",
          "status": "confirmed",
          "htmlLink": "https://www.google.com/calendar/event?eid=M3E3bDl2",
          "updated": "2026-07-02T09:14:11.902Z",
          "summary": "Team sync",
          "description": "Weekly, agenda in the doc",
          "location": "Example City, Central Square",
          "start": { "dateTime": "2026-08-14T10:00:00+02:00", "timeZone": "Europe/Berlin" },
          "end":   { "dateTime": "2026-08-14T11:00:00+02:00", "timeZone": "Europe/Berlin" },
          "iCalUID": "3q7l9v1c8m2p4t6y0x5z@google.com",
          "eventType": "default"
        },
        {
          "kind": "calendar#event",
          "id": "7h2k4n6q8s0u2w4y6a8c",
          "status": "confirmed",
          "summary": "Urlaub",
          "start": { "date": "2026-08-17" },
          "end":   { "date": "2026-08-22" },
          "transparency": "opaque",
          "eventType": "outOfOffice"
        },
        {
          "kind": "calendar#event",
          "id": "base9x7v5t3r1p_20260819T063000Z",
          "status": "confirmed",
          "summary": "Standup",
          "start": { "dateTime": "2026-08-19T08:30:00+02:00", "timeZone": "Europe/Berlin" },
          "end":   { "dateTime": "2026-08-19T08:45:00+02:00", "timeZone": "Europe/Berlin" },
          "recurringEventId": "base9x7v5t3r1p",
          "transparency": "transparent"
        },
        {
          "kind": "calendar#event",
          "id": "0z8x6v4t2r0p8n6l4j2h",
          "status": "confirmed",
          "start": { "dateTime": "2026-08-20T19:00:00+02:00" },
          "end":   { "dateTime": "2026-08-20T21:00:00+02:00" }
        },
        {
          "kind": "calendar#event",
          "id": "base9x7v5t3r1p_20260826T063000Z",
          "status": "cancelled",
          "recurringEventId": "base9x7v5t3r1p"
        }
      ]
    }"#;

    fn berlin() -> HomeTimezone {
        HomeTimezone::parse("Europe/Berlin").unwrap()
    }

    fn page() -> EventsPage {
        serde_json::from_str(EVENTS_LIST).expect("fixture parses")
    }

    fn stored(entry: NewEntry, id: &str) -> Entry {
        Entry {
            id: id.into(),
            kind: entry.kind,
            commitment: entry.commitment,
            title: entry.title,
            starts_at: entry.starts_at,
            ends_at: entry.ends_at,
            all_day: entry.all_day,
            location: entry.location,
            notes: entry.notes,
            attendees: entry.attendees,
            source: entry.source,
            external_id: entry.external_id,
            rhythm_id: entry.rhythm_id,
            payload: entry.payload,
            created_at: "0".into(),
            updated_at: "0".into(),
        }
    }

    #[test]
    fn the_recorded_page_parses_with_its_page_token() {
        let page = page();
        assert_eq!(page.items.len(), 5);
        assert_eq!(page.next_page_token.as_deref(), Some("CkAKMGo"));
    }

    #[test]
    fn a_timed_event_lands_in_home_wall_time_as_a_draft() {
        let entry = map_event(&page().items[0], &berlin()).unwrap();
        assert_eq!(entry.commitment, Commitment::Possible);
        assert_eq!(entry.kind, "busy");
        assert_eq!(entry.source, "google");
        assert_eq!(entry.external_id.as_deref(), Some("3q7l9v1c8m2p4t6y0x5z"));
        assert_eq!(entry.starts_at, "2026-08-14T10:00:00");
        assert_eq!(entry.ends_at, "2026-08-14T11:00:00");
        assert!(!entry.all_day);
        assert_eq!(entry.title, "Team sync");
        assert_eq!(
            entry.location.as_deref(),
            Some("Example City, Central Square")
        );
        assert_eq!(entry.notes.as_deref(), Some("Weekly, agenda in the doc"));
    }

    #[test]
    fn the_original_offset_survives_in_the_payload() {
        // The columns are naive; the offset is not lost, it moves here.
        let entry = map_event(&page().items[0], &berlin()).unwrap();
        assert_eq!(
            entry.payload["original_start"]["dateTime"],
            "2026-08-14T10:00:00+02:00"
        );
        assert_eq!(entry.payload["original_start"]["timeZone"], "Europe/Berlin");
        assert_eq!(entry.payload["home_timezone"], "Europe/Berlin");
        assert_eq!(entry.payload["google_event_id"], "3q7l9v1c8m2p4t6y0x5z");
    }

    #[test]
    fn an_all_day_events_exclusive_end_is_taken_as_written() {
        // Google's end.date is already exclusive, like every end here.
        let entry = map_event(&page().items[1], &berlin()).unwrap();
        assert!(entry.all_day);
        assert_eq!(entry.starts_at, "2026-08-17");
        assert_eq!(entry.ends_at, "2026-08-22");
        // The trade the old `DRAFT_KIND` forced: an out-of-office could not
        // say it was an out-of-office. Now it can, and it is still weightless.
        assert_eq!(entry.kind, "away");
        assert_eq!(
            entry.commitment,
            Commitment::Possible,
            "an unadopted out-of-office still blocks nothing"
        );
        assert_eq!(
            crate::correlate::impact(&entry.kind, entry.commitment),
            crate::correlate::Feasibility::Free
        );
    }

    #[test]
    fn an_expanded_recurring_instance_dedupes_on_its_own_id() {
        let entry = map_event(&page().items[2], &berlin()).unwrap();
        assert_eq!(
            entry.external_id.as_deref(),
            Some("base9x7v5t3r1p_20260819T063000Z"),
            "the instance id, not the series id — twelve instances are twelve entries"
        );
        assert_eq!(entry.payload["recurring_event_id"], "base9x7v5t3r1p");
    }

    #[test]
    fn an_untitled_event_still_gets_a_title() {
        let entry = map_event(&page().items[3], &berlin()).unwrap();
        assert_eq!(entry.title, UNTITLED);
        assert!(entry.location.is_none());
    }

    #[test]
    fn a_cancelled_event_is_never_mapped_into_an_entry() {
        let error = map_event(&page().items[4], &berlin()).unwrap_err();
        assert!(error.contains("cancelled"), "{error}");
    }

    #[test]
    fn mapping_the_same_event_twice_is_byte_identical() {
        // Idempotency is only real if a repeat import produces the same row.
        let event = &page().items[0];
        assert_eq!(
            map_event(event, &berlin()).unwrap(),
            map_event(event, &berlin()).unwrap()
        );
    }

    fn timed(id: &str, start: &str, end: &str) -> GoogleEvent {
        GoogleEvent {
            id: id.into(),
            summary: Some("Nachtschicht".into()),
            start: EventTime {
                date_time: Some(start.into()),
                ..Default::default()
            },
            end: EventTime {
                date_time: Some(end.into()),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn the_two_halves_of_the_repeated_autumn_hour_land_on_one_wall_time() {
        // 02:30+02:00 and 02:30+01:00 on 25 Oct 2026 are an hour apart in real
        // time and render as the same clock face. Both import — and the
        // payload is the only thing that still tells them apart.
        let first = map_event(
            &timed(
                "dst-first",
                "2026-10-25T02:30:00+02:00",
                "2026-10-25T02:45:00+02:00",
            ),
            &berlin(),
        )
        .unwrap();
        let second = map_event(
            &timed(
                "dst-second",
                "2026-10-25T02:30:00+01:00",
                "2026-10-25T02:45:00+01:00",
            ),
            &berlin(),
        )
        .unwrap();

        assert_eq!(first.starts_at, "2026-10-25T02:30:00");
        assert_eq!(second.starts_at, "2026-10-25T02:30:00");
        assert_eq!(
            first.payload["original_start"]["dateTime"],
            "2026-10-25T02:30:00+02:00"
        );
        assert_eq!(
            second.payload["original_start"]["dateTime"],
            "2026-10-25T02:30:00+01:00"
        );
        // Distinct events, distinct rows: the dedupe key is Google's id, not
        // the wall time they collide on.
        assert_ne!(first.external_id, second.external_id);
    }

    #[test]
    fn an_event_running_through_the_autumn_transition_is_refused_with_its_reason() {
        // 02:30 CEST → 03:30 CEST is a real hour, but the wall clock replays
        // 02:00–03:00, so both ends read 02:30 locally. Naive local storage
        // cannot hold that, and the refusal has to say why rather than
        // bottoming out in the generic exclusive-end check.
        let error = map_event(
            &timed(
                "dst-through",
                "2026-10-25T02:30:00+02:00",
                "2026-10-25T03:30:00+02:00",
            ),
            &berlin(),
        )
        .unwrap_err();
        assert!(error.contains("autumn DST transition"), "{error}");
        assert!(error.contains("02:30:00"), "{error}");
    }

    #[test]
    fn a_google_event_across_the_spring_dst_gap_stretches_instead() {
        // The mirror case: one real hour, 01:30 CET → 03:30 CEST, reads as two
        // hours locally because 02:00–03:00 never happens. That one stores.
        let entry = map_event(
            &timed(
                "spring",
                "2026-03-29T01:30:00+01:00",
                "2026-03-29T03:30:00+02:00",
            ),
            &berlin(),
        )
        .unwrap();
        assert_eq!(entry.starts_at, "2026-03-29T01:30:00");
        assert_eq!(entry.ends_at, "2026-03-29T03:30:00");
    }

    #[test]
    fn an_event_with_no_readable_schedule_is_skipped_not_guessed() {
        let mut event = page().items[0].clone();
        event.end = EventTime::default();
        assert!(map_event(&event, &berlin()).unwrap_err().contains("no end"));

        let mut both = page().items[0].clone();
        both.start.date = Some("2026-08-14".into());
        assert!(map_event(&both, &berlin())
            .unwrap_err()
            .contains("both a date and a dateTime"));

        let mut naive = page().items[0].clone();
        naive.start.date_time = Some("2026-08-14T10:00:00".into());
        assert!(map_event(&naive, &berlin())
            .unwrap_err()
            .contains("carries no UTC offset"));

        let mut idless = page().items[0].clone();
        idless.id = "  ".into();
        assert!(map_event(&idless, &berlin()).unwrap_err().contains("no id"));
    }

    #[test]
    fn an_inverted_event_is_rejected_by_the_entry_contract() {
        let mut event = page().items[0].clone();
        event.end.date_time = Some("2026-08-14T09:00:00+02:00".into());
        let error = map_event(&event, &berlin()).unwrap_err();
        assert!(error.contains("ends_at must be after starts_at"), "{error}");
    }

    #[test]
    fn a_new_event_is_created_a_still_draft_one_is_refreshed() {
        let event = page().items[0].clone();
        assert_eq!(decide(&event, None), Action::Create);

        let draft = stored(map_event(&event, &berlin()).unwrap(), "cal:entry:1");
        assert_eq!(decide(&event, Some(&draft)), Action::RefreshDraft);
    }

    #[test]
    fn sjel_wins_once_the_operator_has_adopted_the_draft() {
        let event = page().items[0].clone();
        let mut confirmed = stored(map_event(&event, &berlin()).unwrap(), "cal:entry:1");
        // Adoption is raising the commitment, not re-kinding: he can correct
        // Google's kind without that meaning "I have taken this over".
        confirmed.kind = "work_onsite".into();
        confirmed.commitment = Commitment::Committed;

        assert_eq!(decide(&event, Some(&confirmed)), Action::KeepSjelVersion);

        // ...and a moved Google event still does not move the Sjel one.
        let mut moved = event;
        moved.start.date_time = Some("2026-08-14T15:00:00+02:00".into());
        moved.end.date_time = Some("2026-08-14T16:00:00+02:00".into());
        assert_eq!(decide(&moved, Some(&confirmed)), Action::KeepSjelVersion);
    }

    #[test]
    fn a_cancelled_event_drops_a_draft_but_never_a_confirmed_entry() {
        let cancelled = page().items[4].clone();
        let live = page().items[2].clone();

        let draft = stored(map_event(&live, &berlin()).unwrap(), "cal:entry:2");
        assert_eq!(decide(&cancelled, Some(&draft)), Action::DropDraft);

        let mut adopted = draft;
        adopted.commitment = Commitment::Committed;
        assert_eq!(
            decide(&cancelled, Some(&adopted)),
            Action::KeepCancelledSjelVersion
        );

        // Re-kinding alone is a correction, not an adoption: still Google's.
        let mut recorrected = stored(map_event(&live, &berlin()).unwrap(), "cal:entry:2");
        recorrected.kind = "event".into();
        assert_eq!(decide(&cancelled, Some(&recorrected)), Action::DropDraft);

        assert_eq!(
            decide(&cancelled, None),
            Action::Skip,
            "a withdrawal of something never imported is not a deletion"
        );
    }

    #[test]
    fn an_unchanged_event_is_not_rewritten() {
        let event = page().items[0].clone();
        let candidate = map_event(&event, &berlin()).unwrap();
        let existing = stored(candidate.clone(), "cal:entry:1");
        assert!(!differs(&candidate, &existing));

        let mut moved = event;
        moved.start.date_time = Some("2026-08-14T12:00:00+02:00".into());
        moved.end.date_time = Some("2026-08-14T13:00:00+02:00".into());
        let moved = map_event(&moved, &berlin()).unwrap();
        assert!(differs(&moved, &existing));
    }

    #[test]
    fn export_body_carries_the_offset_and_the_zone() {
        let entry = Entry {
            id: "cal:entry:9".into(),
            kind: "event".into(),
            commitment: Commitment::Committed,
            title: "Vortrag".into(),
            starts_at: "2026-08-14T18:00:00".into(),
            ends_at: "2026-08-14T20:00:00".into(),
            all_day: false,
            location: Some("München".into()),
            notes: None,
            attendees: Vec::new(),
            source: "manual".into(),
            external_id: None,
            rhythm_id: None,
            payload: Value::Null,
            created_at: "0".into(),
            updated_at: "0".into(),
        };
        let body = export_body(&entry, &berlin()).unwrap();
        assert_eq!(body["start"]["dateTime"], "2026-08-14T18:00:00+02:00");
        assert_eq!(body["start"]["timeZone"], "Europe/Berlin");
        assert_eq!(body["end"]["dateTime"], "2026-08-14T20:00:00+02:00");
        assert_eq!(body["summary"], "Vortrag");
        assert_eq!(
            body["extendedProperties"]["private"]["sjel_entry_id"],
            "cal:entry:9"
        );

        let winter = Entry {
            starts_at: "2026-01-15T18:00:00".into(),
            ends_at: "2026-01-15T20:00:00".into(),
            ..entry.clone()
        };
        assert_eq!(
            export_body(&winter, &berlin()).unwrap()["start"]["dateTime"],
            "2026-01-15T18:00:00+01:00"
        );

        let all_day = Entry {
            all_day: true,
            starts_at: "2026-08-17".into(),
            ends_at: "2026-08-22".into(),
            ..entry
        };
        let body = export_body(&all_day, &berlin()).unwrap();
        assert_eq!(body["start"]["date"], "2026-08-17");
        assert_eq!(body["end"]["date"], "2026-08-22");
        assert!(body["start"].get("dateTime").is_none());
    }

    #[test]
    fn an_imported_entry_may_not_be_exported_back() {
        let mut entry = stored(
            map_event(&page().items[0], &berlin()).unwrap(),
            "cal:entry:1",
        );
        let refusal = export_refusal(&entry).expect("a google entry is refused");
        assert!(refusal.contains("duplicate"), "{refusal}");
        assert!(export_body(&entry, &berlin()).is_err());

        entry.source = "manual".into();
        entry.rhythm_id = Some("cal:rhythm:1".into());
        assert!(export_refusal(&entry)
            .expect("a rhythm instance is refused")
            .contains("rhythm"));

        entry.rhythm_id = None;
        entry.kind = "event".into();
        assert_eq!(export_refusal(&entry), None);
    }

    #[test]
    fn export_refuses_a_wall_time_that_never_existed() {
        let entry = Entry {
            id: "cal:entry:gap".into(),
            kind: "busy".into(),
            commitment: Commitment::Committed,
            title: "Unmögliche Stunde".into(),
            starts_at: "2026-03-29T02:15:00".into(),
            ends_at: "2026-03-29T03:15:00".into(),
            all_day: false,
            location: None,
            notes: None,
            attendees: Vec::new(),
            source: "manual".into(),
            external_id: None,
            rhythm_id: None,
            payload: Value::Null,
            created_at: "0".into(),
            updated_at: "0".into(),
        };
        let error = export_body(&entry, &berlin()).unwrap_err();
        assert!(error.contains("spring-forward"), "{error}");
    }
}
