//! Where the space the policy does not name actually is.
//!
//! `report` answers "how much of the disk does no entry account for" and, until now, could
//! not answer "which directories". That is the same blind spot one level down: on
//! 2026-10-11 the report read "1.3 GB reclaimable" while `~/.cargo-target` held 149 GB no
//! class named. The fix for that was to say the gap exists; this says where it lives.
//!
//! ## The shape of the answer
//!
//! One `du -x -k -d <depth>` walk of `$HOME`, then every policy path measured on its own so
//! its bytes can be subtracted from whichever directory contains it. A directory at the
//! requested depth is reported with what is left after the named paths inside it, and a
//! directory that is itself a named path — or lies inside one — is dropped entirely, because
//! none of it is unnamed.
//!
//! The depth is the partition. `du` always walks the whole tree, so asking for depth 3 costs
//! what depth 1 costs; the flag changes what is reported, not what is read.
//!
//! ## What it cannot see, said out loud
//!
//! `du` exits non-zero and prints a partial total when it meets a directory it cannot read.
//! On this machine that is the cloud-backed ones — `~/Library/CloudStorage`, iCloud Drive —
//! where it writes `Resource deadlock avoided` and does not descend. A partial scan is
//! reported as partial rather than presented as a size, for the same reason a class
//! measurement carries `measured`.

use std::cmp::Reverse;

use serde::Serialize;

use crate::measure::first_line;

/// Why a scan is partial, mirroring [`crate::measure::Sizing`] for the directory walk.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scan {
    pub complete: bool,
    pub error: String,
}

/// One directory and its size, from `du -d` output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirSize {
    pub path: String,
    pub bytes: u64,
}

/// A policy path and the bytes it accounts for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Named {
    pub path: String,
    pub bytes: u64,
}

/// One reported directory: its total, and how much of it no policy path names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Row {
    pub path: String,
    pub bytes: u64,
    pub unnamed_bytes: u64,
}

/// Parse `du -k -d` output: one `size<TAB>path` line per directory.
///
/// The split is on the first tab and never on whitespace — a path may contain a space
/// (`~/Library/Application Support`), and splitting on whitespace would silently report a
/// directory called `~/Library/Application`.
pub fn parse_du_depth(text: &str) -> Vec<DirSize> {
    text.lines()
        .filter_map(|line| {
            let (size, path) = line.split_once('\t')?;
            let bytes = size.trim().parse::<u64>().ok()? * 1024;
            Some(DirSize {
                path: path.to_string(),
                bytes,
            })
        })
        .collect()
}

/// How many path segments sit below `root`. `None` when `path` is not under `root`, which is
/// how a `du` run that resolved a symlink out of the tree is kept out of the report.
pub fn depth_of(path: &str, root: &str) -> Option<usize> {
    if path == root {
        return Some(0);
    }
    let rest = path.strip_prefix(root)?.strip_prefix('/')?;
    Some(rest.split('/').filter(|s| !s.is_empty()).count())
}

/// The named paths that are not inside another named path.
///
/// A class path nested in a protected path is one thing on disk. Counting it twice is the
/// over-count `report` documents as a caveat; here it would subtract the same bytes twice
/// and make the unnamed remainder too small, so the subtraction uses only the outermost.
pub fn outermost(named: &[Named]) -> Vec<&Named> {
    named
        .iter()
        .filter(|candidate| {
            !named.iter().any(|other| {
                other.path != candidate.path && is_inside(&candidate.path, &other.path)
            })
        })
        .collect()
}

/// True when `path` is `dir` itself or lies below it.
pub fn is_inside(path: &str, dir: &str) -> bool {
    path == dir
        || path
            .strip_prefix(dir)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// The bytes the policy's paths account for, counting a class that sits inside a protected
/// path (or the reverse) **once**.
///
/// `report` documents "named can over-count a path that is both a class and a protection".
/// This is the exact offset for that shape: a class whose paths all sit inside a protected
/// path adds nothing, because the protection already counted those bytes. A class that only
/// partly straddles the boundary still over-counts, and the report still calls `named` a
/// ceiling for exactly that reason.
///
/// Each entry is `(expanded paths, its bytes)`. Pure, so the offset is tested rather than
/// trusted.
pub fn named_bytes(classes: &[(Vec<String>, u64)], protected: &[(Vec<String>, u64)]) -> u64 {
    let class_inside_protected = |paths: &[String]| {
        !paths.is_empty()
            && paths.iter().all(|p| {
                protected
                    .iter()
                    .any(|(q, _)| q.iter().any(|q| is_inside(p, q)))
            })
    };
    let protected_inside_class = |paths: &[String]| {
        paths.iter().any(|q| {
            classes
                .iter()
                .any(|(p, _)| p.iter().any(|p| is_inside(q, p)))
        })
    };
    let from_classes: u64 = classes
        .iter()
        .filter(|(paths, _)| !class_inside_protected(paths))
        .map(|(_, bytes)| *bytes)
        .sum();
    let from_protected: u64 = protected
        .iter()
        .filter(|(paths, _)| !protected_inside_class(paths))
        .map(|(_, bytes)| *bytes)
        .sum();
    from_classes + from_protected
}

/// True when none of `dir` can be unnamed: it is a named path, or lies inside one.
pub fn fully_named(dir: &str, named: &[Named]) -> bool {
    named.iter().any(|n| is_inside(dir, &n.path))
}

/// Bytes the named paths account for strictly inside `dir`, each counted once.
pub fn named_inside(dir: &str, named: &[Named]) -> u64 {
    outermost(named)
        .into_iter()
        .filter(|n| n.path != dir && is_inside(&n.path, dir))
        .map(|n| n.bytes)
        .sum()
}

/// Reduce a walk to the rows `discover` prints: the largest unnamed directories at one
/// depth. Pure, so the partition is tested rather than lived in the command.
pub fn rows(walk: &[DirSize], named: &[Named], root: &str, depth: usize) -> Vec<Row> {
    let mut rows: Vec<Row> = walk
        .iter()
        .filter(|d| depth_of(&d.path, root) == Some(depth))
        .filter(|d| !fully_named(&d.path, named))
        .map(|d| Row {
            path: d.path.clone(),
            bytes: d.bytes,
            unnamed_bytes: d.bytes.saturating_sub(named_inside(&d.path, named)),
        })
        .filter(|row| row.unnamed_bytes > 0)
        .collect();
    rows.sort_by_key(|row| Reverse(row.unnamed_bytes));
    rows
}

/// `~/Library/Caches` for a path under `home`, unchanged otherwise.
pub fn shorten(path: &str, home: &str) -> String {
    match path.strip_prefix(home) {
        Some("") => "~".to_string(),
        Some(rest) if rest.starts_with('/') => format!("~{rest}"),
        _ => path.to_string(),
    }
}

/// Classify a `du` run's exit status and stderr into a [`Scan`]. The first error line is the
/// one a reader acts on; the rest are the same cloud directories failing the same way.
pub fn scan_state(exit_ok: bool, stderr: &str) -> Scan {
    Scan {
        complete: exit_ok,
        error: if exit_ok {
            String::new()
        } else {
            first_line(stderr)
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(path: &str, bytes: u64) -> Named {
        Named {
            path: path.to_string(),
            bytes,
        }
    }

    #[test]
    fn du_lines_split_on_the_tab_so_a_path_with_a_space_survives() {
        let walk = parse_du_depth("2048\t/Users/x/Library/Application Support\n1024\t/Users/x\n");
        assert_eq!(walk.len(), 2);
        assert_eq!(walk[0].path, "/Users/x/Library/Application Support");
        assert_eq!(walk[0].bytes, 2048 * 1024);
        assert_eq!(walk[1].path, "/Users/x");
    }

    #[test]
    fn depth_counts_segments_below_the_root() {
        assert_eq!(depth_of("/Users/x", "/Users/x"), Some(0));
        assert_eq!(depth_of("/Users/x/Library", "/Users/x"), Some(1));
        assert_eq!(depth_of("/Users/x/Library/Caches", "/Users/x"), Some(2));
        // Not under the root, and not a prefix match on a sibling name.
        assert_eq!(depth_of("/Users/xy/Library", "/Users/x"), None);
        assert_eq!(depth_of("/Users/y", "/Users/x"), None);
    }

    #[test]
    fn a_named_path_inside_another_is_counted_once() {
        // The protected-OrbStack/class-inside-it shape. Counting both would subtract the
        // inner bytes twice.
        let named = vec![
            named("/Users/x/Library/Group Containers/V", 1_000),
            named("/Users/x/Library/Group Containers/V/data", 400),
        ];
        let outer = outermost(&named);
        assert_eq!(outer.len(), 1);
        assert_eq!(outer[0].path, "/Users/x/Library/Group Containers/V");
        assert_eq!(
            named_inside("/Users/x/Library/Group Containers", &named),
            1_000
        );
    }

    #[test]
    fn a_directory_that_is_named_or_inside_one_is_fully_named() {
        let named = vec![named("/Users/x/.cargo-target", 100)];
        assert!(fully_named("/Users/x/.cargo-target", &named));
        assert!(fully_named("/Users/x/.cargo-target/release", &named));
        assert!(!fully_named("/Users/x/.cargo", &named));
        // A sibling with the named path as a name prefix is not inside it.
        assert!(!fully_named("/Users/x/.cargo-target-backup", &named));
    }

    #[test]
    fn a_sibling_prefix_is_not_a_prefix_path() {
        // `/a/tb` must not read as inside `/a/t`. This is the same rule `prune`'s nesting
        // check was fixed to keep.
        let named = vec![named("/a/t", 100)];
        assert!(!fully_named("/a/tb", &named));
        assert_eq!(named_inside("/a/tb", &named), 0);
    }

    #[test]
    fn rows_are_the_unnamed_remainder_at_the_requested_depth() {
        let walk = vec![
            DirSize {
                path: "/home".into(),
                bytes: 10_000,
            },
            // 5 GB total, 2 GB named inside: 3 GB unnamed.
            DirSize {
                path: "/home/Library".into(),
                bytes: 5_000,
            },
            DirSize {
                path: "/home/Developer".into(),
                bytes: 4_000,
            },
            // A deeper directory is not reported at depth 1.
            DirSize {
                path: "/home/Library/Caches".into(),
                bytes: 3_000,
            },
        ];
        let named = vec![named("/home/Library/Caches", 2_000)];
        let rows = rows(&walk, &named, "/home", 1);
        assert_eq!(
            rows.iter().map(|r| r.path.as_str()).collect::<Vec<_>>(),
            ["/home/Developer", "/home/Library"]
        );
        assert_eq!(rows[1].unnamed_bytes, 3_000);
        assert_eq!(rows[0].unnamed_bytes, 4_000);
    }

    #[test]
    fn a_fully_named_directory_is_dropped_and_not_shown_as_zero() {
        let walk = vec![
            DirSize {
                path: "/home/.cargo-target".into(),
                bytes: 9_000,
            },
            DirSize {
                path: "/home/.cargo-target/release".into(),
                bytes: 8_000,
            },
        ];
        let named = vec![named("/home/.cargo-target", 9_000)];
        // Depth 1: the named path itself is dropped.
        assert!(rows(&walk, &named, "/home", 1).is_empty());
        // Depth 2: its child lies inside the named path, so it is dropped too.
        assert!(rows(&walk, &named, "/home", 2).is_empty());
    }

    #[test]
    fn a_class_inside_a_protected_path_is_named_once() {
        let classes = vec![(vec!["/h/Library/Caches".to_string()], 500u64)];
        let protected = vec![(vec!["/h/Library".to_string()], 800u64)];
        // The protection already holds the class's bytes; adding the class doubles them.
        assert_eq!(named_bytes(&classes, &protected), 800);
    }

    #[test]
    fn a_protected_path_inside_a_class_path_is_named_once() {
        let classes = vec![(vec!["/h/Library".to_string()], 800u64)];
        let protected = vec![(vec!["/h/Library/Photos".to_string()], 500u64)];
        assert_eq!(named_bytes(&classes, &protected), 800);
    }

    #[test]
    fn unrelated_paths_sum_normally() {
        let classes = vec![(vec!["/h/a".to_string()], 100u64)];
        let protected = vec![(vec!["/h/b".to_string()], 200u64)];
        assert_eq!(named_bytes(&classes, &protected), 300);
    }

    #[test]
    fn shorten_uses_a_tilde_only_under_the_home_directory() {
        assert_eq!(shorten("/Users/x/Library", "/Users/x"), "~/Library");
        assert_eq!(shorten("/Users/x", "/Users/x"), "~");
        assert_eq!(shorten("/Users/xy", "/Users/x"), "/Users/xy");
        assert_eq!(shorten("/tmp/y", "/Users/x"), "/tmp/y");
    }

    #[test]
    fn a_non_zero_exit_makes_the_scan_partial_and_keeps_the_first_error() {
        let scan = scan_state(
            false,
            "\ndu: /x/CloudStorage/.tmp: Resource deadlock avoided\n",
        );
        assert!(!scan.complete);
        assert_eq!(
            scan.error,
            "du: /x/CloudStorage/.tmp: Resource deadlock avoided"
        );
        assert!(scan_state(true, "").complete);
    }
}
