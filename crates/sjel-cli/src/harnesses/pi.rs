//! pi's two readers: the settings registry and the discovery roots.
//!
//! pi does not copy its skills the way claude, codex and opencode do — it loads them from
//! paths listed in `~/.pi/agent/settings.json`, and it ALSO loads skills and extensions from
//! roots nobody registered (pi docs/skills.md, docs/extensions.md). So a pi row is not a
//! digest comparison at all: it is "registered", "discovered", "missing", or "not deployed".
//!
//! Ported from the `registryStatuses`, `piDiscovery`, `discoveryRoots` and `discoveredAt`
//! functions of tools/harnesses.ts, 2026-10-02, message for message.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::engine::{self, DeployConfig, SkillStatus, StatusRow};
use super::pi_settings;

#[derive(Debug, Clone)]
pub struct PiDiscovered {
    pub name: String,
    /// Discovery root the entry was found in, as a human label.
    pub label: String,
    /// `copy`, `external`, `symlink` or `md`.
    pub kind: &'static str,
    pub detail: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PiExtension {
    pub path: String,
    /// `ledger`, `discovered`, or `both`.
    pub source: &'static str,
}

pub struct PiDiscovery {
    pub entries: Vec<PiDiscovered>,
    pub extensions: Vec<PiExtension>,
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default())
}

/// The roots pi loads without being told: two global ones, plus `.pi/skills` and
/// `.agents/skills` in the cwd and every ancestor up to the repository root.
fn discovery_roots() -> Vec<(PathBuf, String)> {
    let home = home();
    let mut roots = vec![
        (
            home.join(".pi").join("agent").join("skills"),
            "~/.pi/agent/skills".to_owned(),
        ),
        (
            home.join(".agents").join("skills"),
            "~/.agents/skills".to_owned(),
        ),
    ];
    let mut dir = std::env::current_dir().unwrap_or_default();
    loop {
        for name in [".pi", ".agents"] {
            let root = dir.join(name).join("skills");
            if root.exists() {
                roots.push((root.clone(), format!("project {}", root.display())));
            }
        }
        if dir.join(".git").exists() {
            break; // pi stops its walk at the repository root
        }
        match dir.parent() {
            Some(parent) if parent != dir => dir = parent.to_path_buf(),
            _ => break, // filesystem root
        }
    }
    roots
}

fn discovered_at(root: &Path, label: &str) -> Vec<PiDiscovered> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(root) else {
        return out;
    };
    let mut entries: Vec<std::fs::DirEntry> = entries.flatten().collect();
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        let full = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            let target = std::fs::read_link(&full)
                .map(|t| t.display().to_string())
                .unwrap_or_default();
            out.push(PiDiscovered {
                name,
                label: label.to_owned(),
                kind: "symlink",
                detail: Some(format!("→ {target}")),
            });
        } else if file_type.is_dir() {
            if !full.join("SKILL.md").exists() {
                continue;
            }
            let external = full.join(".git").exists();
            out.push(PiDiscovered {
                name,
                label: label.to_owned(),
                kind: if external { "external" } else { "copy" },
                detail: external.then(|| "carries .git; another installer owns it".to_owned()),
            });
        } else if file_type.is_file() && name.ends_with(".md") && name != "SKILL.md" {
            // Root .md files are skills when they carry valid skill frontmatter.
            let head: String = std::fs::read_to_string(&full)
                .unwrap_or_default()
                .chars()
                .take(1000)
                .collect();
            if looks_like_skill_md(&head) {
                out.push(PiDiscovered {
                    name: name.trim_end_matches(".md").to_owned(),
                    label: label.to_owned(),
                    kind: "md",
                    detail: None,
                });
            }
        }
    }
    out
}

/// `^---\s*\n[\s\S]*?\bname:\s*\S+[\s\S]*?\bdescription:\s*\S+` over the first 1000 chars.
fn looks_like_skill_md(head: &str) -> bool {
    let Some(rest) = head.strip_prefix("---") else {
        return false;
    };
    let Some(newline) = rest.find('\n') else {
        return false;
    };
    if !rest[..newline].chars().all(char::is_whitespace) {
        return false;
    }
    let body = &rest[newline + 1..];
    let Some(after_name) = key_with_value(body, "name:") else {
        return false;
    };
    key_with_value(after_name, "description:").is_some()
}

/// The slice after `key`, when a non-whitespace value follows it and the key is word-bounded.
fn key_with_value<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    let mut from = 0;
    while let Some(at) = text[from..].find(key) {
        let pos = from + at;
        let bounded = pos == 0
            || !text[..pos]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric() || c == '_');
        if bounded {
            let after = &text[pos + key.len()..];
            let trimmed = after.trim_start();
            if !trimmed.is_empty() {
                return Some(trimmed);
            }
        }
        from = pos + key.len();
    }
    None
}

/// Everything pi loads that the ledger does not name: discovered skills and extensions.
pub fn pi_discovery() -> Result<PiDiscovery, String> {
    let home = home();
    let mut entries = Vec::new();
    for (root, label) in discovery_roots() {
        entries.extend(discovered_at(&root, &label));
    }

    let settings_path = home.join(".pi").join("agent").join("settings.json");
    let ledger: Vec<String> = match std::fs::read_to_string(&settings_path) {
        Ok(text) => {
            let parsed: serde_json::Value = serde_json::from_str(&text)
                .map_err(|e| format!("{}: {e}", settings_path.display()))?;
            parsed
                .get("extensions")
                .and_then(serde_json::Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(serde_json::Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default()
        }
        Err(_) => Vec::new(),
    };

    let mut found: Vec<String> = Vec::new();
    let exts_root = home.join(".pi").join("agent").join("extensions");
    if let Ok(read) = std::fs::read_dir(&exts_root) {
        let mut read: Vec<std::fs::DirEntry> = read.flatten().collect();
        read.sort_by_key(std::fs::DirEntry::file_name);
        for entry in read {
            let name = entry.file_name().to_string_lossy().into_owned();
            let full = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if (file_type.is_file() || file_type.is_symlink()) && name.ends_with(".ts") {
                found.push(full.display().to_string());
            } else if file_type.is_dir() && full.join("index.ts").exists() {
                found.push(full.join("index.ts").display().to_string());
            }
        }
    }

    // Ledger first, then discovered; a path in both is `both`. Insertion order is the Map's,
    // so the report lists what the ledger names before what it does not.
    let mut by_path: Vec<(String, &'static str)> = Vec::new();
    let mut index: BTreeMap<String, usize> = BTreeMap::new();
    for path in ledger {
        if !index.contains_key(&path) {
            index.insert(path.clone(), by_path.len());
            by_path.push((path, "ledger"));
        }
    }
    for path in found {
        match index.get(&path) {
            Some(i) => by_path[*i].1 = "both",
            None => {
                index.insert(path.clone(), by_path.len());
                by_path.push((path, "discovered"));
            }
        }
    }

    Ok(PiDiscovery {
        entries,
        extensions: by_path
            .into_iter()
            .map(|(path, source)| PiExtension { path, source })
            .collect(),
    })
}

/// A registry harness has no copy, so it has no digest. Its states are its own.
pub fn registry_statuses(
    config: &DeployConfig,
    selected: Option<&str>,
) -> Result<Vec<StatusRow>, String> {
    let settings_path = home().join(".pi").join("agent").join("settings.json");
    let settings: serde_json::Value = match std::fs::read_to_string(&settings_path) {
        Ok(text) => {
            serde_json::from_str(&text).map_err(|e| format!("{}: {e}", settings_path.display()))?
        }
        Err(_) => serde_json::Value::Null,
    };
    let registered = registry_paths(&settings, "skills");
    let registered_extensions = registry_paths(&settings, "extensions");
    let registered_packages = registry_paths(&settings, "packages");

    let mut discovered: BTreeMap<String, Vec<PiDiscovered>> = BTreeMap::new();
    for (root, label) in discovery_roots() {
        for entry in discovered_at(&root, &label) {
            discovered
                .entry(entry.name.clone())
                .or_default()
                .push(entry);
        }
    }

    let mut rows = Vec::new();
    for pack in engine::available_packs(config, true)? {
        if selected.is_some_and(|s| s != pack) {
            continue;
        }
        for unit in engine::pack_units(config, &pack)? {
            if !unit.is_skill {
                continue;
            }
            let source_root = unit.source_root.display().to_string();
            let is_registered = is_registered(&registered, &source_root);
            let hits = discovered.get(&unit.key).cloned().unwrap_or_default();
            let (status, detail) = if is_registered && !hits.is_empty() {
                (
                    SkillStatus::Current,
                    Some(format!(
                        "registered in settings; ALSO discovered from {} — pi keeps the first found, so the copy can shadow the registration",
                        hits.iter()
                            .map(|h| format!("{}/{}", h.label, h.name))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )),
                )
            } else if is_registered {
                (
                    SkillStatus::Current,
                    Some("registered in settings".to_owned()),
                )
            } else if !hits.is_empty() {
                (
                    SkillStatus::Discovered,
                    Some(
                        hits.iter()
                            .map(|h| {
                                let detail = h
                                    .detail
                                    .as_ref()
                                    .map(|d| format!(", {d}"))
                                    .unwrap_or_default();
                                format!("{}/{} ({}{detail})", h.label, h.name, h.kind)
                            })
                            .collect::<Vec<_>>()
                            .join("; "),
                    ),
                )
            } else {
                (SkillStatus::NotDeployed, None)
            };
            rows.push(StatusRow {
                pack: pack.clone(),
                skill: unit.key.clone(),
                status,
                detail,
            });
        }

        // Extensions and vendored packages are registered the same way a skill is — by path — so
        // they belong in the same matrix. Without these rows a `[unit_hold]` naming a package, or
        // a hand-registration that overrides one, is invisible: doctor reads this table, and the
        // row's label is the `<pack>/<path from the Pack root>` form a hold is written in.
        // Only Current and NotDeployed are answered here: whether an unregistered copy of the same
        // name is being loaded from a global root is a question this row does not ask.
        for path in pi_settings::extensions_for_pack(config, &pack)? {
            let registered_here = is_registered(&registered_extensions, &path);
            rows.push(StatusRow {
                pack: pack.clone(),
                skill: format!("extensions/{}", pi_settings::extension_label(&path)),
                status: if registered_here {
                    SkillStatus::Current
                } else {
                    SkillStatus::NotDeployed
                },
                detail: registered_here.then(|| "registered in settings".to_owned()),
            });
        }
        for path in pi_settings::packages_for_pack(config, &pack)? {
            let registered_here = is_registered(&registered_packages, &path);
            rows.push(StatusRow {
                pack: pack.clone(),
                skill: format!("pi-packages/{}", pi_settings::last_segment(&path)),
                status: if registered_here {
                    SkillStatus::Current
                } else {
                    SkillStatus::NotDeployed
                },
                detail: registered_here.then(|| "registered in settings".to_owned()),
            });
        }
    }

    // A registered path that no longer exists is this model's only other defect.
    for path in registered
        .iter()
        .chain(registered_extensions.iter())
        .chain(registered_packages.iter())
    {
        // Only an absolute local path can be judged here: a `git:` or `npm:` source is not a
        // filesystem path, and `~` is not one until a shell expands it.
        if !path.starts_with('/') || Path::new(path).exists() {
            continue;
        }
        let name = Path::new(path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        rows.push(StatusRow {
            pack: "(registered)".to_owned(),
            skill: name,
            status: SkillStatus::Missing,
            detail: Some(format!("{path} does not exist")),
        });
    }
    Ok(rows)
}

/// Whether a Pack path is the path a settings entry names, both read as the absolute path pi would
/// load. A relative entry written by `pi install` resolves from the settings file's directory, so
/// comparing the strings as written would report a registered package as unregistered.
fn is_registered(registered: &[String], path: &str) -> bool {
    let target = pi_settings::canonical_path(path);
    registered
        .iter()
        .any(|entry| pi_settings::registered_path(entry) == target)
}

/// One settings list as registry paths. The object form declares a `source` beside its resource
/// filters (pi docs/packages.md), so a package written that way is still one path here.
fn registry_paths(settings: &serde_json::Value, key: &str) -> Vec<String> {
    settings
        .get(key)
        .and_then(serde_json::Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|entry| match entry {
                    serde_json::Value::String(s) => Some(s.clone()),
                    serde_json::Value::Object(o) => o
                        .get("source")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frontmatter_file_looks_like_a_skill() {
        assert!(looks_like_skill_md(
            "---\nname: interceptor\ndescription: drives browsers\n---\n"
        ));
        assert!(!looks_like_skill_md("# just a readme\n"));
        // `filename:` is not the `name:` key — the word boundary is what the regex encodes.
        assert!(!looks_like_skill_md(
            "---\nfilename: x\ndescription: y\n---\n"
        ));
    }

    #[test]
    fn discovery_roots_start_global() {
        let roots = discovery_roots();
        assert_eq!(roots[0].1, "~/.pi/agent/skills");
        assert_eq!(roots[1].1, "~/.agents/skills");
    }

    /// The two channels a `[unit_hold]` can name and no row used to answer for. The label is the
    /// hold's own `<pack>/<path from the Pack root>` form, because doctor matches the two.
    #[test]
    fn an_extension_and_a_package_each_get_a_row_and_a_sibling_pack_is_ignored() {
        let root = std::env::temp_dir().join(format!("sjel-pi-rows-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let pack_root = root.join("Packs");
        let pack = pack_root.join("demo");
        std::fs::create_dir_all(pack.join("pi-packages/accordion")).unwrap();
        std::fs::create_dir_all(pack.join("extensions")).unwrap();
        std::fs::write(
            pack.join("pack.toml"),
            "name = \"demo\"\ndescription = \"test\"\nskills = []\n",
        )
        .unwrap();
        std::fs::write(
            pack.join("pi-packages/accordion/package.json"),
            "{\"name\":\"accordion\",\"pi\":{\"extensions\":[\"./extension/accordion.ts\"]}}\n",
        )
        .unwrap();
        std::fs::write(pack.join("extensions/thing.ts"), "export default 1;\n").unwrap();

        let config = DeployConfig {
            axon_root: root.clone(),
            pack_roots: Some(vec![pack_root.clone()]),
            destination: root.join("dest"),
            state_file: root.join("state.json"),
            adapter: "pi".to_owned(),
            state_env_var: None,
            tree_convention: None,
            flat_file_convention: None,
            skip_manifest_skills: false,
            validate_adapter_files: None,
        };

        let rows = registry_statuses(&config, Some("demo")).unwrap();
        let label = |skill: &str| {
            rows.iter()
                .find(|r| r.pack == "demo" && r.skill == skill)
                .unwrap_or_else(|| panic!("no row for demo/{skill}: {rows:?}"))
        };
        assert_eq!(
            label("pi-packages/accordion").status,
            SkillStatus::NotDeployed
        );
        assert_eq!(
            label("extensions/thing.ts").status,
            SkillStatus::NotDeployed
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A package declared in the object form is one path, because that is what the hold, the
    /// status row and the settings write all compare.
    #[test]
    fn the_object_form_of_a_package_is_read_as_its_source() {
        let settings = serde_json::json!({
            "packages": [
                {"source": "/tmp/accordion", "skills": []},
                "git:github.com/x/y",
                7
            ]
        });
        assert_eq!(
            registry_paths(&settings, "packages"),
            vec!["/tmp/accordion".to_owned(), "git:github.com/x/y".to_owned()]
        );
        assert!(registry_paths(&settings, "skills").is_empty());
    }
}
