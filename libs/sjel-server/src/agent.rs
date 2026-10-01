//! The agent identity at the gate: read-only, and every response pseudonymized (ISA F9).
//!
//! An agent holds its own token. The deployment stores only its SHA-256, in
//! `<overlay>/config/agent-token.sha256`, so nothing on the server side can hand the token
//! out again. The token itself lives in the login Keychain, and `tools/capability-auth` reads
//! it from there.
//!
//! A request with the agent token:
//!
//! 1. is admitted for `GET` and `HEAD` only (ISC-38);
//! 2. has pseudonym tokens in its query string mapped back, so an agent can filter by a
//!    value it only knows as `<SENDER_k3x9qa>`;
//! 3. gets a JSON response rewritten by `sjel_pseudonymize::view::agent_view`, with c3
//!    objects removed. A response that is not JSON is refused, because it cannot be rewritten.
//!
//! Only a capability that calls [`crate::InboundAuth::admit_agents`] accepts the agent token.
//! Everywhere else it is an unknown token and answers 401.
//!
//! Tokens are keyed ([`sjel_pseudonymize::keyed`]): the key is derived from
//! `<overlay>/secrets/agent-pseudonym.key` and the session id, so every capability gives one
//! value one token without sharing state. The session id is the `X-Sjel-Agent-Session` header,
//! or the current UTC day when it is absent.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::extract::Request;
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sjel_pseudonymize::view::{agent_view, is_withheld};
use sjel_pseudonymize::{EntityRegistry, PseudonymizerSession};

/// The header an agent may send to scope its tokens to one conversation.
pub const AGENT_SESSION_HEADER: &str = "x-sjel-agent-session";
/// The Q9b receipt for the response: what was replaced.
pub const RECEIPT_HEADER: &str = "x-sjel-pseudonymized";
/// How many c3 objects the view removed.
pub const WITHHELD_HEADER: &str = "x-sjel-withheld";

/// A session unused this long is dropped, with the only way back from its tokens.
const SESSION_TTL: Duration = Duration::from_secs(24 * 60 * 60);
/// Larger responses are refused rather than buffered: the view needs the whole body.
const MAX_RESPONSE_BYTES: usize = 32 * 1024 * 1024;
/// Overlay-relative, shared with comms' loader (`capabilities/comms/src/people_registry.rs`).
const PEOPLE_REGISTRY_REL: &str = "data/vault/people-registry.json";
/// Registry entries shorter than this are skipped, as in comms (`MIN_NAME_CHARS`).
const MIN_NAME_CHARS: usize = 3;

/// Everything the gate needs to admit and rewrite agent requests.
pub struct AgentAccess {
    token_sha256: [u8; 32],
    machine_secret: Vec<u8>,
    registry: EntityRegistry,
    sessions: Mutex<HashMap<String, (Instant, PseudonymizerSession)>>,
}

/// Values stay out of `Debug`: the secret, the hash and every session's maps.
impl std::fmt::Debug for AgentAccess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentAccess")
            .field("registry_names", &self.registry.len())
            .finish_non_exhaustive()
    }
}

impl AgentAccess {
    /// No I/O. For tests and for a caller that resolved the three inputs itself.
    pub fn new(token_sha256: [u8; 32], machine_secret: Vec<u8>, registry: EntityRegistry) -> Self {
        Self {
            token_sha256,
            machine_secret,
            registry,
            sessions: Mutex::new(HashMap::new()),
        }
    }

    /// The deployment's agent access, or `None` when it is not enrolled.
    ///
    /// Both the hash and the key must exist. An absent people registry is allowed and
    /// logged: the shape rules (addresses, numbers, links) still apply without it.
    pub fn from_deployment() -> Option<Self> {
        let root = sjel_config::overlay_root()?;
        let hash = read_hash(&root.join("config/agent-token.sha256"))?;
        let secret = std::fs::read(root.join("secrets/agent-pseudonym.key")).ok()?;
        if secret.len() < 32 {
            eprintln!("agent access: agent-pseudonym.key is shorter than 32 bytes; agents refused");
            return None;
        }
        let names = people_registry_path(&root)
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|text| people_from_artifact(&text));
        if names.is_none() {
            eprintln!(
                "agent access: no people registry loaded; names are caught by cue rules only"
            );
        }
        let registry = EntityRegistry::builder()
            .add_people(names.unwrap_or_default())
            .build();
        Some(Self::new(hash, secret, registry))
    }

    pub(crate) fn matches(&self, presented: &str) -> bool {
        let digest: [u8; 32] = Sha256::digest(presented.as_bytes()).into();
        crate::auth::constant_time_eq(&digest, &self.token_sha256)
    }

    fn with_session<T>(&self, id: &str, f: impl FnOnce(&mut PseudonymizerSession) -> T) -> T {
        let mut sessions = self.sessions.lock().unwrap_or_else(|p| p.into_inner());
        let now = Instant::now();
        sessions.retain(|_, (used, _)| now.duration_since(*used) < SESSION_TTL);
        let key = sjel_pseudonymize::keyed::session_key(&self.machine_secret, id);
        let entry = sessions
            .entry(id.to_string())
            .or_insert_with(|| (now, PseudonymizerSession::keyed(key)));
        entry.0 = now;
        f(&mut entry.1)
    }
}

fn read_hash(path: &Path) -> Option<[u8; 32]> {
    let text = std::fs::read_to_string(path).ok()?;
    let hex = text.split_whitespace().next()?;
    if hex.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(hex.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

fn people_registry_path(root: &Path) -> Option<PathBuf> {
    match std::env::var("SJEL_PEOPLE_REGISTRY") {
        Ok(p) if !p.trim().is_empty() => Some(PathBuf::from(p)),
        _ => Some(root.join(PEOPLE_REGISTRY_REL)),
    }
}

/// The `tokens` of the artifact `vault names --json` writes, as comms reads them.
fn people_from_artifact(text: &str) -> Option<Vec<String>> {
    let value: Value = serde_json::from_str(text).ok()?;
    Some(
        value
            .get("tokens")?
            .as_array()?
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|t| t.chars().count() >= MIN_NAME_CHARS)
            .map(str::to_string)
            .collect(),
    )
}

fn session_id(request: &Request) -> String {
    let header = request
        .headers()
        .get(AGENT_SESSION_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|v| !v.is_empty() && v.len() <= 128);
    match header {
        Some(id) => format!("session:{id}"),
        None => {
            let days = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs() / 86_400)
                .unwrap_or(0);
            format!("day:{days}")
        }
    }
}

fn refuse(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
}

/// The agent branch of the gate. The caller has already matched the token.
pub(crate) async fn admit_agent(
    access: Arc<AgentAccess>,
    mut request: Request,
    next: Next,
) -> Response {
    if request.method() != Method::GET && request.method() != Method::HEAD {
        return refuse(
            StatusCode::FORBIDDEN,
            "the agent token is read-only: only GET and HEAD are admitted",
        );
    }
    let id = session_id(&request);

    // Map pseudonyms in the query back, so a filter by `<SENDER_…>` reaches the real value.
    if let Some(query) = request.uri().query() {
        let decoded = percent_decode(query);
        let restored = access.with_session(&id, |s| s.rehydrate_text(&decoded));
        if restored != decoded {
            let path = request.uri().path().to_string();
            let rebuilt = format!("{path}?{}", percent_encode_query(&restored));
            match rebuilt.parse() {
                Ok(uri) => *request.uri_mut() = uri,
                Err(_) => {
                    return refuse(StatusCode::BAD_REQUEST, "the query could not be restored");
                }
            }
        }
    }

    let response = next.run(request).await;
    let (mut parts, body) = response.into_parts();
    let is_json = parts
        .headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("application/json"));
    let Ok(bytes) = axum::body::to_bytes(body, MAX_RESPONSE_BYTES).await else {
        return refuse(
            StatusCode::PAYLOAD_TOO_LARGE,
            "the response is too large to pseudonymize",
        );
    };
    if bytes.is_empty() {
        return Response::from_parts(parts, axum::body::Body::empty());
    }
    if !is_json {
        return refuse(
            StatusCode::NOT_ACCEPTABLE,
            "agent reads are JSON only: this response cannot be pseudonymized",
        );
    }
    let Ok(mut value) = serde_json::from_slice::<Value>(&bytes) else {
        return refuse(
            StatusCode::BAD_GATEWAY,
            "the capability answered invalid JSON",
        );
    };
    if is_withheld(&value) {
        return refuse(StatusCode::FORBIDDEN, "withheld: this item is Secret (c3)");
    }
    let (report, receipt) = access.with_session(&id, |session| {
        session.take_findings();
        let report = agent_view(&mut value, session, &access.registry);
        (report, session.receipt())
    });
    let out = serde_json::to_vec(&value).unwrap_or_default();
    parts.headers.remove(header::CONTENT_LENGTH);
    let receipt = receipt.unwrap_or_else(|| "nothing replaced".to_string());
    if let Ok(v) = HeaderValue::from_str(&receipt) {
        parts.headers.insert(RECEIPT_HEADER, v);
    }
    parts
        .headers
        .insert(WITHHELD_HEADER, HeaderValue::from(report.withheld_secret));
    Response::from_parts(parts, axum::body::Body::from(out))
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let pair = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
                match pair.and_then(|p| u8::from_str_radix(p, 16).ok()) {
                    Some(b) => {
                        out.push(b);
                        i += 3;
                    }
                    None => {
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Re-encodes a decoded query: `&` and `=` stay separators, everything else unsafe is escaped.
fn percent_encode_query(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'&' | b'=' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_artifact_parser_skips_short_entries() {
        let names = people_from_artifact(r#"{"tokens":["Jo","Katrin","Anna Lena"]}"#).unwrap();
        assert_eq!(names, vec!["Katrin".to_string(), "Anna Lena".to_string()]);
    }

    #[test]
    fn a_query_round_trips_through_decode_and_encode() {
        let decoded = percent_decode("from=%3CSENDER_abc%3E&status=proposed");
        assert_eq!(decoded, "from=<SENDER_abc>&status=proposed");
        assert_eq!(
            percent_encode_query("from=a b@x.de&status=proposed"),
            "from=a%20b%40x.de&status=proposed"
        );
    }

    #[test]
    fn the_token_is_matched_by_its_hash() {
        let hash: [u8; 32] = Sha256::digest(b"agent-secret").into();
        let access = AgentAccess::new(hash, vec![7; 32], EntityRegistry::builder().build());
        assert!(access.matches("agent-secret"));
        assert!(!access.matches("agent-secreT"));
    }
}
