//! `tools/skills add|update`: install exactly the bytes the intake gate screened.
//!
//! Upstream's CLI owns source resolution, per-agent layouts and lock entries, but it fetches on
//! every invocation, so a second call after screening can install bytes nobody read. Here it
//! runs once, inside a private stage: `add <source> --copy` with the requested skills and
//! agents. Every staged skill directory is read into an in-memory tree, that tree is what the
//! gate screens, and publication materializes the same tree — the stage is not read again after
//! screening. The project `skills-lock.json` takes the staged entry verbatim, so remote
//! provenance survives.
//!
//! `store` publishes the same screened bytes into the skill store instead of a project: one
//! Pack per upstream source, whose `pack.toml` pins each skill's digest. Every harness reads
//! the store as a Pack root, so profiles select upstream skills like Pack skills. `verify`
//! re-reads the store against those pins.

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

use super::{digest, gate_tree, read_tree, Tree};

const HELD: u8 = 2;
/// The agents a project-scoped `add` installs for when none is named.
const DEFAULT_AGENTS: &[&str] = &["pi", "claude-code"];

pub fn run(args: &[String]) -> ExitCode {
    match run_inner(args) {
        Ok(code) => ExitCode::from(code),
        Err(message) => {
            eprintln!("skills: {message}");
            ExitCode::from(1)
        }
    }
}

fn run_inner(args: &[String]) -> Result<u8, String> {
    let Some((verb, rest)) = args.split_first() else {
        return Err(usage().into());
    };
    match verb.as_str() {
        "add" => add(rest),
        "update" => update(rest),
        "store" => store(rest),
        "verify" => verify(),
        "help" | "-h" | "--help" => {
            println!("{}", usage());
            Ok(0)
        }
        other => Err(format!("unknown command `{other}`. {}", usage())),
    }
}

fn usage() -> &'static str {
    "usage: skills add <package> [--skill <name>]... [--agent <id>]... | skills update | \
     skills store <package> [--skill <name>]... | skills verify"
}

/// One screened skill, bound for one agent's project directory.
struct Job {
    skill: String,
    dest: PathBuf,
    staged: PathBuf,
    tree: Tree,
    before: DestState,
}

/// What occupied the destination before the run, rechecked at publication.
#[derive(Clone)]
enum DestState {
    Absent,
    Other,
    Tree(Tree),
}

fn add(args: &[String]) -> Result<u8, String> {
    let mut package = String::new();
    let mut skills: Vec<String> = Vec::new();
    let mut agents: Vec<String> = Vec::new();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut value = |flag: &str| {
            it.next()
                .cloned()
                .ok_or_else(|| format!("{flag} needs a value"))
        };
        match arg.as_str() {
            "-s" | "--skill" => skills.push(value("--skill")?),
            "-a" | "--agent" => agents.push(value("--agent")?),
            "-g" | "--global" => {
                return Err(
                    "a system-wide skill must be a Sjel Pack: tools/packs-pi deploy <pack>".into(),
                )
            }
            other if other.starts_with('-') => return Err(format!("unknown option {other}")),
            other if package.is_empty() => package = other.to_owned(),
            other => {
                return Err(format!(
                    "add takes one package; got '{package}' and '{other}'"
                ))
            }
        }
    }
    if package.is_empty() {
        return Err("add needs a package".into());
    }
    if agents.is_empty() {
        agents = DEFAULT_AGENTS.iter().map(|a| (*a).to_owned()).collect();
    }

    let project = project_root()?;
    let stage = fresh_stage("add")?;
    println!("staging {package} for {}", agents.join(", "));
    stage_add(&stage, &package, &skills, &agents)?;

    let jobs = select(collect_jobs(&stage, &project)?, &skills, &package)?;
    install(&project, &stage, &jobs, &package)
}

/// Keep the named skills, or every staged one when none is named.
fn select(mut jobs: Vec<Job>, skills: &[String], package: &str) -> Result<Vec<Job>, String> {
    if !skills.is_empty() {
        jobs.retain(|job| skills.contains(&job.skill));
        for name in skills {
            if !jobs.iter().any(|job| &job.skill == name) {
                return Err(format!("{package} staged no skill '{name}'"));
            }
        }
    }
    if jobs.is_empty() {
        return Err(format!("{package} staged no skill"));
    }
    Ok(jobs)
}

/// Gate every job under its package key: 0 passes, HELD holds, 1 is an error.
fn gate_jobs(jobs: &[Job], package: &str) -> u8 {
    let mut code = 0;
    for job in jobs {
        let key = format!("skills:{package}/{}", job.skill);
        match gate_tree(&key, Some(&job.staged), &job.tree, baseline(&job.before)) {
            0 => {}
            HELD if code == 0 => code = HELD,
            HELD => {}
            _ => code = 1,
        }
    }
    code
}

/// Where approved upstream skills live, one Pack per source. Every harness reads it as a Pack
/// root (`harnesses/registry.rs`).
pub fn store_root() -> PathBuf {
    super::state_dir()
        .parent()
        .map(|sjel| sjel.join("skill-store"))
        .unwrap_or_default()
}

/// The store Pack for a source: the repository name, as `owner/repo` or a URL names it.
fn store_pack(package: &str) -> Result<String, String> {
    let name = package
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .trim_end_matches(".git")
        .to_ascii_lowercase();
    crate::harnesses::engine::assert_simple_name(&name, "store pack")?;
    Ok(name)
}

fn store(args: &[String]) -> Result<u8, String> {
    let mut package = String::new();
    let mut skills: Vec<String> = Vec::new();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-s" | "--skill" => skills.push(it.next().cloned().ok_or("--skill needs a value")?),
            other if other.starts_with('-') => return Err(format!("unknown option {other}")),
            other if package.is_empty() => package = other.to_owned(),
            other => {
                return Err(format!(
                    "store takes one package; got '{package}' and '{other}'"
                ))
            }
        }
    }
    if package.is_empty() {
        return Err("store needs a package".into());
    }
    let pack = store_pack(&package)?;
    let pack_dir = store_root().join(&pack);
    let stage = fresh_stage("store")?;
    println!("staging {package} for the skill store");
    stage_add(&stage, &package, &skills, &["claude-code".to_owned()])?;
    let mut jobs = select(collect_jobs(&stage, &stage)?, &skills, &package)?;
    for job in &mut jobs {
        job.dest = pack_dir.join("skills").join(&job.skill);
        job.before = dest_state(&job.dest)?;
    }
    let names: Vec<String> = jobs.iter().map(|job| job.skill.clone()).collect();
    let root = std::env::var("SJEL_ROOT")
        .map_err(|_| "SJEL_ROOT is unset; run it through tools/skills")?;
    collisions(Path::new(&root), &pack, &names)?;
    match gate_jobs(&jobs, &package) {
        0 => {}
        HELD => {
            println!("\nnothing stored: {package} is held");
            return Ok(HELD);
        }
        code => return Ok(code),
    }
    let published = publish(&jobs)?;
    if let Err(e) = write_store_manifest(&pack_dir, &pack, &package, &stage, &jobs) {
        published.rollback();
        return Err(format!("{e}; stored skills were rolled back"));
    }
    published.commit();
    println!(
        "✓ stored {} in Pack '{pack}' at {}\n  Select it in profiles.toml, then: tools/harnesses apply",
        names.join(", "),
        pack_dir.display()
    );
    Ok(0)
}

/// Refuse a store Pack or skill name that another Pack root already declares: two roots with
/// one Pack name stop every harness, and two Packs with one skill name fight over its folder.
fn collisions(root: &Path, pack: &str, names: &[String]) -> Result<(), String> {
    use crate::harnesses::{engine, registry::Registry};
    let config = crate::packs::config(&Registry::new(root), "opencode")?;
    let store = store_root();
    for other in engine::available_packs(&config, true)? {
        let dir = engine::pack_dir(&config, &other)?;
        if dir.starts_with(&store) {
            continue;
        }
        if other == pack {
            return Err(format!("a Pack named '{pack}' exists at {}", dir.display()));
        }
        let theirs = engine::read_pack_skills(&config, &other)?;
        if let Some(name) = names.iter().find(|n| theirs.contains(n)) {
            return Err(format!("skill '{name}' is already in Pack '{other}'"));
        }
    }
    Ok(())
}

/// Write the store Pack's manifest: the Pack fields the deploy engine reads, the upstream
/// source, and one pin per skill. Pins of skills this run did not touch are kept.
fn write_store_manifest(
    pack_dir: &Path,
    pack: &str,
    package: &str,
    stage: &Path,
    jobs: &[Job],
) -> Result<(), String> {
    let path = pack_dir.join("pack.toml");
    let mut pins = match fs::read_to_string(&path) {
        Ok(text) => text
            .parse::<toml::Table>()
            .map_err(|e| format!("{}: {e}", path.display()))?
            .remove("pin")
            .and_then(|v| v.as_table().cloned())
            .unwrap_or_default(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => toml::Table::new(),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let lock: Value = fs::read_to_string(stage.join("skills-lock.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(Value::Null);
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let mut url = String::new();
    for job in jobs {
        let entry = &lock["skills"][&job.skill];
        if let Some(u) = entry["sourceUrl"].as_str() {
            url = u.to_owned();
        }
        let digest = digest(&job.tree);
        let mut pin = toml::Table::new();
        pin.insert("digest".into(), digest.clone().into());
        if let Some(rev) = entry["skillFolderHash"].as_str() {
            pin.insert("revision".into(), rev.into());
        }
        pin.insert(
            "approval".into(),
            super::evidence(&digest)
                .unwrap_or_else(|| format!("passed the intake_review gate {today}"))
                .into(),
        );
        pin.insert("stored".into(), today.clone().into());
        pins.insert(job.skill.clone(), pin.into());
    }
    let mut manifest = toml::Table::new();
    manifest.insert("name".into(), pack.into());
    manifest.insert(
        "description".into(),
        format!("Approved upstream skills from {package}").into(),
    );
    manifest.insert(
        "skills".into(),
        pins.keys().cloned().collect::<Vec<_>>().into(),
    );
    let mut upstream = toml::Table::new();
    upstream.insert("source".into(), package.into());
    if !url.is_empty() {
        upstream.insert("url".into(), url.into());
    }
    manifest.insert("upstream".into(), upstream.into());
    manifest.insert("pin".into(), pins.into());
    let text = toml::to_string(&manifest).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("toml.tmp");
    fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
    fs::rename(&tmp, &path).map_err(|e| format!("{}: {e}", path.display()))
}

/// Re-read every stored skill against its pin. Exit 1 on drift, a missing pin, or a skill
/// folder the manifest does not name.
fn verify() -> Result<u8, String> {
    let root = store_root();
    let Ok(entries) = fs::read_dir(&root) else {
        println!("no skill store at {}", root.display());
        return Ok(0);
    };
    let problems = verify_store(entries.flatten().map(|e| e.path()).collect());
    for p in &problems {
        println!("  ✗ {p}");
    }
    if problems.is_empty() {
        println!("✓ every stored skill matches its pin");
        Ok(0)
    } else {
        Ok(1)
    }
}

fn verify_store(mut packs: Vec<PathBuf>) -> Vec<String> {
    packs.sort();
    let mut problems = Vec::new();
    for dir in packs.into_iter().filter(|d| d.is_dir()) {
        let pack = dir
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let manifest = match fs::read_to_string(dir.join("pack.toml"))
            .map_err(|e| e.to_string())
            .and_then(|t| t.parse::<toml::Table>().map_err(|e| e.to_string()))
        {
            Ok(m) => m,
            Err(e) => {
                problems.push(format!("{pack}: pack.toml: {e}"));
                continue;
            }
        };
        let pins = manifest.get("pin").and_then(toml::Value::as_table);
        let named: Vec<&str> = manifest
            .get("skills")
            .and_then(toml::Value::as_array)
            .map(|a| a.iter().filter_map(toml::Value::as_str).collect())
            .unwrap_or_default();
        for name in &named {
            let pinned = pins
                .and_then(|p| p.get(*name))
                .and_then(|p| p.get("digest"))
                .and_then(toml::Value::as_str);
            match (pinned, read_tree(&dir.join("skills").join(name))) {
                (None, _) => problems.push(format!("{pack}/{name}: no pinned digest")),
                (_, Err(e)) => problems.push(format!("{pack}/{name}: {e}")),
                (Some(want), Ok(tree)) if digest(&tree) != want => {
                    problems.push(format!("{pack}/{name}: content differs from its pin"))
                }
                _ => {}
            }
        }
        if let Ok(found) = fs::read_dir(dir.join("skills")) {
            for entry in found.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if !named.contains(&name.as_str()) {
                    problems.push(format!("{pack}/{name}: in the store but not in pack.toml"));
                }
            }
        }
    }
    problems
}

fn update(args: &[String]) -> Result<u8, String> {
    if !args.is_empty() {
        return Err(
            "update takes no arguments; it refreshes every source in skills-lock.json".into(),
        );
    }
    let project = project_root()?;
    let lock_path = project.join("skills-lock.json");
    if !lock_path.is_file() {
        return Err(format!("no skills-lock.json in {}", project.display()));
    }
    let groups =
        lock_groups(&fs::read_to_string(&lock_path).map_err(|e| lock_error(&lock_path, e))?)?;
    if groups.is_empty() {
        return Err("skills-lock.json names no skills".into());
    }

    let mut jobs: Vec<Job> = Vec::new();
    // Which source each staged layout came from: the gate's approval key is per skill, so a
    // name that two sources both ship would otherwise take the other source's key.
    let mut job_sources: BTreeMap<PathBuf, String> = BTreeMap::new();
    let mut locks: Vec<(PathBuf, Vec<String>)> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    for (source, names) in &groups {
        let stage = fresh_stage("update")?;
        println!("staging {source} for {}", names.join(", "));
        stage_add(&stage, source, names, &["*".to_owned()])?;
        // `-a '*'` staged layouts for agents this project never installed for: only a
        // destination that already holds the skill is updated.
        let mut by_name: BTreeMap<String, Vec<Job>> = BTreeMap::new();
        for job in collect_jobs(&stage, &project)? {
            if matches!(job.before, DestState::Absent) {
                continue;
            }
            by_name.entry(job.skill.clone()).or_default().push(job);
        }
        for name in names {
            match by_name.remove(name) {
                Some(mut found) => {
                    for job in &found {
                        job_sources.insert(job.dest.clone(), source.clone());
                    }
                    jobs.append(&mut found);
                }
                None => skipped.push(format!("{source}/{name}: no installed destination")),
            }
        }
        locks.push((stage.join("skills-lock.json"), names.clone()));
    }
    for line in &skipped {
        println!("  skipped {line}");
    }
    if jobs.is_empty() {
        println!("nothing to update");
        return Ok(0);
    }

    let mut held = false;
    let mut error = false;
    for job in &jobs {
        let source = job_sources.get(&job.dest).cloned().unwrap_or_default();
        let key = format!("skills:{source}/{}", job.skill);
        match gate_tree(&key, Some(&job.staged), &job.tree, baseline(&job.before)) {
            0 => {}
            HELD => held = true,
            _ => error = true,
        }
    }
    if error {
        return Ok(1);
    }
    if held {
        println!("\nnothing updated: a skill is held");
        return Ok(HELD);
    }
    let published = publish(&jobs)?;
    if let Err(e) = write_locks(&project, &locks, &jobs) {
        published.rollback();
        return Err(format!("{e}; published layouts were rolled back"));
    }
    published.commit();
    println!("✓ updated {} skill layout(s)", jobs.len());
    Ok(0)
}

/// Merge every staged lock entry for a skill that was actually published.
fn write_locks(
    project: &Path,
    locks: &[(PathBuf, Vec<String>)],
    jobs: &[Job],
) -> Result<(), String> {
    for (stage_lock, names) in locks {
        let updated: Vec<String> = names
            .iter()
            .filter(|n| jobs.iter().any(|job| job.skill == **n))
            .cloned()
            .collect();
        if !updated.is_empty() {
            merge_lock(&project.join("skills-lock.json"), stage_lock, &updated)?;
        }
    }
    Ok(())
}

fn baseline(before: &DestState) -> Option<&Tree> {
    match before {
        DestState::Tree(tree) => Some(tree),
        _ => None,
    }
}

/// Gate every staged layout, then publish only what passed.
fn install(project: &Path, stage: &Path, jobs: &[Job], package: &str) -> Result<u8, String> {
    match gate_jobs(jobs, package) {
        0 => {}
        HELD => {
            println!("\nnothing installed: {package} is held");
            return Ok(HELD);
        }
        code => return Ok(code),
    }
    let names: Vec<String> = jobs.iter().map(|job| job.skill.clone()).collect();
    let published = publish(jobs)?;
    if let Err(e) = merge_lock(
        &project.join("skills-lock.json"),
        &stage.join("skills-lock.json"),
        &names,
    ) {
        published.rollback();
        return Err(format!("{e}; published layouts were rolled back"));
    }
    published.commit();
    println!("✓ installed {} skill layout(s)", jobs.len());
    Ok(0)
}

/// The staged layouts: every `<agent-dir>/skills/<name>` the upstream CLI wrote.
fn collect_jobs(stage: &Path, project: &Path) -> Result<Vec<Job>, String> {
    let mut jobs = Vec::new();
    for entry in fs::read_dir(stage).map_err(|e| format!("{}: {e}", stage.display()))? {
        let entry = entry.map_err(|e| format!("{}: {e}", stage.display()))?;
        let agent_dir = entry.path();
        let agent_name = agent_dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let skills = agent_dir.join("skills");
        if !skills.is_dir() {
            continue;
        }
        for skill_entry in
            fs::read_dir(&skills).map_err(|e| format!("{}: {e}", skills.display()))?
        {
            let skill_entry = skill_entry.map_err(|e| format!("{}: {e}", skills.display()))?;
            let path = skill_entry.path();
            if !path.join("SKILL.md").is_file() {
                continue;
            }
            let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else {
                continue;
            };
            let dest = project.join(&agent_name).join("skills").join(&name);
            jobs.push(Job {
                skill: name,
                before: dest_state(&dest)?,
                tree: read_tree(&path)?,
                staged: path,
                dest,
            });
        }
    }
    Ok(jobs)
}

fn dest_state(dest: &Path) -> Result<DestState, String> {
    match fs::symlink_metadata(dest) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(DestState::Absent),
        Err(e) => Err(format!("{}: {e}", dest.display())),
        Ok(meta) if meta.file_type().is_symlink() => Ok(DestState::Other),
        Ok(_) => Ok(DestState::Tree(read_tree(dest)?)),
    }
}

/// Write every screened tree, replacing what was there. Backups are kept until the caller
/// commits; a crash between renames is not covered, and the lock is written last.
#[derive(Debug)]
struct Published {
    backup_dir: PathBuf,
    done: Vec<(PathBuf, Option<PathBuf>)>,
}

impl Published {
    fn rollback(&self) {
        for (dest, backup) in self.done.iter().rev() {
            let _ = fs::remove_dir_all(dest);
            if let Some(backup) = backup {
                let _ = fs::rename(backup, dest);
            }
        }
    }

    fn commit(self) {
        let _ = fs::remove_dir_all(&self.backup_dir);
    }
}

fn publish(jobs: &[Job]) -> Result<Published, String> {
    let backup_dir = fresh_stage("backup")?;
    let mut published = Published {
        backup_dir,
        done: Vec::new(),
    };
    if let Err(message) = publish_all(&published.backup_dir.clone(), jobs, &mut published.done) {
        published.rollback();
        return Err(format!("{message}; published layouts were rolled back"));
    }
    Ok(published)
}

fn publish_all(
    backup_dir: &Path,
    jobs: &[Job],
    done: &mut Vec<(PathBuf, Option<PathBuf>)>,
) -> Result<(), String> {
    for job in jobs {
        // The destination must still hold what was screened against.
        match (&job.before, dest_state(&job.dest)?) {
            (DestState::Tree(before), DestState::Tree(now)) if digest(before) == digest(&now) => {}
            (DestState::Absent, DestState::Absent) => {}
            (DestState::Other, DestState::Other) => {
                return Err(format!(
                    "{} is a symlink; another installer owns it",
                    job.dest.display()
                ))
            }
            _ => return Err(format!("{} changed during the run", job.dest.display())),
        }
        let agent = job
            .dest
            .parent()
            .and_then(|p| p.parent())
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let backup = if job.dest.symlink_metadata().is_ok() {
            let to = backup_dir.join(format!("{agent}-{}", job.skill));
            fs::rename(&job.dest, &to).map_err(|e| format!("{}: {e}", job.dest.display()))?;
            Some(to)
        } else {
            None
        };
        // Recorded before the write, so a failure part-way through still restores the backup.
        done.push((job.dest.clone(), backup));
        write_tree(&job.dest, &job.tree)?;
    }
    Ok(())
}

/// Materialize one screened tree. The tree came from `read_tree`, whose keys are directory
/// entries, so `..` cannot appear; the guard keeps that true even if that changes.
fn write_tree(dest: &Path, tree: &Tree) -> Result<(), String> {
    fs::create_dir_all(dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    for (rel, file) in tree {
        if Path::new(rel)
            .components()
            .any(|c| c == std::path::Component::ParentDir)
        {
            return Err(format!("{rel}: refuses a path outside the skill directory"));
        }
        let path = dest.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        fs::write(&path, &file.bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        let mode = if file.executable { 0o755 } else { 0o644 };
        fs::set_permissions(&path, fs::Permissions::from_mode(mode))
            .map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(())
}

/// Replace the named entries with the staged ones, verbatim, keeping every other entry.
fn merge_lock(project_lock: &Path, stage_lock: &Path, names: &[String]) -> Result<(), String> {
    let staged: Value = serde_json::from_str(
        &fs::read_to_string(stage_lock).map_err(|e| lock_error(stage_lock, e))?,
    )
    .map_err(|e| format!("{}: {e}", stage_lock.display()))?;
    let staged_skills = staged["skills"]
        .as_object()
        .ok_or_else(|| format!("{}: no skills table", stage_lock.display()))?;
    let mut lock: Value = match fs::read_to_string(project_lock) {
        Ok(text) => serde_json::from_str(&text).map_err(|e| lock_error_text(project_lock, e))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            serde_json::json!({"version": 3, "skills": {}})
        }
        Err(e) => return Err(lock_error(project_lock, e)),
    };
    if lock["skills"].is_null() {
        lock["skills"] = serde_json::json!({});
    }
    let skills = lock["skills"]
        .as_object_mut()
        .ok_or_else(|| format!("{}: skills is not a table", project_lock.display()))?;
    for name in names {
        let entry = staged_skills
            .get(name)
            .ok_or_else(|| format!("{}: no staged entry for '{name}'", stage_lock.display()))?;
        skills.insert(name.clone(), entry.clone());
    }
    let text = serde_json::to_string_pretty(&lock).map_err(|e| e.to_string())?;
    let tmp = project_lock.with_extension("json.tmp");
    fs::write(&tmp, format!("{text}\n")).map_err(|e| format!("{}: {e}", tmp.display()))?;
    fs::rename(&tmp, project_lock).map_err(|e| format!("{}: {e}", project_lock.display()))?;
    Ok(())
}

/// Group locked skills by their source, keeping `sourceUrl` distinctions apart: one stage per
/// group, staging only the names that group holds.
fn lock_groups(lock_text: &str) -> Result<Vec<(String, Vec<String>)>, String> {
    let lock: Value =
        serde_json::from_str(lock_text).map_err(|e| format!("skills-lock.json: {e}"))?;
    let skills = lock["skills"]
        .as_object()
        .ok_or_else(|| "skills-lock.json: no skills table".to_owned())?;
    let mut groups: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    for (name, entry) in skills {
        let source = entry["source"]
            .as_str()
            .ok_or_else(|| format!("skills-lock.json: '{name}' has no source"))?
            .to_owned();
        let url = entry["sourceUrl"].as_str().unwrap_or_default().to_owned();
        groups.entry((source, url)).or_default().push(name.clone());
    }
    Ok(groups
        .into_iter()
        .map(|((source, _), names)| (source, names))
        .collect())
}

fn stage_add(
    stage: &Path,
    package: &str,
    skills: &[String],
    agents: &[String],
) -> Result<(), String> {
    let mut args: Vec<String> = vec!["add".into(), package.into()];
    for skill in skills {
        args.push("-s".into());
        args.push(skill.clone());
    }
    for agent in agents {
        args.push("-a".into());
        args.push(agent.clone());
    }
    args.push("--copy".into());
    args.push("-y".into());
    let output = Command::new("npx")
        .arg("--yes")
        .arg("skills")
        .args(&args)
        .current_dir(stage)
        .output()
        .map_err(|e| format!("npx skills {}: {e}", args.join(" ")))?;
    if !output.status.success() {
        let tail = String::from_utf8_lossy(&output.stderr);
        let tail: String = tail
            .lines()
            .rev()
            .take(4)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n");
        return Err(format!("npx skills {} failed:\n{tail}", args.join(" ")));
    }
    Ok(())
}

fn project_root() -> Result<PathBuf, String> {
    let root = Command::new("git")
        .arg("rev-parse")
        .arg("--show-toplevel")
        .output()
        .map_err(|e| format!("git rev-parse: {e}"))?;
    if root.status.success() {
        return Ok(PathBuf::from(String::from_utf8_lossy(&root.stdout).trim()));
    }
    std::env::current_dir().map_err(|e| format!("cwd: {e}"))
}

fn fresh_stage(kind: &str) -> Result<PathBuf, String> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let staged = super::state_dir().join("staged");
    // A stage is only useful while the run that made it is alive; keep the tree small.
    if let Ok(entries) = fs::read_dir(&staged) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("skills-") {
                let age = entry
                    .metadata()
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|m| m.elapsed().ok());
                if age.is_some_and(|age| age.as_secs() > 86_400) {
                    let _ = fs::remove_dir_all(entry.path());
                }
            }
        }
    }
    let dir = staged.join(format!("skills-{kind}-{}-{nanos}", std::process::id()));
    fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    Ok(dir)
}

fn lock_error(path: &Path, e: std::io::Error) -> String {
    format!("{}: {e}", path.display())
}

fn lock_error_text(path: &Path, e: serde_json::Error) -> String {
    format!("{}: {e}", path.display())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intake::TreeFile;

    #[test]
    fn a_store_pack_is_named_after_the_repository() {
        assert_eq!(store_pack("wheresryan22/anatomy").unwrap(), "anatomy");
        assert_eq!(
            store_pack("https://github.com/o/Skill-Set.git/").unwrap(),
            "skill-set"
        );
        assert!(store_pack("o/under_score").is_err());
    }

    #[test]
    fn verify_reports_drift_missing_pins_and_unnamed_folders() {
        let dir = scratch("verify");
        let pack = dir.join("p");
        let mut tree = Tree::new();
        tree.insert(
            "SKILL.md".into(),
            TreeFile {
                bytes: b"body".to_vec(),
                executable: false,
            },
        );
        write_tree(&pack.join("skills/ok"), &tree).unwrap();
        write_tree(&pack.join("skills/drifted"), &tree).unwrap();
        write_tree(&pack.join("skills/stray"), &tree).unwrap();
        let pin = digest(&tree);
        fs::write(
            pack.join("pack.toml"),
            format!(
                "name = \"p\"\nskills = [\"ok\", \"drifted\", \"unpinned\"]\n\
                 [pin.ok]\ndigest = \"{pin}\"\n[pin.drifted]\ndigest = \"{pin}\"\n"
            ),
        )
        .unwrap();
        fs::write(pack.join("skills/drifted/SKILL.md"), "changed").unwrap();
        let problems = verify_store(vec![pack]);
        assert_eq!(problems.len(), 3, "{problems:?}");
        assert!(problems[0].contains("drifted") && problems[0].contains("differs"));
        assert!(problems[1].contains("unpinned") && problems[1].contains("no pinned digest"));
        assert!(problems[2].contains("stray"));
        fs::remove_dir_all(&dir).unwrap();
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sjel-skills-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn write_tree_round_trips_bytes_and_the_executable_bit() {
        let dir = scratch("write");
        let mut tree = Tree::new();
        tree.insert(
            "SKILL.md".into(),
            TreeFile {
                bytes: b"body\n".to_vec(),
                executable: false,
            },
        );
        tree.insert(
            "scripts/run.sh".into(),
            TreeFile {
                bytes: b"echo hi\n".to_vec(),
                executable: true,
            },
        );
        write_tree(&dir, &tree).unwrap();
        let read = read_tree(&dir).unwrap();
        assert_eq!(digest(&read), digest(&tree));
        let meta = fs::symlink_metadata(dir.join("scripts/run.sh")).unwrap();
        assert!(meta.permissions().mode() & 0o111 != 0);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn write_tree_refuses_a_escaping_relative_path() {
        let dir = scratch("escape");
        let mut tree = Tree::new();
        tree.insert(
            "../outside".into(),
            TreeFile {
                bytes: b"x".to_vec(),
                executable: false,
            },
        );
        let err = write_tree(&dir, &tree).unwrap_err();
        assert!(err.contains("outside the skill directory"), "{err}");
        assert!(!dir.parent().unwrap().join("outside").exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_symlinked_destination_is_refused_rather_than_replaced() {
        let dir = scratch("symlink");
        let project = dir.join("project");
        let pack = dir.join("pack/demo");
        fs::create_dir_all(&pack).unwrap();
        fs::write(pack.join("SKILL.md"), "pack text").unwrap();
        let dest = project.join(".claude/skills/demo");
        fs::create_dir_all(dest.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&pack, &dest).unwrap();
        let mut tree = Tree::new();
        tree.insert(
            "SKILL.md".into(),
            TreeFile {
                bytes: b"new".to_vec(),
                executable: false,
            },
        );
        let job = Job {
            skill: "demo".into(),
            dest: dest.clone(),
            staged: dir.join("staged"),
            before: dest_state(&dest).unwrap(),
            tree,
        };
        let err = publish(&[job]).unwrap_err();
        assert!(err.contains("another installer owns it"), "{err}");
        assert_eq!(
            fs::read_to_string(pack.join("SKILL.md")).unwrap(),
            "pack text"
        );
        assert!(fs::symlink_metadata(&dest)
            .unwrap()
            .file_type()
            .is_symlink());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_failed_second_layout_restores_the_first() {
        let dir = scratch("rollback");
        let project = dir.join("project");
        let mut jobs = Vec::new();
        for agent in [".claude", ".agents"] {
            let dest = project.join(agent).join("skills/demo");
            fs::create_dir_all(&dest).unwrap();
            fs::write(dest.join("SKILL.md"), "old").unwrap();
            let mut tree = Tree::new();
            tree.insert(
                "SKILL.md".into(),
                TreeFile {
                    bytes: b"new".to_vec(),
                    executable: false,
                },
            );
            jobs.push(Job {
                skill: "demo".into(),
                before: dest_state(&dest).unwrap(),
                dest,
                staged: dir.join("staged"),
                tree,
            });
        }
        // The second destination changes after screening, so it fails after the first was
        // written; both must end up as they were.
        fs::write(jobs[1].dest.join("SKILL.md"), "changed after screening").unwrap();
        let err = publish(&jobs).unwrap_err();
        assert!(err.contains("changed during the run"), "{err}");
        assert_eq!(
            fs::read_to_string(jobs[0].dest.join("SKILL.md")).unwrap(),
            "old"
        );
        assert_eq!(
            fs::read_to_string(jobs[1].dest.join("SKILL.md")).unwrap(),
            "changed after screening"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn lock_groups_split_sources_but_keep_their_skills_together() {
        let lock = r#"{"version":3,"skills":{
            "a":{"source":"octo/pkg","sourceUrl":"https://github.com/octo/pkg.git"},
            "b":{"source":"octo/pkg","sourceUrl":"https://github.com/octo/pkg.git"},
            "c":{"source":"https://example.org/s.git","sourceUrl":"https://example.org/s.git"}
        }}"#;
        let groups = lock_groups(lock).unwrap();
        assert_eq!(groups.len(), 2);
        let octo = groups.iter().find(|(s, _)| s == "octo/pkg").unwrap();
        assert_eq!(octo.1, vec!["a".to_owned(), "b".to_owned()]);
    }

    #[test]
    fn merge_lock_replaces_only_the_named_entries_verbatim() {
        let dir = scratch("merge");
        let stage = dir.join("stage");
        fs::create_dir_all(dir.join("project")).unwrap();
        fs::create_dir_all(&stage).unwrap();
        fs::write(
            dir.join("project/skills-lock.json"),
            r#"{"version":3,"skills":{"old":{"source":"x"},"keep":{"source":"y"}}}"#,
        )
        .unwrap();
        fs::write(
            stage.join("skills-lock.json"),
            r#"{"version":3,"skills":{"old":{"source":"z","skillFolderHash":"abc"}}}"#,
        )
        .unwrap();
        let project_lock = dir.join("project/skills-lock.json");
        merge_lock(
            &project_lock,
            &stage.join("skills-lock.json"),
            &["old".into()],
        )
        .unwrap();
        let merged: Value =
            serde_json::from_str(&fs::read_to_string(&project_lock).unwrap()).unwrap();
        assert_eq!(merged["skills"]["old"]["source"], "z");
        assert_eq!(merged["skills"]["old"]["skillFolderHash"], "abc");
        assert_eq!(merged["skills"]["keep"]["source"], "y");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn publish_replaces_a_destination_and_refuses_one_that_changed() {
        let dir = scratch("publish");
        let project = dir.join("project");
        let dest = project.join(".claude/skills/demo");
        fs::create_dir_all(&dest).unwrap();
        fs::write(dest.join("SKILL.md"), "old").unwrap();
        let mut tree = Tree::new();
        tree.insert(
            "SKILL.md".into(),
            TreeFile {
                bytes: b"new".to_vec(),
                executable: false,
            },
        );
        let job = Job {
            skill: "demo".into(),
            dest: dest.clone(),
            staged: dir.join("staged"),
            before: DestState::Tree(read_tree(&dest).unwrap()),
            tree,
        };
        let published = publish(&[job]).unwrap();
        published.commit();
        assert_eq!(fs::read_to_string(dest.join("SKILL.md")).unwrap(), "new");

        // A destination that changed since screening aborts and leaves it alone.
        let before = DestState::Tree(read_tree(&dest).unwrap());
        fs::write(dest.join("SKILL.md"), "changed after screening").unwrap();
        let mut tree = Tree::new();
        tree.insert(
            "SKILL.md".into(),
            TreeFile {
                bytes: b"newer".to_vec(),
                executable: false,
            },
        );
        let job = Job {
            skill: "demo".into(),
            dest: dest.clone(),
            staged: dir.join("staged"),
            before,
            tree,
        };
        let err = publish(&[job]).unwrap_err();
        assert!(err.contains("changed during the run"), "{err}");
        assert_eq!(
            fs::read_to_string(dest.join("SKILL.md")).unwrap(),
            "changed after screening"
        );
        fs::remove_dir_all(&dir).unwrap();
    }
}
