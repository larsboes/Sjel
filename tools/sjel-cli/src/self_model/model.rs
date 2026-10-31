//! The pure core of Sjel's self-model — `tools/lib/self-model.ts`, ported 2026-10-04.
//!
//! Mapping paths to units, and reading real compile-time coupling out of the two places that
//! declare it: `#[path]` attributes and Cargo path dependencies. No I/O lives here: every
//! function takes its world as an argument, so the cases are unit tests here.

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
            unit_for_path("Packs/knowledge/skills/obsidian/SKILL.md"),
            Some(Unit {
                name: "knowledge".to_string(),
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
