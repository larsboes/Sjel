//! `sjel worktree` — one branch and one checkout per work package.
//!
//! Worktrees live under `<primary>/.claude/worktrees/<branch>`, the directory Claude Code uses
//! for its own worktrees and the repository's checks already skip. A new worktree gets the
//! machine-local files a checkout reads but git does not carry (`LOCAL_FILES`), so the overlay and
//! the machine resolve the same way as in the primary checkout.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const USAGE: &str = "
sjel worktree new <area>-<slug> [--from <ref>]   branch + worktree from origin/main (or <ref>)
sjel worktree list                               every worktree of this repository
sjel worktree remove <area>-<slug> [--force]     remove a worktree and its local branch

Worktrees live in <primary checkout>/.claude/worktrees/<branch>. `new` copies sjel.local.toml,
axon.local.toml and .claude/settings.local.json from the primary checkout when they exist.
`remove` refuses a worktree with uncommitted changes, or with commits on no remote and no
merged pull request, unless --force.
";

/// Gitignored files a checkout needs to resolve its overlay, machine and harness settings.
const LOCAL_FILES: [&str; 3] = [
    "sjel.local.toml",
    "axon.local.toml",
    ".claude/settings.local.json",
];

pub fn run(args: &[String]) -> ExitCode {
    let has = |f: &str| args.iter().any(|a| a == f);
    if args.is_empty() || has("-h") || has("--help") {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    let result = match args[0].as_str() {
        "new" => new(&args[1..]),
        "list" => git_in(&primary_or_cwd(), &["worktree", "list"]).map(|o| print!("{o}")),
        "remove" => remove(&args[1..]),
        other => Err(format!("unknown verb '{other}'")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("sjel worktree: {e}");
            ExitCode::from(1)
        }
    }
}

/// `<area>-<slug>`: lowercase ASCII words joined by single hyphens, at least two of them.
pub fn valid_branch(name: &str) -> bool {
    let words: Vec<&str> = name.split('-').collect();
    words.len() >= 2
        && words.iter().all(|w| {
            !w.is_empty()
                && w.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        })
}

fn new(args: &[String]) -> Result<(), String> {
    let (name, from) = match args {
        [name] => (name, None),
        [name, flag, base] if flag == "--from" => (name, Some(base.as_str())),
        _ => return Err("usage: sjel worktree new <area>-<slug> [--from <ref>]".to_owned()),
    };
    if !valid_branch(name) {
        return Err(format!(
            "'{name}' is not <area>-<slug> (lowercase words joined by hyphens)"
        ));
    }
    let primary = primary_checkout()?;
    let dest = worktree_dir(&primary, name);
    if dest.exists() {
        return Err(format!("{} already exists", dest.display()));
    }
    if git_in(
        &primary,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/heads/{name}"),
        ],
    )
    .is_ok()
    {
        return Err(format!("branch '{name}' already exists"));
    }
    let base = match from {
        Some(b) => b.to_owned(),
        None => {
            git_in(&primary, &["fetch", "--quiet", "origin", "main"])
                .map_err(|e| format!("cannot fetch origin main ({e}); pass --from <ref>"))?;
            "origin/main".to_owned()
        }
    };
    let dest_str = dest.display().to_string();
    git_in(
        &primary,
        &[
            "worktree",
            "add",
            "--quiet",
            "--no-track",
            "-b",
            name,
            &dest_str,
            &base,
        ],
    )?;
    println!("worktree {dest_str} on branch {name}, from {base}");
    for rel in LOCAL_FILES {
        let src = primary.join(rel);
        let dst = dest.join(rel);
        if src.is_file() && !dst.exists() {
            if let Some(parent) = dst.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            std::fs::copy(&src, &dst).map_err(|e| format!("copy {rel}: {e}"))?;
            println!("  copied {rel}");
        }
    }
    println!("next: cd {dest_str}  (the first build there compiles into its own target/)");
    Ok(())
}

fn remove(args: &[String]) -> Result<(), String> {
    let force = args.iter().any(|a| a == "--force");
    let names: Vec<&String> = args.iter().filter(|a| *a != "--force").collect();
    let [name] = names.as_slice() else {
        return Err("usage: sjel worktree remove <area>-<slug> [--force]".to_owned());
    };
    let primary = primary_checkout()?;
    let dest = worktree_dir(&primary, name);
    if !dest.exists() {
        return Err(format!("no worktree at {}", dest.display()));
    }
    if !force {
        let dirty = git_in(&dest, &["status", "--porcelain"])?;
        if !dirty.trim().is_empty() {
            return Err(format!(
                "{} has uncommitted changes; commit them or pass --force",
                dest.display()
            ));
        }
        let unpushed = git_in(
            &primary,
            &["rev-list", "--count", name, "--not", "--remotes"],
        )?;
        if unpushed.trim() != "0" && !pr_merged(&primary, name) {
            return Err(format!(
                "branch '{name}' has {} commit(s) on no remote; push them or pass --force",
                unpushed.trim()
            ));
        }
    }
    let dest_str = dest.display().to_string();
    let mut remove = vec!["worktree", "remove"];
    if force {
        remove.push("--force");
    }
    remove.push(&dest_str);
    git_in(&primary, &remove)?;
    git_in(&primary, &["branch", "-D", name])?;
    println!("removed {dest_str} and local branch {name}");
    Ok(())
}

/// A squash merge leaves the branch's own commits on no remote once GitHub deletes the head
/// branch, so a merged pull request also counts as pushed. `false` when `gh` cannot tell.
fn pr_merged(primary: &Path, name: &str) -> bool {
    Command::new("gh")
        .current_dir(primary)
        .args([
            "pr", "list", "--head", name, "--state", "merged", "--json", "number", "--jq", "length",
        ])
        .output()
        .is_ok_and(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).trim() != "0")
}

fn worktree_dir(primary: &Path, name: &str) -> PathBuf {
    primary.join(".claude/worktrees").join(name)
}

/// The primary checkout, from any worktree of it: the parent of the shared git directory.
fn primary_checkout() -> Result<PathBuf, String> {
    let common = git_in(
        Path::new("."),
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    Path::new(common.trim())
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| format!("unexpected git directory '{}'", common.trim()))
}

fn primary_or_cwd() -> PathBuf {
    primary_checkout().unwrap_or_else(|_| PathBuf::from("."))
}

fn git_in(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::valid_branch;

    #[test]
    fn a_branch_is_an_area_and_a_slug() {
        for ok in ["transit-collector", "packs-sync-profile", "gpui-wp27"] {
            assert!(valid_branch(ok), "{ok}");
        }
        for bad in [
            "transit",
            "Transit-x",
            "transit--x",
            "-x",
            "x-",
            "a_b-c",
            "claude/x-y",
        ] {
            assert!(!valid_branch(bad), "{bad}");
        }
    }
}
