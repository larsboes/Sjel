//! Apply and check Sjel's user-level GitHub Copilot CLI settings and guard hook.
//!
//! Copilot keeps durable settings in `$COPILOT_HOME/settings.json`. The public baseline owns
//! safe defaults; an overlay fragment may add machine-local paths or credential host rules without
//! putting those values in the repository.
//!
//! Copilot has no persistent tool-deny setting — `--deny-tool` is a session flag — so the blocks
//! on `rm`, `git commit` and `git push` live in a `preToolUse` hook under `$COPILOT_HOME/hooks/`.
//! Copilot loads those files for every session, and a command `preToolUse` hook is fail-closed: a
//! crash or a non-zero exit denies the tool call. Both the settings and the hook are checked in
//! one pass, because a security floor that is only checked where it is written is not checked.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde_json::{Map, Value};

const EXIT_USAGE: u8 = 1;
const EXIT_INVALID: u8 = 2;
const EXIT_DRIFT: u8 = 3;

const USAGE: &str = "
sjel copilot [apply|check] [--force] [--dry-run]

  apply       merge Sjel's Copilot security baseline into settings.json, and write the
              preToolUse guard under hooks/
  check       report drift from the settings baseline and the guard, in one pass
  --force     overwrite the settings values Sjel owns; the guard is rewritten either way
  --dry-run   report changes without writing

Target: $COPILOT_HOME, else ~/.copilot.
Baseline: tools/templates/copilot/settings.base.json, optionally overlaid with
$SJEL_OVERLAY_ROOT/config/copilot/settings.fragment.json.
Guard: tools/templates/copilot/hooks/sjel-guard.sh, installed as hooks/sjel-guard.sh and
registered from hooks/sjel-guard.json.

Copilot persists no tool-deny setting, so `rm`, `git commit` and `git push` are blocked by a
command preToolUse hook instead of a launch flag. It loads for every Copilot session, not only
sessions started through tools/copilot. A session that sets disableAllHooks, or a user who
deletes hooks/sjel-guard.json, turns it off; the non-bypassable layer is a root-owned hook in
/etc/github-copilot/policy.d/, which `check` cannot see.
";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verb {
    Apply,
    Check,
}

#[derive(Debug)]
struct Opts {
    verb: Verb,
    force: bool,
    dry_run: bool,
}

pub fn run(root: &Path, args: &[String]) -> ExitCode {
    if args.iter().any(|arg| arg == "-h" || arg == "--help") {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    let opts = match parse(args) {
        Ok(opts) => opts,
        Err(error) => {
            eprintln!("sjel copilot: {error}");
            eprint!("{USAGE}");
            return ExitCode::from(EXIT_USAGE);
        }
    };
    let baseline = match baseline(root) {
        Ok(value) => value,
        Err(error) => return report(error, EXIT_USAGE),
    };
    let target = match settings_path() {
        Ok(path) => path,
        Err(error) => return report(error, EXIT_USAGE),
    };
    let existing = match read_object_if_present(&target) {
        Ok(value) => value,
        Err(error) => return report(error, EXIT_INVALID),
    };
    let settings_drift = drift_paths(&existing, &baseline);

    let artifacts = match guard_artifacts(root) {
        Ok(artifacts) => artifacts,
        Err(error) => return report(error, EXIT_USAGE),
    };
    let mut guard_drift = Vec::new();
    let mut drifted = Vec::new();
    for artifact in &artifacts {
        match artifact.current() {
            Ok(true) => {}
            Ok(false) => {
                guard_drift.push(artifact.label.clone());
                drifted.push(artifact);
            }
            Err(error) => return report(error, EXIT_INVALID),
        }
    }
    let hooks_dir = artifacts
        .first()
        .and_then(|artifact| artifact.path.parent())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("hooks"));

    if opts.verb == Verb::Check {
        if settings_drift.is_empty() {
            println!("Copilot settings are current: {}", target.display());
        } else {
            println!("Copilot settings drifted: {}", target.display());
            for path in &settings_drift {
                println!("  {path}");
            }
        }
        if guard_drift.is_empty() {
            println!("Copilot guard is current: {}", hooks_dir.display());
        } else {
            println!("Copilot guard drifted: {}", hooks_dir.display());
            for label in &guard_drift {
                println!("  {label}");
            }
        }
        return if settings_drift.is_empty() && guard_drift.is_empty() {
            ExitCode::SUCCESS
        } else {
            ExitCode::from(EXIT_DRIFT)
        };
    }

    let mut changed = false;
    if !settings_drift.is_empty() {
        if opts.dry_run {
            println!("Would update {}", target.display());
            changed = true;
        } else {
            let merged = if opts.force {
                merge_over(existing, baseline)
            } else {
                merge_missing(existing, &baseline)
            };
            if let Err(error) = write_json(&target, &merged) {
                return report(error, EXIT_USAGE);
            }
            println!("Updated {}", target.display());
            changed = true;
        }
    }
    for artifact in drifted {
        if opts.dry_run {
            println!("Would write {}", artifact.path.display());
            changed = true;
            continue;
        }
        if let Err(error) = artifact.write() {
            return report(error, EXIT_USAGE);
        }
        println!("Wrote {}", artifact.path.display());
        changed = true;
    }
    if !changed {
        println!("Copilot settings and guard are already current: {}", target.display());
    }
    ExitCode::SUCCESS
}

fn report(error: String, code: u8) -> ExitCode {
    eprintln!("sjel copilot: {error}");
    ExitCode::from(code)
}

fn parse(args: &[String]) -> Result<Opts, String> {
    let mut opts = Opts {
        verb: Verb::Apply,
        force: false,
        dry_run: false,
    };
    for arg in args {
        match arg.as_str() {
            "apply" => opts.verb = Verb::Apply,
            "check" => opts.verb = Verb::Check,
            "--force" | "-f" => opts.force = true,
            "--dry-run" | "-n" => opts.dry_run = true,
            other => return Err(format!("unknown argument '{other}'")),
        }
    }
    Ok(opts)
}

fn copilot_home() -> Result<PathBuf, String> {
    let home = env::var_os("HOME").ok_or("HOME is not set")?;
    Ok(env::var_os("COPILOT_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(home).join(".copilot")))
}

fn settings_path() -> Result<PathBuf, String> {
    Ok(copilot_home()?.join("settings.json"))
}

/// One file Sjel owns outright at a Copilot destination. `current` compares bytes rather than a
/// stored digest: the file is small, rendered deterministically from the template, and a digest
/// recorded at write time cannot notice a file edited afterwards by a session running as the
/// owner — which is the failure `check` exists to report.
struct Artifact {
    path: PathBuf,
    label: String,
    bytes: Vec<u8>,
    executable: bool,
}

impl Artifact {
    fn current(&self) -> Result<bool, String> {
        match fs::read(&self.path) {
            Ok(existing) => Ok(existing == self.bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(format!("cannot read {}: {error}", self.path.display())),
        }
    }

    fn write(&self) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
        }
        fs::write(&self.path, &self.bytes)
            .map_err(|error| format!("{}: {error}", self.path.display()))?;
        if self.executable {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&self.path, fs::Permissions::from_mode(0o755))
                .map_err(|error| format!("{}: {error}", self.path.display()))?;
        }
        Ok(())
    }
}

fn guard_artifacts(root: &Path) -> Result<Vec<Artifact>, String> {
    let hooks = copilot_home()?.join("hooks");
    let script_path = hooks.join("sjel-guard.sh");
    let template = root.join("tools/templates/copilot/hooks/sjel-guard.sh");
    let script = fs::read(&template)
        .map_err(|error| format!("cannot read {}: {error}", template.display()))?;
    let config = serde_json::json!({
        "version": 1,
        "hooks": {
            "preToolUse": [{
                "type": "command",
                "exec": script_path.display().to_string(),
                "timeoutSec": 10,
            }]
        }
    });
    let mut config = serde_json::to_string_pretty(&config)
        .map_err(|error| error.to_string())?
        .into_bytes();
    config.push(b'\n');
    Ok(vec![
        Artifact {
            path: script_path,
            label: "hooks/sjel-guard.sh".to_owned(),
            bytes: script,
            executable: true,
        },
        Artifact {
            path: hooks.join("sjel-guard.json"),
            label: "hooks/sjel-guard.json".to_owned(),
            bytes: config,
            executable: false,
        },
    ])
}

fn baseline(root: &Path) -> Result<Value, String> {
    let base_path = root.join("tools/templates/copilot/settings.base.json");
    let mut value = read_json(&base_path)?;
    if let Some(overlay) =
        env::var_os("SJEL_OVERLAY_ROOT").or_else(|| env::var_os("SJEL_PERSONAL_ROOT"))
    {
        let fragment = PathBuf::from(overlay).join("config/copilot/settings.fragment.json");
        if fragment.exists() {
            value = merge_over(value, read_json(&fragment)?);
        }
    }
    Ok(value)
}

fn read_json(path: &Path) -> Result<Value, String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    serde_json::from_str(&text)
        .map_err(|error| format!("invalid JSON at {}: {error}", path.display()))
}

fn read_object_if_present(path: &Path) -> Result<Value, String> {
    if !path.exists() {
        return Ok(Value::Object(Map::new()));
    }
    let value = read_json(path)?;
    if !value.is_object() {
        return Err(format!("{} must contain a JSON object", path.display()));
    }
    Ok(value)
}

fn merge_missing(mut existing: Value, baseline: &Value) -> Value {
    merge(&mut existing, baseline, false);
    existing
}

fn merge_over(mut existing: Value, baseline: Value) -> Value {
    merge(&mut existing, &baseline, true);
    existing
}

fn merge(target: &mut Value, source: &Value, overwrite: bool) {
    let (Some(target), Some(source)) = (target.as_object_mut(), source.as_object()) else {
        if overwrite {
            *target = source.clone();
        }
        return;
    };
    for (key, value) in source {
        match target.get_mut(key) {
            Some(existing) if existing.is_object() && value.is_object() => {
                merge(existing, value, overwrite);
            }
            Some(existing) if overwrite => *existing = value.clone(),
            None => {
                target.insert(key.clone(), value.clone());
            }
            _ => {}
        }
    }
}

fn drift_paths(existing: &Value, baseline: &Value) -> Vec<String> {
    let mut paths = Vec::new();
    collect_drift(existing, baseline, "", &mut paths);
    paths
}

fn collect_drift(existing: &Value, baseline: &Value, prefix: &str, paths: &mut Vec<String>) {
    let Some(base) = baseline.as_object() else {
        if existing != baseline {
            paths.push(prefix.to_owned());
        }
        return;
    };
    let existing = existing.as_object();
    for (key, value) in base {
        let path = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        match existing.and_then(|object| object.get(key)) {
            Some(actual) if value.is_object() && actual.is_object() => {
                collect_drift(actual, value, &path, paths)
            }
            Some(actual) if actual == value => {}
            _ => paths.push(path),
        }
    }
}

fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
    }
    let text = serde_json::to_string_pretty(value).map_err(|error| error.to_string())?;
    fs::write(path, format!("{text}\n")).map_err(|error| format!("{}: {error}", path.display()))
}
