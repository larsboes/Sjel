//! `~/Library` data that no installed app owns: what an app left behind when it was dragged to
//! the Trash, which is how Spotify and MacSpace were found on 2026-10-22, by hand.
//!
//! A folder counts when its name is a reverse-DNS id, the id is not Apple's, no installed app's
//! bundle id is a prefix of it or it of one (`com.spotify.client.helper` belongs to
//! `com.spotify.client`), and together its folders hold at least `MIN_BYTES`. Below that it is a
//! preferences file, and a review list full of those is one nobody reads.
//!
//! This is a candidate list, not a verdict: an app outside /Applications, a helper with its own
//! id, or a command-line tool can own one. Nothing here removes anything; removal waits for the
//! operator's `remove` decision and an explicit uninstall.

use std::collections::BTreeMap;
use std::path::Path;

use crate::discover::{du, Found};

pub const MIN_BYTES: u64 = 1_048_576;

/// The folders an app's data lands in, and the suffix to strip from an entry's name to get the id.
const PLACES: &[(&str, &str)] = &[
    ("Application Support", ""),
    ("Containers", ""),
    ("Caches", ""),
    ("HTTPStorages", ""),
    ("WebKit", ""),
    ("Saved Application State", ".savedState"),
    ("Preferences", ".plist"),
];

pub fn scan(home: &Path, apps: &[Found]) -> Vec<Found> {
    let lib = home.join("Library");
    let owners: Vec<&str> = apps.iter().map(|a| a.bundle_id.as_str()).collect();
    let mut by_id: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (place, suffix) in PLACES {
        let Ok(entries) = std::fs::read_dir(lib.join(place)) else {
            continue;
        };
        for entry in entries.flatten() {
            let file = entry.file_name().to_string_lossy().into_owned();
            let Some(id) = file.strip_suffix(suffix) else {
                continue;
            };
            if is_orphan_id(id, &owners) {
                by_id
                    .entry(id.to_owned())
                    .or_default()
                    .push(entry.path().to_string_lossy().into_owned());
            }
        }
    }
    by_id
        .into_iter()
        .filter_map(|(id, paths)| {
            let bytes = du(&paths);
            (bytes >= MIN_BYTES).then(|| Found {
                kind: "leftover",
                name: id.clone(),
                bundle_id: id,
                path: String::new(),
                version: String::new(),
                source: "other".to_owned(),
                app_bytes: 0,
                data_bytes: bytes,
                data_paths: paths,
                last_used: None,
            })
        })
        .collect()
}

/// A reverse-DNS id that is not Apple's and that no installed app owns.
pub fn is_orphan_id(id: &str, owners: &[&str]) -> bool {
    if !looks_like_bundle_id(id) || id.starts_with("com.apple.") || id.starts_with("group.") {
        return false;
    }
    let lower = id.to_ascii_lowercase();
    !owners.iter().any(|owner| {
        let owner = owner.to_ascii_lowercase();
        related(&lower, &owner) || related(&owner, &lower)
    })
}

/// `child` is `parent` or sits under it on a dot boundary.
fn related(child: &str, parent: &str) -> bool {
    child == parent
        || child
            .strip_prefix(parent)
            .is_some_and(|rest| rest.starts_with('.'))
}

/// At least three dot-separated labels of letters, digits, dashes and underscores, the first
/// lowercase: `com.spotify.client`, not `Google` or `CloudKit`.
pub fn looks_like_bundle_id(id: &str) -> bool {
    let labels: Vec<&str> = id.split('.').collect();
    labels.len() >= 3
        && labels.iter().all(|label| {
            !label.is_empty()
                && label
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        })
        && labels[0].chars().all(|c| c.is_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_unowned_third_party_ids_are_leftovers() {
        let owners = ["com.spotify.client", "net.whatsapp.WhatsApp"];
        assert!(is_orphan_id("com.valvesoftware.steam", &owners));
        assert!(
            !is_orphan_id("com.spotify.client.helper", &owners),
            "helper of an installed app"
        );
        assert!(
            !is_orphan_id("com.spotify", &owners),
            "parent of an installed app"
        );
        assert!(
            !is_orphan_id("net.whatsapp.whatsapp", &owners),
            "case differs only"
        );
        assert!(!is_orphan_id("com.apple.Safari", &owners));
        assert!(!is_orphan_id("Google", &owners));
        assert!(
            is_orphan_id("com.spotifyx.app", &owners),
            "prefix without a dot boundary"
        );
    }

    #[test]
    fn bundle_ids_need_three_labels() {
        assert!(looks_like_bundle_id("com.example.app"));
        assert!(!looks_like_bundle_id("example.app"));
        assert!(!looks_like_bundle_id("Com.Example.App"));
        assert!(!looks_like_bundle_id("com..app"));
    }
}
