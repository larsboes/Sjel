//! `sjel capability …` — find a capability's HTTP surface, probe it, and call it.
//!
//! Moved from the bash launcher and tools/lib/capability-probe.sh on 2026-10-02. The rows come
//! from the registry module in-process (the same code `tools/capability.sh registry` runs), and
//! HTTP still goes
//! through curl: the inbound token reaches curl on stdin (`-H @-`), so it is in no argv and `ps`
//! cannot show it, the property the bash `-H @<(...)` gave.
//!
//! Where to poll mirrors `probe_url` and `readiness_url` in
//! capabilities/sjel-status/src/status/registry.rs. What the answer means (`up`, `off`,
//! `down`) is this CLI's own, argued at [`state`].

use std::io::Write as _;
use std::path::Path;
use std::process::{Command, ExitCode, Stdio};

use crate::exit_with;
use crate::paths::Paths;
use crate::registry::Service;

/// One registry row, with every field the probe rules read.
#[derive(Debug, Default, Clone)]
pub struct Row {
    pub name: String,
    pub kind: String,
    pub scope: String,
    pub port: String,
    pub endpoint: String,
    pub health_path: String,
    pub ready_path: String,
    pub autostart: String,
}

impl Row {
    fn from_service(s: &Service) -> Self {
        Self {
            name: s.name.clone(),
            kind: s.kind.clone(),
            scope: s.scope.clone(),
            port: s.field("port").to_owned(),
            endpoint: s.endpoint.clone(),
            health_path: s.field("health_path").to_owned(),
            ready_path: s.field("ready_path").to_owned(),
            autostart: s.field("autostart").to_owned(),
        }
    }

    fn is_external(&self) -> bool {
        self.scope == "external"
    }

    /// Where to poll, or `None` when nothing can answer for this capability.
    ///
    /// Readiness first: until 2026-08-07 sjel-status polled `health_path` everywhere, and five
    /// database-backed capabilities answered it from a handler that could not see their
    /// database (Axon#126). An external capability has no port here, because a port is a fact
    /// about the host that binds it, so it is polled at its resolved endpoint and never on
    /// loopback.
    pub fn probe_url(&self) -> Option<String> {
        let path = if self.ready_path.is_empty() {
            &self.health_path
        } else {
            &self.ready_path
        };
        if path.is_empty() {
            return None;
        }
        if self.is_external() {
            (!self.endpoint.is_empty()).then(|| format!("{}{path}", self.endpoint))
        } else {
            (!self.port.is_empty()).then(|| format!("http://127.0.0.1:{}{path}", self.port))
        }
    }

    /// The base URL a caller dials: loopback for a capability this machine runs, the resolved
    /// endpoint for one it only consumes. Empty when neither is known.
    pub fn base_url(&self) -> String {
        if self.is_external() {
            self.endpoint.clone()
        } else if self.port.is_empty() {
            String::new()
        } else {
            format!("http://127.0.0.1:{}", self.port)
        }
    }

    /// `up`, `off` or `down` for an HTTP status code (`000` when nothing answered).
    ///
    /// `off` exists because `down` carried two meanings. `dashboard` declares port 47117 and
    /// `autostart = "false"`: a dev server started by hand. It was reported down while the
    /// dashboard answered 200, and sent sessions to a dead URL. A capability this machine is not
    /// supposed to keep running is not faulty when it is not running.
    ///
    /// An external capability is never `off`. tools/capability.sh blanks its `autostart`,
    /// because how another host runs it is that host's declaration, and reading the blank as
    /// "optional" told the operator to start another host's service with this one's supervisor
    /// (measured 2026-09-08 against vaultwarden).
    ///
    /// A local capability with no `autostart` stays `off`: tools/capability.sh reads an absent
    /// field as "false" too, so the supervisor does not keep it running either.
    pub fn state(&self, code: &str) -> &'static str {
        if code == "200" {
            "up"
        } else if self.is_external() || self.autostart == "true" {
            "down"
        } else {
            "off"
        }
    }
}

/// Every registry row, built in-process (tools/sjel-cli/src/registry.rs) rather than by
/// running tools/capability.sh, whose bash version took most of every verb's 2.5 s.
///
/// A registry that cannot be read is an error, not an empty machine: the bash version of
/// `sjel` printed nothing and exited 0 for `list`, the same thing as a clean bill of health.
pub fn registry(root: &Path) -> Result<Vec<Row>, String> {
    registry_with(&Paths::from_shell(root)?)
}

pub fn registry_with(paths: &Paths) -> Result<Vec<Row>, String> {
    match crate::registry::services(paths) {
        Ok(services) => Ok(services.iter().map(Row::from_service).collect()),
        Err(f) => {
            eprintln!("{}", f.msg);
            Err("tools/capability.sh registry failed".to_owned())
        }
    }
}

/// Rows with an HTTP surface: a port here, or an endpoint elsewhere.
fn http_rows(root: &Path) -> Result<Vec<Row>, String> {
    Ok(registry(root)?
        .into_iter()
        .filter(|r| !r.port.is_empty() || !r.endpoint.is_empty())
        .collect())
}

/// The HTTP status of one probe, or `000`. Timeouts are bounded because an external
/// capability is reached over the tailnet, and a sleeping peer would otherwise hang the sweep.
fn probe_code(url: &str) -> String {
    Command::new("curl")
        .args([
            "-s",
            "-o",
            "/dev/null",
            "-w",
            "%{http_code}",
            "--connect-timeout",
            "2",
            "--max-time",
            "5",
            url,
        ])
        .stderr(Stdio::null())
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .filter(|c| !c.is_empty())
        .unwrap_or_else(|| "000".to_owned())
}

/// Probe every row at once. The bash loop probed one after another, so one sleeping tailnet
/// peer cost the whole sweep up to 5 s per capability. Results keep the registry's order.
fn probe_all(rows: &[Row]) -> Vec<Option<(String, String)>> {
    std::thread::scope(|s| {
        let handles: Vec<_> = rows
            .iter()
            .map(|r| {
                s.spawn(move || {
                    r.probe_url().map(|u| {
                        let code = probe_code(&u);
                        (u, code)
                    })
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap_or(None))
            .collect()
    })
}

pub fn command(root: &Path, args: &[String]) -> ExitCode {
    let (cmd, rest) = match args.split_first() {
        Some((c, r)) => (c.as_str(), r),
        None => ("list", &[][..]),
    };
    let result = match cmd {
        "list" => list(root),
        "health" | "status" => health(root),
        "url" => match rest {
            [name] => url(root, name).map(|u| {
                println!("{u}");
                ExitCode::SUCCESS
            }),
            _ => usage_error("usage: sjel capability url <capability>"),
        },
        "call" => {
            if rest.len() < 3 {
                usage_error(
                    "usage: sjel capability call <name> <get|post|put|patch|delete> <path> [body] [curl-args...]",
                )
            } else {
                call(root, &rest[0], &rest[1], &rest[2], &rest[3..])
            }
        }
        "ingest" => match rest {
            [u] => {
                let body = serde_json::json!({ "url": u }).to_string();
                call(root, "comms", "post", "/ingest", &[body])
            }
            _ => usage_error("usage: sjel capability ingest <url>"),
        },
        "feed" => {
            let days = rest.first().map_or("7", String::as_str);
            if days.is_empty() || !days.bytes().all(|b| b.is_ascii_digit()) {
                usage_error("sjel: days must be a positive integer")
            } else {
                call(root, "comms", "get", &format!("/feed?days={days}"), &[])
            }
        }
        "mail" => {
            // Mail triage without Secret rows. From an agent session the gate also
            // pseudonymizes every row (ISA F9); from the operator's shell the rows are raw.
            let status = rest.first().map_or("", String::as_str);
            if !status.bytes().all(|b| b.is_ascii_lowercase() || b == b'_') {
                usage_error("sjel: status must be a word like proposed")
            } else {
                let filter = if status.is_empty() {
                    String::new()
                } else {
                    format!("&status={status}")
                };
                call(
                    root,
                    "comms",
                    "get",
                    &format!("/triage?max_data_class=c2{filter}"),
                    &[],
                )
            }
        }
        _ => usage_error("usage: sjel capability {list|health|url|call|ingest|feed|mail}"),
    };
    result.unwrap_or_else(|e| {
        eprintln!("{e}");
        ExitCode::from(1)
    })
}

fn usage_error(msg: &str) -> Result<ExitCode, String> {
    Err(msg.to_owned())
}

fn list(root: &Path) -> Result<ExitCode, String> {
    let rows = http_rows(root)?;
    for (row, probe) in rows.iter().zip(probe_all(&rows)) {
        let state = probe.map_or("unknown", |(_, code)| row.state(&code));
        println!("{:<18} {:<38} {state}", row.name, row.base_url());
    }
    Ok(ExitCode::SUCCESS)
}

fn health(root: &Path) -> Result<ExitCode, String> {
    let rows = http_rows(root)?;
    let mut failed = false;
    let mut any = false;
    for (row, probe) in rows.iter().zip(probe_all(&rows)) {
        // Reported, not skipped. A capability with no probe path is one nothing can answer for,
        // and dropping the row said the same thing as a clean bill of health.
        let Some((url, code)) = probe else {
            println!("unknown {:<18} — no health or ready path to poll", row.name);
            continue;
        };
        any = true;
        match row.state(&code) {
            "up" => println!("up      {:<18} {url}", row.name),
            "down" => {
                println!("down    {:<18} {url}  HTTP {code}", row.name);
                failed = true;
            }
            _ => println!(
                "off     {:<18} {url}  not autostarted; start it with tools/service-runner.sh start {}",
                row.name, row.name
            ),
        }
    }
    // An empty sweep is a broken registry, not a healthy machine.
    if !any {
        return Err(
            "sjel: no capability had anything to probe — the registry is empty or unreadable"
                .to_owned(),
        );
    }
    println!();
    println!("down = should be answering and is not, which includes every capability another host");
    println!("runs. off = this machine does not autostart it, so not running is not a fault.");
    println!("Only 'down' sets a non-zero exit.");
    Ok(if failed {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    })
}

/// A capability's base URL. An external capability answers too: its port is blank by
/// construction, and a lookup keyed on the port reported "no registered HTTP surface" for a
/// capability whose URL the registry had already resolved.
fn url(root: &Path, name: &str) -> Result<String, String> {
    registry(root)?
        .into_iter()
        .find(|r| r.name == name)
        .map(|r| r.base_url())
        .filter(|u| !u.is_empty())
        .ok_or_else(|| format!("sjel: capability '{name}' has no registered HTTP surface"))
}

/// An agent session is read-only and every response is pseudonymized (ISA F9). `SJEL_AGENT=1`
/// marks one by hand, `CLAUDECODE=1` is Claude Code's own marker, and `SJEL_AGENT=0` turns it
/// off for the operator.
fn in_agent_session() -> bool {
    match std::env::var("SJEL_AGENT").as_deref() {
        Ok("1") => true,
        Ok("0") => false,
        _ => std::env::var("CLAUDECODE").as_deref() == Ok("1"),
    }
}

/// Which token, if any, rides with a call.
enum Auth {
    None,
    Agent,
    Capability(String),
}

fn auth_tool(root: &Path) -> Command {
    Command::new(root.join("tools/capability-auth/capability-auth"))
}

/// `capability-auth --check <who>` prints `configured` when a token is declared.
fn configured(root: &Path, who: &[&str]) -> bool {
    auth_tool(root)
        .arg("--check")
        .args(who)
        .stderr(Stdio::null())
        .output()
        .is_ok_and(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .any(|l| l == "configured")
        })
}

/// One call through curl. `--fail-with-body`, not `-f`: a capability that rejects a write puts
/// the reason in the body, and `-f` discards it. The exit status is curl's.
///
/// The token is attached to a loopback base URL only. An external capability answers on another
/// host, and the deployment-wide token is a credential for this machine's gate, not for whatever
/// a registry endpoint points at.
fn call(
    root: &Path,
    name: &str,
    method: &str,
    path: &str,
    extra: &[String],
) -> Result<ExitCode, String> {
    let base = url(root, name)?;
    let auth = if base.starts_with("http://127.0.0.1:") {
        if in_agent_session() {
            if !configured(root, &["--agent"]) {
                return Err(
                    "sjel: this is an agent session and no agent token is enrolled. The operator runs 'sjel agent enroll'."
                        .to_owned(),
                );
            }
            Auth::Agent
        } else if configured(root, &[name]) {
            Auth::Capability(name.to_owned())
        } else {
            Auth::None
        }
    } else {
        Auth::None
    };

    let mut curl = Command::new("curl");
    curl.args(["-s", "--fail-with-body"])
        .arg(format!("{base}{path}"));
    match method {
        "get" => {
            curl.args(extra);
        }
        "delete" => {
            curl.args(["-X", "DELETE"]).args(extra);
        }
        // PUT, PATCH and DELETE are here because capabilities serve them: trips has
        // `PATCH /api/plans/:id`, calendar's idempotent upsert is `PUT /api/entries/external`.
        "post" | "put" | "patch" => {
            let (body, rest) = extra
                .split_first()
                .map_or(("", &[][..]), |(b, r)| (b.as_str(), r));
            curl.args(["-X", &method.to_ascii_uppercase()])
                .args(["-H", "Content-Type: application/json", "-d", body])
                .args(rest);
        }
        _ => return Err("sjel: method must be one of get, post, put, patch, delete".to_owned()),
    }

    let header = match &auth {
        Auth::None => None,
        Auth::Agent => Some(token_header(root, &["--agent"])?),
        Auth::Capability(n) => Some(token_header(root, &[n.as_str()])?),
    };
    if header.is_some() {
        curl.args(["-H", "@-"]);
    }
    if let (Auth::Agent, Ok(session)) = (&auth, std::env::var("SJEL_AGENT_SESSION")) {
        if !session.is_empty() {
            curl.args(["-H", &format!("X-Sjel-Agent-Session: {session}")]);
        }
    }

    let Some(header) = header else {
        return Ok(exit_with(curl.status()));
    };
    let mut child = curl
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|e| format!("sjel: cannot run curl: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        // A write error means curl already exited; its status below says why.
        let _ = stdin.write_all(&header);
    }
    Ok(exit_with(child.wait()))
}

/// The header line `capability-auth` prints for `who`. It stays in this process's memory and
/// curl's stdin; it is never an argument.
fn token_header(root: &Path, who: &[&str]) -> Result<Vec<u8>, String> {
    let out = auth_tool(root)
        .args(who)
        .stderr(Stdio::inherit())
        .output()
        .map_err(|e| format!("sjel: cannot run capability-auth: {e}"))?;
    Ok(out.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Moved from tools/capability-probe.test.sh, one case per assertion it held.

    fn row(
        scope: &str,
        port: &str,
        endpoint: &str,
        health: &str,
        ready: &str,
        autostart: &str,
    ) -> Row {
        Row {
            scope: scope.into(),
            port: port.into(),
            endpoint: endpoint.into(),
            health_path: health.into(),
            ready_path: ready.into(),
            autostart: autostart.into(),
            ..Row::default()
        }
    }

    const EXT: &str = "https://homepi.example.ts.net";

    #[test]
    fn where_to_poll() {
        let p = |r: Row| r.probe_url().unwrap_or_default();
        assert_eq!(
            p(row("capability", "8083", "", "/health", "/ready", "")),
            "http://127.0.0.1:8083/ready",
            "readiness wins"
        );
        assert_eq!(
            p(row("capability", "3000", "", "/health", "", "")),
            "http://127.0.0.1:3000/health",
            "health is the fallback"
        );
        assert_eq!(
            p(row("capability", "8080", "", "", "", "")),
            "",
            "no path is nothing to poll"
        );
        assert_eq!(
            p(row("capability", "", "", "/health", "", "")),
            "",
            "no port, no loopback surface"
        );
        assert_eq!(
            p(row("external", "", EXT, "/alive", "", "")),
            format!("{EXT}/alive"),
            "external polled at its endpoint"
        );
        assert_eq!(
            p(row("external", "", "", "/alive", "", "")),
            "",
            "external with no endpoint"
        );
        assert_eq!(
            p(row("external", "8080", EXT, "/alive", "", "")),
            format!("{EXT}/alive"),
            "external never on loopback"
        );
    }

    #[test]
    fn base_urls() {
        assert_eq!(
            row("capability", "8086", "", "", "", "").base_url(),
            "http://127.0.0.1:8086"
        );
        assert_eq!(row("external", "", EXT, "", "", "").base_url(), EXT);
        assert_eq!(row("capability", "", "", "", "", "").base_url(), "");
    }

    #[test]
    fn what_an_answer_means() {
        let local = |a: &str| row("capability", "1", "", "/h", "", a);
        let ext = |a: &str| row("external", "", EXT, "/h", "", a);
        assert_eq!(
            local("false").state("200"),
            "up",
            "a 200 is up whatever the manifest says"
        );
        assert_eq!(
            local("true").state("000"),
            "down",
            "autostart and silent is down"
        );
        assert_eq!(
            local("false").state("000"),
            "off",
            "autostart=false and silent is off"
        );
        assert_eq!(
            local("true").state("500"),
            "down",
            "a 500 from something that should run"
        );
        assert_eq!(local("false").state("404"), "off", "a 404 is not up");
        assert_eq!(
            local("").state("000"),
            "off",
            "absent autostart is no claim it should run"
        );
        assert_eq!(ext("").state("000"), "down", "external silent is down");
        assert_eq!(ext("").state("200"), "up", "external answering is up");
        assert_eq!(
            ext("false").state("000"),
            "down",
            "external is down even if autostart leaked"
        );
    }

    #[test]
    fn a_service_maps_to_its_probe_fields() {
        // The bash suite guarded a tab-separator bug that shifted `autostart` into
        // `health_path` when a field was empty. Fields are named now; this checks the mapping.
        let s = Service {
            name: "sjel-status".into(),
            kind: "process".into(),
            scope: "capability".into(),
            fields: vec![
                ("port", "8082".into()),
                ("health_path", "/health".into()),
                ("ready_path", String::new()),
                ("autostart", "true".into()),
            ],
            endpoint: String::new(),
            proxy_extra: vec![],
            requires: vec![],
        };
        let r = Row::from_service(&s);
        assert_eq!(
            (
                r.endpoint.as_str(),
                r.health_path.as_str(),
                r.ready_path.as_str(),
                r.autostart.as_str()
            ),
            ("", "/health", "", "true")
        );
        assert_eq!(
            r.probe_url().as_deref(),
            Some("http://127.0.0.1:8082/health")
        );
    }
}
