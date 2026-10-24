//! Reversible ownership of selected Pi settings, with drift checks and recoverable writes.

use std::collections::BTreeMap;
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub const USAGE: &str = "sjel pi [plan|apply|check|rollback] [--dry-run] [--keybindings]\n\
  plan       show owned setting keys that would change (default)\n\
  apply      snapshot selected settings and apply the reviewed baseline\n\
  check      report owned setting drift without writing\n\
  rollback   restore the original values of settings Sjel owns\n\
  --dry-run  plan apply or rollback without writing\n\n\
  --keybindings  manage keybindings.json with its own ledger and transaction\n\n\
Target: $PI_CODING_AGENT_DIR/settings.json, else ~/.pi/agent/settings.json.\n\
Baseline: tools/templates/pi/settings.base.json; optional overlay config/pi/settings.fragment.json.\n\
Credentials, model registries and resource arrays are never managed.\n";

const MAX_JSON_BYTES: u64 = 4 * 1024 * 1024;
static TEMP_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Apply,
    Rollback,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Plan,
    Mutate,
    Check,
}

#[derive(Debug)]
struct Options {
    action: Action,
    mode: Mode,
    kind: Kind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Settings,
    Keybindings,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Self::Settings => "settings",
            Self::Keybindings => "keybindings",
        }
    }
    fn validate(self, key: &str, value: &Value) -> Result<(), String> {
        match self {
            Self::Settings => validate_setting(key, value),
            Self::Keybindings => validate_binding(key, value),
        }
    }
    fn owns(self, key: &str) -> bool {
        match self {
            Self::Settings => setting_key(key),
            Self::Keybindings => binding_action(key),
        }
    }
}

#[derive(Clone, Debug)]
struct Paths {
    kind: Kind,
    settings: PathBuf,
    ledger: PathBuf,
    lock: PathBuf,
}

impl Paths {
    fn new(settings: PathBuf, kind: Kind) -> Result<Self, String> {
        let name = settings
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or("settings path has no valid filename")?;
        let stem = name.strip_suffix(".json").unwrap_or(name);
        Ok(Self {
            kind,
            ledger: settings.with_file_name(format!("{stem}.sjel-managed.json")),
            lock: settings.with_file_name(format!("{name}.sjel.lock")),
            settings,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "presence", content = "value", rename_all = "snake_case")]
enum Slot {
    Absent,
    Value(Value),
}

impl Slot {
    fn read(settings: &Map<String, Value>, key: &str) -> Self {
        settings
            .get(key)
            .map_or(Self::Absent, |v| Self::Value(v.clone()))
    }

    fn write(&self, settings: &mut Map<String, Value>, key: &str) {
        match self {
            Self::Absent => {
                settings.remove(key);
            }
            Self::Value(value) => {
                settings.insert(key.to_owned(), value.clone());
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Owned {
    original: Slot,
    last_applied: Slot,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pending {
    before: BTreeMap<String, Slot>,
    after: BTreeMap<String, Slot>,
    next_entries: BTreeMap<String, Owned>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Ledger {
    version: u8,
    entries: BTreeMap<String, Owned>,
    pending: Option<Pending>,
}

impl Default for Ledger {
    fn default() -> Self {
        Self {
            version: 1,
            entries: BTreeMap::new(),
            pending: None,
        }
    }
}

#[derive(Debug)]
struct Prepared {
    current: Map<String, Value>,
    settings: Map<String, Value>,
    ledger: Ledger,
    before: BTreeMap<String, Slot>,
    after: BTreeMap<String, Slot>,
    changed: Vec<String>,
    conflict: Vec<String>,
}

/// Operate on selected Pi settings without reading credential or model-registry files.
///
/// # Errors
/// Returns a nonzero exit code for invalid state, drift or conflicting owned settings.
pub fn run(root: &Path, args: &[String]) -> ExitCode {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    match execute(root, args) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("sjel pi: {error}");
            ExitCode::from(2)
        }
    }
}

fn execute(root: &Path, args: &[String]) -> Result<ExitCode, String> {
    let opts = parse(args)?;
    let paths = resolve_paths(opts.kind)?;
    let desired = if opts.action == Action::Apply {
        baseline(root, opts.kind)?
    } else {
        Map::new()
    };
    let _lock = if opts.mode == Mode::Mutate {
        Some(lock(&paths)?)
    } else {
        None
    };
    let current = read_settings(&paths.settings)?;
    let mut ledger = read_ledger(&paths.ledger, paths.kind)?;
    let recovered = reconcile(&current, &mut ledger)?;
    let prepared = prepare(current, ledger.clone(), &desired, opts.action);
    println!("Pi {}: {}", opts.kind.name(), paths.settings.display());
    if recovered {
        println!("  interrupted transaction: reconciled in the plan");
    }
    for key in &prepared.conflict {
        println!("  conflict: {key}");
    }
    if !prepared.conflict.is_empty() {
        return Ok(ExitCode::from(3));
    }
    for key in &prepared.changed {
        println!(
            "  {}: {key}",
            if opts.action == Action::Rollback {
                "restore"
            } else {
                "manage"
            }
        );
    }
    if opts.mode == Mode::Check {
        return Ok(if prepared.changed.is_empty() && !recovered {
            ExitCode::SUCCESS
        } else {
            ExitCode::from(3)
        });
    }
    if opts.mode == Mode::Mutate {
        if recovered {
            atomic_json(&paths.ledger, &ledger)?;
        }
        persist(&paths, &prepared, None)?;
        println!("  completed; settings values were not printed");
    } else {
        println!("  plan only; no files written");
    }
    Ok(ExitCode::SUCCESS)
}

fn parse(args: &[String]) -> Result<Options, String> {
    let mut verb = None;
    let mut dry_run = false;
    let mut kind = Kind::Settings;
    for arg in args {
        match arg.as_str() {
            "--dry-run" | "-n" => dry_run = true,
            "--keybindings" => kind = Kind::Keybindings,
            "plan" | "apply" | "check" | "rollback" if verb.is_none() => {
                verb = Some(arg.as_str());
            }
            _ => {
                return Err(
                    "expected one of plan, apply, check or rollback and optional --dry-run".into(),
                )
            }
        }
    }
    Ok(Options {
        kind,
        action: if verb == Some("rollback") {
            Action::Rollback
        } else {
            Action::Apply
        },
        mode: match verb.unwrap_or("plan") {
            "check" => Mode::Check,
            "apply" | "rollback" if !dry_run => Mode::Mutate,
            _ => Mode::Plan,
        },
    })
}

fn resolve_paths(kind: Kind) -> Result<Paths, String> {
    if env::var_os("PI_SETTINGS_FILE").is_some() {
        return Err("PI_SETTINGS_FILE is not supported by Pi; use PI_CODING_AGENT_DIR".into());
    }
    let dir = env::var_os("PI_CODING_AGENT_DIR").map_or_else(
        || {
            env::var_os("HOME")
                .map(|home| PathBuf::from(home).join(".pi/agent"))
                .ok_or("HOME is not set")
        },
        |dir| Ok(PathBuf::from(dir)),
    )?;
    Paths::new(dir.join(format!("{}.json", kind.name())), kind)
}

fn baseline(root: &Path, kind: Kind) -> Result<Map<String, Value>, String> {
    let base = root.join(format!("tools/templates/pi/{}.base.json", kind.name()));
    if !base.is_file() {
        return Err(format!("missing Pi baseline at {}", base.display()));
    }
    let mut desired = read_settings(&base)?;
    let overlay = env::var_os("SJEL_OVERLAY_ROOT")
        .map(PathBuf::from)
        .or_else(sjel_config::overlay_root);
    if let Some(fragment) =
        overlay.map(|r| r.join(format!("config/pi/{}.fragment.json", kind.name())))
    {
        for (key, value) in read_settings(&fragment)? {
            if let Some(existing) = desired.get_mut(&key) {
                merge(existing, &value);
            } else {
                desired.insert(key, value);
            }
        }
    }
    for (key, value) in &desired {
        kind.validate(key, value)?;
    }
    Ok(desired)
}

fn merge(target: &mut Value, incoming: &Value) {
    if let (Some(target), Some(incoming)) = (target.as_object_mut(), incoming.as_object()) {
        for (key, value) in incoming {
            if let Some(existing) = target.get_mut(key) {
                merge(existing, value);
            } else {
                target.insert(key.clone(), value.clone());
            }
        }
    } else {
        *target = incoming.clone();
    }
}

fn validate_setting(key: &str, value: &Value) -> Result<(), String> {
    let valid = match key {
        "enableInstallTelemetry"
        | "enableAnalytics"
        | "hideThinkingBlock"
        | "showCacheMissNotices"
        | "showHardwareCursor"
        | "fullscreenCopyOnSelect" => value.is_boolean(),
        "defaultThinkingLevel" => value.as_str().is_some_and(|v| {
            ["off", "minimal", "low", "medium", "high", "xhigh", "max"].contains(&v)
        }),
        "defaultProvider" | "defaultModel" | "theme" => {
            value.as_str().is_some_and(|v| !v.is_empty())
        }
        "quietStartup" => value.is_boolean() || value.as_str() == Some("header"),
        "tuiMode" => value
            .as_str()
            .is_some_and(|v| ["regular", "fullscreen"].contains(&v)),
        "fullscreenExitOutput" => value
            .as_str()
            .is_some_and(|v| ["transcript", "resume-hint"].contains(&v)),
        "cacheWarming" => value
            .as_str()
            .is_some_and(|v| ["off", "streaming", "idle"].contains(&v)),
        "defaultProjectTrust" => value
            .as_str()
            .is_some_and(|v| ["ask", "never", "always"].contains(&v)),
        "editorPaddingX" => value.as_u64().is_some_and(|v| v <= 3),
        "autocompleteMaxVisible" => value.as_u64().is_some_and(|v| (3..=20).contains(&v)),
        "outputPad" => value.as_u64().is_some_and(|v| v <= 1),
        "modelThinkingLevels" => value.as_object().is_some_and(|m| {
            m.iter()
                .all(|(id, v)| model_id(id) && validate_setting("defaultThinkingLevel", v).is_ok())
        }),
        "compaction" => compaction(value, true),
        _ => return Err(format!("unsupported managed setting key: {key}")),
    };
    if valid {
        Ok(())
    } else {
        Err(format!("invalid managed setting type or range: {key}"))
    }
}

fn setting_key(key: &str) -> bool {
    matches!(
        key,
        "enableInstallTelemetry"
            | "enableAnalytics"
            | "hideThinkingBlock"
            | "showCacheMissNotices"
            | "showHardwareCursor"
            | "fullscreenCopyOnSelect"
            | "defaultThinkingLevel"
            | "defaultProvider"
            | "defaultModel"
            | "theme"
            | "quietStartup"
            | "tuiMode"
            | "fullscreenExitOutput"
            | "cacheWarming"
            | "defaultProjectTrust"
            | "editorPaddingX"
            | "autocompleteMaxVisible"
            | "outputPad"
            | "modelThinkingLevels"
            | "compaction"
    )
}

fn model_id(id: &str) -> bool {
    id.split_once('/')
        .is_some_and(|(provider, model)| !provider.is_empty() && !model.is_empty())
        && !id.chars().any(char::is_control)
}

fn compaction(value: &Value, model_overrides: bool) -> bool {
    value.as_object().is_some_and(|m| {
        m.iter().all(|(key, value)| match key.as_str() {
            "enabled" if model_overrides => value.is_boolean(),
            "reserveTokens" | "keepRecentTokens" => {
                value.as_u64().is_some_and(|n| n <= 9_007_199_254_740_991)
            }
            "modelOverrides" if model_overrides => value.as_object().is_some_and(|models| {
                models
                    .iter()
                    .all(|(id, v)| model_id(id) && compaction(v, false))
            }),
            _ => false,
        })
    })
}

fn binding_action(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 160
        && key.contains('.')
        && key
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
        && !key.starts_with('.')
        && !key.ends_with('.')
        && !key.contains("..")
}

fn validate_binding(key: &str, value: &Value) -> Result<(), String> {
    let chord = |value: &Value| {
        value
            .as_str()
            .is_some_and(|s| !s.is_empty() && s.len() <= 80 && !s.chars().any(char::is_control))
    };
    let valid = chord(value)
        || value
            .as_array()
            .is_some_and(|a| a.len() <= 32 && a.iter().all(chord));
    if binding_action(key) && valid {
        Ok(())
    } else {
        Err("invalid keybinding action or value; expected an action ID and key string/list".into())
    }
}

fn read_settings(path: &Path) -> Result<Map<String, Value>, String> {
    let Some(bytes) = read_optional(path)? else {
        return Ok(Map::new());
    };
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|_| format!("invalid JSON at {}", path.display()))?;
    value
        .as_object()
        .cloned()
        .ok_or_else(|| format!("expected JSON object at {}", path.display()))
}

fn read_ledger(path: &Path, kind: Kind) -> Result<Ledger, String> {
    let Some(bytes) = read_optional(path)? else {
        return Ok(Ledger::default());
    };
    let ledger: Ledger = serde_json::from_slice(&bytes)
        .map_err(|_| format!("invalid managed ledger at {}", path.display()))?;
    if ledger.version != 1 || ledger.entries.keys().any(|k| !kind.owns(k)) {
        return Err("unsupported managed ledger version or owned key".into());
    }
    if let Some(pending) = &ledger.pending {
        if pending.before.keys().ne(pending.after.keys())
            || pending
                .before
                .keys()
                .chain(pending.next_entries.keys())
                .any(|k| !kind.owns(k))
        {
            return Err("invalid managed ledger transaction".into());
        }
    }
    Ok(ledger)
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("cannot inspect {}: {e}", path.display())),
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > MAX_JSON_BYTES {
        return Err(format!(
            "expected a bounded regular file at {}",
            path.display()
        ));
    }
    fs::read(path)
        .map(Some)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))
}

fn reconcile(settings: &Map<String, Value>, ledger: &mut Ledger) -> Result<bool, String> {
    let Some(pending) = &ledger.pending else {
        return Ok(false);
    };
    let matches = |slots: &BTreeMap<String, Slot>| {
        slots
            .iter()
            .all(|(key, value)| Slot::read(settings, key) == *value)
    };
    if matches(&pending.after) {
        ledger.entries = pending.next_entries.clone();
    } else if !matches(&pending.before) {
        return Err(
            "interrupted transaction conflicts with current owned settings; nothing was written"
                .into(),
        );
    }
    ledger.pending = None;
    Ok(true)
}

fn prepare(
    current: Map<String, Value>,
    mut ledger: Ledger,
    desired: &Map<String, Value>,
    action: Action,
) -> Prepared {
    let conflict = ledger
        .entries
        .iter()
        .filter(|(key, entry)| Slot::read(&current, key) != entry.last_applied)
        .map(|(key, _)| key.clone())
        .collect();
    let mut settings = current.clone();
    let mut before = BTreeMap::new();
    let mut after = BTreeMap::new();
    let mut changed = Vec::new();
    let keys: std::collections::BTreeSet<_> = ledger
        .entries
        .keys()
        .chain(desired.keys())
        .cloned()
        .collect();
    for key in keys {
        let prior = Slot::read(&current, &key);
        let old_entry = ledger.entries.get(&key).cloned();
        let next = match (action, desired.get(&key)) {
            (Action::Apply, Some(value)) => {
                let mut merged = current.get(&key).cloned().unwrap_or(Value::Null);
                merge(&mut merged, value);
                let next = Slot::Value(merged);
                ledger.entries.insert(
                    key.clone(),
                    Owned {
                        original: old_entry
                            .as_ref()
                            .map_or_else(|| prior.clone(), |entry| entry.original.clone()),
                        last_applied: next.clone(),
                    },
                );
                next
            }
            _ => {
                ledger.entries.remove(&key);
                old_entry
                    .as_ref()
                    .map_or_else(|| prior.clone(), |entry| entry.original.clone())
            }
        };
        if prior != next || old_entry != ledger.entries.get(&key).cloned() {
            changed.push(key.clone());
            before.insert(key.clone(), prior);
            after.insert(key.clone(), next.clone());
        }
        next.write(&mut settings, &key);
    }
    Prepared {
        current,
        settings,
        ledger,
        before,
        after,
        changed,
        conflict,
    }
}

fn lock(paths: &Paths) -> Result<File, String> {
    let parent = paths
        .settings
        .parent()
        .ok_or("settings path has no parent")?;
    fs::create_dir_all(parent).map_err(|e| format!("cannot create settings directory: {e}"))?;
    if fs::symlink_metadata(&paths.lock).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err("managed settings lock must not be a symlink".into());
    }
    let file = secure_options()
        .read(true)
        .write(true)
        .create(true)
        .open(&paths.lock)
        .map_err(|e| format!("cannot open managed settings lock: {e}"))?;
    file.try_lock()
        .map_err(|_| "another settings operation holds the managed lock".to_owned())?;
    Ok(file)
}

fn secure_options() -> OpenOptions {
    let mut opts = OpenOptions::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    opts
}

fn atomic_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let mut bytes =
        serde_json::to_vec_pretty(value).map_err(|_| "cannot serialize settings state")?;
    bytes.push(b'\n');
    let parent = path
        .parent()
        .ok_or("settings state has no parent directory")?;
    let name = path
        .file_name()
        .ok_or("settings state has no filename")?
        .to_string_lossy();
    let temp = parent.join(format!(
        ".{name}.sjel-{}-{}.tmp",
        std::process::id(),
        TEMP_ID.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut file = secure_options()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|e| format!("cannot stage settings state: {e}"))?;
        file.write_all(&bytes)
            .and_then(|()| file.sync_all())
            .map_err(|e| format!("cannot flush settings state: {e}"))?;
        fs::rename(&temp, path).map_err(|e| format!("cannot replace settings state: {e}"))?;
        File::open(parent)
            .and_then(|f| f.sync_all())
            .map_err(|e| format!("cannot flush settings directory: {e}"))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

fn persist(paths: &Paths, prepared: &Prepared, fail_after: Option<u8>) -> Result<(), String> {
    if prepared.changed.is_empty() {
        return Ok(());
    }
    if !prepared.conflict.is_empty() {
        return Err("owned settings conflict; nothing was written".into());
    }
    if read_settings(&paths.settings)? != prepared.current {
        return Err("settings changed during planning; nothing was written".into());
    }
    let old = read_ledger(&paths.ledger, paths.kind)?;
    let journal = Ledger {
        pending: Some(Pending {
            before: prepared.before.clone(),
            after: prepared.after.clone(),
            next_entries: prepared.ledger.entries.clone(),
        }),
        ..old
    };
    atomic_json(&paths.ledger, &journal)?;
    if fail_after == Some(1) {
        return Err("injected interruption after journal".into());
    }
    atomic_json(&paths.settings, &prepared.settings)?;
    if fail_after == Some(2) {
        return Err("injected interruption after settings".into());
    }
    atomic_json(&paths.ledger, &prepared.ledger)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct Scratch {
        root: PathBuf,
        paths: Paths,
    }

    impl Scratch {
        fn new(kind: Kind) -> Self {
            let id = TEMP_ID.fetch_add(1, Ordering::Relaxed);
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = env::temp_dir().join(format!(
                "sjel-pi-settings-{}-{nanos}-{id}",
                std::process::id()
            ));
            fs::create_dir(&root).unwrap();
            let paths = Paths::new(root.join(format!("{}.json", kind.name())), kind).unwrap();
            Self { root, paths }
        }

        fn settings(&self, value: Value) {
            atomic_json(&self.paths.settings, &value).unwrap();
        }

        fn prepared(&self, desired: Value, action: Action) -> Prepared {
            let current = read_settings(&self.paths.settings).unwrap();
            let mut ledger = read_ledger(&self.paths.ledger, self.paths.kind).unwrap();
            reconcile(&current, &mut ledger).unwrap();
            prepare(current, ledger, desired.as_object().unwrap(), action)
        }

        fn apply(&self, value: Value) {
            let _lock = lock(&self.paths).unwrap();
            persist(&self.paths, &self.prepared(value, Action::Apply), None).unwrap();
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn planning_is_default_and_dry_runs_do_not_mutate() {
        let opts = parse(&[]).unwrap();
        assert_eq!(opts.mode, Mode::Plan);
        assert_eq!(opts.action, Action::Apply);
        for verb in ["apply", "rollback"] {
            let opts = parse(&[verb.into(), "--dry-run".into(), "--keybindings".into()]).unwrap();
            assert_eq!(opts.mode, Mode::Plan);
            assert_eq!(opts.kind, Kind::Keybindings);
        }
        for args in [
            vec!["--force".into()],
            vec!["apply".into(), "rollback".into()],
        ] {
            assert!(parse(&args).is_err());
        }
        let scratch = Scratch::new(Kind::Settings);
        let before = fs::read_dir(&scratch.root).unwrap().count();
        let prepared = scratch.prepared(json!({"enableAnalytics":false}), Action::Apply);
        assert_eq!(prepared.changed, ["enableAnalytics"]);
        assert_eq!(fs::read_dir(&scratch.root).unwrap().count(), before);
    }

    #[test]
    fn first_apply_snapshots_existing_values_and_preserves_unowned_data() {
        let scratch = Scratch::new(Kind::Settings);
        let initial = json!({"enableAnalytics":true,"theme":"custom","packages":["x"],"extensions":["mine"],"unknown":{"nested":true}});
        scratch.settings(initial.clone());
        scratch.apply(json!({"enableAnalytics":false,"cacheWarming":"off"}));
        let after = read_settings(&scratch.paths.settings).unwrap();
        assert_eq!(after["enableAnalytics"], false);
        for key in ["theme", "packages", "extensions", "unknown"] {
            assert_eq!(after[key], initial[key]);
        }
        let ledger = read_ledger(&scratch.paths.ledger, Kind::Settings).unwrap();
        assert_eq!(
            ledger.entries["enableAnalytics"].original,
            Slot::Value(json!(true))
        );
        assert_eq!(ledger.entries["cacheWarming"].original, Slot::Absent);
        assert!(ledger.pending.is_none());
    }

    #[test]
    fn rollback_distinguishes_missing_and_explicit_null() {
        let scratch = Scratch::new(Kind::Settings);
        scratch.settings(json!({"enableAnalytics":null,"extensions":["mine"]}));
        scratch.apply(json!({"enableAnalytics":false,"cacheWarming":"off"}));
        let prepared = scratch.prepared(json!({}), Action::Rollback);
        persist(&scratch.paths, &prepared, None).unwrap();
        let after = read_settings(&scratch.paths.settings).unwrap();
        assert_eq!(after["enableAnalytics"], Value::Null);
        assert!(!after.contains_key("cacheWarming"));
        assert_eq!(after["extensions"], json!(["mine"]));
        assert!(read_ledger(&scratch.paths.ledger, Kind::Settings)
            .unwrap()
            .entries
            .is_empty());
    }

    #[test]
    fn repeated_apply_retains_first_original_and_is_byte_idempotent() {
        let scratch = Scratch::new(Kind::Settings);
        scratch.settings(json!({"cacheWarming":"idle"}));
        scratch.apply(json!({"cacheWarming":"off"}));
        let settings_before = fs::read(&scratch.paths.settings).unwrap();
        let ledger_before = fs::read(&scratch.paths.ledger).unwrap();
        scratch.apply(json!({"cacheWarming":"off"}));
        assert_eq!(fs::read(&scratch.paths.settings).unwrap(), settings_before);
        assert_eq!(fs::read(&scratch.paths.ledger).unwrap(), ledger_before);
        scratch.apply(json!({"cacheWarming":"streaming"}));
        assert_eq!(
            read_ledger(&scratch.paths.ledger, Kind::Settings)
                .unwrap()
                .entries["cacheWarming"]
                .original,
            Slot::Value(json!("idle"))
        );
    }

    #[test]
    fn removed_baseline_keys_restore_originals() {
        let scratch = Scratch::new(Kind::Settings);
        scratch.settings(json!({"enableAnalytics":true}));
        scratch.apply(json!({"enableAnalytics":false,"cacheWarming":"off"}));
        scratch.apply(json!({"cacheWarming":"off"}));
        assert_eq!(
            read_settings(&scratch.paths.settings).unwrap()["enableAnalytics"],
            true
        );
        assert!(!read_ledger(&scratch.paths.ledger, Kind::Settings)
            .unwrap()
            .entries
            .contains_key("enableAnalytics"));
    }

    #[test]
    fn owned_drift_blocks_every_write_in_apply_and_rollback() {
        let scratch = Scratch::new(Kind::Settings);
        scratch.apply(json!({"enableAnalytics":false}));
        scratch.settings(json!({"enableAnalytics":true,"unknown":"keep"}));
        let settings_before = fs::read(&scratch.paths.settings).unwrap();
        let ledger_before = fs::read(&scratch.paths.ledger).unwrap();
        for action in [Action::Apply, Action::Rollback] {
            let prepared = scratch.prepared(
                json!({"enableAnalytics":false,"cacheWarming":"off"}),
                action,
            );
            assert_eq!(prepared.conflict, ["enableAnalytics"]);
            assert!(persist(&scratch.paths, &prepared, None).is_err());
            assert_eq!(fs::read(&scratch.paths.settings).unwrap(), settings_before);
            assert_eq!(fs::read(&scratch.paths.ledger).unwrap(), ledger_before);
        }
    }

    #[test]
    fn unrelated_edits_are_preserved_but_late_edits_abort() {
        let scratch = Scratch::new(Kind::Settings);
        scratch.apply(json!({"enableAnalytics":false}));
        scratch.settings(json!({"enableAnalytics":false,"packages":["new"]}));
        let prepared = scratch.prepared(
            json!({"enableAnalytics":false,"cacheWarming":"off"}),
            Action::Apply,
        );
        assert!(prepared.conflict.is_empty());
        scratch.settings(json!({"enableAnalytics":false,"packages":["later"]}));
        assert!(persist(&scratch.paths, &prepared, None).is_err());
        assert_eq!(
            read_settings(&scratch.paths.settings).unwrap()["packages"],
            json!(["later"])
        );
    }

    #[test]
    fn nested_defaults_preserve_extras_and_detect_whole_object_drift() {
        let scratch = Scratch::new(Kind::Settings);
        scratch.settings(json!({"compaction":{"enabled":true,"custom":{"v":1}}}));
        scratch.apply(json!({"compaction":{"enabled":false}}));
        let current = read_settings(&scratch.paths.settings).unwrap();
        assert_eq!(current["compaction"]["custom"], json!({"v":1}));
        scratch.settings(json!({"compaction":{"enabled":false,"custom":{"v":2}}}));
        assert_eq!(
            scratch.prepared(json!({}), Action::Rollback).conflict,
            ["compaction"]
        );
    }

    #[test]
    fn interrupted_write_reconciles_before_or_after_settings_commit() {
        for phase in [1, 2] {
            let scratch = Scratch::new(Kind::Settings);
            scratch.settings(json!({"enableAnalytics":true,"unowned":"kept"}));
            let prepared = scratch.prepared(json!({"enableAnalytics":false}), Action::Apply);
            assert!(persist(&scratch.paths, &prepared, Some(phase)).is_err());
            let settings = read_settings(&scratch.paths.settings).unwrap();
            let mut ledger = read_ledger(&scratch.paths.ledger, Kind::Settings).unwrap();
            assert!(ledger.pending.is_some());
            assert!(reconcile(&settings, &mut ledger).unwrap());
            assert!(ledger.pending.is_none());
            if phase == 1 {
                assert!(ledger.entries.is_empty());
                assert_eq!(settings["enableAnalytics"], true);
            } else {
                assert_eq!(
                    ledger.entries["enableAnalytics"].last_applied,
                    Slot::Value(json!(false))
                );
                assert_eq!(settings["enableAnalytics"], false);
            }
            assert_eq!(settings["unowned"], "kept");
        }
    }

    #[test]
    fn interrupted_transaction_refuses_a_third_owned_value() {
        let scratch = Scratch::new(Kind::Settings);
        scratch.settings(json!({"cacheWarming":"idle"}));
        let prepared = scratch.prepared(json!({"cacheWarming":"off"}), Action::Apply);
        assert!(persist(&scratch.paths, &prepared, Some(1)).is_err());
        scratch.settings(json!({"cacheWarming":"streaming"}));
        let mut ledger = read_ledger(&scratch.paths.ledger, Kind::Settings).unwrap();
        assert!(reconcile(
            &read_settings(&scratch.paths.settings).unwrap(),
            &mut ledger
        )
        .is_err());
        assert!(ledger.pending.is_some());
    }

    #[test]
    fn unsupported_keys_and_malformed_files_do_not_disclose_values() {
        for key in [
            "auth",
            "apiKey",
            "models",
            "packages",
            "extensions",
            "skills",
            "prompts",
            "themes",
        ] {
            assert!(validate_setting(key, &json!("PRIVATE_VALUE_SENTINEL")).is_err());
            assert!(!setting_key(key));
        }
        let scratch = Scratch::new(Kind::Settings);
        fs::write(&scratch.paths.settings, "PRIVATE_VALUE_SENTINEL").unwrap();
        let error = read_settings(&scratch.paths.settings).unwrap_err();
        assert!(!error.contains("PRIVATE_VALUE_SENTINEL"));
        fs::write(
            &scratch.paths.ledger,
            "{\"version\":1,\"entries\":{\"auth\":\"PRIVATE_VALUE_SENTINEL\"}}",
        )
        .unwrap();
        let error = read_ledger(&scratch.paths.ledger, Kind::Settings).unwrap_err();
        assert!(!error.contains("PRIVATE_VALUE_SENTINEL"));
    }

    #[test]
    fn validates_scoped_model_and_compaction_values() {
        assert!(validate_setting("modelThinkingLevels", &json!({"provider/model":"high"})).is_ok());
        assert!(validate_setting("modelThinkingLevels", &json!({"model":"high"})).is_err());
        assert!(validate_setting(
            "compaction",
            &json!({"enabled":false,"modelOverrides":{"provider/model":{"reserveTokens":0}}})
        )
        .is_ok());
        for value in [
            json!({"reserveTokens":-1}),
            json!({"reserveTokens":1.5}),
            json!({"unknown":true}),
            json!({"modelOverrides":{"provider/model":{"enabled":false}}}),
        ] {
            assert!(validate_setting("compaction", &value).is_err());
        }
    }

    #[test]
    fn keybindings_are_an_independent_reversible_transaction() {
        let scratch = Scratch::new(Kind::Keybindings);
        let initial = json!({"app.session.new":"ctrl+n","app.custom.action":["super+k"],"$schema":"local-schema"});
        scratch.settings(initial.clone());
        scratch.apply(json!({"app.session.new":[]}));
        assert_eq!(
            read_settings(&scratch.paths.settings).unwrap()["app.session.new"],
            json!([])
        );
        let prepared = scratch.prepared(json!({}), Action::Rollback);
        persist(&scratch.paths, &prepared, None).unwrap();
        assert_eq!(
            Value::Object(read_settings(&scratch.paths.settings).unwrap()),
            initial
        );
        assert_eq!(
            scratch.paths.ledger.file_name().unwrap(),
            "keybindings.sjel-managed.json"
        );
        assert!(validate_binding("app.session.new", &json!([])).is_ok());
        assert!(validate_binding("app.custom.action", &json!(["super+k", "ctrl+shift+k"])).is_ok());
        assert!(validate_binding("app.custom.action", &json!("ctrl+\nx")).is_err());
        assert!(validate_binding("auth", &json!("secret")).is_err());
    }

    #[test]
    fn concurrent_invocations_do_not_replace_the_lock() {
        let scratch = Scratch::new(Kind::Settings);
        let first = lock(&scratch.paths).unwrap();
        assert!(lock(&scratch.paths).is_err());
        drop(first);
        assert!(lock(&scratch.paths).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn writes_private_files_and_refuses_symlinks() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let scratch = Scratch::new(Kind::Settings);
        scratch.apply(json!({"enableAnalytics":false}));
        for path in [&scratch.paths.settings, &scratch.paths.ledger] {
            assert_eq!(
                fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        fs::remove_file(&scratch.paths.settings).unwrap();
        let destination = scratch.root.join("unrelated.json");
        fs::write(&destination, b"{}").unwrap();
        symlink(&destination, &scratch.paths.settings).unwrap();
        assert!(read_settings(&scratch.paths.settings).is_err());
        assert_eq!(fs::read(&destination).unwrap(), b"{}");
    }
}
