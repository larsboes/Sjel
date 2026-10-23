//! Uninstalls what the operator decided to remove, through whoever installed it.
//!
//! An app or leftover moves to `~/.Trash`, so Finder's Put Back still works and nothing is gone
//! until the Trash is emptied. A cask goes through `brew uninstall --cask`, a formula through
//! `brew uninstall`, and cargo, npm and uv packages through their own uninstall commands, so each
//! manager's record stays true; moving a cask's app by hand is exactly how `macspace` became a
//! cask record with no app.
//!
//! Refused: a path that resolves outside `/Applications`, `~/Applications` and `~/Library`, and
//! an app that is running. The caller has already checked the decision is `remove`.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::store::App;

/// What one uninstall did, step by step, for the response and the log.
pub type Steps = Vec<String>;

/// The PATH a manager's uninstall runs with: launchd gives a service none of these.
const TOOL_PATH: &str = "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin";

pub fn uninstall(app: &App, home: &Path) -> Result<Steps, String> {
    let mut steps = Steps::new();
    match app.kind.as_str() {
        "tool" => steps.push(manager_uninstall(app, home)?),
        "app" => {
            if running(&app.path) {
                return Err(format!("{} is running; quit it first", app.name));
            }
            if let Some(token) = app.source.strip_prefix("cask:") {
                steps.push(run(home, "brew", &["uninstall", "--cask", token])?);
            } else {
                steps.push(trash(Path::new(&app.path), home)?);
            }
            steps.extend(trash_all(&app.data_paths, home)?);
        }
        "leftover" => steps.extend(trash_all(&app.data_paths, home)?),
        other => return Err(format!("nothing knows how to uninstall a '{other}'")),
    }
    Ok(steps)
}

fn manager_uninstall(app: &App, home: &Path) -> Result<String, String> {
    let name = app.name.as_str();
    match app.source.as_str() {
        "brew" => run(home, "brew", &["uninstall", name]),
        "cask" => run(home, "brew", &["uninstall", "--cask", name]),
        "cargo" => run(home, "cargo", &["uninstall", name]),
        "npm" => run(home, "npm", &["uninstall", "-g", name]),
        "uv" => run(home, "uv", &["tool", "uninstall", name]),
        other => Err(format!("no uninstall command for '{other}' tools")),
    }
}

/// Runs a manager's command and returns its first output line, or its error.
fn run(home: &Path, program: &str, args: &[&str]) -> Result<String, String> {
    let resolved = resolve(home, program).ok_or_else(|| format!("{program} is not installed"))?;
    let out = Command::new(&resolved)
        .args(args)
        .env(
            "PATH",
            format!("{}:{TOOL_PATH}", home.join(".cargo/bin").display()),
        )
        .env("HOME", home)
        .output()
        .map_err(|e| format!("{program}: {e}"))?;
    let command = format!("{program} {}", args.join(" "));
    if out.status.success() {
        Ok(command)
    } else {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let reason = stderr
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("failed");
        Err(format!("{command}: {reason}"))
    }
}

fn resolve(home: &Path, program: &str) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = vec![home.join(".cargo/bin"), home.join(".local/bin")];
    dirs.extend(TOOL_PATH.split(':').map(PathBuf::from));
    dirs.into_iter()
        .map(|d| d.join(program))
        .find(|p| p.exists())
}

fn trash_all(paths: &[String], home: &Path) -> Result<Steps, String> {
    // Every path is checked before any moves, so a refusal leaves everything where it was.
    for path in paths {
        allowed(Path::new(path), home)?;
    }
    paths
        .iter()
        .filter(|p| std::fs::symlink_metadata(p).is_ok())
        .map(|p| trash(Path::new(p), home))
        .collect()
}

/// Moves `path` into `~/.Trash`, renaming on a clash the way Finder does.
fn trash(path: &Path, home: &Path) -> Result<String, String> {
    allowed(path, home)?;
    let bin = home.join(".Trash");
    let name = path
        .file_name()
        .ok_or_else(|| format!("{} has no file name", path.display()))?;
    let mut target = bin.join(name);
    let mut n = 2;
    while std::fs::symlink_metadata(&target).is_ok() {
        target = bin.join(format!("{} {n}", name.to_string_lossy()));
        n += 1;
    }
    std::fs::rename(path, &target).map_err(|e| {
        format!(
            "could not move {} to the Trash: {e}{}",
            path.display(),
            if e.kind() == std::io::ErrorKind::PermissionDenied {
                " (owned by another user; delete it in Finder, which asks for an admin password)"
            } else {
                ""
            }
        )
    })?;
    Ok(format!("trashed {}", path.display()))
}

/// The path, with its parent's symlinks resolved, sits under one of the three roots. The entry
/// itself is not resolved: a symlink in `~/Library` is trashed as the link, never its target.
pub fn allowed(path: &Path, home: &Path) -> Result<(), String> {
    let parent = path
        .parent()
        .and_then(|p| p.canonicalize().ok())
        .ok_or_else(|| format!("{} is not reachable", path.display()))?;
    let roots = [
        PathBuf::from("/Applications"),
        home.join("Applications"),
        home.join("Library"),
    ];
    let inside = roots.iter().any(|root| {
        root.canonicalize()
            .is_ok_and(|root| parent.starts_with(root))
    });
    let named = path.file_name().is_some_and(|n| n != ".." && n != ".");
    if inside && named {
        Ok(())
    } else {
        Err(format!(
            "{} is outside /Applications, ~/Applications and ~/Library; refused",
            path.display()
        ))
    }
}

/// True when a process runs the bundle's executable.
fn running(bundle: &str) -> bool {
    if bundle.is_empty() {
        return false;
    }
    Command::new("pgrep")
        .args(["-f", &format!("{bundle}/Contents/MacOS/")])
        .output()
        .is_ok_and(|out| out.status.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_paths_under_the_three_roots_are_allowed() {
        let home = std::env::temp_dir().join(format!("sjel-software-home-{}", std::process::id()));
        let lib = home.join("Library/Caches");
        std::fs::create_dir_all(&lib).unwrap();
        assert!(allowed(&lib.join("com.example.app"), &home).is_ok());
        assert!(allowed(&home.join("Documents/thing"), &home).is_err());
        assert!(allowed(&lib.join(".."), &home).is_err());
        assert!(allowed(Path::new("/etc/hosts"), &home).is_err());
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn trash_moves_and_renames_on_a_clash() {
        let home = std::env::temp_dir().join(format!("sjel-software-trash-{}", std::process::id()));
        let caches = home.join("Library/Caches");
        std::fs::create_dir_all(&caches).unwrap();
        std::fs::create_dir_all(home.join(".Trash/com.example.app")).unwrap();
        let victim = caches.join("com.example.app");
        std::fs::create_dir_all(&victim).unwrap();
        trash(&victim, &home).unwrap();
        assert!(!victim.exists());
        assert!(home.join(".Trash/com.example.app 2").exists());
        std::fs::remove_dir_all(&home).unwrap();
    }
}
