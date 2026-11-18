//! One Markdown file per saved link, in one folder per site group, for the projects tree.
//!
//! The files take over from the Links store (`docs/plans/product.md`, "Open boundary work").
//! The projects index names a record after its file, so the file name is the link's title. The
//! body repeats the address because the note editor does not show frontmatter.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::Link;

/// Folder under the projects root. `Atlas/` is a domain folder (`crates/projects/src/record.rs`).
pub const FOLDER: &str = "Atlas/Links";
const MAX_NAME_CHARS: usize = 100;

/// One file to write: its path relative to the export folder and its bytes.
#[derive(Debug, PartialEq, Eq)]
pub struct ExportFile {
    pub path: String,
    pub content: String,
}

/// Lay out every link as `<group>/<title>.md`. Two links with the same name in one folder get
/// ` (2)`, ` (3)` and so on, in list order.
pub fn plan(links: &[Link]) -> Vec<ExportFile> {
    let mut taken = HashSet::new();
    links
        .iter()
        .map(|link| {
            let folder = file_name(&link.group, &link.host);
            let title = match (link.title.trim(), link.tenant.as_deref()) {
                ("", Some(tenant)) => tenant.to_owned(),
                ("", None) => link.host.clone(),
                (title, _) => title.to_owned(),
            };
            let base = file_name(&title, &link.host);
            let mut name = base.clone();
            let mut n = 2;
            while !taken.insert(format!("{folder}/{name}").to_lowercase()) {
                name = format!("{base} ({n})");
                n += 1;
            }
            let mut content = format!("---\nurl: {}\nclass: c2\n", link.url);
            if link.pinned {
                content.push_str("pinned: true\n");
            }
            content.push_str(&format!("---\n<{}>\n", link.url));
            ExportFile {
                path: format!("{folder}/{name}.md"),
                content,
            }
        })
        .collect()
}

/// Write the planned files under `root`. Refuses before writing anything when a target exists.
///
/// # Errors
/// A target file exists, or a directory or file cannot be written.
pub fn write(root: &Path, files: &[ExportFile]) -> Result<usize, String> {
    let targets: Vec<PathBuf> = files.iter().map(|file| root.join(&file.path)).collect();
    if let Some(existing) = targets.iter().find(|path| path.exists()) {
        return Err(format!(
            "{} exists; nothing was written",
            existing.display()
        ));
    }
    for (file, target) in files.iter().zip(&targets) {
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        std::fs::write(target, &file.content).map_err(|error| error.to_string())?;
    }
    Ok(files.len())
}

/// A file or folder name that every platform and the projects index accept.
fn file_name(raw: &str, fallback: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '#' | '^' | '[' | ']' => '-',
            c if c.is_control() => ' ',
            c => c,
        })
        .take(MAX_NAME_CHARS)
        .collect();
    let trimmed = cleaned.trim().trim_matches('.').trim();
    if trimmed.is_empty() {
        file_name(fallback, "link")
    } else {
        trimmed.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(title: &str, group: &str, url: &str, pinned: bool) -> Link {
        Link {
            id: url.into(),
            url: url.into(),
            title: title.into(),
            host: "example.com".into(),
            tenant: None,
            group: group.into(),
            group_override: None,
            pinned,
            opens: 0,
            last_opened_at: None,
            created_at: String::new(),
        }
    }

    #[test]
    fn lays_out_one_file_per_link_in_its_group_folder() {
        let files = plan(&[
            link("Rust", "Reddit", "https://www.reddit.com/r/rust/", true),
            link("CI: main", "Work/Tools", "https://ci.example.com/", false),
        ]);
        assert_eq!(files[0].path, "Reddit/Rust.md");
        assert_eq!(
            files[0].content,
            "---\nurl: https://www.reddit.com/r/rust/\nclass: c2\npinned: true\n---\n<https://www.reddit.com/r/rust/>\n"
        );
        assert_eq!(files[1].path, "Work-Tools/CI- main.md");
    }

    #[test]
    fn numbers_duplicate_names_and_falls_back_for_empty_titles() {
        let files = plan(&[
            link("Home", "Site", "https://a.example.com/", false),
            link("home", "Site", "https://b.example.com/", false),
            link("  ", "Site", "https://c.example.com/", false),
        ]);
        let paths: Vec<_> = files.iter().map(|file| file.path.as_str()).collect();
        assert_eq!(
            paths,
            ["Site/Home.md", "Site/home (2).md", "Site/example.com.md"]
        );
    }

    #[test]
    fn the_index_reads_the_url_back_from_the_frontmatter() {
        let file = &plan(&[link("Rust", "Reddit", "https://x.example/a?b=c", false)])[0];
        let fields = markdown_root::frontmatter_spanned(&file.content)
            .unwrap()
            .fields;
        assert_eq!(fields["url"], "https://x.example/a?b=c");
        assert_eq!(fields["class"], "c2");
    }

    #[test]
    fn refuses_to_overwrite_and_writes_nothing() {
        let dir = std::env::temp_dir().join(format!("links-export-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let files = plan(&[
            link("A", "G", "https://a.example/", false),
            link("B", "G", "https://b.example/", false),
        ]);
        std::fs::create_dir_all(dir.join("G")).unwrap();
        std::fs::write(dir.join("G/B.md"), "mine").unwrap();
        assert!(write(&dir, &files).is_err());
        assert!(!dir.join("G/A.md").exists());
        assert_eq!(std::fs::read_to_string(dir.join("G/B.md")).unwrap(), "mine");
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(write(&dir, &files).unwrap(), 2);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
