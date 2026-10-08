//! A whole file the machine owns, in a folder the machine may not be alone in.
//!
//! `region.rs` is the other half of the same bridge and the preferred one: when a
//! human note about the subject already exists, the machine writes a marked region
//! into it and the derived figures land where the human already writes. This module
//! covers the case that rule leaves open — *there is no human note* — which PRD Q31
//! (2026-08-23) named pattern B and gave one home: `Resources/Sjel/`.
//!
//! Two subjects genuinely need it, and Q31 names both: a trip export, and a monthly
//! figure whose review note was never written. Neither has a note to write into, and
//! neither earns one automatically.
//!
//! ## Why the whole file, and not a region in it
//!
//! A region guards bytes it does not own. A projection owns every byte, so there is
//! nothing to guard and a region would only add markers around the entire document.
//! The declaration that matters is instead the header this module writes into the
//! file, which states the rule to the one reader who can break it.
//!
//! ## What happens to an edit
//!
//! It is lost — and the file says so before it can happen. There is no hash and no
//! conflict outcome here, deliberately: the marker in `region.rs` exists to protect
//! a human's prose in a file they own, and a projection is not that file. What is
//! guarded instead is the *path*. A file at the projection's path whose header does
//! not name *this* caller as its owner is somebody else's file, and
//! [`ProjectionOutcome::NotOurs`] refuses it rather than overwriting it. That covers
//! the one real accident — a human creating a note where a projection goes — without
//! pretending a safety copy can be merged.
//!
//! Q31's promotion path runs through that refusal: when a human starts writing about
//! the subject, their note takes the path, the projection refuses, and the capability
//! moves to a marked region in the human's note.
//!
//! ## Why the owner, and not just "is there a header"
//!
//! Q49's Sources bridge (2026-08-27) sends the first projection into `Resources/Sources/`,
//! a folder humans write in and a second capability may later write to as well. Both
//! guards therefore compare the header's `owner=` against the caller's, not merely the
//! presence of a header: otherwise comms' sweep deletes finance's projection because it
//! recognised a header, and the two capabilities overwrite each other in the folder they
//! share. A per-capability folder hid that; a shared one does not.
//!
//! ## What it deliberately is not
//!
//! Not an exporter. What a plan, a month or a subscription *looks like* in markdown
//! belongs to the capability that owns the data. This module owns placement,
//! containment, the header, and not writing a file whose bytes already match — so the
//! vault's git history stays free of no-op commits.

use std::path::{Path, PathBuf};

use crate::{check_pattern, frontmatter_spanned, MarkdownRoot, RegionSpec, RootError};

/// The header's opening token. Matched to decide whether a file at a projection's
/// path is one of ours, so it is stable forever: changing it makes every projection
/// already in the vault look foreign and the next export refuse all of them.
const HEADER: &str = "<!-- axon:projection";

/// What a write did, or refused to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectionOutcome {
    /// Nothing was at the path, so the file was written.
    Created,
    /// A projection was there and its bytes differ from the new ones.
    Updated,
    /// A projection was there and it already holds exactly these bytes. Nothing was
    /// opened for writing, so a scheduled export does not produce a vault commit per
    /// run.
    Unchanged,
    /// A file is at the path and this caller does not own it. Nothing was written.
    ///
    /// Two cases, and one answer for both: the file carries no projection header at
    /// all — Q31's promotion signal, a human wrote about the subject — or it carries
    /// another capability's header. Neither is an error, and in neither is
    /// overwriting allowed.
    NotOurs,
}

/// The `owner=` the header at this path declares, or `None` when there is no file
/// there and when the file is not a projection at all.
///
/// Ownership used to be one bit — "does the file contain the header" — and that was
/// enough while every projection had a folder to itself (`Resources/Sjel/Trips/`,
/// `Resources/Sjel/Subscriptions/`). PRD Q49's Sources bridge is the first projection
/// into a folder that two owners can share with each other and with a human, and there
/// one bit is wrong in a specific direction: a sweep would delete another capability's
/// projection because it recognised *a* header. The owner is already written into the
/// header, so it is read back out rather than tracked somewhere new.
fn declared_owner(document: &str) -> Option<&str> {
    let after = document.split_once(HEADER)?.1;
    let after = after.trim_start();
    let value = after.strip_prefix("owner=")?;
    let end = value
        .find(|c: char| c.is_whitespace())
        .unwrap_or(value.len());
    Some(&value[..end])
}

/// The line that tells the one reader who can break the contract what the file is.
///
/// Carries the owner and version for the same reason the region marker does: a later
/// generator can recognise output it no longer knows how to produce.
pub fn header(spec: &RegionSpec) -> String {
    format!(
        "{HEADER} owner={} v={} -->\n\
         <!-- Axon generates this file and overwrites it whole. An edit here is lost on the next export. \
         To write about this subject, make your own note; this projection then refuses the path (PRD Q31). -->\n",
        spec.owner, spec.version
    )
}

/// Place the header inside a rendered document.
///
/// After the frontmatter, never before it: Obsidian reads frontmatter only when the
/// opening `---` is the first line of the file, so a header above it would silently
/// turn every declared key into prose. A document whose frontmatter never closes gets
/// the header at the top, which leaves the broken fence visible rather than repairing
/// a caller's rendering bug behind its back.
pub fn document(spec: &RegionSpec, rendered: &str) -> String {
    let at = frontmatter_spanned(rendered)
        .map(|f| f.body_start)
        .unwrap_or(0);
    let mut out = String::with_capacity(rendered.len() + 256);
    out.push_str(&rendered[..at]);
    out.push_str(&header(spec));
    out.push_str(&rendered[at..]);
    out
}

impl MarkdownRoot {
    /// Write one projection, relative to the root.
    ///
    /// Missing parent directories are created — `Resources/Sjel/Trips/` does not exist
    /// until the first export, and failing on that would make the first run of every
    /// projection a manual `mkdir`.
    pub fn write_projection(
        &self,
        relative: &str,
        spec: &RegionSpec,
        rendered: &str,
    ) -> Result<ProjectionOutcome, RootError> {
        let target = self.projection_path(relative)?;
        let document = document(spec, rendered);

        match std::fs::read_to_string(&target) {
            Ok(existing) if declared_owner(&existing) != Some(spec.owner.as_str()) => {
                Ok(ProjectionOutcome::NotOurs)
            }
            Ok(existing) if existing == document => Ok(ProjectionOutcome::Unchanged),
            Ok(_) => {
                write_file(&target, &document)?;
                Ok(ProjectionOutcome::Updated)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| RootError::Unwritable {
                        path: parent.to_path_buf(),
                        detail: e.to_string(),
                    })?;
                }
                write_file(&target, &document)?;
                Ok(ProjectionOutcome::Created)
            }
            Err(e) => Err(RootError::Unreadable {
                path: target,
                detail: e.to_string(),
            }),
        }
    }

    /// Delete one projection this `spec` owns. `false` when there was nothing to
    /// delete.
    ///
    /// A file that is not ours is left alone and reported as `false`, for the reason
    /// [`ProjectionOutcome::NotOurs`] gives: the source row going away is not authority
    /// to delete a note a human wrote at that path. `spec` rather than a bare path
    /// because a sweep runs over a whole folder, and in a shared folder the file it is
    /// about to delete may belong to another capability that is perfectly up to date.
    pub fn remove_projection(&self, relative: &str, spec: &RegionSpec) -> Result<bool, RootError> {
        let target = self.projection_path(relative)?;
        match std::fs::read_to_string(&target) {
            Ok(existing) if declared_owner(&existing) != Some(spec.owner.as_str()) => Ok(false),
            Ok(_) => {
                std::fs::remove_file(&target).map_err(|e| RootError::Unwritable {
                    path: target.clone(),
                    detail: e.to_string(),
                })?;
                Ok(true)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(RootError::Unreadable {
                path: target,
                detail: e.to_string(),
            }),
        }
    }

    /// The absolute path a projection writes to, proven inside the root.
    ///
    /// `contained()` cannot be used: it canonicalises, and the file does not exist
    /// yet. So the deepest ancestor that *does* exist is canonicalised and checked
    /// instead. That is the same guarantee — `check_pattern` has already refused `..`
    /// and an absolute path, and every directory below the checked ancestor is one
    /// this module creates, so none of them can be a symlink out.
    pub fn projection_path(&self, relative: &str) -> Result<PathBuf, RootError> {
        let checked = check_pattern(relative)?;
        if !checked.ends_with(".md") {
            return Err(RootError::NotMarkdown(checked.to_string()));
        }
        let target = self.path().join(checked);

        let mut probe: &Path = &target;
        while let Some(parent) = probe.parent() {
            if parent.exists() {
                let resolved =
                    std::fs::canonicalize(parent).map_err(|e| RootError::Unreadable {
                        path: parent.to_path_buf(),
                        detail: e.to_string(),
                    })?;
                if !resolved.starts_with(self.path()) {
                    return Err(RootError::Escapes {
                        path: parent.to_path_buf(),
                        root: self.path().to_path_buf(),
                    });
                }
                return Ok(target);
            }
            probe = parent;
        }
        // Unreachable while the root itself exists, which `declare` proved.
        Ok(target)
    }
}

/// A title reduced to something a file system and Obsidian both accept.
///
/// Here rather than in each capability because every projection has the same problem:
/// a row's human-facing name becomes a file name, and the row does not know what a
/// file name may contain. `trips` and `finance` had the same twenty lines.
///
/// The characters replaced are the union of what macOS and Windows refuse in a name and
/// what Obsidian refuses in a note title (`#^[]|`), so a vault synced to a second
/// machine does not lose a file there. A leading dot is stripped as well: it hides the
/// file on every Unix host and makes [`MarkdownRoot::markdown_files_recursive`] skip
/// it, which would leave a safety copy that exists and is never seen again.
///
/// `fallback` is used when nothing survives, because a row with an unusable title still
/// needs a file. Pass something that identifies the row.
pub fn file_stem(title: &str, fallback: &str) -> String {
    let mut cleaned = String::with_capacity(title.len());
    let mut last_was_space = false;
    for ch in title.chars() {
        let ch = match ch {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '#' | '^' | '[' | ']' => '-',
            c if c.is_control() => ' ',
            c => c,
        };
        if ch.is_whitespace() {
            if !last_was_space && !cleaned.is_empty() {
                cleaned.push(' ');
            }
            last_was_space = true;
        } else {
            cleaned.push(ch);
            last_was_space = false;
        }
    }
    let cleaned = cleaned.trim().trim_start_matches('.').trim();
    let truncated: String = cleaned.chars().take(80).collect();
    let truncated = truncated.trim_end().to_string();
    if truncated.is_empty() {
        fallback.to_string()
    } else {
        truncated
    }
}

fn write_file(path: &Path, body: &str) -> Result<(), RootError> {
    std::fs::write(path, body).map_err(|e| RootError::Unwritable {
        path: path.to_path_buf(),
        detail: e.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root() -> (PathBuf, MarkdownRoot) {
        // The counter is what makes this unique, and the pid and clock are what keep it unique
        // ACROSS runs. Rust runs a crate's tests as threads of one process, so the pid is the
        // same for all of them, and `SystemTime` on macOS does not tick per nanosecond -- two
        // tests entering here together got the same path, and the first to reach its closing
        // `remove_dir_all` deleted the other's root mid-test. It failed about one
        // `cargo test --workspace` in two and passed every time the crate ran alone, which is
        // the shape of a flake that gets re-run rather than read.
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "axon-projection-{}-{}-{:?}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let root = MarkdownRoot::declare(&dir).unwrap();
        (dir, root)
    }

    fn spec() -> RegionSpec {
        RegionSpec::new("trips", 1)
    }

    #[test]
    fn the_header_sits_below_the_frontmatter_so_obsidian_still_reads_it() {
        let doc = document(&spec(), "---\naxon_trip_id: p1\n---\n\n# Berlin\n");
        assert!(
            doc.starts_with("---\naxon_trip_id: p1\n---\n"),
            "frontmatter must stay the first thing in the file: {doc}"
        );
        let header_at = doc.find(HEADER).expect("header present");
        assert!(header_at > doc.find("axon_trip_id").unwrap());
        assert!(header_at < doc.find("# Berlin").unwrap());
    }

    #[test]
    fn a_document_without_frontmatter_gets_the_header_first() {
        let doc = document(&spec(), "# Berlin\n");
        assert!(doc.starts_with(HEADER), "{doc}");
    }

    #[test]
    fn creating_then_rewriting_the_same_bytes_reports_unchanged() {
        let (dir, root) = temp_root();
        let body = "---\nid: p1\n---\n\n# One\n";
        assert_eq!(
            root.write_projection("Resources/Sjel/Trips/One.md", &spec(), body)
                .unwrap(),
            ProjectionOutcome::Created
        );
        assert_eq!(
            root.write_projection("Resources/Sjel/Trips/One.md", &spec(), body)
                .unwrap(),
            ProjectionOutcome::Unchanged
        );
        assert_eq!(
            root.write_projection("Resources/Sjel/Trips/One.md", &spec(), "# Two\n")
                .unwrap(),
            ProjectionOutcome::Updated
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_human_note_at_the_path_is_refused_rather_than_overwritten() {
        let (dir, root) = temp_root();
        std::fs::create_dir_all(dir.join("Resources/Sjel/Trips")).unwrap();
        let human = dir.join("Resources/Sjel/Trips/One.md");
        std::fs::write(&human, "# My trip, in my words\n").unwrap();

        assert_eq!(
            root.write_projection("Resources/Sjel/Trips/One.md", &spec(), "# generated\n")
                .unwrap(),
            ProjectionOutcome::NotOurs
        );
        assert_eq!(
            std::fs::read_to_string(&human).unwrap(),
            "# My trip, in my words\n"
        );
        assert!(!root
            .remove_projection("Resources/Sjel/Trips/One.md", &spec())
            .unwrap());
        assert!(human.exists(), "a refused path is never deleted either");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn removing_a_projection_reports_whether_there_was_one() {
        let (dir, root) = temp_root();
        assert!(!root
            .remove_projection("Resources/Sjel/Trips/Gone.md", &spec())
            .unwrap());
        root.write_projection("Resources/Sjel/Trips/Gone.md", &spec(), "# x\n")
            .unwrap();
        assert!(root
            .remove_projection("Resources/Sjel/Trips/Gone.md", &spec())
            .unwrap());
        assert!(!root
            .remove_projection("Resources/Sjel/Trips/Gone.md", &spec())
            .unwrap());
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// Q49 puts two capabilities in `Resources/Sources/`. Before the owner check, the
    /// second one to run overwrote the first's file and its sweep then deleted it,
    /// because both recognised the header as "ours".
    #[test]
    fn another_capabilitys_projection_is_neither_overwritten_nor_swept() {
        let (dir, root) = temp_root();
        let finance = RegionSpec::new("finance", 1);
        root.write_projection("Resources/Sources/Shared.md", &finance, "# theirs\n")
            .unwrap();

        assert_eq!(
            root.write_projection("Resources/Sources/Shared.md", &spec(), "# ours\n")
                .unwrap(),
            ProjectionOutcome::NotOurs,
            "a header is not a licence; the owner in it is"
        );
        assert!(
            !root
                .remove_projection("Resources/Sources/Shared.md", &spec())
                .unwrap(),
            "a sweep must not delete a file another owner keeps up to date"
        );
        assert!(dir.join("Resources/Sources/Shared.md").exists());
        assert!(root
            .remove_projection("Resources/Sources/Shared.md", &finance)
            .unwrap());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_owner_is_read_back_out_of_the_header_that_records_it() {
        assert_eq!(
            declared_owner(&document(&spec(), "---\nid: p1\n---\n\n# One\n")),
            Some("trips")
        );
        assert_eq!(declared_owner("# a human note\n"), None);
        assert_eq!(
            declared_owner("<!-- axon:projection v=1 -->\n"),
            None,
            "a header that names no owner claims nothing"
        );
    }

    #[test]
    fn a_path_that_leaves_the_root_is_refused_before_anything_is_written() {
        let (dir, root) = temp_root();
        assert!(matches!(
            root.write_projection("../escape.md", &spec(), "x"),
            Err(RootError::PatternEscapes(_))
        ));
        assert!(matches!(
            root.write_projection("Resources/Sjel/Trips/One.txt", &spec(), "x"),
            Err(RootError::NotMarkdown(_))
        ));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_symlinked_parent_pointing_out_of_the_root_is_refused() {
        let (dir, root) = temp_root();
        let outside = dir
            .parent()
            .unwrap()
            .join(format!("axon-projection-outside-{}", std::process::id()));
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::create_dir_all(dir.join("Resources")).unwrap();
        std::os::unix::fs::symlink(&outside, dir.join("Resources/Sjel")).unwrap();

        assert!(matches!(
            root.write_projection("Resources/Sjel/One.md", &spec(), "x"),
            Err(RootError::Escapes { .. })
        ));
        std::fs::remove_dir_all(&dir).unwrap();
        std::fs::remove_dir_all(&outside).unwrap();
    }
}
