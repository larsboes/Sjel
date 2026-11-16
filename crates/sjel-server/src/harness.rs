//! Which wall keeps each installed agent harness out of the overlay's `data/`, read from the
//! harness's own configuration (`docs/plans/containment.md`, C1). How a harness is started is not
//! visible here, so a harness without a sandbox of its own is a warning that names `sjel run`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::Serialize;

/// How a wall check came out.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Ok,
    Warn,
    Bad,
}

/// What one installed harness keeps out of the overlay's `data/`, and which agent token it
/// presents. `codex` is `~/.codex/config.toml`, `claude` is `~/.claude/settings.json`;
/// `enrolled` holds the agent names with a token (`config/agent-tokens/<name>.sha256`).
pub struct HarnessWalls<'a> {
    pub data: &'a str,
    pub codex: Option<&'a str>,
    pub claude: Option<&'a str>,
    pub pi: Option<PiLaunch>,
    pub antigravity: bool,
    pub enrolled: &'a BTreeSet<String>,
}

/// How the `pi` command starts, and the last run `pi --unwalled` recorded.
pub struct PiLaunch {
    /// `~/.local/bin/pi` resolves to the checkout's `tools/pi`, which walls it by default.
    pub walled_default: bool,
    /// The last line of [`sjel_config::UNWALLED_RUNS_LOG`] for pi: its UTC time and provider.
    pub last_unwalled: Option<(String, String)>,
}

pub fn harness_walls(h: &HarnessWalls) -> Vec<(Level, String)> {
    let mut out = Vec::new();
    let token = |out: &mut Vec<(Level, String)>, name: &str| {
        if !h.enrolled.contains(name) {
            out.push((
                Level::Warn,
                format!("{name} presents the default agent token — enroll its own in your shell: sjel agent enroll {name}"),
            ));
        }
    };
    if let Some(text) = h.codex {
        let config: toml::Table = toml::from_str(text).unwrap_or_default();
        let profile = config
            .get("default_permissions")
            .and_then(toml::Value::as_str);
        let denies = |path: &str| {
            profile
                .and_then(|p| {
                    config
                        .get("permissions")?
                        .get(p)?
                        .get("filesystem")?
                        .get(path)
                })
                .and_then(toml::Value::as_str)
                == Some("deny")
        };
        if denies(h.data) {
            out.push((
                Level::Ok,
                format!("Codex: its own sandbox denies {}", h.data),
            ));
        } else {
            out.push((
                Level::Bad,
                format!("Codex: its sandbox profile does not deny {} — add it from tools/templates/codex", h.data),
            ));
        }
        token(&mut out, "codex");
    }
    if let Some(text) = h.claude {
        let settings: serde_json::Value = serde_json::from_str(text).unwrap_or_default();
        let read_rule = settings["permissions"]["deny"]
            .as_array()
            .is_some_and(|rules| {
                rules.iter().any(|r| {
                    r.as_str()
                        .is_some_and(|r| r.starts_with("Read(") && r.contains("/data/**"))
                })
            });
        if !read_rule {
            out.push((
                Level::Bad,
                "Claude Code: file tools may read the overlay's data/ — run: sjel claude --force"
                    .into(),
            ));
        }
        if settings["sandbox"]["enabled"] == serde_json::Value::Bool(true) {
            out.push((Level::Ok, "Claude Code: its own shell sandbox is on".into()));
        } else {
            out.push((
                Level::Warn,
                "Claude Code: its shell sandbox is off — start it with `sjel run claude --provider anthropic --project <dir>`".into(),
            ));
        }
        token(&mut out, "claude-code");
    }
    if let Some(pi) = &h.pi {
        if pi.walled_default {
            out.push((
                Level::Ok,
                "pi: walled by default through tools/pi; `pi --unwalled` debugs on a trusted provider".into(),
            ));
        } else {
            out.push((
                Level::Warn,
                "pi: `pi` starts unwalled — link ~/.local/bin/pi to the checkout's tools/pi".into(),
            ));
        }
        if let Some((at, provider)) = &pi.last_unwalled {
            out.push((
                Level::Ok,
                format!("pi: last unwalled run {at} on {provider}"),
            ));
        }
        token(&mut out, "pi");
    }
    if h.antigravity {
        out.push((
            Level::Warn,
            "Antigravity: no reviewed containment; a contained run needs a fresh sign-in (docs/plans/containment.md §4)".into(),
        ));
    }
    out
}

/// The agent names with their own token: each `config/agent-tokens/<name>.sha256`. Hash files
/// only; no token is read.
pub fn enrolled_names(overlay: &Path) -> BTreeSet<String> {
    std::fs::read_dir(overlay.join("config/agent-tokens"))
        .map(|dir| {
            dir.flatten()
                .filter_map(|e| {
                    e.file_name()
                        .to_str()?
                        .strip_suffix(".sha256")
                        .map(str::to_owned)
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The walls for this machine: each harness's config under `home`, the overlay at `overlay`, and
/// the agent names enrolled there.
pub fn on_this_machine(home: &Path, overlay: &Path, claude_dir: &Path) -> Vec<(Level, String)> {
    let read = |path: PathBuf| std::fs::read_to_string(path).ok();
    let data = overlay.join("data").display().to_string();
    let enrolled = enrolled_names(overlay);
    let codex = read(home.join(".codex/config.toml"));
    let claude = read(claude_dir.join("settings.json"));
    harness_walls(&HarnessWalls {
        data: &data,
        codex: codex.as_deref(),
        claude: claude.as_deref(),
        pi: home.join(".pi/agent/bin/pi").exists().then(|| PiLaunch {
            walled_default: std::fs::canonicalize(home.join(".local/bin/pi"))
                .is_ok_and(|p| p.ends_with("tools/pi")),
            last_unwalled: last_unwalled_pi(&read(overlay.join(sjel_config::UNWALLED_RUNS_LOG))),
        }),
        antigravity: home.join(".local/bin/agy").exists(),
        enrolled: &enrolled,
    })
}

/// The time and provider of the last pi line in the unwalled-runs log.
fn last_unwalled_pi(log: &Option<String>) -> Option<(String, String)> {
    log.as_deref()?.lines().rev().find_map(|line| {
        let mut fields = line.split('\t');
        let (at, harness, provider) = (fields.next()?, fields.next()?, fields.next()?);
        Path::new(harness)
            .ends_with("pi")
            .then(|| (at.to_owned(), provider.to_owned()))
    })
}

#[cfg(test)]
mod harness_wall_tests {
    use super::*;

    fn walls(
        codex: Option<&str>,
        claude: Option<&str>,
        pi: Option<PiLaunch>,
        names: &[&str],
    ) -> Vec<(Level, String)> {
        let enrolled: BTreeSet<String> = names.iter().map(|n| n.to_string()).collect();
        harness_walls(&HarnessWalls {
            data: "/o/data",
            codex,
            claude,
            pi,
            antigravity: false,
            enrolled: &enrolled,
        })
    }

    #[test]
    fn codex_passes_only_when_its_active_profile_denies_data() {
        let good =
            "default_permissions = \"p\"\n[permissions.p.filesystem]\n\"/o/data\" = \"deny\"\n";
        let other =
            "default_permissions = \"q\"\n[permissions.p.filesystem]\n\"/o/data\" = \"deny\"\n";
        assert_eq!(
            walls(Some(good), None, None, &["codex"]),
            vec![(Level::Ok, "Codex: its own sandbox denies /o/data".into())]
        );
        assert_eq!(walls(Some(other), None, None, &["codex"])[0].0, Level::Bad);
        assert!(walls(Some(good), None, None, &[])[1]
            .1
            .contains("sjel agent enroll codex"));
    }

    #[test]
    fn claude_without_its_sandbox_names_sjel_run() {
        let claude = r#"{"permissions": {"deny": ["Read(~/o/data/**)"]}}"#;
        let found = walls(None, Some(claude), None, &["claude-code"]);
        assert_eq!(found.len(), 1);
        assert!(found
            .iter()
            .all(|(l, m)| *l == Level::Warn && m.contains("sjel run")));
        let open = walls(None, Some("{}"), None, &["claude-code"]);
        assert_eq!(open[0].0, Level::Bad);
        let sandboxed =
            r#"{"permissions": {"deny": ["Read(~/o/data/**)"]}, "sandbox": {"enabled": true}}"#;
        assert_eq!(
            walls(None, Some(sandboxed), None, &["claude-code"]),
            vec![(Level::Ok, "Claude Code: its own shell sandbox is on".into())]
        );
    }

    #[test]
    fn pi_is_walled_only_through_tools_pi_and_reports_its_last_unwalled_run() {
        let bare = walls(
            None,
            None,
            Some(PiLaunch {
                walled_default: false,
                last_unwalled: None,
            }),
            &["pi"],
        );
        assert_eq!(bare.len(), 1);
        assert_eq!(bare[0].0, Level::Warn);
        assert!(bare[0].1.contains("tools/pi"));

        let log = Some(
            "2026-11-16T10:00:00Z\t/x/pi\topenai\t/p\n2026-11-16T11:00:00Z\t/x/cat\topenai\t/p\n"
                .to_owned(),
        );
        let last = last_unwalled_pi(&log);
        assert_eq!(last, Some(("2026-11-16T10:00:00Z".into(), "openai".into())));
        let walled = walls(
            None,
            None,
            Some(PiLaunch {
                walled_default: true,
                last_unwalled: last,
            }),
            &["pi"],
        );
        assert_eq!(
            walled,
            vec![
                (Level::Ok, "pi: walled by default through tools/pi; `pi --unwalled` debugs on a trusted provider".into()),
                (Level::Ok, "pi: last unwalled run 2026-11-16T10:00:00Z on openai".into()),
            ]
        );
    }
}
