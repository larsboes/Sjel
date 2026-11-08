//! Read-only repository maintenance evidence, derived from tracked inputs.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::process::{Command, ExitCode};
use std::time::Duration;

use regex::Regex;
use semver::{Version, VersionReq};
use serde::Serialize;
use serde_json::{json, Value};

const HELP: &str = "sjel maintenance [--json] [--unused] [--freshness] [--licenses]\n\
    Inventory tracked workspaces, Bun projects, API contracts and stale-path candidates.\n\
    --unused     run cargo-machete without fixes or Cargo metadata\n\
    --freshness  query crates.io and run Bun outdated; change no manifests or lockfiles\n\
    --licenses   run cargo-deny license inventory offline for the current Rust host\n\
    Default: offline inventory only. Missing or failed requested analyzers exit 2.";

#[derive(Serialize)]
struct Report {
    workspaces: Vec<String>,
    bun_projects: Vec<String>,
    api_contracts: Vec<String>,
    plans: Vec<(String, usize)>,
    path_candidates: Vec<Value>,
    dependencies: Vec<Dependency>,
    analyses: Vec<Value>,
}

#[derive(Serialize)]
struct Dependency {
    manifest: String,
    name: String,
    requirement: String,
    source: String,
    revision: Option<String>,
    locked: Vec<String>,
    latest: Option<String>,
    status: String,
}

fn read_toml(path: &Path) -> Result<toml::Value, String> {
    let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
    toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

fn tracked(root: &Path) -> Result<Vec<String>, String> {
    let output = Command::new("git")
        .args(["ls-files", "-z"])
        .current_dir(root)
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err("git could not inventory tracked files".into());
    }
    Ok(String::from_utf8(output.stdout)
        .map_err(|e| e.to_string())?
        .split('\0')
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect())
}

fn requirement(value: &toml::Value) -> &str {
    value
        .as_str()
        .or_else(|| value.get("version").and_then(toml::Value::as_str))
        .unwrap_or("")
}

fn classify(requirement: &str, locked: &[String], latest: &str) -> Result<&'static str, String> {
    let requirement = VersionReq::parse(requirement).map_err(|e| e.to_string())?;
    let latest = Version::parse(latest).map_err(|e| e.to_string())?;
    if !latest.pre.is_empty() {
        return Err("registry supplied no stable version".into());
    }
    if locked.is_empty() {
        return Err("no matching version in the workspace lockfile".into());
    }
    if locked
        .iter()
        .any(|v| Version::parse(v).is_ok_and(|v| v == latest))
    {
        return Ok("current");
    }
    Ok(if requirement.matches(&latest) {
        "compatible_update"
    } else {
        "migration"
    })
}

fn inventory(root: &Path, files: &[String]) -> Result<Report, String> {
    let mut manifests = BTreeMap::new();
    for file in files.iter().filter(|p| p.ends_with("Cargo.toml")) {
        manifests.insert(file.clone(), read_toml(&root.join(file))?);
    }
    let workspaces: Vec<_> = manifests
        .iter()
        .filter(|(_, v)| v.get("workspace").is_some())
        .map(|(path, _)| path.clone())
        .collect();
    let mut dependencies = Vec::new();
    let mut locks = BTreeMap::new();
    for owner in &workspaces {
        let path = root
            .join(Path::new(owner).parent().unwrap())
            .join("Cargo.lock");
        locks.insert(owner, read_toml(&path)?);
    }
    for (manifest, data) in &manifests {
        if data.get("package").is_none() {
            continue;
        }
        let owner = workspaces
            .iter()
            .filter(|workspace| {
                Path::new(manifest)
                    .starts_with(Path::new(workspace).parent().unwrap_or(Path::new("")))
            })
            .max_by_key(|path| path.len());
        let Some(owner) = owner else {
            return Err(format!("no workspace for {manifest}"));
        };
        let workspace = &manifests[owner];
        let lock = &locks[owner];
        let tables = dependency_tables(data);
        for table in tables {
            for (alias, declared) in table {
                let value =
                    if declared.get("workspace").and_then(toml::Value::as_bool) == Some(true) {
                        workspace
                            .get("workspace")
                            .and_then(|v| v.get("dependencies"))
                            .and_then(|v| v.get(alias))
                            .ok_or_else(|| format!("unresolved workspace dependency {alias}"))?
                    } else {
                        declared
                    };
                let source = if value.get("path").is_some() {
                    "path"
                } else if value.get("git").is_some() {
                    "git"
                } else if value.get("registry").is_some() {
                    "registry_override"
                } else {
                    "crates.io"
                };
                let name = value
                    .get("package")
                    .and_then(toml::Value::as_str)
                    .unwrap_or(alias)
                    .to_owned();
                let req = requirement(value).to_owned();
                let bound = VersionReq::parse(&req).ok();
                let locked = lock
                    .get("package")
                    .and_then(toml::Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter(|p| p.get("name").and_then(toml::Value::as_str) == Some(&name))
                    .filter_map(|p| p.get("version").and_then(toml::Value::as_str))
                    .filter(|v| {
                        bound
                            .as_ref()
                            .is_none_or(|b| Version::parse(v).is_ok_and(|v| b.matches(&v)))
                    })
                    .map(str::to_owned)
                    .collect();
                dependencies.push(Dependency {
                    manifest: manifest.clone(),
                    name,
                    requirement: req,
                    source: source.into(),
                    revision: value
                        .get("rev")
                        .or_else(|| value.get("tag"))
                        .or_else(|| value.get("branch"))
                        .and_then(toml::Value::as_str)
                        .map(str::to_owned),
                    locked,
                    latest: None,
                    status: if source == "path" {
                        "local"
                    } else {
                        "not_checked"
                    }
                    .into(),
                });
            }
        }
    }
    let mut report = Report {
        workspaces,
        dependencies,
        bun_projects: Vec::new(),
        api_contracts: Vec::new(),
        plans: Vec::new(),
        path_candidates: Vec::new(),
        analyses: Vec::new(),
    };
    let path_pattern = Regex::new(r"`((?:crates|tools|apps|libs|capabilities|services|Packs|schemas|website|docs|research)/[A-Za-z0-9_./-]+)`")
        .map_err(|e| e.to_string())?;
    for file in files {
        if file.ends_with("/package.json")
            && files.contains(&file.replace("package.json", "bun.lock"))
        {
            report
                .bun_projects
                .push(file.trim_end_matches("/package.json").into());
        }
        if file.starts_with("crates/") && file.ends_with("-api/Cargo.toml") {
            report.api_contracts.push(file.clone());
        }
        if file == "PLAN.md" || (file.starts_with("docs/plans/") && file.ends_with(".md")) {
            let text = fs::read_to_string(root.join(file)).map_err(|e| e.to_string())?;
            report.plans.push((file.clone(), text.lines().count()));
            for (line, content) in text.lines().enumerate() {
                for found in path_pattern.captures_iter(content) {
                    let path = &found[1];
                    if !root.join(path).exists()
                        && !root
                            .join(Path::new(file).parent().unwrap())
                            .join(path)
                            .exists()
                    {
                        report.path_candidates.push(json!({"document": file, "line": line + 1, "path": path,
                            "historical_context": (["histor", "retir", "compatib", "git show"].iter().any(|s| content.to_lowercase().contains(s)))}));
                    }
                }
            }
        }
    }
    Ok(report)
}

fn dependency_tables(data: &toml::Value) -> Vec<&toml::Table> {
    let mut owners = vec![data];
    if let Some(targets) = data.get("target").and_then(toml::Value::as_table) {
        owners.extend(targets.values());
    }
    owners
        .into_iter()
        .flat_map(|owner| {
            ["dependencies", "dev-dependencies", "build-dependencies"]
                .into_iter()
                .filter_map(move |key| owner.get(key).and_then(toml::Value::as_table))
        })
        .collect()
}

fn execute(root: &Path, args: &[&str]) -> Value {
    match Command::new(args[0])
        .args(&args[1..])
        .current_dir(root)
        .output()
    {
        Ok(output) => json!({"command": args, "exit": output.status.code(),
            "stdout": String::from_utf8_lossy(&output.stdout), "stderr": String::from_utf8_lossy(&output.stderr),
            "status": if output.status.success() || (args[0] == "cargo-machete" && output.status.code() == Some(1)) { "reported" } else { "failed" }}),
        Err(e) => json!({"command": args, "status": "unavailable", "error": e.to_string()}),
    }
}

fn freshness(report: &mut Report, root: &Path) -> Result<(), String> {
    let client = sjel_http::client(
        sjel_http::Purpose::new("maintenance"),
        Duration::from_secs(5),
    )
    .map_err(|e| e.to_string())?;
    let names: BTreeSet<_> = report
        .dependencies
        .iter()
        .filter(|d| d.source == "crates.io")
        .map(|d| d.name.clone())
        .collect();
    let mut latest = BTreeMap::new();
    for name in names {
        let result = client
            .get(format!("https://crates.io/api/v1/crates/{name}"))
            .send()
            .and_then(|r| r.error_for_status())
            .and_then(|r| r.json::<Value>())
            .map_err(|e| e.to_string())
            .and_then(|v| {
                v["crate"]["max_stable_version"]
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| "no stable version in registry response".into())
            });
        latest.insert(name, result);
    }
    let mut errors = Vec::new();
    for row in &mut report.dependencies {
        if row.source != "crates.io" {
            continue;
        }
        match &latest[&row.name] {
            Ok(version) => {
                row.latest = Some(version.clone());
                match classify(&row.requirement, &row.locked, version) {
                    Ok(status) => row.status = status.into(),
                    Err(e) => {
                        row.status = "unknown".into();
                        errors.push(e);
                    }
                }
            }
            Err(error) => {
                row.status = "unknown".into();
                errors.push(format!("{}: {error}", row.name));
            }
        }
    }
    report.analyses.push(json!({"analyzer": "crates.io", "status": if errors.is_empty() { "reported" } else { "failed" }, "errors": errors}));
    for project in &report.bun_projects {
        let mut analysis = execute(
            &root.join(project),
            &[
                "bun",
                "--no-env-file",
                "outdated",
                "--no-save",
                "--ignore-scripts",
            ],
        );
        analysis["project"] = json!(project);
        report.analyses.push(analysis);
    }
    Ok(())
}

/// Report repository maintenance inputs without changing source, lockfiles or installations.
pub fn run(root: &Path, args: &[String]) -> ExitCode {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{HELP}");
        return ExitCode::SUCCESS;
    }
    if args
        .iter()
        .any(|a| !["--json", "--unused", "--freshness", "--licenses"].contains(&a.as_str()))
    {
        eprintln!("{HELP}");
        return ExitCode::from(2);
    }
    let result = tracked(root).and_then(|files| inventory(root, &files));
    let mut report = match result {
        Ok(r) => r,
        Err(e) => {
            eprintln!("maintenance: {e}");
            return ExitCode::from(2);
        }
    };
    if args.iter().any(|a| a == "--unused") {
        for workspace in &report.workspaces {
            let dir = Path::new(workspace).parent().unwrap();
            let scan = if dir.as_os_str().is_empty() {
                "crates"
            } else {
                dir.to_str().unwrap()
            };
            let mut analysis = execute(
                root,
                &["cargo-machete", "--json", "--skip-target-dir", scan],
            );
            analysis["workspace"] = json!(workspace);
            report.analyses.push(analysis);
        }
    }
    if args.iter().any(|a| a == "--freshness") {
        if let Err(error) = freshness(&mut report, root) {
            report
                .analyses
                .push(json!({"analyzer": "freshness", "status": "failed", "error": error}));
        }
    }
    if args.iter().any(|a| a == "--licenses") {
        let host = Command::new("rustc")
            .arg("-vV")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .and_then(|s| {
                s.lines()
                    .find_map(|line| line.strip_prefix("host: ").map(str::to_owned))
            });
        match host {
            Some(host) => for workspace in &report.workspaces {
                let mut analysis = execute(root, &["cargo", "deny", "--manifest-path", workspace,
                    "--locked", "--offline", "--target", &host, "list", "--format", "json"]);
                analysis["workspace"] = json!(workspace);
                analysis["target"] = json!(host);
                report.analyses.push(analysis);
            },
            None => report.analyses.push(json!({"analyzer": "licenses", "status": "unavailable", "error": "Rust host target could not be determined"})),
        }
    }
    let incomplete = report.analyses.iter().any(|a| a["status"] != "reported");
    if args.iter().any(|a| a == "--json") {
        match serde_json::to_string_pretty(&report) {
            Ok(text) => println!("{text}"),
            Err(e) => {
                eprintln!("maintenance: {e}");
                return ExitCode::from(2);
            }
        }
    } else {
        println!(
            "{} workspaces, {} Bun projects, {} API contracts",
            report.workspaces.len(),
            report.bun_projects.len(),
            report.api_contracts.len()
        );
        println!(
            "{} plan lines; {} unresolved path candidates (review context before editing)",
            report.plans.iter().map(|p| p.1).sum::<usize>(),
            report.path_candidates.len()
        );
        for row in &report.path_candidates {
            println!("{}:{} {}", row["document"], row["line"], row["path"]);
        }
        for result in &report.analyses {
            println!("{result}");
        }
        if report.analyses.is_empty() {
            println!("Offline inventory; unused dependencies and freshness were not checked.");
        }
    }
    if incomplete {
        ExitCode::from(2)
    } else {
        ExitCode::SUCCESS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inventory_resolves_nested_workspaces_and_only_reads_listed_inputs() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("sjel-maintenance-{}-{stamp}", std::process::id()));
        let inputs = [
            (
                "Cargo.toml",
                "[workspace]\nmembers=['crates/*']\n[workspace.dependencies]\nserde='1'\n",
            ),
            ("Cargo.lock", "[[package]]\nname='serde'\nversion='1.0.0'\n"),
            (
                "crates/domain-api/Cargo.toml",
                "[package]\nname='domain-api'\n[dependencies]\nserde.workspace=true\n",
            ),
            (
                "apps/gpui/Cargo.toml",
                "[workspace]\n[package]\nname='app'\n[dependencies]\nserde='0.9'\n",
            ),
            (
                "apps/gpui/Cargo.lock",
                "[[package]]\nname='serde'\nversion='0.9.0'\n",
            ),
            ("Packs/demo/package.json", "{}"),
            ("Packs/demo/bun.lock", "{}"),
            (
                "PLAN.md",
                "Current `crates/missing/src/lib.rs`.\nHistorical `libs/retired`.\n",
            ),
        ];
        for (path, text) in &inputs {
            let path = root.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        fs::write(root.join("untracked-Cargo.toml"), "not TOML").unwrap();
        let files = inputs
            .iter()
            .map(|(path, _)| path.to_string())
            .collect::<Vec<_>>();
        let report = inventory(&root, &files).unwrap();
        assert_eq!(report.workspaces.len(), 2);
        assert_eq!(report.api_contracts.len(), 1);
        assert_eq!(report.bun_projects, ["Packs/demo"]);
        assert_eq!(report.path_candidates.len(), 2);
        assert_eq!(report.path_candidates[1]["historical_context"], true);
        let root_dep = report
            .dependencies
            .iter()
            .find(|d| d.manifest.starts_with("crates/"))
            .unwrap();
        let app_dep = report
            .dependencies
            .iter()
            .find(|d| d.manifest.starts_with("apps/"))
            .unwrap();
        assert_eq!(root_dep.locked, ["1.0.0"]);
        assert_eq!(app_dep.locked, ["0.9.0"]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn compatible_updates_and_migrations_are_distinct() {
        assert_eq!(
            classify("^0.8", &["0.8.1".into()], "0.8.2").unwrap(),
            "compatible_update"
        );
        assert_eq!(
            classify("^0.8", &["0.8.1".into()], "0.9.0").unwrap(),
            "migration"
        );
        assert_eq!(
            classify("=0.8.1", &["0.8.1".into()], "0.8.1").unwrap(),
            "current"
        );
        assert!(classify("^0.8", &[], "0.9.0-rc.1").is_err());
        assert!(classify("^0.8", &[], "0.8.2").is_err());
        assert_eq!(
            classify("=0.8.1", &["0.8.1".into()], "0.8.2").unwrap(),
            "migration"
        );
    }
}
