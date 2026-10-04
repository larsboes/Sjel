//! Turning bytes on disk into the numbers a human acts on.
//!
//! Two measuring instruments, chosen for two different jobs.
//!
//! `du -sx` measures the policy's classes: it is the kernel's own accounting, it stays on
//! one filesystem, and it reports allocated blocks — which is what actually frees. A class
//! can point anywhere on the machine, including directories this process cannot read, and
//! `du` degrades to a partial number where a walk in here would fail outright.
//!
//! An in-process walk measures the Cargo target dir, because `target` reports per bucket
//! (`deps`, `incremental`, `build`, binaries) and shelling out once per bucket would walk
//! the same tree four times. It uses the same unit `du` does — allocated blocks — and
//! dedups hard links the same way, so the two instruments agree on the same directory.

use std::collections::HashSet;
use std::path::Path;
use std::process::Command;

use serde::Serialize;

use crate::policy::GB;

const MB: u64 = 1024 * 1024;

/// Report-column formatting. The unit switches at 1 GB so the column stays comparable.
pub fn fmt_bytes(b: u64) -> String {
    if b >= GB {
        format!("{:.1} GB", b as f64 / GB as f64)
    } else {
        format!("{} MB", (b as f64 / MB as f64).round() as u64)
    }
}

/// The volume a cleanup actually changes a number on. Field order is the JSON contract
/// `tools/host-watch` and the dashboard already read.
#[derive(Debug, Clone, Serialize)]
pub struct Disk {
    pub total: u64,
    pub used: u64,
    pub free: u64,
    pub target: String,
}

/// Parse `df -k <target>`: second line, columns are total/used/free in 1K blocks.
pub fn parse_df(text: &str) -> (u64, u64, u64) {
    let cols: Vec<&str> = text
        .lines()
        .nth(1)
        .unwrap_or_default()
        .split_whitespace()
        .collect();
    let at = |i: usize| -> u64 {
        cols.get(i)
            .and_then(|c| c.parse::<u64>().ok())
            .unwrap_or_default()
            * 1024
    };
    (at(1), at(2), at(3))
}

/// Sum `du -sx -k` output. A missing path simply does not appear, so it contributes 0.
pub fn parse_du(text: &str) -> u64 {
    text.lines()
        .filter(|l| !l.is_empty())
        .map(|l| {
            l.split('\t')
                .next()
                .and_then(|n| n.trim().parse::<u64>().ok())
                .unwrap_or_default()
                * 1024
        })
        .sum()
}

pub fn size_of(paths: &[String]) -> u64 {
    if paths.is_empty() {
        return 0;
    }
    let out = Command::new("du").arg("-sx").arg("-k").args(paths).output();
    match out {
        Ok(o) => parse_du(&String::from_utf8_lossy(&o.stdout)),
        Err(_) => 0,
    }
}

pub fn disk_usage() -> Disk {
    // APFS volume groups: `/` is the sealed read-only System volume and is always small.
    // The Data volume is where a cleanup actually changes a number. Falls back to `/` on
    // Linux and on macOS releases predating volume groups.
    let target = if Path::new("/System/Volumes/Data").exists() {
        "/System/Volumes/Data"
    } else {
        "/"
    };
    let text = Command::new("df")
        .arg("-k")
        .arg(target)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    let (total, used, free) = parse_df(&text);
    Disk {
        total,
        used,
        free,
        target: target.to_string(),
    }
}

/// A recursive size walk that counts each on-disk extent once.
///
/// Two properties it has to keep to agree with `du -s`:
///
/// * **Allocated blocks, not `len()`.** A file's apparent size is not what freeing it
///   returns. `blocks() * 512` is the number the volume gets back.
/// * **One count per inode.** Cargo hard-links the finished binary from
///   `target/<profile>/deps/` up to `target/<profile>/`, so a naive walk reports it twice.
///   The set is shared across every bucket of one profile, and buckets are walked in a
///   fixed order, so a hard link is attributed to whichever bucket reaches it first —
///   exactly what `du` does with the first path it walks.
///
/// Symlinks are not followed: `read_dir` entry metadata is an `lstat`, so a link counts as
/// the few bytes it is and never leaves the tree.
#[derive(Default)]
pub struct Walk {
    seen: HashSet<(u64, u64)>,
}

impl Walk {
    pub fn size(&mut self, path: &Path) -> u64 {
        // `symlink_metadata` rather than `metadata`, and it is checked before `read_dir`:
        // `read_dir` follows a symlink to a directory, so testing the entry type
        // afterwards would already have left the tree.
        let Ok(meta) = std::fs::symlink_metadata(path) else {
            // The path does not exist. Zero is the honest answer, not an error — a policy
            // may name a cache this machine has never created.
            return 0;
        };
        let mut total = self.charge(&meta);
        if !meta.is_dir() {
            return total;
        }
        let Ok(entries) = std::fs::read_dir(path) else {
            return total;
        };
        for entry in entries.flatten() {
            total += self.size(&entry.path());
        }
        total
    }

    /// Zero for an inode already counted in this walk.
    #[cfg(unix)]
    fn charge(&mut self, meta: &std::fs::Metadata) -> u64 {
        use std::os::unix::fs::MetadataExt;
        if self.seen.insert((meta.dev(), meta.ino())) {
            meta.blocks() * 512
        } else {
            0
        }
    }

    #[cfg(not(unix))]
    fn charge(&mut self, meta: &std::fs::Metadata) -> u64 {
        // No inode identity to dedup on, and no allocated-block count. Apparent size is
        // the best available answer; Sjel's declared platforms are all unix.
        meta.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_du_sums_1k_block_totals_across_paths() {
        assert_eq!(parse_du("1024\t/a\n2048\t/b\n"), 3072 * 1024);
    }

    #[test]
    fn parse_du_treats_absent_paths_as_zero_since_du_omits_them_entirely() {
        assert_eq!(parse_du(""), 0);
    }

    #[test]
    fn parse_df_reads_the_second_line_not_the_header() {
        let df = concat!(
            "Filesystem   1024-blocks      Used Available Capacity  Mounted on\n",
            "/dev/disk3s5   482797652 308596224 148500000    68%    /System/Volumes/Data\n",
        );
        assert_eq!(
            parse_df(df),
            (482797652 * 1024, 308596224 * 1024, 148500000 * 1024)
        );
    }

    #[test]
    fn parse_df_on_empty_output_reports_zero_rather_than_a_wrong_volume() {
        assert_eq!(parse_df(""), (0, 0, 0));
    }

    #[test]
    fn fmt_switches_unit_at_1_gb_and_keeps_the_report_columns_comparable() {
        assert_eq!(fmt_bytes(46 * GB), "46.0 GB");
        assert_eq!(fmt_bytes(GB), "1.0 GB");
        assert_eq!(fmt_bytes(512 * MB), "512 MB");
        assert_eq!(fmt_bytes(0), "0 MB");
    }

    #[test]
    fn a_walk_counts_a_hard_linked_file_once() {
        let root = crate::testutil::tempdir("walk-hardlink");
        let deps = root.join("deps");
        std::fs::create_dir_all(&deps).unwrap();
        // 64 KiB, so the size is several blocks and a double count would be obvious.
        std::fs::write(deps.join("bin-abc123"), vec![0u8; 64 * 1024]).unwrap();
        std::fs::hard_link(deps.join("bin-abc123"), root.join("bin")).unwrap();

        let mut walk = Walk::default();
        let deps_bytes = walk.size(&deps);
        let after_hardlink = walk.size(&root.join("bin"));
        assert!(deps_bytes >= 64 * 1024, "deps: {deps_bytes}");
        assert_eq!(
            after_hardlink, 0,
            "the second path to the same inode frees nothing and must be charged nothing"
        );
    }

    #[test]
    fn a_walk_of_a_missing_path_is_zero_not_an_error() {
        let mut walk = Walk::default();
        assert_eq!(walk.size(Path::new("/nonexistent-axon-fixture/target")), 0);
    }

    #[test]
    fn a_walk_does_not_follow_a_symlink_out_of_the_tree() {
        let root = crate::testutil::tempdir("walk-symlink");
        let inside = root.join("inside");
        std::fs::create_dir_all(&inside).unwrap();
        std::fs::write(inside.join("f"), vec![0u8; 32 * 1024]).unwrap();
        let linked = root.join("link");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&inside, &linked).unwrap();

        let mut only_link = Walk::default();
        let via_link = only_link.size(&linked);
        let mut direct = Walk::default();
        let via_dir = direct.size(&inside);
        assert!(via_dir >= 32 * 1024, "direct walk: {via_dir}");
        assert!(
            via_link < 32 * 1024,
            "a symlink must count as the link, not the tree it points at: {via_link}"
        );
    }
}
