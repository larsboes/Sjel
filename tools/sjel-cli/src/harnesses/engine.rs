//! The read half of tools/lib/pack-deploy.ts, ported 2026-10-02.
//!
//! `tools/harnesses list|status|drift` and the Pack sections of `tools/doctor` read the
//! deployment ledgers, hash the deployed trees and compare them with each Pack's source. The
//! mutating half — deploy, sync, remove, adopt, reconcile, the state lock — stays in
//! TypeScript for now, by decision: read verbs first, write verbs after their parity is
//! proven. So this file is a second reader of one format, not a second engine, and the two
//! meet at the ledger on disk (`tools/lib/pack-deploy.ts` defines it, `DIGEST_POLICY` below
//! names the same rule).
//!
//! What is deliberately NOT here, and why:
//!
//! - The mutation lock and `writeState`. Nothing here writes, so nothing here locks.
//! - `Bun.YAML` for SKILL.md frontmatter: two scalars are read by `frontmatter.rs` instead.
//! - The flat-file `transform`. No read verb reaches a config that sets one — pi's agent
//!   channel (`defaultPiAgentsDeployConfig`) is read by `tools/packs-pi status`, not here —
//!   so `DesiredFile::content` exists but no config populates it.
//!
//! Every error string is pack-deploy.ts's verbatim, because a StatusRow's `detail` is printed
//! by `tools/harnesses` and `tools/doctor`.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::os::unix::fs::MetadataExt as _;
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;
use sha2::{Digest as _, Sha256};

use super::frontmatter;

/// The digest policy a ledger row carries once generated Python artifacts are excluded.
pub const DIGEST_POLICY: &str = "exclude-python-generated-v1";

pub type Files = BTreeMap<String, DesiredFile>;

/// An adapter's extra validation of an assembled unit: codex checks `agents/openai.yaml`.
pub type AdapterValidator = fn(&Files, &str) -> Result<(), String>;

#[derive(Debug, Clone)]
pub struct DesiredFile {
    pub absolute_path: PathBuf,
    pub relative_path: String,
    pub mode: u32,
    /// Set only by a flat-file unit's transform; no read path sets it today.
    pub content: Option<Vec<u8>>,
}

#[derive(Debug, Clone)]
pub struct TreeConvention {
    pub source_dir: String,
    pub destination_root: PathBuf,
}

#[derive(Debug, Clone)]
pub struct FlatFileConvention {
    pub source_dir: String,
    pub destination_root: PathBuf,
}

#[derive(Debug, Clone)]
pub struct DeployConfig {
    pub axon_root: PathBuf,
    /// Ordered Pack roots; the first is the public source of truth.
    pub pack_roots: Option<Vec<PathBuf>>,
    pub destination: PathBuf,
    pub state_file: PathBuf,
    pub adapter: String,
    /// Env var named in the "state belongs to another destination" error.
    pub state_env_var: Option<String>,
    pub tree_convention: Option<TreeConvention>,
    pub flat_file_convention: Option<FlatFileConvention>,
    pub skip_manifest_skills: bool,
    /// An extra per-adapter validation of an assembled unit: codex's `agents/openai.yaml`.
    pub validate_adapter_files: Option<AdapterValidator>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillStatus {
    NotDeployed,
    Current,
    Outdated,
    Drifted,
    MigrationRequired,
    Missing,
    Collision,
    Invalid,
    Discovered,
}

impl SkillStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NotDeployed => "not-deployed",
            Self::Current => "current",
            Self::Outdated => "outdated",
            Self::Drifted => "drifted",
            Self::MigrationRequired => "migration-required",
            Self::Missing => "missing",
            Self::Collision => "collision",
            Self::Invalid => "invalid",
            Self::Discovered => "discovered",
        }
    }
}

#[derive(Debug, Clone)]
pub struct StatusRow {
    pub pack: String,
    pub skill: String,
    pub status: SkillStatus,
    pub detail: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Unit {
    pub key: String,
    pub source_root: PathBuf,
    pub destination: PathBuf,
    pub is_skill: bool,
    pub only_file: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct State {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub destination: String,
    #[serde(default)]
    pub packs: BTreeMap<String, PackRecord>,
}

#[derive(Debug, Default, Deserialize)]
pub struct PackRecord {
    #[serde(default)]
    pub skills: BTreeMap<String, SkillRecord>,
}

#[derive(Debug, Default, Deserialize)]
pub struct SkillRecord {
    #[serde(default, rename = "installedDigest")]
    pub installed_digest: String,
    #[serde(default, rename = "digestPolicy")]
    pub digest_policy: Option<String>,
}

// ---- paths ---------------------------------------------------------------------------------

/// Node's `path.resolve`: absolute against the cwd, with `.` and `..` folded lexically.
///
/// Deliberately lexical and not `canonicalize`: pack-deploy.ts compares a ledger's recorded
/// destination with the configured one, and resolving symlinks on one side only would make
/// two names for the same directory compare unequal.
pub fn resolve(p: &Path) -> PathBuf {
    let absolute = if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(p)
    };
    let mut out: Vec<std::ffi::OsString> = Vec::new();
    for c in absolute.components() {
        match c {
            Component::RootDir => out.clear(),
            Component::CurDir => {}
            Component::ParentDir => {
                if out.last().is_some_and(|s| s != OsStr::new("..")) {
                    out.pop();
                }
            }
            Component::Normal(s) => out.push(s.to_os_string()),
            Component::Prefix(_) => {}
        }
    }
    let mut resolved = PathBuf::from("/");
    for part in out {
        resolved.push(part);
    }
    resolved
}

pub fn pack_roots(config: &DeployConfig) -> Vec<PathBuf> {
    match &config.pack_roots {
        Some(roots) if !roots.is_empty() => roots.clone(),
        _ => vec![config.axon_root.join("Packs")],
    }
}

fn pack_dir(config: &DeployConfig, pack: &str) -> Result<PathBuf, String> {
    assert_simple_name(pack, "pack")?;
    let roots = pack_roots(config);
    let matches: Vec<&PathBuf> = roots
        .iter()
        .filter(|root| root.join(pack).join("pack.toml").is_file())
        .collect();
    match matches.len() {
        0 => Ok(roots[0].join(pack)),
        1 => Ok(matches[0].join(pack)),
        _ => {
            let joined = matches
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            Err(format!(
                "pack '{pack}' is declared in more than one Pack root: {joined}"
            ))
        }
    }
}

fn assert_simple_name(value: &str, label: &str) -> Result<(), String> {
    let simple = !value.is_empty()
        && value.split('-').all(|part| {
            !part.is_empty()
                && part
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        });
    if simple {
        Ok(())
    } else {
        Err(format!("{label} '{value}' must be lowercase hyphen-case"))
    }
}

// ---- the ledger ----------------------------------------------------------------------------

pub fn read_state(config: &DeployConfig) -> Result<State, String> {
    if !config.state_file.is_file() {
        return Ok(State {
            version: 1,
            destination: config.destination.display().to_string(),
            packs: BTreeMap::new(),
        });
    }
    let text = fs::read_to_string(&config.state_file).map_err(|e| {
        format!(
            "cannot read {} deployment state {}: {e}",
            config.adapter,
            config.state_file.display()
        )
    })?;
    let state: State = serde_json::from_str(&text).map_err(|_| {
        format!(
            "unsupported or malformed {} deployment state: {}",
            config.adapter,
            config.state_file.display()
        )
    })?;
    if state.version != 1 {
        return Err(format!(
            "unsupported or malformed {} deployment state: {}",
            config.adapter,
            config.state_file.display()
        ));
    }
    if resolve(Path::new(&state.destination)) != resolve(&config.destination) {
        let hint = config
            .state_env_var
            .as_ref()
            .map(|v| format!("; set {v} for this destination"))
            .unwrap_or_default();
        return Err(format!(
            "state file belongs to {}, not {}{hint}",
            state.destination,
            config.destination.display()
        ));
    }
    Ok(state)
}

// ---- Pack manifests ------------------------------------------------------------------------

pub fn read_pack_skills(config: &DeployConfig, pack: &str) -> Result<Vec<String>, String> {
    let manifest = pack_dir(config, pack)?.join("pack.toml");
    if !manifest.is_file() {
        return Err(format!("no such pack: {pack}"));
    }
    let text = fs::read_to_string(&manifest).map_err(|e| format!("{}: {e}", manifest.display()))?;
    let parsed: toml::Table = text
        .parse()
        .map_err(|e| format!("{}: {e}", manifest.display()))?;
    if parsed.get("name").and_then(toml::Value::as_str) != Some(pack) {
        return Err(format!(
            "{}: name must match directory '{pack}'",
            manifest.display()
        ));
    }
    let Some(skills) = parsed.get("skills").and_then(toml::Value::as_array) else {
        return Err(format!(
            "{}: skills must be an array of names",
            manifest.display()
        ));
    };
    let mut names = Vec::with_capacity(skills.len());
    let mut seen: Vec<&str> = Vec::new();
    for skill in skills {
        let Some(skill) = skill.as_str() else {
            return Err(format!(
                "{}: skills must be an array of names",
                manifest.display()
            ));
        };
        assert_simple_name(skill, "skill")?;
        if seen.contains(&skill) {
            return Err(format!("{}: duplicate skill '{skill}'", manifest.display()));
        }
        seen.push(skill);
        names.push(skill.to_owned());
    }
    Ok(names)
}

/// A Pack whose manifest names a `deployer` is owned by that tool alone. The read is from
/// `axonRoot/Packs`, not the Pack roots — the same place pack-deploy.ts looks.
pub fn pack_deployer(config: &DeployConfig, pack: &str) -> Option<String> {
    let manifest = config.axon_root.join("Packs").join(pack).join("pack.toml");
    let parsed: toml::Table = fs::read_to_string(manifest).ok()?.parse().ok()?;
    parsed
        .get("deployer")
        .and_then(toml::Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

pub fn available_packs(
    config: &DeployConfig,
    include_dedicated: bool,
) -> Result<Vec<String>, String> {
    let mut matches: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for root in pack_roots(config) {
        let Ok(entries) = fs::read_dir(&root) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !entry.path().is_dir() || !entry.path().join("pack.toml").is_file() {
                continue;
            }
            matches
                .entry(name)
                .or_default()
                .push(root.display().to_string());
        }
    }
    for (pack, roots) in &matches {
        if roots.len() > 1 {
            return Err(format!(
                "pack '{pack}' is declared in more than one Pack root: {}",
                roots.join(", ")
            ));
        }
    }
    Ok(matches
        .into_keys()
        .filter(|pack| include_dedicated || pack_deployer(config, pack).is_none())
        .collect())
}

// ---- units and files -----------------------------------------------------------------------

fn tree_key(source_dir: &str) -> String {
    format!("{source_dir}/")
}

fn flat_key(source_dir: &str, file: &str) -> String {
    format!("{source_dir}/{file}")
}

fn flat_source_files(
    config: &DeployConfig,
    pack: &str,
    source_dir: &str,
) -> Result<Vec<String>, String> {
    let dir = pack_dir(config, pack)?.join(source_dir);
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    for entry in read_dir_sorted(&dir)? {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        if !entry.path().is_file() {
            return Err(format!("{pack}/{source_dir}: {name} is not a file"));
        }
        if !name.ends_with(".md") {
            return Err(format!(
                "{pack}/{source_dir}/{name}: only .md files deploy from a flat-file convention; \
                 the harness loads nothing else, so this file would be silently inert"
            ));
        }
        files.push(name);
    }
    files.sort();
    Ok(files)
}

pub fn pack_units(config: &DeployConfig, pack: &str) -> Result<Vec<Unit>, String> {
    let dir = pack_dir(config, pack)?;
    // Read the manifest even when its skills are not deployed, so a mistyped Pack name still
    // fails here rather than reporting nothing deployed.
    let manifest_skills = read_pack_skills(config, pack)?;
    let mut units = Vec::new();
    if !config.skip_manifest_skills {
        for skill in manifest_skills {
            units.push(Unit {
                key: skill.clone(),
                source_root: dir.join("skills").join(&skill),
                destination: config.destination.join(&skill),
                is_skill: true,
                only_file: None,
            });
        }
    }
    if let Some(tree) = &config.tree_convention {
        if dir.join(&tree.source_dir).is_dir() {
            units.push(Unit {
                key: tree_key(&tree.source_dir),
                source_root: dir.join(&tree.source_dir),
                destination: tree.destination_root.join(pack),
                is_skill: false,
                only_file: None,
            });
        }
    }
    if let Some(flat) = &config.flat_file_convention {
        for file in flat_source_files(config, pack, &flat.source_dir)? {
            units.push(Unit {
                key: flat_key(&flat.source_dir, &file),
                source_root: dir.join(&flat.source_dir),
                destination: flat.destination_root.join(&file),
                is_skill: false,
                only_file: Some(file),
            });
        }
    }
    Ok(units)
}

/// `readdir`, sorted by name. TypeScript reads directory order; sorting makes a report that
/// mentions files reproducible between runs and machines, which the drift output needs.
fn read_dir_sorted(dir: &Path) -> Result<Vec<fs::DirEntry>, String> {
    let entries = fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut entries: Vec<fs::DirEntry> = entries.flatten().collect();
    entries.sort_by_key(fs::DirEntry::file_name);
    Ok(entries)
}

fn is_generated_artifact_path(relative_path: &str) -> bool {
    let parts: Vec<&str> = relative_path.split('/').collect();
    parts.contains(&"__pycache__")
        || parts.last().is_some_and(|last| {
            last.ends_with(".pyc") || last.ends_with(".pyo") || last.ends_with(".pyd")
        })
}

fn collect_files(
    config: &DeployConfig,
    root: &Path,
    source_label: &str,
    include_generated_artifacts: bool,
) -> Result<Files, String> {
    let mut files = Files::new();
    let Ok(root_stat) = fs::symlink_metadata(root) else {
        return Ok(files);
    };
    if root_stat.file_type().is_symlink() {
        return Err(format!(
            "{source_label} is a symlink; {} deployment must be materialized",
            config.adapter
        ));
    }
    if !root_stat.is_dir() {
        return Err(format!(
            "{source_label} is not a directory: {}",
            root.display()
        ));
    }

    visit_dir(
        config,
        root,
        source_label,
        include_generated_artifacts,
        root,
        &mut files,
    )?;
    Ok(files)
}

/// The recursive half of `collectFiles`, a free function because a closure cannot call itself.
///
/// Directories are walked in sorted order (see `read_dir_sorted`); TypeScript walks readdir
/// order, and the only report that shows file order is `drift`, where deterministic is worth
/// more than matching one filesystem's answer.
fn visit_dir(
    config: &DeployConfig,
    root: &Path,
    source_label: &str,
    include_generated_artifacts: bool,
    dir: &Path,
    files: &mut Files,
) -> Result<(), String> {
    for entry in read_dir_sorted(dir)? {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == ".DS_Store" {
            continue;
        }
        let absolute_path = entry.path();
        let rel = absolute_path
            .strip_prefix(root)
            .unwrap_or(&absolute_path)
            .to_string_lossy()
            .into_owned();
        if !include_generated_artifacts && is_generated_artifact_path(&rel) {
            continue;
        }
        let Ok(stat) = fs::symlink_metadata(&absolute_path) else {
            continue;
        };
        if stat.file_type().is_symlink() {
            return Err(format!(
                "{source_label} contains a symlink; {} deployment must be materialized: {rel}",
                config.adapter
            ));
        }
        if stat.is_dir() {
            visit_dir(
                config,
                root,
                source_label,
                include_generated_artifacts,
                &absolute_path,
                files,
            )?;
        } else if stat.is_file() {
            files.insert(
                rel.clone(),
                DesiredFile {
                    absolute_path,
                    relative_path: rel,
                    mode: stat.mode() & 0o777,
                    content: None,
                },
            );
        } else {
            return Err(format!(
                "{source_label} contains unsupported filesystem entry: {rel}"
            ));
        }
    }
    Ok(())
}

pub fn desired_files(config: &DeployConfig, pack: &str, unit: &Unit) -> Result<Files, String> {
    if !unit.source_root.exists() {
        return Err(format!(
            "{pack}/{}: source missing at {}",
            unit.key,
            unit.source_root.display()
        ));
    }
    let mut files = collect_files(
        config,
        &unit.source_root,
        &format!("{pack}/{}", unit.key),
        false,
    )?;
    if !unit.is_skill {
        let Some(only_file) = &unit.only_file else {
            return Ok(files);
        };
        let Some(file) = files.remove(only_file) else {
            return Err(format!(
                "{pack}/{}: source file '{only_file}' missing from {}",
                unit.key,
                unit.source_root.display()
            ));
        };
        // A flat unit's transform runs in the TypeScript engine; no read path here reaches a
        // config that sets one, so the source bytes are the deployed bytes.
        return Ok(Files::from([(file.relative_path.clone(), file)]));
    }

    let overlay_root = pack_dir(config, pack)?
        .join(&config.adapter)
        .join(&unit.key);
    let overlay = collect_files(
        config,
        &overlay_root,
        &format!("{pack}/{} {} overlay", unit.key, config.adapter),
        false,
    )?;
    if overlay.contains_key("SKILL.md") {
        return Err(format!(
            "{pack}/{}: {} overlay may not override canonical SKILL.md",
            unit.key, config.adapter
        ));
    }
    for (rel, file) in overlay {
        files.insert(rel, file);
    }
    Ok(files)
}

// ---- digests -------------------------------------------------------------------------------

fn sha256_of(files: &Files) -> String {
    let mut hasher = Sha256::new();
    for (rel, file) in files {
        hasher.update(rel.as_bytes());
        hasher.update([0]);
        hasher.update(format!("{:o}", file.mode).as_bytes());
        hasher.update([0]);
        match &file.content {
            Some(bytes) => hasher.update(bytes),
            None => hasher.update(fs::read(&file.absolute_path).unwrap_or_default()),
        }
        hasher.update([0]);
    }
    // sha2 0.11's `finalize()` returns `Array<u8, U32>`, which no longer implements `LowerHex`
    // the way 0.10's `GenericArray` did; the workspace's idiom is a byte-wise `02x` write
    // (libs/pseudonymize, devices::store, finance, places).
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub fn digest_files(files: &Files) -> String {
    sha256_of(files)
}

fn digest_tree(config: &DeployConfig, root: &Path) -> Result<String, String> {
    let label = root.display().to_string();
    Ok(sha256_of(&collect_files(config, root, &label, false)?))
}

fn legacy_digest_tree(config: &DeployConfig, root: &Path) -> Result<String, String> {
    let label = root.display().to_string();
    Ok(sha256_of(&collect_files(config, root, &label, true)?))
}

fn digest_destination(config: &DeployConfig, destination: &Path) -> Result<String, String> {
    // `metadata`, not `symlink_metadata`, because pack-deploy.ts uses statSync: a symlink at
    // the destination is followed here and refused by collectFiles, which is where the
    // "must be materialized" rule lives.
    let stat = fs::metadata(destination).map_err(|e| format!("{}: {e}", destination.display()))?;
    if stat.is_dir() {
        return digest_tree(config, destination);
    }
    Ok(digest_one_file(destination, stat.mode() & 0o777))
}

fn legacy_digest_destination(config: &DeployConfig, destination: &Path) -> Result<String, String> {
    let stat = fs::metadata(destination).map_err(|e| format!("{}: {e}", destination.display()))?;
    if stat.is_dir() {
        return legacy_digest_tree(config, destination);
    }
    Ok(digest_one_file(destination, stat.mode() & 0o777))
}

fn digest_one_file(path: &Path, mode: u32) -> String {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let files = Files::from([(
        name.clone(),
        DesiredFile {
            absolute_path: path.to_path_buf(),
            relative_path: name,
            mode,
            content: None,
        },
    )]);
    sha256_of(&files)
}

// ---- validation ----------------------------------------------------------------------------

pub fn validate_unit(
    config: &DeployConfig,
    files: &Files,
    unit: &Unit,
    label: &str,
) -> Result<(), String> {
    if !unit.is_skill {
        return Ok(());
    }
    let Some(skill_md) = files.get("SKILL.md") else {
        return Err(format!("{label}: SKILL.md missing"));
    };
    let text = fs::read_to_string(&skill_md.absolute_path)
        .map_err(|e| format!("{}: {e}", skill_md.absolute_path.display()))?;
    frontmatter::validate_skill(&text, &unit.key, label)?;
    if let Some(validate) = config.validate_adapter_files {
        validate(files, label)?;
    }
    Ok(())
}

/// The codex adapter's extra check, named by its config.
pub fn validate_codex_files(files: &Files, label: &str) -> Result<(), String> {
    let Some(openai_yaml) = files.get("agents/openai.yaml") else {
        return Ok(());
    };
    let text = fs::read_to_string(&openai_yaml.absolute_path)
        .map_err(|e| format!("{}: {e}", openai_yaml.absolute_path.display()))?;
    frontmatter::validate_openai_yaml(&text, label)
}

// ---- statuses ------------------------------------------------------------------------------

pub fn get_statuses(
    config: &DeployConfig,
    selected_pack: Option<&str>,
) -> Result<Vec<StatusRow>, String> {
    let state = read_state(config)?;
    let packs: Vec<String> = match selected_pack {
        Some(pack) => vec![pack.to_owned()],
        None => {
            let mut all: Vec<String> = available_packs(config, false)?;
            for pack in state.packs.keys() {
                if !all.contains(pack) {
                    all.push(pack.clone());
                }
            }
            all.sort();
            all
        }
    };

    let mut rows = Vec::new();
    for pack in packs {
        let units = match pack_units(config, &pack) {
            Ok(units) => units,
            Err(e) => {
                rows.push(StatusRow {
                    pack,
                    skill: "(manifest)".to_owned(),
                    status: SkillStatus::Invalid,
                    detail: Some(e),
                });
                continue;
            }
        };
        let seen: Vec<String> = units.iter().map(|u| u.key.clone()).collect();
        for unit in &units {
            let record = state.packs.get(&pack).and_then(|p| p.skills.get(&unit.key));
            let wanted = match desired_files(config, &pack, unit).and_then(|files| {
                validate_unit(config, &files, unit, &format!("{pack}/{}", unit.key))?;
                Ok(digest_files(&files))
            }) {
                Ok(wanted) => wanted,
                Err(e) => {
                    rows.push(StatusRow {
                        pack: pack.clone(),
                        skill: unit.key.clone(),
                        status: SkillStatus::Invalid,
                        detail: Some(e),
                    });
                    continue;
                }
            };
            let Some(record) = record else {
                rows.push(StatusRow {
                    pack: pack.clone(),
                    skill: unit.key.clone(),
                    status: if unit.destination.exists() {
                        SkillStatus::Collision
                    } else {
                        SkillStatus::NotDeployed
                    },
                    detail: None,
                });
                continue;
            };
            if !unit.destination.exists() {
                rows.push(StatusRow {
                    pack: pack.clone(),
                    skill: unit.key.clone(),
                    status: SkillStatus::Missing,
                    detail: None,
                });
                continue;
            }
            match (|| -> Result<StatusRow, String> {
                let actual = digest_destination(config, &unit.destination)?;
                if record.digest_policy.as_deref() != Some(DIGEST_POLICY) {
                    if legacy_digest_destination(config, &unit.destination)?
                        != record.installed_digest
                    {
                        return Ok(StatusRow {
                            pack: pack.clone(),
                            skill: unit.key.clone(),
                            status: SkillStatus::MigrationRequired,
                            detail: Some(
                                "legacy digest differs; review before adopting generated-artifact exclusions"
                                    .to_owned(),
                            ),
                        });
                    }
                } else if actual != record.installed_digest {
                    return Ok(StatusRow {
                        pack: pack.clone(),
                        skill: unit.key.clone(),
                        status: SkillStatus::Drifted,
                        detail: None,
                    });
                }
                Ok(StatusRow {
                    pack: pack.clone(),
                    skill: unit.key.clone(),
                    status: if wanted == actual {
                        SkillStatus::Current
                    } else {
                        SkillStatus::Outdated
                    },
                    detail: None,
                })
            })() {
                Ok(row) => rows.push(row),
                Err(e) => rows.push(StatusRow {
                    pack: pack.clone(),
                    skill: unit.key.clone(),
                    status: SkillStatus::Invalid,
                    detail: Some(e),
                }),
            }
        }
        // Ledger rows whose unit left the manifest.
        let stale: Vec<&String> = state
            .packs
            .get(&pack)
            .map(|p| p.skills.keys().filter(|k| !seen.contains(k)).collect())
            .unwrap_or_default();
        for key in stale {
            rows.push(StatusRow {
                pack: pack.clone(),
                skill: key.clone(),
                status: SkillStatus::Outdated,
                detail: Some("removed from pack manifest".to_owned()),
            });
        }
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("sjel-harnesses-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(path: &Path, body: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }

    fn config_for(root: &Path) -> DeployConfig {
        DeployConfig {
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
        }
    }

    fn pkg(root: &Path, pack: &str, skills: &[&str]) {
        let dir = root.join("repo").join("Packs").join(pack);
        let list: Vec<String> = skills.iter().map(|s| format!("\"{s}\"")).collect();
        write(
            &dir.join("pack.toml"),
            &format!(
                "name = \"{pack}\"\ndescription = \"test\"\nskills = [{}]\n",
                list.join(", ")
            ),
        );
        for skill in skills {
            write(
                &dir.join("skills").join(skill).join("SKILL.md"),
                &format!("---\nname: {skill}\ndescription: does a thing\n---\n\nbody\n"),
            );
        }
    }

    #[test]
    fn an_absent_ledger_reads_as_empty() {
        let root = temp_dir("absent");
        let state = read_state(&config_for(&root)).unwrap();
        assert!(state.packs.is_empty());
    }

    #[test]
    fn a_ledger_for_another_destination_is_refused() {
        let root = temp_dir("elsewhere");
        let config = config_for(&root);
        write(
            &config.state_file,
            "{\"version\":1,\"destination\":\"/somewhere/else\",\"packs\":{}}",
        );
        let err = read_state(&config).unwrap_err();
        assert!(
            err.starts_with("state file belongs to /somewhere/else"),
            "{err}"
        );
    }

    #[test]
    fn a_pack_with_no_deployed_unit_is_not_deployed() {
        let root = temp_dir("not-deployed");
        let config = config_for(&root);
        pkg(&root, "demo", &["alpha"]);
        let rows = get_statuses(&config, None).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status, SkillStatus::NotDeployed);
    }

    #[test]
    fn an_occupied_destination_without_a_ledger_row_is_a_collision() {
        let root = temp_dir("collision");
        let config = config_for(&root);
        pkg(&root, "demo", &["alpha"]);
        write(
            &config.destination.join("alpha").join("SKILL.md"),
            "stray\n",
        );
        let rows = get_statuses(&config, None).unwrap();
        assert_eq!(rows[0].status, SkillStatus::Collision);
    }

    #[test]
    fn an_installed_unit_matching_its_source_is_current() {
        let root = temp_dir("current");
        let config = config_for(&root);
        pkg(&root, "demo", &["alpha"]);
        let src = config.axon_root.join("Packs/demo/skills/alpha/SKILL.md");
        let dest = config.destination.join("alpha/SKILL.md");
        fs::create_dir_all(dest.parent().unwrap()).unwrap();
        fs::copy(&src, &dest).unwrap();
        let mode = fs::symlink_metadata(&dest).unwrap().mode() & 0o777;
        let digest = digest_one_file(&dest, mode);
        write(
            &config.state_file,
            &format!(
                "{{\"version\":1,\"destination\":\"{}\",\"packs\":{{\"demo\":{{\"skills\":{{\"alpha\":{{\"source\":\"x\",\"desiredDigest\":\"d\",\"installedDigest\":\"{digest}\",\"digestPolicy\":\"{DIGEST_POLICY}\",\"deployedAt\":\"now\"}}}}}}}}}}",
                config.destination.display()
            ),
        );
        let rows = get_statuses(&config, None).unwrap();
        assert_eq!(rows[0].status, SkillStatus::Current, "{:?}", rows[0]);
    }

    #[test]
    fn a_destination_differs_from_its_source_as_outdated() {
        let root = temp_dir("outdated");
        let config = config_for(&root);
        pkg(&root, "demo", &["alpha"]);
        let dest = config.destination.join("alpha/SKILL.md");
        write(
            &dest,
            "---\nname: alpha\ndescription: does a thing\n---\n\nCHANGED\n",
        );
        let mode = fs::symlink_metadata(&dest).unwrap().mode() & 0o777;
        let digest = digest_one_file(&dest, mode);
        write(
            &config.state_file,
            &format!(
                "{{\"version\":1,\"destination\":\"{}\",\"packs\":{{\"demo\":{{\"skills\":{{\"alpha\":{{\"source\":\"x\",\"desiredDigest\":\"d\",\"installedDigest\":\"{digest}\",\"digestPolicy\":\"{DIGEST_POLICY}\",\"deployedAt\":\"now\"}}}}}}}}}}",
                config.destination.display()
            ),
        );
        let rows = get_statuses(&config, None).unwrap();
        assert_eq!(rows[0].status, SkillStatus::Outdated);
    }

    #[test]
    fn a_ledger_row_changed_on_disk_is_drifted() {
        let root = temp_dir("drifted");
        let config = config_for(&root);
        pkg(&root, "demo", &["alpha"]);
        let dest = config.destination.join("alpha/SKILL.md");
        write(
            &dest,
            "---\nname: alpha\ndescription: does a thing\n---\n\nEDITED IN PLACE\n",
        );
        write(
            &config.state_file,
            &format!(
                "{{\"version\":1,\"destination\":\"{}\",\"packs\":{{\"demo\":{{\"skills\":{{\"alpha\":{{\"source\":\"x\",\"desiredDigest\":\"d\",\"installedDigest\":\"000000000000\",\"digestPolicy\":\"{DIGEST_POLICY}\",\"deployedAt\":\"now\"}}}}}}}}}}",
                config.destination.display()
            ),
        );
        let rows = get_statuses(&config, None).unwrap();
        assert_eq!(rows[0].status, SkillStatus::Drifted);
    }

    #[test]
    fn a_ledger_owned_unit_that_is_gone_is_missing() {
        let root = temp_dir("missing");
        let config = config_for(&root);
        pkg(&root, "demo", &["alpha"]);
        write(
            &config.state_file,
            &format!(
                "{{\"version\":1,\"destination\":\"{}\",\"packs\":{{\"demo\":{{\"skills\":{{\"alpha\":{{\"source\":\"x\",\"desiredDigest\":\"d\",\"installedDigest\":\"e\",\"digestPolicy\":\"{DIGEST_POLICY}\",\"deployedAt\":\"now\"}}}}}}}}}}",
                config.destination.display()
            ),
        );
        let rows = get_statuses(&config, None).unwrap();
        assert_eq!(rows[0].status, SkillStatus::Missing);
    }

    #[test]
    fn a_malformed_skill_is_invalid_with_the_engine_message() {
        let root = temp_dir("invalid");
        let config = config_for(&root);
        pkg(&root, "demo", &["alpha"]);
        write(
            &config.axon_root.join("Packs/demo/skills/alpha/SKILL.md"),
            "---\nname: wrong\ndescription: d\n---\n",
        );
        let rows = get_statuses(&config, None).unwrap();
        assert_eq!(rows[0].status, SkillStatus::Invalid);
        assert_eq!(
            rows[0].detail.as_deref(),
            Some("demo/alpha: SKILL.md name must be 'alpha'")
        );
    }

    #[test]
    fn a_dedicated_pack_is_not_generic_deployment() {
        let root = temp_dir("dedicated");
        let config = config_for(&root);
        pkg(&root, "demo", &["alpha"]);
        let manifest = config.axon_root.join("Packs/demo/pack.toml");
        let body = fs::read_to_string(&manifest).unwrap();
        write(&manifest, &format!("{body}deployer = \"somewhere-else\"\n"));
        assert_eq!(
            available_packs(&config, false).unwrap(),
            Vec::<String>::new()
        );
        assert_eq!(
            available_packs(&config, true).unwrap(),
            vec!["demo".to_owned()]
        );
    }

    #[test]
    fn digests_ignore_insertion_order_and_read_the_mode() {
        let file = |name: &str, mode: u32, body: &[u8]| {
            (
                name.to_owned(),
                DesiredFile {
                    absolute_path: PathBuf::new(),
                    relative_path: name.to_owned(),
                    mode,
                    content: Some(body.to_vec()),
                },
            )
        };
        let forward = Files::from([file("a.md", 0o644, b"hello"), file("b.md", 0o644, b"x")]);
        let backward = Files::from([file("b.md", 0o644, b"x"), file("a.md", 0o644, b"hello")]);
        assert_eq!(digest_files(&forward), digest_files(&backward));
        // The executable bit is part of the digest, which is what makes a mode change drift.
        let executable = Files::from([file("a.md", 0o755, b"hello"), file("b.md", 0o644, b"x")]);
        assert_ne!(digest_files(&forward), digest_files(&executable));
    }
}
