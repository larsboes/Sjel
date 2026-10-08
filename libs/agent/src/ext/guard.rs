//! The secrets guard (F2): the one extension on by default, and the only exception to
//! "off until named" (D2, D3).
//!
//! A port of `Packs/security/extensions/secrets-guard.ts`, its `tool_call` half only. That file
//! also registers `vault_exec` and `vault_keys` and rewrites `tool_result` content; both need
//! things this core does not have yet — a tool-result hook and those two tools — and `ISA.md`
//! records them as not yet specified rather than quietly porting half of them. What it does here
//! is stop the read at the tool-call boundary, which is the half that decides whether a
//! credential can reach a model at all.
//!
//! The design is that file's design, and worth restating: an enumerated blocklist of readers
//! (`cat`, `head`, `awk`, `python3 -c`, `base64`, `tar`, `git show` …) loses to whichever one
//! nobody listed, and shell indirection is unbounded. So any reference to a secret path is
//! refused unless the command is one of the allowlisted shapes below. Over-extraction costs a
//! blocked command with a reason the caller can act on; under-extraction costs the secret.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use regex::{Captures, Regex};
use serde_json::{json, Value};
use sjel_agent::{Extension, Tool, ToolCall, Verdict};

/// The name `agent.toml` and `--ext` use. `main.rs` puts this one in the default set.
pub const NAME: &str = "guard";

pub struct Guard {
    stop: Arc<AtomicBool>,
}

impl Guard {
    /// `stop` reaches the tools this extension carries, so a Ctrl-C ends a `vault_exec` the way
    /// it ends a `bash` call.
    pub fn new(stop: &Arc<AtomicBool>) -> Self {
        Self {
            stop: Arc::clone(stop),
        }
    }
}

impl Extension for Guard {
    fn name(&self) -> &'static str {
        NAME
    }

    /// The other half of the ported file: the two tools that make a refusal actionable, because
    /// a guard that only says no leaves a run with no way to do legitimate work that needs a
    /// credential. Being tools rather than hooks, they need nothing the core has not got.
    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![
            Box::new(VaultExec(Arc::clone(&self.stop))),
            Box::new(VaultKeys),
        ]
    }

    fn tool_call(&self, call: &ToolCall) -> Verdict {
        // A call whose arguments are not JSON never reaches a tool: the loop answers it with the
        // parse error before `run` is consulted. There is nothing to block, so pass it and let
        // the loop say so.
        let Ok(args) = serde_json::from_str::<Value>(&call.function.arguments) else {
            return Verdict::Allow;
        };
        let arg = |key: &str| args.get(key).and_then(Value::as_str).unwrap_or_default();
        match call.function.name.as_str() {
            "read" => read_gate(arg("path")),
            "edit" => edit_gate(arg("path")),
            "grep" | "find" => search_gate(
                &call.function.name,
                arg("path"),
                arg("pattern"),
                arg("glob"),
            ),
            "bash" => command_gate(arg("command")),
            // `write` is untouched on purpose: writing an env file is how one gets set up, and
            // it is an allow case in the ported header.
            _ => Verdict::Allow,
        }
    }
}

/// Every secret-looking path the command mentions.
///
/// Generous on purpose — the inversion below is what uses it. Splitting on shell metacharacters
/// is what makes the embedded cases land: in `python3 -c "open('.env').read()"` and
/// `git show HEAD:.env` the split still yields `.env`, which `is_secret_path` matches alone.
fn secret_paths_in(command: &str) -> Vec<String> {
    const SPLIT: &str = "'\"`|;&()<>=,:[]{} $\\";
    let mut found: Vec<String> = Vec::new();
    for token in command.split(|c: char| c.is_whitespace() || SPLIT.contains(c)) {
        if token.len() < 3 || !is_secret_path(token) || found.iter().any(|f| f == token) {
            continue;
        }
        found.push(token.to_owned());
    }
    found
}

fn is_secret_path(path: &str) -> bool {
    let normalized = path.replace('\\', "/");
    SECRET_FILE_PATTERNS.iter().any(|p| p.is_match(&normalized))
}

/// Which patterns matched, for a reason the caller can act on. `None` when none did.
fn secret_reason(path: &str) -> Option<String> {
    let normalized = path.replace('\\', "/");
    let matched: Vec<&str> = SECRET_FILE_PATTERNS
        .iter()
        .filter(|p| p.is_match(&normalized))
        .map(Regex::as_str)
        .collect();
    (!matched.is_empty()).then(|| matched.join(", "))
}

/// Whether a command only NAMES a secret path rather than reading it.
///
/// `printf 'env_file: .env\n' > compose.yml` writes a service definition that references the
/// file; it never opens it. Refusing that would obstruct ordinary config work and protect
/// nothing. The exemption is deliberately narrow, because quoting alone proves nothing — an
/// inline script reads a quoted path (`python3 -c "open('.env').read()"`). It needs both a
/// quoted path and a verb that only ever writes text, and `grep -n TOKEN .env` still blocks
/// because there the path is bare.
fn only_mentions_secret_path(command: &str, probe: &str) -> bool {
    let first = command
        .trim()
        .split(|c: char| c.is_whitespace() || ";|&(".contains(c))
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if first != "printf" && first != "echo" {
        return false;
    }
    QUOTED_ENV.is_match(probe)
}

/// The reason a `bash` command would leak a secret, or `None` when it is allowed.
fn secret_read(command: &str) -> Option<String> {
    // The command as it should be judged: namespaces that merely contain "env" are blanked out
    // first, so `rg process.env` is not a read of `.env`.
    let probe = ENV_NAMESPACE.replace_all(command, "ENV_NAMESPACE${1}");

    // `source file` loads vars into the environment without printing them.
    if SOURCES_ENV.is_match(command) {
        return BASH_PATTERNS
            .iter()
            .find(|p| p.is_match(&probe))
            .map(|p| format!("matches secret-read pattern: {}", p.as_str()));
    }

    if EXPORT_ASSIGN.is_match(command)
        || BW_UNLOCK.is_match(command)
        || BW_HARMLESS.is_match(command)
    {
        return None;
    }

    if let Some(p) = BASH_PATTERNS.iter().find(|p| p.is_match(&probe)) {
        return Some(format!("matches secret-read pattern: {}", p.as_str()));
    }

    // The inversion. Run against `probe`, not `command`: `process.env` ends in `.env` and would
    // otherwise be refused as a secret file, which is what ENV_NAMESPACE exists to prevent.
    let referenced = secret_paths_in(&probe);
    if referenced.is_empty() || only_mentions_secret_path(command, &probe) {
        return None;
    }
    Some(format!(
        "references a secret file ({}). The known readers are blocked by name, but the reader \
         here was not one of them, so the command is refused rather than allowed. If this is a \
         false positive, rephrase so the path is not named — or use `source <file> && <command>`, \
         which the guard allows",
        referenced.join(", ")
    ))
}

fn read_gate(path: &str) -> Verdict {
    let Some(patterns) = secret_reason(path) else {
        return Verdict::Allow;
    };
    Verdict::Deny(format!(
        "secrets-guard: reading {path} would expose secrets ({patterns}). Run the command as \
         `source {path} && <command>` in bash instead: the guard allows it, and the values stay \
         in that command's environment instead of arriving here."
    ))
}

fn edit_gate(path: &str) -> Verdict {
    let Some(patterns) = secret_reason(path) else {
        return Verdict::Allow;
    };
    Verdict::Deny(format!(
        "secrets-guard: editing {path} is blocked — env files contain secrets ({patterns})."
    ))
}

fn search_gate(tool: &str, path: &str, pattern: &str, glob: &str) -> Verdict {
    if !path.is_empty() {
        if let Some(patterns) = secret_reason(path) {
            return Verdict::Deny(format!(
                "secrets-guard: {tool} on {path} could expose secrets ({patterns})."
            ));
        }
    }
    let needle = format!("{pattern} {glob}");
    if !needle.trim().is_empty() && SECRET_FILE_PATTERNS.iter().any(|p| p.is_match(&needle)) {
        return Verdict::Deny(format!(
            "secrets-guard: {tool} pattern or glob targets secret files ({}).",
            needle.trim()
        ));
    }
    Verdict::Allow
}

fn command_gate(command: &str) -> Verdict {
    match secret_read(command) {
        Some(reason) => Verdict::Deny(format!(
            "secrets-guard: blocked — {reason}. To run a command that needs a credential, use \
             `source .env && <command>`: the shell reads the file, and the value never has to be \
             printed for the command to use it."
        )),
        None => Verdict::Allow,
    }
}

// ---- the tools ----------------------------------------------------------

const VAULT_TIMEOUT: u64 = 30;
const VAULT_MAX_TIMEOUT: u64 = 600;

/// Run a command with an env file loaded, and keep the values out of the result.
struct VaultExec(Arc<AtomicBool>);

impl Tool for VaultExec {
    fn name(&self) -> &'static str {
        "vault_exec"
    }
    fn description(&self) -> &'static str {
        "Run a shell command with an env file sourced. The file is read here, not by the model, \
         and every value it loaded is replaced with **** in the result. Pass `keys` to put only \
         the variables the command needs into its environment: without it the whole environment \
         and every variable in the file are inherited. Use it for curl, API calls, or any command \
         that needs a credential — the command can use the value, and neither you nor the \
         transcript ever hold it."
    }
    fn parameters(&self) -> Value {
        json!({ "type": "object", "required": ["cmd"], "properties": {
            "cmd": { "type": "string", "description": "The shell command to run, e.g. curl -s https://api.example.com/endpoint" },
            "env_file": { "type": "string", "description": "Env file to source first, e.g. .env or config/secrets.env. Read here; the model never sees the values." },
            "keys": { "type": "array", "items": { "type": "string" }, "description": "Only these variables, plus PATH and HOME, exist in the command's environment, so it has nothing else to leak. Prefer setting it." },
            "timeout": { "type": "integer", "description": "Seconds. Default 30, maximum 600." }
        }})
    }
    fn run(&self, args: &Value) -> Result<String, String> {
        let cmd = str_arg(args, "cmd")?;
        let timeout = args
            .get("timeout")
            .and_then(Value::as_u64)
            .unwrap_or(VAULT_TIMEOUT)
            .clamp(1, VAULT_MAX_TIMEOUT);
        let env_file = args
            .get("env_file")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty());
        // An empty `keys` is not a scope of zero variables, it is no scope at all — the reading
        // the ported file takes of the same argument.
        let wanted: Option<Vec<String>> = args
            .get("keys")
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .filter(|keys: &Vec<String>| !keys.is_empty());

        let mut command = Command::new("bash");
        command.arg("-c").arg(cmd);
        if wanted.is_some() {
            // Nothing but what was asked for. One `printenv | base64` is then enough to defeat
            // the literal stripping below, so the environment is what has to be small.
            command.env_clear();
            for name in ["PATH", "HOME"] {
                if let Some(value) = std::env::var_os(name) {
                    command.env(name, value);
                }
            }
        }

        let mut exposed: Vec<String> = Vec::new();
        if let Some(file) = env_file {
            let vars = load_env_file(file)?;
            let has = |key: &str| vars.iter().any(|(name, _)| name == key);
            if let Some(keys) = &wanted {
                let absent: Vec<&str> = keys
                    .iter()
                    .map(String::as_str)
                    .filter(|key| !has(key))
                    .collect();
                if !absent.is_empty() {
                    let found: Vec<&str> = keys
                        .iter()
                        .map(String::as_str)
                        .filter(|key| has(key))
                        .collect();
                    return Err(format!(
                        "vault_exec: {} not present in {file}{}",
                        absent.join(", "),
                        if found.is_empty() {
                            String::new()
                        } else {
                            format!(" (found: {})", found.join(", "))
                        }
                    ));
                }
            }
            for (name, value) in &vars {
                if wanted.as_ref().is_some_and(|keys| !keys.contains(name)) {
                    continue;
                }
                command.env(name, value);
                exposed.push(value.clone());
            }
        }

        let out = sjel_agent::tools::run_bounded(command, Duration::from_secs(timeout), &self.0)?;
        Ok(scrub(&out, &exposed))
    }
}

/// List the names in an env file, never the values.
struct VaultKeys;

impl Tool for VaultKeys {
    fn name(&self) -> &'static str {
        "vault_keys"
    }
    fn description(&self) -> &'static str {
        "List the variable names in an env file — never the values — so a command can be written \
         for the ones that exist without any of them being exposed."
    }
    fn parameters(&self) -> Value {
        json!({ "type": "object", "required": ["env_file"], "properties": {
            "env_file": { "type": "string", "description": "Path to the env file, e.g. .env or config/secrets.env" }
        }})
    }
    fn run(&self, args: &Value) -> Result<String, String> {
        let file = str_arg(args, "env_file")?;
        let vars = load_env_file(file)?;
        let mut names: Vec<&str> = vars.iter().map(|(name, _)| name.as_str()).collect();
        names.sort_unstable();
        let listed: Vec<String> = names.iter().map(|name| format!("  {name}")).collect();
        Ok(format!(
            "Keys in {file} ({} vars):\n{}",
            names.len(),
            listed.join("\n")
        ))
    }
}

fn str_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("`{key}` is required"))
}

/// A relative env file is relative to the working directory, as every other path in a tool call
/// is.
fn resolve_env_path(cwd: &Path, given: &str) -> PathBuf {
    let path = Path::new(given);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}

/// The `KEY=value` pairs in an env file. Values leave this function only into the environment of
/// the command that asked for them.
fn load_env_file(given: &str) -> Result<Vec<(String, String)>, String> {
    let cwd = std::env::current_dir().map_err(|e| format!("no working directory: {e}"))?;
    let path = resolve_env_path(&cwd, given);
    let body = std::fs::read_to_string(&path)
        .map_err(|_| format!("env file not found: {}", path.display()))?;
    let mut vars = Vec::new();
    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some((name, value)) = trimmed.split_once('=') else {
            continue;
        };
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        // `KEY="x"` and `KEY='x'` mean `x`, not the quotes.
        let value = value.trim();
        let value = value
            .strip_prefix('"')
            .and_then(|inner| inner.strip_suffix('"'))
            .or_else(|| {
                value
                    .strip_prefix('\'')
                    .and_then(|inner| inner.strip_suffix('\''))
            })
            .unwrap_or(value);
        vars.push((name.to_owned(), value.to_owned()));
    }
    Ok(vars)
}

/// Three passes, because each one alone has a hole: the shape of a `KEY=value` line, a long
/// random string, and the values this call actually put into the environment — the last one is
/// what catches a value printed on its own, which no shape check can see.
fn scrub(text: &str, exposed: &[String]) -> String {
    let mut out = ASSIGNMENT
        .replace_all(text, |caps: &Captures<'_>| {
            let name = &caps[2];
            if SECRET_VAR_PATTERNS.iter().any(|p| p.is_match(name)) {
                // A group that did not participate has no match, and indexing it panics —
                // `export` is optional, so it is absent most of the time.
                let export = caps.get(1).map_or("", |m| m.as_str());
                format!("{export}{name}=****")
            } else {
                caps[0].to_owned()
            }
        })
        .into_owned();
    if LONG_RANDOM.is_match(&out) {
        out = LONG_RANDOM.replace_all(&out, "****").into_owned();
    }
    for value in exposed.iter().filter(|value| value.len() >= 8) {
        if out.contains(value.as_str()) {
            out = out.replace(value.as_str(), "****");
        }
    }
    out
}

/// A whole line that assigns a long value, so the value can be replaced without touching the
/// name it belongs to.
static ASSIGNMENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?m)^(export\s+)?([A-Za-z_][A-Za-z0-9_]*)=['"]?[^\s'"]{8,}['"]?$"#)
        .expect("literal pattern")
});

/// Env-var names that hold secrets, for the shape pass above. A name alone is weaker evidence
/// than a value, which is why the by-value pass exists as well.
///
/// The first ten are the ported file's list, kept as it is; the last three are added here, and
/// this is a divergence from that file rather than a port. On 2026-10-08 this operator's own
/// shell environment carried `BW_SESSION`, a live Bitwarden session token, and none of the
/// ported patterns matches that name — a `printenv` through an unscoped `vault_exec`, or
/// `echo $BW_SESSION` in bash, reached the transcript with the token intact. The Bitwarden CLI
/// sets its own `BW_` namespace, and a session token is exactly the kind of value this list is
/// for.
static SECRET_VAR_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    compile(&[
        r"(?i)TOKEN",
        r"(?i)SECRET",
        r"(?i)PASSWORD",
        r"(?i)PASS(_|$)",
        r"(?i)API_KEY",
        r"(?i)APIKEY",
        r"(?i)CREDENTIAL",
        r"(?i)AUTH",
        r"(?i)_KEY$",
        r"(?i)_ID$",
        r"(?i)^BW_",
        r"(?i)_SESSION$",
        r"(?i)PASSPHRASE",
    ])
});

/// OAuth refresh tokens and the like: long enough that nothing else in a shell transcript is.
static LONG_RANDOM: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[A-Za-z0-9_-]{300,}={0,2}").expect("literal pattern"));

/// File path patterns that contain secrets — blocked from `read` and `edit`, and anywhere a
/// command names one.
static SECRET_FILE_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    compile(&[
        r"\.env$",
        r"\.env\.[a-zA-Z0-9_.-]+$",
        r"\.secrets?$",
        r"\.secrets?\.[a-zA-Z0-9_.-]+$",
        r"(?i)credentials?\.",
        r"\.pem$",
        r"\.key$",
        r"\.p12$",
        r"\.pfx$",
        r"(?i)token\.",
        r"(?i)secret\.",
        r"(^|/)\.netrc$",
        r"(^|/)\.git-credentials$",
        r"(^|/)config/git/credentials$",
        r"(?i)kubeconfig",
        r"(^|/)\.ssh/id_",
    ])
});

/// Bash commands or patterns that leak secrets.
static BASH_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    compile(&[
        r"\b(cat|bat|less|more|head|tail|nl|tac|rev)\s+.*\.env",
        r"\b(cat|bat|less|more|head|tail|nl|tac|rev)\s+.*\.secrets?",
        r"\b(grep|rg|ag|ack|find|sed)\s+.*\.env",
        r"\b(grep|rg|ag|ack|find|sed)\s+.*\.secrets?",
        r"\b(wc|sort|uniq)\s+.*\.env",
        r"\b(wc|sort|uniq)\s+.*\.secrets?",
        r"\bbw\s+get\b",
        r"\bbw\s+list\b",
        r"\bbw\s+sync\b",
        r"\bbw\s+export\b",
        r"\becho\s+\$[A-Z_]*(?:TOKEN|SECRET|KEY|PASSWORD|PASS|CREDENTIAL|API_KEY|SECRET_|SESSION|PASSPHRASE)\w*",
        r"\bprintenv\b",
        // A bare `env` dump: `env` at a command position, optionally with flags, then a pipe, a
        // redirect, a separator or the end. Anchored this way the word "env" inside
        // `process.env.X` or `env_file:` is not a dump, while `env`, `env | grep`, `env > f`,
        // `x; env` and `$(env)` are.
        r"(?m)(^|[;&|()`]\s*|\bsudo\s+)env(\s+-{1,2}[\w-]+)*\s*($|[|>;&)`)])",
        r"\bopenssl\s+(?:pkey|rsa|ec|dsa)\s+.*-in\s+.*\.(?:pem|key)",
    ])
});

/// Tokens that look like a secret file but name a variable namespace in code.
static ENV_NAMESPACE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(?:process|import\.meta|Deno|Bun)\.env\b|\bos\.environ\b|\$env:([A-Za-z_])")
        .expect("ENV_NAMESPACE is a literal pattern")
});

static SOURCES_ENV: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(\s*source\s+|\s*\.\s+)[^\n&|;]*\.(env|secrets?)\b")
        .expect("SOURCES_ENV is a literal pattern")
});
static EXPORT_ASSIGN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*export\s+[A-Za-z_][A-Za-z0-9_]*=").expect("literal pattern"));
static BW_UNLOCK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*bw\s+unlock\b").expect("literal pattern"));
static BW_HARMLESS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*bw\s+(encode|generate)\b").expect("literal pattern"));
static QUOTED_ENV: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"['"][^'"]*\.(?:env|secrets?)(?:\.[A-Za-z0-9_.-]+)?[^'"]*['"]"#)
        .expect("literal pattern")
});

fn compile(patterns: &[&str]) -> Vec<Regex> {
    patterns
        .iter()
        .map(|p| Regex::new(p).unwrap_or_else(|e| panic!("pattern `{p}` does not compile: {e}")))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use sjel_agent::FunctionCall;

    fn verdict(tool: &str, arguments: Value) -> Verdict {
        guard().tool_call(&ToolCall {
            id: "c".into(),
            kind: "function".into(),
            function: FunctionCall {
                name: tool.into(),
                arguments: arguments.to_string(),
            },
        })
    }

    /// Every case the ported file's header lists, with the verdict it gives there (AGT-10).
    ///
    /// The header is the spec: "what is blocked" and "what is allowed" are its two lists, and a
    /// case that behaves differently here would be a port that quietly widened or narrowed the
    /// guard. `true` is a block.
    /// One row of the header's table: what the case is, whether it must be blocked, the tool it
    /// is asked of, and how to build that tool's arguments.
    type Case = (&'static str, bool, &'static str, fn() -> Value);

    #[test]
    fn every_case_in_the_header_gets_the_same_verdict() {
        let cases: &[Case] = &[
            // read — *.env, *.secrets, *credentials*, *.pem, *.key, ~/.netrc,
            // ~/.git-credentials, the git credential store, kubeconfig, ~/.ssh/id_*
            ("read .env", true, "read", || json_path(".env")),
            ("read .env.local", true, "read", || json_path(".env.local")),
            ("read .env.production", true, "read", || {
                json_path(".env.production")
            }),
            ("read .secrets", true, "read", || json_path(".secrets")),
            ("read .secrets.staging", true, "read", || {
                json_path(".secrets.staging")
            }),
            ("read credentials.json", true, "read", || {
                json_path("config/credentials.json")
            }),
            ("read server.pem", true, "read", || {
                json_path("certs/server.pem")
            }),
            ("read private.key", true, "read", || {
                json_path("private.key")
            }),
            ("read bundle.p12", true, "read", || json_path("bundle.p12")),
            ("read token.json", true, "read", || json_path("token.json")),
            ("read ~/.netrc", true, "read", || json_path("~/.netrc")),
            ("read ~/.git-credentials", true, "read", || {
                json_path("~/.git-credentials")
            }),
            ("read the git credential store", true, "read", || {
                json_path("config/git/credentials")
            }),
            ("read kubeconfig", true, "read", || {
                json_path("~/.kube/kubeconfig")
            }),
            ("read a private ssh key", true, "read", || {
                json_path("~/.ssh/id_ed25519")
            }),
            ("read an ordinary file", false, "read", || {
                json_path("src/lib.rs")
            }),
            // edit — any edit targeting a secret path
            ("edit .env", true, "edit", || json_path(".env")),
            ("edit an ordinary file", false, "edit", || {
                json_path("src/lib.rs")
            }),
            // ls/grep/find — a secret path, or a pattern/glob that targets one
            ("grep on .env", true, "grep", || json_path(".env")),
            (
                "find glob *.env",
                true,
                "find",
                || json!({ "glob": "*.env" }),
            ),
            (
                "grep for a pattern in an ordinary tree",
                false,
                "grep",
                || json!({ "pattern": "needle", "path": "src" }),
            ),
            (
                "find an ordinary glob",
                false,
                "find",
                || json!({ "glob": "*.rs" }),
            ),
            // bash — cat/grep/head/tail/less/more/nl/wc on those patterns
            ("cat .env", true, "bash", || json_cmd("cat .env")),
            ("grep TOKEN .env", true, "bash", || {
                json_cmd("grep TOKEN .env")
            }),
            ("head -5 .secrets", true, "bash", || {
                json_cmd("head -5 .secrets")
            }),
            ("tail -f .env", true, "bash", || json_cmd("tail -f .env")),
            ("less .env.local", true, "bash", || {
                json_cmd("less .env.local")
            }),
            ("nl .env", true, "bash", || json_cmd("nl .env")),
            ("wc -l .env", true, "bash", || json_cmd("wc -l .env")),
            ("type .env", true, "bash", || json_cmd("type .env")),
            // bash — Bitwarden reads
            ("bw get password github", true, "bash", || {
                json_cmd("bw get password github")
            }),
            ("bw list items", true, "bash", || json_cmd("bw list items")),
            ("bw sync", true, "bash", || json_cmd("bw sync")),
            ("bw export", true, "bash", || {
                json_cmd("bw export --format json")
            }),
            // bash — echoing a secret-looking variable
            ("echo $API_TOKEN", true, "bash", || {
                json_cmd("echo $API_TOKEN")
            }),
            ("echo $GITHUB_SECRET", true, "bash", || {
                json_cmd("echo $GITHUB_SECRET")
            }),
            // Not cases in the ported file: its `echo` looks for TOKEN, SECRET, KEY, PASSWORD and
            // CREDENTIAL, and this operator's shell carries a live session token under a name
            // with none of them. See SECRET_VAR_PATTERNS for the same find on the shape pass.
            ("echo $BW_SESSION", true, "bash", || {
                json_cmd("echo $BW_SESSION")
            }),
            ("echo $SSH_PASSPHRASE", true, "bash", || {
                json_cmd("echo $SSH_PASSPHRASE")
            }),
            // bash — dumping the environment
            ("env", true, "bash", || json_cmd("env")),
            ("env | grep SECRET", true, "bash", || {
                json_cmd("env | grep SECRET")
            }),
            ("env > /tmp/e", true, "bash", || json_cmd("env > /tmp/e")),
            ("x; env", true, "bash", || json_cmd("x; env")),
            ("printenv", true, "bash", || json_cmd("printenv")),
            // bash — openssl reading a private key
            ("openssl rsa -in server.pem", true, "bash", || {
                json_cmd("openssl rsa -in server.pem -noout")
            }),
            // bash — readers nobody listed, caught by the inversion
            ("python3 -c \"open('.env').read()\"", true, "bash", || {
                json_cmd("python3 -c \"open('.env').read()\"")
            }),
            ("base64 .env", true, "bash", || json_cmd("base64 .env")),
            ("tar czf /tmp/x .env", true, "bash", || {
                json_cmd("tar czf /tmp/x .env")
            }),
            ("git show HEAD:.env", true, "bash", || {
                json_cmd("git show HEAD:.env")
            }),
            ("cp .env /tmp/", true, "bash", || json_cmd("cp .env /tmp/")),
            // bash — allowed: sourcing, exporting, unlocking, writing a reference
            ("source .env && npm test", false, "bash", || {
                json_cmd("source .env && npm test")
            }),
            (". .env && npm test", false, "bash", || {
                json_cmd(". .env && npm test")
            }),
            ("source config/secrets.env && run", false, "bash", || {
                json_cmd("source config/secrets.env && run")
            }),
            ("bw unlock", false, "bash", || json_cmd("bw unlock")),
            ("bw encode", false, "bash", || json_cmd("bw encode 'x'")),
            ("bw generate", false, "bash", || {
                json_cmd("bw generate -uln")
            }),
            ("export API_TOKEN=abc", false, "bash", || {
                json_cmd("export API_TOKEN=abc")
            }),
            (
                "printf 'env_file: .env' > compose.yml",
                false,
                "bash",
                || json_cmd("printf 'env_file: .env\\n' > compose.yml"),
            ),
            (
                "echo 'env_file: .env' >> compose.yml",
                false,
                "bash",
                || json_cmd("echo 'env_file: .env' >> compose.yml"),
            ),
            ("rg process.env", false, "bash", || {
                json_cmd("rg process.env")
            }),
            ("grep -n 'import.meta.env' src/", false, "bash", || {
                json_cmd("grep -n 'import.meta.env' src/")
            }),
            ("cargo test", false, "bash", || json_cmd("cargo test")),
            // write — writing an env file is how one gets set up
            ("write .env", false, "write", || json_path(".env")),
        ];

        let mut wrong: Vec<String> = Vec::new();
        for (what, blocked, tool, arguments) in cases {
            let got = verdict(tool, arguments());
            let blocked_here = matches!(got, Verdict::Deny(_));
            if blocked_here != *blocked {
                wrong.push(format!("{what}: expected {blocked}, got {got:?}"));
            }
        }
        assert!(
            wrong.is_empty(),
            "{} case(s) disagree with the header:\n{}",
            wrong.len(),
            wrong.join("\n")
        );
    }

    #[test]
    fn the_guard_covers_every_core_tool_that_names_a_path_or_a_command() {
        // A core tool that takes a `path` or a `command` and is not one of the guarded names is
        // a hole with no test behind it: the model reaches a secret path through a tool the
        // guard has never heard of. `write` takes a path and is deliberately not guarded, so it
        // is the one name this list has to keep saying no to on purpose.
        let root = std::env::temp_dir();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let unguarded_on_purpose = ["write"];
        for tool in sjel_agent::tools::coding(&root, &stop) {
            let schema = tool.parameters().to_string();
            if !schema.contains("\"path\"") && !schema.contains("\"command\"") {
                continue;
            }
            let seen = matches!(tool.name(), "read" | "edit" | "grep" | "find" | "bash")
                || unguarded_on_purpose.contains(&tool.name());
            assert!(
                seen,
                "the core tool `{}` takes a path or a command and the guard does not look at it",
                tool.name()
            );
        }
    }

    #[test]
    fn a_denial_says_which_pattern_matched_and_what_to_do_instead() {
        let Verdict::Deny(reason) = verdict("read", json_path(".env.production")) else {
            panic!("reading .env.production was allowed")
        };
        assert!(reason.contains(".env.production"), "{reason}");
        assert!(
            reason.contains(r"\.env\."),
            "the matched pattern is not named: {reason}"
        );
        assert!(reason.contains("source"), "no way out is offered: {reason}");

        let Verdict::Deny(reason) = verdict("bash", json_cmd("base64 .env")) else {
            panic!("base64 .env was allowed")
        };
        assert!(reason.contains("references a secret file"), "{reason}");
    }

    fn json_path(path: &str) -> Value {
        json!({ "path": path })
    }

    fn json_cmd(command: &str) -> Value {
        json!({ "command": command })
    }

    fn guard() -> Guard {
        Guard::new(&std::sync::Arc::new(AtomicBool::new(false)))
    }

    fn tool(name: &str) -> Box<dyn Tool> {
        guard()
            .tools()
            .into_iter()
            .find(|tool| tool.name() == name)
            .unwrap_or_else(|| panic!("the guard does not carry `{name}`"))
    }

    /// An env file in a directory of its own, so the tests in one process do not collide.
    fn fixture(name: &str) -> String {
        let dir =
            std::env::temp_dir().join(format!("sjel-agent-vault-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("secrets.env");
        std::fs::write(
            &path,
            "DEMO_TOKEN=demo-token-value-1234567890\n\
             OTHER_SECRET=other-secret-value-0987654321\n\
             PLAIN=plain-value-long-enough\n",
        )
        .unwrap();
        path.to_string_lossy().into_owned()
    }

    // ---- vault_exec and vault_keys (F2b) --------------------------------

    #[test]
    fn a_scoped_run_gets_only_the_named_variables() {
        let out = tool("vault_exec")
            .run(&json!({
                "cmd": "printenv | sort",
                "env_file": fixture("scoped"),
                "keys": ["DEMO_TOKEN"],
            }))
            .unwrap();
        assert!(out.contains("DEMO_TOKEN="), "{out}");
        assert!(
            !out.contains("demo-token-value"),
            "the value survived: {out}"
        );
        assert!(
            !out.contains("OTHER_SECRET"),
            "a variable outside the scope was in the environment: {out}"
        );
        assert!(out.contains("PATH="), "no usable environment: {out}");
    }

    #[test]
    fn an_unscoped_run_inherits_the_whole_file() {
        // `printenv NAME` alone would print a bare value, and a bare value is stripped by value —
        // so the assignment shape is what shows the variable was in the environment at all.
        // An unscoped run also inherits this process's environment, which is why no test here
        // dumps `printenv` wholesale: that is how a test log ends up holding a live token out of
        // the operator's shell.
        let out = tool("vault_exec")
            .run(&json!({
                "cmd": "printf 'OTHER_SECRET=%s\\n' \"$OTHER_SECRET\"",
                "env_file": fixture("unscoped"),
            }))
            .unwrap();
        assert!(out.contains("OTHER_SECRET=****"), "{out}");
    }

    #[test]
    fn a_benign_name_keeps_its_value() {
        // The shape pass is a name list, not "redact everything": a long value under a name that
        // is not secret-shaped and did not come from an env file stays readable.
        let out = tool("vault_exec")
            .run(&json!({ "cmd": "printf 'PLAIN=%s\\n' some-literal-value-here" }))
            .unwrap();
        assert!(out.contains("PLAIN=some-literal-value-here"), "{out}");
    }

    #[test]
    fn a_session_token_is_stripped_even_though_the_ported_list_does_not_name_it() {
        // Found on 2026-10-08 in this operator's own shell: the inherited environment holds
        // `BW_SESSION`, a live Bitwarden session token, and the ported name list matches none of
        // it. `echo $BW_SESSION` in bash passes the gate too — the gate's `echo` pattern looks
        // for TOKEN, SECRET, KEY, PASSWORD, CREDENTIAL, none of which is in that name.
        let stripped = scrub(
            "BW_SESSION=06XyuSxTMwmdxWLOouAY+IGhVRvo9ayozoNofOM5gR4z7E1PfC4oSbo6\n",
            &[],
        );
        assert!(!stripped.contains("06Xyu"), "{stripped}");
        assert!(stripped.contains("BW_SESSION=****"), "{stripped}");
        let passphrase = scrub("SSH_PASSPHRASE=correct-horse-battery-staple\n", &[]);
        assert!(passphrase.contains("SSH_PASSPHRASE=****"), "{passphrase}");
    }

    #[test]
    fn a_value_printed_on_its_own_is_still_stripped() {
        let out = tool("vault_exec")
            .run(&json!({ "cmd": "printenv DEMO_TOKEN", "env_file": fixture("bare") }))
            .unwrap();
        assert!(!out.contains("demo-token-value"), "{out}");
        assert!(out.contains("****"), "{out}");
    }

    #[test]
    fn a_missing_key_is_named_with_the_ones_that_are_there() {
        // "found" is the shortlist among the keys that were asked for, as in the ported file:
        // what a caller needs is which of the names it asked for are there, not a directory.
        let err = tool("vault_exec")
            .run(&json!({
                "cmd": "true",
                "env_file": fixture("absent"),
                "keys": ["DEMO_TOKEN", "ABSENT_TOKEN"],
            }))
            .unwrap_err();
        assert!(err.contains("ABSENT_TOKEN not present in"), "{err}");
        assert!(err.contains("found: DEMO_TOKEN"), "{err}");
    }

    #[test]
    fn a_missing_env_file_is_named() {
        let err = tool("vault_exec")
            .run(&json!({ "cmd": "true", "env_file": "/nonexistent/nope.env" }))
            .unwrap_err();
        assert!(err.contains("env file not found"), "{err}");
        assert!(err.contains("/nonexistent/nope.env"), "{err}");
    }

    #[test]
    fn the_timeout_kills_the_command() {
        let start = std::time::Instant::now();
        let out = tool("vault_exec")
            .run(&json!({ "cmd": "sleep 30", "timeout": 1 }))
            .unwrap();
        assert!(out.contains("killed after 1 s"), "{out}");
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "the timeout did not fire: {:?}",
            start.elapsed()
        );
    }

    #[test]
    fn vault_keys_lists_names_and_never_values() {
        let out = tool("vault_keys")
            .run(&json!({ "env_file": fixture("keys") }))
            .unwrap();
        for name in ["DEMO_TOKEN", "OTHER_SECRET", "PLAIN"] {
            assert!(out.contains(name), "{name} is missing from: {out}");
        }
        for value in ["demo-token-value", "other-secret-value", "plain-value"] {
            assert!(!out.contains(value), "a value reached the result: {out}");
        }
        assert!(out.contains("3 vars"), "{out}");
    }

    #[test]
    fn a_relative_env_path_resolves_against_the_working_directory() {
        let cwd = Path::new("/tmp/somewhere");
        assert_eq!(
            resolve_env_path(cwd, ".env"),
            PathBuf::from("/tmp/somewhere/.env")
        );
        assert_eq!(
            resolve_env_path(cwd, "config/secrets.env"),
            PathBuf::from("/tmp/somewhere/config/secrets.env")
        );
        assert_eq!(
            resolve_env_path(cwd, "/etc/x.env"),
            PathBuf::from("/etc/x.env")
        );
    }
}
