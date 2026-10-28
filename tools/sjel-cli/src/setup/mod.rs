//! `sjel setup` — what this machine still needs, and what applying it would do.
//!
//! The knowledge of "what this deployment needs" was spread over five tools that never spoke to
//! each other: `tools/install.sh` bootstraps a host, `<overlay>/config/machine.toml` records the
//! per-machine facts, `tools/capability.sh` owns the enabled set, `tools/harnesses` deploys Packs
//! and `tools/doctor` reports what is broken. This is the one place that turns those facts into a
//! proposal: probes (what is true now), steps (what would change), verify (how we know it worked).
//!
//! Three rules, and they are the whole contract:
//!
//! - **Plan is the default and writes nothing.** `sjel setup plan` is a report. A machine that
//!   is not being changed cannot be broken by asking what it needs.
//! - **Apply needs `--confirm`.** It runs only the steps marked `apply`, and every one of them is
//!   idempotent: the second run reports "already set, untouched" instead of rewriting.
//! - **Every step in a plan is one this tool runs.** That is a constraint, not an accident. The
//!   first version of this listed steps for a person to run — add a cargo feature here, add an env
//!   line there, restart that — and a setup surface that prints commands has moved the work back
//!   to the reader. Those two steps were removed the same day by making them Sjel's job (the
//!   feature became a default, and the runner began passing the deployment env to every process).
//!   A component that genuinely cannot take a step needs a decision about where that step lives,
//!   not a `manual` label.
//!
//! Verify is the part that makes the rest honest: it asks the primary, not the config file,
//! whether the credential works.

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde::Serialize;

use crate::paths::Paths;

mod mint;
mod rotation;
mod turso;

const HELP: &str = "\
sjel setup — what this machine still needs, and what applying it would do.

`sjel setup` with no argument lists the components and their one-line state.

  sjel setup plan [component] [--json]
      Probe this machine and print what would change. Writes nothing; this is the default.

  sjel setup apply [component] --confirm [--url <url>] [--interval-secs <n>] [--token-stdin]
                   [--remint] [--mint-days <n>] [--host-here]
      Run every step in the plan, then verify. Without --confirm it prints the plan and exits 2.
      --token-stdin reads a credential from stdin and stores it in the private overlay; it is
      never echoed, never logged and never written to the tracked tree. Use it for a primary
      somebody else runs.
      With neither a submitted nor a recorded credential, the credential is MINTED here: the
      signing key lives in the Keychain, its public half is written to the overlay for
      `sqld --auth-jwt-key-file`, and the token is written like any other setting. --remint mints
      a fresh one anyway; --mint-days sets its lifetime (default 30).
      --host-here hosts the primary on THIS machine: install the pinned sqld binary, record where
      its data and its public key live, enable the capability and start it. The URL defaults to
      the manifest's port on loopback. Without it, nothing here starts a server.

  sjel setup verify [component] [--json]
      Ask the primary itself whether the credential works. Exits 1 when a check fails.

  sjel setup rotate turso --confirm
      Replace the signing key of the primary this machine hosts. A person, at a terminal, typing a
      confirmation phrase: every token signed by the old key stops working and every client needs
      reprovisioning. Refuses unless the primary is stopped, has no boot persistence loaded and no
      unfinished rotation is recorded. Proves the new key against a disposable sqld, then removes
      the inhibit marker; on any failure the marker stays and this exits nonzero.

  sjel setup rotate recover
      Report an unfinished rotation from its journal and the public key file. Reads files only,
      clears nothing, and stays inhibited.

Components are `turso` (embedded replicas of a remote Turso primary). A step with a kind is gone
rather than kept for a case nothing produces: every step in a plan is a write this tool makes, so
there is nothing here to copy into a shell. It edits no Cargo.toml — the replica engine is a
default feature — and installs no capability.";

/// How a probe came out. Ordered by how much attention it asks for when a list is sorted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum Mark {
    /// True right now.
    Ok,
    /// Absent, and the plan has a step for it.
    Missing,
    /// True, but not the way the plan wants it.
    Warn,
    /// Cannot be satisfied by this tool: a human or a source change is needed.
    Blocked,
}

impl Mark {
    fn glyph(self) -> char {
        match self {
            Mark::Ok => '✓',
            Mark::Missing => '✗',
            Mark::Warn => '!',
            Mark::Blocked => '·',
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct Probe {
    id: &'static str,
    label: String,
    mark: Mark,
    detail: String,
}

#[derive(Debug, Clone, Serialize)]
struct Step {
    label: String,
    detail: String,
    /// True when the probes say this is already done, so `apply` skips it.
    done: bool,
}

#[derive(Debug, Clone, Serialize)]
struct Plan {
    component: &'static str,
    purpose: &'static str,
    probes: Vec<Probe>,
    steps: Vec<Step>,
}

/// The JSON a verb prints, so a UI reads one document per request instead of concatenated ones.
#[derive(Debug, Serialize)]
struct PlansJson {
    components: Vec<Plan>,
}

#[derive(Debug, Serialize)]
struct VerifiesJson {
    components: Vec<VerifyJson>,
}

#[derive(Debug, Serialize)]
struct VerifyJson {
    component: &'static str,
    probes: Vec<Probe>,
}

#[derive(Debug, Serialize)]
struct AppliesJson {
    components: Vec<ApplyJson>,
}

#[derive(Debug, Serialize)]
struct ApplyJson {
    component: &'static str,
    applied: Vec<String>,
    verify: Vec<Probe>,
}

/// The overlay's deployment-wide settings file, which this tool owns.
///
/// It is the `KEY=value` body `sjel_config::deployment_value` reads, in the private overlay rather
/// than the tracked tree. Read and written here, never printed: it holds the primary's credential.
struct EnvFile {
    path: PathBuf,
    body: String,
}

impl EnvFile {
    fn read(path: PathBuf) -> Self {
        let body = std::fs::read_to_string(&path).unwrap_or_default();
        Self { path, body }
    }

    /// The value of `key`, or `None` when the line is absent or empty.
    fn value(&self, key: &str) -> Option<String> {
        sjel_config::deployment_value(&self.body, key)
    }

    /// Set `key=value`, replacing an existing assignment in place and appending otherwise.
    ///
    /// Answers whether the file changed, which is what makes a second `apply` a no-op instead of a
    /// rewrite: an unchanged file is not touched, so its mtime and any comments beside it survive.
    fn set(&mut self, key: &str, value: &str) -> bool {
        if self.value(key).as_deref() == Some(value) {
            return false;
        }
        let prefix = format!("{key}=");
        let mut replaced = false;
        let mut out: Vec<String> = Vec::with_capacity(self.body.lines().count() + 1);
        for line in self.body.lines() {
            if line.starts_with(&prefix) {
                if replaced {
                    // A duplicate line is a person's mistake and this tool's to collapse: which of
                    // two values wins is not something a reader should have to know.
                    continue;
                }
                out.push(format!("{prefix}{value}"));
                replaced = true;
            } else {
                out.push(line.to_owned());
            }
        }
        if !replaced {
            out.push(format!("{prefix}{value}"));
        }
        let mut body = out.join("\n");
        body.push('\n');
        self.body = body;
        true
    }

    /// Write the body, creating the file's directory. Answers whether anything was written.
    fn write(&self) -> std::io::Result<bool> {
        if self.path.is_file() && std::fs::read_to_string(&self.path)? == self.body {
            return Ok(false);
        }
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&self.path, &self.body)?;
        Ok(true)
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

/// Where the deployment-wide settings live, when an overlay is resolved at all.
fn deployment_env(paths: &Paths) -> EnvFile {
    let path = paths
        .overlay_root
        .as_ref()
        .map(|root| root.join("config").join("deployment.env"))
        .unwrap_or_else(|| PathBuf::from("deployment.env"));
    EnvFile::read(path)
}

/// One target a plan can be produced for. One entry today, because one component has a contract;
/// the shape is what a second one follows.
#[derive(Debug)]
struct Component {
    name: &'static str,
    plan: fn(&Paths, &EnvFile) -> Plan,
    verify: fn(&Paths, &EnvFile) -> Vec<Probe>,
}

const COMPONENTS: &[Component] = &[Component {
    name: "turso",
    plan: turso::plan,
    verify: turso::verify,
}];

fn component(name: &str) -> Result<&'static Component, String> {
    COMPONENTS.iter().find(|c| c.name == name).ok_or_else(|| {
        let known: Vec<&str> = COMPONENTS.iter().map(|c| c.name).collect();
        format!(
            "sjel setup: no component '{name}' — known: {}",
            known.join(", ")
        )
    })
}

pub fn run(rest: &[String]) -> ExitCode {
    let mut words: Vec<String> = Vec::new();
    let mut json = false;
    let mut confirm = false;
    let mut url: Option<String> = None;
    let mut interval_secs: Option<u64> = None;
    let mut token_stdin = false;
    let mut mint_days: Option<u64> = None;
    let mut remint = false;
    let mut host_here = false;

    let mut iter = rest.iter().peekable();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{HELP}");
                return ExitCode::SUCCESS;
            }
            "--json" => json = true,
            "--confirm" => confirm = true,
            "--token-stdin" => token_stdin = true,
            "--remint" => remint = true,
            "--host-here" => host_here = true,
            "--mint-days" => match iter.next().map(|v| v.parse::<u64>()) {
                Some(Ok(days)) if days > 0 => mint_days = Some(days),
                _ => return usage("--mint-days needs a whole number of days, at least 1"),
            },
            "--url" => match iter.next() {
                Some(value) => url = Some(value.clone()),
                None => return usage("--url needs a value"),
            },
            "--interval-secs" => match iter.next().map(|v| v.parse::<u64>()) {
                Some(Ok(secs)) if secs > 0 => interval_secs = Some(secs),
                Some(Ok(_)) => return usage("--interval-secs must be greater than zero"),
                _ => return usage("--interval-secs needs a whole number of seconds"),
            },
            other if other.starts_with('-') => {
                return usage(&format!("unknown flag '{other}'"));
            }
            other => words.push(other.to_owned()),
        }
    }

    let verb = words.first().map_or("", String::as_str);
    let named = words.get(1).map(String::as_str);
    let paths = match Paths::from_env() {
        Ok(paths) => paths,
        Err(message) => return usage(&message),
    };
    let env = deployment_env(&paths);

    match verb {
        // The default is the report, so a bare `sjel setup` cannot change anything.
        "" | "plan" => {
            let chosen = match selection(named) {
                Ok(c) => c,
                Err(message) => return usage(&message),
            };
            let plans: Vec<Plan> = chosen.iter().map(|c| (c.plan)(&paths, &env)).collect();
            if json {
                print_json(&PlansJson {
                    components: plans.clone(),
                });
            } else {
                for plan in &plans {
                    print_plan(plan);
                }
                println!(
                    "Dry run: nothing was written. `sjel setup apply <component> --confirm` runs the \
                     {} step(s) still open.",
                    plans
                        .iter()
                        .flat_map(|p| &p.steps)
                        .filter(|s| !s.done)
                        .count()
                );
            }
            ExitCode::SUCCESS
        }
        "apply" => {
            let chosen = match selection(named) {
                Ok(c) => c,
                Err(message) => return usage(&message),
            };
            let options = turso::Options {
                url: url.as_deref(),
                interval_secs,
                mint_days: mint_days.unwrap_or(mint::DEFAULT_MINT_DAYS),
                remint,
                host_here,
            };
            if !confirm {
                for c in &chosen {
                    print_plan(&(c.plan)(&paths, &env));
                }
                eprintln!(
                    "sjel setup apply: --confirm is required. Nothing was written; the plan above is \
                     what it would run."
                );
                return ExitCode::from(2);
            }
            let token = if token_stdin {
                match read_stdin_secret() {
                    Ok(secret) => Some(secret),
                    Err(message) => {
                        eprintln!("sjel setup apply: {message}");
                        return ExitCode::from(2);
                    }
                }
            } else {
                None
            };
            let mut failed = false;
            let mut applies: Vec<ApplyJson> = Vec::new();
            for c in &chosen {
                if c.name != "turso" {
                    eprintln!("sjel setup apply: {} has no apply path", c.name);
                    failed = true;
                    continue;
                }
                match turso::apply(&paths, &env, &options, token.as_deref()) {
                    Ok(summary) => {
                        if !json {
                            for line in &summary.lines {
                                println!("{line}");
                            }
                        }
                        // Verify after every apply: an apply that cannot show its effect is a hope.
                        // Re-read the file, because the snapshot above predates the write — a
                        // verification against the old body reports the setting it just stored as
                        // absent, which is the one failure mode worse than no verification.
                        let written = deployment_env(&paths);
                        let probes = (c.verify)(&paths, &written);
                        if !json {
                            println!("\n{} — verification", c.name);
                            print_probes(&probes);
                        }
                        failed |= probes.iter().any(|p| p.mark != Mark::Ok);
                        applies.push(ApplyJson {
                            component: c.name,
                            applied: summary.lines,
                            verify: probes,
                        });
                    }
                    Err(message) => {
                        eprintln!("sjel setup apply {}: {message}", c.name);
                        failed = true;
                    }
                }
            }
            if json {
                print_json(&AppliesJson {
                    components: applies,
                });
            }
            if failed {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            }
        }
        "verify" => {
            let chosen = match selection(named) {
                Ok(c) => c,
                Err(message) => return usage(&message),
            };
            let mut bad = false;
            let mut components: Vec<VerifyJson> = Vec::new();
            for c in &chosen {
                let probes = (c.verify)(&paths, &env);
                if !json {
                    println!("{} — verification", c.name);
                    print_probes(&probes);
                }
                bad |= probes.iter().any(|p| p.mark != Mark::Ok);
                components.push(VerifyJson {
                    component: c.name,
                    probes,
                });
            }
            if json {
                print_json(&VerifiesJson { components });
            }
            if bad {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            }
        }
        // Rotation is its own surface, not a component verb: it answers for the signing key rather
        // than for a plan, and it is the one verb here that a person has to be at to run.
        "rotate" => {
            let only_confirm = !json
                && url.is_none()
                && interval_secs.is_none()
                && !token_stdin
                && mint_days.is_none()
                && !remint
                && !host_here;
            if !only_confirm {
                return usage(
                    "rotate takes --confirm alone: it is not a plan and it applies no settings",
                );
            }
            rotation::run(&words[1..], confirm)
        }
        other => usage(&format!("unknown verb '{other}'")),
    }
}

/// The components a verb applies to: the named one, or all of them.
fn selection(named: Option<&str>) -> Result<Vec<&'static Component>, String> {
    match named {
        Some(name) => Ok(vec![component(name)?]),
        None => Ok(COMPONENTS.iter().collect()),
    }
}

/// A credential, read from stdin with the terminal's echo left alone (the caller pipes it).
fn read_stdin_secret() -> Result<String, String> {
    let mut raw = String::new();
    std::io::stdin()
        .read_to_string(&mut raw)
        .map_err(|e| format!("cannot read the token from stdin: {e}"))?;
    let token = raw.trim();
    if token.is_empty() {
        return Err("--token-stdin was passed and stdin held no token".to_owned());
    }
    if token.chars().any(char::is_whitespace) {
        return Err("the token on stdin contains whitespace — expected one token".to_owned());
    }
    Ok(token.to_owned())
}

fn usage(message: &str) -> ExitCode {
    eprintln!("{message}");
    eprintln!(
        "Usage: sjel setup [plan|apply|verify] [component] [--confirm] [--url <url>] \
         [--interval-secs <n>] [--token-stdin] [--json]"
    );
    eprintln!("       sjel setup rotate turso --confirm | recover");
    ExitCode::from(2)
}

fn print_json<T: Serialize>(value: &T) {
    match serde_json::to_string_pretty(value) {
        Ok(text) => println!("{text}"),
        Err(e) => eprintln!("sjel setup: cannot render JSON: {e}"),
    }
}

fn print_plan(plan: &Plan) {
    println!("{} — {}", plan.component, plan.purpose);
    if plan.probes.is_empty() && plan.steps.is_empty() {
        println!("  nothing to report");
        return;
    }
    if !plan.probes.is_empty() {
        println!("\nProbes");
        print_probes(&plan.probes);
    }
    if !plan.steps.is_empty() {
        println!("\nSteps");
        for step in &plan.steps {
            println!("  {} {}", if step.done { "✓" } else { "·" }, step.label);
            for line in step.detail.lines() {
                println!("      {line}");
            }
        }
    }
    println!();
}

fn print_probes(probes: &[Probe]) {
    let width = probes.iter().map(|p| p.label.len()).max().unwrap_or(0);
    for probe in probes {
        println!(
            "  {} {:<width$}  {}",
            probe.mark.glyph(),
            probe.label,
            probe.detail,
            width = width
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_with(body: &str) -> EnvFile {
        EnvFile {
            path: PathBuf::from("/tmp/unused-deployment.env"),
            body: body.to_owned(),
        }
    }

    #[test]
    fn a_missing_key_is_appended_once() {
        let mut env = env_with("SJEL_LAN_PORT=8443\n");
        assert!(env.set("SJEL_TURSO_URL", "libsql://primary.turso.io"));
        assert_eq!(
            env.body,
            "SJEL_LAN_PORT=8443\nSJEL_TURSO_URL=libsql://primary.turso.io\n"
        );
        // The second call is the no-op that makes apply idempotent.
        assert!(!env.set("SJEL_TURSO_URL", "libsql://primary.turso.io"));
    }

    #[test]
    fn an_existing_key_is_replaced_in_place() {
        let mut env = env_with("# the primary\nSJEL_TURSO_URL=libsql://old\nSJEL_LAN_PORT=8443\n");
        assert!(env.set("SJEL_TURSO_URL", "libsql://new"));
        assert_eq!(
            env.body,
            "# the primary\nSJEL_TURSO_URL=libsql://new\nSJEL_LAN_PORT=8443\n"
        );
        assert_eq!(env.value("SJEL_TURSO_URL").as_deref(), Some("libsql://new"));
    }

    #[test]
    fn a_duplicate_line_collapses_to_one() {
        let mut env = env_with("SJEL_TURSO_URL=libsql://a\nSJEL_TURSO_URL=libsql://b\n");
        assert!(env.set("SJEL_TURSO_URL", "libsql://c"));
        assert_eq!(env.body, "SJEL_TURSO_URL=libsql://c\n");
    }

    #[test]
    fn an_empty_value_reads_as_absent() {
        // The same rule `sjel_config::deployment_value` applies to every deployment setting: an
        // empty line is not a value, so setting it is a change.
        let mut env = env_with("SJEL_TURSO_URL=\n");
        assert_eq!(env.value("SJEL_TURSO_URL"), None);
        assert!(env.set("SJEL_TURSO_URL", "libsql://primary.turso.io"));
    }

    #[test]
    fn writing_an_unchanged_body_touches_nothing() {
        let dir = std::env::temp_dir().join(format!("sjel-setup-env-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("deployment.env");
        std::fs::write(&path, "SJEL_TURSO_URL=libsql://a\n").unwrap();

        let env = EnvFile::read(path.clone());
        assert!(
            !env.write().unwrap(),
            "an unchanged file must not be rewritten"
        );

        let mut changed = EnvFile::read(path.clone());
        changed.set("SJEL_TURSO_URL", "libsql://b");
        assert!(changed.write().unwrap());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "SJEL_TURSO_URL=libsql://b\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unknown_component_names_the_known_ones() {
        let err = component("postgres").unwrap_err();
        assert!(err.contains("turso"), "{err}");
    }

    #[test]
    fn a_zero_interval_is_refused_before_anything_runs() {
        let code = run(&[
            "apply".to_owned(),
            "turso".to_owned(),
            "--interval-secs".to_owned(),
            "0".to_owned(),
        ]);
        assert_eq!(format!("{code:?}"), format!("{:?}", ExitCode::from(2)));
    }
}
