//! Which sockets this host is listening on, and how far each one reaches.
//!
//! # Why `netstat -anv` and not `lsof`
//!
//! `lsof` is the shorter command and the obvious one, and without root it silently shows only
//! the invoking user's sockets. Measured on the build host 2026-09-02:
//! `lsof -nP -iTCP -sTCP:LISTEN` returned 27 rows, every one of them owned by the invoking
//! user; `netstat -anv -p tcp` returned 29 LISTEN rows across every uid, the two extra being
//! `launchd` (pid 1, uid 0) on `127.0.0.1:8021` and `[::1]:8021`. An earlier mapping pass on
//! the same machine, run with `lsof`, missed a root-owned wildcard `*:443` bind belonging to a
//! VPN system extension for the same reason.
//!
//! A listener the tool cannot see is worse than no tool, because the output still reads as a
//! complete answer. So: netstat, whose cost is a sixteen-character truncation of the process
//! name, repaired by joining on pid against `ps`, which reports the full executable path for
//! root-owned pids without sudo.

use serde::Serialize;
use std::collections::HashMap;

use crate::sys::capture;

/// How far a bind reaches. Four values, because those are the four answers that change what an
/// operator does about a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    /// `127.0.0.0/8` or `::1` — reachable only from this host.
    Loopback,
    /// An address on a `utun*` interface, or inside `100.64.0.0/10`.
    Tailnet,
    /// A specific address on some other local interface.
    Lan,
    /// `*`, `0.0.0.0` or `::` — every interface this host has now or grows later.
    Wildcard,
}

impl Scope {
    pub fn as_str(self) -> &'static str {
        match self {
            Scope::Loopback => "loopback",
            Scope::Tailnet => "tailnet",
            Scope::Lan => "lan",
            Scope::Wildcard => "wildcard",
        }
    }

    /// Sort order for the table: widest reach first, because that is the half of the output an
    /// operator is reading the table to find.
    fn rank(self) -> u8 {
        match self {
            Scope::Wildcard => 0,
            Scope::Lan => 1,
            Scope::Tailnet => 2,
            Scope::Loopback => 3,
        }
    }
}

/// One listening socket, after the pid join has repaired the process name.
#[derive(Debug, Clone, Serialize)]
pub struct Listener {
    pub proto: String,
    pub address: String,
    pub port: String,
    pub scope: Scope,
    pub pid: u32,
    /// The executable's basename, from `ps` when the pid resolved there and from netstat's
    /// truncated field when it did not.
    pub process: String,
    /// The full executable path, when `ps` had one.
    pub path: Option<String>,
    pub uid: Option<u32>,
    /// The launchd label that owns this pid, when the user domain knows it.
    pub launchd: Option<String>,
}

/// A row of `netstat -anv`, before anything is joined onto it.
#[derive(Debug, Clone, PartialEq)]
pub struct RawSocket {
    pub proto: String,
    pub local_address: String,
    pub local_port: String,
    pub foreign: String,
    pub state: Option<String>,
    /// netstat's process field, truncated to sixteen characters and possibly holding spaces.
    pub comm16: String,
    pub pid: u32,
}

/// A process, from `ps -Ao pid=,uid=,comm=`.
#[derive(Debug, Clone, PartialEq)]
pub struct Process {
    pub pid: u32,
    pub uid: u32,
    pub path: String,
}

/// One interface address, from `ifconfig -a`.
#[derive(Debug, Clone, PartialEq)]
pub struct IfAddr {
    pub interface: String,
    pub address: String,
}

/// Parse `netstat -anv -p tcp` or `-p udp`.
///
/// # Read the row from the right
///
/// The process field is the one field that contains spaces, and macOS pads or truncates it to
/// sixteen characters — which can put a space immediately BEFORE the colon. Measured verbatim
/// on the build host: `OrbStack Helper:74138`, `Google Drive:98885` and, the one that breaks
/// every naive pattern, `Obsidian Helper :80286`. A `(\S+):(\d+)` match yields `Helper` and
/// loses the rest of the name; a pattern that requires a non-space before the colon fails on
/// the third shape outright. Six of twenty-nine LISTEN rows hit one of these today.
///
/// Counting fields from the left does not work either, because the `(state)` column is present
/// on TCP rows and empty on UDP rows, so rows in the same table have different widths.
///
/// What IS stable is the tail. The header ends `process:pid state options gencnt flags flags1
/// usecnt rtncnt fltrs`: exactly eight fields after the process field, none of which can
/// contain a space. So the parser anchors on the right — drop eight, then walk left until the
/// first field that is a bare integer, which is `shiwat`. Everything between is the process
/// field, spaces and all.
///
/// Known limit, stated because it is invisible otherwise: a process whose name STARTS with a
/// space-separated bare integer (`1024 Helper`) loses that first word, because the walk stops
/// at it. That costs nothing downstream — the authoritative name comes from `ps` by pid, and
/// this field is only the fallback when the pid has already exited.
pub fn parse_netstat(text: &str) -> Vec<RawSocket> {
    let mut out = Vec::new();
    for line in text.lines() {
        if !(line.starts_with("tcp") || line.starts_with("udp")) {
            continue;
        }
        let f: Vec<&str> = line.split_whitespace().collect();
        // proto, recv-q, send-q, local, foreign, [state], rxbytes, txbytes, rhiwat, shiwat,
        // process:pid, then the eight-field tail.
        if f.len() < 12 {
            continue;
        }
        let proc_end = f.len() - 8;
        let Some(shiwat) = (0..proc_end)
            .rev()
            .find(|&i| !f[i].is_empty() && f[i].bytes().all(|b| b.is_ascii_digit()))
        else {
            continue;
        };
        let proc_start = shiwat + 1;
        if proc_start >= proc_end {
            continue;
        }
        let field = f[proc_start..proc_end].join(" ");
        let Some((name, pid_text)) = field.rsplit_once(':') else {
            continue;
        };
        let Ok(pid) = pid_text.parse::<u32>() else {
            continue;
        };
        let (local_address, local_port) = split_host_port(f[3]);
        // Uppercase because every netstat state is (`LISTEN`, `ESTABLISHED`, `CLOSE_WAIT`), and
        // no address or byte count is. UDP rows leave the column empty and fall through here.
        let state = f
            .get(5)
            .filter(|s| s.bytes().all(|b| b.is_ascii_uppercase() || b == b'_'))
            .map(|s| s.to_string());
        out.push(RawSocket {
            proto: f[0].to_string(),
            local_address,
            local_port,
            foreign: f[4].to_string(),
            state,
            comm16: name.trim().to_string(),
            pid,
        });
    }
    out
}

/// `127.0.0.1.61495` → `("127.0.0.1", "61495")`, `*.*` → `("*", "*")`,
/// `2001:db8::1.443` → `("2001:db8::1", "443")`. netstat separates the port with the last dot,
/// which is what makes one rule cover IPv4, IPv6 and the wildcard forms.
fn split_host_port(field: &str) -> (String, String) {
    match field.rsplit_once('.') {
        Some((host, port)) => (host.to_string(), port.to_string()),
        None => (field.to_string(), String::new()),
    }
}

/// Which netstat rows are listeners.
///
/// TCP is the easy half: `state == LISTEN`.
///
/// UDP has no listen state, so the rule is the local port plus an unbound peer. Measured on the
/// build host: 41 UDP rows, of which 28 have a bare `*.*` local address — sending sockets with
/// no port of their own, not exposure — one is a connected socket with a real peer, and 12 are
/// wildcard binds on a real port. Counting the 28 would make the first `check` run fire on
/// half the daemons on the machine; dropping the 12 would hide mDNS and NetBIOS. Both errors
/// are one condition away from each other, so the filter is written out rather than implied.
pub fn is_listener(s: &RawSocket) -> bool {
    if s.proto.starts_with("tcp") {
        return s.state.as_deref() == Some("LISTEN");
    }
    if s.proto.starts_with("udp") {
        return s.foreign == "*.*"
            && s.local_port.bytes().all(|b| b.is_ascii_digit())
            && !s.local_port.is_empty();
    }
    false
}

/// Parse `ps -Ao pid=,uid=,comm=`. `comm` is last because it is the only field that can hold a
/// space — `/Library/Application Support/Interceptor/interceptor-daemon` is a live example on
/// the build host — so it takes the rest of the line.
pub fn parse_ps(text: &str) -> Vec<Process> {
    let mut out = Vec::new();
    for line in text.lines() {
        let mut it = line.split_whitespace();
        let (Some(pid), Some(uid)) = (it.next(), it.next()) else {
            continue;
        };
        let (Ok(pid), Ok(uid)) = (pid.parse::<u32>(), uid.parse::<u32>()) else {
            continue;
        };
        // Back to the raw line for the path, so its internal spacing survives.
        let rest = line.trim_start();
        let rest = rest[rest.find(char::is_whitespace).unwrap_or(rest.len())..].trim_start();
        let path = rest[rest.find(char::is_whitespace).unwrap_or(rest.len())..].trim();
        if path.is_empty() {
            continue;
        }
        out.push(Process {
            pid,
            uid,
            path: path.to_string(),
        });
    }
    out
}

/// Parse `launchctl list` into pid → label.
///
/// The user domain only. pids 1 and 22458 were both absent from it on the build host while
/// pid 622 was present, so a listener with no label here is normal and renders as an em dash —
/// never as the previous row's label and never as an error.
pub fn parse_launchctl(text: &str) -> HashMap<u32, String> {
    let mut out = HashMap::new();
    for line in text.lines() {
        let mut f = line.split('\t');
        let (Some(pid), Some(_status), Some(label)) = (f.next(), f.next(), f.next()) else {
            continue;
        };
        if let Ok(pid) = pid.trim().parse::<u32>() {
            out.insert(pid, label.trim().to_string());
        }
    }
    out
}

/// Parse the `inet`/`inet6` lines of `ifconfig -a`, keeping which interface carries each
/// address. The `%zone` suffix on a link-local address is dropped: it names the interface, and
/// the interface is already the other half of the pair.
pub fn parse_ifconfig(text: &str) -> Vec<IfAddr> {
    let mut out = Vec::new();
    let mut current = String::new();
    for line in text.lines() {
        if !line.starts_with(char::is_whitespace) {
            if let Some((name, _)) = line.split_once(':') {
                current = name.to_string();
            }
            continue;
        }
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 2 || (f[0] != "inet" && f[0] != "inet6") || current.is_empty() {
            continue;
        }
        let address = f[1].split('%').next().unwrap_or(f[1]).to_string();
        out.push(IfAddr {
            interface: current.clone(),
            address,
        });
    }
    out
}

/// Is this address inside `100.64.0.0/10`, the shared address space Tailscale assigns from?
fn is_cgnat(address: &str) -> bool {
    let mut parts = address.split('.');
    let (Some(a), Some(b)) = (parts.next(), parts.next()) else {
        return false;
    };
    matches!((a.parse::<u8>(), b.parse::<u8>()), (Ok(100), Ok(64..=127)))
}

/// Classify a bind address, resolved against this host's own interface list.
///
/// Two independent tailnet tests, because either can be true without the other: the address is
/// in `100.64.0.0/10`, or it sits on a `utun*` interface. Nothing here hardcodes an interface
/// name it found on one machine.
///
/// An address that matches no interface is reported `lan` rather than dropped. It is a specific
/// address on something, and the conservative direction for an exposure report is the scope
/// that still gets looked at.
pub fn classify(address: &str, interfaces: &[IfAddr]) -> Scope {
    if address == "*" || address == "0.0.0.0" || address == "::" {
        return Scope::Wildcard;
    }
    if address.starts_with("127.") || address == "::1" {
        return Scope::Loopback;
    }
    if is_cgnat(address) {
        return Scope::Tailnet;
    }
    match interfaces.iter().find(|i| i.address == address) {
        Some(i) if i.interface.starts_with("utun") => Scope::Tailnet,
        Some(i) if i.interface.starts_with("lo") => Scope::Loopback,
        _ => Scope::Lan,
    }
}

/// Join the four readings into the listener table.
pub fn assemble(
    sockets: &[RawSocket],
    procs: &[Process],
    launchd: &HashMap<u32, String>,
    interfaces: &[IfAddr],
) -> Vec<Listener> {
    let by_pid: HashMap<u32, &Process> = procs.iter().map(|p| (p.pid, p)).collect();
    let mut out: Vec<Listener> = sockets
        .iter()
        .filter(|s| is_listener(s))
        .map(|s| {
            let proc = by_pid.get(&s.pid);
            Listener {
                proto: s.proto.clone(),
                address: s.local_address.clone(),
                port: s.local_port.clone(),
                scope: classify(&s.local_address, interfaces),
                pid: s.pid,
                // The repair: netstat's field is sixteen characters, ps has the whole path.
                process: proc
                    .map(|p| basename(&p.path).to_string())
                    .unwrap_or_else(|| s.comm16.clone()),
                path: proc.map(|p| p.path.clone()),
                uid: proc.map(|p| p.uid),
                launchd: launchd.get(&s.pid).cloned(),
            }
        })
        .collect();
    out.sort_by(|a, b| {
        (a.scope.rank(), &a.process, port_key(&a.port), &a.proto).cmp(&(
            b.scope.rank(),
            &b.process,
            port_key(&b.port),
            &b.proto,
        ))
    });
    out
}

fn port_key(port: &str) -> u32 {
    port.parse().unwrap_or(u32::MAX)
}

pub fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Read this host's listeners. macOS only; `crate::linux` owns the other branch.
pub fn collect_macos() -> Result<Vec<Listener>, String> {
    let tcp = capture("netstat", &["-anv", "-p", "tcp"])?;
    let udp = capture("netstat", &["-anv", "-p", "udp"])?;
    let ps = capture("ps", &["-Ao", "pid=,uid=,comm="])?;
    let ifc = capture("ifconfig", &["-a"])?;
    // launchctl is the one optional reading: its column is a convenience, and a host where it
    // fails still has a complete and correct listener table.
    let launchd = capture("launchctl", &["list"]).unwrap_or_default();

    let mut sockets = parse_netstat(&tcp);
    sockets.extend(parse_netstat(&udp));
    Ok(assemble(
        &sockets,
        &parse_ps(&ps),
        &parse_launchctl(&launchd),
        &parse_ifconfig(&ifc),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NETSTAT_TCP: &str = include_str!("../fixtures/netstat-tcp.txt");
    const NETSTAT_UDP: &str = include_str!("../fixtures/netstat-udp.txt");
    const PS: &str = include_str!("../fixtures/ps.txt");
    const LAUNCHCTL: &str = include_str!("../fixtures/launchctl.txt");
    const IFCONFIG: &str = include_str!("../fixtures/ifconfig.txt");

    fn fixture_listeners() -> Vec<Listener> {
        let mut sockets = parse_netstat(NETSTAT_TCP);
        sockets.extend(parse_netstat(NETSTAT_UDP));
        assemble(
            &sockets,
            &parse_ps(PS),
            &parse_launchctl(LAUNCHCTL),
            &parse_ifconfig(IFCONFIG),
        )
    }

    fn find<'a>(rows: &'a [Listener], port: &str) -> &'a Listener {
        rows.iter()
            .find(|r| r.port == port)
            .unwrap_or_else(|| panic!("no listener on port {port}"))
    }

    /// The process field holds spaces, and one shape puts a space immediately before the colon.
    /// Both are live on the build host; `(\S+):(\d+)` gets both wrong.
    #[test]
    fn process_field_with_spaces_keeps_the_whole_name() {
        let rows = parse_netstat(NETSTAT_TCP);
        let orbstack = rows.iter().find(|r| r.local_port == "32222").unwrap();
        assert_eq!(orbstack.comm16, "OrbStack Helper");
        assert_eq!(orbstack.pid, 74138);

        let obsidian = rows.iter().find(|r| r.local_port == "27124").unwrap();
        assert_eq!(obsidian.comm16, "Obsidian Helper");
        assert_eq!(obsidian.pid, 80286);
    }

    /// The regression test for the whole netstat-over-lsof decision. Swap the parser back to
    /// `lsof` without root and this row disappears, because it belongs to uid 0.
    #[test]
    fn a_root_owned_listener_survives_the_join() {
        let rows = fixture_listeners();
        let launchd = find(&rows, "8021");
        assert_eq!(launchd.uid, Some(0));
        assert_eq!(launchd.process, "launchd");
        assert_eq!(launchd.scope, Scope::Loopback);
    }

    /// netstat truncates at sixteen characters; ps has the path. Without the join this row
    /// reads `io.tailscale.ipn`, which is a different name from the one that is running.
    #[test]
    fn the_pid_join_repairs_the_sixteen_character_truncation() {
        let rows = fixture_listeners();
        let ext = find(&rows, "48167");
        assert_eq!(ext.process, "example-vpn-network-extension");
        assert_eq!(ext.scope, Scope::Wildcard);
    }

    #[test]
    fn scope_covers_all_four_answers() {
        let ifaces = parse_ifconfig(IFCONFIG);
        assert_eq!(classify("*", &ifaces), Scope::Wildcard);
        assert_eq!(classify("0.0.0.0", &ifaces), Scope::Wildcard);
        assert_eq!(classify("::", &ifaces), Scope::Wildcard);
        assert_eq!(classify("127.0.0.1", &ifaces), Scope::Loopback);
        assert_eq!(classify("::1", &ifaces), Scope::Loopback);
        // In 100.64.0.0/10 by arithmetic, with no interface entry at all.
        assert_eq!(classify("100.101.102.103", &ifaces), Scope::Tailnet);
        // On a utun interface, outside the CGNAT range: the second, independent test.
        assert_eq!(classify("2001:db8:5::1", &ifaces), Scope::Tailnet);
        assert_eq!(classify("192.0.2.10", &ifaces), Scope::Lan);
        assert_eq!(classify("198.51.100.7", &ifaces), Scope::Lan);
    }

    /// 100.63.x and 100.128.x are outside 100.64.0.0/10 and are ordinary addresses.
    #[test]
    fn cgnat_boundaries_hold() {
        assert!(is_cgnat("100.64.0.1"));
        assert!(is_cgnat("100.127.255.254"));
        assert!(!is_cgnat("100.63.255.255"));
        assert!(!is_cgnat("100.128.0.1"));
        assert!(!is_cgnat("10.64.0.1"));
    }

    /// 28 of 41 UDP rows on the build host are unbound sending sockets. Counting them as
    /// listeners would make the first `check` run fire on half the daemons on the machine.
    #[test]
    fn udp_sending_sockets_are_not_listeners() {
        let rows = parse_netstat(NETSTAT_UDP);
        let bare = rows.iter().filter(|r| r.local_port == "*").count();
        assert!(bare >= 2, "fixture must carry the sending-socket shape");
        assert!(rows
            .iter()
            .filter(|r| is_listener(r))
            .all(|r| r.local_port != "*"));
        // ...and the wildcard-with-a-real-port rows are kept.
        assert!(rows
            .iter()
            .any(|r| is_listener(r) && r.local_address == "*" && r.local_port == "5353"));
        // A connected UDP socket has a real peer and is not exposure either.
        assert!(!rows.iter().any(|r| is_listener(r) && r.foreign != "*.*"));
    }

    #[test]
    fn a_pid_launchd_does_not_know_has_no_label() {
        let rows = fixture_listeners();
        assert_eq!(
            find(&rows, "59039").launchd.as_deref(),
            Some("com.example.rapportd")
        );
        // pid 1 is absent from the user-domain list on a healthy machine.
        assert_eq!(find(&rows, "8021").launchd, None);
    }

    #[test]
    fn ps_paths_containing_spaces_parse() {
        let procs = parse_ps(PS);
        let daemon = procs.iter().find(|p| p.pid == 22458).unwrap();
        assert_eq!(
            daemon.path,
            "/Library/Application Support/Example/interceptor-daemon"
        );
    }

    #[test]
    fn ipv6_addresses_split_on_the_last_dot() {
        assert_eq!(
            split_host_port("2001:db8:1722:b10.63396"),
            ("2001:db8:1722:b10".to_string(), "63396".to_string())
        );
        assert_eq!(split_host_port("*.*"), ("*".to_string(), "*".to_string()));
        assert_eq!(
            split_host_port("::1.8021"),
            ("::1".to_string(), "8021".to_string())
        );
    }
}
