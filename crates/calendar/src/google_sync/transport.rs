use super::auth::{http_client, SyncResult, CALENDAR_API};
use super::*;

// ---- the API surface, behind a trait so a fixture can stand in -------------

/// The three Google calls Phase E makes. A trait so the import and export runs
/// can be exercised against recorded payloads: everything below this line is
/// transport, and transport is the one thing a test cannot have.
pub trait CalendarApi {
    fn list_events(
        &self,
        calendar_id: &str,
        from: &str,
        to: &str,
        max: usize,
    ) -> SyncResult<Vec<GoogleEvent>>;
    /// Returns the created event's id.
    fn insert_event(&self, calendar_id: &str, body: &Value) -> SyncResult<String>;
    fn patch_event(&self, calendar_id: &str, event_id: &str, body: &Value) -> SyncResult<String>;
}

/// The real one. Reads the token from the configured credential file on each
/// call (cached in process), and never logs it.
pub struct HttpCalendarApi<'a> {
    env_path: &'a Path,
}

impl<'a> HttpCalendarApi<'a> {
    pub fn new(env_path: &'a Path) -> Self {
        Self { env_path }
    }

    fn token(&self) -> SyncResult<String> {
        access_token(self.env_path)
    }
}

/// Google's own encoding for a path segment that may contain `@` and `.`
/// (a secondary calendar id is an address-shaped string).
pub(super) fn encode_segment(segment: &str) -> String {
    segment
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (byte as char).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}

pub(super) fn fail_for(status: reqwest::StatusCode, what: &str) -> String {
    let hint = match status.as_u16() {
        401 => " — the access token was rejected; the refresh token may be revoked",
        403 => " — the grant is missing the scope this call needs, or the calendar is not shared with this account",
        404 => " — no such calendar or event for this account; check google.calendar_id",
        429 => " — Google rate-limited this run; retry later",
        _ => "",
    };
    format!("{what} failed with HTTP {status}{hint}")
}

impl CalendarApi for HttpCalendarApi<'_> {
    fn list_events(
        &self,
        calendar_id: &str,
        from: &str,
        to: &str,
        max: usize,
    ) -> SyncResult<Vec<GoogleEvent>> {
        let token = self.token()?;
        let client = http_client()?;
        let url = format!("{CALENDAR_API}/{}/events", encode_segment(calendar_id));
        let mut events: Vec<GoogleEvent> = Vec::new();
        let mut page_token: Option<String> = None;

        while events.len() < max {
            let page_size = (max - events.len()).min(250).to_string();
            let mut query: Vec<(&str, String)> = vec![
                ("timeMin", from.to_string()),
                ("timeMax", to.to_string()),
                // Expands a recurring series into instances, each with its own
                // stable id — which is what dedupe needs (google::map_event).
                ("singleEvents", "true".into()),
                ("orderBy", "startTime".into()),
                ("maxResults", page_size),
            ];
            if let Some(cursor) = &page_token {
                query.push(("pageToken", cursor.clone()));
            }

            let response = client
                .get(&url)
                .bearer_auth(&token)
                .query(&query)
                .send()
                .map_err(|error| format!("events.list could not reach Google: {error}"))?;
            if !response.status().is_success() {
                return Err(fail_for(response.status(), "events.list"));
            }
            let page: google::EventsPage = response
                .json()
                .map_err(|error| format!("events.list returned an unreadable body: {error}"))?;

            let exhausted = page.next_page_token.is_none();
            events.extend(page.items);
            page_token = page.next_page_token;
            if exhausted {
                break;
            }
        }
        events.truncate(max);
        Ok(events)
    }

    fn insert_event(&self, calendar_id: &str, body: &Value) -> SyncResult<String> {
        let url = format!("{CALENDAR_API}/{}/events", encode_segment(calendar_id));
        let response = http_client()?
            .post(&url)
            .bearer_auth(self.token()?)
            .json(body)
            .send()
            .map_err(|error| format!("events.insert could not reach Google: {error}"))?;
        if !response.status().is_success() {
            return Err(fail_for(response.status(), "events.insert"));
        }
        created_id(response)
    }

    fn patch_event(&self, calendar_id: &str, event_id: &str, body: &Value) -> SyncResult<String> {
        let url = format!(
            "{CALENDAR_API}/{}/events/{}",
            encode_segment(calendar_id),
            encode_segment(event_id)
        );
        let response = http_client()?
            .patch(&url)
            .bearer_auth(self.token()?)
            .json(body)
            .send()
            .map_err(|error| format!("events.patch could not reach Google: {error}"))?;
        if !response.status().is_success() {
            return Err(fail_for(response.status(), "events.patch"));
        }
        created_id(response)
    }
}

pub(super) fn created_id(response: reqwest::blocking::Response) -> SyncResult<String> {
    let event: GoogleEvent = response
        .json()
        .map_err(|error| format!("Google returned an unreadable event: {error}"))?;
    if event.id.trim().is_empty() {
        return Err("Google returned an event without an id".into());
    }
    Ok(event.id)
}

// ---- the store surface, likewise behind traits ----------------------------

/// The store operations an import needs.
///
/// A trait for one reason: "running an import twice produces one entry per
/// event" is the claim this phase most has to *show*, and showing it needs a
/// second run over the same fixtures — which a test can have, and a Postgres
/// integration test in a sandbox cannot reliably.
pub trait ImportStore {
    fn existing(&self, external_id: &str) -> SyncResult<Option<Entry>>;
    fn upsert(&self, entry: &NewEntry) -> SyncResult<Entry>;
    fn delete(&self, entry_id: &str) -> SyncResult<bool>;
}

/// The store operations an export needs.
pub trait ExportStore {
    fn queue(&self) -> SyncResult<Vec<(ExportOptIn, Entry)>>;
    fn record_push(&self, entry_id: &str, google_event_id: &str) -> SyncResult<()>;
}

impl ImportStore for CalendarStore {
    fn existing(&self, external_id: &str) -> SyncResult<Option<Entry>> {
        self.get_entry_by_external(google::SOURCE, external_id)
            .map_err(|error| format!("reading the existing entry failed: {error}"))
    }

    fn upsert(&self, entry: &NewEntry) -> SyncResult<Entry> {
        self.upsert_external_entry(entry)
            .map_err(|error| error.to_string())
    }

    fn delete(&self, entry_id: &str) -> SyncResult<bool> {
        self.delete_entry(entry_id)
            .map_err(|error| format!("deleting {entry_id}: {error}"))
    }
}

impl ExportStore for CalendarStore {
    fn queue(&self) -> SyncResult<Vec<(ExportOptIn, Entry)>> {
        self.export_queue()
            .map_err(|error| format!("reading the export queue failed: {error}"))
    }

    fn record_push(&self, entry_id: &str, google_event_id: &str) -> SyncResult<()> {
        self.record_export_push(entry_id, google_event_id)
            .map(|_| ())
            .map_err(|error| format!("recording the push for {entry_id}: {error}"))
    }
}
