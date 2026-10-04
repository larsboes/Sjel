//! Discover UI packages that the repository can type-check.
//!
//! `tools/discover-ui-packages` is a CI gate: a package that declares a check but lacks a
//! committed lockfile must fail closed, and a new UI must enter the gate without a hand-list.

use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const SKIP_DIRS: &[&str] = &[
    ".git",
    ".svelte-kit",
    "build",
    "dist",
    "node_modules",
    "target",
];
const LOCKFILES: &[&str] = &["bun.lock", "bun.lockb"];

#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    Checkable,
    NotUi,
    NoCheckScript,
    NoLockfile,
    Untracked,
}

impl Verdict {
    fn is_violation(&self) -> bool {
        matches!(
            self,
            Self::NoCheckScript | Self::NoLockfile | Self::Untracked
        )
    }
}

#[derive(Debug)]
struct UiPackage {
    dir: String,
    verdict: Verdict,
    reason: String,
}

fn package_dirs(root: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, found: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        let entries: Vec<_> = entries.filter_map(Result::ok).collect();
        if entries.iter().any(|entry| {
            entry.file_name() == "package.json"
                && entry.file_type().is_ok_and(|kind| kind.is_file())
        }) {
            found.push(dir.to_path_buf());
        }
        for entry in entries {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() && !SKIP_DIRS.contains(&entry.file_name().to_string_lossy().as_ref()) {
                walk(&entry.path(), found);
            }
        }
    }

    let mut found = Vec::new();
    walk(root, &mut found);
    found.sort();
    found
}

fn relative_dir(root: &Path, dir: &Path) -> String {
    dir.strip_prefix(root)
        .unwrap_or(dir)
        .to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, "/")
}

fn js_string(value: &Value) -> String {
    match value {
        Value::Null => "null".into(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => value.clone(),
        Value::Array(values) => values
            .iter()
            .map(|value| match value {
                Value::Null => String::new(),
                _ => js_string(value),
            })
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "[object Object]".into(),
    }
}

fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_f64().is_some_and(|n| n != 0.0),
        Value::String(value) => !value.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

fn is_tracked(root: &Path, rel_path: &str) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files", "--", rel_path])
        .output()
        .map(|output| {
            !output.status.success() || !String::from_utf8_lossy(&output.stdout).trim().is_empty()
        })
        .unwrap_or(true)
}

fn classify(root: &Path, dir: &Path) -> UiPackage {
    let rel = relative_dir(root, dir);
    let manifest_path = dir.join("package.json");
    let pkg: Value = match fs::read(&manifest_path)
        .map_err(|error| error.to_string())
        .and_then(|bytes| serde_json::from_slice(&bytes).map_err(|error| error.to_string()))
    {
        Ok(pkg) => pkg,
        Err(error) => {
            return UiPackage {
                dir: rel,
                verdict: Verdict::NoCheckScript,
                reason: format!("package.json does not parse ({error})"),
            };
        }
    };
    let object = pkg.as_object();
    let check = object
        .and_then(|pkg| pkg.get("scripts"))
        .and_then(Value::as_object)
        .and_then(|scripts| scripts.get("check"));
    let check = check.filter(|value| truthy(value));
    let is_svelte = ["dependencies", "devDependencies"].iter().any(|key| {
        object
            .and_then(|pkg| pkg.get(*key))
            .and_then(Value::as_object)
            .is_some_and(|dependencies| dependencies.contains_key("svelte"))
    }) || dir.join("svelte.config.js").exists();

    let Some(check) = check else {
        return if is_svelte {
            UiPackage {
                dir: rel,
                verdict: Verdict::NoCheckScript,
                reason: "declares Svelte but no `check` script, so nothing can type-check it"
                    .into(),
            }
        } else {
            UiPackage {
                dir: rel,
                verdict: Verdict::NotUi,
                reason: "no `check` script and no Svelte — nothing declared to check".into(),
            }
        };
    };

    let Some(lockfile) = LOCKFILES
        .iter()
        .find(|file| dir.join(file).exists())
        .copied()
    else {
        return UiPackage {
            dir: rel,
            verdict: Verdict::NoLockfile,
            reason: "declares a `check` script but commits no bun lockfile, so the install it needs is not reproducible".into(),
        };
    };

    for file in ["package.json", lockfile] {
        let tracked_path = Path::new(&rel).join(file);
        if !is_tracked(root, &tracked_path.to_string_lossy()) {
            return UiPackage {
                dir: rel,
                verdict: Verdict::Untracked,
                reason: format!("{file} is not in the index — this package exists on this machine and in no clone (local green, CI red)"),
            };
        }
    }

    UiPackage {
        dir: rel,
        verdict: Verdict::Checkable,
        reason: format!("`bun run check` → {}", js_string(check)),
    }
}

fn discover(root: &Path) -> Vec<UiPackage> {
    package_dirs(root)
        .iter()
        .map(|dir| classify(root, dir))
        .collect()
}

fn root() -> Result<PathBuf, String> {
    if let Some(root) = std::env::var_os("SJEL_UI_DISCOVERY_ROOT") {
        let root = PathBuf::from(root);
        return if root.is_absolute() {
            Ok(root)
        } else {
            std::env::current_dir()
                .map(|cwd| cwd.join(root))
                .map_err(|error| format!("cannot resolve working directory: {error}"))
        };
    }
    crate::paths::Paths::from_env().map(|paths| paths.root)
}

pub fn run(args: &[String]) -> ExitCode {
    let root = match root() {
        Ok(root) => root,
        Err(error) => {
            eprintln!("discover-ui-packages: {error}");
            return ExitCode::from(2);
        }
    };
    let packages = discover(&root);
    let checkable: Vec<_> = packages
        .iter()
        .filter(|package| package.verdict == Verdict::Checkable)
        .collect();
    let violations: Vec<_> = packages
        .iter()
        .filter(|package| package.verdict.is_violation())
        .collect();
    let dirs_only = args.iter().any(|arg| arg == "--dirs");

    if dirs_only {
        if violations.is_empty() {
            for package in &checkable {
                println!("{}", package.dir);
            }
        }
    } else {
        for package in &packages {
            let mark = match package.verdict {
                Verdict::Checkable => "ok  ",
                Verdict::NotUi => "skip",
                _ => "FAIL",
            };
            println!("{mark} {} — {}", package.dir, package.reason);
        }
    }

    if checkable.is_empty() {
        eprintln!("discover-ui-packages: no checkable UI package found — the dashboard alone should produce one, so discovery is broken rather than the repository empty");
        return ExitCode::FAILURE;
    }
    if !violations.is_empty() {
        let dirs = violations
            .iter()
            .map(|package| package.dir.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        eprintln!(
            "discover-ui-packages: {} package(s) declare a UI this gate cannot check: {dirs}",
            violations.len()
        );
        return ExitCode::FAILURE;
    }
    if !dirs_only {
        println!(
            "discover-ui-packages: {} checkable package(s), {} skipped.",
            checkable.len(),
            packages.len() - checkable.len()
        );
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "sjel-ui-packages-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&root).unwrap();
            Self(root)
        }

        fn plant(&self, dir: &str, options: Plant) -> UiPackage {
            let path = self.0.join(dir);
            fs::create_dir_all(&path).unwrap();
            let name = dir.replace('/', "-");
            let package = serde_json::json!({
                "name": name,
                "scripts": options.check.map(|check| serde_json::json!({"check":check})).unwrap_or_default(),
                "dependencies": options.dependencies,
            });
            fs::write(path.join("package.json"), package.to_string()).unwrap();
            if let Some(lockfile) = options.lockfile {
                fs::write(path.join(lockfile), "{}\n").unwrap();
            }
            if options.svelte_config {
                fs::write(path.join("svelte.config.js"), "export default {};\n").unwrap();
            }
            classify(&self.0, &path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[derive(Default)]
    struct Plant {
        check: Option<&'static str>,
        dependencies: serde_json::Map<String, Value>,
        lockfile: Option<&'static str>,
        svelte_config: bool,
    }

    impl Plant {
        fn checkable() -> Self {
            Self {
                check: Some("svelte-check --tsconfig ./tsconfig.json"),
                dependencies: serde_json::Map::from_iter([(
                    "svelte".into(),
                    Value::String("^5.0.0".into()),
                )]),
                lockfile: Some("bun.lock"),
                ..Self::default()
            }
        }
    }

    #[test]
    fn check_script_and_lockfile_are_checkable() {
        let scratch = Scratch::new();
        let package = scratch.plant("dashboard", Plant::checkable());
        assert_eq!(package.verdict, Verdict::Checkable);
        assert!(package.reason.contains("svelte-check"));
    }

    #[test]
    fn svelte_without_a_check_script_is_a_violation() {
        let scratch = Scratch::new();
        let package = scratch.plant(
            "capabilities/mute/ui",
            Plant {
                dependencies: serde_json::Map::from_iter([(
                    "svelte".into(),
                    Value::String("^5.0.0".into()),
                )]),
                ..Plant::default()
            },
        );
        assert_eq!(package.verdict, Verdict::NoCheckScript);
    }

    #[test]
    fn svelte_config_alone_marks_a_ui() {
        let scratch = Scratch::new();
        let package = scratch.plant(
            "capabilities/adapter-only/ui",
            Plant {
                svelte_config: true,
                ..Plant::default()
            },
        );
        assert_eq!(package.verdict, Verdict::NoCheckScript);
    }

    #[test]
    fn check_script_without_lockfile_is_a_violation() {
        let scratch = Scratch::new();
        let package = scratch.plant(
            "capabilities/loose/ui",
            Plant {
                check: Some("tsc --noEmit"),
                ..Plant::default()
            },
        );
        assert_eq!(package.verdict, Verdict::NoLockfile);
    }

    #[test]
    fn bun_lockb_is_a_reproducible_lockfile() {
        let scratch = Scratch::new();
        let package = scratch.plant(
            "capabilities/legacy/ui",
            Plant {
                check: Some("tsc --noEmit"),
                lockfile: Some("bun.lockb"),
                ..Plant::default()
            },
        );
        assert_eq!(package.verdict, Verdict::Checkable);
    }

    #[test]
    fn non_ui_without_a_check_is_skipped() {
        let scratch = Scratch::new();
        let package = scratch.plant("tools/fixture", Plant::default());
        assert_eq!(package.verdict, Verdict::NotUi);
        assert!(!package.verdict.is_violation());
    }

    #[test]
    fn non_svelte_package_with_a_check_is_still_checked() {
        let scratch = Scratch::new();
        let package = scratch.plant(
            "capabilities/plain/ui",
            Plant {
                check: Some("tsc --noEmit"),
                lockfile: Some("bun.lock"),
                ..Plant::default()
            },
        );
        assert_eq!(package.verdict, Verdict::Checkable);
    }

    #[test]
    fn generated_trees_and_installed_dependencies_are_skipped() {
        let scratch = Scratch::new();
        scratch.plant("dashboard", Plant::checkable());
        scratch.plant("dashboard/node_modules/svelte", Plant::checkable());
        scratch.plant("dashboard/.svelte-kit/output", Plant::checkable());
        scratch.plant("dashboard/dist/vendor", Plant::checkable());
        assert_eq!(
            discover(&scratch.0)
                .into_iter()
                .map(|package| package.dir)
                .collect::<Vec<_>>(),
            ["dashboard"]
        );
    }

    #[test]
    fn malformed_package_json_is_a_violation() {
        let scratch = Scratch::new();
        let path = scratch.0.join("capabilities/broken/ui");
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("package.json"), "{ not json").unwrap();
        assert!(classify(&scratch.0, &path).verdict.is_violation());
    }

    #[test]
    fn a_root_package_uses_repository_relative_paths() {
        let scratch = Scratch::new();
        let package = scratch.plant("", Plant::checkable());
        assert!(Command::new("git")
            .args(["-C", &scratch.0.to_string_lossy(), "init", "-q"])
            .status()
            .unwrap()
            .success());
        assert!(Command::new("git")
            .args(["-C", &scratch.0.to_string_lossy(), "add", "-A"])
            .status()
            .unwrap()
            .success());
        assert_eq!(classify(&scratch.0, &scratch.0).verdict, Verdict::Checkable);
        assert!(package.dir.is_empty());
    }

    #[test]
    fn an_ignored_lockfile_is_untracked() {
        let scratch = Scratch::new();
        scratch.plant("capabilities/ignored/ui", Plant::checkable());
        fs::write(scratch.0.join(".gitignore"), "bun.lock\n").unwrap();
        assert!(Command::new("git")
            .args(["-C", &scratch.0.to_string_lossy(), "init", "-q"])
            .status()
            .unwrap()
            .success());
        assert!(Command::new("git")
            .args(["-C", &scratch.0.to_string_lossy(), "add", "-A"])
            .status()
            .unwrap()
            .success());
        let package = discover(&scratch.0)
            .into_iter()
            .find(|package| package.dir == "capabilities/ignored/ui")
            .unwrap();
        assert_eq!(package.verdict, Verdict::Untracked);
    }

    #[test]
    fn a_new_ui_is_discovered_without_a_registry_edit() {
        let scratch = Scratch::new();
        scratch.plant("dashboard", Plant::checkable());
        let before = discover(&scratch.0)
            .into_iter()
            .filter(|package| package.verdict == Verdict::Checkable)
            .count();
        scratch.plant("capabilities/invented-tomorrow/ui", Plant::checkable());
        let after: Vec<_> = discover(&scratch.0)
            .into_iter()
            .filter(|package| package.verdict == Verdict::Checkable)
            .map(|package| package.dir)
            .collect();
        assert_eq!(after.len(), before + 1);
        assert!(after.contains(&"capabilities/invented-tomorrow/ui".into()));
    }
}
