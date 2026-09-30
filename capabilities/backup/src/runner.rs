//! Driving the mechanism, and reading back what it produced.
//!
//! `tools/backup.sh` owns the bytes: this module passes an argv array, captures the output
//! into a log, and then reads the immutable receipt the producer wrote so a run row carries
//! the archive's own digest rather than one this process invented.
//!
//! A run that fails is recorded like a run that succeeds. That is the whole reason this
//! module exists: a failed run leaves no receipt, so before this the only durable trace of
//! it was a log line — and `tools/doctor` read the last *successful* receipt and called it
//! fresh.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

use crate::store::{ArchiveIdentity, Fallible, TargetDecl};
use crate::targets::last_line;

pub fn now_epoch() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub struct Outcome {
    pub exit_code: i64,
    pub detail: String,
    pub archive: Option<ArchiveIdentity>,
    pub log_path: PathBuf,
}

/// Where a capability's immutable per-archive receipts live.
fn history_dir(overlay: &Path, capability: &str) -> PathBuf {
    overlay
        .join("backup")
        .join("receipts")
        .join("history")
        .join(capability)
}

/// The newest receipt's identity, if any. Names are `capability-<UTC stamp>.tar.gz.json`,
/// so the newest is the greatest name and no file metadata is consulted.
pub fn newest_archive(overlay: &Path, capability: &str) -> Option<(ArchiveIdentity, PathBuf)> {
    let dir = history_dir(overlay, capability);
    let mut names: Vec<String> = fs::read_dir(&dir)
        .ok()?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".tar.gz.json"))
        .collect();
    names.sort();
    let name = names.pop()?;
    let path = dir.join(&name);
    let text = fs::read_to_string(&path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let sha256 = value.get("sha256")?.as_str()?.to_string();
    let bytes = value.get("bytes")?.as_i64()?;
    let tarball = value
        .get("tarball")
        .and_then(|v| v.as_str())
        .unwrap_or(name.trim_end_matches(".json"))
        .to_string();
    Some((
        ArchiveIdentity {
            name: tarball,
            bytes,
            sha256,
        },
        path,
    ))
}

/// A failed attempt, written where an offline check can read it.
///
/// `tools/doctor` is offline by contract and must not have to reach this surface over HTTP
/// to learn that last night's run failed. One small file per capability, removed the moment
/// a run succeeds, is the smallest thing that makes a failure outlive the log.
fn write_attempt(overlay: &Path, capability: &str, value: &serde_json::Value) -> Fallible<()> {
    let dir = overlay.join("backup").join("receipts").join("attempts");
    fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{capability}.json"));
    let mut file = fs::File::create(&path)?;
    file.write_all(serde_json::to_string_pretty(value)?.as_bytes())?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    Ok(())
}

fn clear_attempt(overlay: &Path, capability: &str) {
    let _ = fs::remove_file(
        overlay
            .join("backup")
            .join("receipts")
            .join("attempts")
            .join(format!("{capability}.json")),
    );
}

/// Run one capability against one target. Blocking: callers put this on `spawn_blocking`.
pub fn run(root: &Path, overlay: &Path, capability: &str, target: &str) -> Outcome {
    let logs = overlay.join("backup").join("logs");
    let _ = fs::create_dir_all(&logs);
    let log_path = logs.join(format!("{capability}-{}.log", now_epoch()));

    let before = newest_archive(overlay, capability).map(|(archive, _)| archive.name);

    let log = fs::File::create(&log_path);
    let mut command = Command::new(root.join("tools/backup.sh"));
    command.arg("--target").arg(target).arg(capability);
    if let Ok(file) = log {
        match file.try_clone() {
            Ok(second) => {
                command.stdout(Stdio::from(file));
                command.stderr(Stdio::from(second));
            }
            Err(_) => {
                command.stdout(Stdio::null()).stderr(Stdio::null());
            }
        }
    } else {
        command.stdout(Stdio::null()).stderr(Stdio::null());
    }

    let status = command.status();
    let (exit_code, detail) = match status {
        Ok(status) => {
            let code = status.code().unwrap_or(-1) as i64;
            let detail = if code == 0 {
                String::new()
            } else {
                // The log is where the reason is; a shell tool's last line is its message.
                fs::read_to_string(&log_path)
                    .map(|text| last_line(&text))
                    .unwrap_or_default()
            };
            (code, detail)
        }
        Err(error) => (-1, format!("could not run tools/backup.sh: {error}")),
    };

    // A receipt that appeared during this run is this run's archive. Recording the newest
    // one unconditionally would attribute a previous run's archive to a run that produced
    // nothing, which is the mis-attribution this check exists to prevent.
    let archive = newest_archive(overlay, capability).and_then(|(archive, _)| {
        if before.as_deref() == Some(archive.name.as_str()) {
            None
        } else {
            Some(archive)
        }
    });

    if exit_code == 0 {
        clear_attempt(overlay, capability);
    } else {
        let _ = write_attempt(
            overlay,
            capability,
            &serde_json::json!({
                "capability": capability,
                "target": target,
                "at_epoch": now_epoch(),
                "exit_code": exit_code,
                "detail": detail,
                "log_path": log_path.to_string_lossy(),
            }),
        );
    }

    Outcome {
        exit_code,
        detail,
        archive,
        log_path,
    }
}

/// Hash a file in Rust, so the comparison is between two independent statements.
fn digest_of(path: &Path) -> Fallible<String> {
    use std::io::Read;
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

pub struct Verdict {
    pub verdict: &'static str,
    pub detail: String,
    pub archive: Option<ArchiveIdentity>,
}

/// The latest receipt a capability wrote, for capabilities whose archives predate the
/// per-archive history: `backup.sh` has always written this one, and it names its own
/// tarball, so it identifies the archive it is about.
fn latest_receipt_archive(overlay: &Path, capability: &str) -> Option<(ArchiveIdentity, PathBuf)> {
    let path = overlay
        .join("backup")
        .join("receipts")
        .join(format!("{capability}.json"));
    let value: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path).ok()?).ok()?;
    let sha256 = value.get("sha256")?.as_str()?.to_string();
    let name = value.get("tarball")?.as_str()?.to_string();
    let bytes = value.get("bytes").and_then(|v| v.as_i64()).unwrap_or(0);
    Some((
        ArchiveIdentity {
            name,
            bytes,
            sha256,
        },
        path,
    ))
}

/// The archive a rehearsal should use for one capability: the newest one it recorded.
fn recorded_archive(overlay: &Path, capability: &str) -> Option<(ArchiveIdentity, PathBuf)> {
    newest_archive(overlay, capability).or_else(|| latest_receipt_archive(overlay, capability))
}

/// Rehearse a target: hash the newest archive the producer recorded, then restore it in
/// isolation with its own receipt.
///
/// The definition is per kind, and the honest value of two of the three is "not checked":
/// a local path proves its bytes are here, and an `ssh` target cannot be rehearsed without
/// the vault agent — which this process has no business unlocking on a timer. Saying
/// `unchecked` with the reason is the same discipline `tools/doctor` uses for a remote
/// archive it cannot look at.
pub fn verify(root: &Path, overlay: &Path, target: &TargetDecl) -> Verdict {
    if target.kind != "local" {
        return Verdict {
            verdict: "unchecked",
            detail: format!(
                "kind={} has no rehearsal path yet: it needs the remote copy retrieved, and an ssh target needs the vault agent",
                target.kind
            ),
            archive: None,
        };
    }
    if target.declared_by.is_empty() {
        return Verdict {
            verdict: "failed",
            detail: "no capability declares this target, so there is no archive to rehearse".into(),
            archive: None,
        };
    }
    // Rehearse an archive that is actually there. Taking the first declared capability was
    // the wrong question: a capability can declare a target and have no archive recorded yet,
    // and reporting that as a failed rehearsal would blame the target for the calendar.
    // A candidate whose archive is missing from the target is still a real failure, so the
    // first recorded candidate is kept as the fallback when none can be found on disk.
    let mut fallback: Option<(String, ArchiveIdentity, PathBuf)> = None;
    let mut candidate: Option<(String, ArchiveIdentity, PathBuf)> = None;
    for name in &target.declared_by {
        let Some((archive, receipt_path)) = recorded_archive(overlay, name) else {
            continue;
        };
        let at_target = Path::new(&target.path)
            .join(name)
            .join(&archive.name)
            .is_file();
        if at_target {
            candidate = Some((name.clone(), archive, receipt_path));
            break;
        }
        if fallback.is_none() {
            fallback = Some((name.clone(), archive, receipt_path));
        }
    }
    let Some((capability, archive, receipt_path)) = candidate.or(fallback) else {
        return Verdict {
            verdict: "failed",
            detail: format!(
                "no capability declaring '{}' has a recorded archive yet, so there is nothing to rehearse",
                target.id
            ),
            archive: None,
        };
    };
    let archive_path = Path::new(&target.path)
        .join(&capability)
        .join(&archive.name);
    if !archive_path.is_file() {
        return Verdict {
            verdict: "failed",
            detail: format!(
                "{capability}'s recorded archive is not at the target: {}",
                archive.name
            ),
            archive: Some(archive),
        };
    }
    match digest_of(&archive_path) {
        Ok(observed) if observed != archive.sha256 => {
            return Verdict {
                verdict: "failed",
                detail: format!(
                    "the archive at the target hashes to {}… but {}… was recorded",
                    &observed[..12.min(observed.len())],
                    &archive.sha256[..12.min(archive.sha256.len())]
                ),
                archive: Some(archive),
            }
        }
        Ok(_) => {}
        Err(error) => {
            return Verdict {
                verdict: "failed",
                detail: format!("could not read the archive at the target: {error}"),
                archive: Some(archive),
            }
        }
    }

    let scratch = std::env::temp_dir().join(format!("sjel-backup-verify-{}", std::process::id()));
    let _ = fs::remove_dir_all(&scratch);
    // `restore.sh` creates the destination itself but requires its PARENT to exist — it
    // refuses to invent a path, which is right, and makes creating the parent this
    // process's job. Measured by running the rehearsal: without this the verdict was
    // "destination parent does not exist" for a target that was perfectly fine.
    if let Err(error) = fs::create_dir_all(&scratch) {
        return Verdict {
            verdict: "failed",
            detail: format!("could not create a scratch directory for the rehearsal: {error}"),
            archive: Some(archive),
        };
    }
    let destination = scratch.join("restored");
    let output = Command::new(root.join("tools/restore.sh"))
        .arg(&capability)
        .arg(&archive_path)
        .arg("--receipt")
        .arg(&receipt_path)
        .arg("--destination")
        .arg(&destination)
        .output();
    let verdict = match output {
        Ok(output) if output.status.success() => Verdict {
            verdict: "verified",
            detail: format!(
                "{} hashed and restored in isolation from its own receipt",
                archive.name
            ),
            archive: Some(archive),
        },
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
            let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
            let line = last_line(&stderr);
            let line = if line.is_empty() {
                last_line(&stdout)
            } else {
                line
            };
            Verdict {
                verdict: "failed",
                detail: format!("restore.sh refused the rehearsal: {line}"),
                archive: Some(archive),
            }
        }
        Err(error) => Verdict {
            verdict: "failed",
            detail: format!("could not run tools/restore.sh: {error}"),
            archive: Some(archive),
        },
    };
    // The rehearsal ends in a scratch tree, and the tool never removes its own evidence; a
    // rehearsal re-run every time a target is checked would otherwise fill the disk with
    // restored copies of the same archive.
    let _ = fs::remove_dir_all(&scratch);
    verdict
}
