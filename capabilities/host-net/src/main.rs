//! host-net — what this host exposes to the network, read without sudo.
//!
//! Four verbs, one flag, no mutation. This capability observes; `capabilities/host-firewall`
//! is the one that changes a rule, and nothing here proposes one.
//!
//! Exit codes, which `tools/host-watch` and any other caller depend on:
//!
//! * `0` — checked, and everything matched.
//! * `1` — checked, and something is exposed that the policy does not account for, or
//!   (`tailnet`) the running tailscaled is older than the newest release on its track.
//! * `2` — could not check: a usage error, no policy file, an unsupported platform, or a
//!   required command that is missing or refused. Two rather than three because `tools/audit`
//!   and `capabilities/host-firewall/host-firewall` both already spell "usage or setup error"
//!   as 2, and `tools/host-watch` already spells "no policy" as 2. A third meaning invented
//!   here would be the one number in the repo that means something else.

mod firewall;
mod linux;
mod listen;
mod policy;
mod sys;
mod tailnet;

use std::process::ExitCode;

use listen::{Listener, Scope};

const HELP: &str = "host-net — what this host exposes to the network, read without sudo.

  host-net listen [--json]     every listening socket, with the scope its bind reaches (default)
  host-net firewall [--json]   the application firewall's switches and its stale app rules
  host-net tailnet [--json] [--socket=PATH]
                               tailnet posture: backend, shields, key expiry, tailnet lock,
                               and the running version against the newest release
  host-net check [--json]      wildcard listeners the policy does not account for
  host-net -h                  this help

Policy: <overlay>/config/host-net-policy.toml — see schemas/host-net-policy.toml.example.

Exit: 0 = matched the policy, 1 = unexpected exposure or an outdated tailscaled,
      2 = could not check.
";

const EXIT_MATCHED: u8 = 0;
const EXIT_UNEXPECTED: u8 = 1;
const EXIT_CANNOT_CHECK: u8 = 2;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print!("{HELP}");
        return ExitCode::SUCCESS;
    }
    let json = args.iter().any(|a| a == "--json");
    let socket = args.iter().find_map(|a| a.strip_prefix("--socket="));
    if let Some(bad) = args
        .iter()
        .find(|a| a.starts_with('-') && a.as_str() != "--json" && !a.starts_with("--socket="))
    {
        eprintln!("host-net: unknown flag '{bad}'\n\n{HELP}");
        return ExitCode::from(EXIT_CANNOT_CHECK);
    }
    let verb = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .map_or("listen", String::as_str);

    let result = match verb {
        "listen" => cmd_listen(json),
        "firewall" => cmd_firewall(json),
        "tailnet" => cmd_tailnet(json, socket),
        "check" => cmd_check(json),
        other => Err(format!("unknown command '{other}'\n\n{HELP}")),
    };
    match result {
        Ok(code) => ExitCode::from(code),
        Err(message) => {
            eprintln!("host-net: {message}");
            ExitCode::from(EXIT_CANNOT_CHECK)
        }
    }
}

/// Read this host's listeners on whichever platform this is.
///
/// An unknown platform is an error, never an empty list. "No listeners found" and "I cannot
/// look" are opposite answers and only one of them is reassuring
/// (Packs/harness/skills/sjel/references/shared-failure-policy.md).
fn listeners() -> Result<Vec<Listener>, String> {
    match std::env::consts::OS {
        "macos" => listen::collect_macos(),
        "linux" => linux::collect(),
        other => Err(format!(
            "unsupported platform '{other}' — reporting nothing rather than reporting a clean host"
        )),
    }
}

fn cmd_listen(json: bool) -> Result<u8, String> {
    let rows = listeners()?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "os": std::env::consts::OS,
                "listeners": rows,
            }))
            .map_err(|e| e.to_string())?
        );
        return Ok(0);
    }
    let mut table = vec![vec![
        "PROTO".into(),
        "ADDRESS".into(),
        "PORT".into(),
        "SCOPE".into(),
        "PID".into(),
        "PROCESS".into(),
        "LAUNCHD".into(),
    ]];
    for r in &rows {
        table.push(vec![
            r.proto.clone(),
            r.address.clone(),
            r.port.clone(),
            r.scope.as_str().into(),
            r.pid.to_string(),
            r.process.clone(),
            r.launchd.clone().unwrap_or_else(|| "—".into()),
        ]);
    }
    print_table(&table);
    let count = |s: Scope| rows.iter().filter(|r| r.scope == s).count();
    println!(
        "\n{} listener(s): {} wildcard, {} lan, {} tailnet, {} loopback",
        rows.len(),
        count(Scope::Wildcard),
        count(Scope::Lan),
        count(Scope::Tailnet),
        count(Scope::Loopback),
    );
    Ok(0)
}

fn cmd_firewall(json: bool) -> Result<u8, String> {
    if std::env::consts::OS != "macos" {
        return Err(format!(
            "no readable firewall layer on '{}' — `nft list ruleset` needs root and host-net never uses sudo",
            std::env::consts::OS
        ));
    }
    let report = firewall::collect()?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?
        );
        return Ok(0);
    }
    let s = &report.switches;
    let onoff = |v: Option<bool>, yes: &str, no: &str| match v {
        Some(true) => yes.to_string(),
        Some(false) => no.to_string(),
        None => "?".to_string(),
    };
    print_table(&[
        vec!["SWITCH".into(), "STATE".into()],
        vec!["firewall".into(), onoff(s.enabled, "enabled", "disabled")],
        vec!["stealth mode".into(), onoff(s.stealth, "on", "off")],
        vec![
            "allow built-in signed".into(),
            onoff(s.allow_builtin_signed, "enabled", "disabled"),
        ],
        vec![
            "allow downloaded signed".into(),
            onoff(s.allow_downloaded_signed, "enabled", "disabled"),
        ],
        vec![
            "block all".into(),
            onoff(s.block_all, "enabled", "disabled"),
        ],
    ]);

    let stale: Vec<&firewall::AppRule> = report.apps.iter().filter(|a| !a.present).collect();
    let allowed = report.apps.iter().filter(|a| a.allow == Some(true)).count();
    println!(
        "\n{} app rule(s): {allowed} allow, {} block, {} pointing at software that is not here",
        report.apps.len(),
        report
            .apps
            .iter()
            .filter(|a| a.allow == Some(false))
            .count(),
        stale.len(),
    );
    for class in [
        firewall::StaleClass::HomebrewCellar,
        firewall::StaleClass::SystemExtension,
        firewall::StaleClass::AppTranslocation,
        firewall::StaleClass::ExternalVolume,
        firewall::StaleClass::Removed,
    ] {
        let of_class: Vec<&&firewall::AppRule> = stale
            .iter()
            .filter(|a| a.stale_class == Some(class))
            .collect();
        if of_class.is_empty() {
            continue;
        }
        println!("\n  {} ({})", class.as_str(), of_class.len());
        for rule in of_class {
            println!("    {}", rule.path);
        }
    }
    Ok(0)
}

fn cmd_tailnet(json: bool, socket: Option<&str>) -> Result<u8, String> {
    let report = tailnet::collect(socket)?;
    // --json still carries the verdict in the exit code, as `check` does.
    let verdict = if report.version.behind {
        EXIT_UNEXPECTED
    } else {
        EXIT_MATCHED
    };
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?
        );
        return Ok(verdict);
    }
    let v = &report.version;
    let p = &report.prefs;
    let yesno = |v: Option<bool>| match v {
        Some(true) => "yes".to_string(),
        Some(false) => "no".to_string(),
        None => "?".to_string(),
    };
    print_table(&[
        vec!["FACT".into(), "VALUE".into()],
        vec!["backend".into(), report.backend_state.clone()],
        vec![
            "version running".into(),
            match (&v.upstream, v.behind) {
                (Some(u), true) => format!("{} (BEHIND {u})", v.running),
                (Some(_), false) => format!("{} (current)", v.running),
                (None, _) => format!("{} (newest release unknown)", v.running),
            },
        ],
        vec!["tun up".into(), yesno(report.tun)],
        vec!["want running".into(), yesno(p.want_running)],
        vec!["shields up".into(), yesno(p.shields_up)],
        vec!["accept routes".into(), yesno(p.route_all)],
        vec!["tailscale ssh".into(), yesno(p.run_ssh)],
        vec!["posture checking".into(), yesno(p.posture_checking)],
        vec![
            "exit node in use".into(),
            if p.uses_exit_node {
                "yes".into()
            } else {
                "no".into()
            },
        ],
        vec!["routes advertised".into(), p.advertised_routes.to_string()],
        vec![
            "peers".into(),
            format!("{} ({} online)", report.peers, report.peers_online),
        ],
        vec![
            "peers whose key never expires".into(),
            report.peers_never_expiring.len().to_string(),
        ],
        vec![
            "own key expires".into(),
            if report.self_key_expires {
                "yes".into()
            } else {
                "no".into()
            },
        ],
        vec![
            "tailnet lock".into(),
            if report.lock.enabled {
                format!(
                    "enabled, {} trusted signing key(s)",
                    report.lock.trusted_keys
                )
            } else {
                "not enabled".into()
            },
        ],
    ]);
    // The counts are the table's job; the names are --json's. See src/tailnet.rs's header.
    if !report.peers_never_expiring.is_empty() {
        println!(
            "\n{} peer(s) have a node key that never expires — `--json` names them.",
            report.peers_never_expiring.len()
        );
    }
    if report.lock.enabled && report.lock.trusted_keys == 1 {
        println!(
            "Tailnet lock trusts exactly one signing key: losing it locks new nodes out for good."
        );
    }
    if v.behind {
        println!(
            "The running tailscaled is older than the newest release: update it, then restart the daemon."
        );
    }
    Ok(verdict)
}

fn cmd_check(json: bool) -> Result<u8, String> {
    let (path, policy) = policy::require(policy::load()?)?;
    let rows = listeners()?;
    let found = policy::unexpected(&rows, &policy);
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "os": std::env::consts::OS,
                "policy": path.display().to_string(),
                "listeners": rows.len(),
                "wildcard": rows.iter().filter(|r| r.scope == Scope::Wildcard).count(),
                "expected": policy.expect_wildcard,
                "unexpected": found,
            }))
            .map_err(|e| e.to_string())?
        );
    } else if found.is_empty() {
        println!(
            "host-net: {} listener(s), {} wildcard, all accounted for by {}",
            rows.len(),
            rows.iter().filter(|r| r.scope == Scope::Wildcard).count(),
            path.display()
        );
    } else {
        println!(
            "host-net: {} wildcard listener(s) the policy does not account for:",
            found.len()
        );
        for e in &found {
            println!("  {} · pid {}", e.describe(), e.pid);
            if let Some(p) = &e.path {
                println!("      {p}");
            }
        }
        println!("\nAccept one by adding it to {}:", path.display());
        println!("  [[expect_wildcard]]");
        println!("  process = \"{}\"", found[0].process);
        println!("  reason = \"why this may be reachable from everywhere\"");
    }
    // --json still carries the verdict in the exit code: host-watch reads both, and a
    // non-zero exit here is data, not a failure.
    Ok(policy::verdict(&found))
}

/// Left-aligned columns, padded to the widest cell. Row 0 is the header.
fn print_table(rows: &[Vec<String>]) {
    let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
    let widths: Vec<usize> = (0..columns)
        .map(|c| {
            rows.iter()
                .filter_map(|r| r.get(c))
                .map(|s| s.chars().count())
                .max()
                .unwrap_or(0)
        })
        .collect();
    for row in rows {
        let mut line = String::new();
        for (c, cell) in row.iter().enumerate() {
            line.push_str(cell);
            if c + 1 < row.len() {
                let pad = widths[c].saturating_sub(cell.chars().count()) + 2;
                line.push_str(&" ".repeat(pad));
            }
        }
        println!("{}", line.trim_end());
    }
}
