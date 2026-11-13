//! Data models and pure reductions for the Media ledger and ingest gate (`/media`).
//!
//! Replaces the dashboard's `/media` route. Backed by `crates/media` via
//! `GET /media/api/status`, `GET /media/api/ingests`, and `GET /media/api/ingests/{id}`.

pub use media_api::*;

/// Decode only the selected proposal's bounded PNG previews.
///
/// # Errors
/// A preview is invalid base64 or exceeds the evidence contract's size limit.
pub fn content_previews(detail: &ContentDetail) -> Result<Vec<Vec<u8>>, String> {
    use base64::Engine;
    detail
        .frames
        .iter()
        .map(|frame| {
            if frame.preview_png.len() > 700_000 {
                return Err("Preview exceeds size limit".into());
            }
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(&frame.preview_png)
                .map_err(|_| "Invalid preview encoding".to_string())?;
            if bytes.len() > 512 * 1024 {
                return Err("Preview exceeds size limit".into());
            }
            Ok(bytes)
        })
        .collect()
}

/// Split explicit review text into stable, unique metadata names without choosing candidates.
pub fn content_names(text: &str) -> Vec<String> {
    text.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_names_require_explicit_input_and_deduplicate_chosen_names() {
        assert!(content_names("").is_empty());
        assert_eq!(
            content_names(" Travel, , Family,Travel "),
            ["Family", "Travel"]
        );
    }

    #[test]
    fn selected_previews_decode_without_reading_original_paths() {
        let detail: ContentDetail = serde_json::from_value(serde_json::json!({
            "proposal": {"id":1,"sha256":"a".repeat(64),"fingerprint":"b".repeat(64),
                "kind":"image","engine":"fixture","revision":1,"state":"pending","tags":[],"collections":[],"created_at":"fixture"},
            "os":"fixture","filenames":["original.png"],
            "frames":[{"requestedSeconds":null,"seconds":null,"labels":[],"previewPng":"AQID"}]
        })).unwrap();
        assert_eq!(content_previews(&detail).unwrap(), [vec![1, 2, 3]]);
        let mut invalid = detail;
        invalid.frames[0].preview_png = "not base64".into();
        assert!(content_previews(&invalid).is_err());
    }

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
