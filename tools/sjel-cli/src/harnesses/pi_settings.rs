//! The pi settings registry and its agent channel — `tools/packs-pi.ts`'s activation half,
//! ported 2026-10-04.
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
use super::engine::{self, read_pack_skills, read_state, DeployConfig, FlatFileConvention};
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
fn extensions_for_pack(config: &DeployConfig, pack: &str) -> Result<Vec<String>, String> {
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
fn packages_for_pack(config: &DeployConfig, pack: &str) -> Result<Vec<String>, String> {
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

fn canonical_path(path: &str) -> PathBuf {
    engine::resolve(&expand_home(path))
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
        let count = if deployed {
            mutate::sync_pack(&config, pack)?
        } else {
            mutate::deploy_pack(&config, pack, None)?
        };
        messages.push(format!("  → agents: {pack} ({} file(s))", count.len()));
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
    for pack in target.iter() {
        let subset = subsets.get(pack).cloned().flatten();
        let paths = paths_for_pack(skill_config, pack, subset.as_ref())?;
        let ext_paths = extensions_for_pack(skill_config, pack)?;
        let pkg_paths = packages_for_pack(skill_config, pack)?;
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
        packs.insert(
            pack.clone(),
            paths_for_pack(skill_config, pack, subset.as_ref())?,
        );
        ext_state.insert(pack.clone(), extensions_for_pack(skill_config, pack)?);
        pkg_state.insert(pack.clone(), packages_for_pack(skill_config, pack)?);
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
