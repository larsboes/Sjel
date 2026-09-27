//! A declared markdown root, and the only way to get a file out of it.
//!
//! Two capabilities read markdown out of a knowledge store the operator points
//! them at: scouting resolves opportunity notes and interest profiles, calendar
//! imports event notes. Both take the same two inputs — a root path from the
//! private overlay and a glob relative to it — and both have to answer the same
//! question before reading anything: *is this file actually inside the root the
//! operator declared?*
//!
//! Before this existed the answer was "assume so". `scouting/src/score.rs` and
//! `scouting/src/sources/obsidian_md.rs` each held their own `root.join(pattern)`
//! resolver, already diverged (one handled an exact file, the other did not),
//! and neither checked containment: a pattern of `../../.ssh` resolved happily,
//! as would a symlink pointing out of the vault. Calendar's importer would have
//! been the third copy of that.
//!
//! ## What the type guarantees
//!
//! A `MarkdownRoot` exists only for a directory that exists. Every path it
//! hands back is inside that directory, checked after symlink resolution rather
//! than by string prefix. Anything that would escape is an error naming the
//! offending path, never a silently dropped file — the operator gets to fix the
//! config, which they cannot do if the resolver quietly returns less.
//!
//! ## What it deliberately is not
//!
//! Not a glob engine. The patterns in play are `Some/Dir/*.md` and one exact
//! `Some/File.md`, which is what the config shape has ever declared; a real
//! matcher would be a dependency, and these two shapes cover every declared
//! contract without one.
//!
//! Not a reader of *meaning*. `frontmatter()` below owns the format — which
//! lines are frontmatter at all, and what a YAML list flattens to. Which keys
//! matter and what they say belongs to the capability that declared the root:
//! scouting reads `type`/`summary`/`category`, calendar reads `start`/`end`/
//! `status`, and neither has an opinion about the other's.
//!
//! Not tilde expansion: `sjel_config::expand_tilde` owns that, and the caller
//! applies it before declaring a root.

pub mod projection;
pub mod region;

pub use projection::ProjectionOutcome;
pub use region::{apply, find, FoundRegion, RegionError, RegionOutcome, RegionSpec};

use std::collections::HashMap;
use std::fmt;
use std::path::{Component, Path, PathBuf};

/// Why a path could not be produced. Every variant names the thing at fault,
/// because each one is an operator config error rather than a runtime blip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RootError {
    /// The declared root does not exist. Not treated as "empty": a vault that
    /// moved should say so rather than look like a vault with no notes in it.
    Missing(PathBuf),
    /// The declared root exists but is a file.
    NotADirectory(PathBuf),
    /// The root exists but its real location could not be resolved.
    RootUnresolvable { path: PathBuf, detail: String },
    /// A pattern that starts at the filesystem root ignores the declared root
    /// entirely, so it is refused rather than honoured.
    PatternAbsolute(String),
    /// A pattern containing `..` — refused before any read, so no evidence of
    /// what is outside the root leaks through an error message either.
    PatternEscapes(String),
    /// A resolved path that leaves the root once symlinks are followed.
    Escapes { path: PathBuf, root: PathBuf },
    /// A directory named by a valid pattern could not be listed.
    Unreadable { path: PathBuf, detail: String },
    /// A projection path that does not name a markdown file. This root is the
    /// only way into a knowledge store, and a knowledge store holds notes; a
    /// writer that can put arbitrary file types in one has stopped being that.
    NotMarkdown(String),
    /// A projection could not be written or deleted. Read errors keep using
    /// `Unreadable`; this one exists so a full disk and a missing file do not
    /// arrive at the caller as the same sentence.
    Unwritable { path: PathBuf, detail: String },
}

impl fmt::Display for RootError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RootError::Missing(p) => write!(f, "declared root does not exist: {}", p.display()),
            RootError::NotADirectory(p) => {
                write!(f, "declared root is not a directory: {}", p.display())
            }
            RootError::RootUnresolvable { path, detail } => {
                write!(
                    f,
                    "cannot resolve declared root {}: {detail}",
                    path.display()
                )
            }
            RootError::PatternAbsolute(pattern) => write!(
                f,
                "pattern '{pattern}' is absolute; patterns are relative to the declared root"
            ),
            RootError::PatternEscapes(pattern) => {
                write!(f, "pattern '{pattern}' escapes the declared root with '..'")
            }
            RootError::Escapes { path, root } => write!(
                f,
                "{} resolves outside the declared root {}",
                path.display(),
                root.display()
            ),
            RootError::Unreadable { path, detail } => {
                write!(f, "cannot read {}: {detail}", path.display())
            }
            RootError::NotMarkdown(path) => {
                write!(f, "projection path '{path}' does not end in .md")
            }
            RootError::Unwritable { path, detail } => {
                write!(f, "cannot write {}: {detail}", path.display())
            }
        }
    }
}

impl std::error::Error for RootError {}

/// A directory the operator declared, resolved to its real location once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkdownRoot {
    root: PathBuf,
}

impl MarkdownRoot {
    /// Declare a root. Fails rather than deferring: a capability that cannot
    /// name where its notes live has nothing useful to do next.
    pub fn declare(path: impl Into<PathBuf>) -> Result<Self, RootError> {
        let path = path.into();
        if !path.exists() {
            return Err(RootError::Missing(path));
        }
        let root = std::fs::canonicalize(&path).map_err(|e| RootError::RootUnresolvable {
            path: path.clone(),
            detail: e.to_string(),
        })?;
        if !root.is_dir() {
            return Err(RootError::NotADirectory(root));
        }
        Ok(Self { root })
    }

    /// The resolved root. Already canonical, so it is safe to compare against.
    pub fn path(&self) -> &Path {
        &self.root
    }

    /// The one location a pattern names — a directory for `Dir/*.md`, the file
    /// itself for `Dir/File.md`. May not exist: callers that treat an absent
    /// profile as "no profile" need to make that call themselves, and an
    /// existence check here would take it away from them.
    pub fn locate(&self, pattern: &str) -> Result<PathBuf, RootError> {
        let relative = check_pattern(pattern)?;
        let exact = self.root.join(relative);
        if exact.is_file() {
            return self.contained(exact);
        }
        let dir = strip_markdown_glob(relative);
        let located = if dir.is_empty() {
            self.root.clone()
        } else {
            self.root.join(dir)
        };
        // An absent path cannot be canonicalized, so it cannot be proven
        // contained either. It is still lexically inside a root with no `..`
        // in play, and returning it lets the caller report "no profile there"
        // instead of "your config is wrong".
        if located.exists() {
            self.contained(located)
        } else {
            Ok(located)
        }
    }

    /// Every `.md` file the pattern names, sorted, each proven inside the root.
    ///
    /// A pattern naming one file yields that file. A directory pattern yields
    /// its markdown children, without recursing — the config has never declared
    /// a recursive glob, and inventing one here would widen what an operator
    /// already declared.
    pub fn markdown_files(&self, pattern: &str) -> Result<Vec<PathBuf>, RootError> {
        let relative = check_pattern(pattern)?;
        let exact = self.root.join(relative);
        if exact.is_file() {
            return Ok(vec![self.contained(exact)?]);
        }

        let dir = strip_markdown_glob(relative);
        let directory = if dir.is_empty() {
            self.root.clone()
        } else {
            self.root.join(dir)
        };
        let entries = std::fs::read_dir(&directory).map_err(|e| RootError::Unreadable {
            path: directory.clone(),
            detail: e.to_string(),
        })?;

        let mut files = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            files.push(self.contained(path)?);
        }
        files.sort();
        Ok(files)
    }

    /// A file's identity relative to the root — the record half of a stable
    /// `(source, external_id)` pair. Slash-separated regardless of platform, so
    /// the identity a note is imported under does not depend on the machine
    /// that imported it.
    pub fn relative_id(&self, file: &Path) -> Option<String> {
        let resolved = std::fs::canonicalize(file).unwrap_or_else(|_| file.to_path_buf());
        let relative = resolved.strip_prefix(&self.root).ok()?;
        let joined: Vec<String> = relative
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        (!joined.is_empty()).then(|| joined.join("/"))
    }

    /// Prove a path is inside the root with symlinks followed, not by string
    /// prefix: a symlink inside the vault pointing at `~/.ssh` passes a prefix
    /// check and fails this one.
    fn contained(&self, path: PathBuf) -> Result<PathBuf, RootError> {
        let resolved = std::fs::canonicalize(&path).map_err(|e| RootError::Unreadable {
            path: path.clone(),
            detail: e.to_string(),
        })?;
        if resolved.starts_with(&self.root) {
            Ok(resolved)
        } else {
            Err(RootError::Escapes {
                path,
                root: self.root.clone(),
            })
        }
    }
}

/// Refuse a pattern that would leave the root, before it is joined to anything.
fn check_pattern(pattern: &str) -> Result<&str, RootError> {
    let trimmed = pattern.trim();
    let as_path = Path::new(trimmed);
    if as_path.is_absolute() {
        return Err(RootError::PatternAbsolute(trimmed.to_string()));
    }
    if as_path
        .components()
        .any(|c| matches!(c, Component::ParentDir | Component::Prefix(_)))
    {
        return Err(RootError::PatternEscapes(trimmed.to_string()));
    }
    Ok(trimmed)
}

/// `Dir/*.md` and `Dir/` both mean the directory `Dir`.
fn strip_markdown_glob(pattern: &str) -> &str {
    pattern
        .strip_suffix("/*.md")
        .or_else(|| pattern.strip_suffix("*.md"))
        .unwrap_or(pattern)
        .trim_end_matches('/')
}

/// A note's frontmatter as `key -> value`, with YAML lists flattened to a
/// comma-separated string.
///
/// Deliberately YAML-*like* rather than YAML: the whole surface in play is
/// scalars and one level of list, and a real YAML parser is a dependency this
/// crate is built to avoid. It came out of `scouting`'s Obsidian adapter, where
/// it was already the shape both that adapter and calendar's importer needed —
/// same format, different fields.
///
/// Absent frontmatter is an empty map rather than an error: a note without it
/// is a note, just not one either caller can do anything with. Unterminated
/// frontmatter *is* an error, because it means the file's structure is not what
/// it claims and everything read after that point would be invented.
pub fn frontmatter(md: &str) -> Result<HashMap<String, String>, String> {
    if !md.starts_with("---") {
        return Ok(HashMap::new());
    }

    let end = md[3..]
        .find("---")
        .map(|i| i + 3)
        .ok_or("unclosed frontmatter")?;
    Ok(parse_fields(&md[3..end]))
}

/// The field parser, extracted so two delimiters can share it.
///
/// `frontmatter` finds its closing fence by searching for the next `---`
/// anywhere; `frontmatter_spanned` requires that fence to sit alone on a line.
/// They disagree on where the block ends and must not disagree on what the
/// block means, which is what a second copy of this loop would eventually do.
fn parse_fields(block: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let mut in_list = false;
    let mut list_key = String::new();
    let mut list_items: Vec<String> = Vec::new();

    for line in block.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            // Flush pending list on blank line
            if in_list && !list_key.is_empty() {
                map.insert(list_key.clone(), list_items.join(", "));
                in_list = false;
                list_key.clear();
                list_items.clear();
            }
            continue;
        }

        if in_list {
            if trimmed.starts_with('-') {
                let item = trimmed.trim_start_matches('-').trim().trim_matches('"');
                if !item.is_empty() {
                    list_items.push(item.to_string());
                }
                continue;
            } else {
                // End of list — store what we collected
                if !list_key.is_empty() {
                    map.insert(list_key.clone(), list_items.join(", "));
                }
                in_list = false;
                list_key.clear();
                list_items.clear();
            }
        }

        if let Some((key, val)) = trimmed.split_once(':') {
            let k = key.trim().to_string();
            let v = val.trim().to_string();

            if v.starts_with('[') {
                // Inline array: [url1, url2]
                let inner = v.trim_start_matches('[').trim_end_matches(']');
                let items: Vec<String> = inner
                    .split(',')
                    .map(|s| s.trim().trim_matches('"').to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                map.insert(k, items.join(", "));
            } else if v.starts_with('-') {
                // YAML list starting on the same line: `key: - item`
                let first = v.trim_start_matches('-').trim().trim_matches('"');
                in_list = true;
                list_key = k;
                list_items = vec![first.to_string()];
            } else if v.is_empty() {
                // Key with no value on this line — might be a YAML list on the
                // following lines. Open a list and check the next line.
                in_list = true;
                list_key = k;
                list_items.clear();
            } else {
                map.insert(k, v.trim_matches('"').to_string());
            }
        } else if trimmed.starts_with('-') && in_list {
            let item = trimmed.trim_start_matches('-').trim().trim_matches('"');
            if !item.is_empty() {
                list_items.push(item.to_string());
            }
        }
    }

    // Flush any open list at end of block
    if in_list && !list_key.is_empty() {
        map.insert(list_key, list_items.join(", "));
    }

    map
}

// ---------------------------------------------------------------------------
// Whole-root traversal and byte-addressable frontmatter.
//
// Added for `capabilities/vault`, which asks two questions the pattern-based
// API above cannot answer. First: what is EVERY note in this root? The glob
// shapes exist because a config declared them, and widening those would widen
// what an operator asked for — but a vault-wide lint has no pattern to widen,
// it has the vault. Second: where does the frontmatter END in bytes? A tool
// that rewrites frontmatter and must leave the body untouched cannot work from
// a HashMap; it needs an offset it can splice at.
//
// Both live here rather than in the capability for the reason the module doc
// already gives: containment is the guarantee this type exists to provide, and
// a caller that walks the tree itself has quietly opted out of it.

/// A parsed frontmatter block plus the offsets that make it rewritable.
///
/// `body_start` is the byte index of the first character after the closing
/// fence and its newline. Slicing a file at it yields the body exactly as it
/// sits on disk, which is what lets a writer re-serialise the frontmatter and
/// concatenate the original body bytes rather than round-tripping them through
/// a parser that would reformat Bases embeds, Mermaid fences and callouts.
#[derive(Debug, Clone)]
pub struct Frontmatter {
    pub fields: HashMap<String, String>,
    /// Byte range of the YAML between the fences, absent when the file has none.
    pub block: Option<(usize, usize)>,
    pub body_start: usize,
}

impl MarkdownRoot {
    /// Every `.md` file under the root, recursively, sorted, each proven inside it.
    ///
    /// Directories whose name starts with `.` are skipped. In an Obsidian vault
    /// those are `.obsidian`, `.trash`, `.smart-env` and `.git`: plugin state,
    /// deleted notes and the object store. Counting `.trash` is how a vault
    /// reports 2,284 notes when it has 2,248, and a lint that walks `.git`
    /// reports on blobs.
    /// Every file under the root, whatever its extension.
    ///
    /// `markdown_files_recursive` answers "which notes are there"; this answers "which files
    /// are there", which is a different question and has one consumer: a wikilink may name a
    /// PDF, an image, a `.base` or a `.canvas`, and a link checker that only knows notes
    /// reports every one of those as dead. Measured on the operator's vault on 2026-09-07,
    /// that was 685 of 9,362 reported dead links.
    pub fn files_recursive(&self) -> Result<Vec<PathBuf>, RootError> {
        self.walk(false)
    }

    pub fn markdown_files_recursive(&self) -> Result<Vec<PathBuf>, RootError> {
        self.walk(true)
    }

    /// The one walk both public verbs use. `markdown_only` is the whole difference.
    fn walk(&self, markdown_only: bool) -> Result<Vec<PathBuf>, RootError> {
        let wanted =
            |path: &Path| !markdown_only || path.extension().and_then(|e| e.to_str()) == Some("md");
        let mut files = Vec::new();
        let mut stack = vec![self.root.clone()];

        while let Some(dir) = stack.pop() {
            let entries = std::fs::read_dir(&dir).map_err(|e| RootError::Unreadable {
                path: dir.clone(),
                detail: e.to_string(),
            })?;
            for entry in entries.flatten() {
                let path = entry.path();
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if name.starts_with('.') {
                    continue;
                }
                match entry.file_type() {
                    Ok(t) if t.is_dir() => stack.push(path),
                    Ok(t) if t.is_file() => {
                        if wanted(&path) {
                            files.push(self.contained(path)?);
                        }
                    }
                    // A symlink or an entry whose type we cannot read: `contained`
                    // is the authority on whether it belongs, so ask it rather
                    // than guessing from the dirent.
                    _ => {
                        if wanted(&path) && path.is_file() {
                            files.push(self.contained(path)?);
                        }
                    }
                }
            }
        }
        files.sort();
        Ok(files)
    }
}

/// `frontmatter`, plus the offsets a rewriter needs.
///
/// One deliberate divergence from `frontmatter` above: the closing fence must
/// sit alone on its own line. The older function searches for the next `---`
/// anywhere, which is right for the fields it was written to read and wrong the
/// moment a value contains a triple dash — there, it truncates the block early
/// and every key below the dash vanishes. Reading fewer fields is survivable;
/// splicing at an offset three lines above the real fence is not, so this one
/// is line-anchored. The older function keeps its behaviour: scouting and
/// calendar are calibrated against it and this crate does not get to change
/// what they read on the way past.
pub fn frontmatter_spanned(md: &str) -> Result<Frontmatter, String> {
    let no_frontmatter = Frontmatter {
        fields: HashMap::new(),
        block: None,
        body_start: 0,
    };

    // A leading `---` is only a fence when the line holds nothing else.
    let first_line_end = md.find('\n').unwrap_or(md.len());
    if md[..first_line_end].trim_end() != "---" {
        return Ok(no_frontmatter);
    }
    let block_start = first_line_end + 1;

    let mut cursor = block_start;
    let close = loop {
        if cursor >= md.len() {
            return Err("unclosed frontmatter".to_string());
        }
        let line_end = md[cursor..]
            .find('\n')
            .map(|i| cursor + i)
            .unwrap_or(md.len());
        if md[cursor..line_end].trim_end() == "---" {
            break (cursor, line_end);
        }
        cursor = line_end + 1;
    };
    let (fence_start, fence_end) = close;

    // Share the field parser, not the delimiter. Handing the block back to
    // `frontmatter` would re-run its unanchored search and undo the anchoring
    // this function exists for — the test below is the one that caught it.
    let fields = parse_fields(&md[block_start..fence_start]);

    let body_start = if fence_end < md.len() {
        fence_end + 1
    } else {
        md.len()
    };

    Ok(Frontmatter {
        fields,
        block: Some((block_start, fence_start)),
        body_start,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A throwaway tree under the OS temp dir, removed on drop.
    struct Tree(PathBuf);

    impl Tree {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("axon-markdown-root-{name}"));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("temp tree");
            Tree(dir)
        }

        fn file(&self, relative: &str, body: &str) -> PathBuf {
            let path = self.0.join(relative);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("parent");
            }
            std::fs::write(&path, body).expect("write");
            path
        }

        fn dir(&self, relative: &str) -> PathBuf {
            let path = self.0.join(relative);
            std::fs::create_dir_all(&path).expect("dir");
            path
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_root_that_does_not_exist_is_named_rather_than_treated_as_empty() {
        let missing = std::env::temp_dir().join("axon-markdown-root-nope-1a2b3c");
        let _ = std::fs::remove_dir_all(&missing);
        assert_eq!(
            MarkdownRoot::declare(&missing),
            Err(RootError::Missing(missing))
        );
    }

    #[test]
    fn a_file_cannot_be_declared_as_a_root() {
        let tree = Tree::new("root-is-a-file");
        let file = tree.file("notes.md", "x");
        assert!(matches!(
            MarkdownRoot::declare(&file),
            Err(RootError::NotADirectory(_))
        ));
    }

    #[test]
    fn a_directory_pattern_yields_its_markdown_children_sorted() {
        let tree = Tree::new("dir-pattern");
        tree.file("Events/b.md", "b");
        tree.file("Events/a.md", "a");
        tree.file("Events/notes.txt", "not markdown");
        let root = MarkdownRoot::declare(tree.0.clone()).expect("root");

        let files = root.markdown_files("Events/*.md").expect("files");
        let names: Vec<String> = files
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["a.md", "b.md"]);
    }

    #[test]
    fn an_exact_file_pattern_yields_only_that_file() {
        let tree = Tree::new("exact-file");
        tree.file("TELOS/Events Profile.md", "profile");
        tree.file("TELOS/Scholarship Profile.md", "other");
        let root = MarkdownRoot::declare(tree.0.clone()).expect("root");

        let files = root
            .markdown_files("TELOS/Events Profile.md")
            .expect("files");
        assert_eq!(files.len(), 1, "an exact file never sweeps its siblings");
        assert!(files[0].ends_with("Events Profile.md"));
    }

    /// The whole reason this crate exists: the resolvers it replaces would have
    /// walked out of the vault without noticing.
    #[test]
    fn a_pattern_with_a_parent_component_is_refused_before_any_read() {
        let tree = Tree::new("traversal");
        tree.dir("Events");
        let root = MarkdownRoot::declare(tree.0.clone()).expect("root");

        assert_eq!(
            root.markdown_files("../*.md"),
            Err(RootError::PatternEscapes("../*.md".into()))
        );
        assert_eq!(
            root.markdown_files("Events/../../secrets/*.md"),
            Err(RootError::PatternEscapes(
                "Events/../../secrets/*.md".into()
            ))
        );
        assert!(matches!(
            root.locate("../TELOS"),
            Err(RootError::PatternEscapes(_))
        ));
    }

    #[test]
    fn an_absolute_pattern_is_refused_rather_than_honoured() {
        let tree = Tree::new("absolute");
        let root = MarkdownRoot::declare(tree.0.clone()).expect("root");
        assert_eq!(
            root.markdown_files("/etc/*.md"),
            Err(RootError::PatternAbsolute("/etc/*.md".into()))
        );
    }

    /// Prefix checking would pass this; canonicalization is why it does not.
    #[cfg(unix)]
    #[test]
    fn a_symlink_pointing_out_of_the_root_is_an_error_not_a_dropped_file() {
        let outside = Tree::new("symlink-target");
        let secret = outside.file("secret.md", "private");
        let tree = Tree::new("symlink-vault");
        tree.dir("Events");
        std::os::unix::fs::symlink(&secret, tree.0.join("Events/linked.md")).expect("symlink");
        let root = MarkdownRoot::declare(tree.0.clone()).expect("root");

        match root.markdown_files("Events/*.md") {
            Err(RootError::Escapes { path, .. }) => assert!(path.ends_with("linked.md")),
            other => panic!("a symlink out of the vault must fail closed, got {other:?}"),
        }
    }

    #[test]
    fn an_empty_pattern_means_the_root_itself() {
        let tree = Tree::new("empty-pattern");
        tree.file("top.md", "x");
        let root = MarkdownRoot::declare(tree.0.clone()).expect("root");

        assert_eq!(root.markdown_files("*.md").expect("files").len(), 1);
        assert_eq!(root.locate("").expect("located"), root.path());
    }

    #[test]
    fn a_located_path_that_does_not_exist_is_returned_rather_than_refused() {
        let tree = Tree::new("absent-profile");
        let root = MarkdownRoot::declare(tree.0.clone()).expect("root");
        let located = root.locate("TELOS/Missing Profile.md").expect("located");
        assert!(!located.exists(), "the caller decides what absent means");
        assert!(located.starts_with(root.path()));
    }

    #[test]
    fn a_relative_id_is_slash_separated_and_root_anchored() {
        let tree = Tree::new("relative-id");
        let file = tree.file("Atlas/Events/Party.md", "x");
        let root = MarkdownRoot::declare(tree.0.clone()).expect("root");
        assert_eq!(
            root.relative_id(&file).as_deref(),
            Some("Atlas/Events/Party.md")
        );
    }

    #[test]
    fn a_file_outside_the_root_has_no_identity_in_it() {
        let outside = Tree::new("id-outside");
        let stray = outside.file("stray.md", "x");
        let tree = Tree::new("id-root");
        let root = MarkdownRoot::declare(tree.0.clone()).expect("root");
        assert_eq!(root.relative_id(&stray), None);
    }

    // -----------------------------------------------------------------------
    // frontmatter(): the format, not the fields.
    // -----------------------------------------------------------------------

    #[test]
    fn a_note_without_frontmatter_is_an_empty_map_not_an_error() {
        assert_eq!(frontmatter("# Just a heading\n\nbody"), Ok(HashMap::new()));
    }

    #[test]
    fn unterminated_frontmatter_is_an_error_rather_than_a_partial_read() {
        assert!(frontmatter("---\ntype: event\nstart: 2026-02-03\n").is_err());
    }

    #[test]
    fn scalars_lose_their_quotes_and_keep_their_value() {
        let fm = frontmatter("---\ntype: event\nlocation: \"Bonn, DE\"\n---\nbody").unwrap();
        assert_eq!(fm.get("type").map(String::as_str), Some("event"));
        assert_eq!(fm.get("location").map(String::as_str), Some("Bonn, DE"));
    }

    #[test]
    fn a_list_flattens_to_one_comma_separated_value_in_all_three_spellings() {
        let inline = frontmatter("---\nsources: [a, b]\n---\n").unwrap();
        let same_line = frontmatter("---\nsources: - a\n  - b\n---\n").unwrap();
        let block = frontmatter("---\nsources:\n  - a\n  - b\n---\n").unwrap();
        for fm in [inline, same_line, block] {
            assert_eq!(fm.get("sources").map(String::as_str), Some("a, b"));
        }
    }

    /// The shape a real corpus is full of: a key declared with nothing after
    /// it. Two things matter, and only two. It must not swallow the keys below
    /// it, and what it yields must be something a caller rejects rather than
    /// misreads — an empty string parses as no date, no type and no title
    /// everywhere it lands.
    #[test]
    fn an_empty_key_yields_nothing_usable_and_swallows_nothing_below_it() {
        let fm = frontmatter("---\nstart:\nend:\ntype: event\n---\n").unwrap();
        assert_eq!(fm.get("type").map(String::as_str), Some("event"));
        assert_eq!(fm.get("start").map(String::as_str), Some(""));
    }

    #[test]
    fn a_missing_directory_says_so_instead_of_reporting_no_notes() {
        let tree = Tree::new("missing-dir");
        let root = MarkdownRoot::declare(tree.0.clone()).expect("root");
        assert!(matches!(
            root.markdown_files("Nowhere/*.md"),
            Err(RootError::Unreadable { .. })
        ));
    }
}

#[cfg(test)]
mod span_tests {
    use super::*;

    #[test]
    fn body_start_lands_after_the_closing_fence() {
        let md = "---\ntype: note\n---\nBody line one.\n";
        let fm = frontmatter_spanned(md).expect("parse");
        assert_eq!(&md[fm.body_start..], "Body line one.\n");
        assert_eq!(fm.fields.get("type").map(String::as_str), Some("note"));
    }

    /// The reason this function exists rather than reusing `frontmatter`: a
    /// horizontal rule in the body must not be mistaken for the closing fence,
    /// and a value containing a triple dash must not truncate the block.
    #[test]
    fn a_triple_dash_inside_a_value_does_not_close_the_block() {
        let md = "---\nsummary: \"a --- b\"\ntype: note\n---\nBody.\n";
        let fm = frontmatter_spanned(md).expect("parse");
        assert_eq!(fm.fields.get("type").map(String::as_str), Some("note"));
        assert_eq!(&md[fm.body_start..], "Body.\n");
    }

    /// The property A2 depends on: whatever a rewriter does to the frontmatter,
    /// re-concatenating the original body bytes must reproduce them exactly,
    /// including a Bases embed and a Mermaid fence it never parsed.
    #[test]
    fn body_bytes_survive_a_frontmatter_rewrite() {
        let body = "# Title\n\n```base\nfilters:\n  and:\n    - file.inFolder(\"Knowledge\")\n```\n\n---\n\n```mermaid\ngraph TD; A-->B;\n```\n";
        let md = format!("---\ntype: note\n---\n{body}");
        let fm = frontmatter_spanned(&md).expect("parse");
        let rewritten = format!(
            "---\ntype: note\nunderstanding: none\n---\n{}",
            &md[fm.body_start..]
        );
        assert!(rewritten.ends_with(body));
    }

    #[test]
    fn a_file_without_frontmatter_is_all_body() {
        let md = "Just prose.\n";
        let fm = frontmatter_spanned(md).expect("parse");
        assert_eq!(fm.body_start, 0);
        assert!(fm.block.is_none());
    }

    #[test]
    fn an_unclosed_block_is_an_error_not_a_guess() {
        assert!(frontmatter_spanned("---\ntype: note\nno fence ever\n").is_err());
    }
}
