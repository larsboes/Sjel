//! F4 — perceptual layer, advisory (`media_phashes` and `media_reviews`).
//!
//! Exact SHA-256 digest equality (`duplicates.rs`) is the only rule that may
//! authorize `supersede` or `reclaim`. Re-encoded JPEGs, HEIC-to-JPEG exports,
//! or slightly re-compressed copies have different SHA-256 digests and must
//! never be treated as byte-equal duplicates automatically.
//!
//! This module computes a 64-bit horizontal gradient hash (`dhash64`) over a
//! 9x8 luminance grid, stores it in `media_phashes`, proposes near-duplicate
//! pairs within a Hamming distance threshold, and records operator review
//! decisions (`near_duplicate`, `distinct`, `skip`) in `media_reviews`.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

use serde::Serialize;

use crate::store::{Ledger, Result, ReviewRow};

pub const DEFAULT_MAX_DISTANCE: u32 = 10;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PhashIndexReport {
    pub uuid: String,
    pub considered: usize,
    pub computed: usize,
    pub skipped_non_image: usize,
    pub failed: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PerceptualReport {
    pub phashes: usize,
    pub max_distance: u32,
    pub candidates: Vec<NearDuplicateCandidate>,
    pub reviewed_count: usize,
    pub unreviewed_count: usize,
    pub reviews: Vec<ReviewRow>,
    pub advisory_note: &'static str,
}

pub const ADVISORY_NOTE: &str =
    "Advisory only: perceptual near-duplicates have different bytes and are never passed to \
     `supersede` or `reclaim` automatically. Every pair requires explicit operator review.";

/// Compute a 64-bit horizontal gradient hash (`dhash64`) from a 9-column by
/// 8-row luminance grid (`72` bytes in row-major order).
pub fn dhash64_from_luma(grid_9x8: &[u8; 72]) -> u64 {
    let mut bits = 0_u64;
    let mut bit_index = 0_u32;
    for row in 0..8 {
        let base = row * 9;
        for col in 0..8 {
            if grid_9x8[base + col] > grid_9x8[base + col + 1] {
                bits |= 1_u64 << (63 - bit_index);
            }
            bit_index += 1;
        }
    }
    bits
}

/// Hamming distance between two 16-hex-digit 64-bit perceptual hashes.
pub fn hamming_distance(left_hex: &str, right_hex: &str) -> Option<u32> {
    let left = u64::from_str_radix(left_hex.trim(), 16).ok()?;
    let right = u64::from_str_radix(right_hex.trim(), 16).ok()?;
    Some((left ^ right).count_ones())
}

pub fn is_supported_image(relpath: &str) -> bool {
    let ext = relpath
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    matches!(
        ext.as_str(),
        "jpg" | "jpeg" | "png" | "heic" | "heif" | "tif" | "tiff" | "bmp" | "pgm" | "webp"
    )
}

/// Compute `dhash64` for an image file on disk.
///
/// Accepts raw binary PGM (`P5`) or uncompressed BMP directly, and falls back
/// to macOS `sips` (`sips -z 8 9 -s format bmp`) for JPEG, PNG, HEIC, TIFF, and WebP.
pub fn dhash64_file(path: &Path) -> Result<u64> {
    let bytes = std::fs::read(path)?;
    if let Some(grid) = decode_pgm_9x8(&bytes).or_else(|| decode_bmp_9x8(&bytes)) {
        return Ok(dhash64_from_luma(&grid));
    }
    #[cfg(target_os = "macos")]
    {
        let tmp = std::env::temp_dir().join(format!(
            "sjel-media-phash-{}-{}.bmp",
            std::process::id(),
            path.file_name().and_then(|n| n.to_str()).unwrap_or("img")
        ));
        let status = Command::new("sips")
            .args(["-z", "8", "9", "-s", "format", "bmp"])
            .arg(path)
            .arg("--out")
            .arg(&tmp)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()?;
        let result = if status.success() {
            let bmp = std::fs::read(&tmp)?;
            decode_bmp_9x8(&bmp)
                .map(|grid| dhash64_from_luma(&grid))
                .ok_or_else(|| "sips BMP output could not be decoded".into())
        } else {
            Err(format!("sips could not decode {}", path.display()).into())
        };
        let _ = std::fs::remove_file(&tmp);
        result
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err(format!("unsupported image format at {}", path.display()).into())
    }
}

/// Index `dhash64` perceptual hashes for indexed image locations on volume `uuid`.
pub fn index_phashes(ledger: &Ledger, root: &Path, uuid: &str) -> Result<PhashIndexReport> {
    let locations = ledger.locations(uuid)?;
    let existing: BTreeMap<String, String> = ledger
        .phashes()?
        .into_iter()
        .map(|row| (row.digest, row.phash))
        .collect();
    let mut considered = 0;
    let mut computed = 0;
    let mut skipped_non_image = 0;
    let mut failed = Vec::new();

    for loc in locations {
        if !is_supported_image(&loc.relpath) {
            skipped_non_image += 1;
            continue;
        }
        considered += 1;
        if existing.contains_key(&loc.digest) {
            continue;
        }
        let full_path = root.join(&loc.relpath);
        match dhash64_file(&full_path) {
            Ok(hash) => {
                ledger.record_phash(&loc.digest, hash, "dhash64")?;
                computed += 1;
            }
            Err(error) => {
                failed.push(format!("{}: {error}", loc.relpath));
            }
        }
    }

    Ok(PhashIndexReport {
        uuid: uuid.to_string(),
        considered,
        computed,
        skipped_non_image,
        failed,
    })
}

/// Propose near-duplicate image pairs whose perceptual hashes are within
/// `max_distance` bits, joining any recorded operator review decision.
pub fn propose_near_duplicates(
    ledger: &Ledger,
    uuid: Option<&str>,
    max_distance: u32,
) -> Result<PerceptualReport> {
    let phashes = ledger.phashes()?;
    let locations = match uuid {
        Some(vol) => ledger.locations(vol)?,
        None => ledger.all_locations()?,
    };
    let mut locs_by_digest: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for loc in locations {
        let label = if uuid.is_some() {
            loc.relpath
        } else {
            format!("{}:{}", loc.uuid, loc.relpath)
        };
        locs_by_digest.entry(loc.digest).or_default().push(label);
    }

    let reviews = ledger.reviews()?;
    let review_map: BTreeMap<(String, String), ReviewRow> = reviews
        .iter()
        .cloned()
        .map(|row| ((row.left_digest.clone(), row.right_digest.clone()), row))
        .collect();

    let scoped_phashes: Vec<_> = phashes
        .into_iter()
        .filter(|row| uuid.is_none() || locs_by_digest.contains_key(&row.digest))
        .collect();

    let mut candidates = Vec::new();
    for i in 0..scoped_phashes.len() {
        for j in (i + 1)..scoped_phashes.len() {
            let left = &scoped_phashes[i];
            let right = &scoped_phashes[j];
            let Some(distance) = hamming_distance(&left.phash, &right.phash) else {
                continue;
            };
            if distance <= max_distance {
                let review = review_map
                    .get(&(left.digest.clone(), right.digest.clone()))
                    .cloned();
                candidates.push(NearDuplicateCandidate {
                    left_digest: left.digest.clone(),
                    right_digest: right.digest.clone(),
                    left_phash: left.phash.clone(),
                    right_phash: right.phash.clone(),
                    hamming_distance: distance,
                    left_locations: locs_by_digest
                        .get(&left.digest)
                        .cloned()
                        .unwrap_or_default(),
                    right_locations: locs_by_digest
                        .get(&right.digest)
                        .cloned()
                        .unwrap_or_default(),
                    review,
                });
            }
        }
    }

    candidates.sort_by(|a, b| {
        a.hamming_distance
            .cmp(&b.hamming_distance)
            .then_with(|| a.left_digest.cmp(&b.left_digest))
            .then_with(|| a.right_digest.cmp(&b.right_digest))
    });

    let reviewed_count = candidates.iter().filter(|c| c.review.is_some()).count();
    let unreviewed_count = candidates.len() - reviewed_count;

    Ok(PerceptualReport {
        phashes: scoped_phashes.len(),
        max_distance,
        candidates,
        reviewed_count,
        unreviewed_count,
        reviews,
        advisory_note: ADVISORY_NOTE,
    })
}

/// Decode a binary PGM (`P5 <width> <height> <maxval>\n<bytes>`) and resample
/// to a 9x8 luminance grid.
fn decode_pgm_9x8(bytes: &[u8]) -> Option<[u8; 72]> {
    if !bytes.starts_with(b"P5") {
        return None;
    }
    let mut pos = 2;
    let mut tokens = Vec::with_capacity(3);
    while tokens.len() < 3 && pos < bytes.len() {
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        if pos < bytes.len() && bytes[pos] == b'#' {
            while pos < bytes.len() && bytes[pos] != b'\n' {
                pos += 1;
            }
            continue;
        }
        let start = pos;
        while pos < bytes.len() && !bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        if start < pos {
            tokens.push(std::str::from_utf8(&bytes[start..pos]).ok()?.parse::<usize>().ok()?);
        }
    }
    if tokens.len() != 3 || pos >= bytes.len() || !bytes[pos].is_ascii_whitespace() {
        return None;
    }
    pos += 1; // single whitespace byte after maxval
    let (width, height, _maxval) = (tokens[0], tokens[1], tokens[2]);
    if width == 0 || height == 0 {
        return None;
    }
    let pixels = bytes.get(pos..pos + width * height)?;
    let mut out = [0_u8; 72];
    for ry in 0..8 {
        let sy = (ry * height) / 8;
        for rx in 0..9 {
            let sx = (rx * width) / 9;
            out[ry * 9 + rx] = pixels[sy * width + sx];
        }
    }
    Some(out)
}

/// Decode an uncompressed 24-bit or 32-bit BMP into a 9x8 luminance grid.
fn decode_bmp_9x8(bytes: &[u8]) -> Option<[u8; 72]> {
    if bytes.len() < 54 || &bytes[0..2] != b"BM" {
        return None;
    }
    let data_offset = u32::from_le_bytes(bytes[10..14].try_into().ok()?) as usize;
    let width = i32::from_le_bytes(bytes[18..22].try_into().ok()?);
    let height_raw = i32::from_le_bytes(bytes[22..26].try_into().ok()?);
    let bpp = u16::from_le_bytes(bytes[28..30].try_into().ok()?);
    let compression = u32::from_le_bytes(bytes[30..34].try_into().ok()?);
    if width <= 0 || height_raw == 0 || !matches!(bpp, 24 | 32) || !matches!(compression, 0 | 3) {
        return None;
    }
    let w = width as usize;
    let h = height_raw.unsigned_abs() as usize;
    let top_down = height_raw < 0;
    let bytes_per_pixel = (bpp / 8) as usize;
    let row_stride = (w * bytes_per_pixel).div_ceil(4) * 4;

    let mut out = [0_u8; 72];
    for ry in 0..8 {
        let sy = (ry * h) / 8;
        let bmp_row = if top_down { sy } else { h - 1 - sy };
        for rx in 0..9 {
            let sx = (rx * w) / 9;
            let idx = data_offset + bmp_row * row_stride + sx * bytes_per_pixel;
            let b = *bytes.get(idx)? as u32;
            let g = *bytes.get(idx + 1)? as u32;
            let r = *bytes.get(idx + 2)? as u32;
            out[ry * 9 + rx] = ((r * 77 + g * 150 + b * 29) >> 8) as u8;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Location;

    #[test]
    fn a_near_duplicate_is_proposed_from_a_phash_and_reviewed() {
        let dir = std::env::temp_dir().join(format!(
            "sjel-media-perceptual-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let ledger = Ledger::open(&dir.join("media.db")).unwrap();
        ledger.register("VOL-1", "Library").unwrap();

        // Build two 9x8 PGM images with a 1-pixel gradient tweak (different SHA-256,
        // nearly identical perceptual dHash) and one completely inverted image.
        let mut grid_a = [0_u8; 72];
        for r in 0..8 {
            for c in 0..9 {
                grid_a[r * 9 + c] = ((r * 16 + c * 20).min(255)) as u8;
            }
        }
        let mut grid_b = grid_a;
        grid_b[0] = grid_b[1].saturating_add(5); // flips at most 1 bit of the 64-bit dHash

        let mut grid_c = [0_u8; 72];
        for r in 0..8 {
            for c in 0..9 {
                grid_c[r * 9 + c] = (255 - (r * 16 + c * 20).min(255)) as u8;
            }
        }

        let write_pgm = |name: &str, grid: &[u8; 72]| {
            let path = dir.join(name);
            let mut data = b"P5 9 8 255\n".to_vec();
            data.extend_from_slice(grid);
            std::fs::write(&path, &data).unwrap();
            let digest = crate::hash(&path).unwrap();
            ledger
                .record(&Location {
                    uuid: "VOL-1".into(),
                    relpath: name.into(),
                    digest: digest.clone(),
                    size: data.len() as i64,
                    mtime_ns: 0,
                })
                .unwrap();
            digest
        };

        let digest_a = write_pgm("original.pgm", &grid_a);
        let digest_b = write_pgm("reencoded.pgm", &grid_b);
        let digest_c = write_pgm("distinct.pgm", &grid_c);
        assert_ne!(digest_a, digest_b, "near-duplicates have distinct byte digests");

        let index_report = index_phashes(&ledger, &dir, "VOL-1").unwrap();
        assert_eq!(index_report.computed, 3);
        assert!(index_report.failed.is_empty());

        // Propose near-duplicates before review
        let before = propose_near_duplicates(&ledger, Some("VOL-1"), DEFAULT_MAX_DISTANCE).unwrap();
        assert_eq!(before.phashes, 3);
        assert_eq!(
            before.candidates.len(),
            1,
            "only the near-duplicate pair is within distance 10; the inverted image is ~63 bits away"
        );
        assert_eq!(before.unreviewed_count, 1);
        assert_eq!(before.reviewed_count, 0);
        let pair = &before.candidates[0];
        assert!(pair.hamming_distance <= 1);
        assert!(pair.review.is_none());
        assert!(!pair.left_digest.contains(&digest_c));
        assert!(!pair.right_digest.contains(&digest_c));

        // Record operator review decision in `media_reviews`
        let review = ledger
            .record_review(
                &digest_a,
                &digest_b,
                "near_duplicate",
                "re-exported copy of original.pgm",
            )
            .unwrap();
        assert_eq!(review.decision, "near_duplicate");

        // Re-query proposals: the candidate now carries the recorded review
        let after = propose_near_duplicates(&ledger, Some("VOL-1"), DEFAULT_MAX_DISTANCE).unwrap();
        assert_eq!(after.reviewed_count, 1);
        assert_eq!(after.unreviewed_count, 0);
        assert_eq!(
            after.candidates[0].review.as_ref().unwrap().decision,
            "near_duplicate"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
