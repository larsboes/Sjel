//! One note, loaded once, reused by every subcommand.
//!
//! Reading the vault is the expensive part — a few thousand files off iCloud —
//! and every verb needs the same three things from each file: where it sits,
//! what its frontmatter says, and where its body begins. Loading is therefore a
//! single pass that keeps the file text, rather than each verb re-reading with
//! its own idea of what a note is.

use std::collections::HashMap;
use std::path::PathBuf;

use markdown_root::{frontmatter_spanned, MarkdownRoot};

pub struct Note {
    /// Vault-relative, slash-separated. The identity that survives a machine.
    pub id: String,
    /// Kept for the writer in A2, which needs somewhere to write back to.
    /// The reader works entirely off `id` and `text`.
    #[allow(dead_code)]
    pub path: PathBuf,
    /// Filename without `.md`. This, not the id, is what a wikilink usually names.
    pub basename: String,
    /// Top-level folder, or an empty string for a note at the vault root.
    pub folder: String,
    pub fields: HashMap<String, String>,
    /// Byte offset where the body starts. Nothing here rewrites yet; the offset
    /// is carried because a rewriter that recomputes it is a rewriter that can
    /// disagree with the reader about where the body was.
    pub body_start: usize,
    /// The raw frontmatter block, fences excluded. Kept because the parsed map
    /// has already dropped the quoting, and quoting is the dialect drift this
    /// tool is meant to see: `knowledge: reference` and `knowledge: "reference"`
    /// parse identically and are two different conventions on disk.
    pub raw_frontmatter: Option<String>,
    pub text: String,
}

impl Note {
    pub fn field(&self, key: &str) -> Option<&str> {
        self.fields.get(key).map(String::as_str)
    }

    /// Present AND carrying something. A key with an empty value is a template
    /// that was never filled in, and counting it as coverage is how a vault
    /// reports 80% `summary` while a fifth of those summaries are blank.
    pub fn has(&self, key: &str) -> bool {
        self.field(key).is_some_and(|v| !v.trim().is_empty())
    }
}

/// Every note under the root, in one pass.
///
/// A file that cannot be read or whose frontmatter is unterminated is returned
/// as a problem rather than skipped. A lint that silently drops what it cannot
/// parse reports a cleaner vault than the one on disk.
pub fn load_all(root: &MarkdownRoot) -> Result<(Vec<Note>, Vec<String>), String> {
    load_scoped(root, "")
}

/// Every note under one folder of the vault, carrying vault-relative ids.
///
/// The same move `tasks::projects_root` makes and for the same reason: reading
/// every note to answer for a folder's worth of them is a per-request cost a
/// server pays on every call. Measured 2026-09-08 on the operator's vault,
/// warm, debug build: `Atlas/People/` and `Journal/` together are **442 notes
/// in 20 ms** against **2,757 notes in 155 ms** for the whole vault, and 433 ms
/// on the cold first read.
///
/// The ids stay vault-relative — `Atlas/People/Erika.md`, not `Erika.md` —
/// because every consumer keys on the identity that survives a machine, and a
/// note whose id changes with the walk that found it is two different notes.
///
/// A folder that is not there is an error and not an empty list. `people` reads
/// `Journal/` to compute a mention count, and a missing `Journal/` would answer
/// zero for all 89 people — which is exactly the reading `people.rs` refuses,
/// because it cannot be told apart from a producer that never ran.
pub fn load_under(vault: &MarkdownRoot, folder: &str) -> Result<(Vec<Note>, Vec<String>), String> {
    let nested = MarkdownRoot::declare(vault.path().join(folder))
        .map_err(|e| format!("{folder}/ under the vault root: {e}"))?;
    load_scoped(&nested, folder)
}

fn load_scoped(root: &MarkdownRoot, prefix: &str) -> Result<(Vec<Note>, Vec<String>), String> {
    let files = root
        .markdown_files_recursive()
        .map_err(|e| format!("walking the vault: {e}"))?;

    let mut notes = Vec::with_capacity(files.len());
    let mut problems = Vec::new();

    for path in files {
        let id = root
            .relative_id(&path)
            .map(|id| {
                if prefix.is_empty() {
                    id
                } else {
                    format!("{prefix}/{id}")
                }
            })
            .unwrap_or_else(|| path.to_string_lossy().into_owned());

        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                problems.push(format!("{id}: unreadable: {e}"));
                continue;
            }
        };

        let (fields, body_start, raw) = match frontmatter_spanned(&text) {
            Ok(fm) => {
                let raw = fm.block.map(|(s, e)| text[s..e].to_string());
                (fm.fields, fm.body_start, raw)
            }
            Err(e) => {
                problems.push(format!("{id}: {e}"));
                (HashMap::new(), 0, None)
            }
        };

        let basename = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let folder = id.split('/').next().unwrap_or("").to_string();
        let folder = if folder.ends_with(".md") {
            String::new()
        } else {
            folder
        };

        notes.push(Note {
            id,
            path,
            basename,
            folder,
            fields,
            body_start,
            raw_frontmatter: raw,
            text,
        });
    }

    Ok((notes, problems))
}

/// Resolve the vault root: `--root` wins, otherwise the overlay declares it.
///
/// The path is a personal fact and never lives in this repo. `knowledge.toml`
/// is read with a line scan rather than a TOML parser: the file holds one key
/// this tool needs, and a dependency to read one key is a dependency to audit.
pub fn resolve_root(explicit: Option<String>) -> Result<MarkdownRoot, String> {
    if let Some(p) = explicit {
        return MarkdownRoot::declare(sjel_config::expand_tilde(&p))
            .map_err(|e| format!("--root: {e}"));
    }

    let cfg = sjel_config::overlay_config("knowledge.toml").ok_or_else(|| {
        "no vault root: pass --root, or declare vault_root in the overlay's config/knowledge.toml"
            .to_string()
    })?;
    let text = std::fs::read_to_string(&cfg)
        .map_err(|e| format!("reading {}: {e}", cfg.to_string_lossy()))?;

    let value = text
        .lines()
        .filter_map(|l| l.split_once('='))
        .find(|(k, _)| k.trim() == "vault_root")
        .map(|(_, v)| v.trim().trim_matches('"').to_string())
        .ok_or_else(|| format!("{} declares no vault_root", cfg.to_string_lossy()))?;

    MarkdownRoot::declare(sjel_config::expand_tilde(&value)).map_err(|e| format!("vault_root: {e}"))
}
