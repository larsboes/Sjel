//! `tools/harnesses` — Packs across every agent harness at once.
//!
//! Each `packs-<harness>` adapter owns one destination and answers about it alone. Nothing
//! answered the question an operator actually has: what is deployed WHERE, what has drifted,
//! and what is sitting in a harness Sjel does not know about. This asks every harness the same
//! question and prints one answer.
//!
//! Ported from tools/harnesses.ts: the read verbs (`list`, `status`, `drift`) on 2026-10-02,
//! the write verbs (`sync`, `use`, `promote`, `accept`) on 2026-10-04 with the pack-deploy
//! mutation engine and pi's settings registry. The TypeScript file and its test are deleted,
//! so this is the only reader and the only writer of every ledger it touches.
//!
//! Direction: Sjel is the source and `sync` is one-way, Sjel -> harness. The one move in the
//! other direction is `promote`, which is manual on purpose: a skill written inside a harness
//! is brought into a Pack only when a human decides it is worth sharing system-wide.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

mod agentfile;
pub mod engine;
mod frontmatter;
pub(crate) mod mutate;
pub(crate) mod pi;
pub(crate) mod pi_settings;
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

pub fn run(args: &[String]) -> ExitCode {
    let positional = positional(args);
    let verb = positional.first().map_or("list", String::as_str);
    match verb {
        "help" | "-h" | "--help" => {
            println!("{HELP}");
            return ExitCode::SUCCESS;
        }
        "list" | "status" | "drift" | "sync" | "use" | "promote" | "accept" => {}
        other => {
            eprintln!("{HELP}");
            eprintln!("harnesses: unknown verb '{other}'");
            return ExitCode::from(1);
        }
    }
    let root = match std::env::var("SJEL_ROOT").ok().filter(|r| !r.is_empty()) {
        Some(r) => PathBuf::from(r),
        None => {
            eprintln!("harnesses: SJEL_ROOT is unset — run tools/harnesses, which sets it");
            return ExitCode::from(2);
        }
    };
    let registry = Registry::new(&root);
    let result = match verb {
        "list" => list(&registry),
        "status" => status(&registry, args),
        "drift" => drift(&registry, args),
        "sync" => sync(&registry, args),
        "use" => use_profile(&registry, &root, args),
        "promote" => promote(&registry, &root, args),
        "accept" => accept(&registry, &root, args),
        _ => unreachable!(),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("harnesses: {e}");
            ExitCode::from(1)
        }
    }
}

// ---- the write verbs (tools/harnesses.ts, ported 2026-10-04) --------------------------------

/// One-way Sjel -> harness. `--all` is a flag, not a positional value, so the argv filter that
/// strips `--`-prefixed arguments can never swallow it.
fn sync(registry: &Registry, args: &[String]) -> Result<(), String> {
    let positional = positional(args);
    let pack = positional.get(1).map(String::as_str);
    let all = has(args, "all");
    if pack.is_some() && all {
        return Err("tools/harnesses sync: give a pack or --all, not both".to_owned());
    }
    if pack.is_none() && !all {
        return Err("usage: tools/harnesses sync <pack>|--all [--harness <id>]".to_owned());
    }
    for harness in selected_harnesses(registry, args)? {
        println!("{}:", harness.label);
        if harness.model == Model::Registry {
            // A registry harness deploys one pack per invocation, so this line is a hint to run
            // per pack rather than a loop this command could perform.
            println!(
                "  registry harness — run: {} deploy {}",
                harness.cli,
                if all { "<pack>" } else { pack.unwrap_or("") }
            );
            continue;
        }
        let config = &harness.config;
        let known: Vec<String> = engine::read_state(config)?.packs.keys().cloned().collect();
        for target in mutate::sync_targets(pack, all, &known) {
            match mutate::sync_pack(config, &target) {
                Ok(lines) => lines.iter().for_each(|l| println!("  {l}")),
                Err(e) => println!("  ✗ {target}: {e}"),
            }
        }
    }
    Ok(())
}

/// Activate a profile on every selected harness through the registry: pi rewrites settings.json
/// (skills AND extensions); materialized harnesses go through the shared engine, honouring
/// per-Pack skill subsets.
fn use_profile(registry: &Registry, root: &Path, args: &[String]) -> Result<(), String> {
    let positional = positional(args);
    let Some(name) = positional.get(1) else {
        return Err("usage: tools/harnesses use <profile> [--harness <id>]".to_owned());
    };
    let profiles = mutate::read_profiles_at(root)?;
    let Some(profile) = profiles.iter().find(|p| p.name == *name) else {
        return Err(format!("no such profile: '{name}'"));
    };
    for harness in selected_harnesses(registry, args)? {
        println!("── {}", harness.label);
        if harness.model == Model::Registry {
            for line in pi_settings::activate_profile_on_pi(&harness.config, name)? {
                println!("{line}");
            }
        } else {
            for line in mutate::activate_profile(&harness.config, profile)? {
                println!("  {line}");
            }
        }
    }
    Ok(())
}

fn is_skills_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    match trimmed.strip_prefix("skills") {
        Some(rest) => rest.trim_start().starts_with('='),
        None => false,
    }
}

/// The one harness -> Sjel move. Manual by design: a skill written inside a harness is brought
/// into a Pack only when a human decides it is worth sharing system-wide, and promote then claims
/// the live copy rather than replacing it.
fn promote(registry: &Registry, root: &Path, args: &[String]) -> Result<(), String> {
    let positional = positional(args);
    let usage = "usage: tools/harnesses promote <skill> --pack <pack> [--from <harness>]";
    let skill = positional.get(1).ok_or(usage)?;
    let pack = flag(args, "pack").ok_or(usage)?;
    let from = flag(args, "from").unwrap_or("claude");
    let harness = registry.by_id(from)?;
    if harness.model != Model::Materialized {
        return Err(format!(
            "{from} registers Pack paths in place; there is nothing to promote from it"
        ));
    }
    let config = &harness.config;
    let source = config.destination.join(skill);
    if !source.exists() {
        return Err(format!("{} does not exist", source.display()));
    }
    if fs::symlink_metadata(&source).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(format!(
            "{} is a symlink: another installer owns that skill and a copy here would silently pin it",
            source.display()
        ));
    }
    let state = engine::read_state(config)?;
    if let Some((owner, _)) = state
        .packs
        .iter()
        .find(|(_, record)| record.skills.contains_key(skill))
    {
        return Err(format!(
            "{skill} is already owned by Pack '{owner}'; nothing to promote"
        ));
    }

    let pack_dir = root.join("Packs").join(pack);
    let manifest = pack_dir.join("pack.toml");
    // Read the manifest rather than asking `exists` here and reading it after the copy: two
    // answers to the same question, taken from two instants, and the second one is what gets
    // written back. Every refusal below now happens before a single file is copied.
    let body = fs::read_to_string(&manifest)
        .map_err(|_| format!("no Pack at {}", relative(root, &pack_dir)))?;
    let Some(line) = body.split('\n').find(|l| is_skills_line(l)) else {
        return Err(format!(
            "{} has no skills = [...] line",
            relative(root, &manifest)
        ));
    };
    let updated = mutate::skills_line_with(line, skill)
        .map_err(|e| format!("{}: {e}", relative(root, &manifest)))?;
    let target = pack_dir.join("skills").join(skill);
    if target.exists() {
        return Err(format!("{} already exists", relative(root, &target)));
    }

    let mut files = Vec::new();
    walk(&source, "", &mut files)?;
    for rel in &files {
        let to = target.join(rel);
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        fs::copy(source.join(rel), &to).map_err(|e| format!("{}: {e}", to.display()))?;
    }
    // A splice, not a replacement pattern: a `$&` in the skill name would expand inside a
    // `String.replace` replacement and rewrite the line it was inserted into.
    fs::write(&manifest, body.replacen(line, &updated, 1))
        .map_err(|e| format!("{}: {e}", manifest.display()))?;

    println!("✓ copied {skill} → {}", relative(root, &target));
    println!("✓ added to {}", relative(root, &manifest));
    for message in mutate::adopt_pack(config, pack)? {
        println!("  {message}");
    }
    println!("\nThe live copy is now claimed, not replaced. Next: review the files, then");
    println!("deploy the Pack to the other harnesses that should carry it.");
    Ok(())
}

/// The second harness -> Sjel move: destination edits to a skill Sjel ALREADY owns. `sync` would
/// silently destroy such an edit, which is why this direction exists.
fn accept(registry: &Registry, root: &Path, args: &[String]) -> Result<(), String> {
    let positional = positional(args);
    let usage = "usage: tools/harnesses accept <pack> <skill> [--from <harness>]";
    let pack = positional.get(1).ok_or(usage)?;
    let skill = positional.get(2).ok_or(usage)?;
    let from = flag(args, "from").unwrap_or("claude");
    let harness = registry.by_id(from)?;
    if harness.model != Model::Materialized {
        return Err(format!(
            "{from} reads the Pack source in place; it has no copy to accept"
        ));
    }
    let config = &harness.config;
    let unit = engine::pack_units(config, pack)?
        .into_iter()
        .find(|u| u.key == *skill)
        .ok_or_else(|| format!("{pack} does not carry {skill}"))?;
    let deployed = engine::read_state(config)?
        .packs
        .get(pack)
        .is_some_and(|r| r.skills.contains_key(skill));
    if !deployed {
        return Err(format!(
            "{pack}/{skill} is not deployed to {from}; nothing to accept"
        ));
    }
    if !unit.destination.exists() {
        return Err(format!("{} does not exist", unit.destination.display()));
    }

    // Refuse to bury uncommitted work in the Pack source. The destination copy is about to
    // overwrite it, and git is the only undo this move has.
    let source_rel = relative(root, &unit.source_root);
    let dirty = Command::new("git")
        .args([
            "-C",
            &root.display().to_string(),
            "status",
            "--porcelain",
            "--",
            &source_rel,
        ])
        .output()
        .map_err(|e| format!("git: {e}"))?;
    let pending = String::from_utf8_lossy(&dirty.stdout).trim().to_owned();
    if !pending.is_empty() && !has(args, "force") {
        return Err(format!(
            "{source_rel} has uncommitted changes:\n{pending}\ncommit or stash them first, or pass --force to overwrite"
        ));
    }

    let mut incoming = Vec::new();
    walk(&unit.destination, "", &mut incoming)?;
    let mut existing = Vec::new();
    walk(&unit.source_root, "", &mut existing)?;
    for rel in &incoming {
        let to = unit.source_root.join(rel);
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        fs::copy(unit.destination.join(rel), &to).map_err(|e| format!("{}: {e}", to.display()))?;
    }
    let removed: Vec<&String> = existing.iter().filter(|r| !incoming.contains(r)).collect();
    for rel in &removed {
        fs::remove_file(unit.source_root.join(rel))
            .map_err(|e| format!("{}: {e}", unit.source_root.join(rel).display()))?;
    }

    println!(
        "✓ {} files copied into {}",
        incoming.len(),
        relative(root, &unit.source_root)
    );
    for rel in &removed {
        println!("  removed (absent at the destination): {rel}");
    }
    // The ledger still holds the pre-edit digest and would keep reporting drift that no longer
    // exists, so re-record it now that the two agree.
    println!("  {}", mutate::reconcile_unit(config, pack, &unit)?);
    println!(
        "\nReview before committing:  git -C {} diff -- {source_rel}",
        root.display()
    );
    println!("Then deploy the Pack to the other harnesses that carry it.");
    Ok(())
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
        // pi's rows are two channels: the settings registry, and the agent files it copies to a
        // flat destination. Both belong in the matrix — the registry alone reports a Pack current
        // while its agent files sit stale.
        Model::Registry => {
            let packs: Vec<String> = selected.map(str::to_owned).into_iter().collect();
            let mut rows = pi::registry_statuses(&harness.config, selected)?;
            rows.extend(pi_settings::agent_status_rows(&harness.config, &packs)?);
            Ok(rows)
        }
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

fn relative(root: impl AsRef<Path>, path: &Path) -> String {
    match path.strip_prefix(root.as_ref()) {
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
