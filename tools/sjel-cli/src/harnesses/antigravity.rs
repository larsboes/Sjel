//! Antigravity harness adapter: registry-model delivery through `~/.gemini/config/skills.json`.
//!
//! Antigravity's documented machine-local customization root is `~/.gemini/config/`, and its
//! `skills.json` lists directories to scan one level deep. A Pack's `skills/` directory is one
//! such entry, so a Pack is registered in place: there is no copy, so nothing can drift.
//! `include_only` narrows an entry to the profile's selection, the same decision sjel-agent
//! writes as explicit paths in `agent.toml`.
//!
//! Entries this adapter does not own (any path outside the Pack roots, and every other key in
//! the file, such as `inherits`) are carried through untouched, because the operator may keep
//! their own customizations in the same file.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use super::engine::{self, available_packs, pack_units, DeployConfig, SkillStatus, StatusRow};
use super::mutate::{
    held_units, hold_matches, read_profiles, resolve_profile_packs, resolve_profile_skills, Profile,
};

/// One `entries` item this adapter writes. `include_only` names skill directories inside the
/// Pack's `skills/`; `None` means the whole directory, so a skill added to the Pack later loads
/// without another `use`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub path: String,
    pub include_only: Option<Vec<String>>,
}

/// Reads `skills.json`, or an empty document when the file does not exist yet. A file that
/// exists but does not parse is an error: overwriting it would drop the operator's entries.
pub fn read_document(path: &Path) -> Result<Value, String> {
    if !path.exists() {
        return Ok(json!({ "entries": [] }));
    }
    let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

/// The entries this adapter reads back: `entries` items that carry a `path`.
pub fn read_entries(document: &Value) -> Vec<Entry> {
    document
        .get("entries")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let path = item.get("path")?.as_str()?.to_owned();
                    let include_only =
                        item.get("include_only")
                            .and_then(Value::as_array)
                            .map(|names| {
                                names
                                    .iter()
                                    .filter_map(Value::as_str)
                                    .map(str::to_owned)
                                    .collect()
                            });
                    Some(Entry { path, include_only })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Replaces every entry under a Pack root with `wanted`, keeping all other entries and keys.
pub fn update_document(document: Value, pack_roots: &[PathBuf], wanted: &[Entry]) -> Value {
    let mut document = match document {
        Value::Object(map) => map,
        _ => Map::new(),
    };
    let kept: Vec<Value> = document
        .get("entries")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter(|item| {
                    let Some(path) = item.get("path").and_then(Value::as_str) else {
                        return true;
                    };
                    let resolved = engine::resolve(&expand_home(path));
                    !pack_roots.iter().any(|root| resolved.starts_with(root))
                })
                .cloned()
                .collect()
        })
        .unwrap_or_default();

    let mut entries = kept;
    for entry in wanted {
        let mut item = Map::new();
        item.insert("path".to_owned(), json!(entry.path));
        if let Some(names) = &entry.include_only {
            item.insert("include_only".to_owned(), json!(names));
        }
        entries.push(Value::Object(item));
    }
    document.insert("entries".to_owned(), Value::Array(entries));
    Value::Object(document)
}

/// Activates a profile on Antigravity: rewrites the Pack entries in `skills.json`.
pub fn activate_profile(config: &DeployConfig, profile_name: &str) -> Result<Vec<String>, String> {
    let profiles: Vec<Profile> = read_profiles(config)?;
    let profile = profiles
        .iter()
        .find(|p| p.name == profile_name)
        .ok_or_else(|| format!("no such profile: '{profile_name}'"))?;
    let target: BTreeSet<String> = resolve_profile_packs(config, profile)?
        .into_iter()
        .collect();
    let subsets = resolve_profile_skills(config, profile)?;
    let held = held_units(config)?;

    let mut messages = vec![format!(
        "Activating profile '{}' on Antigravity — {}",
        profile.name, profile.description
    )];
    let mut wanted = Vec::new();
    for pack in &target {
        let subset = subsets.get(pack).cloned().flatten();
        let skills: Vec<_> = pack_units(config, pack)?
            .into_iter()
            .filter(|u| u.is_skill)
            .collect();
        let Some(first) = skills.first() else {
            continue;
        };
        let skills_dir = first
            .source_root
            .parent()
            .ok_or_else(|| format!("{pack}: skill has no parent directory"))?
            .to_path_buf();
        let every: BTreeSet<String> = skills.iter().map(|u| u.key.clone()).collect();
        let selected: BTreeSet<String> = skills
            .iter()
            .filter(|u| !hold_matches(&held, &format!("{pack}/{}", u.key)))
            .filter(|u| subset.as_ref().is_none_or(|s| s.contains(&u.key)))
            .map(|u| u.key.clone())
            .collect();
        messages.push(format!("  → {pack}: {} skill(s)", selected.len()));
        if selected.is_empty() {
            continue;
        }
        wanted.push(Entry {
            path: skills_dir.display().to_string(),
            include_only: (selected != every).then(|| selected.into_iter().collect()),
        });
    }

    let dest = &config.destination;
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let document = read_document(dest)?;
    let roots = config
        .pack_roots
        .clone()
        .unwrap_or_else(|| vec![config.axon_root.join("Packs")]);
    let updated = update_document(document, &roots, &wanted);
    let text = serde_json::to_string_pretty(&updated).map_err(|e| format!("{e}"))?;
    // Write beside the file and rename, so a crash mid-write never leaves a truncated document.
    let tmp = dest.with_extension("json.tmp");
    fs::write(&tmp, format!("{text}\n")).map_err(|e| format!("{}: {e}", tmp.display()))?;
    fs::rename(&tmp, dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    Ok(messages)
}

fn expand_home(path: &str) -> PathBuf {
    let home = PathBuf::from(std::env::var("HOME").unwrap_or_default());
    if path == "~" {
        return home;
    }
    match path.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None => PathBuf::from(path),
    }
}

/// Status rows for Antigravity against `skills.json`: a skill is current when an entry covers
/// its directory and, if the entry narrows, names it.
pub fn registry_statuses(
    config: &DeployConfig,
    selected: Option<&str>,
) -> Result<Vec<StatusRow>, String> {
    let entries = read_entries(&read_document(&config.destination)?);
    let mut rows = Vec::new();
    for pack in available_packs(config, true)? {
        if selected.is_some_and(|s| s != pack) {
            continue;
        }
        for unit in pack_units(config, &pack)? {
            if !unit.is_skill {
                continue;
            }
            let Some(skills_dir) = unit.source_root.parent() else {
                continue;
            };
            let wanted_dir = engine::resolve(skills_dir);
            let covered = entries.iter().any(|entry| {
                engine::resolve(&expand_home(&entry.path)) == wanted_dir
                    && entry
                        .include_only
                        .as_ref()
                        .is_none_or(|names| names.contains(&unit.key))
            });
            let (status, detail) = if covered {
                (
                    SkillStatus::Current,
                    Some("registered in skills.json".to_owned()),
                )
            } else {
                (SkillStatus::NotDeployed, None)
            };
            rows.push(StatusRow {
                pack: pack.clone(),
                skill: unit.key,
                status,
                detail,
            });
        }
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sjel-harnesses-antigravity-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(path: &Path, body: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }

    /// A repository with one Pack of three skills, a profile that takes every Pack, and one
    /// hold on `demo/beta`.
    fn fixture(name: &str) -> (PathBuf, DeployConfig) {
        let root = temp_dir(name);
        let repo = root.join("repo");
        for skill in ["alpha", "beta", "gamma"] {
            write(
                &repo.join("Packs/demo/skills").join(skill).join("SKILL.md"),
                &format!("---\nname: {skill}\ndescription: does a thing\n---\n\nbody\n"),
            );
        }
        write(
            &repo.join("Packs/demo/pack.toml"),
            "name = \"demo\"\ndescription = \"test\"\nskills = [\"alpha\", \"beta\", \"gamma\"]\n",
        );
        write(
            &repo.join("profiles.toml"),
            "[unit_hold]\nantigravity = [\"demo/beta\"]\n\n[[profile]]\nname = \"full\"\ndescription = \"test\"\npacks = [\"*\"]\n",
        );
        let config = DeployConfig {
            axon_root: repo.clone(),
            pack_roots: Some(vec![repo.join("Packs")]),
            destination: root.join("home/.gemini/config/skills.json"),
            state_file: root.join("state/antigravity.json"),
            adapter: "antigravity".to_owned(),
            state_env_var: None,
            tree_convention: None,
            flat_file_convention: None,
            skip_manifest_skills: false,
            validate_adapter_files: None,
        };
        (root, config)
    }

    #[test]
    fn update_keeps_entries_it_does_not_own() {
        let pack_roots = vec![PathBuf::from("/repo/Packs")];
        let before = json!({
            "inherits": [{ "path": "/shared/skills.json" }],
            "entries": [
                { "path": "/elsewhere/skills" },
                { "path": "/repo/Packs/old/skills" }
            ]
        });
        let wanted = vec![Entry {
            path: "/repo/Packs/new/skills".to_owned(),
            include_only: Some(vec!["one".to_owned()]),
        }];
        let after = update_document(before, &pack_roots, &wanted);
        let entries = read_entries(&after);
        assert_eq!(
            entries.iter().map(|e| e.path.as_str()).collect::<Vec<_>>(),
            vec!["/elsewhere/skills", "/repo/Packs/new/skills"]
        );
        assert_eq!(after["inherits"][0]["path"], "/shared/skills.json");
    }

    #[test]
    fn activation_registers_packs_and_narrows_a_held_skill() {
        let (root, config) = fixture("activate");
        activate_profile(&config, "full").unwrap();
        let document = read_document(&config.destination).unwrap();
        let entries = read_entries(&document);
        assert_eq!(entries.len(), 1, "{document}");
        assert_eq!(
            entries[0].path,
            root.join("repo/Packs/demo/skills").display().to_string()
        );
        assert_eq!(
            entries[0].include_only,
            Some(vec!["alpha".to_owned(), "gamma".to_owned()]),
            "beta is held, so the entry names the two that load"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn status_reads_the_registration_back() {
        let (root, config) = fixture("status");
        activate_profile(&config, "full").unwrap();
        let rows = registry_statuses(&config, Some("demo")).unwrap();
        let status_of = |skill: &str| {
            rows.iter()
                .find(|r| r.skill == skill)
                .map(|r| r.status.clone())
                .unwrap()
        };
        assert_eq!(status_of("alpha"), SkillStatus::Current);
        assert_eq!(status_of("gamma"), SkillStatus::Current);
        assert_eq!(status_of("beta"), SkillStatus::NotDeployed);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn an_unparseable_file_is_refused_rather_than_overwritten() {
        let (root, config) = fixture("broken");
        write(&config.destination, "{ not json");
        let err = activate_profile(&config, "full").unwrap_err();
        assert!(err.contains("skills.json"), "{err}");
        assert_eq!(
            fs::read_to_string(&config.destination).unwrap(),
            "{ not json"
        );
        let _ = fs::remove_dir_all(&root);
    }
}
