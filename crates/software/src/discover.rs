//! Finds the apps on this Mac and what each holds on disk.
//!
//! Platform tools rather than crates: `plutil` reads an Info.plist whether it is XML or binary,
//! `mdls` answers Spotlight's `kMDItemLastUsedDate`, and `du -sk` counts allocated blocks the
//! way `sjel-storage` does for its policy paths (`tools/storage/src/measure.rs`, `size_of`).
//!
//! The last-used date is advice, never a rule: an app started from a shell (`xcodebuild`,
//! a CLI inside the bundle) never updates it, so a missing date does not mean unused.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;

/// One thing a scan found. For an app, `data_paths` are the `~/Library` entries keyed by its
/// bundle id or name that exist right now; their bytes are `data_bytes`. `bundle_id` is the row
/// key: a bundle id for apps and leftovers, `<manager>:<name>` for tools (`brew:jq`).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Found {
    /// `app`, `leftover` (Library data with no installed app) or `tool` (a package manager's).
    pub kind: &'static str,
    pub bundle_id: String,
    pub name: String,
    pub path: String,
    pub version: String,
    /// Who installed it: `app-store`, `cask:<token>`, `other`, or for a tool its manager.
    pub source: String,
    pub app_bytes: u64,
    pub data_bytes: u64,
    pub data_paths: Vec<String>,
    /// `YYYY-MM-DD HH:MM:SS +0000` from Spotlight, or `None` when it has no date.
    pub last_used: Option<String>,
}

/// Everything a review covers: apps, then the leftovers no app owns, then tooling.
pub fn scan_all(home: &Path) -> Vec<Found> {
    let apps = scan(home);
    let mut out = crate::leftovers::scan(home, &apps);
    out.extend(crate::tooling::scan(home, &apps));
    out.splice(0..0, apps);
    out
}

/// Every `.app` directly in the roots, and one folder deeper (`/Applications/Utilities`,
/// `~/Applications/Chrome Apps`).
pub fn scan(home: &Path) -> Vec<Found> {
    let roots = [PathBuf::from("/Applications"), home.join("Applications")];
    let mut bundles = Vec::new();
    for root in &roots {
        collect(root, 2, &mut bundles);
    }
    bundles.sort();
    bundles.dedup();
    let casks = cask_apps(Path::new(crate::tooling::CASKROOM));
    let mut found: Vec<Found> = bundles
        .iter()
        .filter_map(|bundle| inspect(bundle, home, &casks))
        .collect();
    // Two copies of one bundle id (an app and its stale duplicate) keep the larger.
    found.sort_by(|a, b| {
        a.bundle_id
            .cmp(&b.bundle_id)
            .then(b.app_bytes.cmp(&a.app_bytes))
    });
    found.dedup_by(|a, b| a.bundle_id == b.bundle_id);
    found
}

fn collect(dir: &Path, depth: u8, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        // `file_type` is an lstat: a symlinked app is skipped rather than counted twice.
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if !kind.is_dir() {
            continue;
        }
        if path.extension().is_some_and(|ext| ext == "app") {
            out.push(path);
        } else if depth > 1 {
            collect(&path, depth - 1, out);
        }
    }
}

fn inspect(bundle: &Path, home: &Path, casks: &HashMap<String, String>) -> Option<Found> {
    let plist = info_plist(&bundle.join("Contents/Info.plist"))?;
    let bundle_id = plist.get("CFBundleIdentifier")?.as_str()?.to_owned();
    let name = ["CFBundleDisplayName", "CFBundleName"]
        .iter()
        .find_map(|key| {
            plist
                .get(*key)?
                .as_str()
                .map(clean_name)
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| {
            bundle
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default()
        });
    let version = plist
        .get("CFBundleShortVersionString")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_owned();
    let file = bundle
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let source = if let Some(token) = casks.get(&file) {
        format!("cask:{token}")
    } else if bundle.join("Contents/_MASReceipt/receipt").exists() {
        "app-store".to_owned()
    } else {
        "other".to_owned()
    };
    let data_paths: Vec<String> = data_candidates(home, &bundle_id, &name)
        .into_iter()
        .filter(|path| std::fs::symlink_metadata(path).is_ok())
        .map(|path| path.to_string_lossy().into_owned())
        .collect();
    Some(Found {
        kind: "app",
        app_bytes: du(&[bundle.to_string_lossy().into_owned()]),
        data_bytes: du(&data_paths),
        last_used: last_used(bundle),
        path: bundle.to_string_lossy().into_owned(),
        bundle_id,
        name,
        version,
        source,
        data_paths,
    })
}

/// Which cask installed which `.app`, from each cask's `INSTALL_RECEIPT.json`
/// (`uninstall_artifacts`, the `app` entries). Keyed by the bundle's file name.
pub fn cask_apps(caskroom: &Path) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let Ok(entries) = std::fs::read_dir(caskroom) else {
        return map;
    };
    for entry in entries.flatten() {
        let token = entry.file_name().to_string_lossy().into_owned();
        let receipt = entry.path().join(".metadata/INSTALL_RECEIPT.json");
        let Ok(text) = std::fs::read_to_string(receipt) else {
            continue;
        };
        for app in receipt_apps(&text) {
            map.insert(app, token.clone());
        }
    }
    map
}

/// The `.app` names a cask receipt says it installed.
pub fn receipt_apps(receipt: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(receipt) else {
        return Vec::new();
    };
    value["uninstall_artifacts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|artifact| artifact.get("app")?.as_array())
        .flatten()
        .filter_map(|app| app.as_str().map(str::to_owned))
        .collect()
}

/// Where an app keeps its state. Keyed by bundle id first; by display name only for the two
/// folders apps commonly name after themselves (`Application Support/Discord`, `Logs/OBS`).
pub fn data_candidates(home: &Path, bundle_id: &str, name: &str) -> Vec<PathBuf> {
    let lib = home.join("Library");
    let mut paths = vec![
        lib.join("Application Support").join(bundle_id),
        lib.join("Containers").join(bundle_id),
        lib.join("Caches").join(bundle_id),
        lib.join("HTTPStorages").join(bundle_id),
        lib.join("WebKit").join(bundle_id),
        lib.join("Saved Application State")
            .join(format!("{bundle_id}.savedState")),
        lib.join("Preferences").join(format!("{bundle_id}.plist")),
    ];
    if !name.is_empty() && !name.contains('/') {
        paths.push(lib.join("Application Support").join(name));
        paths.push(lib.join("Logs").join(name));
    }
    // Group containers are named `<team>.<bundle id>` or `group.<bundle id>[.suffix]`.
    if let Ok(entries) = std::fs::read_dir(lib.join("Group Containers")) {
        let mut groups: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .is_some_and(|n| group_matches(&n.to_string_lossy(), bundle_id))
            })
            .collect();
        groups.sort();
        paths.extend(groups);
    }
    paths
}

/// `HUAQ24HBR6.dev.orbstack` and `group.net.whatsapp.WhatsApp.shared` belong to
/// `dev.orbstack` and `net.whatsapp.WhatsApp`; `group.net.whatsapp.WhatsAppX` does not.
pub fn group_matches(folder: &str, bundle_id: &str) -> bool {
    let Some(at) = folder.find(bundle_id) else {
        return false;
    };
    let before = &folder[..at];
    let after = &folder[at + bundle_id.len()..];
    (before.is_empty() || before.ends_with('.')) && (after.is_empty() || after.starts_with('.'))
}

/// A bundle name without the invisible direction marks some apps put in it: WhatsApp's
/// `CFBundleDisplayName` starts with U+200E, which sorts and searches wrong.
pub fn clean_name(raw: &str) -> String {
    raw.chars()
        .filter(|c| !matches!(c, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}'))
        .collect::<String>()
        .trim()
        .to_owned()
}

fn info_plist(path: &Path) -> Option<serde_json::Map<String, serde_json::Value>> {
    let out = Command::new("plutil")
        .args(["-convert", "json", "-o", "-"])
        .arg(path)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    serde_json::from_slice(&out.stdout).ok()
}

fn last_used(bundle: &Path) -> Option<String> {
    let out = Command::new("mdls")
        .args(["-raw", "-name", "kMDItemLastUsedDate"])
        .arg(bundle)
        .output()
        .ok()?;
    parse_mdls(&String::from_utf8_lossy(&out.stdout))
}

/// `mdls -raw` prints `(null)` when Spotlight holds no value.
pub fn parse_mdls(raw: &str) -> Option<String> {
    let value = raw.trim();
    (!value.is_empty() && value != "(null)").then(|| value.to_owned())
}

/// Allocated bytes under the paths, hard links counted once (`du -c`).
pub fn du(paths: &[String]) -> u64 {
    if paths.is_empty() {
        return 0;
    }
    let Ok(out) = Command::new("du").arg("-skc").args(paths).output() else {
        return 0;
    };
    parse_du_total(&String::from_utf8_lossy(&out.stdout))
}

/// The `total` line of `du -skc`, in bytes. `du` exits non-zero on one unreadable entry and still
/// prints the total of what it could read, which is the useful answer.
pub fn parse_du_total(text: &str) -> u64 {
    text.lines()
        .rev()
        .find(|line| line.trim_end().ends_with("total"))
        .and_then(|line| line.split_whitespace().next()?.parse::<u64>().ok())
        .map_or(0, |kb| kb * 1024)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_containers_match_on_dot_boundaries_only() {
        assert!(group_matches("HUAQ24HBR6.dev.orbstack", "dev.orbstack"));
        assert!(group_matches(
            "group.net.whatsapp.WhatsApp.shared",
            "net.whatsapp.WhatsApp"
        ));
        assert!(group_matches("com.example.app", "com.example.app"));
        assert!(!group_matches(
            "group.net.whatsapp.WhatsAppX",
            "net.whatsapp.WhatsApp"
        ));
        assert!(!group_matches("xdev.orbstack", "dev.orbstack"));
    }

    #[test]
    fn names_lose_invisible_direction_marks() {
        assert_eq!(clean_name("\u{200E}WhatsApp"), "WhatsApp");
        assert_eq!(clean_name(" Zed "), "Zed");
    }

    #[test]
    fn cask_receipts_name_their_apps() {
        let receipt =
            r#"{"uninstall_artifacts":[{"uninstall":[{"quit":"x"}]},{"app":["Vorssaint.app"]}]}"#;
        assert_eq!(receipt_apps(receipt), vec!["Vorssaint.app".to_owned()]);
        assert!(receipt_apps("{}").is_empty());
    }

    #[test]
    fn mdls_null_is_no_date() {
        assert_eq!(parse_mdls("(null)"), None);
        assert_eq!(parse_mdls(""), None);
        assert_eq!(
            parse_mdls("2026-10-22 13:00:00 +0000"),
            Some("2026-10-22 13:00:00 +0000".into())
        );
    }

    #[test]
    fn du_total_is_read_even_after_an_unreadable_entry() {
        let text = "12\t/a\n30\t/b\n42\ttotal\n";
        assert_eq!(parse_du_total(text), 42 * 1024);
        assert_eq!(parse_du_total(""), 0);
    }

    #[test]
    fn data_candidates_key_on_bundle_id_and_name() {
        let home = Path::new("/nonexistent-home");
        let paths = data_candidates(home, "com.example.app", "Example");
        let lib = home.join("Library");
        assert!(paths.contains(&lib.join("Containers/com.example.app")));
        assert!(paths.contains(&lib.join("Application Support/Example")));
        assert!(paths.contains(&lib.join("Preferences/com.example.app.plist")));
    }
}
