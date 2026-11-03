//! Command-line tooling, read from each package manager's own record of what it installed:
//!
//! | Manager | Record | Row key |
//! |---|---|---|
//! | brew formulae | `brew leaves --installed-on-request`, versions from the Cellar | `brew:<name>` |
//! | brew casks | `Caskroom/<token>`, except casks whose app is already an app row | `cask:<token>` |
//! | cargo | `~/.cargo/.crates2.json` | `cargo:<crate>` |
//! | npm | the folders in npm's global root | `npm:<package>` |
//! | uv | `~/.local/share/uv/tools/<name>` | `uv:<name>` |
//!
//! Not `tools/updates --json`: that is a staleness report, and offline it collapses brew and uv
//! into one summary row each, so it cannot say what is installed. Vendor CLIs (Claude Code,
//! Codex) update and uninstall through their own installers and are not listed.
//!
//! A cask with no app on disk is still listed: on 2026-10-22 `macspace` and `libreoffice` were
//! cask records whose apps had been deleted by hand, and only `brew uninstall --cask` clears them.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::discover::{du, Found};

pub const CASKROOM: &str = "/opt/homebrew/Caskroom";
const CELLAR: &str = "/opt/homebrew/Cellar";
const NPM_ROOT: &str = "/opt/homebrew/lib/node_modules";

/// Where brew lives. launchd hands a service a PATH without it, so it is found by path.
pub fn brew() -> Option<PathBuf> {
    ["/opt/homebrew/bin/brew", "/usr/local/bin/brew"]
        .iter()
        .map(PathBuf::from)
        .find(|p| p.exists())
}

pub fn scan(home: &Path, apps: &[Found]) -> Vec<Found> {
    let mut out = Vec::new();
    out.extend(formulae());
    out.extend(casks(apps));
    out.extend(cargo(home));
    out.extend(npm());
    out.extend(uv(home));
    out
}

fn tool(manager: &str, name: &str, version: String, paths: Vec<String>) -> Found {
    Found {
        kind: "tool",
        bundle_id: format!("{manager}:{name}"),
        name: name.to_owned(),
        path: paths.first().cloned().unwrap_or_default(),
        version,
        source: manager.to_owned(),
        app_bytes: du(&paths),
        data_bytes: 0,
        data_paths: Vec::new(),
        last_used: None,
    }
}

fn formulae() -> Vec<Found> {
    let Some(brew) = brew() else {
        return Vec::new();
    };
    let Ok(out) = Command::new(brew)
        .args(["leaves", "--installed-on-request"])
        .output()
    else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(|name| {
            // A tap formula prints as `owner/tap/name`; its Cellar folder is the last part.
            let short = name.rsplit('/').next().unwrap_or(name);
            let keg = Path::new(CELLAR).join(short);
            tool(
                "brew",
                name,
                newest_child(&keg),
                vec![keg.to_string_lossy().into_owned()],
            )
        })
        .collect()
}

fn casks(apps: &[Found]) -> Vec<Found> {
    let Ok(entries) = std::fs::read_dir(CASKROOM) else {
        return Vec::new();
    };
    let mut out: Vec<Found> = entries
        .flatten()
        .filter_map(|entry| {
            let token = entry.file_name().to_string_lossy().into_owned();
            let owned = format!("cask:{token}");
            if apps.iter().any(|app| app.source == owned) {
                return None;
            }
            let dir = entry.path();
            Some(tool(
                "cask",
                &token,
                newest_child(&dir),
                vec![dir.to_string_lossy().into_owned()],
            ))
        })
        .collect();
    out.sort_by(|a, b| a.bundle_id.cmp(&b.bundle_id));
    out
}

fn cargo(home: &Path) -> Vec<Found> {
    let Ok(text) = std::fs::read_to_string(home.join(".cargo/.crates2.json")) else {
        return Vec::new();
    };
    let bin = home.join(".cargo/bin");
    crates2(&text)
        .into_iter()
        .map(|(name, version, bins)| {
            let paths = bins
                .iter()
                .map(|b| bin.join(b).to_string_lossy().into_owned())
                .collect();
            tool("cargo", &name, version, paths)
        })
        .collect()
}

/// `(crate, version, binaries)` from `.crates2.json`, whose keys read
/// `bottom 0.14.9 (registry+https://…)`.
pub fn crates2(text: &str) -> Vec<(String, String, Vec<String>)> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return Vec::new();
    };
    let Some(installs) = value["installs"].as_object() else {
        return Vec::new();
    };
    installs
        .iter()
        .filter_map(|(key, entry)| {
            let mut parts = key.split_whitespace();
            let name = parts.next()?.to_owned();
            let version = parts.next().unwrap_or_default().to_owned();
            let bins = entry["bins"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|b| b.as_str().map(str::to_owned))
                .collect();
            Some((name, version, bins))
        })
        .collect()
}

fn npm() -> Vec<Found> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(NPM_ROOT) else {
        return out;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        // A scope folder (`@playwright`) holds the packages; list those, not the scope.
        let packages: Vec<(String, PathBuf)> = if name.starts_with('@') {
            std::fs::read_dir(entry.path())
                .into_iter()
                .flatten()
                .flatten()
                .map(|p| {
                    let inner = p.file_name().to_string_lossy().into_owned();
                    (format!("{name}/{inner}"), p.path())
                })
                .collect()
        } else if name.starts_with('.') {
            continue;
        } else {
            vec![(name, entry.path())]
        };
        for (package, dir) in packages {
            out.push(tool(
                "npm",
                &package,
                package_version(&dir),
                vec![dir.to_string_lossy().into_owned()],
            ));
        }
    }
    out.sort_by(|a, b| a.bundle_id.cmp(&b.bundle_id));
    out
}

fn uv(home: &Path) -> Vec<Found> {
    let root = home.join(".local/share/uv/tools");
    let Ok(entries) = std::fs::read_dir(&root) else {
        return Vec::new();
    };
    let mut out: Vec<Found> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            tool(
                "uv",
                &name,
                String::new(),
                vec![e.path().to_string_lossy().into_owned()],
            )
        })
        .collect();
    out.sort_by(|a, b| a.bundle_id.cmp(&b.bundle_id));
    out
}

fn package_version(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("package.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|v| v["version"].as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// The last version folder in a keg or cask directory, skipping dot folders (`.metadata`).
fn newest_child(dir: &Path) -> String {
    let mut versions: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| !n.starts_with('.'))
        .collect();
    versions.sort();
    versions.pop().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crates2_keys_give_name_version_and_bins() {
        let text = r#"{"installs":{"bottom 0.14.9 (registry+https://x)":{"bins":["btm"]}}}"#;
        assert_eq!(
            crates2(text),
            vec![("bottom".into(), "0.14.9".into(), vec!["btm".into()])]
        );
        assert!(crates2("{}").is_empty());
    }

    #[test]
    fn a_cask_whose_app_is_a_row_is_not_listed_twice() {
        let app = Found {
            kind: "app",
            bundle_id: "com.vorssaint.utils".into(),
            name: "Vorssaint".into(),
            path: String::new(),
            version: String::new(),
            source: "cask:vorssaint".into(),
            app_bytes: 0,
            data_bytes: 0,
            data_paths: vec![],
            last_used: None,
        };
        assert!(!casks(&[app])
            .iter()
            .any(|t| t.bundle_id == "cask:vorssaint"));
    }
}
