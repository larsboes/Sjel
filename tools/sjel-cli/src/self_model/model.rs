//! The pure core of Sjel's self-model — `tools/lib/self-model.ts`, ported 2026-10-04.
//!
//! Rolling graphify's file-and-function graph up to unit-level facts, and reading real
//! compile-time coupling out of the two places that declare it. No I/O lives here: every
//! function takes its world as an argument (the node list, a path predicate, a file's text),
//! so the cases `tools/self.test.ts` held are unit tests here.
//!
//! One thing this deliberately does NOT do: derive coupling from graphify's own import edges.
//! That was built and removed — on this corpus every cross-unit import edge it produced was a
//! phantom, because graphify gives each unqualified symbol ONE global node owned by whichever
//! file it extracted first, so every `use std::collections::BTreeMap` became an edge into
//! whichever capability happened to own `btreemap`, and it missed all 20 files of real
//! capability→lib coupling. Coupling comes from `#[path]` attributes and Cargo path
//! dependencies, which are literal strings in tracked files.

use std::collections::{BTreeMap, BTreeSet};

use regex::Regex;
use serde::{Deserialize, Serialize};

/// The three nouns of CONTRIBUTING.md#three-architectural-nouns, plus the spine directories
/// that hold code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UnitKind {
    Capability,
    Lib,
    Spine,
    Pack,
    /// A declared service whose directory is not one of the three unit roots. The artifact says
    /// so rather than guessing.
    Unknown,
}

impl UnitKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Capability => "capability",
            Self::Lib => "lib",
            Self::Spine => "spine",
            Self::Pack => "pack",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unit {
    pub name: String,
    pub kind: UnitKind,
}

/// Extensions tried when a `source_file` names a module rather than a file.
///
/// graphify records the import specifier as written, so `dashboard/src/lib/api` is the node for
/// `api.ts` and `dashboard/src/lib/capabilities.svelte` is the node for
/// `capabilities.svelte.ts`. `.svelte.ts` must be tried before `.ts` would ever match, and it is
/// listed explicitly because appending `.ts` to `...capabilities.svelte` is the correct answer
/// only by coincidence of that naming convention.
const RESOLVE_EXTENSIONS: [&str; 8] = [
    ".ts",
    ".svelte.ts",
    ".tsx",
    ".js",
    ".mjs",
    ".svelte",
    ".rs",
    ".py",
];
const RESOLVE_INDEXES: [&str; 3] = ["/index.ts", "/index.js", "/mod.rs"];

/// Top-level directories that make a path Sjel-internal. Anything else is foreign.
const INTERNAL_ROOTS: [&str; 6] = [
    "capabilities",
    "libs",
    "dashboard",
    "Packs",
    "tools",
    "schemas",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathClass {
    Internal,
    External,
    Stale,
    Empty,
    Local,
}

#[derive(Debug, Clone)]
pub struct ClassifiedPath {
    pub class: PathClass,
    /// The resolved on-disk path — set only when `class` is [`PathClass::Internal`].
    pub path: Option<String>,
    /// What graphify actually wrote, kept for reporting.
    pub raw: String,
}

/// Classify one `source_file` into exactly one of five states, resolving extensions.
///
/// The split exists because "this path is not a file" conflates three different things and only
/// one of them is a defect:
///
/// - internal: a tracked file in this checkout (possibly after extension resolution)
/// - external: a foreign module specifier — a bare package name or a `$`-alias. Legitimate graph
///   content; a dependency graph is supposed to have foreign nodes.
/// - stale: looks internal (lives under a known root) but resolves to nothing. THIS is the
///   defect — a graph built before a refactor still naming a deleted file.
/// - empty: graphify emitted a node with no `source_file` at all.
/// - local: an existing but untracked path. Ignored rather than reported, so machine-local
///   graphify memory, cache and output can never enter committed metadata.
///
/// Both predicates are injected rather than read from disk so fixtures can describe a virtual
/// tracked tree plus ignored local output independently.
pub fn classify_path(
    raw: Option<&str>,
    tracked: &dyn Fn(&str) -> bool,
    exists: &dyn Fn(&str) -> bool,
) -> ClassifiedPath {
    let raw = raw.unwrap_or("");
    if raw.trim().is_empty() {
        return ClassifiedPath {
            class: PathClass::Empty,
            path: None,
            raw: raw.to_string(),
        };
    }

    let mut candidates: Vec<String> = vec![raw.to_string()];
    for suffix in RESOLVE_EXTENSIONS.iter().chain(RESOLVE_INDEXES.iter()) {
        candidates.push(format!("{raw}{suffix}"));
    }
    for candidate in &candidates {
        if tracked(candidate) {
            return ClassifiedPath {
                class: PathClass::Internal,
                path: Some(candidate.clone()),
                raw: raw.to_string(),
            };
        }
    }

    // Existing-but-untracked files are local checkout state, not stale Sjel sources. Drop them
    // without retaining the raw filename in any reported bucket.
    if candidates.iter().any(|candidate| exists(candidate)) {
        return ClassifiedPath {
            class: PathClass::Local,
            path: None,
            raw: raw.to_string(),
        };
    }

    // Unresolvable. A `$`-alias is always foreign (SvelteKit's `$app/*`, `$lib/*` style).
    // Otherwise: under a known root means it should have existed (stale); anywhere else means it
    // was never ours (a bare npm/crate specifier).
    if raw.starts_with('$') {
        return ClassifiedPath {
            class: PathClass::External,
            path: None,
            raw: raw.to_string(),
        };
    }
    let root = raw.split('/').next().unwrap_or("");
    ClassifiedPath {
        class: if INTERNAL_ROOTS.contains(&root) {
            PathClass::Stale
        } else {
            PathClass::External
        },
        path: None,
        raw: raw.to_string(),
    }
}

/// Map an internal path to the unit that owns it.
///
/// `dashboard` is the spine shell and owns its whole directory, so it has no `<name>` segment to
/// read — it IS the unit. `tools` and `schemas` are the same shape.
pub fn unit_for_path(path: &str) -> Option<Unit> {
    let parts: Vec<&str> = path.split('/').collect();
    let root = *parts.first()?;
    let second = parts.get(1).copied();
    match root {
        "capabilities" => second.map(|name| Unit {
            name: name.to_string(),
            kind: UnitKind::Capability,
        }),
        "libs" => second.map(|name| Unit {
            name: name.to_string(),
            kind: UnitKind::Lib,
        }),
        "Packs" => second.map(|name| Unit {
            name: name.to_string(),
            kind: UnitKind::Pack,
        }),
        "dashboard" | "tools" | "schemas" => Some(Unit {
            name: root.to_string(),
            kind: UnitKind::Spine,
        }),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitRollup {
    pub name: String,
    pub kind: UnitKind,
    /// Distinct canonical files — deduplicated, so a specifier and its target count once.
    pub files: usize,
    /// graphify nodes attributed to this unit, before file dedup.
    pub nodes: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Buckets {
    /// Foreign module specifiers, e.g. `svelte`, `maplibre-gl`, `$app/state`.
    pub external: usize,
    /// Internal-looking paths that resolve to nothing — the staleness signal.
    pub stale: Vec<String>,
    /// Nodes graphify emitted with no `source_file`.
    pub empty: usize,
    /// Resolved internal paths under no known unit root, e.g. a root-level README.
    pub unmatched: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Rollup {
    pub units: Vec<UnitRollup>,
    pub buckets: Buckets,
    /// Nodes admitted through the public-safe boundary; local nodes are excluded.
    pub admitted_nodes: usize,
}

/// One graphify node, as far as this reads it.
#[derive(Debug, Clone, Deserialize)]
pub struct GraphNode {
    #[serde(default)]
    pub source_file: Option<String>,
    /// graphify's label. For an import target it is the specifier as written, e.g. `../core/ops.ts`.
    #[serde(default)]
    pub label: Option<String>,
}

/// An import target graphify resolved against the importing file's directory to a path that does
/// not exist: a relative specifier in a minified bundle, a build script or a tsconfig naming a
/// file this tree never had. It is a dangling reference written in a tracked file, not a source a
/// refactor deleted, so it counts with the foreign specifiers and is never reported as stale.
pub fn is_unresolved_relative_import(node: &GraphNode) -> bool {
    let label = node.label.as_deref().unwrap_or("");
    label.starts_with("./") || label.starts_with("../")
}

/// Roll every graph node up to its unit.
///
/// Node counts and file counts are both reported: they differ by exactly the double-counting
/// graphify introduces when it emits an import specifier and its target file as two nodes, and
/// seeing both numbers is how that stays visible instead of being silently absorbed.
pub fn roll_up(
    nodes: &[GraphNode],
    tracked: &dyn Fn(&str) -> bool,
    exists: &dyn Fn(&str) -> bool,
) -> Rollup {
    let mut files_by_unit: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut nodes_by_unit: BTreeMap<String, usize> = BTreeMap::new();
    let mut kind_by_unit: BTreeMap<String, UnitKind> = BTreeMap::new();
    let mut buckets = Buckets::default();
    let mut stale_seen: BTreeSet<String> = BTreeSet::new();
    let mut unmatched_seen: BTreeSet<String> = BTreeSet::new();
    let mut admitted_nodes = 0usize;

    for node in nodes {
        let classified = classify_path(node.source_file.as_deref(), tracked, exists);
        if classified.class == PathClass::Local {
            continue;
        }
        admitted_nodes += 1;
        match classified.class {
            PathClass::Empty => {
                buckets.empty += 1;
                continue;
            }
            PathClass::External => {
                buckets.external += 1;
                continue;
            }
            PathClass::Stale if is_unresolved_relative_import(node) => {
                buckets.external += 1;
                continue;
            }
            PathClass::Stale => {
                if stale_seen.insert(classified.raw.clone()) {
                    buckets.stale.push(classified.raw);
                }
                continue;
            }
            PathClass::Internal | PathClass::Local => {}
        }

        let path = classified.path.unwrap_or_default();
        let Some(unit) = unit_for_path(&path) else {
            if unmatched_seen.insert(path.clone()) {
                buckets.unmatched.push(path);
            }
            continue;
        };
        kind_by_unit.insert(unit.name.clone(), unit.kind.clone());
        *nodes_by_unit.entry(unit.name.clone()).or_insert(0) += 1;
        files_by_unit
            .entry(unit.name.clone())
            .or_default()
            .insert(path);
    }

    let units: Vec<UnitRollup> = files_by_unit
        .iter()
        .map(|(name, files)| UnitRollup {
            name: name.clone(),
            kind: kind_by_unit
                .get(name)
                .cloned()
                .unwrap_or(UnitKind::Capability),
            files: files.len(),
            nodes: nodes_by_unit.get(name).copied().unwrap_or(0),
        })
        .collect();

    buckets.stale.sort();
    buckets.unmatched.sort();
    Rollup {
        units,
        buckets,
        admitted_nodes,
    }
}

/// How one unit's code reaches into another's, as declared in ground truth.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceCoupling {
    pub from: String,
    pub to: String,
    pub kind: String,
    /// The file the evidence lives in.
    pub file: String,
    /// The literal string that proves it.
    pub evidence: String,
}

/// Normalize a POSIX-ish relative path, resolving `.` and `..` segments.
fn normalize_relative(base: &str, rel: &str) -> String {
    let mut stack: Vec<&str> = base
        .split('/')
        .filter(|segment| !segment.is_empty() && *segment != ".")
        .collect();
    for segment in rel.split('/') {
        if segment.is_empty() || segment == "." {
            continue;
        }
        if segment == ".." {
            stack.pop();
        } else {
            stack.push(segment);
        }
    }
    stack.join("/")
}

/// Rust `#[path = "…"]` includes that resolve into a different unit.
///
/// `file` is the repo-relative path of the source file, so the attribute's relative path resolves
/// against its directory. An include that stays inside the same unit is not coupling.
pub fn coupling_from_rust_path(file: &str, text: &str) -> Vec<SourceCoupling> {
    let Some(unit) = unit_for_path(file) else {
        return Vec::new();
    };
    let dir = file.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("");
    let pattern = Regex::new(r#"#\[path\s*=\s*"([^"]+)"\]"#).expect("static pattern");
    let mut out = Vec::new();
    for captures in pattern.captures_iter(text) {
        let whole = captures.get(0).expect("group 0");
        let target = normalize_relative(dir, &captures[1]);
        let Some(other) = unit_for_path(&target) else {
            continue;
        };
        if other.name == unit.name {
            continue;
        }
        out.push(SourceCoupling {
            from: unit.name.clone(),
            to: other.name,
            kind: "rust-path".to_string(),
            file: file.to_string(),
            evidence: whole.as_str().to_string(),
        });
    }
    out
}

/// Cargo path dependencies naming another unit, from any `Cargo.toml`.
///
/// Every `path = "…"` matches — dependencies, dev-dependencies and build-dependencies all
/// express a real compile-time reach, and treating them alike keeps this from silently missing a
/// fourth table someone adds later. A registry dependency carries no `path` and never matches.
pub fn coupling_from_cargo(file: &str, text: &str) -> Vec<SourceCoupling> {
    let Some(unit) = unit_for_path(file) else {
        return Vec::new();
    };
    let dir = file.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("");
    let pattern = Regex::new(r#"\bpath\s*=\s*"([^"]+)""#).expect("static pattern");
    let mut out: Vec<SourceCoupling> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for captures in pattern.captures_iter(text) {
        let whole = captures.get(0).expect("group 0");
        let Some(other) = unit_for_path(&normalize_relative(dir, &captures[1])) else {
            continue;
        };
        if other.name == unit.name {
            continue;
        }
        // First evidence per pair wins, as the TypeScript's insertion-ordered Map did.
        if !seen.insert(other.name.clone()) {
            continue;
        }
        out.push(SourceCoupling {
            from: unit.name.clone(),
            to: other.name,
            kind: "cargo-dep".to_string(),
            file: file.to_string(),
            evidence: whole.as_str().to_string(),
        });
    }
    out
}

/// One merged pair, in the shape the artifact carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergedCoupling {
    pub from: String,
    pub to: String,
    pub kinds: Vec<String>,
    pub evidence: Vec<String>,
}

/// Merge per-file coupling into a deterministic, deduplicated unit-level map.
///
/// Both evidence kinds are kept per pair rather than collapsed to a boolean: a pair backed by
/// `rust-path` but NOT `cargo-dep` is a source include the manifest never declared, and the
/// reverse is a dependency nothing imports.
pub fn merge_coupling(edges: &[SourceCoupling]) -> Vec<MergedCoupling> {
    let mut acc: BTreeMap<(String, String), (BTreeSet<String>, BTreeSet<String>)> = BTreeMap::new();
    for edge in edges {
        let entry = acc.entry((edge.from.clone(), edge.to.clone())).or_default();
        entry.0.insert(edge.kind.clone());
        entry.1.insert(edge.file.clone());
    }
    acc.into_iter()
        .map(|((from, to), (kinds, evidence))| MergedCoupling {
            from,
            to,
            kinds: kinds.into_iter().collect(),
            evidence: evidence.into_iter().collect(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tracked_from(paths: &[&str]) -> impl Fn(&str) -> bool {
        let set: BTreeSet<String> = paths.iter().map(|p| p.to_string()).collect();
        move |path: &str| set.contains(path)
    }

    fn nothing(_: &str) -> bool {
        false
    }

    fn node(source: Option<&str>, label: Option<&str>) -> GraphNode {
        GraphNode {
            source_file: source.map(str::to_string),
            label: label.map(str::to_string),
        }
    }

    #[test]
    fn a_path_that_exists_verbatim_is_internal() {
        let tracked = tracked_from(&["capabilities/comms/src/main.rs"]);
        let out = classify_path(Some("capabilities/comms/src/main.rs"), &tracked, &tracked);
        assert_eq!(out.class, PathClass::Internal);
        assert_eq!(out.path.as_deref(), Some("capabilities/comms/src/main.rs"));
    }

    #[test]
    fn an_extension_stripped_specifier_resolves_to_its_real_file() {
        let tracked = tracked_from(&["dashboard/src/lib/api.ts"]);
        let out = classify_path(Some("dashboard/src/lib/api"), &tracked, &tracked);
        assert_eq!(out.class, PathClass::Internal);
        assert_eq!(out.path.as_deref(), Some("dashboard/src/lib/api.ts"));
    }

    #[test]
    fn a_svelte_ts_module_resolves_not_just_plain_ts() {
        let tracked = tracked_from(&["dashboard/src/lib/capabilities.svelte.ts"]);
        let out = classify_path(
            Some("dashboard/src/lib/capabilities.svelte"),
            &tracked,
            &tracked,
        );
        assert_eq!(out.class, PathClass::Internal);
        assert_eq!(
            out.path.as_deref(),
            Some("dashboard/src/lib/capabilities.svelte.ts")
        );
    }

    #[test]
    fn a_bare_package_name_is_external_never_stale() {
        let out = classify_path(Some("svelte"), &nothing, &nothing);
        assert_eq!(out.class, PathClass::External);
    }

    #[test]
    fn a_dollar_alias_is_external_even_though_it_contains_a_slash() {
        let out = classify_path(Some("$app/state"), &nothing, &nothing);
        assert_eq!(out.class, PathClass::External);
    }

    #[test]
    fn an_unresolvable_path_under_a_known_root_is_stale() {
        let out = classify_path(Some("capabilities/gone/src/main.rs"), &nothing, &nothing);
        assert_eq!(out.class, PathClass::Stale);
    }

    #[test]
    fn an_existing_but_untracked_path_is_local() {
        let exists = tracked_from(&["graphify-out/cache/thing.json"]);
        let out = classify_path(Some("graphify-out/cache/thing.json"), &nothing, &exists);
        assert_eq!(out.class, PathClass::Local);
    }

    #[test]
    fn a_null_or_empty_source_file_is_its_own_class() {
        assert_eq!(
            classify_path(None, &nothing, &nothing).class,
            PathClass::Empty
        );
        assert_eq!(
            classify_path(Some("  "), &nothing, &nothing).class,
            PathClass::Empty
        );
    }

    #[test]
    fn a_path_maps_to_its_unit_and_the_spine_owns_its_whole_directory() {
        assert_eq!(
            unit_for_path("capabilities/comms/src/main.rs"),
            Some(Unit {
                name: "comms".to_string(),
                kind: UnitKind::Capability
            })
        );
        assert_eq!(
            unit_for_path("libs/sjel-config/src/lib.rs"),
            Some(Unit {
                name: "sjel-config".to_string(),
                kind: UnitKind::Lib
            })
        );
        assert_eq!(
            unit_for_path("Packs/cv/skills/cv-builder/SKILL.md"),
            Some(Unit {
                name: "cv".to_string(),
                kind: UnitKind::Pack
            })
        );
        assert_eq!(
            unit_for_path("tools/sjel-cli/src/main.rs"),
            Some(Unit {
                name: "tools".to_string(),
                kind: UnitKind::Spine
            })
        );
        assert_eq!(unit_for_path("README.md"), None);
    }

    #[test]
    fn a_dangling_relative_import_is_foreign_while_a_deleted_source_stays_stale() {
        let tracked = tracked_from(&["capabilities/comms/src/main.rs"]);
        let nodes = vec![
            node(Some("capabilities/comms/src/main.rs"), None),
            node(Some("tools/demo/out.js"), Some("../core/ops.ts")),
            node(Some("capabilities/gone/src/main.rs"), None),
        ];
        let rollup = roll_up(&nodes, &tracked, &tracked);
        assert_eq!(rollup.buckets.external, 1);
        assert_eq!(rollup.buckets.stale, vec!["capabilities/gone/src/main.rs"]);
        assert_eq!(rollup.admitted_nodes, 3);
    }

    #[test]
    fn local_graph_artifacts_do_not_change_the_public_rollup() {
        let tracked = tracked_from(&["capabilities/comms/src/main.rs"]);
        let exists = tracked_from(&[
            "capabilities/comms/src/main.rs",
            "graphify-out/cache/thing.json",
        ]);
        let nodes = vec![
            node(Some("capabilities/comms/src/main.rs"), None),
            node(Some("graphify-out/cache/thing.json"), None),
        ];
        let rollup = roll_up(&nodes, &tracked, &exists);
        assert_eq!(rollup.admitted_nodes, 1);
        assert_eq!(rollup.units.len(), 1);
        assert_eq!(rollup.units[0].files, 1);
    }

    #[test]
    fn a_specifier_and_its_target_count_as_one_file_but_two_nodes() {
        let tracked = tracked_from(&["dashboard/src/lib/api.ts", "dashboard/src/lib/use-api.ts"]);
        let nodes = vec![
            node(Some("dashboard/src/lib/api"), None),
            node(Some("dashboard/src/lib/api.ts"), None),
            node(Some("dashboard/src/lib/use-api.ts"), None),
        ];
        let rollup = roll_up(&nodes, &tracked, &tracked);
        let dashboard = &rollup.units[0];
        assert_eq!(dashboard.name, "dashboard");
        assert_eq!(dashboard.files, 2);
        assert_eq!(dashboard.nodes, 3);
    }

    #[test]
    fn external_empty_stale_and_unmatched_land_in_named_buckets() {
        let tracked = tracked_from(&["README.md"]);
        let nodes = vec![
            node(Some("svelte"), None),
            node(None, None),
            node(Some("libs/gone/src/lib.rs"), None),
            node(Some("README.md"), None),
        ];
        let rollup = roll_up(&nodes, &tracked, &tracked);
        assert_eq!(rollup.buckets.external, 1);
        assert_eq!(rollup.buckets.empty, 1);
        assert_eq!(rollup.buckets.stale, vec!["libs/gone/src/lib.rs"]);
        assert_eq!(rollup.buckets.unmatched, vec!["README.md"]);
        assert!(rollup.units.is_empty());
    }

    #[test]
    fn units_come_back_sorted_so_two_runs_on_one_graph_agree() {
        let tracked = tracked_from(&[
            "libs/sjel-http/src/lib.rs",
            "capabilities/comms/src/main.rs",
        ]);
        let nodes = vec![
            node(Some("libs/sjel-http/src/lib.rs"), None),
            node(Some("capabilities/comms/src/main.rs"), None),
        ];
        let rollup = roll_up(&nodes, &tracked, &tracked);
        let names: Vec<&str> = rollup.units.iter().map(|u| u.name.as_str()).collect();
        assert_eq!(names, vec!["comms", "sjel-http"]);
    }

    #[test]
    fn a_path_include_reaching_another_unit_is_coupling() {
        let edges = coupling_from_rust_path(
            "capabilities/comms/src/main.rs",
            "mod sjel_config;\n#[path = \"../../../libs/sjel-config/src/lib.rs\"]\n",
        );
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].from, "comms");
        assert_eq!(edges[0].to, "sjel-config");
        assert_eq!(edges[0].kind, "rust-path");
        assert_eq!(
            edges[0].evidence,
            "#[path = \"../../../libs/sjel-config/src/lib.rs\"]"
        );
    }

    #[test]
    fn an_include_staying_inside_its_own_unit_is_not_coupling() {
        let edges = coupling_from_rust_path(
            "capabilities/calendar/src/lib.rs",
            "#[path = \"./helpers/thing.rs\"] mod thing;\n",
        );
        assert!(edges.is_empty(), "{edges:?}");
    }

    #[test]
    fn plain_use_statements_are_ignored_because_they_name_crates() {
        let edges = coupling_from_rust_path(
            "capabilities/comms/src/main.rs",
            "use sjel_config::Config;\nuse std::collections::BTreeMap;\n",
        );
        assert!(edges.is_empty());
    }

    #[test]
    fn a_doc_comment_naming_another_unit_is_not_coupling() {
        let edges = coupling_from_rust_path(
            "capabilities/comms/src/main.rs",
            "// see libs/sjel-config/src/lib.rs for the shape\n",
        );
        assert!(edges.is_empty());
    }

    #[test]
    fn a_path_dependency_naming_another_unit_is_coupling() {
        let edges = coupling_from_cargo(
            "capabilities/comms/Cargo.toml",
            "[dependencies]\nsjel-config = { path = \"../../libs/sjel-config\" }\n",
        );
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].to, "sjel-config");
        assert_eq!(edges[0].kind, "cargo-dep");
    }

    #[test]
    fn registry_dependencies_are_not_unit_coupling() {
        let edges = coupling_from_cargo(
            "capabilities/comms/Cargo.toml",
            "[dependencies]\nserde = { workspace = true, features = [\"derive\"] }\n",
        );
        assert!(edges.is_empty());
    }

    #[test]
    fn a_lib_or_bin_path_inside_the_same_unit_is_not_coupling() {
        let edges = coupling_from_cargo(
            "tools/sjel-cli/Cargo.toml",
            "[[bin]]\nname = \"sjel-cli\"\npath = \"src/main.rs\"\n",
        );
        assert!(edges.is_empty());
    }

    #[test]
    fn a_dev_dependency_reaches_just_as_far_as_a_dependency() {
        let edges = coupling_from_cargo(
            "capabilities/comms/Cargo.toml",
            "[dev-dependencies]\ncivil-date = { path = \"../../libs/civil-date\" }\n",
        );
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].to, "civil-date");
    }

    #[test]
    fn merging_keeps_both_kinds_and_the_files_that_proved_them() {
        let edges = vec![
            SourceCoupling {
                from: "comms".into(),
                to: "sjel-http".into(),
                kind: "rust-path".into(),
                file: "capabilities/comms/src/main.rs".into(),
                evidence: "#[path = \"…\"]".into(),
            },
            SourceCoupling {
                from: "comms".into(),
                to: "sjel-http".into(),
                kind: "cargo-dep".into(),
                file: "capabilities/comms/Cargo.toml".into(),
                evidence: "path = \"…\"".into(),
            },
        ];
        let merged = merge_coupling(&edges);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].kinds, vec!["cargo-dep", "rust-path"]);
        assert_eq!(
            merged[0].evidence,
            vec![
                "capabilities/comms/Cargo.toml",
                "capabilities/comms/src/main.rs"
            ]
        );
    }
}
