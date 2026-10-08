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

use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;
use sjel_agent::{Extension, ToolCall, Verdict};

/// The name `agent.toml` and `--ext` use. `main.rs` puts this one in the default set.
pub const NAME: &str = "guard";

pub struct Guard;

impl Extension for Guard {
    fn name(&self) -> &'static str {
        NAME
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
        r"\becho\s+\$[A-Z_]*(?:TOKEN|SECRET|KEY|PASSWORD|PASS|CREDENTIAL|API_KEY|SECRET_)\w*",
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
        Guard.tool_call(&ToolCall {
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
}
