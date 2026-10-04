//! The write half of `tools/lib/pack-deploy.ts`, ported 2026-10-04. That file is deleted:
//! this and `engine.rs` are now the only implementation of the ledger's format and mutations.
//!
//! `sync`, `promote`, `accept`, `use` and `deployPack`/`removePack` all go through here. The
//! engine copies a unit into a staging directory outside the harness's discovery root,
//! validates it, installs it atomically, and records the digest in the ledger under a lock.
//! The read half (`engine.rs`) shares the ledger format and the digest functions.
//!
//! Two deliberate differences from pack-deploy.ts, both named in `tools/sjel-cli/README.md`:
//! the ledger's `packs`/`skills` maps are sorted here (insertion-ordered there) because a
//! ledger is read by name; and a pid-liveness check goes through `kill -0`, because std cannot
//! signal a pid and the workspace denies `unsafe`.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{ErrorKind, Write as _};
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Deserialize;

use super::engine::{
    self, available_packs, digest_destination, digest_files, digest_tree,
    legacy_digest_destination, pack_deployer, pack_units, read_state, validate_unit, DeployConfig,
    SkillRecord, State, Unit, DIGEST_POLICY,
};
use crate::time;

const LOCK_STALE_MS: u128 = 30_000;

// ---- the mutation lock ---------------------------------------------------------------------
//
// writeState is atomic, so no reader sees half a ledger. The race is different: every mutator
// reads the whole state once, mutates the copy, and writes the whole file back — two overlapping
// processes each hold a snapshot and the last writer erases the other's entries. Re-entrant on
// purpose: activate_profile calls remove_pack and deploy_pack.

struct LockState {
    depth: u32,
    path: Option<PathBuf>,
}

static LOCK: Mutex<LockState> = Mutex::new(LockState {
    depth: 0,
    path: None,
});

fn lock_path_for(config: &DeployConfig) -> PathBuf {
    let mut p = config.state_file.clone().into_os_string();
    p.push(".lock");
    PathBuf::from(p)
}

fn pid_is_alive(pid: i64) -> bool {
    if pid <= 0 {
        return false;
    }
    match std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .output()
    {
        Ok(out) => {
            out.status.success() || String::from_utf8_lossy(&out.stderr).contains("permitted")
        }
        // `kill` itself is missing: treat the holder as alive rather than steal a live lock.
        Err(_) => true,
    }
}

pub fn with_state_lock<T>(
    config: &DeployConfig,
    run: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let path = lock_path_for(config);
    {
        let mut held = LOCK.lock().unwrap();
        if held.depth > 0 && held.path.as_deref() == Some(path.as_path()) {
            held.depth += 1;
            drop(held);
            let out = run();
            LOCK.lock().unwrap().depth -= 1;
            return out;
        }
    }

    let wait_ms: u64 = std::env::var("SJEL_PACK_LOCK_WAIT_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(4_000);
    let deadline = Instant::now() + Duration::from_millis(wait_ms);
    loop {
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
        {
            Ok(mut f) => {
                let _ = writeln!(
                    f,
                    "{}",
                    serde_json::json!({ "pid": std::process::id(), "at": time::now_iso() })
                );
                break;
            }
            Err(e) if e.kind() == ErrorKind::AlreadyExists => {
                // Steal only from a holder that is provably gone, or from a lock old enough that
                // a crashed holder is the only explanation.
                let holder = fs::read_to_string(&path)
                    .ok()
                    .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
                    .and_then(|v| v.get("pid").and_then(serde_json::Value::as_i64))
                    .unwrap_or(-1);
                let age_ms = fs::metadata(&path)
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.elapsed().ok())
                    .map(|d| d.as_millis())
                    .unwrap_or(u128::MAX);
                if !pid_is_alive(holder) || age_ms > LOCK_STALE_MS {
                    let _ = fs::remove_file(&path);
                    continue;
                }
                if Instant::now() > deadline {
                    return Err(format!(
                        "{} ledger is locked by pid {holder} ({}); it has been held for {}s. Wait \
                         for that run to finish, or remove the lock file if that process is gone",
                        config.adapter,
                        path.display(),
                        age_ms / 1000
                    ));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(format!("{}: {e}", path.display())),
        }
    }

    {
        let mut held = LOCK.lock().unwrap();
        held.depth = 1;
        held.path = Some(path.clone());
    }
    let out = run();
    {
        let mut held = LOCK.lock().unwrap();
        held.depth = 0;
        held.path = None;
    }
    let _ = fs::remove_file(&path);
    out
}

fn write_state(config: &DeployConfig, state: &State) -> Result<(), String> {
    if let Some(parent) = config.state_file.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let name = config
        .state_file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let temp = config
        .state_file
        .with_file_name(format!("{name}.tmp-{}", std::process::id()));
    let text = format!(
        "{}\n",
        serde_json::to_string_pretty(state).map_err(|e| e.to_string())?
    );
    fs::write(&temp, text).map_err(|e| format!("{}: {e}", temp.display()))?;
    let _ = fs::set_permissions(&temp, fs::Permissions::from_mode(0o600));
    fs::rename(&temp, &config.state_file)
        .map_err(|e| format!("{}: {e}", config.state_file.display()))
}

/// `path.relative(root)` for the ledger's `source` field, lexically.
fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn basename(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

// ---- the install ---------------------------------------------------------------------------

fn adopt_digest_policy_if_safe(
    config: &DeployConfig,
    record: &mut SkillRecord,
    destination: &Path,
    unit_key: &str,
) -> Result<(), String> {
    if record.digest_policy.as_deref() == Some(DIGEST_POLICY) {
        if digest_destination(config, destination)? != record.installed_digest {
            return Err(format!("{unit_key}: installed copy has local changes"));
        }
        return Ok(());
    }
    if legacy_digest_destination(config, destination)? != record.installed_digest {
        return Err(format!(
            "{unit_key}: legacy digest is ambiguous; review the destination and run \
             migrate-generated <pack> --accept-current"
        ));
    }
    record.installed_digest = digest_destination(config, destination)?;
    record.digest_policy = Some(DIGEST_POLICY.to_owned());
    Ok(())
}

/// The destination a recorded unit occupies. Rebuilt from the key rather than stored, so a
/// ledger written before the tree convention existed still resolves.
pub fn recorded_destination(config: &DeployConfig, pack: &str, unit_key: &str) -> PathBuf {
    if let Some(tree) = &config.tree_convention {
        if unit_key == format!("{}/", tree.source_dir) {
            return tree.destination_root.join(pack);
        }
    }
    if let Some(flat) = &config.flat_file_convention {
        let prefix = format!("{}/", flat.source_dir);
        if let Some(rest) = unit_key.strip_prefix(&prefix) {
            return flat.destination_root.join(rest);
        }
    }
    config.destination.join(unit_key)
}

/// The Pack that already occupies this unit's destination, if any. Ownership is a claim on a
/// PATH, not on a name: a tree unit's key is the source directory while its destination carries
/// the pack name, so two Packs each carrying `agents/` share a key and collide in no other way.
fn owner_of(config: &DeployConfig, state: &State, unit: &Unit) -> Option<String> {
    for (pack, record) in &state.packs {
        for unit_key in record.skills.keys() {
            if recorded_destination(config, pack, unit_key) == unit.destination {
                return Some(pack.clone());
            }
        }
    }
    None
}

fn record_unit(config: &DeployConfig, state: &mut State, pack: &str, unit: &Unit, digest: &str) {
    let record = state.packs.entry(pack.to_owned()).or_default();
    record.skills.insert(
        unit.key.clone(),
        SkillRecord {
            source: relative(&config.axon_root, &unit.source_root),
            desired_digest: digest.to_owned(),
            installed_digest: digest.to_owned(),
            digest_policy: Some(DIGEST_POLICY.to_owned()),
            deployed_at: time::now_iso(),
        },
    );
}

fn unique_temp_dir(parent: &Path, prefix: &str) -> Result<PathBuf, String> {
    for i in 0..1000u32 {
        let candidate = parent.join(format!("{prefix}{}-{i}", std::process::id()));
        match fs::create_dir(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("{}: {e}", candidate.display())),
        }
    }
    Err(format!(
        "{}: cannot create a staging directory",
        parent.display()
    ))
}

fn materialize_stage(
    config: &DeployConfig,
    pack: &str,
    unit: &Unit,
) -> Result<(PathBuf, String), String> {
    let destination_root = unit
        .destination
        .parent()
        .ok_or_else(|| format!("{}: destination has no parent", unit.destination.display()))?
        .to_path_buf();
    fs::create_dir_all(&destination_root)
        .map_err(|e| format!("{}: {e}", destination_root.display()))?;
    // Keep staging outside the discovery root: a harness scans its skill directory recursively,
    // so even a short-lived half-built tree must not appear there.
    let stage_parent = destination_root.parent().unwrap_or(&destination_root);
    let stage = unique_temp_dir(
        stage_parent,
        &format!(
            ".axon-{}-stage-{}-",
            config.adapter,
            basename(&unit.destination)
        ),
    )?;
    let build = || -> Result<String, String> {
        let files = engine::desired_files(config, pack, unit)?;
        validate_unit(config, &files, unit, &format!("{pack}/{}", unit.key))?;
        for file in files.values() {
            let dest = file
                .relative_path
                .split('/')
                .fold(stage.clone(), |p, part| p.join(part));
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
            }
            // A transformed file is written from its bytes; everything else is copied, so the
            // common path keeps the mtime and hard-link behaviour it always had.
            match &file.content {
                Some(bytes) => fs::write(&dest, bytes),
                None => fs::copy(&file.absolute_path, &dest).map(|_| ()),
            }
            .map_err(|e| format!("{}: {e}", dest.display()))?;
            let _ = fs::set_permissions(&dest, fs::Permissions::from_mode(file.mode));
        }
        digest_tree(config, &stage)
    };
    match build() {
        Ok(digest) => Ok((stage, digest)),
        Err(e) => {
            let _ = fs::remove_dir_all(&stage);
            Err(e)
        }
    }
}

fn replace_atomically(config: &DeployConfig, stage: &Path, destination: &Path, single_file: bool) {
    if single_file {
        // The stage is a directory holding this one file, and the destination is a file, so
        // renaming the stage over it would fail outright (ENOTDIR). Rename the file itself.
        let from = stage.join(basename(destination));
        let _ = fs::rename(&from, destination);
        let _ = fs::remove_dir_all(stage);
        return;
    }
    if !destination.exists() {
        let _ = fs::rename(stage, destination);
        return;
    }
    let parent = destination
        .parent()
        .and_then(Path::parent)
        .unwrap_or_else(|| Path::new("."));
    let backup = parent.join(format!(
        ".axon-{}-backup-{}-{}",
        config.adapter,
        basename(destination),
        std::process::id()
    ));
    let _ = fs::rename(destination, &backup);
    if fs::rename(stage, destination).is_err() {
        let _ = fs::rename(&backup, destination);
        return;
    }
    let _ = fs::remove_dir_all(&backup);
}

fn install_one(
    config: &DeployConfig,
    state: &mut State,
    pack: &str,
    unit: &Unit,
    mode: &str,
) -> Result<String, String> {
    let destination = &unit.destination;
    let owner = owner_of(config, state, unit);
    let existing = state
        .packs
        .get(pack)
        .and_then(|p| p.skills.get(&unit.key))
        .cloned();
    if let Some(owner) = &owner {
        if owner != pack {
            return Err(format!("{}: already owned by Pack '{owner}'", unit.key));
        }
    }
    if destination.exists() && existing.is_none() {
        return Err(format!(
            "{}: {} exists and is not owned by this deployment",
            unit.key,
            destination.display()
        ));
    }
    if mode == "deploy" {
        if let (Some(mut record), true) = (existing.clone(), destination.exists()) {
            adopt_digest_policy_if_safe(config, &mut record, destination, &unit.key)?;
            state
                .packs
                .entry(pack.to_owned())
                .or_default()
                .skills
                .insert(unit.key.clone(), record);
            let actual = digest_destination(config, destination)?;
            let wanted = digest_files(&engine::desired_files(config, pack, unit)?);
            return Ok(if wanted == actual {
                format!("= {} (already current)", unit.key)
            } else {
                format!("= {} (deployed; run sync to update)", unit.key)
            });
        }
    } else if let (Some(mut record), true) = (existing.clone(), destination.exists()) {
        if let Err(e) = adopt_digest_policy_if_safe(config, &mut record, destination, &unit.key) {
            return Err(format!("{e}; refusing to overwrite"));
        }
        state
            .packs
            .entry(pack.to_owned())
            .or_default()
            .skills
            .insert(unit.key.clone(), record);
    }

    let (stage, digest) = materialize_stage(config, pack, unit)?;
    if destination.exists() && digest_destination(config, destination)? == digest {
        let _ = fs::remove_dir_all(&stage);
        record_unit(config, state, pack, unit, &digest);
        return Ok(format!("= {} (already current)", unit.key));
    }
    replace_atomically(config, &stage, destination, unit.only_file.is_some());
    record_unit(config, state, pack, unit, &digest);
    Ok(format!(
        "✓ {} {}",
        unit.key,
        if mode == "deploy" {
            "deployed"
        } else {
            "synced"
        }
    ))
}

fn remove_owned_unit(
    config: &DeployConfig,
    state: &mut State,
    pack: &str,
    unit_key: &str,
) -> Result<String, String> {
    let Some(record) = state
        .packs
        .get(pack)
        .and_then(|p| p.skills.get(unit_key))
        .cloned()
    else {
        return Err(format!("{unit_key}: not owned by Pack '{pack}'"));
    };
    let destination = recorded_destination(config, pack, unit_key);
    if destination.exists() {
        let mut record = record;
        adopt_digest_policy_if_safe(config, &mut record, &destination, unit_key)
            .map_err(|e| format!("{e}; refusing to remove"))?;
        fs::remove_dir_all(&destination).map_err(|e| format!("{}: {e}", destination.display()))?;
    }
    if let Some(record) = state.packs.get_mut(pack) {
        record.skills.remove(unit_key);
        if record.skills.is_empty() {
            state.packs.remove(pack);
        }
    }
    Ok(format!("✓ {unit_key} removed"))
}

// ---- the verbs -----------------------------------------------------------------------------

pub fn sync_pack(config: &DeployConfig, pack: &str) -> Result<Vec<String>, String> {
    with_state_lock(config, || {
        let units = pack_units(config, pack)?;
        let mut state = read_state(config)?;
        if !state.packs.contains_key(pack) {
            return Err(format!("{pack}: not deployed; deploy it first"));
        }
        let desired: BTreeSet<String> = units.iter().map(|u| u.key.clone()).collect();
        let mut messages = Vec::new();
        let mut failures: Vec<String> = Vec::new();

        // Preflight both stale removals and desired updates before mutating either.
        for (unit_key, record) in state.packs[pack].skills.clone() {
            let destination = recorded_destination(config, pack, &unit_key);
            if destination.exists() {
                let mut record = record;
                adopt_digest_policy_if_safe(config, &mut record, &destination, &unit_key)
                    .map_err(|e| format!("{e}; refusing to sync"))?;
            }
        }
        for unit in &units {
            let files = engine::desired_files(config, pack, unit)?;
            validate_unit(config, &files, unit, &format!("{pack}/{}", unit.key))?;
            if let Some(owner) = owner_of(config, &state, unit) {
                if owner != pack {
                    return Err(format!("{}: already owned by Pack '{owner}'", unit.key));
                }
            }
            if unit.destination.exists() && !state.packs[pack].skills.contains_key(&unit.key) {
                return Err(format!(
                    "{}: {} exists and is not owned by this deployment",
                    unit.key,
                    unit.destination.display()
                ));
            }
        }

        let stale: Vec<String> = state.packs[pack]
            .skills
            .keys()
            .filter(|k| !desired.contains(*k))
            .cloned()
            .collect();
        for unit_key in stale {
            match remove_owned_unit(config, &mut state, pack, &unit_key).and_then(|m| {
                write_state(config, &state)?;
                Ok(m)
            }) {
                Ok(m) => messages.push(m),
                Err(e) => failures.push(e),
            }
        }
        for unit in &units {
            match install_one(config, &mut state, pack, unit, "sync").and_then(|m| {
                write_state(config, &state)?;
                Ok(m)
            }) {
                Ok(m) => messages.push(m),
                Err(e) => failures.push(e),
            }
        }
        if !failures.is_empty() {
            return Err(failures.join("\n"));
        }
        Ok(messages)
    })
}

pub fn adopt_pack(config: &DeployConfig, pack: &str) -> Result<Vec<String>, String> {
    with_state_lock(config, || {
        let units = pack_units(config, pack)?;
        let mut state = read_state(config)?;
        let mut messages = Vec::new();
        let mut failures: Vec<String> = Vec::new();
        for unit in &units {
            if owner_of(config, &state, unit).as_deref() == Some(pack) {
                messages.push(format!("= {} (already owned)", unit.key));
                continue;
            }
            if let Some(owner) = owner_of(config, &state, unit) {
                failures.push(format!("{}: already owned by Pack '{owner}'", unit.key));
                continue;
            }
            if !unit.destination.exists() {
                messages.push(format!("= {} (not deployed; nothing to adopt)", unit.key));
                continue;
            }
            let files = engine::desired_files(config, pack, unit)?;
            validate_unit(config, &files, unit, &format!("{pack}/{}", unit.key))?;
            let wanted = digest_files(&files);
            let actual = digest_destination(config, &unit.destination)?;
            if wanted != actual {
                failures.push(format!(
                    "{}: {} differs from the Pack source; refusing to adopt",
                    unit.key,
                    unit.destination.display()
                ));
                continue;
            }
            record_unit(config, &mut state, pack, unit, &wanted);
            write_state(config, &state)?;
            messages.push(format!("✓ {} adopted", unit.key));
        }
        if !failures.is_empty() {
            return Err(failures.join("\n"));
        }
        Ok(messages)
    })
}

pub fn reconcile_unit(config: &DeployConfig, pack: &str, unit: &Unit) -> Result<String, String> {
    with_state_lock(config, || {
        let mut state = read_state(config)?;
        let Some(record) = state.packs.get(pack).and_then(|p| p.skills.get(&unit.key)) else {
            return Err(format!("{}: not owned by Pack '{pack}'", unit.key));
        };
        if !unit.destination.exists() {
            return Err(format!(
                "{}: {} does not exist",
                unit.key,
                unit.destination.display()
            ));
        }
        let files = engine::desired_files(config, pack, unit)?;
        // Validate before recording: an accepted edit can have broken the frontmatter, and a
        // ledger that records a broken skill as current is worse than one that reports drift.
        validate_unit(config, &files, unit, &format!("{pack}/{}", unit.key))?;
        let wanted = digest_files(&files);
        let installed = digest_destination(config, &unit.destination)?;
        if wanted != installed {
            return Err(format!(
                "{}: destination still differs from the Pack source; refusing to re-record",
                unit.key
            ));
        }
        if record.installed_digest == wanted && record.desired_digest == wanted {
            return Ok(format!("= {} (already recorded)", unit.key));
        }
        record_unit(config, &mut state, pack, unit, &wanted);
        write_state(config, &state)?;
        Ok(format!("✓ {} re-recorded", unit.key))
    })
}

pub fn deploy_pack(
    config: &DeployConfig,
    pack: &str,
    skill_subset: Option<&BTreeSet<String>>,
) -> Result<Vec<String>, String> {
    with_state_lock(config, || {
        let units: Vec<Unit> = pack_units(config, pack)?
            .into_iter()
            .filter(|u| skill_subset.is_none_or(|s| !u.is_skill || s.contains(&u.key)))
            .collect();
        let mut state = read_state(config)?;
        // Validate every source and collision before the first write so a bad unit cannot leave
        // a normally-failing Pack only partially deployed.
        for unit in &units {
            let files = engine::desired_files(config, pack, unit)?;
            validate_unit(config, &files, unit, &format!("{pack}/{}", unit.key))?;
            if let Some(owner) = owner_of(config, &state, unit) {
                if owner != pack {
                    return Err(format!("{}: already owned by Pack '{owner}'", unit.key));
                }
            }
            let recorded = state
                .packs
                .get(pack)
                .and_then(|p| p.skills.get(&unit.key))
                .cloned();
            if unit.destination.exists() && recorded.is_none() {
                return Err(format!(
                    "{}: {} exists and is not owned by this deployment",
                    unit.key,
                    unit.destination.display()
                ));
            }
            if let (Some(mut record), true) = (recorded, unit.destination.exists()) {
                if let Err(e) =
                    adopt_digest_policy_if_safe(config, &mut record, &unit.destination, &unit.key)
                {
                    return Err(format!("{e}; refusing to redeploy"));
                }
                state
                    .packs
                    .entry(pack.to_owned())
                    .or_default()
                    .skills
                    .insert(unit.key.clone(), record);
            }
        }
        let mut messages = Vec::new();
        let mut failures: Vec<String> = Vec::new();
        for unit in &units {
            match install_one(config, &mut state, pack, unit, "deploy").and_then(|m| {
                write_state(config, &state)?;
                Ok(m)
            }) {
                Ok(m) => messages.push(m),
                Err(e) => failures.push(e),
            }
        }
        if !failures.is_empty() {
            return Err(failures.join("\n"));
        }
        Ok(messages)
    })
}

pub fn remove_pack(config: &DeployConfig, pack: &str) -> Result<Vec<String>, String> {
    with_state_lock(config, || {
        let mut state = read_state(config)?;
        if !state.packs.contains_key(pack) {
            return Err(format!("{pack}: not deployed"));
        }
        for (unit_key, record) in state.packs[pack].skills.clone() {
            let destination = recorded_destination(config, pack, &unit_key);
            if destination.exists() {
                let mut record = record;
                adopt_digest_policy_if_safe(config, &mut record, &destination, &unit_key)
                    .map_err(|e| format!("{e}; refusing to remove"))?;
            }
        }
        let keys: Vec<String> = state.packs[pack].skills.keys().cloned().collect();
        let mut messages = Vec::new();
        let mut failures: Vec<String> = Vec::new();
        for unit_key in keys {
            match remove_owned_unit(config, &mut state, pack, &unit_key).and_then(|m| {
                write_state(config, &state)?;
                Ok(m)
            }) {
                Ok(m) => messages.push(m),
                Err(e) => failures.push(e),
            }
        }
        if !failures.is_empty() {
            return Err(failures.join("\n"));
        }
        Ok(messages)
    })
}

/// The digest-policy migration `tools/packs-codex migrate-generated` runs: remove the generated
/// artifacts a legacy deployment copied in, then re-record the destination under the current
/// policy. `--accept-current` is required because reading the destination before trusting it is
/// the whole point of the verb.
pub fn migrate_generated_artifacts(
    config: &DeployConfig,
    pack: &str,
    accept_current: bool,
) -> Result<Vec<String>, String> {
    if !accept_current {
        return Err(
            "migration requires --accept-current after reviewing non-generated destination files"
                .to_owned(),
        );
    }
    with_state_lock(config, || {
        let mut state = read_state(config)?;
        if !state.packs.contains_key(pack) {
            return Err(format!("{pack}: not deployed"));
        }

        // Every plan is built before the first file is removed, so a destination that has gone
        // missing stops the migration instead of half-performing it.
        let keys: Vec<String> = state.packs[pack].skills.keys().cloned().collect();
        let mut plans: Vec<(String, GeneratedArtifacts)> = Vec::new();
        for unit_key in keys {
            let legacy =
                state.packs[pack].skills[&unit_key].digest_policy.as_deref() != Some(DIGEST_POLICY);
            if !legacy {
                continue;
            }
            let destination = recorded_destination(config, pack, &unit_key);
            if !destination.exists() {
                return Err(format!("{unit_key}: owned destination is missing"));
            }
            plans.push((
                unit_key.clone(),
                known_generated_artifacts(&destination, &format!("{pack}/{unit_key}"))?,
            ));
        }

        let mut messages = Vec::new();
        for (unit_key, artifacts) in plans {
            let destination = recorded_destination(config, pack, &unit_key);
            for file in &artifacts.files {
                let _ = fs::remove_file(file);
            }
            // Deepest first, so a cache directory that held only a cache is empty when its turn
            // comes; one that holds anything else is left alone.
            let mut directories = artifacts.directories.clone();
            directories.sort_by_key(|dir| std::cmp::Reverse(dir.as_os_str().len()));
            for directory in directories {
                let empty = fs::read_dir(&directory)
                    .map(|mut entries| entries.next().is_none())
                    .unwrap_or(false);
                if empty {
                    let _ = fs::remove_dir(&directory);
                }
            }
            let installed = digest_destination(config, &destination)?;
            let record = state
                .packs
                .get_mut(pack)
                .and_then(|p| p.skills.get_mut(&unit_key))
                .ok_or_else(|| format!("{unit_key}: ledger row disappeared"))?;
            record.installed_digest = installed;
            record.digest_policy = Some(DIGEST_POLICY.to_owned());
            messages.push(format!(
                "✓ {unit_key} migrated ({} generated artifact(s) removed)",
                artifacts.files.len()
            ));
        }
        write_state(config, &state)?;
        if messages.is_empty() {
            messages.push(format!("= {pack} (digest policy already current)"));
        }
        Ok(messages)
    })
}

/// What the migration removes, and the directories it may then find empty.
struct GeneratedArtifacts {
    files: Vec<PathBuf>,
    directories: Vec<PathBuf>,
}

/// Walk a destination for the artifacts a legacy deployment copied in: anything under
/// `__pycache__`, any `.py[cod]` file, and `.DS_Store` beside a cache.
///
/// Directory entries are read in name order where pack-deploy.ts took `readdirSync` order, so
/// the message list is the same on every machine; the set removed is identical either way.
fn known_generated_artifacts(root: &Path, label: &str) -> Result<GeneratedArtifacts, String> {
    let mut found = GeneratedArtifacts {
        files: Vec::new(),
        directories: Vec::new(),
    };
    visit_generated(root, root, label, false, &mut found)?;
    Ok(found)
}

fn visit_generated(
    root: &Path,
    dir: &Path,
    label: &str,
    inside_cache: bool,
    found: &mut GeneratedArtifacts,
) -> Result<(), String> {
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect();
    entries.sort();
    for path in entries {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let rel = path
            .strip_prefix(root)
            .map(|r| r.to_string_lossy().into_owned())
            .unwrap_or_default();
        let in_cache = inside_cache || name == "__pycache__";
        let generated_file = engine::is_generated_artifact_name(&name);
        let meta = fs::symlink_metadata(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        if in_cache {
            if meta.file_type().is_symlink() {
                return Err(format!(
                    "{label}: generated-artifact migration refuses symlink {rel}"
                ));
            }
            if meta.is_dir() {
                found.directories.push(path.clone());
                visit_generated(root, &path, label, true, found)?;
            } else if meta.is_file() && (generated_file || name == ".DS_Store") {
                found.files.push(path);
            } else {
                return Err(format!(
                    "{label}: unknown content inside __pycache__: {rel}"
                ));
            }
        } else if generated_file {
            if !meta.is_file() {
                return Err(format!(
                    "{label}: generated-artifact migration refuses non-file {rel}"
                ));
            }
            found.files.push(path);
        } else if meta.is_dir() && !meta.file_type().is_symlink() {
            visit_generated(root, &path, label, false, found)?;
        }
    }
    Ok(())
}

// ---- profiles ------------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct Profile {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub packs: Vec<String>,
    /// Packs a `*` profile must NOT deploy. Only meaningful with `packs = ["*"]`.
    #[serde(default)]
    pub except: Option<Vec<String>>,
    /// Optional per-Pack skill subset. A Pack with no entry loads all its skills.
    #[serde(default)]
    pub skills: Option<BTreeMap<String, Vec<String>>>,
}

pub fn read_profiles(config: &DeployConfig) -> Result<Vec<Profile>, String> {
    read_profiles_at(&config.axon_root)
}

/// `profiles.toml` is repository-wide, so `use` reads it before it knows which harness it is
/// activating; this is the same read without a whole DeployConfig.
pub fn read_profiles_at(root: &Path) -> Result<Vec<Profile>, String> {
    let path = root.join("profiles.toml");
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let parsed: toml::Table = text
        .parse()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let Some(profiles) = parsed.get("profile").and_then(toml::Value::as_array) else {
        return Ok(Vec::new());
    };
    profiles
        .iter()
        .map(|p| {
            p.clone()
                .try_into::<Profile>()
                .map_err(|e| format!("{}: {e}", path.display()))
        })
        .collect()
}

pub fn resolve_profile_packs(
    config: &DeployConfig,
    profile: &Profile,
) -> Result<Vec<String>, String> {
    let except = profile.except.clone().unwrap_or_default();
    if profile.packs.len() == 1 && profile.packs[0] == "*" {
        let available = available_packs(config, false)?;
        let available_set: BTreeSet<&String> = available.iter().collect();
        for pack in &except {
            // Named-but-absent is an error, not a no-op: a typo in an exclusion list reads as
            // "deployed" for a Pack that is not.
            if !available_set.contains(pack) {
                return Err(format!(
                    "profile '{}': except names unknown pack '{pack}'",
                    profile.name
                ));
            }
        }
        let excluded: BTreeSet<&String> = except.iter().collect();
        return Ok(available
            .into_iter()
            .filter(|p| !excluded.contains(p))
            .collect());
    }
    if !except.is_empty() {
        return Err(format!(
            "profile '{}': except is only meaningful with packs = [\"*\"]; list the Packs this \
             profile wants instead of excluding from a list it does not have",
            profile.name
        ));
    }
    let all: BTreeSet<String> = available_packs(config, true)?.into_iter().collect();
    for pack in &profile.packs {
        if !all.contains(pack) {
            return Err(format!("profile '{}': unknown pack '{pack}'", profile.name));
        }
        if let Some(owner) = pack_deployer(config, pack) {
            return Err(format!(
                "profile '{}': pack '{pack}' is deployed by {owner}; remove it from the profile",
                profile.name
            ));
        }
    }
    Ok(profile.packs.clone())
}

/// The per-Pack skill subset a profile asks for: `None` means "all skills of that Pack".
pub fn resolve_profile_skills(
    config: &DeployConfig,
    profile: &Profile,
) -> Result<BTreeMap<String, Option<BTreeSet<String>>>, String> {
    let mut out = BTreeMap::new();
    let Some(skills) = &profile.skills else {
        return Ok(out);
    };
    let profile_packs: BTreeSet<String> = resolve_profile_packs(config, profile)?
        .into_iter()
        .collect();
    for (pack, named) in skills {
        if !profile_packs.contains(pack) {
            return Err(format!(
                "profile '{}': skills names pack '{pack}', which is not in packs",
                profile.name
            ));
        }
        let unit_names: BTreeSet<String> = pack_units(config, pack)?
            .into_iter()
            .filter(|u| u.is_skill)
            .map(|u| u.key)
            .collect();
        for skill in named {
            if !unit_names.contains(skill) {
                return Err(format!(
                    "profile '{}': pack '{pack}' has no skill '{skill}'",
                    profile.name
                ));
            }
        }
        out.insert(pack.clone(), Some(named.iter().cloned().collect()));
    }
    Ok(out)
}

pub fn activate_profile(config: &DeployConfig, profile: &Profile) -> Result<Vec<String>, String> {
    with_state_lock(config, || {
        let target_packs: BTreeSet<String> = resolve_profile_packs(config, profile)?
            .into_iter()
            .collect();
        let subsets = resolve_profile_skills(config, profile)?;
        let state = read_state(config)?;
        let mut messages = vec![format!(
            "Activating profile '{}' — {}",
            profile.name, profile.description
        )];

        let current: Vec<String> = state.packs.keys().cloned().collect();
        let mut target: Vec<String> = target_packs.iter().cloned().collect();
        target.sort();

        let to_remove: Vec<String> = current
            .iter()
            .filter(|p| !target_packs.contains(*p))
            .cloned()
            .collect();
        let to_deploy: Vec<String> = target
            .iter()
            .filter(|p| match state.packs.get(*p) {
                None => true,
                // Re-deploy if any owned destination is missing from disk.
                Some(record) => record
                    .skills
                    .keys()
                    .any(|k| !recorded_destination(config, p, k).exists()),
            })
            .cloned()
            .collect();

        if to_remove.is_empty() && to_deploy.is_empty() {
            messages.push("  → already current".to_owned());
            return Ok(messages);
        }
        if !to_remove.is_empty() {
            messages.push(String::new());
            messages.push(format!(
                "Removing {} pack(s) not in profile:",
                to_remove.len()
            ));
            for pack in &to_remove {
                match remove_pack(config, pack) {
                    Ok(lines) => messages.extend(lines.into_iter().map(|l| format!("  {l}"))),
                    Err(e) => messages.push(format!("  ✗ {pack}: {e}")),
                }
            }
        }
        if !to_deploy.is_empty() {
            messages.push(String::new());
            messages.push(format!("Deploying {} pack(s):", to_deploy.len()));
            for pack in &to_deploy {
                let subset = subsets.get(pack).cloned().flatten();
                match deploy_pack(config, pack, subset.as_ref()) {
                    Ok(lines) => messages.extend(lines.into_iter().map(|l| format!("  {l}"))),
                    Err(e) => messages.push(format!("  ✗ {pack}: {e}")),
                }
            }
        }
        Ok(messages)
    })
}

/// Which of a profile's Packs are deployed right now — the `[active]` marker in the interactive
/// picker `tools/packs-codex use` shows.
pub fn profile_active_packs(
    config: &DeployConfig,
    profile: &Profile,
) -> Result<Vec<String>, String> {
    let target: BTreeSet<String> = resolve_profile_packs(config, profile)?
        .into_iter()
        .collect();
    let state = read_state(config)?;
    Ok(state
        .packs
        .keys()
        .filter(|pack| target.contains(*pack))
        .cloned()
        .collect())
}

// ---- pure helpers from tools/harnesses.ts --------------------------------------------------

/// Which packs `sync` should touch, given what the harness already knows about. `--all` is a
/// flag, not a positional value.
pub fn sync_targets(pack: Option<&str>, all: bool, known: &[String]) -> Vec<String> {
    if all {
        let mut out = known.to_vec();
        out.sort();
        return out;
    }
    pack.map(|p| vec![p.to_owned()]).unwrap_or_default()
}

/// A `pack.toml` `skills = [...]` line with one more skill in it.
///
/// Its own function so it can be tested: a skill name is a directory name off the destination,
/// so every character a macOS filename allows can reach here. A substring test (not a regex)
/// asks whether the name is already there, the splice never builds a replacement pattern, and a
/// line that does not close its array is refused rather than silently returned unchanged.
pub fn skills_line_with(line: &str, skill: &str) -> Result<String, String> {
    if line.contains(&format!("\"{skill}\"")) {
        return Err(format!("{skill} is already in the skills line"));
    }
    let trimmed = line.trim_end();
    if !trimmed.ends_with(']') {
        return Err(
            "the skills line does not end in `]`; tools/lib/toml.sh cannot read a multi-line array"
                .to_owned(),
        );
    }
    // An empty array has nothing to separate the new name from: `skills = []` spliced with a
    // comma gives `skills = [, "x"]`, which no TOML parser reads.
    let head = trimmed[..trimmed.len() - 1].trim_end();
    let separator = if head.ends_with('[') { "" } else { ", " };
    Ok(format!("{head}{separator}\"{skill}\"]"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skills_line_appends_inside_the_array() {
        assert_eq!(
            skills_line_with("skills = [\"trim\"]", "suggest-skills").unwrap(),
            "skills = [\"trim\", \"suggest-skills\"]"
        );
        assert_eq!(
            skills_line_with("skills = [\"trim\"]  ", "cv").unwrap(),
            "skills = [\"trim\", \"cv\"]"
        );
    }

    #[test]
    fn skills_line_refuses_a_name_already_declared() {
        let err = skills_line_with("skills = [\"trim\"]", "trim").unwrap_err();
        assert!(err.contains("already in the skills line"), "{err}");
    }

    #[test]
    fn skills_line_treats_metacharacters_as_characters() {
        assert_eq!(
            skills_line_with("skills = [\"axb\"]", "a.b").unwrap(),
            "skills = [\"axb\", \"a.b\"]"
        );
        assert_eq!(
            skills_line_with("skills = [\"a\"]", "a|b").unwrap(),
            "skills = [\"a\", \"a|b\"]"
        );
        assert_eq!(
            skills_line_with("skills = [\"trim\"]", "a$&b").unwrap(),
            "skills = [\"trim\", \"a$&b\"]"
        );
    }

    #[test]
    fn skills_line_fills_an_empty_array_without_a_leading_comma() {
        assert_eq!(
            skills_line_with("skills = []", "trim").unwrap(),
            "skills = [\"trim\"]"
        );
        assert_eq!(
            skills_line_with("skills = [ ]", "trim").unwrap(),
            "skills = [\"trim\"]"
        );
    }

    #[test]
    fn skills_line_refuses_an_array_that_does_not_close_on_this_line() {
        let err = skills_line_with("skills = [", "trim").unwrap_err();
        assert!(err.contains("multi-line array"), "{err}");
    }

    #[test]
    fn sync_targets_selects_by_pack_or_all() {
        let known = vec![
            "writing".to_owned(),
            "coding".to_owned(),
            "harness".to_owned(),
        ];
        assert_eq!(sync_targets(Some("coding"), false, &known), vec!["coding"]);
        assert_eq!(
            sync_targets(None, true, &known),
            vec!["coding", "harness", "writing"]
        );
        assert!(sync_targets(None, true, &[]).is_empty());
        assert!(sync_targets(None, false, &known).is_empty());
    }

    /// A migration removes the generated artifacts a legacy deployment copied in and re-records
    /// the destination under the current policy; a second run has nothing left to do.
    #[test]
    fn a_legacy_deployment_migrates_once_and_then_says_so() {
        let root =
            std::env::temp_dir().join(format!("sjel-harnesses-migrate-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let pack = root.join("repo/Packs/demo");
        fs::create_dir_all(pack.join("skills/alpha")).unwrap();
        fs::write(
            pack.join("pack.toml"),
            "name = \"demo\"\ndescription = \"test\"\nskills = [\"alpha\"]\n",
        )
        .unwrap();
        fs::write(
            pack.join("skills/alpha/SKILL.md"),
            "---\nname: alpha\ndescription: does a thing\n---\n\nbody\n",
        )
        .unwrap();
        let config = DeployConfig {
            axon_root: root.join("repo"),
            pack_roots: None,
            destination: root.join("dest"),
            state_file: root.join("state.json"),
            adapter: "test".to_owned(),
            state_env_var: None,
            tree_convention: None,
            flat_file_convention: None,
            skip_manifest_skills: false,
            validate_adapter_files: None,
        };
        deploy_pack(&config, "demo", None).unwrap();

        // A legacy deployment: generated artifacts in the destination and no policy on the row.
        let installed = config.destination.join("alpha");
        fs::create_dir_all(installed.join("__pycache__")).unwrap();
        fs::write(installed.join("__pycache__/mod.cpython-311.pyc"), "x").unwrap();
        fs::write(installed.join("__pycache__/.DS_Store"), "x").unwrap();
        fs::write(installed.join("stray.pyc"), "x").unwrap();
        let mut state = read_state(&config).unwrap();
        state
            .packs
            .get_mut("demo")
            .unwrap()
            .skills
            .get_mut("alpha")
            .unwrap()
            .digest_policy = None;
        write_state(&config, &state).unwrap();

        assert!(migrate_generated_artifacts(&config, "demo", false).is_err());
        let messages = migrate_generated_artifacts(&config, "demo", true).unwrap();
        assert_eq!(messages.len(), 1);
        assert!(
            messages[0].contains("3 generated artifact(s) removed"),
            "{messages:?}"
        );
        assert!(!installed.join("__pycache__").exists());
        assert!(!installed.join("stray.pyc").exists());
        assert!(
            installed.join("SKILL.md").exists(),
            "the skill itself stays"
        );

        let state = read_state(&config).unwrap();
        let record = &state.packs["demo"].skills["alpha"];
        assert_eq!(record.digest_policy.as_deref(), Some(DIGEST_POLICY));
        assert_eq!(
            record.installed_digest,
            digest_destination(&config, &installed).unwrap()
        );

        let again = migrate_generated_artifacts(&config, "demo", true).unwrap();
        assert_eq!(
            again,
            vec!["= demo (digest policy already current)".to_string()]
        );
        let _ = fs::remove_dir_all(&root);
    }
}
