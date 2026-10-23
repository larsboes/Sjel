//! The Packs page data models, wire payloads, and matrix reductions.
//!
//! Replaces the SvelteKit dashboard's `/packs` route. Reads deployment status across agent harnesses
//! (`claude`, `pi`, `codex`, `opencode`), computes the skill-by-harness matrix, identifies drifted
//! or outdated installations, and plans sync/deploy/remove operations.

use serde::{Deserialize, Serialize};

/// One Pack skill as one harness currently holds it.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PackUnitView {
    pub pack: String,
    pub skill: String,
    /// current · outdated · drifted · missing · collision · invalid · not-deployed · migration-required
    pub status: String,
    #[serde(default)]
    pub detail: Option<String>,
}

/// A stray directory at a harness skill root that no Pack ledger claims.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PackStrayView {
    pub name: String,
    /// `copy` is a promote candidate; `external` and `symlink` are owned by another installer.
    pub kind: String,
    #[serde(default)]
    pub detail: Option<String>,
}

/// An agent harness known to Sjel.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HarnessView {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub installed: bool,
    /// `materialized` copies and can drift; `registry` reads the Pack source in place.
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub marker: String,
    #[serde(default)]
    pub destination: Option<String>,
    #[serde(default)]
    pub cli: String,
    #[serde(default)]
    pub units: Vec<PackUnitView>,
    #[serde(default)]
    pub unowned: Vec<PackStrayView>,
}

/// An unsupported harness or capability.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UnsupportedView {
    pub id: String,
    pub label: String,
    pub why: String,
}

/// The full payload emitted by `tools/harnesses status --json` and served by `GET /api/sjel-status/packs`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PacksView {
    #[serde(default)]
    pub measured_at: String,
    #[serde(default)]
    pub harnesses: Vec<HarnessView>,
    #[serde(default)]
    pub unsupported: Vec<UnsupportedView>,
}

/// Request payload for `POST /api/sjel-status/packs`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PackWriteRequest {
    pub action: String,
    pub harness: String,
    pub packs: Vec<String>,
}

/// Success reply from `POST /api/sjel-status/packs`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct PackWriteResult {
    pub action: String,
    pub harness: String,
    pub packs: Vec<String>,
    pub detail: String,
}

/// A unique (pack, skill) matrix row.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MatrixRow {
    pub pack: String,
    pub skill: String,
}

/// Return all unique (pack, skill) pairs across installed harnesses.
pub fn matrix_rows(view: &PacksView) -> Vec<MatrixRow> {
    let mut rows = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for harness in view.harnesses.iter().filter(|h| h.installed) {
        for unit in &harness.units {
            let key = (unit.pack.clone(), unit.skill.clone());
            if seen.insert(key.clone()) {
                rows.push(MatrixRow {
                    pack: key.0,
                    skill: key.1,
                });
            }
        }
    }
    rows
}

/// Extract all unique Pack names from the matrix rows.
pub fn all_packs(rows: &[MatrixRow]) -> Vec<String> {
    let mut packs = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for row in rows {
        if seen.insert(row.pack.clone()) {
            packs.push(row.pack.clone());
        }
    }
    packs
}

/// Look up the deployment status of a skill in a harness.
pub fn status_of<'a>(harness: &'a HarnessView, pack: &str, skill: &str) -> &'a str {
    harness
        .units
        .iter()
        .find(|u| u.pack == pack && u.skill == skill)
        .map(|u| u.status.as_str())
        .unwrap_or("not-deployed")
}

/// Anything that is neither `current` nor `not-deployed` needs human attention or a sync.
pub fn needs_attention(status: &str) -> bool {
    status != "current" && status != "not-deployed"
}

/// Check if any installed harness has this row in an attention-needed state.
pub fn row_needs_attention(view: &PacksView, row: &MatrixRow) -> bool {
    view.harnesses
        .iter()
        .filter(|h| h.installed)
        .any(|h| needs_attention(status_of(h, &row.pack, &row.skill)))
}

/// Return the units in a harness that are actually deployed (status != "not-deployed").
pub fn deployed_units(harness: &HarnessView) -> Vec<&PackUnitView> {
    harness
        .units
        .iter()
        .filter(|u| u.status != "not-deployed")
        .collect()
}

/// The set of Pack names a harness holds, deduplicated.
pub fn held_packs(harness: &HarnessView) -> Vec<String> {
    let mut packs = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for u in deployed_units(harness) {
        if seen.insert(u.pack.clone()) {
            packs.push(u.pack.clone());
        }
    }
    packs
}

/// Installed harnesses.
pub fn installed_harnesses(view: &PacksView) -> Vec<&HarnessView> {
    view.harnesses.iter().filter(|h| h.installed).collect()
}

/// A harness nobody installed that still holds deployed copies — orphaned state.
pub fn orphaned_harnesses(view: &PacksView) -> Vec<&HarnessView> {
    view.harnesses
        .iter()
        .filter(|h| !h.installed && !deployed_units(h).is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_view() -> PacksView {
        PacksView {
            measured_at: "2026-10-23T09:12:40.618Z".into(),
            harnesses: vec![
                HarnessView {
                    id: "claude".into(),
                    label: "Claude Code".into(),
                    installed: true,
                    model: "materialized".into(),
                    marker: "/Users/test/.claude".into(),
                    destination: Some("/Users/test/.claude/skills".into()),
                    cli: "tools/packs-claude".into(),
                    units: vec![
                        PackUnitView {
                            pack: "coding".into(),
                            skill: "effective-rust".into(),
                            status: "current".into(),
                            detail: None,
                        },
                        PackUnitView {
                            pack: "coding".into(),
                            skill: "gpui".into(),
                            status: "drifted".into(),
                            detail: Some("local edit".into()),
                        },
                        PackUnitView {
                            pack: "harness".into(),
                            skill: "sjel".into(),
                            status: "outdated".into(),
                            detail: None,
                        },
                        PackUnitView {
                            pack: "cv".into(),
                            skill: "cv-builder".into(),
                            status: "not-deployed".into(),
                            detail: None,
                        },
                    ],
                    unowned: vec![PackStrayView {
                        name: "my-custom-tool".into(),
                        kind: "copy".into(),
                        detail: Some("untracked file".into()),
                    }],
                },
                HarnessView {
                    id: "pi".into(),
                    label: "Pi".into(),
                    installed: true,
                    model: "registry".into(),
                    marker: "/Users/test/.pi".into(),
                    destination: None,
                    cli: "tools/packs-pi".into(),
                    units: vec![
                        PackUnitView {
                            pack: "coding".into(),
                            skill: "effective-rust".into(),
                            status: "current".into(),
                            detail: None,
                        },
                        PackUnitView {
                            pack: "coding".into(),
                            skill: "gpui".into(),
                            status: "missing".into(),
                            detail: None,
                        },
                    ],
                    unowned: vec![],
                },
                HarnessView {
                    id: "codex".into(),
                    label: "Codex".into(),
                    installed: false,
                    model: "materialized".into(),
                    marker: "/Users/test/.codex".into(),
                    destination: Some("/Users/test/.codex/skills".into()),
                    cli: "tools/packs-codex".into(),
                    units: vec![PackUnitView {
                        pack: "deliberation".into(),
                        skill: "council".into(),
                        status: "current".into(),
                        detail: None,
                    }],
                    unowned: vec![],
                },
            ],
            unsupported: vec![UnsupportedView {
                id: "gemini".into(),
                label: "Gemini CLI".into(),
                why: "No skills adapter yet".into(),
            }],
        }
    }

    #[test]
    fn test_matrix_rows_and_all_packs() {
        let view = sample_view();
        let rows = matrix_rows(&view);
        assert_eq!(rows.len(), 4);
        assert_eq!(
            rows,
            vec![
                MatrixRow {
                    pack: "coding".into(),
                    skill: "effective-rust".into()
                },
                MatrixRow {
                    pack: "coding".into(),
                    skill: "gpui".into()
                },
                MatrixRow {
                    pack: "harness".into(),
                    skill: "sjel".into()
                },
                MatrixRow {
                    pack: "cv".into(),
                    skill: "cv-builder".into()
                },
            ]
        );

        let packs = all_packs(&rows);
        assert_eq!(packs, vec!["coding", "harness", "cv"]);
    }

    #[test]
    fn test_status_and_attention() {
        let view = sample_view();
        let claude = &view.harnesses[0];
        assert_eq!(status_of(claude, "coding", "effective-rust"), "current");
        assert_eq!(status_of(claude, "coding", "gpui"), "drifted");
        assert_eq!(status_of(claude, "nonexistent", "skill"), "not-deployed");

        assert!(!needs_attention("current"));
        assert!(!needs_attention("not-deployed"));
        assert!(needs_attention("drifted"));
        assert!(needs_attention("outdated"));
        assert!(needs_attention("missing"));

        let row_rust = MatrixRow {
            pack: "coding".into(),
            skill: "effective-rust".into(),
        };
        let row_gpui = MatrixRow {
            pack: "coding".into(),
            skill: "gpui".into(),
        };
        assert!(!row_needs_attention(&view, &row_rust));
        assert!(row_needs_attention(&view, &row_gpui));
    }

    #[test]
    fn test_deployed_and_held_packs() {
        let view = sample_view();
        let claude = &view.harnesses[0];
        let deployed = deployed_units(claude);
        assert_eq!(deployed.len(), 3);
        let held = held_packs(claude);
        assert_eq!(held, vec!["coding", "harness"]);
    }

    #[test]
    fn test_orphaned_harnesses() {
        let view = sample_view();
        let orphans = orphaned_harnesses(&view);
        assert_eq!(orphans.len(), 1);
        assert_eq!(orphans[0].id, "codex");
    }

    #[test]
    fn test_deserialization_from_live_wire_shape() {
        let json_str = r#"{
            "measuredAt": "2026-10-23T09:12:40.618Z",
            "harnesses": [
                {
                    "id": "claude",
                    "label": "Claude Code",
                    "installed": true,
                    "model": "materialized",
                    "marker": "/Users/larsboes/.claude",
                    "destination": "/Users/larsboes/.claude/skills",
                    "cli": "tools/packs-claude",
                    "units": [
                        {
                            "pack": "coding",
                            "skill": "effective-rust",
                            "status": "current"
                        }
                    ],
                    "unowned": []
                }
            ],
            "unsupported": []
        }"#;

        let parsed: PacksView = serde_json::from_str(json_str).expect("Valid JSON");
        assert_eq!(parsed.harnesses.len(), 1);
        assert_eq!(parsed.harnesses[0].label, "Claude Code");
        assert_eq!(parsed.harnesses[0].units[0].status, "current");
    }
}
