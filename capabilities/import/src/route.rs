//! Which capability owns a file. Best guess from the path and the extension,
//! plus the header line for a CSV: a wrong guess is fixed later, an unsorted
//! file is never imported (decision 2026-10-23, `README.md`).

use std::path::Path;

/// Where one file goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    Documents,
    Media,
    Finance,
    Notes,
    /// Not personal data, or not readable as a file. The reason is shown to the operator.
    Skip(&'static str),
}

impl Route {
    /// Stable name stored in the ledger.
    pub fn name(self) -> &'static str {
        match self {
            Self::Documents => "documents",
            Self::Media => "media",
            Self::Finance => "finance",
            Self::Notes => "notes",
            Self::Skip(_) => "skip",
        }
    }
}

/// Directories that hold software, caches or agent tooling, never personal files.
const SOFTWARE_DIRS: &[&str] = &[
    "node_modules",
    "__pycache__",
    "site-packages",
    "venv",
    "target",
    "build",
    "dist",
    "DerivedData",
    "Pods",
    "cache",
    "Caches",
    "plugins",
    "skills",
    "agents",
    "commands",
];
/// Directory suffixes of macOS bundles; their content belongs to the bundle.
const BUNDLE_SUFFIXES: &[&str] = &[
    ".app",
    ".bundle",
    ".framework",
    ".xcodeproj",
    ".xcworkspace",
    ".xcassets",
    ".lproj",
    ".photoslibrary",
];
const DOCUMENTS: &[&str] = &[
    "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "odt", "ods", "odp", "rtf", "epub",
    "pages", "numbers", "keynote",
];
const MEDIA: &[&str] = &[
    "jpg", "jpeg", "png", "heic", "heif", "avif", "gif", "tif", "tiff", "webp", "dng", "cr2",
    "nef", "arw", "mov", "mp4", "m4v", "avi", "m4a", "mp3", "wav", "aac", "flac", "aif", "aiff",
    "ogg", "opus",
];
/// `note` and `goodnotes` are Notability and GoodNotes handwriting archives.
const NOTES: &[&str] = &["md", "markdown", "txt", "note", "goodnotes"];
const SOFTWARE: &[&str] = &[
    "py", "pyc", "swift", "ts", "tsx", "js", "mjs", "cjs", "rs", "go", "java", "kt", "c", "h",
    "cpp", "m", "sh", "zsh", "json", "jsonl", "plist", "strings", "yaml", "yml", "toml", "lock",
    "css", "scss", "html", "svelte", "vue", "xml", "sql", "log", "dae", "obj", "map",
];
/// Lowercase header words of a bank or card statement export.
const BANK_HEADER: &[&str] = &[
    "iban",
    "buchungstag",
    "valuta",
    "verwendungszweck",
    "auftraggeber",
    "booking date",
    "value date",
    "betrag",
    "amount",
];

/// Why a folder at `relative` holds no personal files, if it does not.
pub fn skip_folder(relative: &Path) -> Option<&'static str> {
    for dir in relative.iter() {
        let Some(dir) = dir.to_str() else {
            return Some("name is not UTF-8");
        };
        if dir.starts_with('.') {
            return Some("inside a hidden folder");
        }
        if SOFTWARE_DIRS.contains(&dir) {
            return Some("inside a software or tooling folder");
        }
        if BUNDLE_SUFFIXES.iter().any(|suffix| dir.ends_with(suffix)) {
            return Some("inside an app or project bundle");
        }
    }
    None
}

/// Sort one file by its path relative to the import root. `csv_header` is the
/// file's first line, read only for a `.csv`.
pub fn route(relative: &Path, csv_header: Option<&str>) -> Route {
    let Some(name) = relative.file_name().and_then(|n| n.to_str()) else {
        return Route::Skip("name is not UTF-8");
    };
    if let Some(reason) = relative.parent().and_then(skip_folder) {
        return Route::Skip(reason);
    }
    if name.starts_with('.') {
        return Route::Skip("hidden file");
    }
    let extension = name
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .unwrap_or_default();
    let extension = extension.as_str();
    if DOCUMENTS.contains(&extension) {
        Route::Documents
    } else if MEDIA.contains(&extension) {
        Route::Media
    } else if NOTES.contains(&extension) {
        Route::Notes
    } else if extension == "csv" {
        let header = csv_header.unwrap_or_default().to_lowercase();
        if BANK_HEADER
            .iter()
            .filter(|word| header.contains(*word))
            .count()
            >= 2
        {
            Route::Finance
        } else {
            Route::Skip("data export, not a bank statement")
        }
    } else if SOFTWARE.contains(&extension) {
        Route::Skip("code or configuration")
    } else {
        Route::Skip("unknown file type")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(path: &str) -> Route {
        route(Path::new(path), None)
    }

    #[test]
    fn personal_files_go_to_their_owner() {
        assert_eq!(at("Steuer/2025/Bescheid.PDF"), Route::Documents);
        assert_eq!(at("Urlaub/IMG_0001.HEIC"), Route::Media);
        assert_eq!(at("Recordings/voice.m4a"), Route::Media);
        assert_eq!(at("Notes/idea.md"), Route::Notes);
        assert_eq!(at("Finanzen/Steuer.note"), Route::Notes);
    }

    #[test]
    fn software_and_tooling_are_skipped_by_folder_before_type() {
        assert_eq!(
            at("proj/node_modules/x/README.md"),
            Route::Skip("inside a software or tooling folder")
        );
        assert_eq!(
            at("state/skills/Tldraw/SketchDiagram.md"),
            Route::Skip("inside a software or tooling folder")
        );
        assert_eq!(
            at("Thing.app/Contents/Resources/icon.png"),
            Route::Skip("inside an app or project bundle")
        );
        assert_eq!(at("repo/.git/HEAD"), Route::Skip("inside a hidden folder"));
        assert_eq!(at("src/main.swift"), Route::Skip("code or configuration"));
        assert_eq!(at(".DS_Store"), Route::Skip("hidden file"));
    }

    #[test]
    fn a_csv_is_finance_only_with_a_bank_header() {
        let bank = "Buchungstag;Valuta;Verwendungszweck;Betrag";
        assert_eq!(route(Path::new("umsaetze.csv"), Some(bank)), Route::Finance);
        assert_eq!(
            route(Path::new("export.csv"), Some("id,name,email")),
            Route::Skip("data export, not a bank statement")
        );
    }
}
