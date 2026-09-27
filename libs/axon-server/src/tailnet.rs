//! The tailnet identity gate: who a request proves it is when it arrived
//! through `tailscale serve`.
//!
//! ## Why a second gate exists at all
//!
//! [`crate::auth`] gates on a shared secret, which works for every caller that
//! can set a header — `curl`, a sibling capability, the dashboard's Vite proxy.
//! It does not work for the one caller the tailnet exists to serve: a browser
//! on the phone, loading the built SPA, issuing relative `fetch` calls. Giving
//! that page the token means shipping the deployment's shared secret to a
//! browser, where it lives in the bundle and in every cache that touches it.
//!
//! `tailscale serve` already knows who the caller is. It terminates TLS for
//! `https://<host>.<tailnet>.ts.net`, authenticates the peer against the
//! tailnet, and hands the backend a set of identity headers. So the identity is
//! available without a secret, and the gate below reads it.
//!
//! ## Why the header can be trusted
//!
//! Because the proxy overwrites it. Measured against tailscale 1.102.3 on
//! 2026-09-06: a request carrying `Tailscale-User-Login: attacker@evil.example`
//! and `X-Forwarded-For: 9.9.9.9` reached the backend as the authenticated
//! node's own login and tailnet address. A client cannot inject an identity
//! through the proxy; it can only fail to have one.
//!
//! That measurement is the whole basis of this module, so it is a test
//! (`the_header_is_not_a_capability_a_client_can_grant_itself` records the
//! reasoning) and a `tools/doctor` check (`Tailnet identity gate`) rather than a
//! sentence. If `tailscale serve` is ever reconfigured as a raw TCP forward, no
//! identity header is injected, every tailnet request becomes indistinguishable
//! from a loopback one, and this gate silently stops gating. Doctor fails on
//! exactly that shape.
//!
//! ## What this gate does NOT do
//!
//! It does not change the loopback trust model. A process on this machine can
//! set any header it likes, so the identity below is meaningful only for
//! requests that came through the proxy. That is not a weakening: a local
//! process already reaches `127.0.0.1:<port>` directly, which is the trust
//! boundary every capability has always had (PRD §7.1, single operator, Q3).
//!
//! It also never satisfies [`crate::InboundAuth::refuse_without_token`]. comms
//! turns that on because `POST /ingest` fetches an attacker-chosen URL and "a
//! page open in the operator's own browser is already inside the loopback
//! boundary" — a route with that property wants the secret, not a name.

use axum::http::HeaderMap;

/// The login `tailscale serve` vouches for. Lower-case because `HeaderMap`
/// lookups are case-insensitive but the constant is compared in tests.
const IDENTITY_HEADER: &str = "tailscale-user-login";

/// The deployment key naming who may reach this machine from the tailnet.
const OPERATOR_KEY: &str = "SJEL_TAILNET_OPERATOR";

/// How a request reached this server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Arrival {
    /// No identity header. Either a direct loopback caller, or a tailnet caller
    /// whose proxy is not injecting identity — which is why the doctor check
    /// exists rather than this variant being treated as proof of locality.
    Direct,
    /// `tailscale serve` authenticated the peer and named it.
    Tailnet(String),
}

/// Reads the arrival identity out of the request headers.
pub(crate) fn arrival(headers: &HeaderMap) -> Arrival {
    match headers.get(IDENTITY_HEADER).and_then(|v| v.to_str().ok()) {
        Some(login) if !login.trim().is_empty() => Arrival::Tailnet(login.trim().to_string()),
        _ => Arrival::Direct,
    }
}

/// `true` when `login` is the declared operator.
///
/// ASCII-case-insensitive because the local part of a mail address is compared
/// case-sensitively by the RFC and case-insensitively by every provider that
/// issues one, including the identity provider behind this tailnet. Matching
/// the stricter rule would reject the operator for typing their own address in
/// a different case, which is a lockout with no attacker on the other side.
///
/// Not constant-time, deliberately, and this is the difference from
/// [`crate::auth`]: a login is not a secret. It appears in the tailnet admin
/// console, in `tailscale status`, and in the header of every request that
/// arrives. Timing-hardening a public value would imply it is one.
pub(crate) fn is_operator(login: &str, operator: &str) -> bool {
    let (login, operator) = (login.trim(), operator.trim());
    // Neither end can be blank. Both callers already filter empties — the
    // resolver on the config side, `arrival` on the request side — so this is
    // the third guard on a comparison whose "" == "" case would admit everyone.
    // A predicate that is safe only because of who calls it is one refactor away
    // from being unsafe.
    !login.is_empty() && !operator.is_empty() && login.eq_ignore_ascii_case(operator)
}

/// `SJEL_TAILNET_OPERATOR` from `<overlay>/config/deployment.env`.
///
/// A value rather than a file reference, unlike `SJEL_INBOUND_TOKEN_FILE`: this
/// is a login, not a credential, and the split that file documents is exactly
/// between the two. Absent yields `None`, which leaves every server behaving as
/// it did before this module existed.
pub fn deployment_operator() -> Option<String> {
    let body = std::fs::read_to_string(axon_config::overlay_config("deployment.env")?).ok()?;
    axon_config::deployment_value(&body, OPERATOR_KEY)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(
                axum::http::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                v.parse().unwrap(),
            );
        }
        h
    }

    #[test]
    fn a_request_without_the_header_is_direct() {
        assert_eq!(arrival(&headers(&[])), Arrival::Direct);
        assert_eq!(
            arrival(&headers(&[("x-forwarded-for", "100.64.0.1")])),
            Arrival::Direct
        );
    }

    #[test]
    fn an_empty_header_is_direct_rather_than_an_empty_identity() {
        // A blank value must never compare equal to a blank operator key. The
        // operator resolver filters empties for the same reason, at the other end.
        assert_eq!(
            arrival(&headers(&[("tailscale-user-login", "   ")])),
            Arrival::Direct
        );
    }

    #[test]
    fn the_header_names_the_login() {
        assert_eq!(
            arrival(&headers(&[("Tailscale-User-Login", "someone@example.com")])),
            Arrival::Tailnet("someone@example.com".into())
        );
    }

    #[test]
    fn the_header_is_not_a_capability_a_client_can_grant_itself() {
        // This test cannot reach the network, so what it pins is the reasoning,
        // not the proxy. Measured directly against tailscale 1.102.3 on
        // 2026-09-06: a curl carrying `Tailscale-User-Login: attacker@evil.example`
        // and `X-Forwarded-For: 9.9.9.9` through `https://<host>.ts.net` arrived at
        // the backend as the authenticated login and 100.x address of the calling
        // node. The proxy sets these headers; it does not forward them.
        //
        // What this module must therefore never do is trust the header on a path
        // the proxy did not create. It does not: `is_operator` is consulted only
        // when the deployment declared an operator, and the reachable surface is
        // loopback either way.
        let forged = arrival(&headers(&[(
            "tailscale-user-login",
            "attacker@evil.example",
        )]));
        assert_eq!(forged, Arrival::Tailnet("attacker@evil.example".into()));
        assert!(!is_operator("attacker@evil.example", "lars@example.com"));
    }

    #[test]
    fn the_operator_matches_case_insensitively_and_trims() {
        assert!(is_operator("Lars@Example.com", "lars@example.com"));
        assert!(is_operator("  lars@example.com  ", "lars@example.com"));
        assert!(!is_operator("lars@example.com.evil", "lars@example.com"));
        assert!(!is_operator("", "lars@example.com"));
        assert!(!is_operator("lars@example.com", ""));
    }
}
