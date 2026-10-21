//! The `turso` component: follow a remote Turso (LibSQL) primary as a local replica.
//!
//! `libs/sjel-turso` carries embedded replicas as a default feature (decided 2026-10-21), and
//! `tools/service-runner.sh` passes the deployment's own settings into every process it starts.
//! Those two facts are what make this component configuration instead of source work: nothing here
//! edits a Cargo.toml, and a capability needs no per-capability wiring to see the three settings.
//!
//! So the whole component is "write two names into the private overlay, restart what follows the
//! primary, then ask the primary whether the credential works". A step that still needed a person
//! would be a defect in that sentence, not in the plan.
//!
//! The setting names are `sjel_turso::ReplicaConfig::from_env`'s, which is the reader. Renaming one
//! here without renaming it there would produce a setup that verifies and a process that does not
//! follow the primary, so the three constants are the interface.

use serde::Serialize;
use std::path::PathBuf;
use std::time::Duration;

use super::{EnvFile, Mark, Plan, Probe, Step};
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
    /// Lifetime of a credential this tool mints, in days.
    pub mint_days: u64,
    /// Mint a fresh credential even when one is already recorded.
    pub remint: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct Applied {
    pub lines: Vec<String>,
}

/// A capability that links `sjel-turso`, and so follows the primary once it restarts.
#[derive(Debug, Clone)]
struct Consumer {
    name: String,
    enabled: bool,
}

/// Every crate in this checkout that depends on `sjel-turso`.
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
        let mut dirs: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect();
        dirs.sort();
        for dir in dirs {
            let Ok(body) = std::fs::read_to_string(dir.join("Cargo.toml")) else {
                continue;
            };
            if !depends_on_turso(&body) {
                continue;
            }
            let name = dir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            out.push(Consumer {
                enabled: enabled.contains(&name),
                name,
            });
        }
    }
    out
}

/// Whether a Cargo.toml declares `sjel-turso`.
///
/// The feature list no longer matters — `replica` is a default feature of the crate — so this is a
/// presence check. It reads the dependency line and the line after it, because a long `features`
/// array wraps, and it skips a comment: a line whose first non-space character is `#` is prose.
fn depends_on_turso(body: &str) -> bool {
    let lines: Vec<&str> = body.lines().collect();
    lines.iter().enumerate().any(|(index, line)| {
        let code = line.split('#').next().unwrap_or("").trim();
        if !code.starts_with("sjel-turso") {
            return false;
        }
        let window = lines[index..lines.len().min(index + 2)].join(" ");
        window
            .split('#')
            .next()
            .unwrap_or("")
            .contains("sjel-turso")
    })
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
            Some(url) => url.clone(),
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
            false => {
                "absent — a primary that authenticates answers 401 without it. Apply mints one \
                 from this deployment's own signing key unless a credential is submitted"
                    .to_owned()
            }
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

    let enabled: Vec<&Consumer> = consumers.iter().filter(|c| c.enabled).collect();
    probes.push(Probe {
        id: "consumers",
        label: "followers".to_owned(),
        mark: match (consumers.is_empty(), enabled.is_empty()) {
            (true, _) => Mark::Warn,
            (false, true) => Mark::Warn,
            (false, false) => Mark::Ok,
        },
        detail: match consumers.is_empty() {
            true => "no crate in this checkout links sjel-turso".to_owned(),
            false => format!(
                "{}, and the runner passes the deployment env into each one's process",
                consumers
                    .iter()
                    .map(|c| format!(
                        "{} ({})",
                        c.name,
                        if c.enabled { "enabled" } else { "disabled" }
                    ))
                    .collect::<Vec<String>>()
                    .join("; ")
            ),
        },
    });

    // Everything below is `apply`: each step is a write this tool makes, and the plan is the only
    // place a reader sees it first.
    let nothing_to_write = url.is_some() && token.is_some();
    steps.push(Step {
        label: format!("record {URL_KEY} and {INTERVAL_KEY}"),
        detail: match (paths.overlay_root.is_some(), &url) {
            (false, _) => "blocked: no overlay resolved".to_owned(),
            (true, None) => format!("writes both names to {}", env.path().display()),
            (true, Some(_)) => "already recorded, untouched".to_owned(),
        },
        done: url.is_some(),
    });

    steps.push(Step {
        label: format!("store {TOKEN_KEY}"),
        detail: match token.is_some() {
            true => "already present, untouched".to_owned(),
            // Two routes reach this setting, and which one runs is the caller's choice: a
            // submitted credential is written as given (it belongs to somebody else's primary),
            // and otherwise one is minted from the key in the Keychain.
            false => format!(
                "minted from this deployment's signing key and written to {}; the value is never \
                 printed, and the public half goes beside it for sqld --auth-jwt-key-file. A \
                 submitted credential is used instead when one is piped in",
                env.path().display()
            ),
        },
        done: token.is_some(),
    });

    if !enabled.is_empty() {
        steps.push(Step {
            label: format!(
                "restart {} so it reads the settings",
                enabled
                    .iter()
                    .map(|c| c.name.as_str())
                    .collect::<Vec<&str>>()
                    .join(", ")
            ),
            detail: "each one restarts against the deployment env the runner now passes it"
                .to_owned(),
            done: nothing_to_write,
        });
    }

    Plan {
        component: "turso",
        purpose: PURPOSE,
        probes,
        steps,
    }
}

/// Write what this tool owns, restart what follows it, and report what happened.
///
/// Idempotent: a second run with the same arguments finds the values already there, writes
/// nothing, restarts nothing and says so.
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
    // One outcome per setting, from that setting's own write: a shared flag made the second key
    // report "set" because the first one had changed.
    let mut changed = false;
    // Provisioning a primary this deployment owns is the default when nothing else supplies a
    // credential: mint one from the Keychain key, and publish the public half where `sqld` reads
    // it. A submitted credential wins, because that one belongs to somebody else's primary.
    let mut minted_public: Option<PathBuf> = None;
    let effective_token: Option<String> = match token {
        Some(submitted) => {
            if submitted.trim().is_empty() {
                return Err("the credential submitted was empty".to_owned());
            }
            Some(submitted.trim().to_owned())
        }
        None => {
            let recorded = env.value(TOKEN_KEY);
            if recorded.is_some() && !options.remint {
                recorded
            } else {
                let (minted, public) =
                    super::mint::mint_for_local_primary(paths, options.mint_days)?;
                minted_public = Some(public);
                lines.push(format!(
                    "{TOKEN_KEY}: minted for {} day(s) from this deployment's signing key",
                    options.mint_days
                ));
                Some(minted)
            }
        }
    };
    if let Some(url) = options.url {
        let wrote = next.set(URL_KEY, url.trim());
        changed |= wrote;
        lines.push(format!(
            "{URL_KEY}: {}",
            if wrote { "set" } else { "unchanged" }
        ));
    }
    if let Some(secs) = options.interval_secs {
        let wrote = next.set(INTERVAL_KEY, &secs.to_string());
        changed |= wrote;
        lines.push(format!(
            "{INTERVAL_KEY}: {}",
            if wrote {
                secs.to_string()
            } else {
                "unchanged".to_owned()
            }
        ));
    }
    if let Some(token) = &effective_token {
        let wrote = next.set(TOKEN_KEY, token);
        changed |= wrote;
        if minted_public.is_none() {
            lines.push(format!(
                "{TOKEN_KEY}: {}",
                if wrote {
                    "stored (value hidden)"
                } else {
                    "unchanged (value hidden)"
                }
            ));
        }
    }
    if let Some(public) = &minted_public {
        lines.push(format!(
            "public key: {} — start sqld with --auth-jwt-key-file pointing at it",
            public.display()
        ));
    }
    if !changed {
        // Per-setting lines are noise when every one of them says nothing happened.
        lines.clear();
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

    // A capability reads the settings at start, so the write reaches it only through a restart.
    // This is the step that used to be printed for a person to run.
    for consumer in consumers(paths).iter().filter(|c| c.enabled) {
        lines.push(restart(paths, &consumer.name));
    }
    lines.push("next: sjel setup verify turso".to_owned());
    Ok(Applied { lines })
}

/// Restart one enabled capability through the runner that owns its lifecycle.
///
/// The runner is the only thing that knows how a capability is started on this machine (a launchd
/// unit, a systemd unit, a plain process), so this shells out to it rather than re-deriving that.
fn restart(paths: &Paths, name: &str) -> String {
    let runner = paths.root.join("tools/service-runner.sh");
    match std::process::Command::new(&runner)
        .args(["restart", name])
        .output()
    {
        Ok(output) if output.status.success() => format!("restart {name}: up"),
        Ok(output) => format!(
            "restart {name}: failed — {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ),
        Err(error) => format!(
            "restart {name}: could not run {}: {error}",
            runner.display()
        ),
    }
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
            detail: "not probed: no credential to present".to_owned(),
        },
        _ => Probe {
            id: "primary",
            label: "primary".to_owned(),
            mark: Mark::Missing,
            detail: "not probed: no primary named".to_owned(),
        },
    };
    probes.push(primary);

    let enabled: Vec<Consumer> = consumers(paths).into_iter().filter(|c| c.enabled).collect();
    probes.push(Probe {
        id: "wiring",
        label: "followers".to_owned(),
        mark: match enabled.is_empty() {
            true => Mark::Warn,
            false => Mark::Ok,
        },
        detail: match enabled.is_empty() {
            true => "no enabled capability links sjel-turso, so nothing follows the primary yet"
                .to_owned(),
            false => enabled
                .iter()
                .map(|c| format!("{} restarts against the settings", c.name))
                .collect::<Vec<String>>()
                .join("; "),
        },
    });

    probes
}

/// Ask the primary whether this credential works, over the door libsql itself uses.
///
/// Two endpoints, because two kinds of primary answer differently — measured against
/// `sqld 0.24.32` on 2026-10-21, where `GET /info` (Turso Cloud's sync-info endpoint) answers 404
/// and `POST /v2/pipeline` (the HTTP half of Hrana) is the one that runs a statement:
///
/// 1. `POST <base>/v2/pipeline` with a `SELECT 1`: the same authenticated path a replica's writes
///    take, so a 2xx means the value was accepted *and* a statement ran.
/// 2. `GET <base>/info`, which is what a Turso Cloud primary offers.
/// 3. `GET <base>/health`, unauthenticated on `sqld`: it proves the host is there and says nothing
///    about the credential, and its report says exactly that.
///
/// 401/403 from either of the first two is a refusal and is reported as one. That matters more
/// than it looks: libsql's client reports a refused value as "Timeout performing handshake with
/// primary", so without this probe a wrong one reads as a network fault.
fn probe_primary(url: &str, token: &str) -> Probe {
    let base = url.trim_end_matches('/');
    let endpoint = match base.strip_prefix("libsql://") {
        Some(rest) => format!("https://{rest}"),
        None => base.to_owned(),
    };

    let http = match sjel_http::client(
        sjel_http::Purpose::new("sjel-setup"),
        Duration::from_secs(10),
    ) {
        Ok(client) => client,
        Err(e) => {
            return Probe {
                id: "primary",
                label: "primary".to_owned(),
                mark: Mark::Warn,
                detail: format!("cannot build an HTTP client: {e}"),
            }
        }
    };

    let statement = format!("{endpoint}/v2/pipeline");
    let sql = r#"{"requests":[{"type":"execute","stmt":{"sql":"SELECT 1"}}]}"#;
    match http
        .post(&statement)
        .bearer_auth(token)
        .header("content-type", "application/json")
        .body(sql)
        .send()
    {
        Ok(response) if response.status().is_success() => {
            return Probe {
                id: "primary",
                label: "primary".to_owned(),
                mark: Mark::Ok,
                detail: format!(
                    "{statement} accepted it and ran a statement ({}) — the primary speaks Hrana \
                     over HTTP",
                    response.status()
                ),
            }
        }
        Ok(response) if refused(response.status()) => {
            return Probe {
                id: "primary",
                label: "primary".to_owned(),
                mark: Mark::Blocked,
                detail: format!(
                    "{statement} answered {} — it was refused. Mint a fresh one with `sjel setup \
                     apply turso --confirm --remint`, or submit the primary's own",
                    response.status()
                ),
            }
        }
        // A primary that does not serve the pipeline endpoint is not a failure by itself: a Turso
        // Cloud database answers at `/info` instead.
        Ok(_) | Err(_) => {}
    }

    let info = format!("{endpoint}/info");
    if let Ok(response) = http.get(&info).bearer_auth(token).send() {
        let status = response.status();
        if status.is_success() {
            return Probe {
                id: "primary",
                label: "primary".to_owned(),
                mark: Mark::Ok,
                detail: format!("{info} answered {status} — the primary offers the sync protocol"),
            };
        }
        if refused(status) {
            return Probe {
                id: "primary",
                label: "primary".to_owned(),
                mark: Mark::Blocked,
                detail: format!("{info} answered {status} — it was refused; submit a fresh one"),
            };
        }
    }

    // Last: is anything there at all? This proves reachability and nothing else.
    let health = format!("{endpoint}/health");
    match http.get(&health).send() {
        Ok(response) if response.status().is_success() => Probe {
            id: "primary",
            label: "primary".to_owned(),
            mark: Mark::Warn,
            detail: format!(
                "{health} answered {} but neither {statement} nor {info} did — the host is up, and \
                 this probe cannot tell from here whether the value works; a pull is the only \
                 answer, and `libs/sjel-turso/tests/live_primary.rs` runs one",
                response.status()
            ),
        },
        Ok(response) => Probe {
            id: "primary",
            label: "primary".to_owned(),
            mark: Mark::Warn,
            detail: format!(
                "{health} answered {} — check the URL and that the primary is running",
                response.status()
            ),
        },
        Err(e) => Probe {
            id: "primary",
            label: "primary".to_owned(),
            mark: Mark::Warn,
            detail: format!("{health} did not answer: {e}"),
        },
    }
}

/// Whether a status means "this value is not accepted" — a different failure from "this endpoint
/// is not here", and one that must not be reported as the other.
fn refused(status: reqwest::StatusCode) -> bool {
    status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_crate_that_links_the_engine_is_a_follower() {
        let body = "[dependencies]\nsjel-turso = { path = \"../../libs/sjel-turso\" }\n";
        assert!(depends_on_turso(body));
    }

    #[test]
    fn a_wrapped_dependency_line_is_still_found() {
        let body = "[dependencies]\nsjel-turso = { path = \"../../libs/sjel-turso\",\n    default-features = false }\n";
        assert!(depends_on_turso(body));
    }

    #[test]
    fn a_comment_about_the_crate_is_not_a_dependency() {
        let body =
            "# sjel-turso is the engine here, see the README\n[dependencies]\nserde = \"1\"\n";
        assert!(!depends_on_turso(body));
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
        let env = EnvFile {
            path: PathBuf::from("/tmp/does-not-matter.env"),
            body: "untouched\n".to_owned(),
        };
        let err = apply(
            &paths_stub(),
            &env,
            &Options {
                url: Some("axon-owner.turso.io"),
                interval_secs: None,
                mint_days: super::super::mint::DEFAULT_MINT_DAYS,
                remint: false,
            },
            None,
        )
        .unwrap_err();
        assert!(err.contains("names no primary"), "{err}");
        assert_eq!(env.value(URL_KEY), None);
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
                mint_days: super::super::mint::DEFAULT_MINT_DAYS,
                remint: false,
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
            mint_days: super::super::mint::DEFAULT_MINT_DAYS,
            remint: false,
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

    /// A credential already in the deployment record is reused, and nothing is minted: these
    /// tests must never touch this machine's Keychain, so every path that mints is kept out of
    /// them. Minting is exercised end to end by `sjel setup apply` against a real primary.
    #[test]
    fn a_recorded_credential_is_reused_without_minting() {
        let dir = std::env::temp_dir().join(format!("sjel-setup-reuse-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("config")).unwrap();
        let record = dir.join("config").join("deployment");
        std::fs::write(
            &record,
            "SJEL_TURSO_URL=libsql://first.turso.io\nSJEL_TURSO_AUTH_TOKEN=already-there\n",
        )
        .unwrap();
        let mut paths = paths_stub();
        paths.overlay_root = Some(dir.clone());

        let options = Options {
            url: Some("libsql://second.turso.io"),
            interval_secs: None,
            mint_days: super::super::mint::DEFAULT_MINT_DAYS,
            remint: false,
        };
        let env = EnvFile::read(record.clone());
        let applied = apply(&paths, &env, &options, None).unwrap();

        let body = std::fs::read_to_string(&record).unwrap();
        assert!(
            body.contains("SJEL_TURSO_AUTH_TOKEN=already-there"),
            "the recorded credential must survive a url change: {body}"
        );
        assert!(
            !applied.lines.join("\n").contains("minted"),
            "nothing should have been minted: {:?}",
            applied.lines
        );
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
