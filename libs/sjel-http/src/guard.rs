//! What an outbound URL has to prove before a request is made.
//!
//! Promoted from `capabilities/comms/src/media.rs`, which was the only place in the
//! workspace that validated an outbound URL's destination — `rg` found no equivalent
//! anywhere else, while `capabilities/scouting` fetches URLs that come from adapter
//! config and remote feeds and `capabilities/places` fetches from a configured
//! geocoder base. The guard belongs on the way in to `client.get(url)`, which is
//! this crate, so one lib owns both halves of the same door.
//!
//! Three rules. The first two close two halves of one hole, and the third closes the
//! door the first two cannot see through:
//!
//! - [`check_scheme`] refuses anything that is not plain http(s). `file://` would
//!   make an extractor read the local disk.
//! - [`check_destination`] refuses a URL that resolves inside this machine or this
//!   network. `http://127.0.0.1:8086/api/plans` is still http, and every Axon service
//!   binds loopback (`libs/sjel-server`), so without it an ingested link drives an
//!   internal API from the outside. CodeQL `rust/request-forgery` reported exactly
//!   that against comms' `extract_article`.
//! - [`redirect_policy`] refuses a *hop* that leaves the public internet for this
//!   machine or this network. It is opt-out rather than opt-in, because it is the one
//!   rule a caller cannot apply for itself: the caller checks the URL it wrote, and a
//!   redirect is by definition a URL it did not write. Every client [`super::client`]
//!   and [`super::builder`] hand out carries it.
//!
//! Nothing here reads configuration. The allowlist [`check_destination`] consults is
//! the caller's, passed as a closure so a capability that has none pays nothing and
//! a capability that has one does not read its config file on the ordinary path.
//! [`redirect_policy`] needs no allowlist at all — see its own note.

use std::fmt;
use std::net::{IpAddr, ToSocketAddrs};

/// A URL this guard will not fetch, and why.
///
/// One opaque string rather than an enum of reasons: every caller reports it and
/// none of them branches on it, and the messages are asserted verbatim by comms'
/// `tests/ingest_allowlist.rs`. Wrap it in the capability's own error type with
/// `to_string()`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused(String);

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Refused {}

/// Reject anything that is not plain http(s) before a URL reaches a fetcher.
///
/// `file://` would make a subprocess extractor read the local disk, and this runs
/// behind an HTTP endpoint — the check belongs at the one door every caller goes
/// through, not at each call site.
pub fn check_scheme(url: &str) -> Result<(), Refused> {
    let low = url.trim().to_lowercase();
    if low.starts_with("http://") || low.starts_with("https://") {
        Ok(())
    } else {
        Err(Refused("only http(s) URLs can be fetched".into()))
    }
}

/// Refuse a URL that resolves to an address inside this machine or this network.
///
/// [`check_scheme`] closes `file://`; this closes the other half of the same hole.
///
/// One escape, and it is written down rather than inferred: an origin the caller
/// lists passes even when it resolves inside this machine. It exists for
/// `tools/demo-up`, which stands a synthetic origin up on loopback and seeds Comms
/// by asking it to fetch from there — the one caller that legitimately points ingest
/// at this machine. `allowed_origins` is a closure, not a slice, because comms reads
/// its from a config file and that file must not be read on the ordinary path: the
/// closure runs only after the address check has already failed. A caller with no
/// allowlist passes `Vec::new`.
///
/// Two residuals, both deliberate and neither closed here:
///
/// 1. **DNS rebinding.** The name is resolved once for this check and again by the
///    connector, so a record with a one-second TTL can answer public here and private
///    there. Closing it needs the checked address to be the address the socket gets —
///    a pinned `ClientBuilder::resolve` or a custom connector — which is a larger
///    change than this guard, and one no unit test in this file could observe.
/// 2. **A blocking resolve on a redirect hop.** `to_socket_addrs` is synchronous, and
///    comms' redirect policy calls this from inside reqwest's own runtime thread, so a
///    slow resolver on a hop can push a request past the timeout set beside that
///    policy. One client per fetch bounds the damage to that fetch.
pub fn check_destination(
    url: &str,
    allowed_origins: impl FnOnce() -> Vec<String>,
) -> Result<(), Refused> {
    let parsed = reqwest::Url::parse(url.trim())
        .map_err(|e| Refused(format!("refused: unparsable URL ({e})")))?;
    let host = parsed
        .host_str()
        .ok_or_else(|| Refused("refused: URL names no host".into()))?;
    // `host_str` keeps the brackets on an IPv6 literal; `to_socket_addrs` parses an
    // address literal before it resolves, and it cannot parse the brackets.
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let port = parsed.port_or_known_default().unwrap_or(80);
    let addrs: Vec<_> = (host, port)
        .to_socket_addrs()
        .map_err(|e| Refused(format!("refused: cannot resolve {host} ({e})")))?
        .collect();
    if addrs.is_empty() {
        return Err(Refused(format!("refused: {host} resolves to no address")));
    }
    // ALL, not ANY. A name that answers with one public address and one private
    // address is the ordinary way this check is bypassed.
    if addrs.iter().all(|a| is_public(a.ip())) {
        return Ok(());
    }
    if origin_is_allowed(&parsed, &allowed_origins()) {
        return Ok(());
    }
    Err(Refused(format!(
        "refused: {host} resolves to a non-public address"
    )))
}

/// `http://127.0.0.1:8099/articles/x` -> `http://127.0.0.1:8099`. `None` for anything
/// that is not an absolute http(s) URL naming a host.
///
/// The port is always written out, so `http://example.com` and
/// `http://example.com:80` normalise to one string and cannot be configured apart.
/// One normaliser, because the configured entries and the URL being checked have to
/// be compared as the same shape or the comparison is a coin toss:
/// [`origin_is_allowed`] calls this on the URL, and the caller assembling its
/// allowlist calls it on every entry.
pub fn normalize_origin(raw: &str) -> Option<String> {
    let url = reqwest::Url::parse(raw.trim()).ok()?;
    let scheme = url.scheme();
    if scheme != "http" && scheme != "https" {
        return None;
    }
    // `host_str` keeps the brackets on an IPv6 literal. They stay: both sides of the
    // comparison come through here, and a bracketed host is what the operator writes
    // in the config file too.
    let host = url.host_str()?.to_ascii_lowercase();
    let port = url.port_or_known_default()?;
    Some(format!("{scheme}://{host}:{port}"))
}

/// Whether a URL's own origin — the scheme, host and port as written, never the
/// address it resolved to — is one the operator listed.
///
/// Matching the written host is the point. An attacker who publishes a name that
/// resolves to 127.0.0.1 still does not match `http://127.0.0.1:8099`, so the entry
/// clears exactly the origin it names and nothing that merely lands in the same place.
pub fn origin_is_allowed(url: &reqwest::Url, allowed: &[String]) -> bool {
    match normalize_origin(url.as_str()) {
        Some(origin) => allowed.contains(&origin),
        None => false,
    }
}

/// Whether an address is routable on the public internet. Stricter than "not
/// loopback" on purpose: link-local carries the cloud metadata service at
/// 169.254.169.254, and the CGNAT range carries this machine's VPN peers.
pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            // Four ranges written out by hand. Three of them have a std predicate
            // that is still behind feature `ip` on stable (rust-toolchain.toml) --
            // `is_shared`, `is_reserved`, `is_benchmarking` -- and 0.0.0.0/8 has none
            // at all. Every other test below is stable std, so do not hand-roll those.
            let o = v4.octets();
            // RFC 6598 carrier-grade NAT, 100.64.0.0/10: this machine's VPN peers.
            let cgnat = o[0] == 100 && (64..128).contains(&o[1]);
            // RFC 1122 "this network", 0.0.0.0/8. Some stacks route 0.x.y.z to
            // localhost, and no destination on it is legitimate.
            let this_network = o[0] == 0;
            // RFC 1112 reserved, 240.0.0.0/4, and RFC 2544 benchmarking,
            // 198.18.0.0/15. Neither is routable, so neither is a public host.
            let reserved = o[0] >= 240;
            let benchmarking = o[0] == 198 && (18..20).contains(&o[1]);
            !(v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_multicast()
                || v4.is_unspecified()
                || cgnat
                || this_network
                || reserved
                || benchmarking)
        }
        IpAddr::V6(v6) => {
            // The v6 tests run FIRST. `::1` is an IPv4-compatible address as well as
            // the loopback one, and unwrapping it before testing it yields 0.0.0.1 --
            // which the v4 arm would have to special-case to avoid calling loopback
            // public.
            if v6.is_loopback()
                || v6.is_multicast()
                || v6.is_unspecified()
                || v6.is_unique_local()
                || v6.is_unicast_link_local()
            {
                return false;
            }
            // `::ffff:a.b.c.d` (mapped) and the deprecated `::a.b.c.d` (compatible)
            // are both an IPv4 destination wearing a v6 name; `::ffff:127.0.0.1` has
            // to fail for the reason 127.0.0.1 fails, and so does `::7f00:1`.
            match v6.to_ipv4() {
                Some(v4) => is_public(IpAddr::V4(v4)),
                None => true,
            }
        }
    }
}

/// Does every address this URL resolves to sit on the public internet?
///
/// `false` for a name that cannot be resolved at all, because "unknown" and "inside"
/// have to be treated the same by anything that then decides whether to fetch it.
/// ALL, not ANY, for the reason [`check_destination`] gives: a name that answers with
/// one public address and one private address is the ordinary bypass.
fn resolves_public(url: &reqwest::Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let port = url.port_or_known_default().unwrap_or(80);
    match (host, port).to_socket_addrs() {
        Ok(addrs) => {
            let addrs: Vec<_> = addrs.collect();
            !addrs.is_empty() && addrs.iter().all(|a| is_public(a.ip()))
        }
        Err(_) => false,
    }
}

/// How many hops any client from this crate will follow. reqwest's own default.
pub const MAX_REDIRECTS: usize = 10;

/// The redirect policy every client this crate builds carries.
///
/// ## Why a default and not an option
///
/// [`check_destination`] checks the URL the caller handed over, and the caller is the
/// one place a redirect is invisible. `capabilities/comms/src/media/http.rs` wrote this
/// out first — "checking only the URL the caller handed over leaves
/// `302 -> http://169.254.169.254/` as a complete bypass" — and then hand-rolled the
/// policy for itself, which left it as one capability's habit rather than the crate's
/// behaviour. Measured 2026-09-08: thirty-four call sites in this workspace build a
/// client through this crate and exactly one of them — comms' — set a redirect policy.
/// The other thirty-three ran reqwest's default, which follows ten hops and checks
/// nothing, so the *remote server* chose the final destination on every scouting feed
/// fetch, every Nominatim and Open-Meteo call, every Hugging Face dataset download and
/// every price provider.
///
/// ## The rule, and why it is not "refuse every private hop"
///
/// A hop is refused when it leaves the public internet for this machine or this
/// network. A chain that started inside stays free to move inside: eight call sites in
/// this workspace point a client at `http://127.0.0.1:<port>` on purpose, because that
/// is how one capability calls another (`capabilities/trips/src/finance_client.rs`,
/// `interior_client.rs`, `places/src/backfill.rs`, calendar's trips client), and a
/// policy that refused a private destination outright would break them the first time
/// axum answered a 307. The pivot is the attack; a loopback caller reaching loopback is
/// the architecture.
///
/// It also means no configuration. comms' `check_destination` needs an allowlist
/// because `tools/demo-up` legitimately points *ingest* at loopback; this needs none,
/// because a chain that begins at that same loopback origin is already inside.
///
/// ## What it does not close
///
/// The two residuals [`check_destination`] records are unchanged and are the same two.
/// DNS rebinding: the name is resolved here and again by the connector, so a one-second
/// TTL can answer public here and private there. And this resolve is synchronous inside
/// reqwest's own runtime thread, so a slow resolver on a hop can push a request past
/// its timeout. Neither is made worse by running the check; both are the price of
/// checking a name rather than pinning an address.
///
/// The second one reaches further here than it did at comms' one-client-per-fetch call
/// site, and the difference is worth writing down rather than inheriting. [`super::client`]
/// pools a client per `(purpose, timeout)`, and `reqwest::blocking::Client` drives every
/// request over one background current-thread runtime (reqwest-0.13.4
/// `src/blocking/client.rs`, `new_current_thread` plus `tokio::spawn` per request). So a
/// slow resolve in this callback holds up any other request in flight on the same pooled
/// client, not only the one being redirected. It is still the right trade against
/// following an unchecked hop, and it is the reason not to add a second resolve here.
pub fn redirect_policy() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
        match check_redirect(attempt.previous(), attempt.url()) {
            Ok(()) => attempt.follow(),
            Err(refusal) => attempt.error(refusal),
        }
    })
}

/// The decision [`redirect_policy`] makes, as a function of the chain so far and the
/// next URL. Separated from the policy so it can be driven with address literals: a
/// test of the rule must not need the internet to supply a public host, and
/// `to_socket_addrs` parses a literal without touching a resolver.
pub fn check_redirect(previous: &[reqwest::Url], next: &reqwest::Url) -> Result<(), Refused> {
    // `>`, not `>=`: `previous` starts with the initial URL, which is not a redirection
    // (reqwest-0.13.4 `src/redirect.rs`, whose own `Policy::limited` compares the same
    // way). comms found this the hard way — with `>=` the error said ten and followed
    // nine.
    if previous.len() > MAX_REDIRECTS {
        return Err(Refused(format!(
            "refused: more than {MAX_REDIRECTS} redirects"
        )));
    }
    if check_scheme(next.as_str()).is_err() {
        return Err(Refused(format!(
            "refused: redirect to a non-http(s) URL ({})",
            next.scheme()
        )));
    }
    // The cheap order. A public destination is the overwhelmingly common case and costs
    // one resolution; only a private one pays for a second.
    if resolves_public(next) {
        return Ok(());
    }
    match previous.first() {
        // The chain began inside already: one capability calling another. Staying
        // inside is the architecture, not a pivot.
        Some(initial) if !resolves_public(initial) => Ok(()),
        _ => Err(Refused(format!(
            "refused: redirect to {} leaves the public internet for this network",
            next.host_str().unwrap_or("an unnamed host")
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_allowlist() -> Vec<String> {
        Vec::new()
    }

    #[test]
    fn check_scheme_rejects_non_http() {
        assert!(check_scheme("https://example.com").is_ok());
        assert!(check_scheme("http://example.com").is_ok());
        assert!(check_scheme("file:///etc/passwd").is_err());
        assert!(check_scheme("ftp://example.com/x").is_err());
        assert!(check_scheme("example.com").is_err());
    }

    /// The scheme test is case- and whitespace-insensitive, because a caller passes
    /// the string a remote feed handed it.
    #[test]
    fn check_scheme_reads_the_url_as_written_not_as_typed() {
        assert!(check_scheme("  HTTPS://example.com  ").is_ok());
        assert!(check_scheme("HtTp://example.com").is_ok());
        assert!(check_scheme("FILE:///etc/passwd").is_err());
    }

    /// Every case is an IP literal, so the test asserts the address policy and never
    /// touches a resolver. comms' redirect policy calls this same function on each
    /// hop, but `reqwest::redirect::Attempt` cannot be constructed outside reqwest,
    /// so that wiring is only exercised by an end-to-end ingest of a public URL that
    /// redirects to a private one.
    #[test]
    fn check_destination_refuses_non_public_addresses() {
        for url in [
            "http://127.0.0.1:8086/api/plans",
            "http://[::1]/x",
            "http://[::ffff:127.0.0.1]/x",
            "http://10.0.0.1/",
            "http://192.168.1.1/",
            "http://172.16.0.1/",
            "http://169.254.169.254/latest/meta-data/",
            "http://[fe80::1]/",
            "http://[fd00::1]/",
            "http://100.64.0.1/",
            "http://0.0.0.0/",
            "http://255.255.255.255/",
            "http://[ff02::1]/",
            // The deprecated IPv4-compatible spelling of 127.0.0.1. Modern stacks do
            // not route it, but the guard must not be the thing that depends on that.
            "http://[::7f00:1]/",
            "http://240.0.0.1/",
            "http://198.18.0.1/",
            "http://0.1.2.3/",
        ] {
            let err = check_destination(url, no_allowlist).expect_err(url);
            assert!(err.to_string().starts_with("refused: "), "{url}: {err}");
        }
    }

    #[test]
    fn check_destination_allows_a_public_address() {
        assert!(check_destination("http://93.184.216.34/index.html", no_allowlist).is_ok());
        assert!(check_destination("https://[2606:2800:220:1::1]/", no_allowlist).is_ok());
    }

    #[test]
    fn check_destination_refuses_a_url_without_a_host() {
        assert!(check_destination("http:///nowhere", no_allowlist).is_err());
    }

    #[test]
    fn check_destination_refuses_what_is_not_a_url_at_all() {
        let err = check_destination("not a url", no_allowlist).expect_err("unparsable");
        assert!(
            err.to_string().starts_with("refused: unparsable URL"),
            "{err}"
        );
    }

    /// A listed origin clears the address check, and it is the only thing that does.
    #[test]
    fn check_destination_lets_a_listed_loopback_origin_through() {
        let listed = || vec!["http://127.0.0.1:8099".to_string()];
        assert!(check_destination("http://127.0.0.1:8099/articles/x", listed).is_ok());
        // The neighbouring port on the same host is a different Axon service.
        assert!(check_destination("http://127.0.0.1:8086/api/plans", listed).is_err());
    }

    /// The allowlist closure is the reason comms does not read its config file on
    /// every fetch. A destination that passes the address check must not run it.
    #[test]
    fn the_allowlist_is_only_consulted_after_the_address_check_fails() {
        let mut consulted = false;
        let watcher = || {
            consulted = true;
            Vec::new()
        };
        assert!(check_destination("http://93.184.216.34/", watcher).is_ok());
        assert!(!consulted, "a public address must not read the allowlist");

        let mut consulted = false;
        let watcher = || {
            consulted = true;
            Vec::new()
        };
        assert!(check_destination("http://127.0.0.1:8086/", watcher).is_err());
        assert!(consulted, "a private address must read the allowlist");
    }

    #[test]
    fn is_public_maps_the_boundaries_of_the_hand_written_ranges() {
        // Every range below is written out because its std predicate is behind
        // feature `ip`. The neighbours on each side prove the mask, not just the
        // middle.
        // 100.64.0.0/10, carrier-grade NAT.
        assert!(!is_public("100.64.0.0".parse().unwrap()));
        assert!(!is_public("100.127.255.255".parse().unwrap()));
        assert!(is_public("100.63.255.255".parse().unwrap()));
        assert!(is_public("100.128.0.0".parse().unwrap()));
        // 0.0.0.0/8, "this network".
        assert!(!is_public("0.255.255.255".parse().unwrap()));
        assert!(is_public("1.0.0.0".parse().unwrap()));
        // 198.18.0.0/15, benchmarking. 198.20.0.0 is an ordinary public host.
        assert!(!is_public("198.18.0.0".parse().unwrap()));
        assert!(!is_public("198.19.255.255".parse().unwrap()));
        assert!(is_public("198.17.255.255".parse().unwrap()));
        assert!(is_public("198.20.0.0".parse().unwrap()));
        // 240.0.0.0/4, reserved. Its lower neighbour is inside multicast
        // (224.0.0.0/4), so the last public IPv4 address is 223.255.255.255.
        assert!(!is_public("240.0.0.0".parse().unwrap()));
        assert!(!is_public("239.255.255.254".parse().unwrap()));
        assert!(is_public("223.255.255.255".parse().unwrap()));
    }

    /// `::1` is IPv4-compatible as well as loopback, so it is the case that decides
    /// the order of the two tests in the v6 arm: unwrapped first it becomes 0.0.0.1,
    /// which no v4 predicate calls loopback.
    #[test]
    fn is_public_unwraps_both_v4_in_v6_forms_without_laundering_v6_loopback() {
        assert!(!is_public("::1".parse().unwrap()));
        assert!(!is_public("::ffff:127.0.0.1".parse().unwrap()));
        assert!(!is_public("::7f00:1".parse().unwrap()));
        assert!(!is_public("::ffff:192.168.1.1".parse().unwrap()));
        assert!(is_public("::ffff:93.184.216.34".parse().unwrap()));
        assert!(is_public("2606:2800:220:1::1".parse().unwrap()));
    }

    /// The allowlist is matched against the URL as written, so it clears the origin
    /// the operator named and nothing else that happens to resolve to the same
    /// machine. `tools/demo-up` is the one caller that sets it.
    #[test]
    fn origin_is_allowed_matches_the_written_origin_only() {
        let allowed = vec!["http://127.0.0.1:8099".to_string()];
        let url = |u: &str| reqwest::Url::parse(u).unwrap();

        assert!(origin_is_allowed(
            &url("http://127.0.0.1:8099/articles/a-slug"),
            &allowed
        ));
        // A different port on the same host is a different Axon service. This is the
        // whole reason the entry is an origin and not a host.
        assert!(!origin_is_allowed(
            &url("http://127.0.0.1:8086/api/plans"),
            &allowed
        ));
        // A name that resolves to loopback is still not the listed origin.
        assert!(!origin_is_allowed(
            &url("http://localhost:8099/articles/a-slug"),
            &allowed
        ));
        assert!(!origin_is_allowed(
            &url("https://127.0.0.1:8099/x"),
            &allowed
        ));
        // Nothing is allowed by default, which is what every non-demo machine runs
        // with.
        assert!(!origin_is_allowed(&url("http://127.0.0.1:8099/x"), &[]));
    }

    #[test]
    fn normalize_origin_writes_the_port_out_and_drops_the_path() {
        assert_eq!(
            normalize_origin("http://127.0.0.1:8099/articles/x").as_deref(),
            Some("http://127.0.0.1:8099")
        );
        // The default port is written out, so the two spellings of one origin are one
        // string on both sides of the comparison.
        assert_eq!(
            normalize_origin("http://example.com").as_deref(),
            Some("http://example.com:80")
        );
        assert_eq!(
            normalize_origin("https://Example.COM/").as_deref(),
            Some("https://example.com:443")
        );
        assert_eq!(
            normalize_origin("http://[::1]:9000/").as_deref(),
            Some("http://[::1]:9000")
        );
    }

    #[test]
    fn normalize_origin_refuses_what_is_not_an_http_origin() {
        // A scheme that is not http(s) must not be configurable as an escape from a
        // guard whose other half exists to refuse `file://`.
        assert_eq!(normalize_origin("file:///etc/passwd"), None);
        assert_eq!(normalize_origin("ftp://example.com/"), None);
        // Relative, and host-less: neither names an origin.
        assert_eq!(normalize_origin("127.0.0.1:8099"), None);
        assert_eq!(normalize_origin(""), None);
    }

    fn url(raw: &str) -> reqwest::Url {
        reqwest::Url::parse(raw).expect("a test URL parses")
    }

    /// Address literals, never names. `to_socket_addrs` parses a literal before it
    /// resolves anything, so this whole set runs with no resolver and no network — and
    /// a test of an SSRF rule that needed the internet to supply a public host would be
    /// a test nobody could run offline.
    ///
    /// 93.184.216.34 is the documentation address for example.com (RFC 2606's name,
    /// IANA's address); nothing here connects to it.
    const PUBLIC: &str = "http://93.184.216.34/feed.xml";

    /// The attack the crate-wide policy exists for: an operator-configured feed URL on
    /// a public host answers 302 and names something inside this machine or this
    /// network. `check_destination` cannot see it, because the caller never wrote it.
    #[test]
    fn a_redirect_off_the_public_internet_into_this_network_is_refused() {
        for inside in [
            // Loopback, where every Axon capability binds.
            "http://127.0.0.1:8082/api/sjel-status/capabilities",
            "http://[::1]:8090/api/dashboard",
            // The cloud metadata service, which is why `is_public` refuses link-local.
            "http://169.254.169.254/latest/meta-data/",
            // A LAN host and a tailnet peer.
            "http://192.168.1.1/",
            "http://100.100.100.100/",
        ] {
            let refusal = check_redirect(&[url(PUBLIC)], &url(inside))
                .expect_err("a pivot inward must be refused");
            assert!(
                refusal.to_string().contains("leaves the public internet"),
                "{inside}: {refusal}"
            );
        }
    }

    /// The control, and the reason the rule is a pivot rather than a ban. Eight call
    /// sites in this workspace point a client at loopback on purpose, because that is
    /// how one capability calls another; a chain that began inside may stay inside.
    #[test]
    fn a_chain_that_began_inside_may_stay_inside() {
        assert!(check_redirect(
            &[url("http://127.0.0.1:8086/api/plans")],
            &url("http://127.0.0.1:8090/api/dashboard"),
        )
        .is_ok());
    }

    /// The other control: an ordinary public redirect, which is most redirects.
    #[test]
    fn a_public_redirect_is_followed() {
        assert!(check_redirect(&[url(PUBLIC)], &url("http://93.184.216.34/moved")).is_ok());
    }

    /// The two limits that are not about addresses at all.
    #[test]
    fn the_chain_is_bounded_and_stays_on_http() {
        let chain: Vec<_> = std::iter::repeat_n(url(PUBLIC), MAX_REDIRECTS + 1).collect();
        let refusal = check_redirect(&chain, &url("http://93.184.216.34/again"))
            .expect_err("the eleventh hop must be refused");
        assert!(refusal.to_string().contains("more than 10 redirects"));
        // One below the limit still follows, so the boundary is asserted from both
        // sides rather than assumed.
        let chain: Vec<_> = std::iter::repeat_n(url(PUBLIC), MAX_REDIRECTS).collect();
        assert!(check_redirect(&chain, &url("http://93.184.216.34/again")).is_ok());

        let refusal = check_redirect(&[url(PUBLIC)], &url("file:///etc/passwd"))
            .expect_err("a scheme change must be refused");
        assert!(refusal.to_string().contains("non-http(s)"), "{refusal}");
    }
}

/// The policy driven through a real client, because a rule that is correct and not
/// installed is the failure this whole crate exists to prevent.
///
/// Everything here is loopback, so it runs with no network. That bounds what it can
/// prove: the refusal it exercises is the hop limit, not the pivot — reaching the
/// pivot end to end would need a public host that answers 302, which is exactly the
/// dependency the unit tests above avoid. What it does prove is that
/// [`super::builder`] installs THIS policy rather than reqwest's default, which is the
/// half a unit test of `check_redirect` cannot see.
#[cfg(test)]
mod wired_tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// `Connection: close` on every reply, matching the stub
    /// `capabilities/places/src/geocode.rs` already uses. Without it hyper keeps the
    /// connection alive, this one-request-per-socket loop drops it, and the client
    /// reports `IncompleteMessage` for a response that was in fact complete — measured
    /// while writing these two tests.
    fn found(location: &str) -> String {
        format!(
            "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )
    }

    fn ok(body: &str) -> String {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    /// Bind first, so the replies can name the port; then answer from `replies` in
    /// order, repeating the last one. Returns the base URL.
    fn serve(replies: impl Fn(&str) -> Vec<String>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port");
        let base = format!("http://{}", listener.local_addr().expect("an address"));
        let scripted = replies(&base);
        std::thread::spawn(move || {
            for (index, stream) in listener.incoming().enumerate() {
                let Ok(mut stream) = stream else { break };
                let mut buffer = [0_u8; 4096];
                let _ = stream.read(&mut buffer);
                let reply = scripted
                    .get(index)
                    .or_else(|| scripted.last())
                    .cloned()
                    .unwrap_or_default();
                let _ = stream.write_all(reply.as_bytes());
                let _ = stream.flush();
            }
        });
        base
    }

    /// One capability redirecting to another over loopback still works. This is the
    /// case a policy written as "refuse every private destination" would have broken,
    /// and nothing in this workspace would have failed until a capability answered its
    /// first 307.
    #[test]
    fn a_loopback_redirect_is_followed() {
        let base = serve(|base| vec![found(&format!("{base}/second")), ok("arrived")]);
        let client = crate::client(
            crate::Purpose::new("test-redirect"),
            std::time::Duration::from_secs(5),
        )
        .expect("client builds");
        let body = client
            .get(format!("{base}/first"))
            .send()
            .expect("the redirect is followed")
            .text()
            .expect("a body");
        assert_eq!(body, "arrived");
    }

    /// The refusal, end to end, and the proof that `builder` installs THIS policy:
    /// reqwest's own default answers "too many redirects", and this one names the
    /// number it enforces. Swap `redirect_policy()` out of `builder` and this assertion
    /// is the thing that fails.
    #[test]
    fn a_redirect_loop_is_refused_with_this_crates_message() {
        let base = serve(|base| vec![found(&format!("{base}/again"))]);
        let client = crate::client(
            crate::Purpose::new("test-redirect-loop"),
            std::time::Duration::from_secs(5),
        )
        .expect("client builds");
        let error = client
            .get(format!("{base}/start"))
            .send()
            .expect_err("an endless redirect must fail");
        let printed = format!("{error:?}");
        assert!(
            printed.contains("more than 10 redirects"),
            "the crate's own policy is not installed: {printed}"
        );
    }
}
