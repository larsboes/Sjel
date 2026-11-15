//! Refresh existing owned deployments without reconciling profile selections.

use super::{engine, mutate, pi_settings, registry, statuses_for, Harness, Model, Registry};
use engine::SkillStatus;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

/// Read the maintained harness set independently of installation and profile selection.
///
/// # Errors
/// The configuration is unreadable, malformed, duplicated, or names an unknown harness.
pub(super) fn maintained(root: &Path, registry: &Registry) -> Result<BTreeSet<String>, String> {
    let path = root.join("profiles.toml");
    let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let document: toml::Table = text
        .parse()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let Some(table) = document.get("maintenance") else {
        return Ok(BTreeSet::new());
    };
    let values = table
        .get("harnesses")
        .and_then(toml::Value::as_array)
        .ok_or("maintenance.harnesses must be an array of harness IDs")?;
    let mut ids = BTreeSet::new();
    for value in values {
        let id = value
            .as_str()
            .ok_or("maintenance.harnesses must contain strings")?;
        registry.by_id(id)?;
        if !ids.insert(id.to_owned()) {
            return Err(format!("maintenance.harnesses repeats '{id}'"));
        }
    }
    Ok(ids)
}

pub(super) fn run(registry: &Registry, root: &Path, check: bool) -> Result<(), String> {
    let ids = maintained(root, registry)?;
    let mut failures = Vec::new();
    for harness in &registry.harnesses {
        if !ids.contains(harness.id) {
            continue;
        }
        if !registry::is_installed(harness) {
            println!("{}: maintained, not installed; skipped", harness.id);
            continue;
        }
        if let Err(error) = refresh(harness, check) {
            failures.push(format!("{}: {error}", harness.id));
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("\n"))
    }
}

fn refresh(harness: &Harness, check: bool) -> Result<(), String> {
    let rows = statuses_for(harness, None)?;
    let config = if harness.id == "pi" {
        pi_settings::agents_config(&harness.config)
    } else {
        harness.config.clone()
    };
    let state = engine::read_state(&config)?;
    let held = mutate::held_units(&config)?;
    let mut updates: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut failures = Vec::new();
    for row in rows {
        if mutate::hold_matches(&held, &format!("{}/{}", row.pack, row.skill)) {
            continue;
        }
        let pack_record = state.packs.get(&row.pack);
        let record = pack_record.and_then(|p| p.skills.get(&row.skill));
        let upstream = |record: &engine::SkillRecord| {
            !record.source.is_empty()
                && engine::resolve(&config.sjel_root.join(&record.source))
                    .starts_with(crate::intake::skills::store_root())
        };
        if row.skill == "(manifest)" && row.status == SkillStatus::Invalid {
            if let Some(pack) = pack_record {
                if !pack.skills.is_empty() && pack.skills.values().all(upstream) {
                    println!(
                        "{}: {} managed by another lifecycle; skipped",
                        harness.id, row.pack
                    );
                } else {
                    failures.push(format!(
                        "{}: invalid manifest; left unchanged: {}",
                        row.pack,
                        row.detail.unwrap_or_default()
                    ));
                }
            }
            continue;
        }
        let recorded = record.is_some();
        if !recorded && harness.model == Model::Materialized {
            continue;
        }
        if record.is_some_and(upstream) {
            println!(
                "{}: {}/{} managed by another lifecycle; skipped",
                harness.id, row.pack, row.skill
            );
            continue;
        }
        let units = engine::pack_units(&config, &row.pack)?;
        let unit = units.iter().find(|unit| unit.key == row.skill);
        if recorded && unit.is_none() {
            failures.push(format!(
                "{}/{}: recorded unit absent from manifest; left unchanged",
                row.pack, row.skill
            ));
            continue;
        }
        let owned = unit.is_some_and(|unit| {
            engine::pack_roots(&config).iter().any(|root| {
                root.file_name().is_some_and(|n| n == "Packs") && unit.source_root.starts_with(root)
            })
        });
        if recorded && !owned {
            failures.push(format!(
                "{}/{}: source ownership unresolved; left unchanged",
                row.pack, row.skill
            ));
            continue;
        }
        match row.status {
            SkillStatus::Outdated if recorded && owned => {
                updates.entry(row.pack).or_default().insert(row.skill);
            }
            SkillStatus::Drifted
            | SkillStatus::MigrationRequired
            | SkillStatus::Missing
            | SkillStatus::Invalid
            | SkillStatus::Collision => {
                failures.push(format!(
                    "{}/{}: {}; left unchanged",
                    row.pack,
                    row.skill,
                    row.status.as_str()
                ));
            }
            _ => {}
        }
    }
    for (pack, keys) in updates {
        if check {
            failures.push(format!("{pack}: {} outdated unit(s)", keys.len()));
            continue;
        }
        match mutate::refresh_units(&config, &pack, &keys) {
            Ok(lines) => {
                for line in lines {
                    println!("{}: {pack}: {line}", harness.id);
                }
                let after = statuses_for(harness, Some(&pack))?;
                for key in keys {
                    if !after
                        .iter()
                        .any(|r| r.skill == key && r.status == SkillStatus::Current)
                    {
                        failures.push(format!("{pack}/{key}: destination verification failed"));
                    }
                }
            }
            Err(error) => failures.push(format!("{pack}: {error}")),
        }
    }
    if failures.is_empty() {
        println!("{}: existing owned deployments current", harness.id);
        Ok(())
    } else {
        Err(failures.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> (std::path::PathBuf, Registry) {
        let root =
            std::env::temp_dir().join(format!("sjel-maintenance-{name}-{}", std::process::id()));
        fs::create_dir_all(root.join("Packs/demo/skills/alpha")).unwrap();
        fs::write(
            root.join("profiles.toml"),
            "[maintenance]\nharnesses = ['claude', 'codex']\n",
        )
        .unwrap();
        fs::write(
            root.join("Packs/demo/pack.toml"),
            "name = 'demo'\ndescription = 'test'\nskills = ['alpha']\n",
        )
        .unwrap();
        let source = root.join("Packs/demo/skills/alpha/SKILL.md");
        fs::write(
            &source,
            "---\nname: alpha\ndescription: test\n---\noriginal\n",
        )
        .unwrap();
        let mut registry = Registry::new(&root);
        registry
            .harnesses
            .retain(|h| ["claude", "codex", "copilot"].contains(&h.id));
        for harness in &mut registry.harnesses {
            harness.marker = root.join(harness.id);
            fs::create_dir_all(&harness.marker).unwrap();
            harness.config.destination = harness.marker.join("skills");
            harness.config.state_file = harness.marker.join("state.json");
            harness.config.pack_roots = Some(vec![root.join("Packs")]);
            harness.config.tree_convention = None;
            mutate::deploy_pack(&harness.config, "demo", None).unwrap();
        }
        (root, registry)
    }

    #[test]
    fn maintenance_preserves_excluded_harnesses_drift_and_selection() {
        let (root, registry) = fixture("selection");
        let source = root.join("Packs/demo/skills/alpha/SKILL.md");
        fs::write(
            &source,
            "---\nname: alpha\ndescription: test\n---\nupdated\n",
        )
        .unwrap();
        let drifted = root.join("claude/skills/alpha/SKILL.md");
        fs::write(&drifted, "local change").unwrap();
        assert!(run(&registry, &root, true).is_err());
        assert!(fs::read_to_string(root.join("codex/skills/alpha/SKILL.md"))
            .unwrap()
            .contains("original"));
        let error = run(&registry, &root, false).unwrap_err();
        assert!(error.contains("drifted"), "{error}");
        assert_eq!(fs::read_to_string(drifted).unwrap(), "local change");
        assert!(fs::read_to_string(root.join("codex/skills/alpha/SKILL.md"))
            .unwrap()
            .contains("updated"));
        assert!(
            fs::read_to_string(root.join("copilot/skills/alpha/SKILL.md"))
                .unwrap()
                .contains("original")
        );
        assert_eq!(
            engine::read_state(&registry.by_id("codex").unwrap().config)
                .unwrap()
                .packs
                .len(),
            1
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn invalid_and_missing_owned_manifests_fail_without_mutation() {
        let (root, registry) = fixture("manifest-errors");
        let manifest = root.join("Packs/demo/pack.toml");
        fs::write(&manifest, "invalid = [").unwrap();
        for check in [true, false] {
            assert!(run(&registry, &root, check)
                .unwrap_err()
                .contains("invalid manifest"));
        }
        fs::remove_file(manifest).unwrap();
        for check in [true, false] {
            assert!(run(&registry, &root, check)
                .unwrap_err()
                .contains("invalid manifest"));
        }
        assert!(
            fs::read_to_string(root.join("claude/skills/alpha/SKILL.md"))
                .unwrap()
                .contains("original")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn removed_owned_units_are_unresolved_not_upstream_managed() {
        let (root, registry) = fixture("removed-units");
        fs::write(
            root.join("Packs/demo/pack.toml"),
            "name = 'demo'\ndescription = 'test'\nskills = []\n",
        )
        .unwrap();
        fs::write(root.join("claude/skills/alpha/SKILL.md"), "local edit").unwrap();
        fs::remove_dir_all(root.join("codex/skills/alpha")).unwrap();
        for check in [true, false] {
            let error = run(&registry, &root, check).unwrap_err();
            assert!(
                error.contains("claude: demo/alpha: recorded unit absent"),
                "{error}"
            );
            assert!(
                error.contains("codex: demo/alpha: recorded unit absent"),
                "{error}"
            );
        }
        assert_eq!(
            fs::read_to_string(root.join("claude/skills/alpha/SKILL.md")).unwrap(),
            "local edit"
        );
        assert!(!root.join("codex/skills/alpha").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn recorded_upstream_sources_keep_their_own_lifecycle() {
        let (root, registry) = fixture("upstream-records");
        for harness in &registry.harnesses {
            let mut state = engine::read_state(&harness.config).unwrap();
            state
                .packs
                .get_mut("demo")
                .unwrap()
                .skills
                .get_mut("alpha")
                .unwrap()
                .source = crate::intake::skills::store_root()
                .join("demo/skills/alpha")
                .display()
                .to_string();
            fs::write(
                &harness.config.state_file,
                serde_json::to_vec(&state).unwrap(),
            )
            .unwrap();
        }
        fs::write(
            root.join("claude/skills/alpha/SKILL.md"),
            "upstream local edit",
        )
        .unwrap();
        for check in [true, false] {
            run(&registry, &root, check).unwrap();
        }
        fs::write(root.join("Packs/demo/pack.toml"), "invalid = [").unwrap();
        for check in [true, false] {
            run(&registry, &root, check).unwrap();
        }
        assert_eq!(
            fs::read_to_string(root.join("claude/skills/alpha/SKILL.md")).unwrap(),
            "upstream local edit"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn maintained_is_explicit_validated_and_independent_of_installation() {
        let root = std::env::temp_dir().join(format!("sjel-maintained-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let root = root.as_path();
        let registry = Registry::new(root);
        let path = root.join("profiles.toml");
        fs::write(&path, "[maintenance]\nharnesses = ['pi', 'claude']\n").unwrap();
        assert_eq!(
            maintained(root, &registry).unwrap(),
            BTreeSet::from(["pi".into(), "claude".into()])
        );
        for invalid in ["['unknown']", "['pi','pi']", "[1]"] {
            fs::write(&path, format!("[maintenance]\nharnesses = {invalid}\n")).unwrap();
            assert!(maintained(root, &registry).is_err());
        }
        fs::write(&path, "").unwrap();
        assert!(maintained(root, &registry).unwrap().is_empty());
        fs::remove_dir_all(root).unwrap();
    }
}
