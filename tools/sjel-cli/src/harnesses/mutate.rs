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

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{ErrorKind, Write as _};
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
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

thread_local! {
    /// The ledger locks THIS THREAD already holds, outermost first.
    ///
    /// Reentrancy is a property of the call stack, not of the process. It used to be one
    /// process-global `(depth, path)` pair, which conflated two different things: two threads
    /// deploying two different state files reset each other's counter, so one thread's matching
    /// decrement landed on a zero another thread had just written. Measured 2026-10-24 as
    /// `attempt to subtract with overflow` at `depth` 0, from two tests running in parallel —
    /// and a real `activate_profile` on one ledger beside a `sync` on another is the same shape,
    /// which a test only made visible.
    static HELD_LOCKS: RefCell<Vec<PathBuf>> = const { RefCell::new(Vec::new()) };
}

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
    if HELD_LOCKS.with(|held| held.borrow().iter().any(|p| p == &path)) {
        // This thread already owns the file lock for this ledger: the caller is nested inside
        // the run that took it, so it must not wait on itself.
        return run();
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

    HELD_LOCKS.with(|held| held.borrow_mut().push(path.clone()));
    let out = run();
    HELD_LOCKS.with(|held| {
        let mut held = held.borrow_mut();
        if let Some(pos) = held.iter().rposition(|p| p == &path) {
            held.remove(pos);
        }
    });
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
        // A unit is a directory in every materialized channel except pi's agents, which are flat
        // `.md` files under `~/.pi/agent/agents/` (pi_settings.rs, `pi_agents_root`).
        // `remove_dir_all` on a file fails with ENOTDIR, and the `?` here used to skip the ledger
        // update below — so the Pack stayed recorded, the files stayed on disk and live to
        // pi-subagents, and every agent file reported an error instead. Measured 2026-10-13:
        // `tools/packs-pi remove writing` left eight academic-writing agents in place.
        let outcome = if destination.is_dir() {
            fs::remove_dir_all(&destination)
        } else {
            fs::remove_file(&destination)
        };
        outcome.map_err(|e| format!("{}: {e}", destination.display()))?;
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
        let sources = held_sources(config)?;
        let held: BTreeSet<String> = sources.keys().cloned().collect();
        let (units, held_here): (Vec<Unit>, Vec<Unit>) = pack_units(config, pack)?
            .into_iter()
            .partition(|u| !hold_matches(&held, &format!("{pack}/{}", u.key)));
        let mut state = read_state(config)?;
        if !state.packs.contains_key(pack) {
            return Err(format!("{pack}: not deployed; deploy it first"));
        }
        // The recorded profile's selection bounds what sync refreshes. A unit it leaves out is
        // stale here, so a unit an earlier unfiltered sync wrote is removed again.
        let units: Vec<Unit> = match recorded_profile_units(config, &state)?
            .and_then(|p| p.units.get(pack).cloned())
        {
            Some(keys) => units
                .into_iter()
                .filter(|u| keys.contains(&u.key))
                .collect(),
            None => units,
        };
        let desired: BTreeSet<String> = units.iter().map(|u| u.key.clone()).collect();
        let mut messages = held_messages(pack, &config.adapter, &held_here, &sources);
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

        // A held unit that is already recorded stays recorded: the hold means "never write this",
        // not "delete what is there". doctor reports a held-and-deployed unit as a violation.
        let stale: Vec<String> = state.packs[pack]
            .skills
            .keys()
            .filter(|k| !desired.contains(*k) && !hold_matches(&held, &format!("{pack}/{k}")))
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

/// `accept`'s mirror: replace a drifted destination with the Pack's own text, and re-record it.
/// The Pack is never written.
///
/// `accept` was the only way out of a drifted cell, and it imports the destination into the Pack
/// first. That is right when the destination holds an edit worth keeping, and wrong when the
/// destination is simply behind — the case that put an old table rule into Claude Code's copy of
/// `design/ui-craftsmanship`. The destination is the only copy of its bytes, so it is kept aside
/// before anything is replaced, and the returned line names where it went.
pub fn discard_unit(config: &DeployConfig, pack: &str, unit: &Unit) -> Result<String, String> {
    with_state_lock(config, || {
        let mut state = read_state(config)?;
        if !state
            .packs
            .get(pack)
            .is_some_and(|p| p.skills.contains_key(&unit.key))
        {
            return Err(format!(
                "{}: not deployed from {pack}; nothing to discard",
                unit.key
            ));
        }
        if !unit.destination.exists() {
            return Err(format!(
                "{}: {} does not exist",
                unit.key,
                unit.destination.display()
            ));
        }
        // Validate the Pack's side before it lands: an adapter that requires a file the source
        // no longer carries would otherwise write a copy its harness cannot read.
        let files = engine::desired_files(config, pack, unit)?;
        validate_unit(config, &files, unit, &format!("{pack}/{}", unit.key))?;
        let (stage, digest) = materialize_stage(config, pack, unit)?;
        if digest_destination(config, &unit.destination)? == digest {
            // Nothing to replace, but a stale ledger still needs the Pack's digest.
            let _ = fs::remove_dir_all(&stage);
            record_unit(config, &mut state, pack, unit, &digest);
            write_state(config, &state)?;
            return Ok(format!("= {} (already current)", unit.key));
        }
        let kept = match keep_aside(config, pack, unit) {
            Ok(kept) => kept,
            Err(e) => {
                let _ = fs::remove_dir_all(&stage);
                return Err(e);
            }
        };
        replace_atomically(config, &stage, &unit.destination, unit.only_file.is_some());
        let _ = fs::remove_dir_all(&stage);
        // `replace_atomically` reports nothing: a failed rename leaves the old copy where it was.
        // Recording the Pack's digest over that copy would hide the drift for good.
        if digest_destination(config, &unit.destination)? != digest {
            return Err(format!(
                "{}: the replacement did not land; the ledger is unchanged. The copy is kept at {}",
                unit.key,
                kept.display()
            ));
        }
        record_unit(config, &mut state, pack, unit, &digest);
        write_state(config, &state)?;
        Ok(format!(
            "✓ {} re-recorded from the Pack; the replaced copy is kept at {}",
            unit.key,
            kept.display()
        ))
    })
}

/// What a discard would replace, add and remove at the destination, named per file. The verb
/// prints it before asking for `--force`, so the operator sees what the copy holds that the Pack
/// does not, and what the Pack holds that the copy does not.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Replacement {
    pub replaced: Vec<String>,
    pub added: Vec<String>,
    pub removed: Vec<String>,
}

pub fn replacement_plan(
    config: &DeployConfig,
    pack: &str,
    unit: &Unit,
) -> Result<Replacement, String> {
    let wanted = engine::desired_files(config, pack, unit)?;
    let mut plan = Replacement::default();
    if !unit.destination.is_dir() {
        // A single-file unit: the destination is the file itself, so the only question is whether
        // its bytes differ from the Pack's.
        for (rel, file) in &wanted {
            if super::file_digest(&file.absolute_path)? != super::file_digest(&unit.destination)? {
                plan.replaced.push(rel.clone());
            }
        }
        return Ok(plan);
    }
    let mut walked = Vec::new();
    super::walk(&unit.destination, "", &mut walked)?;
    let mut actual: BTreeSet<String> = walked.into_iter().collect();
    for (rel, file) in &wanted {
        if !actual.remove(rel) {
            plan.added.push(rel.clone());
        } else if super::file_digest(&file.absolute_path)?
            != super::file_digest(&unit.destination.join(rel))?
        {
            plan.replaced.push(rel.clone());
        }
    }
    plan.removed = actual.into_iter().collect();
    Ok(plan)
}

/// Copies the destination aside before a discard replaces it. It lives beside the ledger: outside
/// the Pack, and outside every harness's discovery root, so nothing scans it and nothing deploys it.
fn keep_aside(config: &DeployConfig, pack: &str, unit: &Unit) -> Result<PathBuf, String> {
    let base = config
        .state_file
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("discarded")
        .join(&config.adapter)
        .join(pack);
    fs::create_dir_all(&base).map_err(|e| format!("{}: {e}", base.display()))?;
    let stamp = time::now_iso().replace(':', "-");
    let kept = base.join(format!("{}-{stamp}", basename(&unit.destination)));
    copy_tree(&unit.destination, &kept)?;
    Ok(kept)
}

fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
    if from.is_dir() {
        fs::create_dir_all(to).map_err(|e| format!("{}: {e}", to.display()))?;
        for entry in fs::read_dir(from).map_err(|e| format!("{}: {e}", from.display()))? {
            let entry = entry.map_err(|e| format!("{}: {e}", from.display()))?;
            copy_tree(&entry.path(), &to.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        fs::copy(from, to)
            .map(|_| ())
            .map_err(|e| format!("{}: {e}", to.display()))
    }
}

pub fn deploy_pack(
    config: &DeployConfig,
    pack: &str,
    skill_subset: Option<&BTreeSet<String>>,
) -> Result<Vec<String>, String> {
    with_state_lock(config, || {
        let sources = held_sources(config)?;
        let held: BTreeSet<String> = sources.keys().cloned().collect();
        let (units, held_here): (Vec<Unit>, Vec<Unit>) = pack_units(config, pack)?
            .into_iter()
            .filter(|u| skill_subset.is_none_or(|s| !u.is_skill || s.contains(&u.key)))
            .partition(|u| !hold_matches(&held, &format!("{pack}/{}", u.key)));
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
        let mut messages = held_messages(pack, &config.adapter, &held_here, &sources);
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

/// Per-harness unit holds, from a `[unit_hold]` table: `<harness> = ["<pack>/<unit>"]`.
/// `profiles.toml` holds a unit on every machine; the overlay's machine.toml holds it on that
/// machine only. `held_sources` merges the two.
///
/// A held unit is never written by `deploy` or `sync` for that harness, and `tools/doctor` reads
/// the same table instead of reporting the unit as missing. It lives here and not in a Pack
/// manifest for the same reason a Pack's manifest never carried a harness veto: a
/// harness-specific rule does not belong in a manifest the schema keeps harness-neutral,
/// and this file is where operator decisions live.
///
/// A hold is read at mutation time rather than remembered from `use`, because `sync` — the verb
/// that writes without asking — has to honour it too.
pub type UnitHolds = BTreeMap<String, Vec<String>>;

/// The `[unit_hold]` key that names every harness, including ones the table does not mention.
///
/// Without it, one decision had to be written once per harness, and a harness that arrived later
/// inherited nothing: measured 2026-10-24, `knowledge/defuddle` was spelled out five times (claude,
/// pi, codex, antigravity, opencode), each copy added by hand when that harness appeared, and
/// copilot arrived on `full` with no line and no verdict. That is the same shape as the
/// protection-zones default `c196797b` removed on the same day, one table over.
pub const HOLD_ALL: &str = "*";

/// The shared holds: `profiles.toml`'s `[unit_hold]` under `root`.
pub fn read_unit_holds_at(root: &Path) -> Result<UnitHolds, String> {
    read_unit_holds_file(&root.join("profiles.toml"))
}

/// One file's `[unit_hold]` table. A missing file or table holds nothing; a malformed entry is an
/// error naming that file.
pub fn read_unit_holds_file(path: &Path) -> Result<UnitHolds, String> {
    let path = path.to_path_buf();
    if !path.is_file() {
        return Ok(UnitHolds::new());
    }
    let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let parsed: toml::Table = text
        .parse()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let Some(holds) = parsed.get("unit_hold").and_then(toml::Value::as_table) else {
        return Ok(UnitHolds::new());
    };
    let mut out = UnitHolds::new();
    for (harness, units) in holds {
        let list = units.as_array().ok_or_else(|| {
            format!(
                "{}: unit_hold.{harness} must be an array of \"<pack>/<unit>\" strings",
                path.display()
            )
        })?;
        let mut keys = Vec::new();
        for unit in list {
            let key = unit.as_str().ok_or_else(|| {
                format!(
                    "{}: unit_hold.{harness} entries must be strings",
                    path.display()
                )
            })?;
            // A hold naming no Pack reads as enforced while holding nothing, so it is an error
            // rather than a no-op.
            if !key.contains('/') {
                return Err(format!(
                    "{}: unit_hold.{harness} entry '{key}' must be \"<pack>/<unit>\"",
                    path.display()
                ));
            }
            keys.push(key.to_owned());
        }
        out.insert(harness.clone(), keys);
    }
    Ok(out)
}

/// The `<pack>/<unit>` keys held back from this deployment's harness: the `*` entries, plus the
/// ones written for this harness by name.
///
/// A per-harness entry only ADDS to the wildcard, so a harness cannot disagree with a decision
/// the table says applies everywhere. It does not need to: a unit held everywhere is held here.
pub fn held_units(config: &DeployConfig) -> Result<BTreeSet<String>, String> {
    Ok(held_sources(config)?.into_keys().collect())
}

/// The source label of the shared holds file.
pub const PROFILES_SOURCE: &str = "profiles.toml";

#[cfg(test)]
thread_local! {
    /// The machine manifest a test substitutes for the overlay's, so a test never reads the
    /// holds of the machine it runs on.
    static TEST_MACHINE_TOML: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn set_test_machine_toml(path: Option<PathBuf>) {
    TEST_MACHINE_TOML.with(|p| *p.borrow_mut() = path);
}

/// This machine's manifest in the overlay, resolved the way `tools/doctor` resolves it.
fn machine_toml_for(root: &Path) -> Option<PathBuf> {
    #[cfg(test)]
    {
        let _ = root;
        TEST_MACHINE_TOML.with(|p| p.borrow().clone())
    }
    #[cfg(not(test))]
    {
        let overlay = crate::doctor::overlay::resolve_overlay_root(root)?;
        Some(crate::doctor::overlay::resolve_machine_toml(root, &overlay.root).path)
    }
}

/// The keys held back from this deployment's harness, each with the file that holds it:
/// `profiles.toml` (shared by every machine) merged with this machine's manifest
/// (`[unit_hold]` in the overlay's machine.toml), which holds a unit on this machine only.
/// A key both files name reports `profiles.toml`.
pub fn held_sources(config: &DeployConfig) -> Result<BTreeMap<String, String>, String> {
    let mut out = BTreeMap::new();
    if let Some(path) = machine_toml_for(&config.axon_root) {
        let label = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "machine.toml".to_owned());
        let holds = read_unit_holds_file(&path)?;
        for key in holds_for_harness(&holds, &config.adapter) {
            out.insert(key, label.clone());
        }
    }
    let shared = read_unit_holds_at(&config.axon_root)?;
    for key in holds_for_harness(&shared, &config.adapter) {
        out.insert(key, PROFILES_SOURCE.to_owned());
    }
    Ok(out)
}

fn holds_for_harness(holds: &UnitHolds, harness: &str) -> BTreeSet<String> {
    let mut out: BTreeSet<String> = holds
        .get(HOLD_ALL)
        .map(|keys| keys.iter().cloned().collect())
        .unwrap_or_default();
    if let Some(keys) = holds.get(harness) {
        out.extend(keys.iter().cloned());
    }
    out
}

/// The file whose hold names this key, for a message; `profiles.toml` when none matches.
pub fn hold_source<'a>(sources: &'a BTreeMap<String, String>, key: &str) -> &'a str {
    sources
        .iter()
        .find(|(hold, _)| hold_matches(&BTreeSet::from([(*hold).clone()]), key))
        .map(|(_, source)| source.as_str())
        .unwrap_or(PROFILES_SOURCE)
}

/// A key in `<pack>/<unit>` form, whichever channel spelled it.
///
/// A Pack's skill is `<pack>/<skill>` to a materialized channel and `<pack>/skills/<skill>` to a
/// path-keyed registry channel (pi_settings.rs, `held_for`, which joins the Pack root). A hold
/// written once has to name both, so it is written in the unit-key form and every comparison is
/// made in that form.
fn unit_key_form(key: &str) -> String {
    match key.split_once('/') {
        Some((pack, rest)) => match rest.strip_prefix("skills/") {
            Some(unit) => format!("{pack}/{unit}"),
            None => key.to_owned(),
        },
        None => key.to_owned(),
    }
}

/// Both spellings of one hold key, for a channel that compares PATHS instead of unit keys.
///
/// `held_for` (pi_settings.rs) turns each key into one absolute path per Pack root and matches it
/// against what the registry holds, so a skill needs `skills/` in the middle of that path. Emitting
/// both = one hold line covers a materialized harness and a registry harness; the spelling that
/// names nothing simply matches nothing.
///
/// Only a two-segment key grows a second spelling: `<pack>/skills/pi-packages/accordion` is not a
/// path any channel holds, and inventing it would be a hold that reads as enforced and is not.
pub fn path_spellings(key: &str) -> Vec<String> {
    let mut out = vec![key.to_owned()];
    let parts: Vec<&str> = key.split('/').collect();
    if parts.len() == 2 && !key.ends_with('/') && parts[1] != "skills" {
        out.push(format!("{}/skills/{}", parts[0], parts[1]));
    }
    out
}

/// Whether a `[unit_hold]` entry names this unit, where the key is `<pack>/<unit>`.
///
/// An exact key names one unit. A key ending in `/` names everything under it, because one
/// intent has a different unit shape per harness: claude carries a Pack's `agents/` tree as a
/// single unit, while pi materializes that same tree as `agents/<file>.md` per file
/// (pi_settings.rs, `pi_agents_root`). Matching exactly meant the operator had to spell one
/// decision eight times for pi and once for claude — and got no warning for the eight, because a
/// hold that matches nothing looks identical to a hold that works. Measured 2026-10-13: with
/// `writing/agents/` held, claude dropped the tree and pi silently redeployed all eight files.
pub fn hold_matches(held: &BTreeSet<String>, key: &str) -> bool {
    let key = unit_key_form(key);
    held.iter().any(|hold| {
        let hold = unit_key_form(hold);
        match hold.strip_suffix('/') {
            Some(prefix) => key == hold || key.starts_with(&format!("{prefix}/")),
            None => key == hold,
        }
    })
}

/// The note a deploy or sync prints for each unit a hold kept out, so an absent unit reads as a
/// decision rather than as a failure.
fn held_messages(
    pack: &str,
    harness: &str,
    held: &[Unit],
    sources: &BTreeMap<String, String>,
) -> Vec<String> {
    held.iter()
        .map(|unit| {
            let key = format!("{pack}/{}", unit.key);
            let source = hold_source(sources, &key);
            format!("- {key} held back from {harness} ({source} [unit_hold])")
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

/// The unit keys a profile would actually deploy from one Pack: everything `deploy_pack` would
/// install for that subset, minus the units a hold covers.
///
/// The hold half is what makes this comparable to a ledger. A held unit is never deployed and
/// never recorded, so a check that compared the Pack's raw key list against the ledger would
/// report a mismatch on every run and redeploy forever.
fn profile_unit_keys(
    config: &DeployConfig,
    pack: &str,
    subset: Option<&BTreeSet<String>>,
) -> Result<BTreeSet<String>, String> {
    let held = held_units(config)?;
    Ok(pack_units(config, pack)?
        .into_iter()
        .filter(|u| subset.is_none_or(|s| !u.is_skill || s.contains(&u.key)))
        .filter(|u| !hold_matches(&held, &format!("{pack}/{}", u.key)))
        .map(|u| u.key)
        .collect())
}

/// The profile a ledger records, with the unit keys it selects from each of its Packs.
pub struct RecordedProfile {
    pub name: String,
    pub units: BTreeMap<String, BTreeSet<String>>,
}

/// What the profile recorded in `state` selects. `None` when no profile is recorded or
/// `profiles.toml` no longer names it, which leaves every Pack unnarrowed.
///
/// # Errors
/// `profiles.toml` does not parse, or the profile no longer resolves against the Packs.
pub fn recorded_profile_units(
    config: &DeployConfig,
    state: &State,
) -> Result<Option<RecordedProfile>, String> {
    let Some(name) = &state.profile else {
        return Ok(None);
    };
    let Some(profile) = read_profiles(config)?.into_iter().find(|p| &p.name == name) else {
        return Ok(None);
    };
    let subsets = resolve_profile_skills(config, &profile)?;
    let mut units = BTreeMap::new();
    for pack in resolve_profile_packs(config, &profile)? {
        let subset = subsets.get(&pack).cloned().flatten();
        let keys = profile_unit_keys(config, &pack, subset.as_ref())?;
        units.insert(pack, keys);
    }
    Ok(Some(RecordedProfile {
        name: name.clone(),
        units,
    }))
}

fn record_profile(config: &DeployConfig, name: &str) -> Result<(), String> {
    let mut state = read_state(config)?;
    if state.profile.as_deref() != Some(name) {
        state.profile = Some(name.to_owned());
        write_state(config, &state)?;
    }
    Ok(())
}

pub fn activate_profile(config: &DeployConfig, profile: &Profile) -> Result<Vec<String>, String> {
    with_state_lock(config, || {
        let target_packs: BTreeSet<String> = resolve_profile_packs(config, profile)?
            .into_iter()
            .collect();
        let subsets = resolve_profile_skills(config, profile)?;
        let mut state = read_state(config)?;
        let mut messages = vec![format!(
            "Activating profile '{}' — {}",
            profile.name, profile.description
        )];

        let current: Vec<String> = state.packs.keys().cloned().collect();
        let mut target: Vec<String> = target_packs.iter().cloned().collect();
        target.sort();

        // A profile's skill subset used to reach a Pack only on its FIRST deploy: this verb
        // compared destinations, never unit sets, so a harness that already carried the whole
        // Pack kept every skill the profile stopped naming and kept missing the ones it added —
        // and still reported "already current". Both directions are reconciled here.
        let mut narrowed: Vec<(String, BTreeSet<String>)> = Vec::new();
        for pack in &target {
            let Some(record) = state.packs.get(pack) else {
                continue;
            };
            let want =
                profile_unit_keys(config, pack, subsets.get(pack).cloned().flatten().as_ref())?;
            let have: BTreeSet<String> = record.skills.keys().cloned().collect();
            if want != have {
                narrowed.push((pack.clone(), want));
            }
        }

        let to_remove: Vec<String> = current
            .iter()
            .filter(|p| !target_packs.contains(*p))
            .cloned()
            .collect();
        let to_deploy: Vec<String> = target
            .iter()
            .filter(|p| match state.packs.get(*p) {
                None => true,
                // Re-deploy if the Pack needs narrowing to this profile's subset, or if any
                // owned destination is missing from disk.
                Some(record) => {
                    narrowed.iter().any(|(pack, _)| pack == *p)
                        || record
                            .skills
                            .keys()
                            .any(|k| !recorded_destination(config, p, k).exists())
                }
            })
            .cloned()
            .collect();

        if to_remove.is_empty() && to_deploy.is_empty() && narrowed.is_empty() {
            record_profile(config, &profile.name)?;
            messages.push("  → already current".to_owned());
            return Ok(messages);
        }
        if !narrowed.is_empty() {
            messages.push(String::new());
            messages.push(format!(
                "Narrowing {} pack(s) to the profile's skills:",
                narrowed.len()
            ));
            for (pack, want) in &narrowed {
                let surplus: Vec<String> = state
                    .packs
                    .get(pack)
                    .map(|r| {
                        r.skills
                            .keys()
                            .filter(|k| !want.contains(*k))
                            .cloned()
                            .collect()
                    })
                    .unwrap_or_default();
                for unit_name in surplus {
                    match remove_owned_unit(config, &mut state, pack, &unit_name) {
                        Ok(line) => messages.push(format!("  {line}")),
                        Err(e) => messages.push(format!("  ✗ {pack}/{unit_name}: {e}")),
                    }
                }
            }
            // `deploy_pack` below re-reads the ledger, so the removals have to reach disk first.
            write_state(config, &state)?;
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
        record_profile(config, &profile.name)?;
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

    /// Every profile in the repository's own `profiles.toml` resolves against the Packs in the
    /// same tree: every Pack it names exists, every skill it narrows to exists in that Pack, and
    /// every `except` names a Pack that is there.
    ///
    /// Nothing checked this. A profile naming a renamed skill failed only when the operator ran
    /// `use` — on the harness that is their daily driver — and the two wildcard profiles
    /// (`full`, `everything`) resolve whatever is on disk, so CI exercised none of the narrow
    /// ones. That is the shape `check-skill-metadata.sh` was written for, one file over: a rule
    /// nothing checks has already been broken. Added with `thesis` on 2026-10-24, which is the
    /// profile this test proves.
    #[test]
    fn every_shipped_profile_resolves_against_the_packs_in_this_tree() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let profiles = read_profiles_at(&root).expect("profiles.toml must parse");
        assert!(
            !profiles.is_empty(),
            "no profiles found under {}",
            root.display()
        );
        let config = DeployConfig {
            axon_root: root.clone(),
            pack_roots: None,
            destination: root.join("target/profile-check"),
            state_file: root.join("target/profile-check.json"),
            adapter: "claude".to_owned(),
            state_env_var: None,
            tree_convention: None,
            flat_file_convention: None,
            skip_manifest_skills: false,
            validate_adapter_files: None,
        };
        for profile in &profiles {
            let packs = resolve_profile_packs(&config, profile)
                .unwrap_or_else(|e| panic!("profile '{}': {e}", profile.name));
            let subsets = resolve_profile_skills(&config, profile)
                .unwrap_or_else(|e| panic!("profile '{}': {e}", profile.name));
            for (pack, subset) in &subsets {
                assert!(
                    packs.contains(pack),
                    "profile '{}' narrows '{pack}' but does not deploy it",
                    profile.name
                );
                assert!(
                    subset.is_some(),
                    "profile '{}' resolved an empty subset",
                    profile.name
                );
            }
        }
        // The one profile whose point is a skill that loads nowhere else: a typo here would put
        // academic-writing back to zero harnesses, which is the state `thesis` exists to end.
        let thesis = profiles.iter().find(|p| p.name == "thesis");
        let thesis = thesis.expect("the thesis profile is gone");
        assert_eq!(
            resolve_profile_packs(&config, thesis).unwrap(),
            vec!["writing".to_owned()]
        );
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

    #[test]
    fn a_unit_hold_scopes_to_its_harness_and_must_name_a_unit() {
        let root =
            std::env::temp_dir().join(format!("sjel-harnesses-holds-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("profiles.toml"),
            "[unit_hold]\nclaude = [\"harness/agents/\"]\n",
        )
        .unwrap();

        let holds = read_unit_holds_at(&root).unwrap();
        assert_eq!(holds["claude"], vec!["harness/agents/".to_owned()]);
        assert!(!holds.contains_key("pi"), "a hold names one harness");

        // An entry that names no unit is an error, not a hold that silently holds nothing.
        fs::write(
            root.join("profiles.toml"),
            "[unit_hold]\nclaude = [\"harness\"]\n",
        )
        .unwrap();
        let err = read_unit_holds_at(&root).unwrap_err();
        assert!(err.contains("<pack>/<unit>"), "{err}");

        // No table, no holds: a repository that declares none behaves as it did before.
        fs::write(root.join("profiles.toml"), "[[profile]]\nname = \"p\"\n").unwrap();
        assert!(read_unit_holds_at(&root).unwrap().is_empty());
        let _ = fs::remove_dir_all(&root);
    }

    /// A machine's own `[unit_hold]` adds to the shared one: the effective set is the union, each
    /// key reports the file that holds it, and a malformed machine entry names machine.toml.
    #[test]
    fn a_unit_hold_in_machine_toml_merges_with_profiles_toml() {
        let root = std::env::temp_dir().join(format!(
            "sjel-harnesses-machine-holds-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("repo")).unwrap();
        fs::create_dir_all(root.join("overlay/config")).unwrap();
        fs::write(
            root.join("repo/profiles.toml"),
            "[unit_hold]\n\"*\" = [\"demo/gamma\"]\n",
        )
        .unwrap();
        let machine = root.join("overlay/config/machine.toml");
        fs::write(
            &machine,
            "os = \"linux\"\n\n[unit_hold]\npi = [\"demo/extensions/keys\", \"demo/gamma\"]\nclaude = [\"demo/beta\"]\n",
        )
        .unwrap();
        set_test_machine_toml(Some(machine.clone()));
        let mut config = DeployConfig {
            axon_root: root.join("repo"),
            pack_roots: None,
            destination: root.join("dest"),
            state_file: root.join("state.json"),
            adapter: "pi".to_owned(),
            state_env_var: None,
            tree_convention: None,
            flat_file_convention: None,
            skip_manifest_skills: false,
            validate_adapter_files: None,
        };

        let sources = held_sources(&config).unwrap();
        assert_eq!(
            sources.keys().cloned().collect::<Vec<_>>(),
            vec!["demo/extensions/keys".to_owned(), "demo/gamma".to_owned()],
            "the union of both files, scoped to pi"
        );
        assert_eq!(
            hold_source(&sources, "demo/extensions/keys"),
            "machine.toml"
        );
        assert_eq!(
            hold_source(&sources, "demo/gamma"),
            "profiles.toml",
            "a key both files hold reports the shared file"
        );
        assert!(hold_matches(
            &held_units(&config).unwrap(),
            "demo/extensions/keys"
        ));

        config.adapter = "claude".to_owned();
        let held = held_units(&config).unwrap();
        assert!(hold_matches(&held, "demo/beta"));
        assert!(hold_matches(&held, "demo/gamma"));
        assert!(!hold_matches(&held, "demo/extensions/keys"));

        fs::write(&machine, "[unit_hold]\npi = [\"demo\"]\n").unwrap();
        let err = held_units(&config).unwrap_err();
        assert!(err.contains("machine.toml"), "{err}");
        assert!(err.contains("<pack>/<unit>"), "{err}");

        // No machine manifest: only the shared holds apply.
        set_test_machine_toml(Some(root.join("overlay/config/absent.toml")));
        assert_eq!(
            held_units(&config).unwrap(),
            ["demo/gamma".to_owned()].into_iter().collect()
        );
        set_test_machine_toml(None);
        let _ = fs::remove_dir_all(&root);
    }

    /// One decision, every harness. `[unit_hold]` is keyed per harness, so a decision that applies
    /// everywhere had to be written once per harness, and a harness arriving later inherited
    /// nothing: measured 2026-10-24, `knowledge/defuddle` stood in five lists, each copy added by
    /// hand when its harness appeared, and copilot -- added that morning -- had no entry at all.
    ///
    /// The spellings are asserted in the same test because a wildcard that covers four harnesses
    /// and silently misses the fifth is the failure it exists to prevent: a materialized channel
    /// names a skill `demo/gamma`, a path-keyed registry channel names it `demo/skills/gamma`.
    #[test]
    fn a_wildcard_hold_covers_a_harness_the_table_never_names() {
        let root =
            std::env::temp_dir().join(format!("sjel-harnesses-hold-all-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let pack = root.join("repo/Packs/demo");
        for name in ["alpha", "beta", "gamma"] {
            fs::create_dir_all(pack.join("skills").join(name)).unwrap();
            fs::write(
                pack.join("skills").join(name).join("SKILL.md"),
                format!("---\nname: {name}\ndescription: does a thing\n---\n\nbody\n"),
            )
            .unwrap();
        }
        fs::write(
            pack.join("pack.toml"),
            "name = \"demo\"\ndescription = \"test\"\nskills = [\"alpha\", \"beta\", \"gamma\"]\n",
        )
        .unwrap();
        fs::write(
            root.join("repo/profiles.toml"),
            "[unit_hold]\n\"*\" = [\"demo/gamma\"]\nclaude = [\"demo/beta\"]\n",
        )
        .unwrap();
        let mut config = DeployConfig {
            axon_root: root.join("repo"),
            pack_roots: None,
            destination: root.join("dest"),
            state_file: root.join("state.json"),
            adapter: "opencode".to_owned(),
            state_env_var: None,
            tree_convention: None,
            flat_file_convention: None,
            skip_manifest_skills: false,
            validate_adapter_files: None,
        };

        // A harness the table never names still carries the wildcard, and nothing else.
        let held = held_units(&config).unwrap();
        assert_eq!(held, ["demo/gamma".to_owned()].into_iter().collect());
        assert!(hold_matches(&held, "demo/gamma"));
        assert!(
            hold_matches(&held, "demo/skills/gamma"),
            "the path spelling is the same hold"
        );
        assert!(
            !hold_matches(&held, "demo/beta"),
            "another harness's entry is not this harness's"
        );

        deploy_pack(&config, "demo", None).unwrap();
        assert!(config.destination.join("alpha").exists());
        assert!(
            config.destination.join("beta").exists(),
            "opencode does not hold beta"
        );
        assert!(
            !config.destination.join("gamma").exists(),
            "the wildcard holds gamma on a harness that never named it"
        );

        // A harness that names a unit itself takes the union: a named entry adds to the wildcard.
        config.adapter = "claude".to_owned();
        config.destination = root.join("dest-claude");
        config.state_file = root.join("state-claude.json");
        let held = held_units(&config).unwrap();
        assert_eq!(held.len(), 2);
        assert!(hold_matches(&held, "demo/beta"));
        assert!(hold_matches(&held, "demo/gamma"));
        let _ = fs::remove_dir_all(&root);
    }

    /// A hold names both spellings of a skill, and only one spelling of anything else: inventing
    /// `harness/skills/pi-packages/accordion` would be a hold that reads as enforced and matches
    /// nothing, which is the failure mode `hold_matches` exists to make impossible.
    #[test]
    fn a_hold_names_both_spellings_of_a_skill_and_neither_of_a_deeper_path() {
        assert_eq!(
            path_spellings("knowledge/defuddle"),
            vec![
                "knowledge/defuddle".to_owned(),
                "knowledge/skills/defuddle".to_owned()
            ]
        );
        assert_eq!(
            path_spellings("harness/pi-packages/accordion"),
            vec!["harness/pi-packages/accordion".to_owned()]
        );
        assert_eq!(
            path_spellings("writing/agents/"),
            vec!["writing/agents/".to_owned()]
        );
    }

    /// A held unit is not written by deploy, is reported as held, and stays out of the ledger —
    /// while a sibling unit in the same Pack still deploys. sync honours the same hold and
    /// leaves an already-recorded held unit alone, so doctor can report an override rather than
    /// sync silently undoing one.
    /// One hold line, two unit shapes. claude carries a Pack's `agents/` tree as a single unit;
    /// pi materializes the same tree as `agents/<file>.md` per file. Both are named by one
    /// `writing/agents/` entry. The bug this closes: with that entry in place, claude dropped the
    /// tree and pi silently redeployed all eight files, and nothing reported the difference
    /// because a hold that matches nothing looks exactly like a hold that works.
    /// A profile's `skills` map is a subset, and it has to hold on a harness that is already
    /// deployed: `activate_profile` used to compare destinations only, so a Pack deployed whole
    /// kept the skill the profile stopped naming — and kept missing one the profile added — while
    /// still reporting "already current". Both directions are asserted here, and a held skill is
    /// checked in the same fixture because a hold is the one unit a ledger must never hold.
    #[test]
    fn activating_a_profile_narrows_a_pack_that_is_already_deployed() {
        let root =
            std::env::temp_dir().join(format!("sjel-harnesses-narrow-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let pack = root.join("repo/Packs/demo");
        for name in ["alpha", "beta", "gamma"] {
            fs::create_dir_all(pack.join("skills").join(name)).unwrap();
            fs::write(
                pack.join("skills").join(name).join("SKILL.md"),
                format!("---\nname: {name}\ndescription: does a thing\n---\n\nbody\n"),
            )
            .unwrap();
        }
        fs::write(
            pack.join("pack.toml"),
            "name = \"demo\"\ndescription = \"test\"\nskills = [\"alpha\", \"beta\", \"gamma\"]\n",
        )
        .unwrap();
        fs::write(
            root.join("repo/profiles.toml"),
            "[unit_hold]\nclaude = [\"demo/gamma\"]\n\n[[profile]]\nname = \"narrow\"\ndescription = \"one skill\"\npacks = [\"demo\"]\nskills = { demo = [\"alpha\"] }\n",
        )
        .unwrap();
        let config = DeployConfig {
            axon_root: root.join("repo"),
            pack_roots: None,
            destination: root.join("dest"),
            state_file: root.join("state.json"),
            adapter: "claude".to_owned(),
            state_env_var: None,
            tree_convention: None,
            flat_file_convention: None,
            skip_manifest_skills: false,
            validate_adapter_files: None,
        };
        deploy_pack(&config, "demo", None).unwrap();
        assert!(config.destination.join("beta").exists());

        let profile = read_profiles(&config)
            .unwrap()
            .into_iter()
            .find(|p| p.name == "narrow")
            .unwrap();
        let messages = activate_profile(&config, &profile).unwrap();
        assert!(
            messages.iter().any(|m| m.contains("Narrowing 1 pack")),
            "{messages:?}"
        );
        assert!(
            config.destination.join("alpha").exists(),
            "the named skill stays"
        );
        assert!(
            !config.destination.join("beta").exists(),
            "the skill the profile stopped naming must leave the destination"
        );
        let state = read_state(&config).unwrap();
        assert_eq!(
            state.packs["demo"].skills.keys().collect::<Vec<_>>(),
            vec!["alpha"],
            "the ledger must match the destination, or the next run reports a phantom drift"
        );

        // Re-activating the same profile is a no-op: the narrowing converged.
        let messages = activate_profile(&config, &profile).unwrap();
        assert!(
            messages.iter().any(|m| m.contains("already current")),
            "{messages:?}"
        );
        let _ = fs::remove_dir_all(&root);
    }

    /// `sync` used to install every skill of a Pack, so after `use` narrowed one it put the left-out
    /// skills straight back. It now refreshes what the recorded profile selects, and removes a
    /// left-out unit an earlier unfiltered sync wrote.
    #[test]
    fn sync_keeps_a_pack_to_the_recorded_profile() {
        let root = std::env::temp_dir().join(format!(
            "sjel-harnesses-sync-profile-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let pack = root.join("repo/Packs/demo");
        for name in ["alpha", "beta"] {
            fs::create_dir_all(pack.join("skills").join(name)).unwrap();
            fs::write(
                pack.join("skills").join(name).join("SKILL.md"),
                format!("---\nname: {name}\ndescription: does a thing\n---\n\nbody\n"),
            )
            .unwrap();
        }
        fs::write(
            pack.join("pack.toml"),
            "name = \"demo\"\ndescription = \"test\"\nskills = [\"alpha\", \"beta\"]\n",
        )
        .unwrap();
        fs::write(
            root.join("repo/profiles.toml"),
            "[[profile]]\nname = \"narrow\"\ndescription = \"one skill\"\npacks = [\"demo\"]\nskills = { demo = [\"alpha\"] }\n",
        )
        .unwrap();
        let config = DeployConfig {
            axon_root: root.join("repo"),
            pack_roots: None,
            destination: root.join("dest"),
            state_file: root.join("state.json"),
            adapter: "claude".to_owned(),
            state_env_var: None,
            tree_convention: None,
            flat_file_convention: None,
            skip_manifest_skills: false,
            validate_adapter_files: None,
        };
        let profile = read_profiles(&config).unwrap().remove(0);
        activate_profile(&config, &profile).unwrap();
        assert_eq!(
            read_state(&config).unwrap().profile.as_deref(),
            Some("narrow")
        );

        sync_pack(&config, "demo").unwrap();
        assert!(config.destination.join("alpha").exists());
        assert!(
            !config.destination.join("beta").exists(),
            "sync must not add a skill the profile leaves out"
        );

        // A whole-Pack deploy puts beta back; the next sync converges on the profile again.
        deploy_pack(&config, "demo", None).unwrap();
        assert!(config.destination.join("beta").exists());
        sync_pack(&config, "demo").unwrap();
        assert!(!config.destination.join("beta").exists());
        let state = read_state(&config).unwrap();
        assert_eq!(
            state.profile.as_deref(),
            Some("narrow"),
            "sync keeps the record"
        );
        assert_eq!(
            state.packs["demo"].skills.keys().collect::<Vec<_>>(),
            vec!["alpha"]
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_trailing_slash_hold_names_every_unit_under_it() {
        let held: BTreeSet<String> = [
            "writing/agents/".to_owned(),
            "harness/pi-packages/accordion".to_owned(),
        ]
        .into_iter()
        .collect();

        // The tree shape: the whole `agents/` directory is one unit.
        assert!(hold_matches(&held, "writing/agents/"));
        // The per-file shape: the same directory, one unit per agent file.
        assert!(hold_matches(
            &held,
            "writing/agents/academic-writing-layout-auditor.md"
        ));
        // A sibling in the same Pack is not swept up by the prefix.
        assert!(!hold_matches(&held, "writing/skills/human-writing"));
        // The prefix stops at a path boundary, not mid-name.
        assert!(!hold_matches(&held, "writing/agents-extra/file.md"));
        // A key with no trailing slash stays an exact match.
        assert!(hold_matches(&held, "harness/pi-packages/accordion"));
        assert!(!hold_matches(
            &held,
            "harness/pi-packages/accordion/extension"
        ));
    }

    #[test]
    fn a_held_unit_is_never_deployed_and_never_enters_the_ledger() {
        let root = std::env::temp_dir().join(format!("sjel-harnesses-hold-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let pack = root.join("repo/Packs/demo");
        fs::create_dir_all(pack.join("skills/alpha")).unwrap();
        fs::create_dir_all(pack.join("agents")).unwrap();
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
        fs::write(
            pack.join("agents/Explore.md"),
            "---\nname: explore\n---\n\nbody\n",
        )
        .unwrap();
        fs::write(
            root.join("repo/profiles.toml"),
            "[unit_hold]\nclaude = [\"demo/agents/\"]\n",
        )
        .unwrap();

        let agents_dest = root.join("agents-dest");
        let config = DeployConfig {
            axon_root: root.join("repo"),
            pack_roots: None,
            destination: root.join("dest"),
            state_file: root.join("state.json"),
            adapter: "claude".to_owned(),
            state_env_var: None,
            tree_convention: Some(crate::harnesses::engine::TreeConvention {
                source_dir: "agents".to_owned(),
                destination_root: agents_dest.clone(),
            }),
            flat_file_convention: None,
            skip_manifest_skills: false,
            validate_adapter_files: None,
        };

        let messages = deploy_pack(&config, "demo", None).unwrap();
        assert!(
            config.destination.join("alpha").exists(),
            "the skill in the same Pack still deploys"
        );
        assert!(
            !agents_dest.join("demo").exists(),
            "the held agents/ tree must not be written"
        );
        let state = read_state(&config).unwrap();
        assert!(state.packs["demo"].skills.contains_key("alpha"));
        assert!(
            !state.packs["demo"].skills.contains_key("agents/"),
            "a held unit must not enter the ledger: {:?}",
            state.packs["demo"].skills.keys().collect::<Vec<_>>()
        );
        assert!(
            messages.iter().any(|m| m.contains("held back from claude")),
            "{messages:?}"
        );

        let mut state = read_state(&config).unwrap();
        let record = state.packs["demo"].skills["alpha"].clone();
        state
            .packs
            .get_mut("demo")
            .unwrap()
            .skills
            .insert("agents/".to_owned(), record);
        write_state(&config, &state).unwrap();
        let messages = sync_pack(&config, "demo").unwrap();
        assert!(
            messages.iter().any(|m| m.contains("held back from claude")),
            "{messages:?}"
        );
        let state = read_state(&config).unwrap();
        assert!(
            state.packs["demo"].skills.contains_key("agents/"),
            "sync must not silently delete a unit a hold covers"
        );
        let _ = fs::remove_dir_all(&root);
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

#[cfg(test)]
mod discard_tests {
    use super::*;
    use crate::harnesses::engine::{get_statuses, pack_units, SkillStatus};

    /// A deployed `demo/alpha`, with the Pack and the ledger agreeing before each test edits it.
    fn deployed(name: &str) -> (PathBuf, DeployConfig) {
        let root = std::env::temp_dir().join(format!(
            "sjel-harnesses-discard-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let pack = root.join("repo/Packs/demo");
        fs::create_dir_all(pack.join("skills/alpha/references")).unwrap();
        fs::write(
            pack.join("pack.toml"),
            "name = \"demo\"\ndescription = \"test\"\nskills = [\"alpha\"]\n",
        )
        .unwrap();
        fs::write(
            pack.join("skills/alpha/SKILL.md"),
            "---\nname: alpha\ndescription: does a thing\n---\n\npack body\n",
        )
        .unwrap();
        fs::write(pack.join("skills/alpha/references/guide.md"), "guide\n").unwrap();
        let config = DeployConfig {
            axon_root: root.join("repo"),
            pack_roots: None,
            destination: root.join("dest"),
            state_file: root.join("state/claude.json"),
            adapter: "claude".to_owned(),
            state_env_var: None,
            tree_convention: None,
            flat_file_convention: None,
            skip_manifest_skills: false,
            validate_adapter_files: None,
        };
        deploy_pack(&config, "demo", None).unwrap();
        (root, config)
    }

    fn alpha(config: &DeployConfig) -> Unit {
        pack_units(config, "demo")
            .unwrap()
            .into_iter()
            .find(|u| u.key == "alpha")
            .unwrap()
    }

    fn status(config: &DeployConfig) -> SkillStatus {
        get_statuses(config, Some("demo")).unwrap()[0]
            .status
            .clone()
    }

    #[test]
    fn discard_puts_the_pack_text_back_and_keeps_the_copy_aside() {
        let (root, config) = deployed("restores");
        let dest = config.destination.join("alpha");
        fs::write(
            dest.join("SKILL.md"),
            "---\nname: alpha\ndescription: edited here\n---\n\nold table rule\n",
        )
        .unwrap();
        fs::write(dest.join("extra.md"), "only here\n").unwrap();
        assert_eq!(status(&config), SkillStatus::Drifted);

        let unit = alpha(&config);
        let plan = replacement_plan(&config, "demo", &unit).unwrap();
        assert_eq!(plan.replaced, vec!["SKILL.md"]);
        assert!(plan.added.is_empty());
        assert_eq!(plan.removed, vec!["extra.md"]);

        let line = discard_unit(&config, "demo", &unit).unwrap();
        assert!(line.contains("kept at"), "{line}");
        assert_eq!(
            fs::read_to_string(dest.join("SKILL.md")).unwrap(),
            fs::read_to_string(root.join("repo/Packs/demo/skills/alpha/SKILL.md")).unwrap(),
            "the destination takes the Pack's bytes"
        );
        assert!(!dest.join("extra.md").exists());
        assert_eq!(status(&config), SkillStatus::Current, "{line}");

        let kept = fs::read_dir(
            config
                .state_file
                .parent()
                .unwrap()
                .join("discarded/claude/demo"),
        )
        .unwrap()
        .flatten()
        .next()
        .unwrap()
        .path();
        assert_eq!(
            fs::read_to_string(kept.join("extra.md")).unwrap(),
            "only here\n",
            "the replaced edit survives beside the ledger"
        );
        assert!(fs::read_to_string(kept.join("SKILL.md"))
            .unwrap()
            .contains("old table rule"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn discard_of_a_current_copy_keeps_nothing_aside() {
        let (root, config) = deployed("current");
        let line = discard_unit(&config, "demo", &alpha(&config)).unwrap();
        assert_eq!(line, "= alpha (already current)");
        assert!(
            !config
                .state_file
                .parent()
                .unwrap()
                .join("discarded")
                .exists(),
            "nothing differed, so nothing is kept"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn discard_refuses_a_unit_the_ledger_does_not_hold() {
        let (root, config) = deployed("unowned");
        let mut state = read_state(&config).unwrap();
        state.packs.get_mut("demo").unwrap().skills.clear();
        write_state(&config, &state).unwrap();
        let err = discard_unit(&config, "demo", &alpha(&config)).unwrap_err();
        assert!(err.contains("nothing to discard"), "{err}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn the_plan_names_a_file_the_copy_lost() {
        let (root, config) = deployed("added");
        fs::remove_file(config.destination.join("alpha/references/guide.md")).unwrap();
        let plan = replacement_plan(&config, "demo", &alpha(&config)).unwrap();
        assert_eq!(plan.added, vec!["references/guide.md"]);
        assert!(
            plan.replaced.is_empty() && plan.removed.is_empty(),
            "{plan:?}"
        );
        let _ = fs::remove_dir_all(&root);
    }
}
