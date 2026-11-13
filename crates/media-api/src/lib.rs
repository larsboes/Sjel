//! Pure wire models, contracts, and route constants for the Sjel media capability.
//!
//! Shared across server capability (`sjel-media`) and client apps (`sjel-gpui-core`).

mod content;
pub use content::*;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

// ============================================================================
// Route Constants
// ============================================================================

pub const DEFAULT_MEDIA_PORT: u16 = 8102;

pub const ROUTE_MEDIA_HEALTH: &str = "/health";
pub const ROUTE_MEDIA_ROUTES: &str = "/routes";
pub const ROUTE_MEDIA_STATUS: &str = "/api/status";
pub const ROUTE_MEDIA_INGESTS: &str = "/api/ingests";
pub const ROUTE_MEDIA_INGEST_BY_ID: &str = "/api/ingests/{id}";
pub const ROUTE_MEDIA_PERCEPTUAL: &str = "/api/perceptual";
pub const ROUTE_MEDIA_GEOTAGS: &str = "/api/geotags";
pub const ROUTE_MUSIC_PROBE: &str = "/api/probe";
pub const ROUTE_MUSIC_BUILD: &str = "/api/build";
pub const ROUTE_MUSIC_JOBS: &str = "/api/jobs";
pub const ROUTE_MUSIC_JOB_BY_ID: &str = "/api/jobs/{id}";

// ============================================================================
// Media Ledger and Ingest Models
// ============================================================================

/// One mounted volume the ledger knows: its identifier, path count, and total indexed size.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct MediaVolume {
    pub uuid: String,
    pub label: String,
    pub locations: i64,
    pub bytes: i64,
    pub last_seen: String,
}

pub type VolumeRow = MediaVolume;

/// Overall ledger counts and the list of mounted volumes.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct MediaStatus {
    /// Distinct SHA-256 digests indexed in the ledger.
    pub files: i64,
    /// Total path locations where these digests sit across volumes.
    pub locations: i64,
    #[serde(default)]
    pub volumes: Vec<MediaVolume>,
}

/// One ingest run and its summary counts.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
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

pub type IngestRow = MediaIngestRun;

/// Response payload from `GET /api/ingests`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct MediaIngestRunsResponse {
    #[serde(default)]
    pub runs: Vec<MediaIngestRun>,
}

/// One path considered during an ingest run and what happened to it.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
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

pub type ItemRow = MediaIngestItem;

/// Detailed run breakdown including every path considered and its disposition.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct MediaIngestDetail {
    pub run: MediaIngestRun,
    #[serde(default)]
    pub items: Vec<MediaIngestItem>,
}

/// Combined view model for the primary `/media` screen.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct MediaView {
    pub status: MediaStatus,
    pub runs: Vec<MediaIngestRun>,
}

// ============================================================================
// Perceptual and Location Models
// ============================================================================

pub const DEFAULT_MAX_DISTANCE: u32 = 10;

pub const ADVISORY_NOTE: &str =
    "Advisory only: perceptual near-duplicates have different bytes and are never passed to \
     `supersede` or `reclaim` automatically. Every pair requires explicit operator review.";

/// One digest's perceptual hash in the advisory perceptual layer.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct PhashRow {
    pub digest: String,
    pub phash: String,
    pub algorithm: String,
    pub computed_at: String,
}

/// One operator review decision over a pair of distinct digests.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct ReviewRow {
    pub left_digest: String,
    pub right_digest: String,
    pub decision: String,
    pub note: String,
    pub decided_at: String,
}

/// One digest's embedded GPS coordinates and capture day.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, JsonSchema)]
pub struct GeotagRow {
    pub digest: String,
    pub latitude: f64,
    pub longitude: f64,
    #[serde(default)]
    pub captured_at: Option<String>,
    pub indexed_at: String,
}

/// Response payload from `GET /api/geotags`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, JsonSchema)]
pub struct GeotagsResponse {
    #[serde(default)]
    pub geotags: Vec<GeotagRow>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct PhashIndexReport {
    pub uuid: String,
    pub considered: usize,
    pub computed: usize,
    pub skipped_non_image: usize,
    pub failed: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct NearDuplicateCandidate {
    pub left_digest: String,
    pub right_digest: String,
    pub left_phash: String,
    pub right_phash: String,
    pub hamming_distance: u32,
    pub left_locations: Vec<String>,
    pub right_locations: Vec<String>,
    pub review: Option<ReviewRow>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct PerceptualReport {
    pub phashes: usize,
    pub max_distance: u32,
    pub candidates: Vec<NearDuplicateCandidate>,
    pub reviewed_count: usize,
    pub unreviewed_count: usize,
    pub reviews: Vec<ReviewRow>,
    pub advisory_note: &'static str,
}

// ============================================================================
// Reductions and Utilities
// ============================================================================

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

// ============================================================================
// Music Ingest & Build Models
// ============================================================================

/// Metadata returned by `POST /music/api/probe`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct MusicProbe {
    pub title: String,
    pub uploader: String,
    pub duration_hms: String,
    #[serde(default)]
    pub chapters: Vec<MusicChapter>,
    #[serde(default)]
    pub tracklist_from_chapters: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct MusicChapter {
    pub start: u64,
    pub title: String,
}

/// Request and response contract for a music build.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MusicJob {
    pub id: String,
    pub album: String,
    pub state: String,
    #[serde(default)]
    pub log: Vec<String>,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub finished_at: Option<String>,
    #[serde(default)]
    pub outdir: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, JsonSchema)]
pub struct MusicBuild {
    pub url: String,
    /// `"song"` imports one track; empty or `"album"` cuts a tracklist.
    pub mode: String,
    pub album: String,
    pub artist: String,
    pub year: String,
    pub tracklist: String,
    /// Song mode only: the track title and an optional `M:SS` range.
    pub title: String,
    pub start: String,
    pub end: String,
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
