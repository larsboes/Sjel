//! Project setup, private delivery libraries and reviewed knowledge publication.

use std::{
    fs,
    io::Write,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    process::Command,
};

use markdown_root::{frontmatter_spanned, MarkdownRoot};
use serde::Deserialize;

use crate::{ops, record, Kind, Projects, Record};

type Fallible<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Adoption {
    library: PathBuf,
    assets: Vec<Asset>,
    projects: Vec<ProjectIndex>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Asset {
    source: PathBuf,
    target: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectIndex {
    source: PathBuf,
    name: String,
}

/// Run the operator-only workspace commands.
///
/// # Errors
/// Inputs are invalid, a source or destination conflicts, or a capability write fails.
pub async fn run(projects: &Projects, args: &[String]) -> Fallible<()> {
    match args.first().map(String::as_str) {
        Some("adopt") if args.len() == 2 => adopt(projects, Path::new(&args[1])).await,
        Some("setup") if args.len() == 3 => setup(projects, &args[1], Path::new(&args[2])).await,
        Some("status") if args.len() == 1 || (args.len() == 2 && args[1] == "--github") => {
            status(projects, args.len() == 2).await
        }
        Some("lesson") if args.get(1).map(String::as_str) == Some("draft") && (args.len() == 5 || (args.len() == 7 && args[5] == "--proposal")) => {
            draft(projects, &args[2], &args[3], &args[4], args.get(6).map(Path::new)).await
        }
        Some("lesson") if args.get(1).map(String::as_str) == Some("publish") && args.len() == 8 && args[6] == "--declassify" => {
            publish(projects, &args[2], Path::new(&args[3]), Path::new(&args[4]), &args[5], &args[7]).await
        }
        _ => Err("usage: projects workspace adopt <manifest.json> | setup <name> <template-directory> | status [--github] | lesson draft <project> <title> <evidence> | lesson publish <draft-id> <reviewed-note.md> <vault-root> <relative-note.md> --declassify <owner-rationale>".into()),
    }
}

async fn adopt(projects: &Projects, manifest: &Path) -> Fallible<()> {
    let config: Adoption = serde_json::from_slice(&fs::read(manifest)?)?;
    let overlay = sjel_config::overlay_root().ok_or("no private overlay configured")?;
    let resources = overlay.join("resources");
    fs::create_dir_all(&resources)?;
    let resources = resources.canonicalize()?;
    if !config.library.starts_with(&resources) || config.library == resources {
        return Err(
            "the delivery library must be under the private overlay's resources directory".into(),
        );
    }
    checked_relative(
        config
            .library
            .strip_prefix(&resources)?
            .to_str()
            .ok_or("invalid library path")?,
    )?;
    let mut copies = Vec::new();
    for asset in &config.assets {
        let target = checked_relative(&asset.target)?;
        collect_files(&asset.source, &config.library.join(target), &mut copies)?;
    }
    // Check the complete batch before the first write; a retry never replaces edits.
    for (source, target) in &copies {
        contained_destination(&resources, target)?;
        match fs::read(target) {
            Ok(current) if current != fs::read(source)? => {
                return Err(format!("{} differs; nothing overwritten", target.display()).into())
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    let mut hubs = Vec::new();
    for project in config.projects {
        let name = record::check_name(&project.name)?;
        let content = fs::read_to_string(&project.source)?;
        let mut content = record::set_field(&content, "type", Some("project"))?;
        content = record::set_field(&content, "class", Some("c2"))?;
        content = record::set_field(&content, "status", Some("active"))?;
        content = record::set_field(
            &content,
            "source_index",
            Some(&project.source.to_string_lossy()),
        )?;
        let index = copies
            .iter()
            .find(|(source, _)| source == &project.source)
            .map_or(project.source.as_path(), |(_, target)| target.as_path());
        content = record::set_field(&content, "library_index", Some(&index.to_string_lossy()))?;
        let body_start = frontmatter_spanned(&content)?.body_start;
        content.truncate(body_start);
        content.push_str(&format!("\n# {name}\n\nThe declared project source owns its tasks, decisions, code and production runtime.\n\n[Delivery context and source links]({})\n\nSjel holds the project hub and reads the configured task source without copying it. Shared delivery assets belong to the private delivery library.\n", index.to_string_lossy().replace(' ', "%20")));
        let hub = Record::from_content(&format!("{name}/{name}.md"), content)?;
        if let Some(current) = projects.get(&hub.path).await? {
            if current.content != hub.content {
                return Err(format!("{} differs; existing project is preserved", hub.path).into());
            }
        }
        hubs.push(hub);
    }
    for (source, target) in &copies {
        create_same(target, &fs::read(source)?)?;
    }
    for hub in &hubs {
        if projects.get(&hub.path).await?.is_none() {
            projects.put(hub).await?;
        }
    }
    println!(
        "adopted {} project hubs and verified {} library files; source workspace preserved",
        hubs.len(),
        copies.len()
    );
    Ok(())
}

fn collect_files(
    source: &Path,
    target: &Path,
    copies: &mut Vec<(PathBuf, PathBuf)>,
) -> Fallible<()> {
    let metadata = fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() {
        return Err("delivery assets cannot be symlinks".into());
    }
    if metadata.is_dir() {
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            let name = entry.file_name();
            if name.to_string_lossy().starts_with('.') || name == "node_modules" {
                continue;
            }
            collect_files(&entry.path(), &target.join(name), copies)?;
        }
    } else if metadata.is_file() {
        let extension = source
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        if !["md", "ts", "js", "json", "toml"].contains(&extension) {
            return Err(format!("unsupported library asset {}", source.display()).into());
        }
        if metadata.len() > ops::MAX_CONTENT_BYTES as u64 {
            return Err("delivery asset exceeds the file size limit".into());
        }
        copies.push((source.into(), target.into()));
    } else {
        return Err("delivery assets must be ordinary files".into());
    }
    Ok(())
}

async fn setup(projects: &Projects, name: &str, template: &Path) -> Fallible<()> {
    let name = record::check_name(name)?;
    if !projects.by_project(name).await?.is_empty() {
        return Err("project already exists".into());
    }
    let tree = MarkdownRoot::declare(template)?;
    let mut records = Vec::new();
    for file in tree.markdown_files_recursive()? {
        let relative = tree
            .relative_id(&file)
            .ok_or("template has no relative path")?;
        let relative = if relative == "hub.md" {
            format!("{name}.md")
        } else {
            relative
        };
        let content = fs::read_to_string(file)?.replace("{{project}}", name);
        let content = record::set_field(&content, "class", Some("c2"))?;
        records.push(Record::from_content(
            &format!("{name}/{relative}"),
            content,
        )?);
    }
    if records.iter().filter(|r| r.kind == Kind::Project).count() != 1 {
        return Err("a setup template must contain exactly one project hub".into());
    }
    for record in &records {
        projects.put(record).await?;
    }
    println!(
        "prepared {name}: {} records; platform choices remain unconfirmed",
        records.len()
    );
    Ok(())
}

async fn status(projects: &Projects, github: bool) -> Fallible<()> {
    let records = ops::view_records(projects).await?;
    for hub in records
        .iter()
        .filter(|r| r.kind == Kind::Project && !r.archived)
    {
        let fields = hub.fields();
        let field = |key: &str| fields.get(key).map_or("", String::as_str);
        let tasks: Vec<_> = records
            .iter()
            .filter(|r| r.project == hub.project && r.kind == Kind::Task && !r.done)
            .collect();
        println!(
            "{} | phase {} | {} open | next: {}",
            hub.project,
            field("phase"),
            tasks.len(),
            field("next")
        );
        if !field("local").is_empty() {
            let root = sjel_config::expand_tilde(field("local"));
            let changes = Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(["status", "--porcelain"])
                .output()?;
            if !changes.status.success() {
                return Err(format!("{}: checkout status unavailable", hub.project).into());
            }
            let revision = Command::new("git")
                .arg("-C")
                .arg(root)
                .args(["rev-parse", "--short", "HEAD"])
                .output()?;
            if !revision.status.success() {
                return Err("checkout revision unavailable".into());
            }
            println!(
                "  checkout {} | {} changed paths | task source: {}",
                String::from_utf8(revision.stdout)?.trim(),
                String::from_utf8(changes.stdout)?.lines().count(),
                field("task_source")
            );
        }
        if github && !field("repo").is_empty() {
            let output = Command::new("gh")
                .args([
                    "run",
                    "list",
                    "-R",
                    field("repo"),
                    "--limit",
                    "1",
                    "--json",
                    "status,conclusion,headSha,url",
                ])
                .output()?;
            if !output.status.success() {
                return Err(format!("{}: GitHub CI read failed", hub.project).into());
            }
            println!("  CI {}", String::from_utf8(output.stdout)?.trim());
        }
    }
    Ok(())
}

async fn draft(
    projects: &Projects,
    project: &str,
    title: &str,
    evidence: &str,
    proposal: Option<&Path>,
) -> Fallible<()> {
    let (path, content) = ops::new_note(
        project,
        title,
        "Candidate reusable lesson; review evidence and confidentiality before vault publication",
    )?;
    let content = record::set_field(&content, "class", Some("c2"))?;
    let content = record::set_field(&content, "lesson_status", Some("candidate"))?;
    let mechanism = match proposal {
        Some(path) => {
            let proposed = fs::read_to_string(path)?;
            let body_start = frontmatter_spanned(&proposed)?.body_start;
            proposed[body_start..].to_string()
        }
        None => "## Mechanism\n\n## Assumptions and limits\n\n## Reuse test\n".into(),
    };
    let content = format!("{content}\n{mechanism}\n## Evidence\n\n{evidence}\n\n## Confidentiality review\n\nClient details and private source references remain in this record. The separate concept proposal requires owner review before publication.\n");
    ops::save(projects, &path, &content, None).await?;
    println!("drafted {path}; no vault write");
    Ok(())
}

async fn publish(
    projects: &Projects,
    draft_id: &str,
    reviewed: &Path,
    vault: &Path,
    relative: &str,
    rationale: &str,
) -> Fallible<()> {
    if rationale.trim().len() < 20 {
        return Err("publication requires the owner's confidentiality-review rationale".into());
    }
    let current = projects.get(draft_id).await?.ok_or("no lesson draft")?;
    if current.fields().get("lesson_status").map(String::as_str) != Some("candidate") {
        return Err("only a candidate lesson can be published".into());
    }
    let content = fs::read_to_string(reviewed)?;
    let fields = frontmatter_spanned(&content)?.fields;
    if fields.get("type").map(String::as_str) != Some("concept")
        || fields.get("maturity").map(String::as_str) != Some("seedling")
        || !["summary", "categories", "sources"]
            .iter()
            .all(|key| fields.get(*key).is_some_and(|s| !s.trim().is_empty()))
        || !matches!(fields.get("class").map(String::as_str), Some("c0" | "c1"))
    {
        return Err("reviewed concept requires type, seedling maturity, summary, categories, sources and c0/c1 class".into());
    }
    if !relative.starts_with("Knowledge/") {
        return Err("a reusable concept belongs under Knowledge/".into());
    }
    let root = MarkdownRoot::declare(vault)?;
    let target = root.projection_path(relative)?;
    if !target.parent().is_some_and(Path::is_dir) {
        return Err("choose an existing knowledge folder".into());
    }
    let contract = root.markdown_files("Resources/Vault-Contract.md")?;
    if contract.is_empty() {
        return Err("vault-writing contract is missing".into());
    }
    create_same(&target, content.as_bytes())?;
    let mut ledger = record::set_field(&current.content, "vault_note", Some(relative))?;
    ledger = record::set_field(&ledger, "lesson_status", Some("published"))?;
    ledger.push_str(&format!(
        "\n## Publication review\n\n{}\n\nReviewed file: {}\n",
        rationale,
        reviewed.display()
    ));
    ops::save(projects, draft_id, &ledger, Some(&current.content)).await?;
    println!("published {relative}; provenance and review remain in {draft_id}");
    Ok(())
}

fn checked_relative(value: &str) -> Fallible<&Path> {
    let path = Path::new(value);
    if value.is_empty()
        || path
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        return Err("library target must be a relative path without traversal".into());
    }
    Ok(path)
}

fn contained_destination(root: &Path, path: &Path) -> Fallible<()> {
    let ancestor = path
        .ancestors()
        .find(|p| p.exists())
        .ok_or("destination has no existing ancestor")?;
    if !ancestor.canonicalize()?.starts_with(root) {
        return Err("destination escapes the private resources directory".into());
    }
    Ok(())
}

fn create_same(path: &Path, bytes: &[u8]) -> Fallible<()> {
    if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err("destination cannot be a symlink".into());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
    {
        Ok(mut file) => {
            file.write_all(bytes)?;
            file.sync_all()?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            if fs::read(path)? != bytes {
                return Err(format!("{} exists with different content", path.display()).into());
            }
        }
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn setup_creates_one_private_project_and_refuses_a_retry() {
        let (projects, dir) = crate::store::tests::scratch("workspace-setup").await;
        let template = dir.join("template");
        fs::create_dir(&template).unwrap();
        fs::write(
            template.join("hub.md"),
            "---\ntype: project\n---\n# {{project}}\n",
        )
        .unwrap();
        fs::write(
            template.join("Acceptance.md"),
            "---\ntype: note\n---\n# Acceptance\n",
        )
        .unwrap();
        setup(&projects, "Pilot", &template).await.unwrap();
        let records = projects.by_project("Pilot").await.unwrap();
        assert_eq!(records.len(), 2);
        assert!(records.iter().all(|r| r.fields()["class"] == "c2"));
        assert!(setup(&projects, "Pilot", &template).await.is_err());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn copies_are_idempotent_and_preserve_an_edited_destination() {
        let dir = std::env::temp_dir().join(format!("sjel-workspace-copy-{}", std::process::id()));
        let file = dir.join("asset.md");
        create_same(&file, b"original").unwrap();
        create_same(&file, b"original").unwrap();
        assert!(create_same(&file, b"replacement").is_err());
        assert_eq!(fs::read(file).unwrap(), b"original");
        assert!(checked_relative("../outside").is_err());
        fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn publication_requires_review_and_never_replaces_an_existing_note() {
        let (projects, dir) = crate::store::tests::scratch("workspace-publish").await;
        ops::add_project(&projects, "Pilot", "").await.unwrap();
        draft(&projects, "Pilot", "Lesson", "local evidence", None)
            .await
            .unwrap();
        let vault = dir.join("vault");
        fs::create_dir_all(vault.join("Knowledge")).unwrap();
        fs::create_dir_all(vault.join("Resources")).unwrap();
        fs::write(vault.join("Resources/Vault-Contract.md"), "contract").unwrap();
        let reviewed = dir.join("review.md");
        fs::write(&reviewed, "---\ntype: concept\nmaturity: seedling\nclass: c1\nsummary: A mechanism\ncategories: [Knowledge]\nsources: [Public source]\n---\n\nA mechanism.\n").unwrap();
        let id = "Pilot/Lesson.md";
        assert!(
            publish(&projects, id, &reviewed, &vault, "Knowledge/Lesson.md", "")
                .await
                .is_err()
        );
        fs::write(vault.join("Knowledge/Lesson.md"), "human note").unwrap();
        assert!(publish(
            &projects,
            id,
            &reviewed,
            &vault,
            "Knowledge/Lesson.md",
            "Owner reviewed the abstraction and removed private details"
        )
        .await
        .is_err());
        assert_eq!(
            fs::read_to_string(vault.join("Knowledge/Lesson.md")).unwrap(),
            "human note"
        );
        fs::remove_file(vault.join("Knowledge/Lesson.md")).unwrap();
        publish(
            &projects,
            id,
            &reviewed,
            &vault,
            "Knowledge/Lesson.md",
            "Owner reviewed the abstraction and removed private details",
        )
        .await
        .unwrap();
        assert_eq!(
            projects.get(id).await.unwrap().unwrap().fields()["lesson_status"],
            "published"
        );
        assert_eq!(
            projects.get(id).await.unwrap().unwrap().fields()["class"],
            "c2"
        );
        fs::remove_dir_all(dir).unwrap();
    }
}
