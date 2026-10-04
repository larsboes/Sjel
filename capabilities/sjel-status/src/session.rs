//! The local browser login (ISA ISC-45).
//!
//! The shell's TCP listener requires a credential, and a browser at `127.0.0.1:8082` can carry
//! neither the deployment token nor a tailnet identity. The Mac menu-bar app holds the token
//! (login Keychain), so it trades it for a ticket here, opens the browser at
//! [`sjel_server::SESSION_OPEN_PATH`] with that ticket, and the browser leaves with a cookie.
//! The principal's rulings, 2026-10-01: no command line for the user, and a session that lasts
//! 30 days and is renewed while it is used.
//!
//! The session itself is minted and verified by [`sjel_server::session`] since 2026-10-04: a
//! signed token keyed by the deployment token, which every capability verifies. This module owns
//! only what is shell-specific — the single-use ticket and the HTTP handlers. The session table
//! that used to live here is gone: a capability serving its own panel (soundscape) could not read
//! the shell's store, so its browser was answered `401`; a signature is verifiable anywhere.
//!
//! - A ticket is single-use and lives 60 seconds, in memory: it only has to survive the hop from
//!   the app to the browser. Only its digest is kept.
//! - The cookie is `HttpOnly` and `SameSite=Strict`, so a page on another site can neither read
//!   it nor make the browser send it. It is not `Secure`, because the listener is plain HTTP on
//!   loopback.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::extract::Query;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use sjel_server::SignedSessions;

/// How long a ticket waits for the browser.
const TICKET_TTL: Duration = Duration::from_secs(60);

pub struct Sessions {
    signer: SignedSessions,
    tickets: Mutex<HashMap<String, Instant>>,
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
    pub fn new(signer: SignedSessions) -> Self {
        Self {
            signer,
            tickets: Mutex::new(HashMap::new()),
        }
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

    /// Consumes `ticket`, and returns a new signed session cookie value when it was live.
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
        Ok(Some(self.signer.mint()?))
    }
}

/// The process's one ticket table. `None` when no deployment token is configured, because then
/// nothing can be signed: the gate's other credentials still work.
pub fn sessions() -> Option<Arc<Sessions>> {
    static SESSIONS: OnceLock<Option<Arc<Sessions>>> = OnceLock::new();
    SESSIONS
        .get_or_init(|| match SignedSessions::from_deployment() {
            Some(signer) => Some(Arc::new(Sessions::new(signer))),
            None => {
                eprintln!(
                    "[sjel-status] browser sessions are unavailable: no deployment token is configured"
                );
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
                    sjel_server::session_cookie_header(&session, sjel_server::SESSION_TTL_SECONDS),
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

/// `POST /api/sjel-status/session/logout`. Clears the cookie.
///
/// The signed token cannot be revoked on its own — that is the trade for verifying without a
/// store — so a stolen copy stays valid until it expires; rotating the deployment token ends
/// every session at once.
pub async fn logout_handler(_headers: HeaderMap) -> Response {
    (
        StatusCode::OK,
        [(
            header::SET_COOKIE,
            sjel_server::session_cookie_header("", 0),
        )],
        Json(json!({ "ended": true })),
    )
        .into_response()
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

    fn sessions() -> Sessions {
        Sessions::new(SignedSessions::new(b"deployment-token".to_vec()))
    }

    fn signer() -> SignedSessions {
        SignedSessions::new(b"deployment-token".to_vec())
    }

    #[test]
    fn a_ticket_opens_one_session_once() {
        let sessions = sessions();
        let ticket = sessions.mint_ticket().unwrap();
        let session = sessions
            .redeem(&ticket)
            .unwrap()
            .expect("a live ticket redeems");
        assert!(signer().verify(&session));
        assert!(
            sessions.redeem(&ticket).unwrap().is_none(),
            "a ticket is single-use"
        );
        assert!(!signer().verify("not-a-session"));
    }

    #[test]
    fn an_unknown_ticket_opens_nothing() {
        let sessions = sessions();
        assert!(sessions.redeem("never-minted").unwrap().is_none());
    }

    #[test]
    fn a_session_signed_here_is_not_readable_without_the_key() {
        let sessions = sessions();
        let session = sessions
            .redeem(&sessions.mint_ticket().unwrap())
            .unwrap()
            .unwrap();
        assert!(signer().verify(&session));
        assert!(!SignedSessions::new(b"another-token".to_vec()).verify(&session));
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
