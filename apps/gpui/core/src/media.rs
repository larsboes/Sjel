//! Data models and pure reductions for the Media ledger and ingest gate (`/media`).
//!
//! Replaces the dashboard's `/media` route. Backed by `crates/media` via
//! `GET /media/api/status`, `GET /media/api/ingests`, and `GET /media/api/ingests/{id}`.

pub use media_api::*;

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
