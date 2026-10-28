//! Gmail access. Reads are used by the bounded sweep; writes happen only after
//! an explicit dashboard action:
//!   - OAuth token refresh: POST https://oauth2.googleapis.com/token
//!   - list threads:        GET  .../gmail/v1/users/me/threads?q=in:inbox
//!   - thread metadata:     GET  .../gmail/v1/users/me/threads/{id}?format=metadata
//!   - archive a thread:    POST .../gmail/v1/users/me/threads/{id}/modify
//!   - move to Trash:       POST .../gmail/v1/users/me/threads/{id}/trash
//!   - restore from Trash:  POST .../gmail/v1/users/me/threads/{id}/untrash
//!   - restore an archive:  POST .../gmail/v1/users/me/threads/{id}/modify
//!
//! There is no permanent-delete, send, or arbitrary-label operation.
//! Credentials come from the keychain grant `google-comms`, or from the overlay's `comms.env`
//! until the grant moves there (see [`access_token`]); token values are never logged.

use std::collections::BTreeSet;
use std::io::Read;
use std::path::Path;

use serde::Deserialize;

use crate::{CommsError, Result};

const GMAIL_URL: &str = "https://gmail.googleapis.com/gmail/v1/users/me";

/// `SJEL_COMMS_GMAIL_URL` points Gmail calls at a loopback stand-in (`tools/demo-origin`).
const GMAIL_URL_ENV: &str = "SJEL_COMMS_GMAIL_URL";

/// The Gmail base an override names: `None` when unset, an error when it is not loopback.
fn gmail_override(value: Option<&str>) -> std::result::Result<Option<String>, String> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let base = value.trim_end_matches('/');
    let loopback = ["http://127.0.0.1:", "http://localhost:"]
        .iter()
        .any(|prefix| base.starts_with(prefix));
    if loopback {
        Ok(Some(base.to_owned()))
    } else {
        Err(format!(
            "{GMAIL_URL_ENV} must be http on 127.0.0.1 or localhost"
        ))
    }
}

fn gmail_base() -> String {
    let value = sjel_config::env_var(GMAIL_URL_ENV).ok();
    gmail_override(value.as_deref())
        .ok()
        .flatten()
        .unwrap_or_else(|| GMAIL_URL.to_owned())
}

fn threads_url() -> String {
    format!("{}/threads", gmail_base())
}

fn labels_url() -> String {
    format!("{}/labels", gmail_base())
}

/// The only label Sjel creates, and the doctrine's stated ceiling: one state
/// label, no topic labels. `Finance`, `University`, `Newsletters` and friends
/// are deliberately absent — a topic label is a second classification competing
/// with the stream a thread already has, and the two drift.
pub const WAITING_LABEL: &str = "Waiting";
/// Courtesy pause between per-thread metadata fetches (see `thread_meta`).
const THREAD_FETCH_PAUSE: std::time::Duration = std::time::Duration::from_millis(60);

fn is_missing_thread_status(status: u16) -> bool {
    matches!(status, 404 | 410)
}

fn client() -> Result<reqwest::blocking::Client> {
    // 30s, matching comms' other outbound fetches (src/sources.rs, src/media.rs).
    // This carried no timeout before.
    sjel_http::builder(
        sjel_http::Purpose::new("comms-google"),
        std::time::Duration::from_secs(30),
    )
    .gzip(true)
    .build()
    .map_err(CommsError::from)
}

/// The credential ID of comms' Google grant (Gmail and Calendar) in the overlay catalog, as
/// `capabilities/comms/service.toml` declares it. Calendar reads the same grant.
pub const GRANT_ID: &str = "google-comms";

/// An access token for comms' grant: the keychain first, `env_path` while the grant has not
/// moved there (`libs/sjel-credentials/src/google.rs`). Token values are never logged.
///
/// With a loopback `SJEL_COMMS_GMAIL_URL` this returns a placeholder and reads no credential, so
/// a demo cannot send a real token. Any other override value fails, so nothing is sent at all.
pub fn access_token(env_path: &Path) -> Result<String> {
    let value = sjel_config::env_var(GMAIL_URL_ENV).ok();
    if gmail_override(value.as_deref())
        .map_err(CommsError::Auth)?
        .is_some()
    {
        return Ok("demo-placeholder".to_owned());
    }
    sjel_credentials::google::access_token(GRANT_ID, env_path).map_err(CommsError::Auth)
}

/// A thread id from the list endpoint.
#[derive(Debug, Clone)]
pub struct ThreadStub {
    pub id: String,
}

#[derive(Debug, Clone)]
pub struct ThreadPage {
    pub threads: Vec<ThreadStub>,
    pub next_page_token: Option<String>,
}

/// Metadata for one thread's latest message.
#[derive(Debug, Clone, Default)]
pub struct ThreadMeta {
    pub id: String,
    pub from_addr: Option<String>,
    pub subject: Option<String>,
    pub date: Option<String>,
    pub list_unsubscribe: Option<String>,
    pub snippet: Option<String>,
    pub label_ids: Vec<String>,
    /// internalDate of the latest message, epoch milliseconds.
    pub internal_date_ms: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadLocation {
    Inbox,
    Archive,
    Trash,
}

impl ThreadLocation {
    pub fn from_labels(labels: &[String]) -> Self {
        if labels.iter().any(|label| label == "TRASH") {
            Self::Trash
        } else if labels.iter().any(|label| label == "INBOX") {
            Self::Inbox
        } else {
            Self::Archive
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Inbox => "inbox",
            Self::Archive => "archive",
            Self::Trash => "trash",
        }
    }
}

impl ThreadMeta {
    pub fn has_list_unsubscribe(&self) -> bool {
        self.list_unsubscribe.is_some()
    }
}

#[derive(Deserialize)]
struct ThreadListResponse {
    #[serde(default)]
    threads: Vec<ThreadListEntry>,
    #[serde(rename = "nextPageToken")]
    next_page_token: Option<String>,
}
#[derive(Deserialize)]
struct ThreadListEntry {
    id: String,
}

#[derive(Deserialize)]
struct GoogleErrorBody {
    error: Option<GoogleError>,
}

#[derive(Deserialize)]
struct GoogleError {
    #[serde(default)]
    errors: Vec<GoogleErrorReason>,
}

#[derive(Deserialize)]
struct GoogleErrorReason {
    reason: Option<String>,
}

fn safe_google_error_reason(body: &[u8]) -> Option<&'static str> {
    let parsed: GoogleErrorBody = serde_json::from_slice(body).ok()?;
    let reason = parsed
        .error?
        .errors
        .into_iter()
        .find_map(|error| error.reason)?;
    match reason.as_str() {
        "rateLimitExceeded" | "userRateLimitExceeded" | "dailyLimitExceeded" | "quotaExceeded" => {
            Some("quota")
        }
        "authError" | "insufficientPermissions" => Some("permissions"),
        "accessNotConfigured" => Some("api_disabled"),
        "domainPolicy" => Some("domain_policy"),
        _ => None,
    }
}

fn safe_google_error_reason_from_response(
    response: reqwest::blocking::Response,
) -> Option<&'static str> {
    let mut body = Vec::with_capacity(1024);
    response.take(8 * 1024).read_to_end(&mut body).ok()?;
    safe_google_error_reason(&body)
}

/// Lists inbox threads (read-only, `q=in:inbox`), paging until `limit` ids are
/// collected or the inbox is exhausted.
pub fn list_inbox_threads(token: &str, limit: usize) -> Result<Vec<ThreadStub>> {
    let mut out: Vec<ThreadStub> = Vec::new();
    let mut page_token: Option<String> = None;

    while out.len() < limit {
        let remaining = limit - out.len();
        let page = list_inbox_threads_page(token, remaining.min(100), page_token.as_deref())?;
        for thread in page.threads {
            out.push(thread);
            if out.len() >= limit {
                break;
            }
        }
        match page.next_page_token {
            Some(pt) if out.len() < limit => page_token = Some(pt),
            _ => break,
        }
    }
    Ok(out)
}

/// Read one Gmail inbox page. The opaque cursor is returned to the caller but
/// never persisted; a fresh review session can safely begin at the newest page
/// again because triage upserts are idempotent.
pub fn list_inbox_threads_page(
    token: &str,
    limit: usize,
    page_token: Option<&str>,
) -> Result<ThreadPage> {
    let page_size = limit.clamp(1, 100).to_string();
    let mut query: Vec<(&str, String)> =
        vec![("q", "in:inbox".to_string()), ("maxResults", page_size)];
    if let Some(cursor) = page_token.filter(|value| !value.trim().is_empty()) {
        query.push(("pageToken", cursor.to_string()));
    }

    let response = client()?
        .get(threads_url())
        .bearer_auth(token)
        .query(&query)
        .send()?;
    if !response.status().is_success() {
        let status = response.status();
        let reason = (status == reqwest::StatusCode::FORBIDDEN)
            .then(|| safe_google_error_reason_from_response(response))
            .flatten()
            .map(|reason| format!(" ({reason})"))
            .unwrap_or_default();
        return Err(CommsError::Other(format!(
            "thread list failed with HTTP {status}{reason}"
        )));
    }
    let parsed: ThreadListResponse = response.json()?;
    Ok(ThreadPage {
        threads: parsed
            .threads
            .into_iter()
            .map(|thread| ThreadStub { id: thread.id })
            .collect(),
        next_page_token: parsed.next_page_token,
    })
}

#[derive(Deserialize)]
struct ThreadDetail {
    #[serde(default)]
    messages: Vec<MessageDetail>,
}
#[derive(Deserialize)]
struct MessageDetail {
    #[serde(default)]
    snippet: Option<String>,
    #[serde(rename = "labelIds", default)]
    label_ids: Vec<String>,
    #[serde(rename = "internalDate", default)]
    internal_date: Option<String>,
    #[serde(default)]
    payload: Option<MessagePayload>,
}
#[derive(Deserialize)]
struct MessagePayload {
    #[serde(default)]
    headers: Vec<Header>,
    // Present only under `format=full`. A metadata fetch simply leaves these
    // absent, which is why one struct serves both shapes.
    #[serde(rename = "mimeType", default)]
    mime_type: Option<String>,
    #[serde(default)]
    body: Option<MessageBody>,
    #[serde(default)]
    parts: Vec<MessagePayload>,
}

#[derive(Deserialize)]
struct MessageBody {
    /// base64url, per Gmail's API. Absent for a part that is only a container.
    #[serde(default)]
    data: Option<String>,
}
#[derive(Deserialize)]
struct Header {
    name: String,
    value: String,
}

fn find_header<'a>(headers: &'a [Header], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case(name))
        .map(|h| h.value.as_str())
}

/// Fetches metadata for one thread (read-only, `format=metadata`). Gmail 404
/// and 410 responses are an authoritative missing observation, not a retryable
/// transport failure.
pub fn thread_meta_lookup(token: &str, id: &str) -> Result<Option<ThreadMeta>> {
    std::thread::sleep(THREAD_FETCH_PAUSE);
    let http = client()?;
    let url = format!("{}/{id}", threads_url());
    let resp = http
        .get(&url)
        .bearer_auth(token)
        .query(&[
            ("format", "metadata"),
            ("metadataHeaders", "From"),
            ("metadataHeaders", "Subject"),
            ("metadataHeaders", "Date"),
            ("metadataHeaders", "List-Unsubscribe"),
        ])
        .send()?;
    if is_missing_thread_status(resp.status().as_u16()) {
        return Ok(None);
    }
    if !resp.status().is_success() {
        return Err(CommsError::Other(format!(
            "thread metadata failed with HTTP {}",
            resp.status()
        )));
    }

    let detail: ThreadDetail = resp.json()?;
    let mut meta = ThreadMeta {
        id: id.to_string(),
        ..Default::default()
    };

    meta.label_ids = detail
        .messages
        .iter()
        .flat_map(|message| message.label_ids.iter().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    if let Some(msg) = detail.messages.last() {
        meta.snippet = msg.snippet.clone();
        meta.internal_date_ms = msg
            .internal_date
            .as_ref()
            .and_then(|s| s.parse::<i64>().ok());
        if let Some(payload) = &msg.payload {
            meta.from_addr = find_header(&payload.headers, "From").map(str::to_string);
            meta.subject = find_header(&payload.headers, "Subject").map(str::to_string);
            meta.date = find_header(&payload.headers, "Date").map(str::to_string);
            meta.list_unsubscribe =
                find_header(&payload.headers, "List-Unsubscribe").map(str::to_string);
        }
    }
    Ok(Some(meta))
}

/// Max characters of decoded body text returned. The digest engine caps again
/// at its own input limit; this one exists so a 40 MB newsletter never becomes
/// a 40 MB `String` in the first place.
const BODY_TEXT_CAP: usize = 60_000;

/// The plain text of a thread's latest message, fetched with `format=full`.
///
/// **Nothing here is persisted.** The value is handed straight to the digest
/// engine and dropped. A raw mail is never kept as a local copy, so the only
/// thing that survives this call is the digest row. It is deliberately not reachable from the sweep: the sweep stays
/// on `format=metadata`, and reading a body is a separate, bounded, explicit
/// act.
///
/// The latest message rather than the whole thread, matching
/// [`thread_meta_lookup`] — a reply chain repeats its own quoted history, so
/// concatenating every message digests the same text several times over.
///
/// `text/plain` is preferred and `text/html` is stripped through the same
/// extraction implementation the article path uses. A missing thread is `None`,
/// the same authoritative-absence signal metadata gives.
pub fn thread_body_text(token: &str, id: &str) -> Result<Option<String>> {
    std::thread::sleep(THREAD_FETCH_PAUSE);
    let http = client()?;
    let response = http
        .get(format!("{}/{id}", threads_url()))
        .bearer_auth(token)
        .query(&[("format", "full")])
        .send()?;
    if is_missing_thread_status(response.status().as_u16()) {
        return Ok(None);
    }
    if !response.status().is_success() {
        return Err(CommsError::Other(format!(
            "thread body failed with HTTP {}",
            response.status()
        )));
    }
    let detail: ThreadDetail = response.json()?;
    let Some(payload) = detail.messages.last().and_then(|m| m.payload.as_ref()) else {
        return Ok(None);
    };

    let text = collect_part(payload, "text/plain")
        .or_else(|| {
            collect_part(payload, "text/html").map(|html| crate::extraction::html_to_lines(&html))
        })
        .map(|text| {
            let text = text.trim();
            text.chars().take(BODY_TEXT_CAP).collect::<String>()
        })
        .filter(|text| !text.is_empty());
    Ok(text)
}

/// Depth-first search for the first part of a MIME type, decoded.
///
/// Depth-first because a `multipart/alternative` nests its plain and HTML
/// alternatives underneath, and a breadth-first walk would find the container
/// before either.
fn collect_part(payload: &MessagePayload, want: &str) -> Option<String> {
    if payload
        .mime_type
        .as_deref()
        .is_some_and(|mime| mime.eq_ignore_ascii_case(want))
    {
        if let Some(text) = payload.body.as_ref().and_then(|body| body.data.as_deref()) {
            if let Some(decoded) = decode_body(text) {
                return Some(decoded);
            }
        }
    }
    payload
        .parts
        .iter()
        .find_map(|part| collect_part(part, want))
}

/// Gmail encodes body data base64url without padding. A part that does not
/// decode, or is not UTF-8, is skipped rather than returned as replacement
/// characters — the next alternative is usually readable.
fn decode_body(data: &str) -> Option<String> {
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(data.trim_end_matches('='))
        .ok()?;
    String::from_utf8(bytes).ok()
}

/// Compatibility helper for callers that require an existing thread.
pub fn thread_meta(token: &str, id: &str) -> Result<ThreadMeta> {
    thread_meta_lookup(token, id)?
        .ok_or_else(|| CommsError::Other("Gmail thread is missing".into()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadAction {
    Archive,
    Trash,
    RestoreArchive,
    RestoreTrash,
}

impl ThreadAction {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "archive" => Some(Self::Archive),
            "trash" => Some(Self::Trash),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Archive => "archive",
            Self::Trash => "trash",
            Self::RestoreArchive | Self::RestoreTrash => "restore",
        }
    }
}

fn valid_thread_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && id.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Apply one deliberately small Gmail mutation. Archive removes only the
/// INBOX label. Trash uses Gmail's recoverable Trash operation; restore either
/// adds INBOX back or uses untrash. Sjel never calls permanent delete.
pub fn apply_thread_action(token: &str, id: &str, action: ThreadAction) -> Result<()> {
    if !valid_thread_id(id) {
        return Err(CommsError::Other("invalid Gmail thread id".into()));
    }

    let http = client()?;
    let response = thread_action_request(&http, &threads_url(), token, id, action).send()?;

    if !response.status().is_success() {
        return Err(CommsError::Other(format!(
            "Gmail {} failed with HTTP {}",
            action.as_str(),
            response.status()
        )));
    }
    Ok(())
}

#[derive(Deserialize)]
struct LabelList {
    #[serde(default)]
    labels: Vec<Label>,
}

#[derive(Deserialize)]
struct Label {
    id: String,
    name: String,
}

/// Resolve `Waiting` to its Gmail label id, creating the label if this account
/// has never had one.
///
/// Looked up by name every call rather than cached. The id is stable, but a
/// cache would hold it across the operator deleting the label in the Gmail UI,
/// and the next write would then fail with a 404 that reads like a bug rather
/// than like "the label is gone". Looking it up costs one request on a path that
/// is already an explicit, human-triggered action.
///
/// `labelListVisibility`/`messageListVisibility` are left at Gmail's defaults on
/// create, so the label shows up in the sidebar. A state label nobody can see is
/// a state nobody reviews, which is the whole point of having one.
pub fn ensure_waiting_label(token: &str) -> Result<String> {
    let http = client()?;
    let response = http.get(labels_url()).bearer_auth(token).send()?;
    if !response.status().is_success() {
        return Err(CommsError::Other(format!(
            "Gmail label list failed with HTTP {}",
            response.status()
        )));
    }

    if let Some(existing) = response
        .json::<LabelList>()?
        .labels
        .into_iter()
        .find(|label| label.name == WAITING_LABEL)
    {
        return Ok(existing.id);
    }

    let created = http
        .post(labels_url())
        .bearer_auth(token)
        .json(&serde_json::json!({ "name": WAITING_LABEL }))
        .send()?;
    if !created.status().is_success() {
        return Err(CommsError::Other(format!(
            "Gmail label create failed with HTTP {}",
            created.status()
        )));
    }
    Ok(created.json::<Label>()?.id)
}

/// Add or remove `Waiting` on one thread.
///
/// Deliberately *only* the label. It does not archive, and archiving does not
/// set it. They are two decisions — "this is out of my inbox" and "I am blocked
/// on someone else" — and folding them together would archive as a side effect
/// of marking, which is the mutation the doctrine reserves for an explicit ask.
/// A thread can be Waiting and still sitting in the inbox; that is a valid, and
/// common, state.
pub fn set_thread_waiting(token: &str, id: &str, label_id: &str, waiting: bool) -> Result<()> {
    if !valid_thread_id(id) {
        return Err(CommsError::Other("invalid Gmail thread id".into()));
    }

    let http = client()?;
    let response = waiting_request(&http, &threads_url(), token, id, label_id, waiting).send()?;
    if !response.status().is_success() {
        return Err(CommsError::Other(format!(
            "Gmail waiting {} failed with HTTP {}",
            if waiting { "set" } else { "clear" },
            response.status()
        )));
    }
    Ok(())
}

fn waiting_request(
    http: &reqwest::blocking::Client,
    threads_url: &str,
    token: &str,
    id: &str,
    label_id: &str,
    waiting: bool,
) -> reqwest::blocking::RequestBuilder {
    let body = if waiting {
        serde_json::json!({ "addLabelIds": [label_id] })
    } else {
        serde_json::json!({ "removeLabelIds": [label_id] })
    };
    http.post(format!("{threads_url}/{id}/modify"))
        .bearer_auth(token)
        .json(&body)
}

fn thread_action_request(
    http: &reqwest::blocking::Client,
    threads_url: &str,
    token: &str,
    id: &str,
    action: ThreadAction,
) -> reqwest::blocking::RequestBuilder {
    match action {
        ThreadAction::Archive => http
            .post(format!("{threads_url}/{id}/modify"))
            .bearer_auth(token)
            .json(&serde_json::json!({ "removeLabelIds": ["INBOX"] })),
        ThreadAction::Trash => http
            .post(format!("{threads_url}/{id}/trash"))
            .bearer_auth(token)
            .header(reqwest::header::CONTENT_LENGTH, "0")
            .body(Vec::new()),
        ThreadAction::RestoreArchive => http
            .post(format!("{threads_url}/{id}/modify"))
            .bearer_auth(token)
            .json(&serde_json::json!({ "addLabelIds": ["INBOX"] })),
        ThreadAction::RestoreTrash => http
            .post(format!("{threads_url}/{id}/untrash"))
            .bearer_auth(token)
            .header(reqwest::header::CONTENT_LENGTH, "0")
            .body(Vec::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gmail_override_accepts_only_loopback() {
        assert_eq!(gmail_override(None), Ok(None));
        assert_eq!(gmail_override(Some("  ")), Ok(None));
        assert_eq!(
            gmail_override(Some("http://127.0.0.1:8099/gmail/")),
            Ok(Some("http://127.0.0.1:8099/gmail".to_owned()))
        );
        assert!(gmail_override(Some("http://localhost:8099/gmail")).is_ok());
        for refused in [
            "https://gmail.example.org",
            "http://127.0.0.1.example.org:80",
            "http://10.0.0.5:8099",
        ] {
            assert!(gmail_override(Some(refused)).is_err(), "{refused}");
        }
    }

    #[test]
    fn header_lookup_is_case_insensitive() {
        let headers = vec![
            Header {
                name: "From".into(),
                value: "a@b.com".into(),
            },
            Header {
                name: "List-Unsubscribe".into(),
                value: "<mailto:x>".into(),
            },
        ];
        assert_eq!(find_header(&headers, "from"), Some("a@b.com"));
        assert_eq!(
            find_header(&headers, "LIST-UNSUBSCRIBE"),
            Some("<mailto:x>")
        );
        assert_eq!(find_header(&headers, "Subject"), None);
    }

    #[test]
    fn google_error_reason_is_allowlisted_and_discards_provider_message() {
        let quota = br#"{"error":{"errors":[{"reason":"userRateLimitExceeded","message":"private content"}]}}"#;
        assert_eq!(safe_google_error_reason(quota), Some("quota"));
        let unknown =
            br#"{"error":{"errors":[{"reason":"newProviderReason","message":"private content"}]}}"#;
        assert_eq!(safe_google_error_reason(unknown), None);
    }

    #[test]
    fn thread_actions_and_ids_are_bounded() {
        assert_eq!(ThreadAction::parse("archive"), Some(ThreadAction::Archive));
        assert_eq!(ThreadAction::parse("trash"), Some(ThreadAction::Trash));
        assert_eq!(ThreadAction::parse("delete"), None);
        assert!(valid_thread_id("18f17d0a9bc123ef"));
        assert!(!valid_thread_id("../thread"));
        assert!(!valid_thread_id(""));
    }

    #[test]
    fn thread_location_prefers_trash_then_inbox() {
        assert_eq!(
            ThreadLocation::from_labels(&["STARRED".into(), "INBOX".into()]),
            ThreadLocation::Inbox
        );
        assert_eq!(
            ThreadLocation::from_labels(&["INBOX".into(), "TRASH".into()]),
            ThreadLocation::Trash
        );
        assert_eq!(
            ThreadLocation::from_labels(&["STARRED".into()]),
            ThreadLocation::Archive
        );
        assert!(is_missing_thread_status(404));
        assert!(is_missing_thread_status(410));
        assert!(!is_missing_thread_status(429));
    }

    #[test]
    fn gmail_action_requests_have_explicit_bodies() {
        let http = client().unwrap();
        let archive = thread_action_request(
            &http,
            "https://gmail.example/threads",
            "test-token",
            "18f17d0a9bc123ef",
            ThreadAction::Archive,
        )
        .build()
        .unwrap();
        assert_eq!(archive.method(), reqwest::Method::POST);
        assert!(archive.url().path().ends_with("/modify"));
        assert_eq!(
            archive.headers()[reqwest::header::CONTENT_TYPE],
            "application/json"
        );
        assert!(archive
            .body()
            .and_then(reqwest::blocking::Body::as_bytes)
            .is_some_and(|body| body == br#"{"removeLabelIds":["INBOX"]}"#));

        let trash = thread_action_request(
            &http,
            "https://gmail.example/threads",
            "test-token",
            "18f17d0a9bc123ef",
            ThreadAction::Trash,
        )
        .build()
        .unwrap();
        assert_eq!(trash.method(), reqwest::Method::POST);
        assert!(trash.url().path().ends_with("/trash"));
        assert_eq!(trash.headers()[reqwest::header::CONTENT_LENGTH], "0");
        assert!(trash
            .body()
            .and_then(reqwest::blocking::Body::as_bytes)
            .is_some_and(|body| body.is_empty()));

        let restore_archive = thread_action_request(
            &http,
            "https://gmail.example/threads",
            "test-token",
            "18f17d0a9bc123ef",
            ThreadAction::RestoreArchive,
        )
        .build()
        .unwrap();
        assert!(restore_archive.url().path().ends_with("/modify"));
        assert!(restore_archive
            .body()
            .and_then(reqwest::blocking::Body::as_bytes)
            .is_some_and(|body| body == br#"{"addLabelIds":["INBOX"]}"#));

        let restore_trash = thread_action_request(
            &http,
            "https://gmail.example/threads",
            "test-token",
            "18f17d0a9bc123ef",
            ThreadAction::RestoreTrash,
        )
        .build()
        .unwrap();
        assert!(restore_trash.url().path().ends_with("/untrash"));
        assert_eq!(
            restore_trash.headers()[reqwest::header::CONTENT_LENGTH],
            "0"
        );
    }

    /// The `Waiting` write touches one label and nothing else.
    ///
    /// Asserted on the exact bytes rather than on "it calls modify", because the
    /// failure that matters here is a body that also carries `removeLabelIds:
    /// ["INBOX"]` — marking a thread as blocked would then archive it, and the
    /// doctrine reserves archiving for an explicit ask.
    #[test]
    fn waiting_requests_touch_only_the_waiting_label() {
        let http = client().unwrap();

        let set = waiting_request(
            &http,
            "https://gmail.example/threads",
            "test-token",
            "18f17d0a9bc123ef",
            "Label_42",
            true,
        )
        .build()
        .unwrap();
        assert_eq!(set.method(), reqwest::Method::POST);
        assert!(set.url().path().ends_with("/modify"));
        assert!(set
            .body()
            .and_then(reqwest::blocking::Body::as_bytes)
            .is_some_and(|body| body == br#"{"addLabelIds":["Label_42"]}"#));

        let cleared = waiting_request(
            &http,
            "https://gmail.example/threads",
            "test-token",
            "18f17d0a9bc123ef",
            "Label_42",
            false,
        )
        .build()
        .unwrap();
        assert!(cleared
            .body()
            .and_then(reqwest::blocking::Body::as_bytes)
            .is_some_and(|body| body == br#"{"removeLabelIds":["Label_42"]}"#));
    }

    /// The doctrine's ceiling, as a test rather than as a sentence in a README.
    #[test]
    fn only_one_label_is_ever_created() {
        assert_eq!(WAITING_LABEL, "Waiting");
    }

    #[test]
    fn waiting_rejects_a_thread_id_that_is_not_one() {
        assert!(set_thread_waiting("token", "not a thread id", "Label_42", true).is_err());
        assert!(set_thread_waiting("token", "", "Label_42", true).is_err());
    }
}
