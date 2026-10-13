//! sjel-agent harness adapter: registry-model delivery of skills into `config/agent.toml`.
//!
//! sjel-agent reads `[skills] paths = [...]` from the overlay's `config/agent.toml`.
//! This adapter computes skill paths from active profiles, writes them to `agent.toml`
//! preserving other sections and comments, and reports per-skill registry status.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use super::engine::{self, available_packs, pack_units, DeployConfig, SkillStatus, StatusRow};
use super::mutate::{
    held_units, hold_matches, read_profiles, resolve_profile_packs, resolve_profile_skills, Profile,
};

/// Reads `[skills] paths = [...]` from the target `agent.toml`.
pub fn read_skill_paths(path: &Path) -> Result<Vec<String>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let table: toml::Table = text
        .parse::<toml::Table>()
        .map_err(|e| format!("{}: {e}", path.display()))?;

    let paths = table
        .get("skills")
        .and_then(toml::Value::as_table)
        .and_then(|s| s.get("paths"))
        .and_then(toml::Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(toml::Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();

    Ok(paths)
}

/// Updates or inserts `[skills] paths = [...]` in `agent.toml` while preserving existing
/// sections (`[agent]`, `[compaction]`, `[mcp]`) and top-level comments.
pub fn update_agent_toml_skills(current_text: &str, paths: &[String]) -> String {
    let mut out = String::new();
    let lines: Vec<&str> = current_text.lines().collect();
    let mut in_skills = false;
    let mut in_paths_array = false;
    let mut replaced_paths = false;
    let mut skills_header_seen = false;

    for line in &lines {
        let trimmed = line.trim();
        if trimmed == "[skills]" {
            in_skills = true;
            skills_header_seen = true;
            out.push_str(line);
            out.push('\n');
            continue;
        } else if trimmed.starts_with('[') && trimmed.ends_with(']') {
            if in_skills && !replaced_paths {
                out.push_str(&format_paths_array(paths));
                replaced_paths = true;
            }
            in_skills = false;
            in_paths_array = false;
        }

        if in_skills {
            if trimmed.starts_with("paths") && trimmed.contains('=') {
                if trimmed.ends_with(']') {
                    out.push_str(&format_paths_array(paths));
                    replaced_paths = true;
                    continue;
                } else {
                    in_paths_array = true;
                    out.push_str(&format_paths_array(paths));
                    replaced_paths = true;
                    continue;
                }
            } else if in_paths_array {
                if trimmed.ends_with(']') {
                    in_paths_array = false;
                }
                continue;
            }
        }

        out.push_str(line);
        out.push('\n');
    }

    if in_skills && !replaced_paths {
        out.push_str(&format_paths_array(paths));
    }

    if !skills_header_seen {
        if !out.is_empty() && !out.ends_with("\n\n") {
            out.push('\n');
        }
        out.push_str("[skills]\n");
        out.push_str(&format_paths_array(paths));
    }

    out
}

fn format_paths_array(paths: &[String]) -> String {
    if paths.is_empty() {
        return "paths = []\n".to_string();
    }
    let mut s = String::from("paths = [\n");
    for p in paths {
        s.push_str(&format!("  \"{p}\",\n"));
    }
    s.push_str("]\n");
    s
}

/// Activates a profile on sjel-agent: rewrites `[skills] paths` in `agent.toml`.
pub fn activate_profile_on_agent(
    config: &DeployConfig,
    profile_name: &str,
) -> Result<Vec<String>, String> {
    let profiles: Vec<Profile> = read_profiles(config)?;
    let profile = profiles
        .iter()
        .find(|p| p.name == profile_name)
        .ok_or_else(|| format!("no such profile: '{profile_name}'"))?;
    let target: BTreeSet<String> = resolve_profile_packs(config, profile)?
        .into_iter()
        .collect();
    let subsets = resolve_profile_skills(config, profile)?;

    let mut messages = vec![format!(
        "Activating profile '{}' on sjel-agent — {}",
        profile.name, profile.description
    )];

    let held = held_units(config)?;
    let mut all_paths: Vec<String> = Vec::new();

    for pack in &target {
        let subset = subsets.get(pack).cloned().flatten();
        let mut paths = Vec::new();
        for unit in pack_units(config, pack)? {
            if !unit.is_skill {
                continue;
            }
            let label = format!("{pack}/{}", unit.key);
            if hold_matches(&held, &label) {
                continue;
            }
            if let Some(sub) = &subset {
                if !sub.contains(&unit.key) {
                    continue;
                }
            }
            paths.push(unit.source_root.display().to_string());
        }
        let count = paths.len();
        all_paths.extend(paths);
        messages.push(format!("  → {pack}: {count} skill(s)"));
    }

    let dest = &config.destination;
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let current_text = if dest.exists() {
        fs::read_to_string(dest).map_err(|e| format!("{}: {e}", dest.display()))?
    } else {
        String::new()
    };

    let updated = update_agent_toml_skills(&current_text, &all_paths);
    fs::write(dest, updated).map_err(|e| format!("{}: {e}", dest.display()))?;

    Ok(messages)
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default())
}

fn expand_home(path: &str) -> PathBuf {
    if path == "~" {
        return home();
    }
    match path.strip_prefix("~/") {
        Some(rest) => home().join(rest),
        None => PathBuf::from(path),
    }
}

/// Status rows for sjel-agent skills against `[skills] paths = [...]` in `agent.toml`.
pub fn registry_statuses(
    config: &DeployConfig,
    selected: Option<&str>,
) -> Result<Vec<StatusRow>, String> {
    let registered_paths = read_skill_paths(&config.destination)?;
    let registered_resolved: BTreeSet<PathBuf> = registered_paths
        .iter()
        .map(|p| engine::resolve(&expand_home(p)))
        .collect();

    let mut rows = Vec::new();
    for pack in available_packs(config, true)? {
        if selected.is_some_and(|s| s != pack) {
            continue;
        }
        for unit in pack_units(config, &pack)? {
            if !unit.is_skill {
                continue;
            }
            let resolved_unit = engine::resolve(&unit.source_root);
            let is_registered = registered_resolved.contains(&resolved_unit)
                || registered_paths
                    .iter()
                    .any(|p| p == &unit.source_root.display().to_string());

            let (status, detail) = if is_registered {
                (
                    SkillStatus::Current,
                    Some("registered in agent.toml".to_owned()),
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

    #[test]
    fn toml_update_preserves_other_sections_and_replaces_paths() {
        let initial = r#"# Initial comment
[agent]
extensions = ["guard", "skills"]

[skills]
paths = [
  "old/path"
]

[compaction]
max_tokens = 4096
"#;
        let new_paths = vec!["new/path1".to_string(), "new/path2".to_string()];
        let updated = update_agent_toml_skills(initial, &new_paths);
        assert!(updated.contains("# Initial comment"));
        assert!(updated.contains(r#"extensions = ["guard", "skills"]"#));
        assert!(updated.contains(r#"max_tokens = 4096"#));
        assert!(updated.contains(r#""new/path1","#));
        assert!(updated.contains(r#""new/path2","#));
        assert!(!updated.contains(r#""old/path""#));
    }

    #[test]
    fn toml_update_adds_skills_section_if_missing() {
        let initial = r#"[agent]
extensions = ["guard"]
"#;
        let new_paths = vec!["path/one".to_string()];
        let updated = update_agent_toml_skills(initial, &new_paths);
        assert!(updated.contains("[skills]"));
        assert!(updated.contains(r#""path/one","#));
    }
}
