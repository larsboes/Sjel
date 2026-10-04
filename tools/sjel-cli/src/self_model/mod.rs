//! `tools/self` — Sjel's self-model: what this repository contains, what is wired to what, where
//! each upstream stands, and how much code each unit holds. One committed artifact (`self.json`)
//! plus a query surface over it. Ported from `tools/self.ts` and `tools/lib/self-model.ts` on
//! 2026-10-04; `model.rs` is the pure half.
//!
//! What is committed vs fused on read is the load-bearing distinction:
//!
//!   committed   structure, provenance and coupling — all derived from tracked files, so two
//!               runs on an unchanged tree are byte-identical and the artifact survives a fresh
//!               clone.
//!   fused       per-unit code counts and the graph accounting (both rolled up from
//!               `graphify-out/`, which is git-ignored and machine-local), live process health
//!               (sjel-status owns it) and open issue counts (the tracker owns them).
//!
//! Nothing fused is ever written into the artifact. `tools/doctor` runs `tools/self check` on
//! every invocation, which is why this moved: it was a bun start-up inside a Rust tool's path.
//!
//!   tools/self generate          regenerate self.json from the working tree
//!   tools/self status            one row per unit (--online adds open issue counts)
//!   tools/self explain <unit>    wiring, code size, provenance for one unit
//!   tools/self coupling          what is compiled into what, with evidence
//!   tools/self check             is the committed self.json still current?
//!   tools/self -h                this help
//!
//! Exit 0 = fine, 1 = stale (`check`) or an unknown unit (`explain`).

mod model;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use model::{
    coupling_from_cargo, coupling_from_rust_path, merge_coupling, roll_up, GraphNode,
    MergedCoupling, SourceCoupling, UnitKind,
};

const HELP: &str = "\
tools/self — Sjel's self-model: structure, coupling, provenance, code size.

  tools/self generate          regenerate self.json from the working tree
  tools/self status            one row per unit (--online adds open issue counts)
  tools/self explain <unit>    wiring, code size, provenance for one unit
  tools/self coupling          what is compiled into what, with evidence
  tools/self check             is the committed self.json still current? (exit 1 if not)

  --json                       machine-readable output for status/explain/coupling
  --out <path>                 generate writes there instead of self.json
  --against <path>             check this tree against that artifact instead of self.json

Code size is fused on read from graphify-out/, which is git-ignored: status and explain show it
where a graph exists, and self.json never carries it. Run tools/graphify.sh to build one.
";

/// The artifact's shape. Bump `schema` when a consumer would need to care.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelfModel {
    pub schema: u32,
    /// Deliberately NOT a timestamp: a generated-at field would make every run differ.
    pub generator: String,
    pub units: Vec<Unit>,
    /// Compile-time coupling: what is pulled into what. Distinct from service `requires`.
    pub coupling: Vec<MergedCoupling>,
    /// url/verdict/license/why is the whole register; `pin` was deleted 2026-09-02 (Q77).
    pub upstreams: Vec<Upstream>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Unit {
    pub name: String,
    pub kind: UnitKind,
    /// Present for anything with a `service.toml`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service: Option<Service>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Service {
    pub kind: String,
    pub requires: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Upstream {
    pub name: String,
    pub verdict: String,
}

/// Per-unit code counts and the graph accounting. Fused on read, never committed.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CodeLayer {
    pub by_unit: BTreeMap<String, CodeCount>,
    pub nodes: usize,
    pub external: usize,
    /// Paths the graph still holds a node for and the tree no longer has.
    pub stale: Vec<String>,
    pub unmatched: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeCount {
    pub files: usize,
    pub nodes: usize,
}

pub fn run(args: &[String]) -> ExitCode {
    let root = match std::env::var("SJEL_ROOT").ok().filter(|r| !r.is_empty()) {
        Some(root) => PathBuf::from(root),
        None => {
            eprintln!("tools/self: SJEL_ROOT is unset — run tools/self, which sets it");
            return ExitCode::from(2);
        }
    };
    let self_json = root.join("self.json");
    let want_json = args.iter().any(|arg| arg == "--json");
    let online = args.iter().any(|arg| arg == "--online");
    // The first argument that is not a flag, as the TypeScript's `args.find(a => !a.startsWith("-"))`
    // was — including its quirk that `--out <path> generate` names the path, not the verb.
    let cmd = args
        .iter()
        .find(|arg| !arg.starts_with('-'))
        .map(String::as_str)
        .unwrap_or("status");

    if args.iter().any(|arg| arg == "-h" || arg == "--help") {
        println!("{HELP}");
        return ExitCode::SUCCESS;
    }

    match cmd {
        "generate" => match generate(&root, &self_json, args) {
            Ok(()) => ExitCode::SUCCESS,
            Err(message) => {
                eprintln!("{message}");
                ExitCode::from(1)
            }
        },
        "check" => match check(&root, &self_json, args) {
            Ok(true) => {
                println!("self.json is current.");
                ExitCode::SUCCESS
            }
            Ok(false) => ExitCode::from(1),
            Err(message) => {
                eprintln!("{message}");
                ExitCode::from(1)
            }
        },
        _ => match query(&root, &self_json, cmd, args, want_json, online) {
            Ok(code) => ExitCode::from(code),
            Err(message) => {
                eprintln!("{message}");
                ExitCode::from(1)
            }
        },
    }
}

/// The one flag that takes a value, read the way `args.indexOf(name)` was: absent is the
/// default, present with nothing after it is an error.
fn flag_value<'a>(args: &'a [String], name: &str) -> Option<Option<&'a str>> {
    let at = args.iter().position(|arg| arg == name)?;
    Some(args.get(at + 1).map(String::as_str))
}

fn generate(root: &Path, self_json: &Path, args: &[String]) -> Result<(), String> {
    let target = match flag_value(args, "--out") {
        None => self_json.to_path_buf(),
        Some(None) => return Err("tools/self generate --out needs a path".to_string()),
        Some(Some(path)) => PathBuf::from(path),
    };
    let out = serialize(&build(root)?);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
    }
    fs::write(&target, &out).map_err(|error| format!("{}: {error}", target.display()))?;
    println!("wrote {} ({} bytes)", target.display(), out.len());
    Ok(())
}

fn check(root: &Path, self_json: &Path, args: &[String]) -> Result<bool, String> {
    let compare = match flag_value(args, "--against") {
        None => self_json.to_path_buf(),
        Some(None) => return Err("tools/self check --against needs a path".to_string()),
        Some(Some(path)) => PathBuf::from(path),
    };
    let Ok(committed) = fs::read_to_string(&compare) else {
        return Err(if compare == self_json {
            "self.json is missing. Run: tools/self generate".to_string()
        } else {
            format!("cannot read {}", compare.display())
        });
    };
    let fresh = serialize(&build(root)?);
    if fresh == committed {
        return Ok(true);
    }
    eprintln!("self.json is stale. Run: tools/self generate");
    print_drift(&committed, &fresh);
    Ok(false)
}

/// Show what actually differs, not only that something does. `diff -u` rather than a diff engine
/// written here, capped because a terminal full of JSON is the same non-answer as no diff at all.
fn print_drift(committed: &str, fresh: &str) {
    const CAP: usize = 120;
    let dir = std::env::temp_dir().join(format!("sjel-self-check.{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    if fs::create_dir_all(&dir).is_err() {
        eprintln!("  (could not render a diff: no scratch directory)");
        return;
    }
    let left = dir.join("committed");
    let right = dir.join("fresh");
    if fs::write(&left, committed).is_err() || fs::write(&right, fresh).is_err() {
        eprintln!("  (could not render a diff: could not write the scratch files)");
        let _ = fs::remove_dir_all(&dir);
        return;
    }
    let output = Command::new("diff")
        .args([
            "-u",
            "-L",
            "self.json (committed)",
            "-L",
            "self.json (this tree)",
        ])
        .arg(&left)
        .arg(&right)
        .output();
    let _ = fs::remove_dir_all(&dir);
    let Ok(output) = output else {
        eprintln!("  (could not render a diff: diff could not run)");
        return;
    };
    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.split('\n').filter(|line| !line.is_empty()).collect();
    if lines.is_empty() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        eprintln!(
            "  (could not render a diff: {})",
            if stderr.is_empty() {
                "diff produced no output".to_string()
            } else {
                stderr
            }
        );
        return;
    }
    for line in lines.iter().take(CAP) {
        eprintln!("  {line}");
    }
    if lines.len() > CAP {
        eprintln!("  ... {} more diff lines", lines.len() - CAP);
    }
}

/// `status`, `explain` and `coupling`, which read the committed artifact when there is one and
/// build from the tree when there is not.
fn query(
    root: &Path,
    self_json: &Path,
    cmd: &str,
    args: &[String],
    want_json: bool,
    online: bool,
) -> Result<u8, String> {
    let model = match fs::read_to_string(self_json) {
        Ok(text) => serde_json::from_str::<SelfModel>(&text)
            .map_err(|error| format!("{}: {error}", self_json.display()))?,
        Err(_) => build(root)?,
    };
    // Fused on read: absent on any machine that has not built graphify-out/. Status and explain
    // degrade to a dash rather than to a number nobody can check.
    let code = read_code_layer(root);

    match cmd {
        "coupling" => {
            if want_json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&model.coupling).map_err(|e| e.to_string())?
                );
                return Ok(0);
            }
            println!(
                "Compile-time coupling — what is pulled into what ({} pairs).",
                model.coupling.len()
            );
            println!("Distinct from service `requires`, which is what must be RUNNING.\n");
            for edge in &model.coupling {
                println!(
                    "  {:14} -> {:14} [{}]",
                    edge.from,
                    edge.to,
                    edge.kinds.join("+")
                );
            }
            Ok(0)
        }
        "explain" => {
            // `args.find(a => !a.startsWith("-") && a !== "explain")`, which renders an absent
            // name as the string "undefined" in the error below.
            let name = args
                .iter()
                .find(|arg| !arg.starts_with('-') && arg.as_str() != "explain")
                .map(String::as_str)
                .unwrap_or("undefined");
            let Some(unit) = model.units.iter().find(|unit| unit.name == name) else {
                let known: Vec<&str> = model.units.iter().map(|u| u.name.as_str()).collect();
                return Err(format!(
                    "unknown unit '{name}'. Known: {}",
                    known.join(", ")
                ));
            };
            let counts = code
                .as_ref()
                .and_then(|layer| layer.by_unit.get(&unit.name));
            if want_json {
                let payload = ExplainJson {
                    name: &unit.name,
                    kind: &unit.kind,
                    service: unit.service.as_ref(),
                    code: counts,
                };
                println!(
                    "{}",
                    serde_json::to_string_pretty(&payload).map_err(|e| e.to_string())?
                );
                return Ok(0);
            }
            println!("{} ({})", unit.name, unit.kind.as_str());
            if let Some(counts) = counts {
                println!(
                    "  code       {} files, {} graph nodes",
                    counts.files, counts.nodes
                );
            }
            if let Some(service) = &unit.service {
                let port = service
                    .port
                    .as_ref()
                    .map(|port| format!(" port={port}"))
                    .unwrap_or_default();
                println!("  service    kind={}{port}", service.kind);
                println!(
                    "  requires   {} (must be running)",
                    if service.requires.is_empty() {
                        "—".to_string()
                    } else {
                        service.requires.join(", ")
                    }
                );
            }
            let outs: Vec<&str> = model
                .coupling
                .iter()
                .filter(|edge| edge.from == unit.name)
                .map(|edge| edge.to.as_str())
                .collect();
            let ins: Vec<&str> = model
                .coupling
                .iter()
                .filter(|edge| edge.to == unit.name)
                .map(|edge| edge.from.as_str())
                .collect();
            println!(
                "  compiles in {}",
                if outs.is_empty() {
                    "—".to_string()
                } else {
                    outs.join(", ")
                }
            );
            println!(
                "  used by     {}",
                if ins.is_empty() {
                    "—".to_string()
                } else {
                    ins.join(", ")
                }
            );
            Ok(0)
        }
        _ => {
            let work = if online {
                open_issues_by_unit(root)
            } else {
                None
            };
            if want_json {
                let payload = StatusJson {
                    schema: model.schema,
                    generator: &model.generator,
                    units: &model.units,
                    coupling: &model.coupling,
                    upstreams: &model.upstreams,
                    code: code.as_ref().map(|layer| &layer.by_unit),
                    work: work.as_ref().map(|work| &work.counts),
                };
                println!(
                    "{}",
                    serde_json::to_string_pretty(&payload).map_err(|e| e.to_string())?
                );
                return Ok(0);
            }
            println!(
                "Sjel self-model — {} units, {} coupling pairs",
                model.units.len(),
                model.coupling.len()
            );
            match &code {
                Some(layer) => println!(
                    "Code graph: {} nodes, {} external, {} stale\n",
                    layer.nodes,
                    layer.external,
                    layer.stale.len()
                ),
                None => println!("Code graph: absent (run tools/graphify.sh)\n"),
            }
            let mut header = format!(
                "  {:18}{:12}{:>6}{:>7}{:>12}",
                "unit", "kind", "files", "port", "requires"
            );
            if work.is_some() {
                header.push_str("   open");
            }
            println!("{header}");
            for unit in &model.units {
                let files = code
                    .as_ref()
                    .and_then(|layer| layer.by_unit.get(&unit.name))
                    .map(|counts| counts.files.to_string())
                    .unwrap_or_else(|| "—".to_string());
                let port = unit
                    .service
                    .as_ref()
                    .and_then(|service| service.port.clone())
                    .unwrap_or_else(|| "—".to_string());
                let requires = unit
                    .service
                    .as_ref()
                    .map(|service| {
                        if service.requires.is_empty() {
                            "—".to_string()
                        } else {
                            service.requires.join(",")
                        }
                    })
                    .unwrap_or_else(|| "—".to_string());
                let mut row = format!(
                    "  {:18}{:12}{:>6}{:>7}{:>12}",
                    unit.name,
                    unit.kind.as_str(),
                    files,
                    port,
                    requires
                );
                if let Some(work) = &work {
                    row.push_str(&format!(
                        "{:>7}",
                        work.counts.get(&unit.name).copied().unwrap_or(0)
                    ));
                }
                println!("{row}");
            }
            if let Some(work) = &work {
                println!("\n  {} open issues match no unit prefix.", work.unmatched);
            }
            if let Some(layer) = &code {
                if !layer.stale.is_empty() {
                    println!(
                        "\n  ⚠ {} graph paths no longer exist — run tools/graphify.sh",
                        layer.stale.len()
                    );
                }
            }
            Ok(0)
        }
    }
}

/// `status --json` carries the artifact plus the two fused layers.
#[derive(Serialize)]
struct StatusJson<'a> {
    schema: u32,
    generator: &'a str,
    units: &'a [Unit],
    coupling: &'a [MergedCoupling],
    upstreams: &'a [Upstream],
    code: Option<&'a BTreeMap<String, CodeCount>>,
    work: Option<&'a BTreeMap<String, usize>>,
}

/// `explain --json` is the unit with its counts, when a graph exists.
#[derive(Serialize)]
struct ExplainJson<'a> {
    name: &'a str,
    kind: &'a UnitKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    service: Option<&'a Service>,
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<&'a CodeCount>,
}

/// Stable stringify: the struct field order above IS the artifact's key order, so two runs on an
/// unchanged tree are byte-identical.
fn serialize(model: &SelfModel) -> String {
    let mut text = serde_json::to_string_pretty(model).expect("a SelfModel is serializable");
    text.push('\n');
    text
}

// ---- reading the world ---------------------------------------------------------------------

fn git(root: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Public-safe graph input boundary: only Git-tracked paths may become internal metadata.
fn read_tracked_paths(root: &Path) -> BTreeSet<String> {
    git(root, &["ls-files", "-z"])
        .map(|text| {
            text.split('\0')
                .filter(|path| !path.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// Walk tracked sources for ground-truth coupling.
///
/// `git ls-files` rather than a filesystem glob: an untracked scratch file is not part of what
/// this repository IS, and including it would make the committed artifact depend on whatever
/// happens to be lying in the working tree.
fn read_coupling(root: &Path) -> Vec<SourceCoupling> {
    let Some(listing) = git(
        root,
        &["ls-files", "--", "*.rs", "Cargo.toml", "*/Cargo.toml"],
    ) else {
        return Vec::new();
    };
    let mut edges = Vec::new();
    for file in listing.split('\n').filter(|line| !line.is_empty()) {
        let Ok(text) = fs::read_to_string(root.join(file)) else {
            continue;
        };
        if file.ends_with(".rs") {
            edges.extend(coupling_from_rust_path(file, &text));
        }
        if file.ends_with("Cargo.toml") {
            edges.extend(coupling_from_cargo(file, &text));
        }
    }
    edges
}

/// Declared service facts, read from `capabilities/<name>/service.toml` in this checkout.
///
/// NOT from `tools/capability.sh registry`, which merges the running machine's `machine.toml`
/// overrides — a port in `self.json` would then be whatever the generating machine resolved
/// rather than what the repository declares, and `check` could not run anywhere without an
/// overlay. Overlay capabilities are absent by construction: they live in the overlay's tree,
/// which this never reads, because a capability name is itself a fact about a private deployment.
fn read_declared_services(root: &Path, tracked: &BTreeSet<String>) -> BTreeMap<String, Service> {
    let mut out = BTreeMap::new();
    for path in tracked {
        if !path.ends_with("/service.toml") {
            continue;
        }
        let segments: Vec<&str> = path.split('/').collect();
        let name = match segments.as_slice() {
            ["capabilities", name, "service.toml"] => *name,
            [name, "service.toml"] => *name,
            _ => continue,
        };
        // A manifest that does not parse is tools/doctor's finding, not a reason to abort.
        let Ok(text) = fs::read_to_string(root.join(path)) else {
            continue;
        };
        let Ok(parsed) = text.parse::<toml::Table>() else {
            continue;
        };
        let requires = match parsed.get("requires") {
            Some(toml::Value::Array(items)) => items.iter().map(toml_text).collect(),
            _ => Vec::new(),
        };
        out.insert(
            name.to_string(),
            Service {
                // service-runner.sh treats an absent kind as a container; the manifest and the
                // model have to agree on that default.
                kind: parsed
                    .get("kind")
                    .map(toml_text)
                    .unwrap_or_else(|| "container".to_string()),
                requires,
                port: parsed.get("port").map(toml_text),
                image: parsed.get("image").map(toml_text),
            },
        );
    }
    out
}

/// `String(value)` for a TOML scalar, as the TypeScript's `String(parsed.kind)` was.
fn toml_text(value: &toml::Value) -> String {
    match value {
        toml::Value::String(text) => text.clone(),
        toml::Value::Integer(number) => number.to_string(),
        toml::Value::Float(number) => number.to_string(),
        toml::Value::Boolean(flag) => flag.to_string(),
        other => other.to_string(),
    }
}

fn read_upstreams(root: &Path) -> Vec<Upstream> {
    let Ok(text) = fs::read_to_string(root.join("upstreams.toml")) else {
        return Vec::new();
    };
    let Ok(parsed) = text.parse::<toml::Table>() else {
        return Vec::new();
    };
    let mut out: Vec<Upstream> = parsed
        .iter()
        .map(|(name, value)| Upstream {
            name: name.clone(),
            verdict: value.get("verdict").map(toml_text).unwrap_or_default(),
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// The per-unit code counts and the graph accounting, read from `graphify-out/graph.json`.
///
/// `SJEL_SELF_GRAPH` is a test seam: the obvious way to probe this rollup is to plant a graph,
/// and on this machine `graphify-out/` holds a real one that took a run to build, so a test that
/// wrote there to prove something would destroy the thing it was protecting.
fn read_code_layer(root: &Path) -> Option<CodeLayer> {
    let path = std::env::var("SJEL_SELF_GRAPH")
        .ok()
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("graphify-out").join("graph.json"));
    let text = fs::read_to_string(path).ok()?;
    let parsed: Value = serde_json::from_str(&text).ok()?;
    let nodes: Vec<GraphNode> = parsed
        .get("nodes")
        .cloned()
        .and_then(|nodes| serde_json::from_value(nodes).ok())
        .unwrap_or_default();
    let tracked = read_tracked_paths(root);
    let tracked_predicate = |path: &str| tracked.contains(path);
    let exists_predicate = |path: &str| root.join(path).exists();
    let rollup = roll_up(&nodes, &tracked_predicate, &exists_predicate);
    let mut by_unit = BTreeMap::new();
    for unit in &rollup.units {
        by_unit.insert(
            unit.name.clone(),
            CodeCount {
                files: unit.files,
                nodes: unit.nodes,
            },
        );
    }
    Some(CodeLayer {
        by_unit,
        nodes: rollup.admitted_nodes,
        external: rollup.buckets.external,
        stale: rollup.buckets.stale,
        unmatched: rollup.buckets.unmatched,
    })
}

struct Work {
    counts: BTreeMap<String, usize>,
    unmatched: usize,
}

/// Open issues per unit, joined on the `<unit>:` title prefix the tracker already uses.
fn open_issues_by_unit(root: &Path) -> Option<Work> {
    let output = Command::new("gh")
        .args([
            "issue", "list", "--state", "open", "--limit", "200", "--json", "title",
        ])
        .current_dir(root)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let rows: Vec<Value> = serde_json::from_str(&String::from_utf8_lossy(&output.stdout)).ok()?;
    let pattern = Regex::new(r"^([A-Za-z0-9._-]+):").expect("static pattern");
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut unmatched = 0usize;
    for row in rows {
        let title = row.get("title").and_then(Value::as_str).unwrap_or_default();
        match pattern.captures(title) {
            Some(captures) => *counts.entry(captures[1].to_string()).or_insert(0) += 1,
            None => unmatched += 1,
        }
    }
    Some(Work { counts, unmatched })
}

/// Every directory under a unit root is a unit of that kind. A missing top-level directory is
/// not an error: a minimal install has no Packs.
fn add_unit_dirs(root: &Path, parent: &str, kind: UnitKind, into: &mut BTreeMap<String, UnitKind>) {
    let Ok(entries) = fs::read_dir(root.join(parent)) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.path().is_dir() {
            into.insert(
                entry.file_name().to_string_lossy().into_owned(),
                kind.clone(),
            );
        }
    }
}

/// The committed artifact: structure, provenance and coupling, all from tracked files.
fn build(root: &Path) -> Result<SelfModel, String> {
    let tracked = read_tracked_paths(root);
    let declared = read_declared_services(root, &tracked);

    // The unit inventory comes from the tracked tree, never from the code graph: deriving it from
    // the graph made the whole artifact depend on git-ignored graphify-out/, and a fresh clone
    // silently collapsed the unit list.
    let mut kind_by_unit: BTreeMap<String, UnitKind> = BTreeMap::new();
    add_unit_dirs(
        root,
        "capabilities",
        UnitKind::Capability,
        &mut kind_by_unit,
    );
    add_unit_dirs(root, "libs", UnitKind::Lib, &mut kind_by_unit);
    add_unit_dirs(root, "Packs", UnitKind::Pack, &mut kind_by_unit);
    for spine in ["dashboard", "tools", "schemas"] {
        if root.join(spine).exists() {
            kind_by_unit.insert(spine.to_string(), UnitKind::Spine);
        }
    }

    let mut names: BTreeSet<String> = kind_by_unit.keys().cloned().collect();
    names.extend(declared.keys().cloned());

    let units: Vec<Unit> = names
        .into_iter()
        .map(|name| {
            let service = declared.get(&name).cloned();
            let kind = kind_by_unit.get(&name).cloned().unwrap_or({
                if service.is_some() {
                    UnitKind::Capability
                } else {
                    UnitKind::Unknown
                }
            });
            Unit {
                name,
                kind,
                service,
            }
        })
        .collect();

    Ok(SelfModel {
        schema: 2,
        generator: "tools/self".to_string(),
        units,
        coupling: merge_coupling(&read_coupling(root)),
        upstreams: read_upstreams(root),
    })
}
