//! Data models and pure reductions for the Media ledger and ingest gate (`/media`).
//!
//! Replaces the dashboard's `/media` route. Backed by `capabilities/media` via
//! `GET /media/api/status`, `GET /media/api/ingests`, and `GET /media/api/ingests/{id}`.

use serde::{Deserialize, Serialize};

/// One mounted volume the ledger knows: its identifier, path count, and total indexed size.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct MediaVolume {
    pub uuid: String,
    pub label: String,
    pub locations: i64,
    pub bytes: i64,
    pub last_seen: String,
}

/// Overall ledger counts and the list of mounted volumes.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct MediaStatus {
    /// Distinct SHA-256 digests indexed in the ledger.
    pub files: i64,
    /// Total path locations where these digests sit across volumes.
    pub locations: i64,
    #[serde(default)]
    pub volumes: Vec<MediaVolume>,
}

/// One ingest run and its summary counts.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct MediaIngestRun {
    pub id: i64,
    pub source: String,
    pub staging: String,
    pub started: String,
    #[serde(default)]
    pub finished: Option<String>,
    pub considered: i64,
    pub imported: i64,
    pub duplicates: i64,
    pub refused: i64,
    pub failed: i64,
    pub resumed: i64,
    pub verified: i64,
}

/// Response payload from `GET /media/api/ingests`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct MediaIngestRunsResponse {
    #[serde(default)]
    pub runs: Vec<MediaIngestRun>,
}

/// One path considered during an ingest run and what happened to it.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct MediaIngestItem {
    pub relpath: String,
    #[serde(default)]
    pub digest: Option<String>,
    #[serde(default)]
    pub size: Option<i64>,
    pub disposition: String,
    pub reason: String,
    #[serde(default)]
    pub imported_relpath: Option<String>,
    pub verified: bool,
}

/// Detailed run breakdown including every path considered and its disposition.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct MediaIngestDetail {
    pub run: MediaIngestRun,
    #[serde(default)]
    pub items: Vec<MediaIngestItem>,
}

/// Combined view model for the primary `/media` screen.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct MediaView {
    pub status: MediaStatus,
    pub runs: Vec<MediaIngestRun>,
}

/// Format a byte count into a human-readable string with units (B, KiB, MiB, GiB, TiB).
pub fn format_bytes(bytes: i64) -> String {
    if bytes <= 0 {
        return "0 B".to_string();
    }
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut val = bytes as f64;
    let mut idx = 0;
    while val >= 1024.0 && idx < UNITS.len() - 1 {
        val /= 1024.0;
        idx += 1;
    }
    if idx == 0 {
        format!("{} B", bytes)
    } else {
        format!("{:.1} {}", val, UNITS[idx])
    }
}

/// Format the run title by taking the leaf folder of the source path.
pub fn run_label(run: &MediaIngestRun) -> String {
    let name = run
        .source
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty());
    match name {
        Some(n) => format!("{n} · run {}", run.id),
        None => format!("run {}", run.id),
    }
}

/// Sum the total bytes across all indexed volumes.
pub fn total_volume_bytes(status: &MediaStatus) -> i64 {
    status.volumes.iter().map(|v| v.bytes).sum()
}

/// Sort run items so that errors and refusals (`failed`, `refused`) appear first,
/// followed by alphabetical order by relative path.
pub fn sort_run_items(items: &[MediaIngestItem]) -> Vec<MediaIngestItem> {
    let mut sorted = items.to_vec();
    sorted.sort_by(|a, b| {
        let rank = |item: &MediaIngestItem| match item.disposition.as_str() {
            "failed" => 0,
            "refused" => 1,
            "resumed" => 2,
            "imported" => 3,
            "duplicate" => 4,
            _ => 5,
        };
        rank(a)
            .cmp(&rank(b))
            .then_with(|| a.relpath.cmp(&b.relpath))
    });
    sorted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(-50), "0 B");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1024), "1.0 KiB");
        assert_eq!(format_bytes(1536), "1.5 KiB");
        assert_eq!(format_bytes(1048576), "1.0 MiB");
        assert_eq!(format_bytes(1073741824), "1.0 GiB");
        assert_eq!(format_bytes(1099511627776), "1.0 TiB");
    }

    #[test]
    fn test_run_label() {
        let run = MediaIngestRun {
            id: 42,
            source: "/Volumes/Photos/2026-Import/".into(),
            ..Default::default()
        };
        assert_eq!(run_label(&run), "2026-Import · run 42");

        let run_bare = MediaIngestRun {
            id: 7,
            source: "".into(),
            ..Default::default()
        };
        assert_eq!(run_label(&run_bare), "run 7");
    }

    #[test]
    fn test_sort_run_items() {
        let items = vec![
            MediaIngestItem {
                relpath: "b.jpg".into(),
                disposition: "imported".into(),
                ..Default::default()
            },
            MediaIngestItem {
                relpath: "corrupt.mov".into(),
                disposition: "failed".into(),
                ..Default::default()
            },
            MediaIngestItem {
                relpath: "system.ds_store".into(),
                disposition: "refused".into(),
                ..Default::default()
            },
            MediaIngestItem {
                relpath: "a.jpg".into(),
                disposition: "imported".into(),
                ..Default::default()
            },
        ];

        let sorted = sort_run_items(&items);
        assert_eq!(sorted[0].disposition, "failed");
        assert_eq!(sorted[1].disposition, "refused");
        assert_eq!(sorted[2].relpath, "a.jpg");
        assert_eq!(sorted[3].relpath, "b.jpg");
    }

    #[test]
    fn test_deserialization_from_live_wire_shapes() {
        let status_json = r#"{
            "files": 45120,
            "locations": 52310,
            "volumes": [
                {
                    "uuid": "A1B2-C3D4",
                    "label": "SanDisk Extreme",
                    "locations": 52310,
                    "bytes": 284729103820,
                    "last_seen": "2026-10-20 18:42:10"
                }
            ]
        }"#;

        let status: MediaStatus = serde_json::from_str(status_json).expect("valid status JSON");
        assert_eq!(status.files, 45120);
        assert_eq!(status.volumes.len(), 1);
        assert_eq!(status.volumes[0].label, "SanDisk Extreme");
        assert_eq!(total_volume_bytes(&status), 284729103820);
    }
}
