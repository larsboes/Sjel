//! The Linux branch: `ss -tulpn` for listeners, and nothing for the packet filter.
//!
//! # Unverified, and it says so
//!
//! No Linux host was reachable while this was written, so this parser is built from `ss`'s
//! documented output and not from a capture of a running machine. The README repeats that. The
//! two things it will not do are return an empty list when `ss` is missing, and claim a
//! firewall is clean when it could not read one — both of which read as "nothing is exposed"
//! (Packs/harness/skills/sjel/references/shared-failure-policy.md).
//!
//! `nft list ruleset` needs root. host-net never uses sudo, so `firewall` on Linux reports the
//! layer as unavailable and exits rather than printing a half-answer.

use std::collections::HashMap;

use crate::listen::{assemble, classify, IfAddr, Listener, Process, RawSocket};
use crate::sys::capture;

/// Parse `ss -tulpn`.
///
/// The columns are `Netid State Recv-Q Send-Q Local Peer [Process]`, and the process column is
/// EMPTY for a socket owned by another user when `ss` runs unprivileged. That row is still
/// printed, and it is still exposure, so it is kept with an unknown pid rather than dropped.
pub fn parse_ss(text: &str) -> Vec<(RawSocket, Option<String>)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 6 || !(f[0].starts_with("tcp") || f[0].starts_with("udp")) {
            continue;
        }
        let (address, port) = split_ss_addr(f[4]);
        // ss spells an unbound peer `0.0.0.0:*`, `[::]:*` or `*:*`; netstat spells it `*.*`.
        // Normalising here means one `is_listener` rule covers both platforms.
        let (_, peer_port) = split_ss_addr(f[5]);
        let foreign = if peer_port == "*" {
            "*.*".to_string()
        } else {
            f[5].to_string()
        };
        let (name, pid) = f.get(6).map_or((None, 0), |p| parse_ss_process(p));
        out.push((
            RawSocket {
                proto: f[0].to_string(),
                local_address: address,
                local_port: port,
                foreign,
                state: Some(f[1].to_string()),
                comm16: name.clone().unwrap_or_else(|| "(unknown)".into()),
                pid,
            },
            name,
        ));
    }
    out
}

/// `[::]:22` → `("::", "22")`, `0.0.0.0:5353` → `("0.0.0.0", "5353")`, `*:111` → `("*", "111")`.
fn split_ss_addr(field: &str) -> (String, String) {
    let (host, port) = field.rsplit_once(':').unwrap_or((field, ""));
    let host = host.trim_start_matches('[').trim_end_matches(']');
    (host.to_string(), port.to_string())
}

/// `users:(("sshd",pid=812,fd=3))` → `(Some("sshd"), 812)`.
fn parse_ss_process(field: &str) -> (Option<String>, u32) {
    let name = field
        .split_once("((\"")
        .and_then(|(_, rest)| rest.split_once('"'))
        .map(|(n, _)| n.to_string());
    let pid = field
        .split_once("pid=")
        .and_then(|(_, rest)| {
            let end = rest
                .find(|c: char| !c.is_ascii_digit())
                .unwrap_or(rest.len());
            rest[..end].parse::<u32>().ok()
        })
        .unwrap_or(0);
    (name, pid)
}

/// Read this host's listeners on Linux.
pub fn collect() -> Result<Vec<Listener>, String> {
    let text = capture("ss", &["-tulpn"])?;
    let rows = parse_ss(&text);
    // ss already names the process, so there is no truncation to repair and no `ps` join. The
    // interface table still comes from `ip addr`, so a tailnet or LAN bind classifies the same
    // way it does on macOS.
    let interfaces =
        capture("ip", &["-o", "addr"]).map_or_else(|_| Vec::new(), |t| parse_ip_addr(&t));
    let procs: Vec<Process> = rows
        .iter()
        .filter_map(|(s, name)| {
            name.as_ref().map(|n| Process {
                pid: s.pid,
                uid: 0,
                path: n.clone(),
            })
        })
        .collect();
    let sockets: Vec<RawSocket> = rows.into_iter().map(|(s, _)| s).collect();
    let mut listeners = assemble(&sockets, &procs, &HashMap::new(), &interfaces);
    // `assemble` classifies against the address it was given; ss's `*` and `[::]` forms are
    // already normalised by `split_ss_addr`, so re-running the classifier costs nothing and
    // keeps one implementation of the rule.
    for l in &mut listeners {
        l.scope = classify(&l.address, &interfaces);
        // ss does not report an owning uid, and `assemble` filled in the placeholder above.
        // Reporting every Linux listener as root-owned would be a fact this branch never read.
        l.uid = None;
    }
    Ok(listeners)
}

/// Parse `ip -o addr`: `2: eth0    inet 192.0.2.10/24 brd ... scope global eth0`.
pub fn parse_ip_addr(text: &str) -> Vec<IfAddr> {
    text.lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split_whitespace().collect();
            if f.len() < 4 || (f[2] != "inet" && f[2] != "inet6") {
                return None;
            }
            Some(IfAddr {
                interface: f[1].trim_end_matches(':').to_string(),
                address: f[3].split('/').next().unwrap_or(f[3]).to_string(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::listen::{is_listener, Scope};

    const SS: &str = include_str!("../fixtures/ss.txt");

    #[test]
    fn ss_rows_parse_into_sockets() {
        let rows = parse_ss(SS);
        let sockets: Vec<&RawSocket> = rows.iter().map(|(s, _)| s).collect();
        let sshd = sockets.iter().find(|s| s.local_port == "22").unwrap();
        assert_eq!(sshd.local_address, "0.0.0.0");
        assert_eq!(sshd.comm16, "sshd");
        assert_eq!(sshd.pid, 812);
        let v6 = sockets.iter().find(|s| s.proto == "tcp6").unwrap();
        assert_eq!(v6.local_address, "::");
    }

    /// A socket ss could name but not attribute is still exposure. Dropping it is the same
    /// mistake the macOS branch avoids by not using lsof.
    #[test]
    fn a_row_with_no_process_column_is_kept() {
        let rows = parse_ss(SS);
        let unattributed = rows
            .iter()
            .find(|(s, _)| s.local_port == "5432")
            .expect("the unattributed fixture row");
        assert_eq!(unattributed.1, None);
        assert_eq!(unattributed.0.pid, 0);
    }

    #[test]
    fn ss_scopes_classify_like_the_macos_branch() {
        let ifaces =
            parse_ip_addr("2: eth0    inet 192.0.2.10/24 brd 192.0.2.255 scope global eth0\n");
        assert_eq!(classify("0.0.0.0", &ifaces), Scope::Wildcard);
        assert_eq!(classify("::", &ifaces), Scope::Wildcard);
        assert_eq!(classify("127.0.0.1", &ifaces), Scope::Loopback);
        assert_eq!(classify("192.0.2.10", &ifaces), Scope::Lan);
    }

    /// UNCONN is how ss spells a bound UDP socket; it is not a TCP LISTEN and the shared filter
    /// must not silently drop it.
    #[test]
    fn udp_unconn_rows_are_listeners() {
        let rows = parse_ss(SS);
        let udp = rows
            .iter()
            .map(|(s, _)| s)
            .find(|s| s.proto == "udp")
            .unwrap();
        assert!(is_listener(udp));
    }
}
