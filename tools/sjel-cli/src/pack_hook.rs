//! `tools/pack-drift-hook` — the Claude Code hook that makes Pack drift visible at the moment it
//! matters, ported from `tools/pack-drift-hook.ts` on 2026-10-04.
//!
//! Two events, both answered from the deployment ledgers: `SessionStart` lists every deployed
//! copy that no longer matches its Pack source, and `FileChanged` says so when the file that just
//! changed is one of those copies. It reports; it never blocks, and it never fails a session over
//! a status check.
//!
//! It is Rust rather than a shell or a bun script because it runs on every session start and
//! every file change: the whole job is reading a handful of ledgers and hashing a few trees, and
//! the interpreter's start-up was most of the cost. The two readers it used — `getStatuses`,
//! `packUnits`, `readState` — are the engine's, in this process, so there is no second reader of
//! the ledger format left in TypeScript.

use std::io::Read as _;
use std::path::Path;
use std::process::ExitCode;

use serde_json::Value;

use crate::harnesses::engine::{self, SkillStatus};
use crate::harnesses::registry::{is_installed, Harness, Model, Registry};

pub fn run() -> ExitCode {
    // Every failure path below is already a quiet one; this is the last one.
    let _ = report();
    ExitCode::SUCCESS
}

fn report() -> Result<(), String> {
    let mut raw = String::new();
    let _ = std::io::stdin().read_to_string(&mut raw);
    // Malformed hook input is the harness's problem, not a reason to shout.
    let input: Value = serde_json::from_str(raw.trim()).unwrap_or(Value::Null);
    let event = input
        .get("hook_event_name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let root = std::env::var("SJEL_ROOT").unwrap_or_default();
    let root = Path::new(&root);
    let registry = Registry::new(root);

    let lines = match event {
        "SessionStart" => session_start(&registry, root)?,
        "FileChanged" => {
            let paths: Vec<String> = input
                .get("file_paths")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            file_changed(&registry, root, &paths)?
        }
        _ => Vec::new(),
    };
    if !lines.is_empty() {
        println!("{}", lines.join("\n"));
    }
    Ok(())
}

fn materialized(registry: &Registry) -> Vec<&Harness> {
    registry
        .harnesses
        .iter()
        .filter(|harness| harness.model == Model::Materialized && is_installed(harness))
        .collect()
}

fn session_start(registry: &Registry, root: &Path) -> Result<Vec<String>, String> {
    let mut lines = Vec::new();
    for harness in materialized(registry) {
        for row in engine::get_statuses(&harness.config, None)? {
            // Only a drift: an outdated copy is a sync waiting to happen, not a hand edit.
            if row.status != SkillStatus::Drifted {
                continue;
            }
            let unit = engine::pack_units(&harness.config, &row.pack)?
                .into_iter()
                .find(|unit| unit.key == row.skill);
            let source = unit
                .map(|unit| relative(root, &unit.source_root))
                .unwrap_or_else(|| format!("{}/{}", row.pack, row.skill));
            lines.push(format!(
                "  {}: {}/{} — the installed copy differs from {source}",
                harness.id, row.pack, row.skill
            ));
        }
    }
    if lines.is_empty() {
        return Ok(Vec::new());
    }
    let mut out =
        vec!["Axon Pack drift, deployed copies that no longer match their source:".to_string()];
    out.extend(lines);
    out.push("  Keep the edit: tools/harnesses accept <pack> <skill> --from <harness>".to_string());
    out.push("  Discard it:    tools/harnesses sync <pack>".to_string());
    out.push("  Detail:        tools/harnesses drift --diff".to_string());
    Ok(out)
}

fn file_changed(registry: &Registry, root: &Path, paths: &[String]) -> Result<Vec<String>, String> {
    let mut lines = Vec::new();
    for path in paths {
        let full = engine::resolve(Path::new(path));
        for harness in materialized(registry) {
            let destination = &harness.config.destination;
            let inside = full
                .display()
                .to_string()
                .starts_with(&format!("{}/", destination.display()));
            if !inside {
                continue;
            }
            let Some(skill) = full
                .strip_prefix(destination)
                .ok()
                .and_then(|rel| rel.components().next())
                .map(|part| part.as_os_str().to_string_lossy().into_owned())
            else {
                continue;
            };
            let state = engine::read_state(&harness.config)?;
            let owner = state
                .packs
                .iter()
                .find(|(_, record)| record.skills.contains_key(&skill))
                .map(|(pack, _)| pack.clone());
            let Some(pack) = owner else { continue };
            let unit = engine::pack_units(&harness.config, &pack)?
                .into_iter()
                .find(|unit| unit.key == skill);
            let Some(unit) = unit else { continue };
            if !unit.source_root.exists() {
                continue;
            }
            lines.push(format!("{path} is a DEPLOYED COPY, not the source."));
            lines.push(format!(
                "  Source:        {} (Pack '{pack}', harness {})",
                relative(root, &unit.source_root),
                harness.id
            ));
            lines.push(format!(
                "  Keep this edit: tools/harnesses accept {pack} {skill} --from {}",
                harness.id
            ));
            lines.push(format!(
                "  Next sync of Pack '{pack}' refuses to run until one of those happens."
            ));
        }
    }
    Ok(lines)
}

fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .map(|rel| rel.display().to_string())
        .unwrap_or_else(|_| path.display().to_string())
}
