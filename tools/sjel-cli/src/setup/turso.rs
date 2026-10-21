//! The `turso` component: follow a remote Turso (LibSQL) primary as a local replica.
//!
//! `libs/sjel-turso` gained embedded replicas on 2026-10-21 (`replica` feature). This is the
//! setup half of that: a deployment needs one URL, one credential and one pull interval, and a
//! capability needs to be built with the feature and to receive those three settings in its own
//! environment. Two of those four things this tool can write; the other two belong to files with
//! another owner, so they are printed with the exact path and line instead.
//!
//! The settings names are `sjel_turso::ReplicaConfig::from_env`'s, which is the reader. Renaming
//! one here without renaming it there would produce a setup that verifies and a process that does
//! not follow the primary, so the three constants are the interface.

use std::path::PathBuf;
use std::time::Duration;

use serde::Serialize;

use super::{EnvFile, Kind, Mark, Plan, Probe, Step};
use crate::paths::Paths;

/// The setting that names the primary, read by `sjel_turso::ReplicaConfig::from_env`.
const URL_KEY: &str = "SJEL_TURSO_URL";
/// The primary's credential, same reader.
const TOKEN_KEY: &str = "SJEL_TURSO_AUTH_TOKEN";
/// The pull interval in whole seconds, same reader.
const INTERVAL_KEY: &str = "SJEL_TURSO_SYNC_INTERVAL_SECS";

pub(super) const PURPOSE: &str = "follow a remote Turso primary as a local replica";

/// The default pull interval `sjel_turso::ReplicaConfig` uses when nothing is set.
const DEFAULT_INTERVAL_SECS: u64 = 60;

/// What `apply` was asked to write. Every field is optional: an apply with nothing to say is a
/// re-run that reports what is already true.
pub(crate) struct Options<'a> {
    pub url: Option<&'a str>,
    pub interval_secs: Option<u64>,
}

#[derive(Debug, Serialize)]
pub(crate) struct Applied {
    pub lines: Vec<String>,
}

/// How a capability would receive the three settings in its own environment, or why it cannot.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Channel {
    /// A process-kind capability: `<overlay>/config/machine.toml` `[capability.<name>] env`.
    MachineEnv,
    /// A container: its `service.toml` `env_file`, relative to the overlay.
    ManifestEnvFile(PathBuf),
    /// No manifest and no machine section, so nothing would read the settings.
    None,
}

/// A capability (or app) that links `sjel-turso` and could therefore follow the primary.
#[derive(Debug, Clone)]
struct Consumer {
    name: String,
    /// `sjel-turso = { …, features = ["replica"] }` is present in its Cargo.toml.
    replica_feature: bool,
    channel: Channel,
    enabled: bool,
}

/// Every crate in this checkout that depends on `sjel-turso`, with the two facts that decide
/// whether following a primary would do anything.
fn consumers(paths: &Paths) -> Vec<Consumer> {
    let enabled = enabled_capabilities(paths);
    let mut out = Vec::new();
    let mut roots: Vec<PathBuf> = vec![paths.caps_dir.clone()];
    if let Some(overlay_caps) = &paths.overlay_caps_dir {
        roots.push(overlay_caps.clone());
    }
    roots.push(paths.root.join("apps"));
    for root in roots {
        let Ok(entries) = std::fs::read_dir(&root) else {
            continue;
        };
        let mut names: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect();
        names.sort();
        for dir in names {
            let manifest = dir.join("Cargo.toml");
            let Ok(body) = std::fs::read_to_string(&manifest) else {
                continue;
            };
            let Some(replica_feature) = turso_dependency(&body) else {
                continue;
            };
            let name = dir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            out.push(Consumer {
                channel: channel(paths, &name),
                enabled: enabled.contains(&name),
                replica_feature,
                name,
            });
        }
    }
    out
}

/// `Some(has_replica_feature)` when `body` declares `sjel-turso`, `None` when it does not.
///
/// Reads the dependency line and the two that follow it, because `features = [...]` wraps when it
/// is long. What it must not do is match on a comment that mentions the crate: a line whose first
/// non-space character is `#` is skipped.
fn turso_dependency(body: &str) -> Option<bool> {
    let lines: Vec<&str> = body.lines().collect();
    for (index, line) in lines.iter().enumerate() {
        let code = line.split('#').next().unwrap_or("").trim();
        if !code.starts_with("sjel-turso") {
            continue;
        }
        let window = lines[index..lines.len().min(index + 3)].join(" ");
        let code_part = window.split('#').next().unwrap_or("");
        return Some(code_part.contains("\"replica\""));
    }
    None
}

/// How `name` would get the settings: the machine's `[capability.<name>] env` for a process, the
/// manifest's `env_file` for a container, nothing when neither exists.
fn channel(paths: &Paths, name: &str) -> Channel {
    let Some(manifest) = paths.manifest_for(name) else {
        // No manifest: this crate is a library or an app the shell does not start, so only a
        // process environment outside this deployment could carry the settings.
        return Channel::None;
    };
    let body = std::fs::read_to_string(&manifest).unwrap_or_default();
    let field = |key: &str| {
        body.lines()
            .map(|line| line.split('#').next().unwrap_or("").trim())
            .find_map(|line| line.strip_prefix(&format!("{key} = ")))
            .map(|value| value.trim().trim_matches('"').to_owned())
            .filter(|value| !value.is_empty())
    };
    match field("env_file") {
        Some(file) => Channel::ManifestEnvFile(
            paths
                .overlay_root
                .clone()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(file),
        ),
        // A process-kind capability, whose environment comes from machine.toml.
        None => Channel::MachineEnv,
    }
}

/// The enabled capability names from `<overlay>/config/machine.toml`.
fn enabled_capabilities(paths: &Paths) -> Vec<String> {
    let Some(path) = paths.machine_toml() else {
        return Vec::new();
    };
    let Ok(body) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Some(line) = body
        .lines()
        .find(|l| l.trim_start().starts_with("capabilities"))
    else {
        return Vec::new();
    };
    let Some((_, rest)) = line.split_once('=') else {
        return Vec::new();
    };
    rest.split('#')
        .next()
        .unwrap_or("")
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .map(|entry| entry.trim().trim_matches('"').to_owned())
        .filter(|entry| !entry.is_empty())
        .collect()
}

/// The resolved primary URL, from the deployment record or this process's environment.
fn url(env: &EnvFile) -> Option<String> {
    env.value(URL_KEY)
        .or_else(|| std::env::var(URL_KEY).ok().filter(|v| !v.trim().is_empty()))
        .map(|value| value.trim().to_owned())
}

/// The resolved credential. Read, never printed: every message about it says whether it is there.
fn token(env: &EnvFile) -> Option<String> {
    env.value(TOKEN_KEY)
        .or_else(|| {
            std::env::var(TOKEN_KEY)
                .ok()
                .filter(|v| !v.trim().is_empty())
        })
        .map(|value| value.trim().to_owned())
}

fn interval(env: &EnvFile) -> Option<u64> {
    env.value(INTERVAL_KEY)
        .or_else(|| std::env::var(INTERVAL_KEY).ok())
        .and_then(|value| value.trim().parse().ok())
        .filter(|secs| *secs > 0)
}

/// How this consumer's process would receive the settings, in one line for a step's detail.
fn channel_note(consumer: &Consumer) -> String {
    match &consumer.channel {
        Channel::MachineEnv => {
            format!("through [capability.{}] env in machine.toml", consumer.name)
        }
        Channel::ManifestEnvFile(path) => format!("through its env_file {}", path.display()),
        Channel::None => {
            "no manifest and no machine section, so nothing carries the settings yet".to_owned()
        }
    }
}

pub(super) fn plan(paths: &Paths, env: &EnvFile) -> Plan {
    let consumers = consumers(paths);
    let url = url(env);
    let token = token(env);
    let mut probes = Vec::new();
    let mut steps = Vec::new();

    probes.push(Probe {
        id: "record",
        label: "deployment record".to_owned(),
        mark: if paths.overlay_root.is_some() {
            Mark::Ok
        } else {
            Mark::Blocked
        },
        detail: match paths.overlay_root.as_ref() {
            Some(_) => env.path().display().to_string(),
            None => "no overlay resolved, so there is no private file to write".to_owned(),
        },
    });

    probes.push(Probe {
        id: "url",
        label: URL_KEY.to_owned(),
        mark: if url.is_some() {
            Mark::Ok
        } else {
            Mark::Missing
        },
        detail: match &url {
            Some(url) => format!("{url} (where a primary it is; verify says whether it answers)"),
            None => "no primary named, and no replica can be opened without one".to_owned(),
        },
    });

    probes.push(Probe {
        id: "token",
        label: TOKEN_KEY.to_owned(),
        mark: if token.is_some() {
            Mark::Ok
        } else {
            Mark::Missing
        },
        detail: match token.is_some() {
            true => "present (value never printed)".to_owned(),
            false => "absent — a token-protected primary answers 401 without it".to_owned(),
        },
    });

    probes.push(Probe {
        id: "interval",
        label: INTERVAL_KEY.to_owned(),
        mark: Mark::Ok,
        detail: format!(
            "{}s (the default when unset)",
            interval(env).unwrap_or(DEFAULT_INTERVAL_SECS)
        ),
    });

    let wired: Vec<&Consumer> = consumers.iter().filter(|c| c.enabled).collect();
    probes.push(Probe {
        id: "consumers",
        label: "consumers".to_owned(),
        mark: match wired.is_empty() {
            true => Mark::Warn,
            false => Mark::Ok,
        },
        detail: match consumers.is_empty() {
            true => "no crate in this checkout links sjel-turso".to_owned(),
            false => format!(
                "links sjel-turso: {}",
                consumers
                    .iter()
                    .map(|c| format!(
                        "{} ({}, {})",
                        c.name,
                        if c.enabled { "enabled" } else { "disabled" },
                        if c.replica_feature {
                            "replica feature on"
                        } else {
                            "replica feature off"
                        }
                    ))
                    .collect::<Vec<String>>()
                    .join("; ")
            ),
        },
    });

    // Steps. `apply` writes the two settings it owns, and nothing else.
    steps.push(Step {
        kind: Kind::Apply,
        label: format!("record {URL_KEY} in the deployment env"),
        detail: match (paths.overlay_root.is_some(), &url) {
            (false, _) => "blocked: no overlay resolved".to_owned(),
            (true, None) => "run: sjel setup apply turso --confirm --url <libsql://…>".to_string(),
            (true, Some(_)) => "already recorded, untouched".to_owned(),
        },
        done: url.is_some(),
    });

    steps.push(Step {
        kind: Kind::Apply,
        label: format!("store {TOKEN_KEY} from stdin in the deployment env"),
        detail: match token.is_some() {
            true => "already present, untouched".to_owned(),
            false => "run: sjel setup apply turso --confirm --token-stdin < token.txt".to_owned(),
        },
        done: token.is_some(),
    });

    let unbuilt: Vec<&Consumer> = consumers.iter().filter(|c| !c.replica_feature).collect();
    if !unbuilt.is_empty() {
        let channels: Vec<String> = unbuilt
            .iter()
            .map(|c| format!("{} ({})", c.name, channel_note(c)))
            .collect();
        steps.push(Step {
            kind: Kind::Blocked,
            label: "build a consumer with the replica feature".to_owned(),
            detail: format!(
                "add `features = [\"replica\"]` to the sjel-turso dependency in each Cargo.toml \
                 named here. Sjel does not edit a Cargo.toml: a feature is a build decision its \
                 owner makes.\nWired up afterwards: {}",
                channels.join("; ")
            ),
            done: false,
        });
    }

    if let Some(pending) = consumers
        .iter()
        .find(|c| c.enabled && c.channel == Channel::None)
    {
        steps.push(Step {
            kind: Kind::Blocked,
            label: "give an enabled consumer somewhere to read the settings".to_owned(),
            detail: format!(
                "{} is enabled but has no service.toml env_file and no [capability.{}] section, so \
                 nothing passes {URL_KEY} into its process",
                pending.name, pending.name
            ),
            done: false,
        });
    }

    for consumer in &consumers {
        let (kind, detail) = match &consumer.channel {
            Channel::MachineEnv => (
                Kind::Manual,
                format!(
                    "in <overlay>/config/machine.toml:\n[capability.{}]\nenv = [\"{URL_KEY}=…\", \
                     \"{TOKEN_KEY}=…\"]",
                    consumer.name
                ),
            ),
            Channel::ManifestEnvFile(path) => (
                Kind::Manual,
                format!(
                    "append {URL_KEY}, {TOKEN_KEY} and {INTERVAL_KEY} to {} (private, in the \
                     overlay, the same place its other runtime values live)",
                    path.display()
                ),
            ),
            Channel::None => continue,
        };
        if !consumer.replica_feature {
            // The feature decides whether the settings are read at all; the env line is only worth
            // stating once they would be.
            continue;
        }
        steps.push(Step {
            kind,
            label: format!("give {} the three settings", consumer.name),
            detail,
            done: false,
        });
        steps.push(Step {
            kind: Kind::Manual,
            label: format!("restart {}", consumer.name),
            detail: format!("tools/service-runner.sh restart {}", consumer.name),
            done: false,
        });
    }

    Plan {
        component: "turso",
        purpose: PURPOSE,
        probes,
        steps,
    }
}

/// Write what this tool owns, and report every step it left to a person.
///
/// Idempotent: a second run with the same arguments finds the values already there, writes
/// nothing and says so.
pub(crate) fn apply(
    paths: &Paths,
    env: &EnvFile,
    options: &Options<'_>,
    token: Option<&str>,
) -> Result<Applied, String> {
    let mut lines = Vec::new();

    if let Some(url) = options.url {
        let url = url.trim();
        if !(url.starts_with("libsql://")
            || url.starts_with("http://")
            || url.starts_with("https://"))
        {
            return Err(format!(
                "'{url}' names no primary: expected libsql://, http:// or https://"
            ));
        }
    }
    if options.interval_secs == Some(0) {
        return Err("--interval-secs must be greater than zero".to_owned());
    }

    // Build the new body first, then write it once: a failure part way through leaves the file as
    // it was instead of half-written.
    let mut next = EnvFile {
        path: env.path().to_path_buf(),
        body: env.body.clone(),
    };
    let mut changed = false;
    if let Some(url) = options.url {
        changed |= next.set(URL_KEY, url.trim());
        lines.push(format!(
            "{URL_KEY}: {}",
            if changed { "set" } else { "unchanged" }
        ));
    }
    if let Some(secs) = options.interval_secs {
        changed |= next.set(INTERVAL_KEY, &secs.to_string());
        lines.push(format!("{INTERVAL_KEY}: {secs}"));
    }
    if let Some(token) = token {
        if token.trim().is_empty() {
            return Err("the token on stdin was empty".to_owned());
        }
        changed |= next.set(TOKEN_KEY, token.trim());
        lines.push(format!("{TOKEN_KEY}: stored (value hidden)"));
    }
    if !changed {
        lines.push(format!(
            "{}: already up to date, untouched",
            env.path().display()
        ));
        return Ok(Applied { lines });
    }
    if paths.overlay_root.is_none() {
        return Err(format!(
            "no overlay resolved, so {} is not a private file — refusing to write a deployment \
             setting outside one",
            env.path().display()
        ));
    }
    next.write()
        .map_err(|e| format!("cannot write {}: {e}", env.path().display()))?;
    lines.insert(0, format!("{}: written", env.path().display()));

    for consumer in consumers(paths)
        .iter()
        .filter(|c| c.enabled && c.replica_feature)
    {
        lines.push(format!(
            "{}: settings reach it only after the step printed by `sjel setup plan turso`",
            consumer.name
        ));
    }
    lines.push("next: sjel setup verify turso".to_owned());
    Ok(Applied { lines })
}

/// Ask the primary itself whether the credential works.
///
/// This is the only part of the component that leaves the machine, and it is the only part whose
/// answer means anything: a config file that names a primary proves nothing about whether the
/// token is still valid.
pub(super) fn verify(paths: &Paths, env: &EnvFile) -> Vec<Probe> {
    let mut probes = Vec::new();
    let url = url(env);
    let token = token(env);

    probes.push(Probe {
        id: "record",
        label: "deployment record".to_owned(),
        mark: match (url.as_ref(), token.as_ref()) {
            (Some(_), Some(_)) => Mark::Ok,
            _ => Mark::Missing,
        },
        detail: format!(
            "{}: url {}, token {}",
            env.path().display(),
            if url.is_some() { "present" } else { "absent" },
            if token.is_some() { "present" } else { "absent" }
        ),
    });

    let primary = match (url.as_deref(), token.as_deref()) {
        (Some(url), Some(token)) => probe_primary(url, token),
        (Some(_), None) => Probe {
            id: "primary",
            label: "primary".to_owned(),
            mark: Mark::Missing,
            detail: "not probed: no token to present".to_owned(),
        },
        _ => Probe {
            id: "primary",
            label: "primary".to_owned(),
            mark: Mark::Missing,
            detail: "not probed: no primary named".to_owned(),
        },
    };
    probes.push(primary);

    let consumers = consumers(paths);
    let wired: Vec<&Consumer> = consumers
        .iter()
        .filter(|c| c.enabled && c.replica_feature)
        .collect();
    probes.push(Probe {
        id: "wiring",
        label: "consumer wiring".to_owned(),
        mark: match wired.is_empty() {
            true => Mark::Blocked,
            false => Mark::Ok,
        },
        detail: match wired.is_empty() {
            true => {
                "no enabled capability is built with the replica feature, so nothing follows the \
                 primary yet; the plan prints the two steps that change that"
                    .to_owned()
            }
            false => wired
                .iter()
                .map(|c| format!("{} ready to follow it", c.name))
                .collect::<Vec<String>>()
                .join("; "),
        },
    });

    probes
}

/// `GET <primary>/info` with the credential, which is the probe libsql itself performs when it
/// opens a replica. A 200 means the primary offers the sync protocol; 401/403 means the token is
/// wrong or revoked; anything else is reported as it came.
fn probe_primary(url: &str, token: &str) -> Probe {
    let base = url.trim_end_matches('/');
    let endpoint = match base.strip_prefix("libsql://") {
        Some(rest) => format!("https://{rest}"),
        None => base.to_owned(),
    };
    let info = format!("{endpoint}/info");
    let label = "primary".to_owned();

    let http = match sjel_http::client(
        sjel_http::Purpose::new("sjel-setup"),
        Duration::from_secs(10),
    ) {
        Ok(client) => client,
        Err(e) => {
            return Probe {
                id: "primary",
                label,
                mark: Mark::Warn,
                detail: format!("cannot build an HTTP client: {e}"),
            }
        }
    };

    let response = http.get(&info).bearer_auth(token).send();
    match response {
        Ok(response) => {
            let status = response.status();
            let mark = if status.is_success() {
                Mark::Ok
            } else if status == reqwest::StatusCode::UNAUTHORIZED
                || status == reqwest::StatusCode::FORBIDDEN
            {
                Mark::Blocked
            } else {
                Mark::Warn
            };
            let detail = match status.is_success() {
                true => format!(
                    "{info} answered {status} — the primary offers the sync protocol to this token"
                ),
                false => format!(
                    "{info} answered {status} — {}",
                    if mark == Mark::Blocked {
                        "the token was refused; rotate it with --token-stdin"
                    } else {
                        "check the URL and that the database exists"
                    }
                ),
            };
            Probe {
                id: "primary",
                label,
                mark,
                detail,
            }
        }
        Err(e) => Probe {
            id: "primary",
            label,
            mark: Mark::Warn,
            detail: format!("{info} did not answer: {e}"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dependency_line_without_the_feature_is_off() {
        let body = "[dependencies]\nsjel-turso = { path = \"../../libs/sjel-turso\" }\n";
        assert_eq!(turso_dependency(body), Some(false));
    }

    #[test]
    fn the_feature_is_found_on_its_own_line() {
        let body = "[dependencies]\nsjel-turso = { path = \"../../libs/sjel-turso\",\n    features = [\"replica\"] }\n";
        assert_eq!(turso_dependency(body), Some(true));
    }

    #[test]
    fn a_comment_about_the_crate_is_not_a_dependency() {
        let body =
            "# sjel-turso is the engine here, see the README\n[dependencies]\nserde = \"1\"\n";
        assert_eq!(turso_dependency(body), None);
    }

    #[test]
    fn the_settings_names_are_the_ones_the_reader_expects() {
        // `sjel_turso::ReplicaConfig::from_env` is the reader. A rename on either side would make
        // setup verify a primary that no process follows, so the three names are asserted here.
        assert_eq!(URL_KEY, "SJEL_TURSO_URL");
        assert_eq!(TOKEN_KEY, "SJEL_TURSO_AUTH_TOKEN");
        assert_eq!(INTERVAL_KEY, "SJEL_TURSO_SYNC_INTERVAL_SECS");
    }

    #[test]
    fn an_apply_with_a_bad_scheme_is_refused_before_writing() {
        let mut env = EnvFile {
            path: PathBuf::from("/tmp/does-not-matter.env"),
            body: String::new(),
        };
        let err = apply(
            &paths_stub(),
            &env,
            &Options {
                url: Some("axon-owner.turso.io"),
                interval_secs: None,
            },
            None,
        )
        .unwrap_err();
        assert!(err.contains("names no primary"), "{err}");
        assert_eq!(env.value(URL_KEY), None);
        env.body.push_str("untouched\n");
        assert!(env.body.contains("untouched"));
    }

    #[test]
    fn a_zero_interval_is_refused() {
        let env = EnvFile {
            path: PathBuf::from("/tmp/does-not-matter.env"),
            body: String::new(),
        };
        let err = apply(
            &paths_stub(),
            &env,
            &Options {
                url: None,
                interval_secs: Some(0),
            },
            None,
        )
        .unwrap_err();
        assert!(err.contains("greater than zero"), "{err}");
    }

    #[test]
    fn apply_records_the_settings_once_and_never_echoes_the_credential() {
        let dir = std::env::temp_dir().join(format!("sjel-setup-apply-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("config")).unwrap();
        let record = dir.join("config").join("deployment");
        let mut paths = paths_stub();
        paths.overlay_root = Some(dir.clone());
        let options = Options {
            url: Some("libsql://example.turso.io"),
            interval_secs: Some(120),
        };

        let env = EnvFile::read(record.clone());
        let first = apply(&paths, &env, &options, Some("primary-credential")).unwrap();
        assert!(first.lines[0].contains("written"), "{:?}", first.lines);
        let body = std::fs::read_to_string(&record).unwrap();
        assert!(
            body.contains("SJEL_TURSO_URL=libsql://example.turso.io"),
            "{body}"
        );
        assert!(body.contains("SJEL_TURSO_SYNC_INTERVAL_SECS=120"), "{body}");
        assert!(
            body.contains("SJEL_TURSO_AUTH_TOKEN=primary-credential"),
            "{body}"
        );
        // The report names the key and hides the value: this text goes to a terminal and a log.
        assert!(
            !first.lines.join("\n").contains("primary-credential"),
            "{:?}",
            first.lines
        );

        // The second run finds everything in place, so it is a no-op rather than a rewrite.
        let again = EnvFile::read(record.clone());
        let second = apply(&paths, &again, &options, Some("primary-credential")).unwrap();
        assert!(
            second
                .lines
                .iter()
                .any(|line| line.contains("already up to date")),
            "{:?}",
            second.lines
        );
        assert_eq!(std::fs::read_to_string(&record).unwrap(), body);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Paths for the argument checks, which never reach the filesystem.
    fn paths_stub() -> Paths {
        Paths {
            root: PathBuf::from("/nonexistent"),
            machine_toml: None,
            overlay_root: None,
            caps_dir: PathBuf::from("/nonexistent/capabilities"),
            overlay_caps_dir: None,
        }
    }
}
