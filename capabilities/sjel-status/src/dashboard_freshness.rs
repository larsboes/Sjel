//! Keeps the dashboard bundle this shell serves in step with the dashboard's source.
//!
//! The shell serves `dashboard/dist`, a build artifact, and nothing rebuilt it: the runner
//! counted any non-cargo artifact as current once it existed (`tools/sjel-cli/src/runner.rs`,
//! `artifact_is_current`). On 2026-10-07 the bundle the menu bar app opens was two days older
//! than the source, so a fix and a new panel were live on the Vite dev port and invisible on
//! this one. Here the shell checks every 30 s and runs `bun run build` once when the source is
//! newer than the bundle.
//!
//! ponytail: `vite build` empties `dist` first, so for the ~2 s of a build a page load can 404.
//! Build into a scratch directory and swap it in if that window ever matters.
//!
//! `SJEL_DASHBOARD_AUTOBUILD=0` turns it off. `SJEL_BUN` names bun when launchd's PATH lacks it.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

const INTERVAL: Duration = Duration::from_secs(30);
/// A file written this recently may be one save of several; build after the burst.
const SETTLE: Duration = Duration::from_secs(5);
/// What a build reads. `node_modules`, `.svelte-kit` and `dist` are outputs or vendored.
const INPUTS: &[&str] = &[
    "src",
    "static",
    "package.json",
    "svelte.config.js",
    "vite.config.ts",
];

pub fn spawn(dist: PathBuf) {
    if sjel_config::env_var("SJEL_DASHBOARD_AUTOBUILD").is_ok_and(|v| v == "0") {
        return;
    }
    let Some(root) = dist.parent().map(Path::to_path_buf) else {
        return;
    };
    // launchd hands this job a PATH without /opt/homebrew/bin (sjel-personal machine.toml,
    // [capability.backup]), so a bare `bun` is the last resort, not the first.
    let bun = sjel_config::env_var("SJEL_BUN").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_default();
        [
            "/opt/homebrew/bin/bun".to_string(),
            format!("{home}/.bun/bin/bun"),
            "/usr/local/bin/bun".to_string(),
        ]
        .into_iter()
        .find(|p| Path::new(p).is_file())
        .unwrap_or_else(|| "bun".to_string())
    });
    std::thread::spawn(move || {
        // The newest input a failed build was attempted against; retried only once it moves.
        let mut failed_at: Option<SystemTime> = None;
        loop {
            if let Some(newest) = newest_input(&root) {
                let built = modified(&dist.join("index.html"));
                if is_stale(newest, built, SystemTime::now()) && failed_at != Some(newest) {
                    eprintln!("[sjel-status] dashboard source is newer than {dist:?}; rebuilding");
                    match Command::new(&bun)
                        .args(["run", "build"])
                        .current_dir(&root)
                        .output()
                    {
                        Ok(out) if out.status.success() => {
                            failed_at = None;
                            eprintln!("[sjel-status] dashboard bundle rebuilt");
                        }
                        Ok(out) => {
                            failed_at = Some(newest);
                            let tail = String::from_utf8_lossy(&out.stderr);
                            let tail: String =
                                tail.lines().rev().take(5).collect::<Vec<_>>().join(" | ");
                            eprintln!(
                                "[sjel-status] dashboard build failed ({}): {tail}",
                                out.status
                            );
                        }
                        Err(e) => {
                            failed_at = Some(newest);
                            eprintln!("[sjel-status] dashboard build could not start ({bun}): {e}");
                        }
                    }
                }
            }
            std::thread::sleep(INTERVAL);
        }
    });
}

/// Stale when an input is newer than the bundle and has settled. A missing bundle is stale.
fn is_stale(newest_input: SystemTime, built: Option<SystemTime>, now: SystemTime) -> bool {
    let settled = now
        .duration_since(newest_input)
        .is_ok_and(|age| age >= SETTLE);
    settled && built.is_none_or(|built| newest_input > built)
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn newest_input(root: &Path) -> Option<SystemTime> {
    let mut newest = None;
    let mut stack: Vec<PathBuf> = INPUTS.iter().map(|p| root.join(p)).collect();
    while let Some(path) = stack.pop() {
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        if meta.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&path) {
                stack.extend(entries.flatten().map(|e| e.path()));
            }
        } else if let Ok(m) = meta.modified() {
            if newest.is_none_or(|n| m > n) {
                newest = Some(m);
            }
        }
    }
    newest
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(secs: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(secs)
    }

    #[test]
    fn source_newer_than_the_bundle_is_stale_once_it_settles() {
        assert!(is_stale(at(100), Some(at(50)), at(200)));
        // Still being saved: wait for the burst to end.
        assert!(!is_stale(at(100), Some(at(50)), at(102)));
        // Bundle already newer than every input.
        assert!(!is_stale(at(100), Some(at(150)), at(200)));
        // No bundle at all.
        assert!(is_stale(at(100), None, at(200)));
    }

    #[test]
    fn newest_input_reads_nested_source_and_ignores_outputs() {
        let dir = std::env::temp_dir().join(format!("sjel-dash-fresh-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src/lib/travel")).unwrap();
        std::fs::create_dir_all(dir.join("dist")).unwrap();
        let src = dir.join("src/lib/travel/Panel.svelte");
        std::fs::write(&src, "x").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&src)
            .unwrap()
            .set_modified(at(1_000))
            .unwrap();
        let out = dir.join("dist/index.html");
        std::fs::write(&out, "x").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&out)
            .unwrap()
            .set_modified(at(9_000))
            .unwrap();
        assert_eq!(newest_input(&dir), Some(at(1_000)));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
