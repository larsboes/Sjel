//! Read-only collection preview. No ledger, hashes, inference, or execution path.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::store::Result;

#[derive(Debug, Deserialize)]
pub struct Draft {
    draft_version: u32,
    pub(crate) archive_root: PathBuf,
    pub(crate) source_root: PathBuf,
    pub(crate) expected_volume_uuid: String,
    organization: Organization,
    pub(crate) mappings: Vec<Mapping>,
    execution: Execution,
}

#[derive(Debug, Deserialize)]
struct Organization {
    categories: Vec<Category>,
    default_rule: Option<DefaultRule>,
}
#[derive(Debug, Deserialize)]
struct Category {
    name: String,
}

/// The declared default placement. It is not a guesser: it fires only on a collection whose name
/// carries a `YYYY-` prefix, and the operator declares the category and the shape in the draft.
/// Anything that does not match stays `unresolved`, which is a reportable state and not an error.
#[derive(Debug, Deserialize)]
struct DefaultRule {
    category: String,
    /// `{category}`, `{year}` and `{collection}` are substituted. `{collection}` is mandatory.
    destination: String,
    #[serde(default = "default_true")]
    require_dated_name: bool,
    /// Collections this rule must not place, named one by one.
    #[serde(default)]
    skip: Vec<String>,
}
fn default_true() -> bool {
    true
}
impl DefaultRule {
    fn destination_for(&self, name: &str, categories: &BTreeSet<&str>) -> Option<String> {
        if self.skip.iter().any(|skip| skip == name) {
            return None;
        }
        let year = dated_prefix_year(name);
        if self.require_dated_name && year.is_none() {
            return None;
        }
        let year = year.unwrap_or_default();
        if self.destination.contains("{year}") && year.is_empty() {
            return None;
        }
        let candidate = self
            .destination
            .replace("{category}", &self.category)
            .replace("{year}", year)
            .replace("{collection}", name);
        destination_ok(&candidate, categories).then_some(candidate)
    }
}

/// `YYYY-` prefix, with or without a month: `2018-03-Iceland` and `2018-Finland` both answer
/// `2018`. Anything else answers `None` rather than a guessed year.
fn dated_prefix_year(name: &str) -> Option<&str> {
    let bytes = name.as_bytes();
    if bytes.len() < 5 || !name.is_char_boundary(4) {
        return None;
    }
    (bytes[..4].iter().all(u8::is_ascii_digit) && bytes[4] == b'-').then(|| &name[..4])
}

#[derive(Debug, Deserialize)]
struct Execution {
    file_moves_authorized: bool,
}
// The shared suffix is the draft's on-disk vocabulary (`destination_reviewed`,
// `provisional_event_membership_needs_review`, `rule_reviewed`). Renaming the variants would
// decouple the code from the file an operator actually edits, so the repetition is the point.
#[allow(clippy::enum_variant_names)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Review {
    DestinationReviewed,
    ProvisionalEventMembershipNeedsReview,
    /// Placed by the operator's declared rule rather than one destination written out by hand.
    RuleReviewed,
}
#[derive(Debug, Deserialize)]
pub(crate) struct Mapping {
    source_collection: String,
    proposed_destination_relative: Option<String>,
    candidate_destination_relative: Option<String>,
    status: Review,
}
impl Mapping {
    fn destination(&self) -> Option<&str> {
        self.proposed_destination_relative
            .as_deref()
            .or(self.candidate_destination_relative.as_deref())
    }
}

fn component_ok(name: &str) -> bool {
    !name.is_empty()
        && !matches!(name, "." | "..")
        && !name.contains(['/', '\\', '\0'])
        && !Path::new(name).is_absolute()
}

fn destination_ok(path: &str, categories: &BTreeSet<&str>) -> bool {
    let mut segments = path.split('/');
    let first = segments.next().unwrap_or_default();
    categories.contains(first) && component_ok(first) && segments.all(component_ok)
}

impl Draft {
    /// Preview semantics. A draft that authorizes moves is refused, so preparing a plan can never
    /// be confused with approving one.
    pub fn parse(input: &str) -> Result<Self> {
        let draft: Self = serde_json::from_str(input)?;
        draft.validate(false)?;
        Ok(draft)
    }

    /// Apply semantics. The operator's `file_moves_authorized` flag is the authorization, and the
    /// organizing verb will not run without it.
    pub fn parse_authorized(input: &str) -> Result<Self> {
        let draft: Self = serde_json::from_str(input)?;
        draft.validate(true)?;
        Ok(draft)
    }

    fn validate(&self, moves_authorized: bool) -> Result<()> {
        let draft = self;
        if draft.draft_version != 1 {
            return Err("draft_version must be 1".into());
        }
        if draft.execution.file_moves_authorized != moves_authorized {
            return Err(match moves_authorized {
                true => "apply requires file_moves_authorized=true in the draft".into(),
                false => "preview requires file_moves_authorized=false in the draft".into(),
            });
        }
        if draft.expected_volume_uuid.is_empty()
            || !draft
                .expected_volume_uuid
                .bytes()
                .all(|b| b.is_ascii_hexdigit() || b == b'-')
        {
            return Err("expected_volume_uuid must be a nonempty volume UUID".into());
        }
        for root in [&draft.archive_root, &draft.source_root] {
            let text = root.to_str().ok_or("non-UTF-8 root")?;
            if !root.is_absolute()
                || text.contains(['\\', '\0'])
                || text.split('/').any(|s| matches!(s, "." | ".."))
            {
                return Err("roots must be absolute paths without traversal".into());
            }
        }
        let mut categories = BTreeSet::new();
        for category in &draft.organization.categories {
            if !component_ok(&category.name) || !categories.insert(category.name.as_str()) {
                return Err("invalid or duplicate category name".into());
            }
        }
        if categories.is_empty() {
            return Err("at least one category is required".into());
        }
        if let Some(rule) = &draft.organization.default_rule {
            if !categories.contains(rule.category.as_str()) {
                return Err("default_rule category is not declared in organization.categories".into());
            }
            if !rule.destination.contains("{collection}") {
                return Err("default_rule destination must use {collection}".into());
            }
            for skip in &rule.skip {
                if !component_ok(skip) {
                    return Err("default_rule skip entry must be a plain name".into());
                }
            }
            let sample = rule
                .destination
                .replace("{category}", &rule.category)
                .replace("{year}", "2000")
                .replace("{collection}", "2000-01-Sample");
            if !destination_ok(&sample, &categories) {
                return Err(
                    "default_rule destination must be a safe relative path in a declared category"
                        .into(),
                );
            }
        }
        let mut sources = BTreeSet::new();
        for mapping in &draft.mappings {
            if !component_ok(&mapping.source_collection)
                || !sources.insert(mapping.source_collection.as_str())
            {
                return Err("invalid or duplicate source_collection".into());
            }
            if let (Some(a), Some(b)) = (
                &mapping.proposed_destination_relative,
                &mapping.candidate_destination_relative,
            ) {
                if a != b {
                    return Err("ambiguous proposed and candidate destinations".into());
                }
            }
            for path in [
                mapping.proposed_destination_relative.as_deref(),
                mapping.candidate_destination_relative.as_deref(),
            ]
            .into_iter()
            .flatten()
            {
                if !destination_ok(path, &categories) {
                    return Err(
                        "destination must be a safe relative path in a declared category".into(),
                    );
                }
            }
            if matches!(mapping.status, Review::DestinationReviewed)
                && mapping.destination().is_none()
            {
                return Err("destination_reviewed requires a destination".into());
            }
        }
        Ok(())
    }
}

/// Check every existing component with lstat, not exists()/canonicalize() alone.
pub(crate) fn real_directory(path: &Path) -> Result<()> {
    if !path.is_absolute() {
        return Err("directory path must be absolute".into());
    }
    let mut ancestor = PathBuf::new();
    for part in path.components() {
        if !matches!(part, Component::RootDir | Component::Normal(_)) {
            return Err("invalid directory component".into());
        }
        ancestor.push(part);
        let meta = fs::symlink_metadata(&ancestor)?;
        if meta.file_type().is_symlink() || !meta.is_dir() {
            return Err(format!("not a real directory: {}", ancestor.display()).into());
        }
    }
    Ok(())
}

pub(crate) fn validated_roots(draft: &Draft, moves_authorized: bool) -> Result<(PathBuf, PathBuf)> {
    draft.validate(moves_authorized)?;
    real_directory(&draft.source_root)?;
    real_directory(&draft.archive_root)?;
    let source = draft.source_root.canonicalize()?;
    let archive = draft.archive_root.canonicalize()?;
    if source.starts_with(&archive) || archive.starts_with(&source) {
        return Err("source and archive trees overlap".into());
    }
    Ok((source, archive))
}

/// Fixture-facing evaluation: filesystem metadata only, no UUID helper, subprocess or database.
/// Parsing and root validation remain mandatory; no writes or hashes are performed.
pub fn evaluate(draft: &Draft) -> Result<Report> {
    let (source, archive) = validated_roots(draft, false)?;
    Ok(scan(draft, &source, &archive, false))
}

/// A collection the draft actually places, and how it was placed.
#[derive(Debug, Clone)]
pub(crate) struct Resolved {
    pub(crate) collection: String,
    pub(crate) destination: String,
    pub(crate) origin: &'static str,
}

/// Every collection the draft places. A hand mapping wins over the rule, so an exception is
/// written once, in one place, and the rule stays general.
///
/// `rule_reviewed` is an override carried in `mappings[]`; `rule` is the default rule firing. Both
/// are actionable. The provisional status is deliberately absent: an unresolved human choice must
/// not reach an executing verb.
pub(crate) fn resolved_destinations(draft: &Draft) -> Result<Vec<Resolved>> {
    let categories: BTreeSet<&str> = draft
        .organization
        .categories
        .iter()
        .map(|category| category.name.as_str())
        .collect();
    let mut resolved = Vec::new();
    let mut mapped = BTreeSet::new();
    for mapping in &draft.mappings {
        mapped.insert(mapping.source_collection.as_str());
        if matches!(
            mapping.status,
            Review::ProvisionalEventMembershipNeedsReview
        ) {
            continue;
        }
        let destination = mapping
            .destination()
            .ok_or("a reviewed mapping must carry a destination")?;
        resolved.push(Resolved {
            collection: mapping.source_collection.clone(),
            destination: destination.to_owned(),
            origin: "mapping",
        });
    }
    if let Some(rule) = &draft.organization.default_rule {
        for name in collection_names(&draft.source_root)? {
            if mapped.contains(name.as_str()) {
                continue;
            }
            if let Some(destination) = rule.destination_for(&name, &categories) {
                resolved.push(Resolved {
                    collection: name,
                    destination,
                    origin: "rule",
                });
            }
        }
    }
    Ok(resolved)
}

pub(crate) fn category_names(draft: &Draft) -> BTreeSet<&str> {
    draft
        .organization
        .categories
        .iter()
        .map(|category| category.name.as_str())
        .collect()
}

/// Real child directories of the source root, sorted. Symlinks and non-directories are left out;
/// the caller reports what it does not act on rather than acting on a guess.
fn collection_names(source: &Path) -> Result<Vec<String>> {
    real_directory(source)?;
    let mut names = Vec::new();
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        if entry.file_type()?.is_dir() {
            names.push(name);
        }
    }
    names.sort();
    Ok(names)
}

/// The production entry point checks both mount identities before any traversal.
pub fn preview(structure: &Path, metadata: bool) -> Result<Report> {
    let draft = Draft::parse(&fs::read_to_string(structure)?)?;
    let (source, archive) = validated_roots(&draft, false)?;
    for root in [&source, &archive] {
        let actual = crate::volume_uuid(root)?;
        if !actual.eq_ignore_ascii_case(&draft.expected_volume_uuid) {
            return Err(format!("volume UUID mismatch for {}", root.display()).into());
        }
    }
    Ok(scan(&draft, &source, &archive, metadata))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileStamp {
    pub path: PathBuf,
    pub size: u64,
    pub mtime_seconds: i64,
    pub mtime_nanoseconds: i64,
    pub inode: u64,
    pub device: u64,
}

#[cfg(unix)]
fn file_stamp(path: &Path) -> Result<FileStamp> {
    use std::os::unix::fs::MetadataExt;
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_file() || meta.file_type().is_symlink() {
        return Err("not a regular file".into());
    }
    Ok(FileStamp {
        path: path.to_path_buf(),
        size: meta.len(),
        mtime_seconds: meta.mtime(),
        mtime_nanoseconds: meta.mtime_nsec(),
        inode: meta.ino(),
        device: meta.dev(),
    })
}
#[cfg(not(unix))]
fn file_stamp(_path: &Path) -> Result<FileStamp> {
    Err("preview requires Unix filesystem identity checks".into())
}

#[cfg(unix)]
fn directory_stamp(path: &Path) -> Result<(u64, u64, i64, i64)> {
    use std::os::unix::fs::MetadataExt;
    real_directory(path)?;
    let meta = fs::symlink_metadata(path)?;
    Ok((meta.dev(), meta.ino(), meta.mtime(), meta.mtime_nsec()))
}
#[cfg(not(unix))]
fn directory_stamp(_path: &Path) -> Result<(u64, u64, i64, i64)> {
    Err("preview requires Unix filesystem identity checks".into())
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub moves_authorized: bool,
    pub complete: bool,
    pub source_root: PathBuf,
    pub archive_root: PathBuf,
    pub collections: Vec<Collection>,
    pub loose_files: Vec<FileStamp>,
    pub issues: Vec<String>,
    pub limitations: Vec<&'static str>,
}
#[derive(Debug, Serialize)]
pub struct Collection {
    pub name: String,
    pub status: &'static str,
    pub proposal_destination_relative: Option<String>,
    pub moves_authorized: bool,
    pub complete: bool,
    pub file_count: usize,
    pub apparent_bytes: u64,
    pub extension_counts: BTreeMap<String, usize>,
    pub files: Vec<FileStamp>,
    pub issues: Vec<String>,
    pub metadata: Option<MetadataSummary>,
}
impl Collection {
    fn new(name: String) -> Self {
        Self {
            name,
            status: "review",
            proposal_destination_relative: None,
            moves_authorized: false,
            complete: true,
            file_count: 0,
            apparent_bytes: 0,
            extension_counts: BTreeMap::new(),
            files: Vec::new(),
            issues: Vec::new(),
            metadata: None,
        }
    }
    fn incomplete(&mut self, issue: String) {
        self.complete = false;
        self.issues.push(issue);
    }
}

fn target_conflict(archive: &Path, destination: &str) -> Result<bool> {
    real_directory(archive)?;
    let mut path = archive.to_path_buf();
    let segments: Vec<_> = destination.split('/').collect();
    for (i, segment) in segments.iter().enumerate() {
        path.push(segment);
        match fs::symlink_metadata(&path) {
            Ok(meta) => {
                if i + 1 == segments.len() || meta.file_type().is_symlink() || !meta.is_dir() {
                    return Ok(true);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(e.into()),
        }
    }
    Ok(false)
}

fn scan(draft: &Draft, source: &Path, archive: &Path, metadata: bool) -> Report {
    let mut report = Report {
        moves_authorized: false,
        complete: true,
        source_root: source.into(),
        archive_root: archive.into(),
        collections: Vec::new(),
        loose_files: Vec::new(),
        issues: Vec::new(),
        limitations: vec![
            "No hashing is performed; this preview provides no content-integrity guarantee.",
            "Read-only path checks are not an atomic filesystem snapshot; concurrent changes can race checks.",
        ],
    };
    if metadata {
        report.limitations.push("Metadata is requested only for explicitly mapped collections; other collections receive filesystem inventory only.");
        report.limitations.push("ExifTool user configuration is disabled. Each batch is limited to 128 files, 30 seconds and 2 MiB of stdout; diagnostic bodies are withheld and no deep -ee pass is used.");
        report.limitations.push("All date candidates remain unvalidated evidence. Civil days are not timezone-converted; container creation and other lower-priority fields may date digitization/export, not capture. Collections are never automatically split.");
    }
    let source_before = directory_stamp(source);
    let archive_before = directory_stamp(archive);
    if source_before.is_err() || archive_before.is_err() {
        report.complete = false;
        report
            .issues
            .push("source or archive directory unavailable".into());
        return report;
    }
    let device = source_before.as_ref().expect("checked source stamp").0;
    let mappings: BTreeMap<_, _> = draft
        .mappings
        .iter()
        .map(|m| (m.source_collection.as_str(), m))
        .collect();
    let mut found = BTreeSet::new();
    let entries = match fs::read_dir(source) {
        Ok(entries) => entries,
        Err(_) => {
            report.complete = false;
            report.issues.push("cannot list source root".into());
            return report;
        }
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                report.complete = false;
                report.issues.push("unreadable source-root entry".into());
                continue;
            }
        };
        let path = entry.path();
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            report.complete = false;
            report
                .issues
                .push("non-UTF-8 source-root name; skipped".into());
            continue;
        };
        let ty = match entry.file_type() {
            Ok(ty) => ty,
            Err(_) => {
                report.complete = false;
                report
                    .issues
                    .push(format!("cannot inspect source-root entry: {name}"));
                continue;
            }
        };
        if ty.is_file() {
            match file_stamp(&path) {
                Ok(stamp) if stamp.device == device => report.loose_files.push(stamp),
                _ => {
                    report.complete = false;
                    report
                        .issues
                        .push(format!("cannot safely inspect loose file: {name}"));
                }
            }
        } else if ty.is_dir() {
            found.insert(name.clone());
            let mut collection = Collection::new(name.clone());
            let directories = walk_collection(&path, device, &mut collection);
            if metadata && mappings.contains_key(name.as_str()) {
                let mut summary = extract_metadata(&collection.files, &name);
                // Re-inventory after extraction: file stamps alone miss newly added entries.
                let mut after = Collection::new(name.clone());
                let after_directories = walk_collection(&path, device, &mut after);
                if !after.complete
                    || after.files != collection.files
                    || after_directories != directories
                {
                    summary
                        .fail("collection entries changed/disappeared during metadata extraction");
                    collection.issues.extend(after.issues);
                }
                collection.complete &= summary.complete;
                collection.metadata = Some(summary);
            }
            match mappings.get(name.as_str()) {
                Some(mapping)
                    if !matches!(
                        mapping.status,
                        Review::ProvisionalEventMembershipNeedsReview
                    ) =>
                {
                    let destination = mapping.destination().expect("validated reviewed destination");
                    // A hand-written destination is reported as a proposal; one the operator's own
                    // rule placed is reported as a rule placement. Both are actionable, and the
                    // distinction is visible so a rule's reach can be reviewed before it runs.
                    let placed = match mapping.status {
                        Review::DestinationReviewed => "proposal",
                        _ => "rule",
                    };
                    match target_conflict(archive, destination) {
                        Ok(true) => {
                            collection.status = "conflict";
                            collection.issues.push("destination exists or an ancestor is a symlink/non-directory; no overwrite proposed".into());
                        }
                        Ok(false) => {
                            collection.status = placed;
                            collection.proposal_destination_relative = Some(destination.to_owned());
                        }
                        Err(_) => collection.incomplete("cannot inspect destination ancestors".into()),
                    }
                }
                Some(_) => collection.issues.push("event membership needs human review; candidate is not an executable destination".into()),
                None => {
                    let categories = category_names(draft);
                    let placed = draft
                        .organization
                        .default_rule
                        .as_ref()
                        .and_then(|rule| rule.destination_for(&name, &categories));
                    match placed {
                        Some(destination) => match target_conflict(archive, &destination) {
                            Ok(true) => {
                                collection.status = "conflict";
                                collection.issues.push("rule destination exists or an ancestor is a symlink/non-directory; no overwrite proposed".into());
                            }
                            Ok(false) => {
                                collection.status = "rule";
                                collection.proposal_destination_relative = Some(destination);
                            }
                            Err(_) => collection
                                .incomplete("cannot inspect destination ancestors".into()),
                        },
                        None => {
                            collection.status = "unresolved";
                            collection
                                .issues
                                .push("no reviewed mapping and no rule match for collection".into());
                        }
                    }
                }
            }
            if !collection.complete {
                collection.status = "blocked";
                collection.proposal_destination_relative = None;
            }
            report.complete &= collection.complete;
            report.collections.push(collection);
        } else {
            report.complete = false;
            report.issues.push(format!(
                "symlink or non-regular source-root entry skipped: {name}"
            ));
        }
    }
    for mapping in &draft.mappings {
        if !found.contains(&mapping.source_collection) {
            let mut collection = Collection::new(mapping.source_collection.clone());
            collection.incomplete("mapped collection is absent or is not a real directory".into());
            collection.status = "blocked";
            report.collections.push(collection);
            report.complete = false;
        }
    }
    // Two reviewed units cannot share or nest their destination, even when neither exists yet.
    let proposals: Vec<_> = report
        .collections
        .iter()
        .enumerate()
        .filter_map(|(i, c)| {
            c.proposal_destination_relative
                .as_ref()
                .map(|p| (i, PathBuf::from(p)))
        })
        .collect();
    let mut collisions = BTreeSet::new();
    for (i, a) in &proposals {
        for (j, b) in &proposals {
            if i != j && (a.starts_with(b) || b.starts_with(a)) {
                collisions.insert(*i);
            }
        }
    }
    for i in collisions {
        let collection = &mut report.collections[i];
        collection.status = "conflict";
        collection.proposal_destination_relative = None;
        collection
            .issues
            .push("reviewed destinations overlap".into());
    }
    if !report.loose_files.is_empty() {
        report
            .issues
            .push("loose root files are unresolved; no collection/category assigned".into());
    }
    if source_before.ok() != directory_stamp(source).ok()
        || archive_before.ok() != directory_stamp(archive).ok()
    {
        report.complete = false;
        report
            .issues
            .push("source or archive root changed/disappeared during preview".into());
        for collection in &mut report.collections {
            collection.status = "blocked";
            collection.proposal_destination_relative = None;
        }
    }
    for file in &report.loose_files {
        if file_stamp(&file.path).ok().as_ref() != Some(file) {
            report.complete = false;
            report
                .issues
                .push("loose file changed/disappeared during preview".into());
        }
    }
    report.collections.sort_by(|a, b| a.name.cmp(&b.name));
    report.loose_files.sort_by(|a, b| a.path.cmp(&b.path));
    report
}

fn extension(path: &Path) -> String {
    path.extension()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

type DirectoryStamp = (u64, u64, i64, i64);

fn walk_collection(
    root: &Path,
    device: u64,
    collection: &mut Collection,
) -> Vec<(PathBuf, DirectoryStamp)> {
    let mut pending = vec![root.to_path_buf()];
    let mut directories = Vec::new();
    while let Some(dir) = pending.pop() {
        let before = match directory_stamp(&dir) {
            Ok(stamp) if stamp.0 == device => stamp,
            _ => {
                collection.incomplete(format!(
                    "unsafe, unavailable or different-filesystem directory skipped: {}",
                    dir.display()
                ));
                continue;
            }
        };
        directories.push((dir.clone(), before));
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => {
                collection.incomplete(format!("cannot list directory: {}", dir.display()));
                continue;
            }
        };
        for entry in entries {
            let path = match entry {
                Ok(entry) => entry.path(),
                Err(_) => {
                    collection.incomplete("unreadable collection entry".into());
                    continue;
                }
            };
            if path.to_str().is_none() {
                collection.incomplete("non-UTF-8 collection path skipped".into());
                continue;
            }
            match fs::symlink_metadata(&path) {
                Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => pending.push(path),
                Ok(meta) if meta.is_file() && !meta.file_type().is_symlink() => {
                    match file_stamp(&path) {
                        Ok(stamp) if stamp.device == device => {
                            if let Some(total) = collection.apparent_bytes.checked_add(stamp.size) {
                                collection.apparent_bytes = total;
                            } else {
                                collection.incomplete("apparent byte count overflow".into());
                            }
                            *collection
                                .extension_counts
                                .entry(extension(&path))
                                .or_default() += 1;
                            collection.files.push(stamp);
                        }
                        _ => collection.incomplete(format!(
                            "unavailable or different-filesystem file skipped: {}",
                            path.display()
                        )),
                    }
                }
                _ => collection.incomplete(format!(
                    "unavailable, symlink or non-regular path skipped: {}",
                    path.display()
                )),
            }
        }
    }
    for (dir, before) in &directories {
        if directory_stamp(dir).ok() != Some(*before) {
            collection.incomplete(format!(
                "directory changed/disappeared during traversal: {}",
                dir.display()
            ));
        }
    }
    let changed = collection
        .files
        .iter()
        .any(|file| file_stamp(&file.path).ok().as_ref() != Some(file));
    if changed {
        collection.incomplete("file changed/disappeared during traversal".into());
    }
    collection.files.sort_by(|a, b| a.path.cmp(&b.path));
    collection.file_count = collection.files.len();
    directories.sort_by(|a, b| a.0.cmp(&b.0));
    directories
}

fn recognized_media(path: &Path) -> bool {
    matches!(
        extension(path).as_str(),
        "jpg"
            | "jpeg"
            | "heic"
            | "heif"
            | "png"
            | "tif"
            | "tiff"
            | "webp"
            | "gif"
            | "avif"
            | "dng"
            | "cr2"
            | "cr3"
            | "nef"
            | "nrw"
            | "arw"
            | "orf"
            | "rw2"
            | "raf"
            | "pef"
            | "mov"
            | "mp4"
            | "m4v"
            | "avi"
            | "mkv"
            | "mts"
            | "m2ts"
            | "3gp"
            | "webm"
            | "mpg"
            | "mpeg"
    )
}

#[derive(Debug, Default, Serialize)]
pub struct MetadataSummary {
    pub availability: &'static str,
    pub complete: bool,
    pub requested_files: usize,
    pub rows_read: usize,
    pub missing_rows: usize,
    pub unexpected_rows: usize,
    pub duplicate_rows: usize,
    pub changed_files: usize,
    pub date_min: Option<String>,
    pub date_max: Option<String>,
    pub month_counts: BTreeMap<String, usize>,
    /// Selected (priority-winning) field counts, distinct from all observed date fields.
    pub date_source_counts: BTreeMap<String, usize>,
    pub date_field_counts: BTreeMap<String, usize>,
    pub container_only_files: usize,
    pub lower_priority_date_only_files: usize,
    pub disagreement_files: usize,
    pub outside_collection_month_files: usize,
    pub evidence_outside_collection_month_files: usize,
    pub invalid_date_fields: usize,
    pub undated_files: usize,
    pub gps_tag_files: usize,
    pub gps_tag_presence_counts: BTreeMap<String, usize>,
    pub warning_count: usize,
    pub error_count: usize,
    pub issues: Vec<&'static str>,
}
impl MetadataSummary {
    fn new(requested: usize) -> Self {
        Self {
            availability: "available",
            complete: true,
            requested_files: requested,
            ..Self::default()
        }
    }
    fn fail(&mut self, issue: &'static str) {
        self.complete = false;
        if !self.issues.contains(&issue) {
            self.issues.push(issue);
        }
    }
}

// Keep the civil day as recorded, without applying the suffix's UTC offset.
pub(crate) fn metadata_day(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    if bytes.len() < 10
        || !bytes[..4].iter().all(u8::is_ascii_digit)
        || !bytes[5..7].iter().all(u8::is_ascii_digit)
        || !bytes[8..10].iter().all(u8::is_ascii_digit)
        || !matches!((bytes[4], bytes[7]), (b':', b':') | (b'-', b'-'))
        || (bytes.len() > 10 && !matches!(bytes[10], b' ' | b'T'))
    {
        return None;
    }
    let year: i64 = input.get(..4)?.parse().ok()?;
    let month: u32 = input.get(5..7)?.parse().ok()?;
    let day: u32 = input.get(8..10)?.parse().ok()?;
    if year == 0 || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let unix = civil_date::ymd_to_unix_day(year, month, day);
    if civil_date::unix_day_to_ymd(unix) != (year, month, day) {
        return None;
    }
    Some(civil_date::iso_of_unix_day(unix))
}

fn collection_month(name: &str) -> Option<&str> {
    let prefix = name.get(..7)?;
    if name.as_bytes().get(7).is_some_and(u8::is_ascii_digit) {
        return None;
    }
    metadata_day(&format!("{prefix}-01"))?;
    Some(prefix)
}

pub(crate) fn date_priority(key: &str) -> Option<u8> {
    let (group, tag) = key.rsplit_once(':').unwrap_or(("", key));
    match tag {
        "DateTimeOriginal" => Some(0),
        "CreationDate" if group == "Keys" => Some(1),
        "CreateDate" => Some(2),
        "TrackCreateDate" => Some(3),
        "MediaCreateDate" => Some(4),
        _ => None,
    }
}

fn container_date(key: &str) -> bool {
    let group = key.rsplit_once(':').map_or("", |(group, _)| group);
    matches!(group, "QuickTime" | "Matroska" | "RIFF" | "ASF" | "FLV")
        || ["Track", "Media"].iter().any(|prefix| {
            group.strip_prefix(prefix).is_some_and(|suffix| {
                !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
            })
        })
}

fn summarize_row(row: &serde_json::Map<String, Value>, label: &str, summary: &mut MetadataSummary) {
    let mut dates = Vec::new();
    let mut gps_tags = BTreeSet::new();
    for (key, value) in row {
        let tag = key.rsplit(':').next().unwrap_or(key);
        match tag {
            "GPSLatitude"
            | "GPSLongitude"
            | "GPSCoordinates"
            | "GPSPosition"
            | "LocationInformation" => {
                // Presence, including numeric zero, never truthiness or coordinate values.
                if !value.is_null() {
                    gps_tags.insert(tag);
                }
            }
            "Warning" => summary.warning_count += 1,
            "Error" => {
                summary.error_count += 1;
                summary.fail("ExifTool reported an error; diagnostic bodies withheld");
            }
            _ => (),
        }
        if let Some(priority) = date_priority(key) {
            *summary.date_field_counts.entry(key.clone()).or_default() += 1;
            match value.as_str().and_then(metadata_day) {
                Some(day) => dates.push((priority, key.as_str(), day)),
                None => summary.invalid_date_fields += 1,
            }
        }
    }
    if !gps_tags.is_empty() {
        summary.gps_tag_files += 1;
        for tag in gps_tags {
            *summary
                .gps_tag_presence_counts
                .entry(tag.into())
                .or_default() += 1;
        }
    }
    dates.sort();
    if let Some((priority, key, day)) = dates.first() {
        *summary.date_source_counts.entry((*key).into()).or_default() += 1;
        *summary.month_counts.entry(day[..7].into()).or_default() += 1;
        summary.lower_priority_date_only_files += usize::from(*priority >= 2);
        summary.container_only_files +=
            usize::from(dates.iter().all(|(_, field, _)| container_date(field)));
        summary.disagreement_files += usize::from(dates.iter().any(|(_, _, other)| other != day));
        summary.evidence_outside_collection_month_files += usize::from(
            collection_month(label)
                .is_some_and(|month| dates.iter().any(|(_, _, evidence)| month != &evidence[..7])),
        );
        summary.outside_collection_month_files +=
            usize::from(collection_month(label).is_some_and(|month| month != &day[..7]));
        if summary.date_min.as_ref().is_none_or(|min| day < min) {
            summary.date_min = Some(day.clone());
        }
        if summary.date_max.as_ref().is_none_or(|max| day > max) {
            summary.date_max = Some(day.clone());
        }
    } else {
        summary.undated_files += 1;
    }
}

fn consume_metadata(
    input: &[u8],
    expected: &BTreeSet<String>,
    label: &str,
    summary: &mut MetadataSummary,
) {
    let rows: Vec<Value> = match serde_json::from_slice(input) {
        Ok(rows) => rows,
        Err(_) => {
            summary.missing_rows += expected.len();
            summary.fail("invalid ExifTool JSON; diagnostic bodies withheld");
            return;
        }
    };
    // Validate row identity before using any evidence. Duplicate rows contribute no dates.
    let mut by_source: BTreeMap<&str, Vec<&serde_json::Map<String, Value>>> = BTreeMap::new();
    for row in &rows {
        let Some(object) = row.as_object() else {
            summary.unexpected_rows += 1;
            summary.fail("unexpected ExifTool row");
            continue;
        };
        let source = object.get("SourceFile").and_then(Value::as_str);
        match source {
            Some(source) if expected.contains(source) => {
                by_source.entry(source).or_default().push(object)
            }
            _ => {
                summary.unexpected_rows += 1;
                summary.fail("unexpected ExifTool SourceFile; values withheld");
            }
        }
    }
    for source in expected {
        match by_source.get(source.as_str()) {
            None => {
                summary.missing_rows += 1;
                summary.fail("missing ExifTool SourceFile rows");
            }
            Some(rows) if rows.len() != 1 => {
                summary.duplicate_rows += rows.len() - 1;
                summary.fail("duplicate ExifTool SourceFile rows");
            }
            Some(rows) => {
                summary.rows_read += 1;
                summarize_row(rows[0], label, summary);
            }
        }
    }
}

pub(crate) const EXIF_ARGS: &[&str] = &[
    "-config",
    "",
    "-json",
    "-a",
    "-G1",
    "-s",
    "-api",
    "LargeFileSupport=1",
    "-DateTimeOriginal",
    "-CreationDate",
    "-CreateDate",
    "-TrackCreateDate",
    "-MediaCreateDate",
    "-GPSLatitude",
    "-GPSLongitude",
    "-GPSCoordinates",
    "-GPSPosition",
    "-LocationInformation",
    "-Warning",
    "-Error",
];

// Drain stdout concurrently so a full pipe cannot prevent process exit. Return on
// the deadline even if a descendant retains the pipe; do not join a blocked reader.
fn bounded_output(
    command: &mut Command,
    timeout: Duration,
    limit: u64,
) -> std::result::Result<Output, &'static str> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                "ExifTool is not installed; no filesystem-date fallback"
            } else {
                "ExifTool could not be started; no filesystem-date fallback"
            }
        })?;
    let stdout = child.stdout.take().expect("stdout was piped");
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stdout
            .take(limit + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes);
        let _ = sender.send(result);
    });
    let started = Instant::now();
    let mut bytes = None;
    let outcome = loop {
        if started.elapsed() >= timeout {
            break Err("ExifTool batch timed out; diagnostic bodies withheld");
        }
        if bytes.is_none() {
            match receiver.try_recv() {
                Ok(Ok(output)) if output.len() as u64 <= limit => bytes = Some(output),
                Ok(Ok(_)) => break Err("ExifTool output exceeded limit; values withheld"),
                Ok(Err(_)) | Err(mpsc::TryRecvError::Disconnected) => {
                    break Err("ExifTool stdout could not be read; values withheld");
                }
                Err(mpsc::TryRecvError::Empty) => (),
            }
        }
        match child.try_wait() {
            Ok(Some(status)) if bytes.is_some() => {
                break Ok(Output {
                    status,
                    stdout: bytes.take().expect("checked output"),
                    stderr: Vec::new(),
                });
            }
            Ok(_) => (),
            Err(_) => break Err("ExifTool process could not be observed"),
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    if outcome.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    outcome
}

fn extract_metadata(files: &[FileStamp], label: &str) -> MetadataSummary {
    let media: Vec<_> = files
        .iter()
        .filter(|file| recognized_media(&file.path))
        .collect();
    let mut summary = MetadataSummary::new(media.len());
    if media.is_empty() {
        summary.availability = "not_needed";
        return summary;
    }
    for batch in media.chunks(128) {
        let mut safe = Vec::new();
        for file in batch {
            if real_directory(file.path.parent().expect("absolute file has parent")).is_err()
                || file_stamp(&file.path).ok().as_ref() != Some(*file)
            {
                summary.changed_files += 1;
                summary.fail("file changed/disappeared before metadata extraction");
            } else {
                safe.push(*file);
            }
        }
        if safe.is_empty() {
            continue;
        }
        let expected: BTreeSet<_> = safe
            .iter()
            .map(|file| {
                file.path
                    .to_str()
                    .expect("UTF-8 checked during traversal")
                    .to_owned()
            })
            .collect();
        // Absolute paths cannot be interpreted as ExifTool options.
        let output = bounded_output(
            Command::new("exiftool")
                .args(EXIF_ARGS)
                .args(safe.iter().map(|file| &file.path)),
            Duration::from_secs(30),
            2 * 1024 * 1024,
        );
        match output {
            Ok(output) => {
                if !output.status.success() {
                    summary.fail("ExifTool exited unsuccessfully; diagnostic bodies withheld");
                }
                consume_metadata(&output.stdout, &expected, label, &mut summary);
            }
            Err(error) => {
                summary.availability = "unavailable";
                summary.missing_rows += expected.len();
                summary.fail(error);
            }
        }
        for file in safe {
            if real_directory(file.path.parent().expect("absolute file has parent")).is_err()
                || file_stamp(&file.path).ok().as_ref() != Some(file)
            {
                summary.changed_files += 1;
                summary.fail("file changed/disappeared during metadata extraction");
            }
        }
    }
    summary
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[cfg(unix)]
    #[test]
    fn subprocess_limits_refuse_stalls_and_excess_output() {
        let good = bounded_output(
            Command::new("/bin/sh").args(["-c", "printf '[]'"]),
            Duration::from_secs(2),
            128,
        )
        .unwrap();
        assert_eq!(good.stdout, b"[]");
        assert!(bounded_output(
            Command::new("/bin/sh").args(["-c", "while :; do :; done"]),
            Duration::from_millis(30),
            128,
        )
        .unwrap_err()
        .contains("timed out"));
        assert!(bounded_output(
            Command::new("/bin/sh").args(["-c", "while :; do printf 'abcdefgh'; done"]),
            Duration::from_secs(2),
            128,
        )
        .unwrap_err()
        .contains("exceeded limit"));
    }

    fn draft_value() -> Value {
        json!({
            "draft_version": 1,
            "archive_root": "/archive",
            "source_root": "/source",
            "expected_volume_uuid": "1234-ABCD",
            "organization": {"categories": [{"name": "Trips", "annotation": "intentional"}]},
            "mappings": [{
                "source_collection": "2016-06 New York",
                "proposed_destination_relative": "Trips/2016/2016-06 New York",
                "status": "destination_reviewed",
                "date_probe": {"unvalidated": true}
            }],
            "execution": {"file_moves_authorized": false, "annotation": "draft only"}
        })
    }
    fn parse(value: &Value) -> Result<Draft> {
        Draft::parse(&value.to_string())
    }

    #[test]
    fn draft_subset_accepts_annotations_and_only_explicit_review_statuses() {
        let mut value = draft_value();
        assert!(parse(&value).is_ok());
        value["mappings"][0]["status"] = json!("provisional_event_membership_needs_review");
        assert!(parse(&value).is_ok());
        for status in [
            "reviewed",
            "approved",
            "arbitrary",
            "",
            "destination_reviewed ",
        ] {
            value["mappings"][0]["status"] = json!(status);
            assert!(parse(&value).is_err(), "{status}");
        }
        let mut value = draft_value();
        value["execution"]["file_moves_authorized"] = json!(true);
        assert!(parse(&value).is_err());
        value["execution"]["file_moves_authorized"] = json!(false);
        value["draft_version"] = json!(2);
        assert!(parse(&value).is_err());
        for field in [
            "execution",
            "organization",
            "archive_root",
            "source_root",
            "expected_volume_uuid",
            "mappings",
            "draft_version",
        ] {
            let mut missing = draft_value();
            missing.as_object_mut().unwrap().remove(field);
            assert!(parse(&missing).is_err(), "missing {field}");
        }
        for field in [
            "source_collection",
            "status",
            "proposed_destination_relative",
        ] {
            let mut missing = draft_value();
            missing["mappings"][0]
                .as_object_mut()
                .unwrap()
                .remove(field);
            assert!(parse(&missing).is_err(), "missing {field}");
        }
    }

    #[test]
    fn draft_names_destinations_duplicates_and_candidate_alias() {
        for name in ["", ".", "..", "/absolute", "a/b", "a\\b", "a\0b"] {
            for field in ["source", "category"] {
                let mut value = draft_value();
                if field == "source" {
                    value["mappings"][0]["source_collection"] = json!(name);
                } else {
                    value["organization"]["categories"][0]["name"] = json!(name);
                }
                assert!(parse(&value).is_err(), "{field}: {name:?}");
            }
        }
        for destination in [
            "/Trips/a",
            "Trips/../a",
            "Trips/./a",
            "Trips//a",
            "Trips/a/",
            "Other/a",
            "Trips/a\\b",
            "Trips/a\0b",
            "",
        ] {
            let mut value = draft_value();
            value["mappings"][0]["proposed_destination_relative"] = json!(destination);
            assert!(parse(&value).is_err(), "{destination:?}");
        }
        let mut value = draft_value();
        value["organization"]["categories"]
            .as_array_mut()
            .unwrap()
            .push(json!({"name": "Trips"}));
        assert!(parse(&value).is_err());
        let mut value = draft_value();
        let duplicate = value["mappings"][0].clone();
        value["mappings"].as_array_mut().unwrap().push(duplicate);
        assert!(parse(&value).is_err());
        let mut value = draft_value();
        let mapping = value["mappings"][0].as_object_mut().unwrap();
        let destination = mapping.remove("proposed_destination_relative").unwrap();
        mapping.insert("candidate_destination_relative".into(), destination);
        assert!(parse(&value).is_ok());
        value["mappings"][0]["proposed_destination_relative"] = json!("Trips/other");
        assert!(parse(&value).is_err());
        for path in [
            "relative",
            "/source/../other",
            "/source/./other",
            "/source\0",
        ] {
            let mut value = draft_value();
            value["source_root"] = json!(path);
            assert!(parse(&value).is_err());
        }
    }

    fn summary(rows: Value, label: &str, expected: &[&str]) -> MetadataSummary {
        let expected: BTreeSet<String> = expected.iter().map(|s| (*s).into()).collect();
        let mut result = MetadataSummary::new(expected.len());
        consume_metadata(
            &serde_json::to_vec(&rows).unwrap(),
            &expected,
            label,
            &mut result,
        );
        result
    }

    #[test]
    fn new_york_keys_2016_wins_over_2026_container_with_no_timezone_conversion() {
        let result = summary(
            json!([{
                "SourceFile": "/photo.mov",
                "Keys:CreationDate": "2016:06:30 23:59:59-04:00",
                "QuickTime:CreateDate": "2026:01:01 01:00:00",
                "Track1:TrackCreateDate": "2026:01:01 01:00:00"
            }]),
            "2016-06 New York",
            &["/photo.mov"],
        );
        assert!(result.complete);
        assert_eq!(result.date_min.as_deref(), Some("2016-06-30"));
        assert_eq!(result.date_source_counts["Keys:CreationDate"], 1);
        assert_eq!(result.container_only_files, 0);
        assert_eq!(result.disagreement_files, 1);
        assert_eq!(result.outside_collection_month_files, 0);
        assert_eq!(result.evidence_outside_collection_month_files, 1);
        assert_eq!(result.date_field_counts["QuickTime:CreateDate"], 1);
        let result = summary(
            json!([{
                "SourceFile": "/photo.jpg",
                "ExifIFD:DateTimeOriginal": "2015:12:31 23:59:59",
                "Keys:CreationDate": "2016:06:01 00:00:00",
                "QuickTime:CreateDate": "2026:01:01 00:00:00"
            }]),
            "2016-06 New York",
            &["/photo.jpg"],
        );
        assert_eq!(result.date_min.as_deref(), Some("2015-12-31"));
        assert_eq!(result.outside_collection_month_files, 1);
        assert_eq!(result.date_source_counts["ExifIFD:DateTimeOriginal"], 1);
    }

    #[test]
    fn container_only_month_spanning_invalid_and_empty_dates_are_not_invented() {
        let result = summary(
            json!([
                {"SourceFile": "/a.mov", "QuickTime:CreateDate": "2026:01:31 23:59:00"},
                {"SourceFile": "/b.mov", "Track1:TrackCreateDate": "2026:02:01 00:01:00"},
                {"SourceFile": "/c.mov", "Keys:CreationDate": "", "QuickTime:CreateDate": "2026:02:30 01:00:00"},
                {"SourceFile": "/d.mov", "ExifIFD:DateTimeOriginal": "0000:00:00 00:00:00"}
            ]),
            "2026-01 collection",
            &["/a.mov", "/b.mov", "/c.mov", "/d.mov"],
        );
        assert!(result.complete);
        assert_eq!(result.container_only_files, 2);
        assert_eq!(result.date_min.as_deref(), Some("2026-01-31"));
        assert_eq!(result.date_max.as_deref(), Some("2026-02-01"));
        assert_eq!(result.month_counts.len(), 2);
        assert_eq!(result.outside_collection_month_files, 1);
        assert_eq!(result.undated_files, 2);
        assert_eq!(result.invalid_date_fields, 3);
        for invalid in [
            "",
            "no date",
            "2026:02:29",
            "2026:13:01",
            "0000:01:01",
            "2026-01-01garbage",
            "2026:01-01",
            "ééééééééé",
        ] {
            assert_eq!(metadata_day(invalid), None, "{invalid}");
        }
        assert_eq!(metadata_day("2024:02:29"), Some("2024-02-29".into()));
    }

    #[test]
    fn digitization_and_xmp_creation_dates_are_not_mislabeled_as_container_only() {
        let result = summary(
            json!([
                {"SourceFile": "/a.jpg", "ExifIFD:CreateDate": "2016:01:01"},
                {"SourceFile": "/b.jpg", "XMP-xmp:CreateDate": "2016:01:01"},
                {"SourceFile": "/c.mov", "QuickTime:CreateDate": "2016:01:01"},
                {"SourceFile": "/d.mov", "QuickTime:CreateDate": "2016:01:01", "XMP-xmp:CreateDate": "2016:01:01"}
            ]),
            "collection",
            &["/a.jpg", "/b.jpg", "/c.mov", "/d.mov"],
        );
        assert_eq!(result.lower_priority_date_only_files, 4);
        assert_eq!(result.container_only_files, 1);
    }

    #[test]
    fn gps_presence_including_zero_and_diagnostic_counts_never_serialize_values() {
        let result = summary(
            json!([
                {"SourceFile": "/a.jpg", "GPS:GPSLatitude": 0, "GPS:GPSLongitude": 0,
                 "Composite:GPSPosition": "SECRET_COORDINATES", "ExifTool:Warning": "RAW_WARNING"},
                {"SourceFile": "/b.mov", "Keys:GPSCoordinates": "+52.12345+013.98765",
                 "QuickTime:LocationInformation": "PRIVATE_LOCATION", "ExifTool:Error": "RAW_ERROR"}
            ]),
            "collection",
            &["/a.jpg", "/b.mov"],
        );
        assert_eq!(result.gps_tag_files, 2);
        assert_eq!(result.gps_tag_presence_counts["GPSLatitude"], 1);
        assert_eq!(result.gps_tag_presence_counts["GPSLongitude"], 1);
        assert_eq!(result.warning_count, 1);
        assert_eq!(result.error_count, 1);
        assert!(!result.complete);
        let serialized = serde_json::to_string(&result).unwrap();
        for private in [
            "SECRET_COORDINATES",
            "52.12345",
            "013.98765",
            "PRIVATE_LOCATION",
            "RAW_WARNING",
            "RAW_ERROR",
            "/a.jpg",
        ] {
            assert!(!serialized.contains(private), "{private}");
        }
    }

    #[test]
    fn unexpected_missing_and_duplicate_metadata_rows_are_blockers() {
        let result = summary(
            json!([
                {"SourceFile": "/a.jpg", "ExifIFD:DateTimeOriginal": "2016:01:01"},
                {"SourceFile": "/a.jpg", "ExifIFD:DateTimeOriginal": "2017:01:01"},
                {"SourceFile": "/unexpected.jpg", "ExifIFD:DateTimeOriginal": "2020:01:01"},
                {"ExifTool:Error": "SECRET_RAW_ERROR"}
            ]),
            "collection",
            &["/a.jpg", "/missing.jpg"],
        );
        assert!(!result.complete);
        assert_eq!(result.duplicate_rows, 1);
        assert_eq!(result.missing_rows, 1);
        assert_eq!(result.unexpected_rows, 2);
        assert_eq!(result.rows_read, 0);
        assert!(result.date_min.is_none());
        assert!(!serde_json::to_string(&result)
            .unwrap()
            .contains("SECRET_RAW_ERROR"));
        let mut result = MetadataSummary::new(1);
        consume_metadata(
            b"RAW_NOT_JSON",
            &BTreeSet::from(["/a.jpg".into()]),
            "collection",
            &mut result,
        );
        assert!(!result.complete);
        assert_eq!(result.missing_rows, 1);
        assert!(!serde_json::to_string(&result)
            .unwrap()
            .contains("RAW_NOT_JSON"));
    }

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let root = std::env::temp_dir().canonicalize().unwrap().join(format!(
                "media-preview-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&root).unwrap();
            fs::create_dir(root.join("source")).unwrap();
            fs::create_dir(root.join("archive")).unwrap();
            Self(root)
        }
        fn draft(&self) -> Draft {
            let mut value = draft_value();
            value["source_root"] = json!(self.0.join("source"));
            value["archive_root"] = json!(self.0.join("archive"));
            parse(&value).unwrap()
        }
        fn collection(&self) -> PathBuf {
            let path = self.0.join("source/2016-06 New York");
            fs::create_dir(&path).unwrap();
            path
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    fn tree_snapshot(path: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        let mut result = BTreeMap::new();
        for entry in fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                result.insert(path.clone(), Vec::new());
                result.extend(tree_snapshot(&path));
            } else {
                result.insert(path.clone(), fs::read(path).unwrap());
            }
        }
        result
    }

    #[test]
    fn fixture_preview_has_no_writes_db_or_hashes_and_unmapped_loose_files_are_unresolved() {
        let fixture = Fixture::new();
        let collection = fixture.collection();
        fs::write(
            collection.join("photo.JPG"),
            b"not real media; metadata-only fixture",
        )
        .unwrap();
        fs::create_dir(collection.join("nested")).unwrap();
        fs::write(collection.join("nested/video.mov"), b"synthetic").unwrap();
        fs::create_dir(fixture.0.join("source/unmapped")).unwrap();
        fs::write(fixture.0.join("source/.DS_Store"), b"loose fixture").unwrap();
        let before = tree_snapshot(&fixture.0);
        let stamps: Vec<_> = before
            .keys()
            .filter(|path| path.is_file())
            .map(|path| file_stamp(path).unwrap())
            .collect();
        let report = evaluate(&fixture.draft()).unwrap();
        assert!(report.complete);
        assert!(!report.moves_authorized);
        assert_eq!(report.loose_files.len(), 1);
        assert!(report
            .issues
            .iter()
            .any(|issue| issue.contains("unresolved")));
        assert_eq!(report.collections[0].status, "proposal");
        assert_eq!(report.collections[0].file_count, 2);
        assert_eq!(report.collections[0].extension_counts["jpg"], 1);
        assert_eq!(report.collections[0].extension_counts["mov"], 1);
        assert_eq!(
            report.collections[0].apparent_bytes,
            stamps
                .iter()
                .filter(|s| s.path.starts_with(&collection))
                .map(|s| s.size)
                .sum::<u64>()
        );
        assert_eq!(report.collections[1].status, "unresolved");
        assert!(report
            .collections
            .iter()
            .all(|c| !c.moves_authorized && c.metadata.is_none()));
        assert_eq!(before, tree_snapshot(&fixture.0));
        for stamp in stamps {
            assert_eq!(file_stamp(&stamp.path).unwrap(), stamp);
        }
        assert!(before
            .keys()
            .all(|path| path.extension().is_none_or(|e| e != "db")));
    }

    #[test]
    fn provisional_mapping_never_proposes_and_absent_mapping_is_blocked() {
        let fixture = Fixture::new();
        fixture.collection();
        let mut draft = fixture.draft();
        draft.mappings[0].status = Review::ProvisionalEventMembershipNeedsReview;
        let report = evaluate(&draft).unwrap();
        assert!(report.complete);
        assert_eq!(report.collections[0].status, "review");
        assert!(report.collections[0]
            .proposal_destination_relative
            .is_none());
        fs::remove_dir(fixture.0.join("source/2016-06 New York")).unwrap();
        let report = evaluate(&draft).unwrap();
        assert!(!report.complete);
        assert_eq!(report.collections[0].status, "blocked");
    }

    #[test]
    fn a_rule_reviewed_mapping_is_reported_as_a_rule_placement_not_as_unresolved_event_membership() {
        let fixture = Fixture::new();
        fixture.collection();
        let mut draft = fixture.draft();
        draft.mappings[0].status = Review::RuleReviewed;
        let report = evaluate(&draft).unwrap();
        assert!(report.complete);
        assert_eq!(report.collections[0].status, "rule");
        assert_eq!(
            report.collections[0].proposal_destination_relative.as_deref(),
            Some("Trips/2016/2016-06 New York")
        );
        assert!(report.collections[0].issues.is_empty());
    }

    #[test]
    fn existing_target_or_non_directory_ancestor_is_conflict_not_overwrite() {
        let fixture = Fixture::new();
        fixture.collection();
        let target = fixture.0.join("archive/Trips/2016/2016-06 New York");
        fs::create_dir_all(&target).unwrap();
        let report = evaluate(&fixture.draft()).unwrap();
        assert!(report.complete);
        assert_eq!(report.collections[0].status, "conflict");
        assert!(report.collections[0]
            .proposal_destination_relative
            .is_none());
        fs::remove_dir_all(fixture.0.join("archive/Trips")).unwrap();
        fs::write(fixture.0.join("archive/Trips"), b"not directory").unwrap();
        assert_eq!(
            evaluate(&fixture.draft()).unwrap().collections[0].status,
            "conflict"
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlink_containment_non_utf8_and_overlap_are_blockers() {
        use std::os::unix::{ffi::OsStringExt, fs::symlink};
        let fixture = Fixture::new();
        let collection = fixture.collection();
        symlink(fixture.0.join("source"), fixture.0.join("source-link")).unwrap();
        let mut draft = fixture.draft();
        draft.source_root = fixture.0.join("source-link");
        assert!(evaluate(&draft).is_err());
        let mut draft = fixture.draft();
        draft.archive_root = collection.clone();
        assert!(evaluate(&draft).is_err());
        symlink(&fixture.0, fixture.0.join("archive/Trips")).unwrap();
        assert_eq!(
            evaluate(&fixture.draft()).unwrap().collections[0].status,
            "conflict"
        );
        symlink(fixture.0.join("archive"), collection.join("escape")).unwrap();
        let report = evaluate(&fixture.draft()).unwrap();
        assert!(!report.complete);
        assert_eq!(report.collections[0].status, "blocked");
        assert_eq!(report.collections[0].file_count, 0);
        fs::remove_file(collection.join("escape")).unwrap();
        let mut draft = fixture.draft();
        draft
            .source_root
            .push(std::ffi::OsString::from_vec(vec![0xff]));
        assert!(evaluate(&draft).is_err());
        // macOS filesystems refuse non-UTF-8 names at creation; Linux permits them.
        #[cfg(not(target_os = "macos"))]
        {
            fs::write(
                collection.join(std::ffi::OsString::from_vec(vec![0xff])),
                b"fixture",
            )
            .unwrap();
            assert!(!evaluate(&fixture.draft()).unwrap().complete);
            fs::write(
                fixture
                    .0
                    .join("source")
                    .join(std::ffi::OsString::from_vec(vec![0xfe])),
                b"fixture",
            )
            .unwrap();
            assert!(evaluate(&fixture.draft())
                .unwrap()
                .issues
                .iter()
                .any(|s| s.contains("non-UTF-8")));
        }
    }

    #[test]
    fn directory_inventory_detects_added_removed_entries_and_file_changes() {
        let fixture = Fixture::new();
        let root = fixture.collection();
        fs::write(root.join("a.mov"), b"fixture").unwrap();
        let device = directory_stamp(&root).unwrap().0;
        let mut before = Collection::new("fixture".into());
        let dirs = walk_collection(&root, device, &mut before);
        fs::create_dir(root.join("new_empty_dir")).unwrap();
        let mut added = Collection::new("fixture".into());
        assert_ne!(dirs, walk_collection(&root, device, &mut added));
        assert_eq!(before.files, added.files);
        fs::write(root.join("a.mov"), b"changed fixture").unwrap();
        let mut changed = Collection::new("fixture".into());
        walk_collection(&root, device, &mut changed);
        assert_ne!(before.files, changed.files);
        fs::remove_file(root.join("a.mov")).unwrap();
        let mut removed = Collection::new("fixture".into());
        walk_collection(&root, device, &mut removed);
        assert_ne!(before.files, removed.files);
    }

    #[test]
    fn reviewed_destinations_cannot_overlap() {
        let fixture = Fixture::new();
        fixture.collection();
        fs::create_dir(fixture.0.join("source/other")).unwrap();
        let mut draft = fixture.draft();
        draft.mappings.push(Mapping {
            source_collection: "other".into(),
            proposed_destination_relative: Some("Trips/2016".into()),
            candidate_destination_relative: None,
            status: Review::DestinationReviewed,
        });
        let report = evaluate(&draft).unwrap();
        assert!(report.complete);
        assert!(report
            .collections
            .iter()
            .all(|c| c.status == "conflict" && c.proposal_destination_relative.is_none()));
    }
}
