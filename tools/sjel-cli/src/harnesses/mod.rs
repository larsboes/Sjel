//! `tools/harnesses` — Packs across every agent harness at once, reads half.
//!
//! Each `packs-<harness>` adapter owns one destination and answers about it alone. Nothing
//! answered the question an operator actually has: what is deployed WHERE, what has drifted,
//! and what is sitting in a harness Sjel does not know about. This asks every harness the same
//! question and prints one answer.
//!
//! Ported from tools/harnesses.ts on 2026-10-02, read verbs first (decided 2026-10-02): `list`,
//! `status` and `drift` are Rust, and `sync`, `use`, `promote` and `accept` still run the
//! TypeScript implementation through this same launcher. The split is temporary and named: the
//! write verbs keep the engine that owns the mutation lock and the atomic install, and they
//! move when their parity is proven. The read half they used to share is gone from the
//! TypeScript file rather than left as a second reader of the same ledger.
//!
//! Direction: Sjel is the source and `sync` is one-way, Sjel -> harness. The one move in the
//! other direction is `promote`, which is manual on purpose: a skill written inside a harness
//! is brought into a Pack only when a human decides it is worth sharing system-wide.

use std::collections::{BTreeMap, BTreeSet};
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

pub mod engine;
mod frontmatter;
pub(crate) mod pi;
pub(crate) mod registry;

use engine::{SkillStatus, StatusRow};
use registry::{is_installed, Harness, Model, Registry};

const HELP: &str = "tools/harnesses — Packs across every agent harness at once.

  list                                  which harnesses exist, and which are installed here
  status [<pack>] [--json]              one matrix: every Pack skill x every harness
  drift [<pack>] [--diff]               per-file detail for anything that drifted
  sync <pack>|--all                     one-way Sjel -> harness (installed harnesses only)
  use <profile> [--harness <id>]        activate a profile on every installed harness (or one)
  promote <skill> --pack <p> [--from h] bring a harness-level skill Sjel does not own into a Pack
  accept <pack> <skill> [--from h]      keep a destination edit to a skill Sjel already owns

  --harness <id>     restrict to one harness (implies it, installed or not)
  --all-harnesses    include harnesses that are not installed
";

/// Verbs that still run the TypeScript engine: the ones that write.
const WRITE_VERBS: [&str; 4] = ["sync", "use", "promote", "accept"];

pub fn run(args: &[String]) -> ExitCode {
    let positional = positional(args);
    let verb = positional.first().map_or("list", String::as_str);
    match verb {
        "list" | "status" | "drift" => match read(verb, args) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("harnesses: {e}");
                ExitCode::from(1)
            }
        },
        "help" | "-h" | "--help" => {
            println!("{HELP}");
            ExitCode::SUCCESS
        }
        v if WRITE_VERBS.contains(&v) => forward(args),
        other => {
            eprintln!("{HELP}");
            eprintln!("harnesses: unknown verb '{other}'");
            ExitCode::from(1)
        }
    }
}

/// Hand a write verb to the TypeScript implementation, argv unchanged.
fn forward(args: &[String]) -> ExitCode {
    let root = match std::env::var("SJEL_ROOT") {
        Ok(r) if !r.is_empty() => PathBuf::from(r),
        _ => {
            eprintln!("harnesses: SJEL_ROOT is unset — run tools/harnesses, which sets it");
            return ExitCode::from(2);
        }
    };
    let mut cmd = Command::new("bun");
    cmd.arg("run")
        .arg(root.join("tools/harnesses.ts"))
        .args(args);
    let err = cmd.exec();
    eprintln!(
        "harnesses: cannot run {}: {err}",
        cmd.get_program().to_string_lossy()
    );
    ExitCode::from(127)
}

fn read(verb: &str, args: &[String]) -> Result<(), String> {
    let root = std::env::var("SJEL_ROOT")
        .ok()
        .filter(|r| !r.is_empty())
        .map(PathBuf::from)
        .ok_or("SJEL_ROOT is unset — run tools/harnesses, which sets it")?;
    let registry = Registry::new(&root);
    match verb {
        "list" => list(&registry),
        "status" => status(&registry, args),
        "drift" => drift(&registry, args),
        _ => unreachable!("read() is only called for the three read verbs"),
    }
}

// ---- argv ----------------------------------------------------------------------------------

/// The arguments the TypeScript tool called `positional`: everything that is not a flag, and
/// not the value of `--harness` or `--pack`. `--diff`, `--json` and `--all-harnesses` filter
/// themselves out, and a value after `--from` does not.
fn positional(args: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for (i, arg) in args.iter().enumerate() {
        if arg.starts_with("--") {
            continue;
        }
        if i > 0 && (args[i - 1] == "--harness" || args[i - 1] == "--pack") {
            continue;
        }
        out.push(arg.clone());
    }
    out
}

fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    let needle = format!("--{name}");
    let i = args.iter().position(|a| *a == needle)?;
    args.get(i + 1).map(String::as_str)
}

fn has(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == &format!("--{name}"))
}

fn selected_harnesses<'a>(
    registry: &'a Registry,
    args: &[String],
) -> Result<Vec<&'a Harness>, String> {
    if let Some(id) = flag(args, "harness") {
        return Ok(vec![registry.by_id(id)?]);
    }
    if has(args, "all-harnesses") {
        return Ok(registry.harnesses.iter().collect());
    }
    Ok(registry
        .harnesses
        .iter()
        .filter(|h| is_installed(h))
        .collect())
}

pub(crate) fn statuses_for(
    harness: &Harness,
    selected: Option<&str>,
) -> Result<Vec<StatusRow>, String> {
    match harness.model {
        Model::Registry => pi::registry_statuses(&harness.config, selected),
        Model::Materialized => engine::get_statuses(&harness.config, selected),
    }
}

// ---- list ----------------------------------------------------------------------------------

fn list(registry: &Registry) -> Result<(), String> {
    println!("harness      state       delivery       marker");
    for h in &registry.harnesses {
        let state = if is_installed(h) {
            "installed"
        } else {
            "absent   "
        };
        println!(
            "{:<12} {}   {:<14} {}",
            h.id,
            state,
            h.model.as_str(),
            h.marker.display()
        );
    }
    for u in &registry.unsupported {
        println!("{:<12} unsupported —              {}", u.id, u.why);
    }
    Ok(())
}

// ---- status --------------------------------------------------------------------------------

const MARK_LEGEND: &str = "\n· current   o outdated   D drifted   M missing   C collision   ! invalid   ~ discovered (loaded outside the ledger)   (blank) not deployed";

fn mark(status: &SkillStatus) -> &'static str {
    match status {
        SkillStatus::Current => "·",
        SkillStatus::NotDeployed => " ",
        SkillStatus::Outdated => "o",
        SkillStatus::Drifted => "D",
        SkillStatus::Missing => "M",
        SkillStatus::Collision => "C",
        SkillStatus::Invalid => "!",
        SkillStatus::Discovered => "~",
        SkillStatus::MigrationRequired => "m",
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct PacksView {
    measured_at: String,
    harnesses: Vec<HarnessView>,
    unsupported: Vec<UnsupportedView>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct UnsupportedView {
    id: &'static str,
    label: &'static str,
    why: &'static str,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct HarnessView {
    id: &'static str,
    label: &'static str,
    installed: bool,
    model: &'static str,
    marker: String,
    destination: Option<String>,
    cli: &'static str,
    units: Vec<UnitView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    discovered: Option<DiscoveryView>,
    unowned: Vec<StrayView>,
}

#[derive(serde::Serialize)]
struct UnitView {
    pack: String,
    skill: String,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
}

#[derive(serde::Serialize)]
struct StrayView {
    name: String,
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
}

#[derive(serde::Serialize)]
struct DiscoveryView {
    entries: Vec<EntryView>,
    extensions: Vec<ExtensionView>,
}

#[derive(serde::Serialize)]
struct EntryView {
    name: String,
    label: String,
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
}

#[derive(serde::Serialize)]
struct ExtensionView {
    path: String,
    source: &'static str,
}

fn status(registry: &Registry, args: &[String]) -> Result<(), String> {
    let selected = positional(args).get(1).cloned();
    if has(args, "json") {
        // The machine-readable shape any surface reads — a dashboard panel, a doctor section,
        // a hook. Emitted by the same code path as the table so the two cannot disagree.
        let mut harnesses = Vec::with_capacity(registry.harnesses.len());
        for h in &registry.harnesses {
            let units = statuses_for(h, selected.as_deref())?
                .into_iter()
                .map(|row| UnitView {
                    pack: row.pack,
                    skill: row.skill,
                    status: row.status.as_str(),
                    detail: row.detail,
                })
                .collect();
            let discovered = match h.model {
                Model::Registry => {
                    let d = pi::pi_discovery()?;
                    Some(DiscoveryView {
                        entries: d
                            .entries
                            .into_iter()
                            .map(|e| EntryView {
                                name: e.name,
                                label: e.label,
                                kind: e.kind,
                                detail: e.detail,
                            })
                            .collect(),
                        extensions: d
                            .extensions
                            .into_iter()
                            .map(|x| ExtensionView {
                                path: x.path,
                                source: x.source,
                            })
                            .collect(),
                    })
                }
                Model::Materialized => None,
            };
            harnesses.push(HarnessView {
                id: h.id,
                label: h.label,
                installed: is_installed(h),
                model: h.model.as_str(),
                marker: h.marker.display().to_string(),
                destination: match h.model {
                    Model::Materialized => Some(h.config.destination.display().to_string()),
                    Model::Registry => None,
                },
                cli: h.cli,
                units,
                discovered,
                unowned: foreign_at(h)?
                    .into_iter()
                    .map(|s| StrayView {
                        name: s.name,
                        kind: s.kind,
                        detail: s.detail,
                    })
                    .collect(),
            });
        }
        let view = PacksView {
            measured_at: now_iso(),
            harnesses,
            unsupported: registry
                .unsupported
                .iter()
                .map(|u| UnsupportedView {
                    id: u.id,
                    label: u.label,
                    why: u.why,
                })
                .collect(),
        };
        let text = serde_json::to_string_pretty(&view).map_err(|e| e.to_string())?;
        println!("{text}");
        return Ok(());
    }

    let harnesses = selected_harnesses(registry, args)?;
    if harnesses.is_empty() {
        println!("no harness is installed; nothing to compare");
        return Ok(());
    }

    let mut per_harness: Vec<(&str, BTreeMap<String, StatusRow>)> = Vec::new();
    let mut keys: Vec<String> = Vec::new();
    for h in &harnesses {
        let mut map = BTreeMap::new();
        for row in statuses_for(h, selected.as_deref())? {
            let key = format!("{}/{}", row.pack, row.skill);
            if !keys.contains(&key) {
                keys.push(key.clone());
            }
            map.insert(key, row);
        }
        per_harness.push((h.id, map));
    }

    let width = keys
        .iter()
        .map(|k| k.chars().count() + 2)
        .max()
        .unwrap_or(0)
        .max(28);
    let mut header = " ".repeat(width);
    for h in &harnesses {
        header.push_str(&format!("{:<10}", h.id));
    }
    println!("{header}");
    let mut previous_pack = String::new();
    for key in &keys {
        let (pack, name) = key.split_once('/').unwrap_or((key.as_str(), ""));
        if *pack != previous_pack {
            println!("{pack}");
            previous_pack = pack.to_owned();
        }
        let mut cells = String::new();
        for (_, map) in &per_harness {
            let cell = match map.get(key) {
                Some(row) => mark(&row.status).to_owned(),
                None => " ".to_owned(),
            };
            cells.push_str(&format!("{cell:<10}"));
        }
        println!("  {:<width$}{cells}", name, width = width - 2);
    }
    println!("{MARK_LEGEND}");

    // An absent harness holding deployed copies is invisible in the matrix above, because the
    // matrix only shows harnesses that are installed. It is also the condition this tool was
    // written for, so it gets its own line.
    for harness in &registry.harnesses {
        if is_installed(harness) || harness.model != Model::Materialized {
            continue;
        }
        let deployed: Vec<StatusRow> = statuses_for(harness, None)?
            .into_iter()
            .filter(|row| row.status != SkillStatus::NotDeployed)
            .collect();
        if deployed.is_empty() {
            continue;
        }
        let packs: BTreeSet<String> = deployed.iter().map(|r| r.pack.clone()).collect();
        println!(
            "\n{} is NOT installed (no {}), and {} units from {} Pack(s) are deployed at {}:",
            harness.label,
            harness.marker.display(),
            deployed.len(),
            packs.len(),
            harness.config.destination.display()
        );
        println!("  {}", packs.iter().cloned().collect::<Vec<_>>().join(", "));
        let pi = registry.by_id("pi")?;
        let home = std::env::var("HOME").unwrap_or_default();
        let pi_reads_destination = is_installed(pi)
            && harness.config.destination == Path::new(&home).join(".agents").join("skills");
        if pi_reads_destination {
            println!(
                "  pi IS installed and discovers {} — these units are loaded by pi right now.",
                harness.config.destination.display()
            );
            println!(
                "  If pi should keep them, deploy them there FIRST: tools/packs-pi deploy {}.",
                packs.iter().cloned().collect::<Vec<_>>().join(" ")
            );
            println!(
                "  Removing without that step strips them from pi without replacement: {} remove {}",
                harness.cli,
                packs.iter().cloned().collect::<Vec<_>>().join(" ")
            );
        } else {
            println!(
                "  Nothing on this machine reads them. Remove: {} remove {}",
                harness.cli,
                packs.iter().cloned().collect::<Vec<_>>().join(" ")
            );
        }
    }

    // Pi's discovery roots are live surfaces with no ledger entry, and a registry harness has
    // no destination section above — so discovered skills and extensions get their own lines.
    if harnesses.iter().any(|h| h.id == "pi") {
        let discovery = pi::pi_discovery()?;
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let any_discovered = !discovery.entries.is_empty()
            || discovery.extensions.iter().any(|e| e.source != "ledger");
        if any_discovered {
            println!("\npi loads these outside settings.json (discovered, not ledger-owned):");
            for d in &discovery.entries {
                let key = format!("{}/{}", d.label, d.name);
                if !seen.insert(key.clone()) {
                    continue;
                }
                let detail = d
                    .detail
                    .as_ref()
                    .map(|s| format!(", {s}"))
                    .unwrap_or_default();
                println!("  ~ {key}  ({}{detail})", d.kind);
            }
            for e in &discovery.extensions {
                if e.source == "ledger" {
                    continue;
                }
                let source = if e.source == "both" {
                    "ledger + discovered"
                } else {
                    "discovered, not in the ledger"
                };
                println!("  ~ {}  (extension; {source})", e.path);
            }
        }
    }

    for h in &harnesses {
        let strays = foreign_at(h)?;
        if strays.is_empty() {
            continue;
        }
        println!("\n{}: at the destination, not owned by any Pack", h.label);
        for s in &strays {
            let detail = s
                .detail
                .as_ref()
                .map(|d| format!("  {d}"))
                .unwrap_or_default();
            println!("  {:<8} {}{detail}", s.kind, s.name);
        }
        if strays.iter().any(|s| s.kind == "copy") {
            println!(
                "  a copy is a promote candidate: tools/harnesses promote <name> --pack <pack> --from {}",
                h.id
            );
        }
    }
    Ok(())
}

// ---- unowned files at a destination --------------------------------------------------------

#[derive(Debug, Clone)]
struct Stray {
    name: String,
    kind: &'static str,
    detail: Option<String>,
}

/// A marker that says some other installer owns this directory. Promoting one of these would
/// fork it: the clone re-syncs from its own remote and the tool install rewrites the directory
/// on upgrade.
const EXTERNAL_MARKERS: [&str; 2] = [".git", ".graphify_version"];

/// Skill directories at a harness destination that no Pack ledger claims.
fn foreign_at(harness: &Harness) -> Result<Vec<Stray>, String> {
    if harness.model != Model::Materialized {
        return Ok(Vec::new());
    }
    let config = &harness.config;
    if !config.destination.exists() {
        return Ok(Vec::new());
    }
    let state = engine::read_state(config)?;
    let mut owned: BTreeSet<String> = BTreeSet::new();
    for record in state.packs.values() {
        for key in record.skills.keys() {
            owned.insert(key.clone());
        }
    }
    let entries = std::fs::read_dir(&config.destination)
        .map_err(|e| format!("{}: {e}", config.destination.display()))?;
    let mut strays = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || owned.contains(&name) {
            continue;
        }
        let full = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let is_link = file_type.is_symlink();
        if !is_link && !file_type.is_dir() {
            continue;
        }
        let marker = if is_link {
            None
        } else {
            EXTERNAL_MARKERS.iter().find(|m| full.join(m).exists())
        };
        strays.push(Stray {
            name,
            kind: if is_link {
                "symlink"
            } else if marker.is_some() {
                "external"
            } else {
                "copy"
            },
            detail: if is_link {
                std::fs::read_link(&full)
                    .ok()
                    .map(|t| format!("→ {}", t.display()))
            } else {
                marker.map(|m| format!("carries {m}; another installer owns it"))
            },
        });
    }
    // Sorted by name, where TypeScript used localeCompare. Byte order is deterministic and
    // differs only for names that mix case or non-ASCII.
    strays.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(strays)
}

// ---- drift ---------------------------------------------------------------------------------

fn file_digest(path: &Path) -> Result<String, String> {
    use sha2::{Digest as _, Sha256};
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let hex: String = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Ok(hex[..12].to_owned())
}

fn walk(root: &Path, prefix: &str, out: &mut Vec<String>) -> Result<(), String> {
    let entries = std::fs::read_dir(root).map_err(|e| format!("{}: {e}", root.display()))?;
    let mut entries: Vec<std::fs::DirEntry> = entries.flatten().collect();
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == "__pycache__" || name.starts_with(".DS_Store") {
            continue;
        }
        let rel = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        let full = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            walk(&full, &rel, out)?;
        } else {
            out.push(rel);
        }
    }
    Ok(())
}

fn drift(registry: &Registry, args: &[String]) -> Result<(), String> {
    let selected = positional(args).get(1).cloned();
    let root = std::env::var("SJEL_ROOT").unwrap_or_default();
    let mut found = false;
    for harness in selected_harnesses(registry, args)? {
        if harness.model != Model::Materialized {
            continue;
        }
        let config = &harness.config;
        for row in statuses_for(harness, selected.as_deref())? {
            if !matches!(
                row.status,
                SkillStatus::Drifted | SkillStatus::Outdated | SkillStatus::Missing
            ) {
                continue;
            }
            found = true;
            println!(
                "\n{} · {}/{} — {}",
                harness.label,
                row.pack,
                row.skill,
                row.status.as_str()
            );
            let unit = engine::pack_units(config, &row.pack)?
                .into_iter()
                .find(|u| u.key == row.skill);
            let Some(unit) = unit else {
                continue;
            };
            if !unit.destination.exists() {
                println!("  the destination is gone: {}", unit.destination.display());
                continue;
            }
            let wanted = engine::desired_files(config, &row.pack, &unit)?;
            let mut walked = Vec::new();
            walk(&unit.destination, "", &mut walked)?;
            let mut actual: BTreeSet<String> = walked.into_iter().collect();
            for (rel, file) in &wanted {
                let there = unit.destination.join(rel);
                if !actual.contains(rel) {
                    println!("  missing at destination  {rel}");
                    continue;
                }
                actual.remove(rel);
                if file_digest(&file.absolute_path)? != file_digest(&there)? {
                    println!("  differs                 {rel}");
                    if has(args, "diff") {
                        let out = Command::new("diff")
                            .arg("-u")
                            .arg(&file.absolute_path)
                            .arg(&there)
                            .output()
                            .map_err(|e| format!("diff: {e}"))?;
                        print!("{}", String::from_utf8_lossy(&out.stdout));
                    }
                }
            }
            for rel in &actual {
                println!("  only at destination     {rel}");
            }
            println!("  source:      {}", relative(&root, &unit.source_root));
            println!("  destination: {}", unit.destination.display());
            println!(
                "  discard it:  {} sync {}   (overwrites the destination)",
                harness.cli, row.pack
            );
            println!(
                "  keep it:     tools/harnesses accept {} {} --from {}",
                row.pack, row.skill, harness.id
            );
        }
    }
    if !found {
        println!("no drift: every deployed unit matches its Pack source");
    }
    Ok(())
}

fn relative(root: &str, path: &Path) -> String {
    match path.strip_prefix(root) {
        Ok(rel) => rel.display().to_string(),
        Err(_) => path.display().to_string(),
    }
}

// ---- timestamps ----------------------------------------------------------------------------

// `now_iso` lives in `crate::time` since tools/updates needed the same string; `use super::*`
// below still reaches it.
use crate::time::now_iso;

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn positional_drops_flags_and_their_values() {
        assert_eq!(
            positional(&args(&["status", "writing", "--json"])),
            vec!["status", "writing"]
        );
        assert_eq!(
            positional(&args(&["status", "--harness", "claude", "writing"])),
            vec!["status", "writing"]
        );
        assert_eq!(
            positional(&args(&[
                "promote", "trim", "--pack", "writing", "--from", "claude"
            ])),
            vec!["promote", "trim", "claude"]
        );
    }

    #[test]
    fn the_timestamp_is_iso_with_milliseconds() {
        let now = now_iso();
        assert_eq!(now.len(), 24, "{now}");
        assert!(now.ends_with('Z'), "{now}");
        assert_eq!(&now[10..11], "T");
    }
}
