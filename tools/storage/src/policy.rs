//! The overlay's `storage-policy.toml`: what accumulates on this machine, what may be
//! reclaimed, and what "low on space" means here.
//!
//! Nothing in this file names a path on this machine. Classes, protected paths, expected
//! services and thresholds are deployment facts, so they live in the overlay
//! (README.md#public-core-and-private-overlays). `schemas/storage-policy.toml.example` is
//! the shape.
//!
//! `reclaim_argv` is the one function here that can destroy something. It decides whether
//! a policy-supplied string reaches a shell, and whether `apply` deletes the paths this
//! tool measured or a path a policy string claimed.

use std::path::Path;

use serde::{Deserialize, Serialize};

pub const GB: u64 = 1024 * 1024 * 1024;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Policy {
    #[serde(default)]
    pub thresholds: Thresholds,
    /// `[[class]]` in the file. Renamed because the plural reads correctly at every use
    /// site and `class` is a word Rust will want back one day.
    #[serde(default, rename = "class")]
    pub classes: Vec<StorageClass>,
    #[serde(default)]
    pub protected: Vec<ProtectedEntry>,
    #[serde(default)]
    pub expected_service: Vec<ExpectedService>,
}

/// Absent thresholds must never manufacture an alarm, so each one is optional and the
/// caller supplies the neutral default: 0 GB for the free-space bands (nothing is ever
/// below zero) and "no flag" for the class size.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Thresholds {
    pub free_warn_gb: Option<f64>,
    pub free_critical_gb: Option<f64>,
    pub class_flag_gb: Option<f64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct StorageClass {
    pub name: String,
    #[serde(default)]
    pub paths: Vec<String>,
    pub reclaim: Option<String>,
    #[serde(default)]
    pub apply: bool,
    #[serde(default)]
    pub regrows: bool,
    #[allow(dead_code)] // Read by an operator in the policy file; the report prints sizes.
    pub note: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProtectedEntry {
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ExpectedService {
    pub name: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Read and parse the policy. The error is the message a caller prints verbatim.
pub fn load(path: &Path) -> Result<Policy, String> {
    let text = std::fs::read_to_string(path).map_err(|e| {
        format!(
            "no policy at {} ({e})\nSee schemas/storage-policy.toml.example for the expected shape.",
            path.display()
        )
    })?;
    toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

/// A class is only touchable by `apply` when the policy both allows it and says how.
/// `apply = true` with no reclaim command is a policy bug, not a licence to guess.
pub fn is_applicable(c: &StorageClass) -> bool {
    c.apply && c.reclaim.as_deref().is_some_and(|r| !r.is_empty())
}

/// The safety boundary. A policy-supplied string is data, not code — the one case where it
/// reaches a shell is an explicit `rm -rf`, and even then the paths substituted are the
/// ones THIS tool measured rather than a command line the policy handed over. Anything
/// else runs verbatim because it is a named tool's own cleanup verb (`brew cleanup`,
/// `cargo clean`) that only its own CLI can express.
pub fn reclaim_argv(c: &StorageClass, measured_paths: &[String]) -> Option<Vec<String>> {
    if !is_applicable(c) {
        return None;
    }
    let reclaim = c.reclaim.as_deref().unwrap_or_default();
    if reclaim == "rm -rf" {
        // Never a bare `rm -rf`. The failure this guards: argv collapsing to two elements
        // and the command inheriting a cwd, or a policy path expanding to "" and taking
        // "/" with it.
        if measured_paths.is_empty() {
            return None;
        }
        let mut argv = vec!["rm".to_string(), "-rf".to_string()];
        argv.extend(measured_paths.iter().cloned());
        return Some(argv);
    }
    Some(vec![
        "bash".to_string(),
        "-lc".to_string(),
        reclaim.to_string(),
    ])
}

/// Free-space verdict against the policy thresholds. CRITICAL wins on the boundary where
/// the bands would overlap.
pub fn disk_state(free_bytes: u64, warn_gb: f64, critical_gb: f64) -> &'static str {
    let free = free_bytes as f64;
    let gb = GB as f64;
    if free < critical_gb * gb {
        "CRITICAL"
    } else if free < warn_gb * gb {
        "warn"
    } else {
        "ok"
    }
}

/// `~` and `~/...` only. A path merely STARTING with `~` is not a home reference —
/// `~backup/data` is a directory named `~backup`, and expanding it would silently point
/// the scan somewhere else.
///
/// `sjel_config::expand_tilde` covers the `~/` half but reads `HOME` from the process,
/// which the tests here have to vary, and it does not accept a bare `~`.
pub fn expand_home(p: &str, home: &str) -> String {
    if p == "~" {
        return home.to_string();
    }
    match p.strip_prefix("~/") {
        Some(rest) => format!("{home}/{rest}"),
        None => p.to_string(),
    }
}

/// `*` matches any run of characters; nothing else is special. Scoped to one path segment
/// by its only caller, which is what the policy's glob contract promises.
pub fn segment_matches(pattern: &str, name: &str) -> bool {
    let (pat, text): (Vec<char>, Vec<char>) = (pattern.chars().collect(), name.chars().collect());
    // Greedy scan with one backtrack point, which is all a `*`-only pattern ever needs.
    let (mut p, mut t) = (0usize, 0usize);
    let (mut star, mut resume) = (None, 0usize);
    while t < text.len() {
        if p < pat.len() && (pat[p] == text[t]) {
            p += 1;
            t += 1;
        } else if p < pat.len() && pat[p] == '*' {
            star = Some(p);
            resume = t;
            p += 1;
        } else if let Some(s) = star {
            p = s + 1;
            resume += 1;
            t = resume;
        } else {
            return false;
        }
    }
    pat[p..].iter().all(|c| *c == '*')
}

/// Glob only where the policy actually uses it: a single `*` in one path segment, which is
/// what "every crate's target dir" needs. Anything richer and the policy would be doing
/// work that belongs in the scanner.
///
/// The tail has to exist, not just the globbed segment, or the scan reports paths that were
/// never built.
pub fn expand_glob(pattern: &str, home: &str) -> Vec<String> {
    let full = expand_home(pattern, home);
    if !full.contains('*') {
        return if Path::new(&full).exists() {
            vec![full]
        } else {
            vec![]
        };
    }
    let star = full.find('*').expect("checked above");
    // The directory holding the globbed segment. A pattern with no `/` before the `*` has
    // no base to scan, so it matches nothing rather than being resolved against a cwd this
    // tool never chose.
    let Some(base_end) = full[..=star].rfind('/') else {
        return vec![];
    };
    let base = &full[..base_end];
    let seg_end = full[star..].find('/').map(|i| star + i);
    let (segment, tail) = match seg_end {
        Some(end) => (&full[base_end + 1..end], &full[end..]),
        None => (&full[base_end + 1..], ""),
    };
    let Ok(entries) = std::fs::read_dir(base) else {
        return vec![];
    };
    let mut out: Vec<String> = entries
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|name| segment_matches(segment, name))
        .map(|name| format!("{base}/{name}{tail}"))
        .filter(|candidate| Path::new(candidate).exists())
        .collect();
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cls(reclaim: Option<&str>, apply: bool) -> StorageClass {
        StorageClass {
            name: "fixture".to_string(),
            paths: vec!["~/nowhere".to_string()],
            reclaim: reclaim.map(str::to_string),
            apply,
            regrows: false,
            note: None,
        }
    }

    // ── reclaim_argv — the `apply` safety boundary ────────────────────────────────

    #[test]
    fn rm_rf_deletes_the_paths_we_measured_never_a_policy_supplied_string() {
        let measured = ["/tmp/a/target".to_string(), "/tmp/b/target".to_string()];
        assert_eq!(
            reclaim_argv(&cls(Some("rm -rf"), true), &measured),
            Some(vec![
                "rm".to_string(),
                "-rf".to_string(),
                "/tmp/a/target".to_string(),
                "/tmp/b/target".to_string(),
            ])
        );
    }

    #[test]
    fn rm_rf_with_nothing_measured_is_a_no_op_not_a_bare_rm_rf() {
        assert_eq!(reclaim_argv(&cls(Some("rm -rf"), true), &[]), None);
    }

    #[test]
    fn a_named_tools_cleanup_verb_runs_verbatim_through_a_shell() {
        assert_eq!(
            reclaim_argv(&cls(Some("brew cleanup --prune=all"), true), &[]),
            Some(vec![
                "bash".to_string(),
                "-lc".to_string(),
                "brew cleanup --prune=all".to_string(),
            ])
        );
    }

    #[test]
    fn shell_metacharacters_in_a_measured_path_stay_one_argv_element() {
        // rm goes through argv, so a path is a path even when it looks like a command.
        let nasty = "/tmp/weird; rm -rf ~".to_string();
        assert_eq!(
            reclaim_argv(&cls(Some("rm -rf"), true), std::slice::from_ref(&nasty)),
            Some(vec!["rm".to_string(), "-rf".to_string(), nasty])
        );
    }

    #[test]
    fn report_only_classes_yield_no_command_whatever_they_declare() {
        let paths = ["/tmp/x".to_string()];
        assert_eq!(reclaim_argv(&cls(Some("rm -rf"), false), &paths), None);
    }

    #[test]
    fn apply_without_a_reclaim_command_is_a_policy_bug_not_a_licence_to_guess() {
        let paths = ["/tmp/x".to_string()];
        assert_eq!(reclaim_argv(&cls(None, true), &paths), None);
        assert!(!is_applicable(&cls(None, true)));
        // An empty string is the same bug wearing a value, and TOML makes it easy to type.
        assert!(!is_applicable(&cls(Some(""), true)));
    }

    // ── expand_glob ───────────────────────────────────────────────────────────────

    #[test]
    fn expands_a_single_star_segment_and_keeps_the_tail_sorted() {
        let root = crate::testutil::tempdir("glob");
        for crate_name in ["comms", "scouting", "trips"] {
            std::fs::create_dir_all(root.join("capabilities").join(crate_name).join("target"))
                .unwrap();
        }
        // A crate with no target dir must not appear.
        std::fs::create_dir_all(root.join("capabilities").join("agentbox")).unwrap();

        let base = root.display();
        assert_eq!(
            expand_glob(&format!("{base}/capabilities/*/target"), "/home/runner"),
            vec![
                format!("{base}/capabilities/comms/target"),
                format!("{base}/capabilities/scouting/target"),
                format!("{base}/capabilities/trips/target"),
            ]
        );
    }

    #[test]
    fn a_trailing_star_segment_needs_no_tail() {
        let root = crate::testutil::tempdir("glob-tail");
        for name in ["cache-a", "cache-b", "other"] {
            std::fs::create_dir_all(root.join(name)).unwrap();
        }
        let base = root.display();
        assert_eq!(
            expand_glob(&format!("{base}/cache-*"), "/home/runner"),
            vec![format!("{base}/cache-a"), format!("{base}/cache-b")]
        );
    }

    #[test]
    fn a_literal_path_is_returned_only_when_it_exists() {
        let root = crate::testutil::tempdir("lit");
        std::fs::write(root.join("real"), "x").unwrap();
        let base = root.display();
        assert_eq!(
            expand_glob(&format!("{base}/real"), "/home/runner"),
            vec![format!("{base}/real")]
        );
        assert!(expand_glob(&format!("{base}/absent"), "/home/runner").is_empty());
    }

    #[test]
    fn a_missing_base_directory_yields_nothing_rather_than_failing() {
        assert!(expand_glob("/nonexistent-axon-fixture/*/target", "/home/runner").is_empty());
    }

    #[test]
    fn segment_matching_is_star_only_and_anchored() {
        assert!(segment_matches("cache-*", "cache-a"));
        assert!(segment_matches("*", "anything"));
        assert!(segment_matches("*-target", "comms-target"));
        assert!(!segment_matches("cache-*", "other"));
        // Anchored at both ends: a policy segment is a whole directory name, not a search.
        assert!(!segment_matches("cache", "cache-a"));
        assert!(!segment_matches("ache-*", "cache-a"));
        // A `.` is a literal here, unlike the regex the TypeScript predecessor built.
        assert!(!segment_matches("cache.*", "cache-a"));
    }

    // ── expand_home ───────────────────────────────────────────────────────────────

    #[test]
    fn expands_tilde_slash_and_bare_tilde_leaves_absolute_paths_alone() {
        assert_eq!(
            expand_home("~/.omlx/cache", "/home/runner"),
            "/home/runner/.omlx/cache"
        );
        assert_eq!(expand_home("~", "/home/runner"), "/home/runner");
        assert_eq!(expand_home("/var/log", "/home/runner"), "/var/log");
    }

    #[test]
    fn a_path_merely_starting_with_tilde_is_not_a_home_reference() {
        assert_eq!(expand_home("~backup/data", "/home/runner"), "~backup/data");
    }

    // ── disk_state ────────────────────────────────────────────────────────────────

    #[test]
    fn classifies_against_the_policy_thresholds() {
        assert_eq!(disk_state(200 * GB, 80.0, 40.0), "ok");
        assert_eq!(disk_state(60 * GB, 80.0, 40.0), "warn");
        assert_eq!(disk_state(10 * GB, 80.0, 40.0), "CRITICAL");
    }

    #[test]
    fn critical_wins_on_the_boundary_where_the_bands_would_overlap() {
        assert_eq!(disk_state(39 * GB, 80.0, 40.0), "CRITICAL");
        assert_eq!(disk_state(40 * GB, 80.0, 40.0), "warn");
    }

    #[test]
    fn absent_thresholds_never_manufacture_an_alarm() {
        assert_eq!(disk_state(0, 0.0, 0.0), "ok");
    }

    // ── the policy file itself ────────────────────────────────────────────────────

    #[test]
    fn the_shipped_example_parses_as_a_policy() {
        // schemas/storage-policy.toml.example is the contract an overlay copies. If it
        // stops deserialising, every overlay written from it is already wrong.
        let example =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schemas/storage-policy.toml.example");
        let policy = load(&example).expect("the shipped example must parse");
        assert_eq!(policy.thresholds.free_critical_gb, Some(40.0));
        assert!(policy.classes.iter().any(is_applicable));
        assert!(policy.classes.iter().any(|c| !is_applicable(c)));
        assert!(!policy.protected.is_empty());
        assert!(!policy.expected_service.is_empty());
    }

    #[test]
    fn a_class_with_no_optional_keys_is_report_only() {
        let policy: Policy =
            toml::from_str("[[class]]\nname = \"bare\"\npaths = [\"/tmp\"]\n").unwrap();
        assert_eq!(policy.classes.len(), 1);
        assert!(!is_applicable(&policy.classes[0]));
        assert!(!policy.classes[0].regrows);
        assert_eq!(policy.thresholds.class_flag_gb, None);
    }
}
