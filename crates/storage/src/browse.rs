//! Policy-independent, single-walk directory allocation snapshots.

use std::cmp::Reverse;
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use status_api::{StorageDirectory, StorageDirectorySnapshot};

use crate::discover::{self, DirSize};
use crate::measure::fmt_bytes;

/// Print one policy-independent allocation snapshot.
///
/// # Errors
/// Returns an error for invalid roots, missing totals or tool/serialization failures.
pub fn run(json: bool, requested: Option<String>, top: usize) -> Result<u8, String> {
    if top == 0 {
        return Err("--top must be at least 1".into());
    }
    let root = requested.unwrap_or_else(|| crate::measure::disk_usage().target);
    let root = if root == "/" && Path::new(discover::DATA_VOLUME).is_dir() {
        discover::DATA_VOLUME.to_owned()
    } else {
        root
    };
    let root = std::fs::canonicalize(&root)
        .map_err(|error| format!("cannot resolve --root {root}: {error}"))?;
    if !root.is_dir() {
        return Err(format!("--root {} is not a directory", root.display()));
    }
    let root = root.to_str().ok_or("--root is not valid UTF-8")?;
    let output = Command::new("du")
        .args(["-x", "-k", "-d", "1", root])
        .output()
        .map_err(|error| format!("could not run du: {error}"))?;
    let scan = discover::scan_state(
        output.status.success(),
        &String::from_utf8_lossy(&output.stderr),
    );
    let walk = discover::parse_du_depth(&String::from_utf8_lossy(&output.stdout));
    let measured_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("cannot timestamp storage snapshot: {error}"))?
        .as_secs();
    let snapshot = snapshot(root, walk, scan, top, measured_at)?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&snapshot).map_err(|error| error.to_string())?
        );
    } else {
        println!("Sjel storage browse · {}", snapshot.root);
        println!(
            "  {} allocated · measured at Unix time {}",
            fmt_bytes(snapshot.bytes),
            snapshot.measured_at
        );
        if snapshot.partial {
            println!(
                "  PARTIAL — sizes are lower bounds: {}",
                snapshot.error.as_deref().unwrap_or("incomplete scan")
            );
        }
        for directory in &snapshot.directories {
            println!("  {:>9}  {}", fmt_bytes(directory.bytes), directory.path);
        }
        println!("  {} not listed (direct files, directory metadata or omitted directories; {} directories omitted)", fmt_bytes(snapshot.unlisted_bytes), snapshot.omitted_directories);
        println!("  Allocated directory bytes can share filesystem blocks; they are not cleanup estimates.");
    }
    Ok(crate::EXIT_OK)
}

fn snapshot(
    root: &str,
    walk: Vec<DirSize>,
    scan: discover::Scan,
    top: usize,
    measured_at: u64,
) -> Result<StorageDirectorySnapshot, String> {
    let bytes = walk
        .iter()
        .find(|directory| directory.path == root)
        .map(|directory| directory.bytes)
        .ok_or_else(|| format!("du returned no total for {root}: {}", scan.error))?;
    let mut directories: Vec<_> = walk
        .into_iter()
        .filter(|directory| discover::depth_of(&directory.path, root) == Some(1))
        .map(|directory| StorageDirectory {
            path: discover::normalize(&directory.path, root),
            bytes: directory.bytes,
        })
        .collect();
    directories.sort_by(|left, right| {
        (Reverse(left.bytes), &left.path).cmp(&(Reverse(right.bytes), &right.path))
    });
    let omitted_directories = directories.len().saturating_sub(top);
    directories.truncate(top);
    let listed_bytes = directories.iter().map(|directory| directory.bytes).sum();
    Ok(StorageDirectorySnapshot {
        root: discover::normalize(root, root),
        measured_at,
        bytes,
        directories,
        unlisted_bytes: bytes.saturating_sub(listed_bytes),
        omitted_directories,
        partial: !scan.complete,
        error: (!scan.error.is_empty()).then_some(scan.error),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn directory(path: &str, bytes: u64) -> DirSize {
        DirSize {
            path: path.into(),
            bytes,
        }
    }

    #[test]
    fn top_limit_keeps_unlisted_allocation_visible() {
        let snapshot = snapshot(
            "/data",
            vec![
                directory("/data", 120),
                directory("/data/a", 40),
                directory("/data/b", 70),
                directory("/data/b/deeper", 30),
            ],
            discover::Scan {
                complete: true,
                error: String::new(),
            },
            1,
            10,
        )
        .unwrap();
        assert_eq!(
            snapshot.directories,
            vec![StorageDirectory {
                path: "/data/b".into(),
                bytes: 70
            }]
        );
        assert_eq!(snapshot.unlisted_bytes, 50);
        assert_eq!(snapshot.omitted_directories, 1);
        assert!(!snapshot.partial);
    }

    #[test]
    fn direct_files_are_counted_even_without_child_directories() {
        let snapshot = snapshot(
            "/data",
            vec![directory("/data", 42)],
            discover::Scan {
                complete: true,
                error: String::new(),
            },
            80,
            10,
        )
        .unwrap();
        assert!(snapshot.directories.is_empty());
        assert_eq!(snapshot.unlisted_bytes, 42);
    }

    #[test]
    fn partial_scan_is_not_reported_as_complete() {
        let snapshot = snapshot(
            "/data",
            vec![directory("/data", 0)],
            discover::Scan {
                complete: false,
                error: "permission denied".into(),
            },
            80,
            10,
        )
        .unwrap();
        assert!(snapshot.partial);
        assert_eq!(snapshot.error.as_deref(), Some("permission denied"));
        assert!(super::snapshot("/data", vec![], discover::Scan::default(), 80, 10).is_err());
    }

    #[test]
    fn data_volume_aliases_are_normalized_for_drilldown() {
        let root = discover::DATA_VOLUME;
        let snapshot = snapshot(
            root,
            vec![
                directory(root, 100),
                directory(&format!("{root}/Users"), 90),
            ],
            discover::Scan {
                complete: true,
                error: String::new(),
            },
            80,
            10,
        )
        .unwrap();
        assert_eq!(snapshot.root, "/");
        assert_eq!(snapshot.directories[0].path, "/Users");
        assert_eq!(snapshot.unlisted_bytes, 10);
    }

    #[test]
    fn real_walk_does_not_follow_child_symlinks() {
        let root = crate::testutil::tempdir("browse");
        let outside = crate::testutil::tempdir("browse-outside");
        std::fs::write(outside.join("payload"), vec![1_u8; 65536]).unwrap();
        std::fs::create_dir(root.join("child")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, root.join("link")).unwrap();
        let output = Command::new("du")
            .args(["-x", "-k", "-d", "1"])
            .arg(&root)
            .output()
            .unwrap();
        assert!(output.status.success());
        let walk = discover::parse_du_depth(&String::from_utf8_lossy(&output.stdout));
        let snapshot = snapshot(
            root.to_str().unwrap(),
            walk,
            discover::Scan {
                complete: true,
                error: String::new(),
            },
            80,
            10,
        )
        .unwrap();
        assert!(snapshot
            .directories
            .iter()
            .all(|directory| !directory.path.ends_with("/link")));
        assert_eq!(
            snapshot.bytes,
            snapshot
                .directories
                .iter()
                .map(|directory| directory.bytes)
                .sum::<u64>()
                + snapshot.unlisted_bytes
        );
        std::fs::remove_dir_all(root).unwrap();
        std::fs::remove_dir_all(outside).unwrap();
    }
}
