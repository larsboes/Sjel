// tools/sjel-mcp/src/main.rs — register Sjel's MCP server with the agent harnesses on this
// machine, and verify each registration against the server itself (ISA ISC-40).
//
// Why this exists: the server shipped 2026-10-01 and was registered in no harness at all. The
// ISA named the command that would have done it, nobody ran it, and nothing in this repository
// could have noticed. Wiring that lives only in a document rots without a claim failing. This
// is the claim: it writes the registration, then speaks MCP to the server it wrote and reports
// the tool count it got back.
//
// ## Two harnesses, and why the list is short
//
// This is a table of MEASURED MCP registration paths, not a harness registry.
// tools/sjel-cli/src/harnesses/registry.rs is the authority on which harnesses exist and which
// are installed; what this file adds is the two whose MCP surface has actually been driven and
// watched working. Codex is absent on purpose: it supports MCP, its config format is
// documented elsewhere, and nothing here has run it — ~/.codex exists on the dev machine
// while no `codex` binary does. That registry states the rule, from the day three Packs
// were deployed for a Codex that was not installed: a row moves in when someone verifies the
// format, never on the strength of a guess.
//
// ## Why pi is written directly and claude is not
//
// tools/agent-integrations.sh drives an upstream's own installer rather than keeping a copy of
// what it emits, and this tool follows that where it can: Claude Code is registered by running
// `claude mcp add`. pi is the exception, and the measurement is the reason — `pi mcp add`
// rejects `--timeout` ("Unknown option"), and that per-request timeout is load-bearing here,
// because an ask-mode write waits up to 120 s for the owner's Allow while pi's default is 60 s.
//
// ## What the retired managed layer used to say here
//
// Until 2026-10-02 the Claude Code registration was also constrained by a root-owned managed
// policy with `allowManagedMcpServersOnly: true` and an allowlist. That layer is gone (see
// tools/claude-code-config/README.md), so there is no policy to satisfy and no allowlist for
// this tool to stay in step with. `claude mcp add` is the whole story now.
//
// Usage: sjel mcp | sjel mcp register [<harness>...] | sjel mcp unregister [<harness>...]
//
// With no verb this process IS the server (`src/server.rs`); the verbs below are the
// registration half, and they verify themselves by speaking MCP to it.

use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use serde_json::{json, Map, Value};

mod server;
mod tools;

pub const SERVER_NAME: &str = "sjel";

/// What the server calls itself, in the harness's own words.
const DESCRIPTION: &str = "Sjel's local capabilities on this Mac (comms, calendar, devices). \
Answers are pseudonymized: pass tokens like <SENDER_ab12cd> back unchanged. Writes follow the \
owner's per-capability mode (off, read-only, ask, auto) and an ask-mode write waits for Allow \
in the menu-bar app.";

/// pi's per-request timeout, in seconds.
///
/// 180 rather than pi's default 60, because the server polls for up to 120 s while the owner
/// decides an ask-mode write. This single field is why pi's entry is written here instead of
/// delegated to `pi mcp add`, which cannot express it.
const PI_TIMEOUT_SECONDS: u64 = 180;

fn home() -> PathBuf {
    PathBuf::from(env::var_os("HOME").unwrap_or_default())
}

/// The command and arguments that start the server.
pub fn server_command() -> Vec<String> {
    vec![
        home()
            .join(".local/bin/sjel")
            .to_string_lossy()
            .into_owned(),
        "mcp".to_string(),
    ]
}

/// pi's entry. `exposure: codemode` keeps eighty-odd tool schemas out of every prompt, and the
/// server's own guidance says to filter large capability payloads inside a script — one
/// `GET /triage` answers 384 rows, about 147k tokens, and ignores a `limit` parameter.
pub fn pi_entry() -> Value {
    let command = server_command();
    json!({
        "command": command[0],
        "args": command[1..].to_vec(),
        "exposure": "codemode",
        "timeout": PI_TIMEOUT_SECONDS,
        "description": DESCRIPTION,
    })
}

/// pi's config with our entry set, and every other server and key left alone.
pub fn pi_config_with(existing: &Value, entry: &Value) -> Value {
    let mut base = match existing.as_object() {
        Some(map) => map.clone(),
        None => Map::new(),
    };
    let mut servers = match base.get("mcpServers").and_then(Value::as_object) {
        Some(map) => map.clone(),
        None => Map::new(),
    };
    servers.insert(SERVER_NAME.to_string(), entry.clone());
    base.insert("mcpServers".to_string(), Value::Object(servers));
    Value::Object(base)
}

/// pi's config with our entry removed; an emptied `mcpServers` is dropped rather than left as
/// an empty object, so unregistering a lone server leaves the file as it was before.
pub fn pi_config_without(existing: &Value) -> Value {
    let mut base = match existing.as_object() {
        Some(map) => map.clone(),
        None => Map::new(),
    };
    if let Some(servers) = base.get("mcpServers").and_then(Value::as_object) {
        let mut remaining = servers.clone();
        remaining.remove(SERVER_NAME);
        if remaining.is_empty() {
            base.remove("mcpServers");
        } else {
            base.insert("mcpServers".to_string(), Value::Object(remaining));
        }
    }
    Value::Object(base)
}

pub fn claude_add_args() -> Vec<String> {
    let mut args = vec![
        "mcp".to_string(),
        "add".to_string(),
        SERVER_NAME.to_string(),
        "--scope".to_string(),
        "user".to_string(),
        "--".to_string(),
    ];
    args.extend(server_command());
    args
}

pub fn claude_remove_args() -> Vec<String> {
    vec![
        "mcp".to_string(),
        "remove".to_string(),
        SERVER_NAME.to_string(),
        "--scope".to_string(),
        "user".to_string(),
    ]
}

/// Which harness's marker means it is installed here. Repeated from
/// tools/sjel-cli/src/harnesses/registry.rs deliberately and only for the two harnesses this
/// tool knows: a registry shared across two crates is a synchronisation problem, and that
/// registry stays the authority for the full set.
fn installed_marker(harness: &str) -> Option<PathBuf> {
    match harness {
        "pi" => Some(home().join(".pi/agent/settings.json")),
        "claude" => Some(home().join(".claude")),
        _ => None,
    }
}

fn pi_config_path() -> PathBuf {
    home().join(".pi/agent/mcp.json")
}

fn read_json(path: &Path) -> Result<Value, String> {
    let text = fs::read_to_string(path).map_err(|error| error.to_string())?;
    serde_json::from_str(&text).map_err(|error| error.to_string())
}

fn read_json_or_empty(path: &Path) -> Result<Value, String> {
    if path.exists() {
        read_json(path)
    } else {
        Ok(Value::Object(Map::new()))
    }
}

fn write_atomic(target: &Path, contents: &str) -> std::io::Result<()> {
    let dir = target.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(dir)?;
    let name = target
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("mcp.json");
    let tmp = dir.join(format!(".{name}.sjel-{}.tmp", std::process::id()));
    let write = || -> std::io::Result<()> {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(contents.as_bytes())?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, target)
    };
    match write() {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = fs::remove_file(&tmp);
            Err(error)
        }
    }
}

/// Speak MCP to the server this tool just registered, and report what it answers with.
fn handshake() -> Result<usize, String> {
    let command = server_command();
    let binary = Path::new(&command[0]);
    if !binary.exists() {
        return Err(format!("not found: {}", command[0]));
    }
    let request = [
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "clientInfo": {"name": "sjel-mcp", "version": "0"},
                "capabilities": {}
            }
        }),
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
    ]
    .iter()
    .map(|message| message.to_string())
    .collect::<Vec<_>>()
    .join("\n");

    let mut child = Command::new(&command[0])
        .args(&command[1..])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| error.to_string())?;
    {
        let mut stdin = child.stdin.take().ok_or("no stdin on the server")?;
        stdin
            .write_all(format!("{request}\n").as_bytes())
            .map_err(|error| error.to_string())?;
        // Dropping stdin closes it, which is what lets the server exit.
    }
    let output = child
        .wait_with_output()
        .map_err(|error| error.to_string())?;
    let stdout = String::from_utf8_lossy(&output.stdout);

    let mut initialized = false;
    for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
        let Ok(reply) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if reply.get("error").is_some() {
            return Err(reply["error"]["message"]
                .as_str()
                .unwrap_or("the server returned an error")
                .to_string());
        }
        match reply.get("id").and_then(Value::as_u64) {
            Some(1) => initialized = true,
            Some(2) => {
                let count = reply["result"]["tools"]
                    .as_array()
                    .map(Vec::len)
                    .ok_or("tools/list returned no tool array")?;
                return Ok(count);
            }
            _ => {}
        }
    }
    if initialized {
        Err("the server answered initialize but not tools/list".to_string())
    } else {
        Err("the server did not answer MCP".to_string())
    }
}

struct Outcome {
    harness: String,
    ok: bool,
    notes: Vec<String>,
}

fn run(command: &str, args: &[String]) -> Result<std::process::Output, String> {
    Command::new(command)
        .args(args)
        .output()
        .map_err(|error| format!("could not run {command}: {error}"))
}

fn act(harness: &str, removing: bool) -> Outcome {
    let mut notes = Vec::new();
    let marker = installed_marker(harness);
    if marker.is_none() {
        return Outcome {
            harness: harness.to_string(),
            ok: false,
            notes: vec![
                "no measured MCP registration path in this repository — not touched".to_string(),
            ],
        };
    }
    if !marker.as_ref().is_some_and(|path| path.exists()) {
        return Outcome {
            harness: harness.to_string(),
            ok: false,
            notes: vec![format!(
                "not installed (no {})",
                marker
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default()
            )],
        };
    }

    match harness {
        "pi" => {
            let path = pi_config_path();
            let existing = match read_json_or_empty(&path) {
                Ok(value) => value,
                Err(error) => {
                    return Outcome {
                        harness: harness.to_string(),
                        ok: false,
                        notes: vec![format!("{error} — fix or move {} first", path.display())],
                    }
                }
            };
            let next = if removing {
                pi_config_without(&existing)
            } else {
                pi_config_with(&existing, &pi_entry())
            };
            let mut text = match serde_json::to_string_pretty(&next) {
                Ok(text) => text,
                Err(error) => {
                    return Outcome {
                        harness: harness.to_string(),
                        ok: false,
                        notes: vec![error.to_string()],
                    }
                }
            };
            text.push('\n');
            match write_atomic(&path, &text) {
                Ok(()) => {
                    let what = if removing { "removed from" } else { "wrote" };
                    notes.push(format!("{what} {}", path.display()));
                    Outcome {
                        harness: harness.to_string(),
                        ok: true,
                        notes,
                    }
                }
                Err(error) => Outcome {
                    harness: harness.to_string(),
                    ok: false,
                    notes: vec![format!("could not write {}: {error}", path.display())],
                },
            }
        }
        "claude" => {
            let args = if removing {
                claude_remove_args()
            } else {
                claude_add_args()
            };
            match run("claude", &args) {
                Err(error) => Outcome {
                    harness: harness.to_string(),
                    ok: false,
                    notes: vec![error],
                },
                Ok(output) => {
                    let combined = format!(
                        "{}{}",
                        String::from_utf8_lossy(&output.stdout),
                        String::from_utf8_lossy(&output.stderr)
                    );
                    let combined = combined.trim();
                    if output.status.success() {
                        notes.push(if combined.is_empty() {
                            format!("claude {} ok", args[..3].join(" "))
                        } else {
                            combined.to_string()
                        });
                        Outcome {
                            harness: harness.to_string(),
                            ok: true,
                            notes,
                        }
                    } else {
                        notes.push(format!("claude {} failed: {combined}", args[..3].join(" ")));
                        Outcome {
                            harness: harness.to_string(),
                            ok: false,
                            notes,
                        }
                    }
                }
            }
        }
        other => Outcome {
            harness: other.to_string(),
            ok: false,
            notes: vec!["unknown harness".to_string()],
        },
    }
}

const USAGE: &str = "\
tools/sjel-mcp — Sjel's capabilities as an MCP server, and its registration in a harness.

  sjel mcp                                            the stdio MCP server itself
  tools/sjel-mcp/sjel-mcp register [<harness>...]     pi and Claude Code, or the named ones
  tools/sjel-mcp/sjel-mcp unregister [<harness>...]

Each registration is followed by a real MCP handshake against the server, which reports the
tool count it answered with. Restart a harness session, or /reload it, to pick the server up.

Codex has no measured registration path in this repository and is reported, not guessed at.
";

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "-h" || arg == "--help") {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    match args.first().map(String::as_str) {
        // No verb is the server itself: `sjel mcp` from a harness, and the handshake this tool's
        // registration performs below.
        None | Some("serve") => {
            server::serve();
            ExitCode::SUCCESS
        }
        Some("register" | "unregister") => registration(&args),
        Some(_) => {
            eprint!("{USAGE}");
            ExitCode::from(1)
        }
    }
}

fn registration(args: &[String]) -> ExitCode {
    let (verb, names) = match args.split_first() {
        Some((verb, rest)) => (verb.clone(), rest.to_vec()),
        None => return ExitCode::from(1),
    };
    let removing = verb == "unregister";

    // No names means the two harnesses this tool has a measured path for, whether or not they
    // are installed: an uninstalled one reports that, which is more useful than being skipped
    // silently. Codex is only reached when it is named.
    let targets: Vec<String> = if names.is_empty() {
        vec!["pi".to_string(), "claude".to_string()]
    } else {
        names
    };

    let outcomes: Vec<Outcome> = targets
        .iter()
        .map(|harness| act(harness, removing))
        .collect();

    let check = if removing { None } else { Some(handshake()) };

    let mut failed = false;
    for outcome in &outcomes {
        println!("{} {}", if outcome.ok { "✓" } else { "✗" }, outcome.harness);
        for note in &outcome.notes {
            println!("    {note}");
        }
        if !outcome.ok {
            failed = true;
        }
    }

    match check {
        Some(Ok(count)) => {
            println!("\n✓ server verified: {count} tools over stdio");
            println!("  Restart a harness session, or /reload it, to pick the server up.");
        }
        Some(Err(error)) => {
            println!("\n✗ server did not answer MCP: {error}");
            failed = true;
        }
        None => println!("\nNothing to verify after a removal."),
    }

    ExitCode::from(if failed { 1 } else { 0 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_entry_carries_the_timeout_pi_cannot_set() {
        let entry = pi_entry();
        assert_eq!(entry["timeout"], json!(180));
        assert_eq!(entry["exposure"], json!("codemode"));
    }

    #[test]
    fn the_entry_starts_the_command_this_tool_verifies() {
        let entry = pi_entry();
        let from_entry: Vec<String> =
            std::iter::once(entry["command"].as_str().unwrap().to_string())
                .chain(
                    entry["args"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|value| value.as_str().unwrap().to_string()),
                )
                .collect();
        assert_eq!(from_entry, server_command());
    }

    #[test]
    fn pi_config_keeps_what_it_does_not_own() {
        let before = json!({
            "autoEnableCodemode": false,
            "mcpServers": {"other": {"command": "other", "args": []}}
        });
        let after = pi_config_with(&before, &pi_entry());
        assert_eq!(after["autoEnableCodemode"], json!(false));
        assert!(after["mcpServers"]["other"].is_object());
        assert!(after["mcpServers"][SERVER_NAME].is_object());
        assert!(
            before["mcpServers"][SERVER_NAME].is_null(),
            "input is untouched"
        );
    }

    #[test]
    fn registering_twice_is_the_same_document() {
        let once = pi_config_with(&Value::Object(Map::new()), &pi_entry());
        let twice = pi_config_with(&once, &pi_entry());
        assert_eq!(once, twice);
    }

    #[test]
    fn unregister_removes_only_our_entry() {
        let two = pi_config_with(
            &json!({"mcpServers": {"other": {"command": "other"}}}),
            &pi_entry(),
        );
        let after = pi_config_without(&two);
        assert_eq!(after["mcpServers"].as_object().unwrap().len(), 1);
        assert!(after["mcpServers"]["other"].is_object());

        let lone = pi_config_with(&Value::Object(Map::new()), &pi_entry());
        assert!(pi_config_without(&lone).get("mcpServers").is_none());
    }

    #[test]
    fn claude_registration_names_the_server_scope_and_command() {
        let args = claude_add_args();
        assert_eq!(&args[..3], &["mcp", "add", SERVER_NAME]);
        let separator = args
            .iter()
            .position(|arg| arg == "--")
            .expect("-- separator");
        assert_eq!(&args[separator + 1..], server_command().as_slice());
        assert_eq!(
            claude_remove_args(),
            vec!["mcp", "remove", SERVER_NAME, "--scope", "user"]
        );
    }

    #[test]
    fn an_unknown_harness_has_no_marker_and_is_never_touched() {
        assert!(installed_marker("codex").is_none());
        let outcome = act("codex", false);
        assert!(!outcome.ok);
        assert!(outcome.notes[0].contains("no measured MCP registration path"));
    }
}
