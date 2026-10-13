//! The pi settings registry and its agent channel — the whole of `tools/packs-pi.ts`'s registry
//! half, ported 2026-10-04 (that file is deleted; `src/packs.rs` is its verb surface).
//!
//! pi is a MIXED delivery model. Skills, extensions and vendored packages are REGISTERED: the
//! Pack source stays where it is and `~/.pi/agent/settings.json` carries a path to it, so there
//! is no drift. Agent files are MATERIALIZED, because pi-subagents reads
//! `$PI_CODING_AGENT_DIR/agents/*.md` on disk and does not recurse — a flat destination shared by
//! every Pack, which is what the engine's flat-file convention is for. So this adapter drives two
//! ledgers: its own settings ledger, and the shared engine's ledger for the agent files.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::agentfile::translate_agent_for_pi;
use super::engine::{
    self, read_pack_skills, read_state, DeployConfig, FlatFileConvention, StatusRow,
};
use super::mutate::{self, Profile};

// ---- locations -----------------------------------------------------------------------------

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default())
}

fn expand_home(path: &str) -> PathBuf {
    if path == "~" {
        return home();
    }
    match path.strip_prefix("~/") {
        Some(rest) => home().join(rest),
        None => PathBuf::from(path),
    }
}

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var(name)
        .ok()
        .filter(|s| !s.is_empty())
        .map(|s| expand_home(&s))
}

fn state_home() -> PathBuf {
    env_path("XDG_STATE_HOME").unwrap_or_else(|| home().join(".local").join("state"))
}

pub fn settings_path() -> PathBuf {
    engine::resolve(
        &env_path("PI_SETTINGS_FILE")
            .unwrap_or_else(|| home().join(".pi").join("agent").join("settings.json")),
    )
}

pub fn state_path() -> PathBuf {
    engine::resolve(&env_path("SJEL_PI_STATE_FILE").unwrap_or_else(|| {
        state_home()
            .join("axon")
            .join("pack-deployments")
            .join("pi.json")
    }))
}

fn agents_state_path() -> PathBuf {
    engine::resolve(&env_path("SJEL_PI_AGENTS_STATE_FILE").unwrap_or_else(|| {
        state_home()
            .join("axon")
            .join("pack-deployments")
            .join("pi-agents.json")
    }))
}

/// Where pi-subagents looks for agent files: flat, no pack subdirectory, no recursion.
fn pi_agents_root() -> PathBuf {
    if let Some(dir) = env_path("SJEL_PI_AGENTS_DIR") {
        return engine::resolve(&dir);
    }
    let agent_dir =
        env_path("PI_CODING_AGENT_DIR").unwrap_or_else(|| home().join(".pi").join("agent"));
    engine::resolve(&agent_dir.join("agents"))
}

/// The agent channel's config: the general engine pointed at a flat destination, with manifest
/// skills switched off because this adapter registers those rather than copying them.
fn agents_config(skill_config: &DeployConfig) -> DeployConfig {
    let root = pi_agents_root();
    DeployConfig {
        axon_root: skill_config.axon_root.clone(),
        pack_roots: skill_config.pack_roots.clone(),
        destination: root.clone(),
        state_file: agents_state_path(),
        adapter: "pi".to_owned(),
        state_env_var: Some("SJEL_PI_AGENTS_STATE_FILE".to_owned()),
        tree_convention: None,
        flat_file_convention: Some(FlatFileConvention {
            source_dir: "agents".to_owned(),
            destination_root: root,
            transform: Some(translate_agent_for_pi),
        }),
        skip_manifest_skills: true,
        validate_adapter_files: None,
    }
}

// ---- the settings file ---------------------------------------------------------------------

fn read_settings() -> Result<serde_json::Map<String, serde_json::Value>, String> {
    let path = settings_path();
    if !path.is_file() {
        return Ok(serde_json::Map::new());
    }
    let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    match serde_json::from_str::<serde_json::Value>(&text) {
        Ok(serde_json::Value::Object(map)) => Ok(map),
        _ => Err(format!("{}: expected a JSON object", path.display())),
    }
}

fn write_settings(settings: &serde_json::Map<String, serde_json::Value>) -> Result<(), String> {
    let path = settings_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let temp = path.with_file_name(format!(
        "{}.tmp-{}",
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        std::process::id()
    ));
    let text = format!(
        "{}\n",
        serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?
    );
    fs::write(&temp, text).map_err(|e| format!("{}: {e}", temp.display()))?;
    let _ = fs::set_permissions(&temp, fs::Permissions::from_mode(0o600));
    fs::rename(&temp, &path).map_err(|e| format!("{}: {e}", path.display()))
}

// ---- the settings ledger -------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PiState {
    version: u32,
    #[serde(rename = "settingsPath")]
    settings_path: String,
    #[serde(default)]
    packs: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    extensions: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    packages: BTreeMap<String, Vec<String>>,
}

fn read_pi_state() -> Result<PiState, String> {
    let path = state_path();
    let settings = settings_path();
    if !path.is_file() {
        return Ok(PiState {
            version: 1,
            settings_path: settings.display().to_string(),
            packs: BTreeMap::new(),
            extensions: BTreeMap::new(),
            packages: BTreeMap::new(),
        });
    }
    let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let state: PiState = serde_json::from_str(&text).map_err(|_| {
        format!(
            "{}: malformed or belongs to another Pi settings file",
            path.display()
        )
    })?;
    if state.version != 1 || engine::resolve(Path::new(&state.settings_path)) != settings {
        return Err(format!(
            "{}: malformed or belongs to another Pi settings file",
            path.display()
        ));
    }
    Ok(state)
}

fn write_pi_state(state: &PiState) -> Result<(), String> {
    let path = state_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let temp = path.with_file_name(format!(
        "{}.tmp-{}",
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        std::process::id()
    ));
    let text = format!(
        "{}\n",
        serde_json::to_string_pretty(state).map_err(|e| e.to_string())?
    );
    fs::write(&temp, text).map_err(|e| format!("{}: {e}", temp.display()))?;
    let _ = fs::set_permissions(&temp, fs::Permissions::from_mode(0o600));
    fs::rename(&temp, &path).map_err(|e| format!("{}: {e}", path.display()))
}

// ---- what a Pack contributes ---------------------------------------------------------------

/// Skills a Pack contributes, as paths pi can load. Exactly one root must hold each skill.
fn paths_for_pack(
    config: &DeployConfig,
    pack: &str,
    subset: Option<&BTreeSet<String>>,
) -> Result<Vec<String>, String> {
    let skills = read_pack_skills(config, pack)?;
    let roots = config.pack_roots.clone().unwrap_or_default();
    let mut out = Vec::new();
    for skill in skills {
        if subset.is_some_and(|s| !s.contains(&skill)) {
            continue;
        }
        let matches: Vec<PathBuf> = roots
            .iter()
            .map(|root| root.join(pack).join("skills").join(&skill))
            .filter(|p| p.exists())
            .collect();
        if matches.len() != 1 {
            return Err(format!("{pack}/{skill}: source is missing or ambiguous"));
        }
        out.push(matches[0].display().to_string());
    }
    Ok(out)
}

/// Extensions a Pack carries. A top-level `*.ts` file is one extension; a DIRECTORY is one too
/// when it has an `index.ts`, for an extension whose entry imports sidecars it must keep
/// together. Sorted, so the settings ledger is stable across runs.
pub(super) fn extensions_for_pack(
    config: &DeployConfig,
    pack: &str,
) -> Result<Vec<String>, String> {
    let roots = config.pack_roots.clone().unwrap_or_default();
    let mut names: BTreeSet<String> = BTreeSet::new();
    for root in &roots {
        let dir = root.join(pack).join("extensions");
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path();
            if path.is_file() && name.ends_with(".ts") {
                names.insert(name);
            } else if path.is_dir() && path.join("index.ts").exists() {
                names.insert(format!("{name}/index.ts"));
            }
        }
    }
    let mut out = Vec::new();
    for name in names {
        let matches: Vec<PathBuf> = roots
            .iter()
            .map(|root| root.join(pack).join("extensions").join(&name))
            .filter(|p| p.exists())
            .collect();
        if matches.len() != 1 {
            return Err(format!(
                "{pack}/extensions/{name}: source is missing or ambiguous"
            ));
        }
        out.push(matches[0].display().to_string());
    }
    Ok(out)
}

/// Vendored pi packages a Pack carries, one directory each under `pi-packages/`. A directory is
/// a package only if its package.json declares a `pi` manifest, because that manifest is what pi
/// reads to find the entry points; without it pi would accept the path and load nothing.
pub(super) fn packages_for_pack(config: &DeployConfig, pack: &str) -> Result<Vec<String>, String> {
    let roots = config.pack_roots.clone().unwrap_or_default();
    let mut found = Vec::new();
    for root in &roots {
        let dir = root.join(pack).join("pi-packages");
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if !entry.path().is_dir() {
                continue;
            }
            let manifest_path = entry.path().join("package.json");
            if !manifest_path.is_file() {
                continue;
            }
            let manifest: serde_json::Value =
                serde_json::from_str(&fs::read_to_string(&manifest_path).unwrap_or_default())
                    .unwrap_or(serde_json::Value::Null);
            if !manifest.get("pi").is_some_and(|p| p.is_object()) {
                return Err(format!(
                    "{pack}/pi-packages/{}/package.json declares no 'pi' manifest, so pi would \
                     accept the path and load nothing from it",
                    entry.file_name().to_string_lossy()
                ));
            }
            found.push(entry.path().display().to_string());
        }
    }
    found.sort();
    Ok(found)
}

pub(super) fn canonical_path(path: &str) -> PathBuf {
    engine::resolve(&expand_home(path))
}

/// A settings entry as an absolute path. `~/` expands to HOME, and a relative entry resolves from
/// the settings file's own directory — `pi install <path>` writes that form (`~/.pi/agent` holds
/// the file), and pi resolves it there (docs/packages.md). Comparing the strings as written reads
/// a registered package as unregistered, and makes `prune_dead_owned_paths` run against the cwd.
pub(super) fn registered_path(entry: &str) -> PathBuf {
    match entry.strip_prefix("~/") {
        Some(rest) => engine::resolve(&home().join(rest)),
        None => {
            let expanded = expand_home(entry);
            if expanded.is_absolute() {
                engine::resolve(&expanded)
            } else {
                engine::resolve(&home().join(".pi").join("agent").join(expanded))
            }
        }
    }
}

/// The `[unit_hold]` entries for this harness, as `(key, absolute path)`.
///
/// A hold is written `<pack>/<path from the Pack root>` — `harness/pi-packages/accordion` —
/// because a path is the only form that names a pi unit: all three channels this adapter writes
/// (skills, extensions, vendored packages) are registered by path. `mutate::held_units` owns the
/// read, so `profiles.toml` stays the operator's file and this adapter only obeys it.
fn held_for(config: &DeployConfig) -> Result<Vec<(String, String)>, String> {
    let roots = config.pack_roots.clone().unwrap_or_default();
    let mut out = Vec::new();
    for key in mutate::held_units(config)? {
        for root in &roots {
            out.push((
                key.clone(),
                engine::resolve(&root.join(&key)).display().to_string(),
            ));
        }
    }
    Ok(out)
}

/// One channel's paths, with the held ones dropped. A hold removes the unit the Pack names and
/// nothing else, so the rest of that Pack still deploys.
fn without_held(paths: Vec<String>, held: &[(String, String)]) -> Vec<String> {
    paths
        .into_iter()
        .filter(|path| !is_held(path, held))
        .collect()
}

/// Whether a path is one a `[unit_hold]` entry names, compared the way the registry compares
/// paths (`canonical_path`), so `~/…` and an absolute path are one hold.
fn is_held(path: &str, held: &[(String, String)]) -> bool {
    let resolved = canonical_path(path).display().to_string();
    held.iter().any(|(_, held_path)| *held_path == resolved)
}

/// One line per held unit this Pack carries, so an absent unit reads as a decision rather than as
/// a failure — the same voice `mutate::held_messages` uses for the materialized harnesses.
fn held_notes(
    pack: &str,
    carried: &[String],
    held: &[(String, String)],
    harness: &str,
) -> Vec<String> {
    held.iter()
        .filter(|(key, _)| key.starts_with(&format!("{pack}/")))
        .filter(|(_, held_path)| {
            carried
                .iter()
                .any(|path| canonical_path(path).display().to_string() == *held_path)
        })
        .map(|(key, _)| format!("- {key} held back from {harness} (profiles.toml [unit_hold])"))
        .collect()
}

fn dedupe_canonical(paths: Vec<String>) -> Vec<String> {
    let mut seen: Vec<PathBuf> = Vec::new();
    let mut out = Vec::new();
    for path in paths {
        let key = canonical_path(&path);
        if !seen.contains(&key) {
            seen.push(key);
            out.push(path);
        }
    }
    out
}

// ---- the agent channel ---------------------------------------------------------------------

fn agent_units(config: &DeployConfig, pack: &str) -> Result<Vec<engine::Unit>, String> {
    engine::pack_units(config, pack)
}

fn agents_ledger_owns(config: &DeployConfig, pack: &str) -> bool {
    config.state_file.is_file() && read_state(config).is_ok_and(|s| s.packs.contains_key(pack))
}

/// Remove every agent file this adapter's ledger owns for a Pack. Returns the count.
fn remove_agents_for(config: &DeployConfig, pack: &str) -> Result<usize, String> {
    if !agents_ledger_owns(config, pack) {
        return Ok(0);
    }
    let count = read_state(config)?
        .packs
        .get(pack)
        .map(|r| r.skills.len())
        .unwrap_or(0);
    mutate::remove_pack(config, pack)?;
    Ok(count)
}

/// Move the agent channel with a profile activation. Deliberately NOT the engine's own
/// activate_profile: that resolves per-Pack skill subsets against pack_units(), and this config
/// deploys no skills, so every subset in profiles.toml would fail validation.
fn activate_agents_for_profile(
    skill_config: &DeployConfig,
    active_packs: &[String],
) -> Result<Vec<String>, String> {
    let config = agents_config(skill_config);
    let mut messages = Vec::new();
    let mut ledger_packs: Vec<String> = if config.state_file.is_file() {
        read_state(&config)?.packs.keys().cloned().collect()
    } else {
        Vec::new()
    };
    ledger_packs.sort();
    for pack in ledger_packs.iter().filter(|p| !active_packs.contains(p)) {
        let removed = remove_agents_for(&config, pack)?;
        if removed > 0 {
            messages.push(format!("  → agents: {pack} removed ({removed} file(s))"));
        }
    }
    for pack in active_packs {
        if agent_units(&config, pack)?.is_empty() {
            continue;
        }
        let deployed = agents_ledger_owns(&config, pack);
        let lines = if deployed {
            mutate::sync_pack(&config, pack)?
        } else {
            mutate::deploy_pack(&config, pack, None)?
        };
        // The engine's own lines, not a count of them. `sync_pack` returns one line per unit
        // moved AND one per unit a hold kept out, so reporting `lines.len()` as "file(s)" said
        // "8 file(s)" for a Pack whose eight files were all held back — the opposite of what
        // happened. Measured 2026-10-13, twice.
        messages.extend(lines.into_iter().map(|l| format!("  → agents: {l}")));
    }
    Ok(messages)
}

// ---- activation ----------------------------------------------------------------------------

/// Registry-model profile activation: the pi settings file IS the selection, so activating a
/// profile rewrites `settings.json`'s skills AND extensions to exactly the profile's set (per-Pack
/// skill subsets honoured) and drops ledger entries for Pack(s) the profile does not name.
pub fn activate_profile_on_pi(
    skill_config: &DeployConfig,
    profile_name: &str,
) -> Result<Vec<String>, String> {
    let profiles: Vec<Profile> = mutate::read_profiles(skill_config)?;
    let profile = profiles
        .iter()
        .find(|p| p.name == profile_name)
        .ok_or_else(|| format!("no such profile: '{profile_name}'"))?;
    let target: BTreeSet<String> = mutate::resolve_profile_packs(skill_config, profile)?
        .into_iter()
        .collect();
    let subsets = mutate::resolve_profile_skills(skill_config, profile)?;
    let mut state = read_pi_state()?;
    let mut settings = read_settings()?;
    let mut messages = vec![format!(
        "Activating profile '{}' — {}",
        profile.name, profile.description
    )];

    let mut active: Vec<String> = Vec::new();
    let mut skills: Vec<String> = Vec::new();
    let mut extensions: Vec<String> = Vec::new();
    let mut packages: Vec<String> = Vec::new();
    let held = held_for(skill_config)?;
    for pack in target.iter() {
        let subset = subsets.get(pack).cloned().flatten();
        let paths = paths_for_pack(skill_config, pack, subset.as_ref())?;
        let ext_paths = extensions_for_pack(skill_config, pack)?;
        let pkg_paths = packages_for_pack(skill_config, pack)?;
        let carried: Vec<String> = paths
            .iter()
            .chain(ext_paths.iter())
            .chain(pkg_paths.iter())
            .cloned()
            .collect();
        let paths = without_held(paths, &held);
        let ext_paths = without_held(ext_paths, &held);
        let pkg_paths = without_held(pkg_paths, &held);
        skills.extend(paths.iter().cloned());
        extensions.extend(ext_paths.iter().cloned());
        packages.extend(pkg_paths.iter().cloned());
        active.push(pack.clone());
        messages.push(format!(
            "  → {pack}: {} skill(s), {} extension(s), {} package(s)",
            paths.len(),
            ext_paths.len(),
            pkg_paths.len()
        ));
        messages.extend(held_notes(pack, &carried, &held, &skill_config.adapter));
    }
    let removed: String = state
        .packs
        .keys()
        .filter(|p| !active.contains(p))
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    if !removed.is_empty() {
        messages.push(format!("Removing Pack(s) not in profile: {removed}"));
    }
    messages.extend(activate_agents_for_profile(skill_config, &active)?);

    let skills = dedupe_canonical(skills);
    let extensions = dedupe_canonical(extensions);
    let packages = dedupe_canonical(packages);
    settings.insert("skills".to_owned(), serde_json::json!(skills));
    settings.insert("extensions".to_owned(), serde_json::json!(extensions));
    settings.insert("packages".to_owned(), serde_json::json!(packages));

    let mut packs = BTreeMap::new();
    let mut ext_state = BTreeMap::new();
    let mut pkg_state = BTreeMap::new();
    for pack in &active {
        let subset = subsets.get(pack).cloned().flatten();
        // The ledger records what was registered, so a held unit never enters it: otherwise
        // `remove` would later be asked to remove a path this adapter never wrote.
        packs.insert(
            pack.clone(),
            without_held(paths_for_pack(skill_config, pack, subset.as_ref())?, &held),
        );
        ext_state.insert(
            pack.clone(),
            without_held(extensions_for_pack(skill_config, pack)?, &held),
        );
        pkg_state.insert(
            pack.clone(),
            without_held(packages_for_pack(skill_config, pack)?, &held),
        );
    }
    state.packs = packs;
    state.extensions = ext_state;
    state.packages = pkg_state;

    write_settings(&settings)?;
    write_pi_state(&state)?;

    let skill_count = settings
        .get("skills")
        .and_then(|v| v.as_array())
        .map_or(0, Vec::len);
    let ext_count = settings
        .get("extensions")
        .and_then(|v| v.as_array())
        .map_or(0, Vec::len);
    let pkg_count = settings
        .get("packages")
        .and_then(|v| v.as_array())
        .map_or(0, Vec::len);
    messages.push(format!(
        "✓ profile '{}' active for pi: {} pack(s), {skill_count} skill(s), {ext_count} extension(s), \
         {pkg_count} package(s){}",
        profile.name,
        active.len(),
        if removed.is_empty() {
            String::new()
        } else {
            format!("; removed {removed}")
        }
    ));
    Ok(messages)
}

/// Registry-model profile activation, as the CLI's `use` verb drives it.
pub fn activate_profile(skill_config: &DeployConfig, name: &str) -> Result<Vec<String>, String> {
    activate_profile_on_pi(skill_config, name)
}

// ---- the CLI half --------------------------------------------------------------------------

/// One settings array, validated the way `stringArraySetting` did: absent is empty, anything
/// that is not an array of strings is an error rather than a silent skip.
fn setting_strings(
    settings: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<Vec<String>, String> {
    let bad = || {
        format!(
            "{}: {key} must be an array of strings",
            settings_path().display()
        )
    };
    match settings.get(key) {
        None => Ok(Vec::new()),
        Some(serde_json::Value::Array(items)) => items
            .iter()
            .map(|item| item.as_str().map(str::to_owned).ok_or_else(bad))
            .collect(),
        Some(_) => Err(bad()),
    }
}

pub(super) fn last_segment(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_string()
}

/// An extension's name in a status row: a directory extension is its own name, not `index.ts`.
pub(super) fn extension_label(path: &str) -> String {
    let parts: Vec<&str> = path.split('/').collect();
    let base = parts.last().copied().unwrap_or(path);
    let parent = parts
        .len()
        .checked_sub(2)
        .map(|index| parts[index])
        .unwrap_or_default();
    if base == "index.ts" && parent != "extensions" {
        parent.to_string()
    } else {
        base.to_string()
    }
}

/// Add each path to a settings array, replacing an equivalent entry in place rather than
/// appending a second spelling of it.
fn register_into(target: &mut Vec<String>, paths: &[String]) {
    for path in paths {
        match target
            .iter()
            .position(|existing| canonical_path(existing) == canonical_path(path))
        {
            None => target.push(path.clone()),
            Some(index) => {
                if target[index] != *path {
                    target[index] = path.clone();
                }
            }
        }
    }
}

/// Drop registration entries that point where this deployment owns but nothing exists. Both
/// conditions are required: the path must be gone from disk AND sit under one of this
/// deployment's Pack roots, so an npm source or an operator's own directory is never touched.
fn prune_dead_owned_paths(paths: Vec<String>, config: &DeployConfig) -> Vec<String> {
    let roots: Vec<String> = config
        .pack_roots
        .clone()
        .unwrap_or_default()
        .iter()
        .map(|root| format!("{}/", engine::resolve(root).display()))
        .collect();
    paths
        .into_iter()
        .filter(|path| {
            let absolute = registered_path(path);
            let ours = roots
                .iter()
                .any(|root| absolute.display().to_string().starts_with(root));
            !ours || absolute.exists()
        })
        .collect()
}

/// Every settings row, then every agent row — the shape `tools/packs-pi status` prints.
pub fn status_lines(config: &DeployConfig, packs: &[String]) -> Result<Vec<String>, String> {
    let selected: Vec<String> = if packs.is_empty() {
        engine::available_packs(config, true)?
    } else {
        packs.to_vec()
    };
    let settings = read_settings()?;
    let skills = setting_strings(&settings, "skills")?;
    let extensions = setting_strings(&settings, "extensions")?;
    let packages = setting_strings(&settings, "packages")?;
    let state = read_pi_state()?;

    let label = |registered: bool, owned: bool| {
        if !registered {
            "not-deployed"
        } else if owned {
            "current"
        } else {
            "selected-unmanaged"
        }
    };
    let owned = |channel: &BTreeMap<String, Vec<String>>, pack: &str, path: &str| {
        channel
            .get(pack)
            .is_some_and(|paths| paths.iter().any(|p| p == path))
    };

    // Read here, not enforced — status writes nothing — so a held unit prints as the decision it
    // is rather than as the "not-deployed" warning a reader would act on.
    let held = held_for(config).unwrap_or_default();
    let state_of = |path: &str, label: &'static str| -> &'static str {
        if is_held(path, &held) {
            "held"
        } else {
            label
        }
    };
    let mut out = Vec::new();
    for pack in &selected {
        for path in paths_for_pack(config, pack, None)? {
            out.push(format!(
                "{pack}/{}: {}",
                last_segment(&path),
                state_of(
                    &path,
                    label(skills.contains(&path), owned(&state.packs, pack, &path))
                )
            ));
        }
        for path in extensions_for_pack(config, pack)? {
            out.push(format!(
                "{pack}/extensions/{}: {}",
                extension_label(&path),
                state_of(
                    &path,
                    label(
                        extensions.contains(&path),
                        owned(&state.extensions, pack, &path)
                    )
                )
            ));
        }
        for path in packages_for_pack(config, pack)? {
            out.push(format!(
                "{pack}/pi-packages/{}: {}",
                last_segment(&path),
                state_of(
                    &path,
                    label(
                        packages.contains(&path),
                        owned(&state.packages, pack, &path)
                    )
                )
            ));
        }
    }
    out.extend(agent_status_lines(config, &selected)?);
    Ok(out)
}

/// Agent rows, in the same shape as the settings rows so one listing reads as one table. The
/// two channels answer different questions: the settings rows ask "is this path registered",
/// these ask "are the bytes at that flat destination the ones this Pack would write".
fn agent_status_lines(config: &DeployConfig, packs: &[String]) -> Result<Vec<String>, String> {
    Ok(agent_status_rows(config, packs)?
        .into_iter()
        .map(|row| {
            let detail = row
                .detail
                .as_deref()
                .map(|d| format!(" ({d})"))
                .unwrap_or_default();
            format!(
                "{}/{}: {}{detail}",
                row.pack,
                row.skill,
                row.status.as_str()
            )
        })
        .collect())
}

/// The agent channel as rows, so `tools/harnesses status` shows it beside the settings rows.
/// Without this the matrix reports a Pack `current` while the agent files it owns sit stale at
/// the flat destination, which is the drift that went unseen from 2026-10-04 to 2026-10-08: the
/// Rust port of `translate_agent_for_pi` changed the generated provenance header and nothing in
/// the matrix compared the bytes.
pub fn agent_status_rows(
    config: &DeployConfig,
    packs: &[String],
) -> Result<Vec<StatusRow>, String> {
    let agents = agents_config(config);
    let selected: Vec<String> = if packs.is_empty() {
        engine::available_packs(&agents, true)?
    } else {
        packs.to_vec()
    };
    let mut out = Vec::new();
    for pack in &selected {
        if agent_units(&agents, pack)?.is_empty() && !agents_ledger_owns(&agents, pack) {
            continue;
        }
        out.extend(engine::get_statuses(&agents, Some(pack))?);
    }
    Ok(out)
}

/// Register one or more Packs in `settings.json` and materialize their agent files.
pub fn deploy_packs(config: &DeployConfig, packs: &[String]) -> Result<Vec<String>, String> {
    if packs.is_empty() {
        return Err("deploy needs one or more Pack names".to_owned());
    }
    let mut settings = read_settings()?;
    let mut skills = setting_strings(&settings, "skills")?;
    let mut extensions = setting_strings(&settings, "extensions")?;
    let mut packages = setting_strings(&settings, "packages")?;
    let mut state = read_pi_state()?;
    let mut out = Vec::new();

    let held = held_for(config)?;
    for pack in packs {
        let paths = paths_for_pack(config, pack, None)?;
        let ext_paths = extensions_for_pack(config, pack)?;
        let pkg_paths = packages_for_pack(config, pack)?;
        let carried: Vec<String> = paths
            .iter()
            .chain(ext_paths.iter())
            .chain(pkg_paths.iter())
            .cloned()
            .collect();
        let paths = without_held(paths, &held);
        let ext_paths = without_held(ext_paths, &held);
        let pkg_paths = without_held(pkg_paths, &held);
        register_into(&mut skills, &paths);
        register_into(&mut extensions, &ext_paths);
        register_into(&mut packages, &pkg_paths);
        state.packs.insert(pack.clone(), paths.clone());
        state.extensions.insert(pack.clone(), ext_paths.clone());
        state.packages.insert(pack.clone(), pkg_paths.clone());
        out.push(format!(
            "✓ {pack}: {} skill(s), {} extension(s), {} package(s) selected for Pi",
            paths.len(),
            ext_paths.len(),
            pkg_paths.len()
        ));
        out.extend(held_notes(pack, &carried, &held, &config.adapter));
    }

    // Pi treats equivalent absolute and ~/ paths as the same source, so one path is kept; the
    // ledger is trimmed on the same terms so `remove` is never asked to remove what is gone.
    skills = dedupe_canonical(prune_dead_owned_paths(skills, config));
    extensions = dedupe_canonical(prune_dead_owned_paths(extensions, config));
    packages = dedupe_canonical(prune_dead_owned_paths(packages, config));
    for channel in [&mut state.packs, &mut state.extensions, &mut state.packages] {
        for paths in channel.values_mut() {
            *paths = prune_dead_owned_paths(paths.clone(), config);
        }
    }
    settings.insert("skills".to_owned(), serde_json::json!(skills));
    settings.insert("extensions".to_owned(), serde_json::json!(extensions));
    settings.insert("packages".to_owned(), serde_json::json!(packages));
    write_settings(&settings)?;
    write_pi_state(&state)?;
    out.extend(deploy_agents(config, packs)?);
    Ok(out)
}

/// Materialize the agent files of the named Packs. A Pack with no `agents/` directory is skipped
/// rather than reported, because that is the common case and not a fault.
fn deploy_agents(config: &DeployConfig, packs: &[String]) -> Result<Vec<String>, String> {
    let agents = agents_config(config);
    let mut out = Vec::new();
    for pack in packs {
        if agent_units(&agents, pack)?.is_empty() {
            if !agents_ledger_owns(&agents, pack) {
                continue;
            }
            // The Pack dropped its agents/ directory; take the deployed copies with it.
            let removed = remove_agents_for(&agents, pack)?;
            out.push(format!(
                "  {pack}: agents/ is gone from the Pack, removing {removed} file(s)"
            ));
            continue;
        }
        let deployed = agents_ledger_owns(&agents, pack);
        let count = if deployed {
            mutate::sync_pack(&agents, pack)?.len()
        } else {
            mutate::deploy_pack(&agents, pack, None)?.len()
        };
        out.push(format!(
            "  {pack}: {count} agent file(s) at {}",
            agents.destination.display()
        ));
    }
    Ok(out)
}

/// Drop one or more Packs from `settings.json` and remove the agent files they own.
pub fn remove_packs(config: &DeployConfig, packs: &[String]) -> Result<Vec<String>, String> {
    if packs.is_empty() {
        return Err("remove needs one or more Pack names".to_owned());
    }
    let mut settings = read_settings()?;
    let mut state = read_pi_state()?;
    let agents = agents_config(config);
    let mut remove_paths: BTreeSet<String> = BTreeSet::new();
    let mut remove_extensions: BTreeSet<String> = BTreeSet::new();
    let mut remove_packages: BTreeSet<String> = BTreeSet::new();
    let mut out = Vec::new();

    for pack in packs {
        // A Pack can be recorded in the skill ledger, the agent ledger, or both: they are separate
        // files with separate destinations, and one failed removal can leave one without the
        // other. Requiring the skill ledger here made such a Pack unremovable — its agent files
        // stayed on disk and live to pi-subagents, and the error named a Pack the caller could see
        // deployed. Either ledger owning it is enough to remove it. Measured 2026-10-13: a
        // `remove writing` whose agent step failed left exactly this state.
        if !state.packs.contains_key(pack) && !agents_ledger_owns(&agents, pack) {
            return Err(format!("{pack}: not owned by Axon Pi deployment"));
        }
        remove_paths.extend(state.packs.get(pack).cloned().unwrap_or_default());
        remove_extensions.extend(state.extensions.get(pack).cloned().unwrap_or_default());
        remove_packages.extend(state.packages.get(pack).cloned().unwrap_or_default());
        state.packs.remove(pack);
        state.extensions.remove(pack);
        state.packages.remove(pack);
        out.push(format!("✓ {pack}: removed from Pi selection"));
    }

    let keep = |paths: Vec<String>, drop: &BTreeSet<String>| -> Vec<String> {
        paths
            .into_iter()
            .filter(|path| !drop.contains(path))
            .collect()
    };
    let skills = keep(setting_strings(&settings, "skills")?, &remove_paths);
    let extensions = keep(
        setting_strings(&settings, "extensions")?,
        &remove_extensions,
    );
    let packages = keep(setting_strings(&settings, "packages")?, &remove_packages);
    settings.insert("skills".to_owned(), serde_json::json!(skills));
    settings.insert("extensions".to_owned(), serde_json::json!(extensions));
    settings.insert("packages".to_owned(), serde_json::json!(packages));
    write_settings(&settings)?;
    write_pi_state(&state)?;

    for pack in packs {
        let removed = remove_agents_for(&agents, pack)?;
        if removed > 0 {
            out.push(format!("  {pack}: {removed} agent file(s) removed"));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `<pack>/<unit>` hold written the way `profiles.toml` writes one reaches the path pi would
    /// register, that path is what all three channels filter on, and the note names the decision.
    /// The sibling package in the same Pack is untouched: a hold names a unit, not a Pack, and a
    /// hold scoped to another harness does not reach this one.
    #[test]
    fn a_unit_hold_keeps_one_vendored_package_out_of_pis_registry() {
        let root = std::env::temp_dir().join(format!("sjel-pi-hold-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let pack_root = root.join("Packs");
        let accordion = pack_root.join("harness/pi-packages/accordion");
        let subagents = pack_root.join("harness/pi-packages/pi-subagents");
        fs::create_dir_all(&accordion).unwrap();
        fs::create_dir_all(&subagents).unwrap();
        fs::write(
            root.join("profiles.toml"),
            "[unit_hold]\npi = [\"harness/pi-packages/accordion\"]\nclaude = [\"harness/agents/\"]\n",
        )
        .unwrap();
        let config = DeployConfig {
            axon_root: root.clone(),
            pack_roots: Some(vec![pack_root.clone()]),
            destination: root.join("dest"),
            state_file: root.join("state.json"),
            adapter: "pi".to_owned(),
            state_env_var: None,
            tree_convention: None,
            flat_file_convention: None,
            skip_manifest_skills: false,
            validate_adapter_files: None,
        };

        let held = held_for(&config).unwrap();
        assert_eq!(
            held,
            vec![(
                "harness/pi-packages/accordion".to_owned(),
                accordion.display().to_string()
            )],
            "the claude-only hold must not reach pi"
        );

        let carried = vec![
            accordion.display().to_string(),
            subagents.display().to_string(),
        ];
        assert_eq!(
            without_held(carried.clone(), &held),
            vec![subagents.display().to_string()],
            "the held package leaves, its sibling stays"
        );
        assert_eq!(
            held_notes("harness", &carried, &held, "pi"),
            vec![
                "- harness/pi-packages/accordion held back from pi (profiles.toml [unit_hold])"
                    .to_owned()
            ]
        );
        let _ = fs::remove_dir_all(&root);
    }
}
