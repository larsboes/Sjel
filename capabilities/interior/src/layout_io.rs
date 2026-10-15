//! Writing a layout back without losing its reasoning.
//!
//! Layouts are files, not rows: their header carries the argument for a decision (why a table was
//! rejected, say), and the overlay is a git repository, so every change has a diff. The one
//! requirement on this writer is that the prose survives.
//!
//! No `toml_edit`: measured 2026-08-31, no layout has a comment after the first `[[item]]`, and the
//! file knows five keys. So the file is cut at the first `[[item]]`, the header stays character for
//! character, and the entries are rewritten. `check_header_suffices` refuses a file where that would
//! lose a comment.

use crate::model::{Layout, Model, ModelError, PlacedItem};
use std::path::Path;

/// The header of a layout file: everything before the first `[[item]]`.
fn header(text: &str) -> &str {
    match text.find("[[item]]") {
        Some(i) => &text[..i],
        None => text,
    }
}

/// A comment after the first `[[item]]` would be gone after writing, so the writer stops instead
/// of truncating silently.
fn check_header_suffices(text: &str, path: &Path) -> Result<(), ModelError> {
    let Some(i) = text.find("[[item]]") else {
        return Ok(());
    };
    for line in text[i..].lines() {
        let z = line.trim();
        if z.starts_with('#') {
            return Err(ModelError::Missing(format!(
                "{}: there is a comment after the first [[item]] ({z:?}). \
                 This writer keeps only the header and would lose it; \
                 move it up or edit the file by hand",
                path.display()
            )));
        }
    }
    Ok(())
}

/// The entries as TOML, in their order.
fn entries(items: &[PlacedItem]) -> String {
    let mut s = String::new();
    for it in items {
        s.push_str("[[item]]\n");
        s.push_str(&format!("ref = \"{}\"\n", it.reference));
        s.push_str(&format!("x = {}\ny = {}\nrot = {}\n", it.x, it.y, it.rot));
        if let Some(sz) = it.size {
            s.push_str(&format!("size = [{}, {}]\n", sz[0], sz[1]));
        }
        if let Some(k) = &it.kind {
            s.push_str(&format!("kind = \"{k}\"\n"));
        }
        s.push('\n');
    }
    s
}

/// Write new positions into an existing layout. The header stays.
pub fn update(model: &Model, id: &str, items: &[PlacedItem]) -> Result<(), ModelError> {
    let path = model.layouts_dir().join(format!("{id}.toml"));
    let text = std::fs::read_to_string(&path).map_err(|source| ModelError::Read {
        path: path.clone(),
        source,
    })?;
    check_header_suffices(&text, &path)?;
    let content = format!("{}{}", header(&text), entries(items));
    std::fs::write(&path, content).map_err(|source| ModelError::Read { path, source })
}

/// A layout name that may be a file name. This decides what a caller can write into the overlay
/// over HTTP, so a dot or a separator is a way out of the directory.
pub fn check_id(id: &str) -> Result<(), ModelError> {
    if id.is_empty()
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(ModelError::Missing(format!(
            "`{id}` is no layout name; allowed are letters, digits, - and _"
        )));
    }
    Ok(())
}

/// The note as a comment block, line by line: a layout's origin is a paragraph.
fn as_comment(note: &str) -> String {
    note.lines()
        .map(|z| {
            let z = z.trim_end();
            if z.is_empty() {
                "#\n".to_string()
            } else {
                format!("# {z}\n")
            }
        })
        .collect()
}

/// Create a new layout; the header says where it came from, in `note`. It claims nothing else: a
/// file that misstates its own origin is worse than one without a header.
pub fn create(model: &Model, id: &str, layout: &Layout, note: &str) -> Result<(), ModelError> {
    check_id(id)?;
    let path = model.layouts_dir().join(format!("{id}.toml"));
    if path.exists() {
        return Err(ModelError::Missing(format!(
            "`{id}` exists already; an existing layout is not overwritten"
        )));
    }
    let header = format!(
        "{}#\n\
         # This header is the place for the WHY: what this arrangement stands for and what it was\n\
         # decided against. It survives every later move; the entries below are rewritten,\n\
         # this paragraph is not.\n\n\
         name = \"{}\"\n\n",
        as_comment(note),
        layout.name.replace('"', "'")
    );
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|source| ModelError::Read {
            path: dir.to_path_buf(),
            source,
        })?;
    }
    std::fs::write(&path, format!("{header}{}", entries(&layout.items)))
        .map_err(|source| ModelError::Read { path, source })
}

/// Where an archived layout goes: a subdirectory, not a bin. The name is a directory in the
/// overlay, so it stays as existing machines have it.
pub const ARCHIVE: &str = "archiv";

/// Take a layout out of the list without losing it. Its header can be the only place an argument
/// for rejecting a piece is written, so nothing is deleted: `archiv/` has no `.toml` ending and
/// drops out of `Model::layout_names`, and the file stays in the overlay's git.
pub fn archive_layout(model: &Model, id: &str) -> Result<String, ModelError> {
    check_id(id)?;
    let from = model.layouts_dir().join(format!("{id}.toml"));
    if !from.exists() {
        return Err(ModelError::Missing(format!("no layout `{id}`")));
    }
    let archive = model.layouts_dir().join(ARCHIVE);
    std::fs::create_dir_all(&archive).map_err(|source| ModelError::Read {
        path: archive.clone(),
        source,
    })?;
    let to = archive.join(format!("{id}.toml"));
    if to.exists() {
        return Err(ModelError::Missing(format!(
            "the archive already holds a `{id}`; two versions under one name would lose a reason"
        )));
    }
    std::fs::rename(&from, &to).map_err(|source| ModelError::Read { path: from, source })?;
    // Relative to the layouts dir: the absolute path points into the private overlay.
    Ok(format!("{ARCHIVE}/{id}.toml"))
}

/// Today as `YYYY-MM-DD`, from the clock.
pub fn today() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let (j, m, t) = date_from_days(seconds.div_euclid(86_400));
    format!("{j:04}-{m:02}-{t:02}")
}

/// Days since 1970-01-01 as a calendar date, after Howard Hinnant's `civil_from_days`; one date in
/// a comment does not justify a calendar dependency.
fn date_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let tag = doy - (153 * mp + 2) / 5 + 1;
    // Hinnant's year starts in March, so the leap day falls at its end.
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, tag)
}

#[cfg(test)]
mod tests {
    use super::{as_comment, check_header_suffices, check_id, date_from_days, entries, header};
    use crate::model::PlacedItem;
    use std::path::Path;

    const FILE: &str = "# Variant K: bed across the north wall.\n\
                         #\n\
                         # Three paragraphs of reasoning no serialiser knows.\n\n\
                         name = \"K\"\n\n\
                         [[item]]\nref = \"bett\"\nx = 0\ny = 0\nrot = 0\n\n";

    #[test]
    fn the_header_survives_character_for_character() {
        let k = header(FILE);
        assert!(k.contains("Three paragraphs of reasoning"));
        assert!(k.contains("name = \"K\""));
        assert!(!k.contains("[[item]]"));
    }

    #[test]
    fn the_entries_are_rewritten() {
        let items = vec![PlacedItem {
            reference: "bett".into(),
            x: 120,
            y: 40,
            rot: 90,
            size: None,
            kind: None,
        }];
        let s = entries(&items);
        assert!(s.contains("ref = \"bett\""));
        assert!(s.contains("x = 120"));
        assert!(s.contains("rot = 90"));
        assert!(!s.contains("size"), "no size when none is set");
    }

    /// A comment between entries stops the write instead of being lost.
    #[test]
    fn a_comment_after_the_entries_prevents_writing() {
        let with =
            format!("{FILE}# a late thought\n[[item]]\nref = \"x\"\nx = 1\ny = 1\nrot = 0\n");
        let e = check_header_suffices(&with, Path::new("k.toml"))
            .expect_err("the comment would be lost");
        assert!(e.to_string().contains("late thought"), "{e}");
    }

    #[test]
    fn without_trailing_comments_writing_is_allowed() {
        assert!(check_header_suffices(FILE, Path::new("k.toml")).is_ok());
    }

    /// A paragraph stays a paragraph, and the empty line stays a line.
    #[test]
    fn a_multi_line_note_becomes_a_comment_block() {
        let k = as_comment("First line.\n\nThird line.");
        assert_eq!(k, "# First line.\n#\n# Third line.\n");
    }

    /// The name becomes a file name; what passes here may write into the overlay.
    #[test]
    fn a_layout_name_cannot_point_out_of_the_dir() {
        assert!(check_id("p-gerechnet_2").is_ok());
        for bad in ["", "../heim", "a/b", "a.toml", "a b"] {
            assert!(check_id(bad).is_err(), "`{bad}` must not be a file");
        }
    }

    /// Against two days checkable by hand, and against the leap day.
    #[test]
    fn days_since_the_epoch_become_the_calendar_date() {
        assert_eq!(date_from_days(0), (1970, 1, 1));
        assert_eq!(date_from_days(-1), (1969, 12, 31));
        assert_eq!(date_from_days(11_016), (2000, 2, 29), "Schalttag");
        assert_eq!(date_from_days(20_696), (2026, 8, 31));
    }
}
