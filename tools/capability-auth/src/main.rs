//! Prints the request header a caller needs to pass a capability's inbound gate, and enrolls
//! the agent identity.
//!
//! `sjel capability call` used to send no credential, so every gated route answered 403 to
//! an agent session. This binary is the one place the shell learns the token, and it hands
//! it over as a header line for `curl -H @<(...)` so the value never appears in `ps` output.
//!
//! An optional capability name selects that capability's own token where it has one. Only
//! comms does (`api_secret_file`); every other name gets the deployment-wide token.
//!
//! `--agent` prints the agent token's header instead, read from the login Keychain. That
//! token is read-only and every response to it is pseudonymized (ISA F9). `enroll` creates
//! it: see [`enroll`].
//!
//! Exit status carries the answer for callers that must not see the value:
//! `--check` prints `configured` or `absent` and exits 0 in both cases.

use std::io::Write;
use std::path::Path;
use std::process::{Command, ExitCode, Stdio};

use sha2::{Digest, Sha256};
use sjel_server::InboundAuth;

/// The Keychain item that holds the agent token.
const KEYCHAIN_SERVICE: &str = "sjel-agent-token";

/// The capability's own token first, then the deployment's, in the order comms' gate
/// resolves them (`InboundAuth::resolve`, `libs/sjel-server/src/auth.rs`).
fn bearer_for(capability: Option<&str>) -> Option<String> {
    let own = match capability {
        Some("comms") => sjel_server::comms_config_token(),
        _ => None,
    };
    InboundAuth::resolve(own).bearer_header()
}

/// The agent token, read from the login Keychain. `None` when it is not enrolled.
fn agent_bearer() -> Option<String> {
    let out = Command::new("security")
        .args(["find-generic-password", "-s", KEYCHAIN_SERVICE, "-w"])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let token = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (out.status.success() && !token.is_empty()).then(|| format!("Bearer {token}"))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn write_private(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(contents)
}

/// Creates (or rotates) the agent token.
///
/// 1. The pseudonym key, `<overlay>/secrets/agent-pseudonym.key`, is created once and kept:
///    rotating it would change every token an agent already holds.
/// 2. A new random token goes into the login Keychain. It is written through `security -i` on
///    stdin, so it never appears in an argument list.
/// 3. Its SHA-256 goes to `<overlay>/config/agent-token.sha256`. The server reads only that.
///
/// The token is never printed.
fn enroll() -> Result<(), String> {
    let root = sjel_config::overlay_root().ok_or("no overlay is configured")?;
    let key_path = root.join("secrets/agent-pseudonym.key");
    if !key_path.exists() {
        let mut key = [0u8; 32];
        getrandom::fill(&mut key).map_err(|e| format!("secure random source: {e}"))?;
        write_private(&key_path, &key).map_err(|e| format!("{}: {e}", key_path.display()))?;
        println!("created {}", key_path.display());
    }

    let mut raw = [0u8; 32];
    getrandom::fill(&mut raw).map_err(|e| format!("secure random source: {e}"))?;
    let token = hex(&raw);
    let account = std::env::var("USER").unwrap_or_else(|_| "sjel".into());
    let mut child = Command::new("security")
        .arg("-i")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("security: {e}"))?;
    child
        .stdin
        .take()
        .ok_or("security: no stdin")?
        .write_all(
            format!(
                "add-generic-password -U -a {account} -s {KEYCHAIN_SERVICE} -l \"Sjel agent token\" -w {token}\n"
            )
            .as_bytes(),
        )
        .map_err(|e| format!("security: {e}"))?;
    let out = child
        .wait_with_output()
        .map_err(|e| format!("security: {e}"))?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !out.status.success() || !stderr.trim().is_empty() {
        return Err(format!("the Keychain refused the token: {}", stderr.trim()));
    }
    if agent_bearer().as_deref() != Some(format!("Bearer {token}").as_str()) {
        return Err(
            "the Keychain item could not be read back; nothing was changed on the server side"
                .into(),
        );
    }

    let hash_path = root.join("config/agent-token.sha256");
    let digest = hex(&Sha256::digest(token.as_bytes()));
    write_private(&hash_path, format!("{digest}\n").as_bytes())
        .map_err(|e| format!("{}: {e}", hash_path.display()))?;
    println!("agent token stored in the login Keychain as '{KEYCHAIN_SERVICE}'");
    println!("wrote {}", hash_path.display());
    println!("restart each capability that admits agents to load it: comms");
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("enroll") {
        if args.len() > 1 {
            eprintln!("sjel-capability-auth: enroll takes no arguments");
            return ExitCode::from(2);
        }
        return match enroll() {
            Ok(()) => ExitCode::SUCCESS,
            Err(message) => {
                eprintln!("sjel-capability-auth: {message}");
                ExitCode::FAILURE
            }
        };
    }
    let mut check = false;
    let mut agent = false;
    let mut rest = Vec::new();
    for arg in &args {
        match arg.as_str() {
            "--check" => check = true,
            "--agent" => agent = true,
            other => rest.push(other),
        }
    }
    let capability = match rest.as_slice() {
        [] => None,
        [name] if !name.starts_with('-') => Some(*name),
        _ => {
            eprintln!(
                "sjel-capability-auth: usage: capability-auth [--check] [--agent] [<capability>] | enroll"
            );
            return ExitCode::from(2);
        }
    };
    // The agent token is one token for every capability; the name only matters for the full one.
    let bearer = if agent {
        agent_bearer()
    } else {
        bearer_for(capability)
    };
    if check {
        println!(
            "{}",
            if bearer.is_some() {
                "configured"
            } else {
                "absent"
            }
        );
        return ExitCode::SUCCESS;
    }
    match bearer {
        Some(value) => {
            println!("Authorization: {value}");
            ExitCode::SUCCESS
        }
        // Exit 3, not 0: an empty header line handed to curl is a malformed request,
        // and "no token declared" is the loopback-only deployment where none is needed.
        None => ExitCode::from(3),
    }
}
