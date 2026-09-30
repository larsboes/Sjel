//! The declared targets, read from the one place that already derives them.
//!
//! "Which capabilities are backed up, and to where" is answered by
//! `tools/backup-all.sh --list` and `--targets-json`. That script derives the set from the
//! capability registry, so re-deriving it here would be a second definition of the same
//! fact — the drift this repository refuses everywhere else (`capabilities/store/README.md`
//! "import, never redefine").

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::store::{Fallible, TargetDecl};

/// The Axon checkout `tools/` lives in.
///
/// `SJEL_ROOT` first, because `tools/lib/paths.sh` exports it and every entry point that
/// matters sets it. The error names the fix rather than guessing a path: a capability
/// resolving its own sibling directory by walking up from `argv[0]` works until the binary
/// is installed somewhere else, and then reports all targets missing.
pub fn root() -> Fallible<PathBuf> {
    sjel_config::env_var("SJEL_ROOT")
        .map(PathBuf::from)
        .map_err(|_| "SJEL_ROOT is not set — start this through tools/service-runner.sh".into())
}

/// `(capability, target_id)` per contract, in registry order.
pub fn contracts(root: &Path) -> Fallible<Vec<(String, String)>> {
    let output = Command::new(root.join("tools/backup-all.sh"))
        .arg("--list")
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "backup-all.sh --list failed: {}",
            last_line(&String::from_utf8_lossy(&output.stderr))
        )
        .into());
    }
    let text = String::from_utf8(output.stdout)?;
    let rows = text
        .lines()
        .filter_map(|line| line.split_once('\t'))
        .map(|(cap, target)| (cap.trim().to_string(), target.trim().to_string()))
        .filter(|(cap, target)| !cap.is_empty() && !target.is_empty())
        .collect();
    Ok(rows)
}

/// The declared targets with their kind and coordinates, as the tools resolved them.
pub fn declared(root: &Path) -> Fallible<Vec<TargetDecl>> {
    let output = Command::new(root.join("tools/backup-all.sh"))
        .arg("--targets-json")
        .output()?;
    let text = String::from_utf8(output.stdout)?;
    // A non-zero exit with parseable output is still an answer (`[]` plus a named reason on
    // stderr when the overlay has no coordinates), so the JSON is what decides, and the
    // exit status is only reported when there is nothing to parse.
    match serde_json::from_str::<Vec<TargetDecl>>(&text) {
        Ok(targets) => Ok(targets),
        Err(error) => Err(format!(
            "backup-all.sh --targets-json did not answer with JSON ({error}); stderr: {}",
            last_line(&String::from_utf8_lossy(&output.stderr))
        )
        .into()),
    }
}

/// The last non-empty line of captured output: where a shell tool writes its reason.
pub fn last_line(text: &str) -> String {
    text.lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("")
        .trim()
        .to_string()
}
