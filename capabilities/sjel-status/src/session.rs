//! The local browser login (ISA ISC-45).
//!
//! The shell's TCP listener requires a credential, and a browser at `127.0.0.1:8082` can carry
//! neither the deployment token nor a tailnet identity. The Mac menu-bar app holds the token
//! (login Keychain), so it trades it for a ticket here, opens the browser at
//! [`sjel_server::SESSION_OPEN_PATH`] with that ticket, and the browser leaves with a cookie.
//! The principal's rulings, 2026-10-01: no command line for the user, and a session that lasts
//! 30 days and is renewed while it is used.
//!
//! - A ticket is single-use and lives 60 seconds, in memory: it only has to survive the hop from
//!   the app to the browser.
//! - A session lives in the shared store, so a restart of this process logs nobody out. The
//!   store keeps a SHA-256 of the cookie, never the cookie, so a copy of the database opens
//!   nothing.
//! - The cookie is `HttpOnly` and `SameSite=Strict`, so a page on another site can neither read
//!   it nor make the browser send it. It is not `Secure`, because the listener is plain HTTP on
//!   loopback.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::extract::Query;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use sjel_store::rusqlite::{params, OptionalExtension};

/// How long a session lives without use.
const SESSION_SECONDS: i64 = 30 * 24 * 60 * 60;
/// A used session is renewed at most this often, so a page load is not a write per request.
const RENEW_AFTER_SECONDS: i64 = 60 * 60;
/// How long a ticket waits for the browser.
const TICKET_TTL: Duration = Duration::from_secs(60);
/// The table prefix in the shared store.
const PREFIX: &str = "shell_session";

pub struct Sessions {
    pool: sjel_store::Pool,
    tickets: Mutex<HashMap<String, Instant>>,
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn digest(value: &str) -> String {
    Sha256::digest(value.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn random_token() -> Result<String, String> {
    let mut raw = [0u8; 32];
    getrandom::fill(&mut raw).map_err(|e| format!("secure random source: {e}"))?;
    Ok(raw.iter().map(|b| format!("{b:02x}")).collect())
}

impl Sessions {
    pub fn open(database: &std::path::Path) -> Result<Self, String> {
        let pool = sjel_store::open_pool(database, PREFIX, |conn| {
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS shell_session_sessions (
                     hash TEXT PRIMARY KEY,
                     created_at INTEGER NOT NULL,
                     expires_at INTEGER NOT NULL,
                     renewed_at INTEGER NOT NULL
                 );",
            )?;
            Ok(())
        })
        .map_err(|e| e.to_string())?;
        Ok(Self {
            pool,
            tickets: Mutex::new(HashMap::new()),
        })
    }

    /// A new single-use ticket. Only its digest is kept.
    pub fn mint_ticket(&self) -> Result<String, String> {
        let ticket = random_token()?;
        let mut tickets = self.tickets.lock().unwrap_or_else(|p| p.into_inner());
        let at = Instant::now();
        tickets.retain(|_, expiry| *expiry > at);
        tickets.insert(digest(&ticket), at + TICKET_TTL);
        Ok(ticket)
    }

    /// Consumes `ticket`, and returns a new session cookie value when it was live.
    pub fn redeem(&self, ticket: &str) -> Result<Option<String>, String> {
        let live = {
            let mut tickets = self.tickets.lock().unwrap_or_else(|p| p.into_inner());
            tickets
                .remove(&digest(ticket))
                .is_some_and(|expiry| expiry > Instant::now())
        };
        if !live {
            return Ok(None);
        }
        let session = random_token()?;
        let at = now();
        let conn = self.pool.get().map_err(|e| e.to_string())?;
        conn.execute(
            "DELETE FROM shell_session_sessions WHERE expires_at <= ?1",
            params![at],
        )
        .map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO shell_session_sessions (hash, created_at, expires_at, renewed_at)
             VALUES (?1, ?2, ?3, ?2)",
            params![digest(&session), at, at + SESSION_SECONDS],
        )
        .map_err(|e| e.to_string())?;
        Ok(Some(session))
    }

    pub fn end(&self, session: &str) -> Result<(), String> {
        let conn = self.pool.get().map_err(|e| e.to_string())?;
        conn.execute(
            "DELETE FROM shell_session_sessions WHERE hash = ?1",
            params![digest(session)],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn live(&self, session: &str) -> Result<bool, String> {
        let hash = digest(session);
        let at = now();
        let conn = self.pool.get().map_err(|e| e.to_string())?;
        let row: Option<(i64, i64)> = conn
            .query_row(
                "SELECT expires_at, renewed_at FROM shell_session_sessions WHERE hash = ?1",
                params![hash],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let Some((expires_at, renewed_at)) = row else {
            return Ok(false);
        };
        if expires_at <= at {
            return Ok(false);
        }
        if at - renewed_at >= RENEW_AFTER_SECONDS {
            conn.execute(
                "UPDATE shell_session_sessions SET expires_at = ?1, renewed_at = ?2 WHERE hash = ?3",
                params![at + SESSION_SECONDS, at, hash],
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(true)
    }
}

impl sjel_server::SessionVerifier for Sessions {
    /// A store error refuses, because an unreadable session table is not a reason to admit.
    fn verify(&self, session: &str) -> bool {
        self.live(session).unwrap_or(false)
    }
}

/// The process's one session table, opened on first use. `None` when the shared store cannot be
/// opened, and then nobody logs in through a browser: the gate's other credentials still work.
pub fn sessions() -> Option<Arc<Sessions>> {
    static SESSIONS: OnceLock<Option<Arc<Sessions>>> = OnceLock::new();
    SESSIONS
        .get_or_init(|| match Sessions::open(&sjel_config::database_path()) {
            Ok(sessions) => Some(Arc::new(sessions)),
            Err(error) => {
                eprintln!("[sjel-status] browser sessions are unavailable: {error}");
                None
            }
        })
        .clone()
}

fn unavailable() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({ "error": "browser sessions are unavailable on this shell" })),
    )
        .into_response()
}

/// `POST /api/sjel-status/session/ticket`. Behind the gate, so only a caller that already holds a
/// credential (the Mac app, with the deployment token) can mint one.
pub async fn ticket_handler() -> Response {
    let Some(sessions) = sessions() else {
        return unavailable();
    };
    match sessions.mint_ticket() {
        Ok(ticket) => Json(json!({
            "open": format!("{}?ticket={ticket}", sjel_server::SESSION_OPEN_PATH),
            "expires_in": TICKET_TTL.as_secs(),
        }))
        .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": error })),
        )
            .into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub struct OpenQuery {
    ticket: Option<String>,
}

const EXPIRED_PAGE: &str = "<!doctype html><meta charset=utf-8><title>Sjel</title>\
<body style=\"font-family: system-ui; margin: 4rem; max-width: 32rem\">\
<h1>This link has expired</h1>\
<p>Open the dashboard again from the Sjel icon in the menu bar.</p></body>";

/// `GET /session/open?ticket=…`. Exempt from the gate on this listener: the ticket is the
/// credential. A live ticket becomes a session cookie and a redirect to the dashboard.
pub async fn open_handler(Query(query): Query<OpenQuery>) -> Response {
    let Some(sessions) = sessions() else {
        return unavailable();
    };
    let redeemed = match query.ticket.as_deref().filter(|t| !t.is_empty()) {
        Some(ticket) => sessions.redeem(ticket),
        None => Ok(None),
    };
    match redeemed {
        Ok(Some(session)) => (
            StatusCode::SEE_OTHER,
            [
                (header::LOCATION, "/".to_string()),
                (
                    header::SET_COOKIE,
                    session_cookie_header(&session, SESSION_SECONDS),
                ),
                (header::CACHE_CONTROL, "no-store".to_string()),
            ],
        )
            .into_response(),
        Ok(None) => (
            StatusCode::UNAUTHORIZED,
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            EXPIRED_PAGE,
        )
            .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": error })),
        )
            .into_response(),
    }
}

/// `POST /api/sjel-status/session/logout`. Ends the presented session and clears the cookie.
pub async fn logout_handler(headers: HeaderMap) -> Response {
    if let (Some(sessions), Some(session)) = (sessions(), sjel_server::session_cookie(&headers)) {
        if let Err(error) = sessions.end(session) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": error })),
            )
                .into_response();
        }
    }
    (
        StatusCode::OK,
        [(header::SET_COOKIE, session_cookie_header("", 0))],
        Json(json!({ "ended": true })),
    )
        .into_response()
}

fn session_cookie_header(value: &str, max_age: i64) -> String {
    format!(
        "{}={value}; Path=/; HttpOnly; SameSite=Strict; Max-Age={max_age}",
        sjel_server::SESSION_COOKIE
    )
}

/// The request's `Cookie` header without the session cookie, for the proxy hop: a capability
/// behind the shell has no use for it, and should not see it.
pub fn without_session_cookie(value: &str) -> Option<String> {
    let rest: Vec<&str> = value
        .split(';')
        .map(str::trim)
        .filter(|pair| !pair.is_empty())
        .filter(|pair| {
            pair.split_once('=')
                .is_none_or(|(name, _)| name != sjel_server::SESSION_COOKIE)
        })
        .collect();
    (!rest.is_empty()).then(|| rest.join("; "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sjel_server::SessionVerifier;

    fn store(name: &str) -> Sessions {
        let dir =
            std::env::temp_dir().join(format!("sjel-status-session-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Sessions::open(&dir.join("store.db")).unwrap()
    }

    #[test]
    fn a_ticket_opens_one_session_once() {
        let sessions = store("once");
        let ticket = sessions.mint_ticket().unwrap();
        let session = sessions
            .redeem(&ticket)
            .unwrap()
            .expect("a live ticket redeems");
        assert!(sessions.verify(&session));
        assert!(
            sessions.redeem(&ticket).unwrap().is_none(),
            "a ticket is single-use"
        );
        assert!(!sessions.verify("not-a-session"));
    }

    #[test]
    fn the_store_keeps_a_digest_not_the_cookie() {
        let sessions = store("digest");
        let ticket = sessions.mint_ticket().unwrap();
        let session = sessions.redeem(&ticket).unwrap().unwrap();
        let conn = sessions.pool.get().unwrap();
        let stored: String = conn
            .query_row("SELECT hash FROM shell_session_sessions", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_ne!(stored, session);
        assert_eq!(stored, digest(&session));
    }

    #[test]
    fn an_ended_or_expired_session_is_refused() {
        let sessions = store("ended");
        let first = sessions
            .redeem(&sessions.mint_ticket().unwrap())
            .unwrap()
            .unwrap();
        sessions.end(&first).unwrap();
        assert!(!sessions.verify(&first));

        let second = sessions
            .redeem(&sessions.mint_ticket().unwrap())
            .unwrap()
            .unwrap();
        let conn = sessions.pool.get().unwrap();
        conn.execute("UPDATE shell_session_sessions SET expires_at = 0", [])
            .unwrap();
        assert!(!sessions.verify(&second));
    }

    #[test]
    fn a_used_session_is_renewed_but_not_on_every_request() {
        let sessions = store("renew");
        let session = sessions
            .redeem(&sessions.mint_ticket().unwrap())
            .unwrap()
            .unwrap();
        let conn = sessions.pool.get().unwrap();
        let old = now() - RENEW_AFTER_SECONDS - 10;
        conn.execute(
            "UPDATE shell_session_sessions SET renewed_at = ?1, expires_at = ?2",
            params![old, now() + 100],
        )
        .unwrap();
        assert!(sessions.verify(&session));
        let (expires_at, renewed_at): (i64, i64) = conn
            .query_row(
                "SELECT expires_at, renewed_at FROM shell_session_sessions",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert!(expires_at >= now() + SESSION_SECONDS - 5);
        assert!(renewed_at > old);
    }

    #[test]
    fn the_proxy_drops_only_the_session_cookie() {
        assert_eq!(
            without_session_cookie("theme=dark; sjel_session=abc; lang=de").as_deref(),
            Some("theme=dark; lang=de")
        );
        assert_eq!(without_session_cookie("sjel_session=abc"), None);
    }
}
