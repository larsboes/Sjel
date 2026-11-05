//! The Packs page data models, wire payloads, and matrix reductions.
//!
//! Replaces the SvelteKit dashboard's `/packs` route. Reads deployment status across agent harnesses
//! (`claude`, `pi`, `codex`, `opencode`), computes the skill-by-harness matrix, identifies drifted
//! or outdated installations, and plans sync/deploy/remove operations.

pub use status_api::{
    all_packs, deployed_units, held_packs, installed_harnesses, matrix_rows, needs_attention,
    orphaned_harnesses, row_needs_attention, status_of, HarnessView, MatrixRow, PackStrayView,
    PackUnitView, PackWriteRequest, PackWriteResult, PacksView, UnsupportedView,
};

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
                    "marker": "/Users/example/.claude",
                    "destination": "/Users/example/.claude/skills",
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
