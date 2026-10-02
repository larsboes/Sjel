//! Which overlay and which machine.toml, and where each answer came from.
//!
//! A port of libs/overlay/overlay.ts, because the doctor reports the SOURCE ("from
//! axon.local.toml"), which tools/lib/paths.sh does not export. Three copies of one rule now:
//! paths.sh for the shell, overlay.ts for TypeScript, this for the doctor. A change to the
//! resolution order is a change to all three.

use std::path::{Path, PathBuf};

pub struct OverlayResolution {
    pub root: String,
    pub source: &'static str,
}

fn expand_home(path: &str) -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    if home.is_empty() {
        return path.to_owned();
    }
    if path == "~" {
        return home;
    }
    match path.strip_prefix("~/") {
        Some(rest) => Path::new(&home).join(rest).display().to_string(),
        None => path.to_owned(),
    }
}

fn read_key(file: &Path, read: impl Fn(&toml::Table) -> Option<String>) -> Option<String> {
    let doc = std::fs::read_to_string(file)
        .ok()?
        .parse::<toml::Table>()
        .ok()?;
    read(&doc)
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

fn overlay_key(file: &Path) -> Option<String> {
    read_key(file, |d| {
        let direct = d
            .get("overlay")
            .and_then(toml::Value::as_str)
            .filter(|s| !s.trim().is_empty());
        let nested = || {
            d.get("platform")?
                .get("overlay")?
                .as_str()
                .filter(|s| !s.trim().is_empty())
        };
        direct.or_else(nested).map(str::to_owned)
    })
}

pub fn resolve_overlay_root(root: &Path) -> Option<OverlayResolution> {
    for name in ["SJEL_OVERLAY_ROOT", "SJEL_PERSONAL_ROOT"] {
        if let Ok(v) = std::env::var(name) {
            let v = v.trim();
            if !v.is_empty() {
                return Some(OverlayResolution {
                    root: expand_home(v),
                    source: name,
                });
            }
        }
    }
    for name in ["axon.local.toml", "axon.toml"] {
        if let Some(v) = overlay_key(&root.join(name)) {
            return Some(OverlayResolution {
                root: expand_home(&v),
                source: name,
            });
        }
    }
    None
}

pub struct MachineResolution {
    pub path: PathBuf,
    pub source: &'static str,
    pub name: Option<String>,
}

fn short_hostname() -> String {
    std::process::Command::new("hostname")
        .output()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .trim()
                .split('.')
                .next()
                .unwrap_or("")
                .to_owned()
        })
        .unwrap_or_default()
}

/// `machine = "<name>"` in axon.local.toml, else `config/machines/<short-hostname>.toml` when it
/// exists, else the single-file `config/machine.toml`.
pub fn resolve_machine_toml(root: &Path, overlay: &str) -> MachineResolution {
    let machines = Path::new(overlay).join("config/machines");
    let explicit = read_key(&root.join("axon.local.toml"), |d| {
        d.get("machine")
            .and_then(toml::Value::as_str)
            .map(str::to_owned)
    });
    if let Some(name) = explicit {
        return MachineResolution {
            path: machines.join(format!("{name}.toml")),
            source: "axon.local.toml",
            name: Some(name),
        };
    }
    let host = short_hostname();
    if !host.is_empty() {
        let by_host = machines.join(format!("{host}.toml"));
        if by_host.exists() {
            return MachineResolution {
                path: by_host,
                source: "hostname",
                name: Some(host),
            };
        }
    }
    MachineResolution {
        path: Path::new(overlay).join("config/machine.toml"),
        source: "config/machine.toml",
        name: None,
    }
}
