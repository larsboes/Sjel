//! Finding and fetching the monthly parquet releases.
//!
//! The files are immutable once published — a new month is a new file, never a rewrite
//! of an old one — so a local copy that exists is a local copy that is current, and
//! ingest re-runs cost nothing after the first. That is also why the raw directory is
//! a cache and not state worth backing up: every byte of it is re-downloadable from the
//! upstream at any time.

use serde::Deserialize;
use std::io::Write;
use std::path::{Path, PathBuf};

const REPO: &str = "piebro/deutsche-bahn-data";
const DIR: &str = "monthly_processed_data";

/// First month covering every station rather than the largest ~100.
///
/// Upstream widened collection on 2025-11-02, which makes 2025-11 itself a mixed month:
/// two days of the narrow set, the rest of the wide one. Starting at 2025-12 costs one
/// month of history and buys a window where station coverage is constant — without
/// that, a station's statistics would silently depend on when it entered the dataset,
/// and comparing two stations would partly be comparing their collection start dates.
pub const FIRST_FULL_COVERAGE_MONTH: &str = "2025-12";

#[derive(Debug, thiserror::Error)]
pub enum DatasetError {
    #[error("listing {REPO} failed: {0}")]
    Listing(#[source] reqwest::Error),
    #[error("downloading {0} failed: {1}")]
    Download(String, #[source] reqwest::Error),
    #[error("writing {0} failed: {1}")]
    Write(PathBuf, #[source] std::io::Error),
    #[error("no monthly files in range {0}..={1} — upstream lists {2} file(s) total")]
    EmptyRange(String, String, usize),
}

#[derive(Deserialize)]
struct TreeEntry {
    path: String,
    /// Hugging Face's blob identity lets ingest notice a corrected historical month
    /// instead of treating an old filename as immutable forever.
    oid: Option<String>,
}

/// A published monthly release: `2026-06` plus where it lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Month {
    pub id: String,
    pub remote_path: String,
    /// The upstream blob identity, when the listing exposes one.
    pub oid: Option<String>,
}

impl Month {
    pub fn url(&self) -> String {
        format!(
            "https://huggingface.co/datasets/{REPO}/resolve/main/{}",
            self.remote_path
        )
    }

    pub fn local_path(&self, raw_dir: &Path) -> PathBuf {
        raw_dir.join(format!("data-{}.parquet", self.id))
    }
}

/// Parses `monthly_processed_data/data-2026-06.parquet` into `2026-06`. Anything that
/// does not match that shape is not a monthly release and is skipped rather than
/// guessed at — upstream is free to put other things in that directory.
fn month_id(path: &str) -> Option<String> {
    let file = path.rsplit('/').next()?;
    let stem = file.strip_prefix("data-")?.strip_suffix(".parquet")?;
    let (year, month) = stem.split_once('-')?;
    if year.len() == 4 && month.len() == 2 && stem.chars().all(|c| c.is_ascii_digit() || c == '-') {
        Some(stem.to_string())
    } else {
        None
    }
}

/// Every monthly release upstream publishes, oldest first.
pub fn list_months(client: &reqwest::blocking::Client) -> Result<Vec<Month>, DatasetError> {
    let url = format!("https://huggingface.co/api/datasets/{REPO}/tree/main/{DIR}");
    let entries: Vec<TreeEntry> = client
        .get(&url)
        .send()
        .and_then(|r| r.error_for_status())
        .and_then(|r| r.json())
        .map_err(DatasetError::Listing)?;

    let mut months: Vec<Month> = entries
        .into_iter()
        .filter_map(|e| {
            month_id(&e.path).map(|id| Month {
                id,
                remote_path: e.path,
                oid: e.oid,
            })
        })
        .collect();
    // Ids are zero-padded YYYY-MM, so lexical order is chronological order.
    months.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(months)
}

pub fn select(
    months: Vec<Month>,
    from: &str,
    to: Option<&str>,
) -> Result<Vec<Month>, DatasetError> {
    let total = months.len();
    let upper = to.unwrap_or("9999-99").to_string();
    let picked: Vec<Month> = months
        .into_iter()
        .filter(|m| m.id.as_str() >= from && m.id <= upper)
        .collect();
    if picked.is_empty() {
        return Err(DatasetError::EmptyRange(from.to_string(), upper, total));
    }
    Ok(picked)
}

/// How many times a month's download may fail before the run gives up.
///
/// Not decoration: a 600 MB body over a long connection drops occasionally, and the
/// first real run died on month six of seven with `request or response body error`
/// after five months were already parsed. Aggregation cannot commit a partial window
/// (see `Store::replace_stats`), so one transient socket error would otherwise throw
/// away ~74M rows of work.
const DOWNLOAD_ATTEMPTS: u32 = 4;

/// Downloads a month if it is not already on disk. Writes to a `.part` file and renames
/// on success, so an interrupted download can never be mistaken for a complete one by
/// the next run — which is exactly what saved the first failure from becoming a
/// truncated parquet parsed as if it were whole.
pub fn ensure_local(
    client: &reqwest::blocking::Client,
    month: &Month,
    raw_dir: &Path,
    refresh: bool,
) -> Result<PathBuf, DatasetError> {
    let target = month.local_path(raw_dir);
    if target.is_file() && !refresh {
        return Ok(target);
    }
    std::fs::create_dir_all(raw_dir).map_err(|e| DatasetError::Write(raw_dir.to_path_buf(), e))?;

    let partial = target.with_extension("part");
    for attempt in 1..=DOWNLOAD_ATTEMPTS {
        match fetch_to(client, month, &partial) {
            Ok(()) => {
                std::fs::rename(&partial, &target)
                    .map_err(|e| DatasetError::Write(target.clone(), e))?;
                return Ok(target);
            }
            // A write failure is not retried: a full or read-only disk does not fix
            // itself, and three more attempts would only delay the message.
            Err(e @ DatasetError::Write(..)) => return Err(e),
            Err(e) => {
                // Start over rather than resume. Resuming needs upstream to honour a
                // range request, and a range silently ignored would append a second
                // prefix onto the first and produce a file that looks whole. Slower
                // beats subtly wrong.
                let _ = std::fs::remove_file(&partial);
                if attempt == DOWNLOAD_ATTEMPTS {
                    return Err(e);
                }
                let backoff = std::time::Duration::from_secs(2u64.pow(attempt));
                eprintln!(
                    "punctuality: {} attempt {attempt}/{DOWNLOAD_ATTEMPTS} failed ({e}), retrying in {}s",
                    month.id,
                    backoff.as_secs()
                );
                std::thread::sleep(backoff);
            }
        }
    }
    unreachable!("the final attempt returns from inside the loop")
}

fn fetch_to(
    client: &reqwest::blocking::Client,
    month: &Month,
    partial: &Path,
) -> Result<(), DatasetError> {
    let mut response = client
        .get(month.url())
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|e| DatasetError::Download(month.id.clone(), e))?;
    let mut file = std::fs::File::create(partial)
        .map_err(|e| DatasetError::Write(partial.to_path_buf(), e))?;
    response
        .copy_to(&mut file)
        .map_err(|e| DatasetError::Download(month.id.clone(), e))?;
    file.flush()
        .map_err(|e| DatasetError::Write(partial.to_path_buf(), e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn month_ids_come_only_from_monthly_release_names() {
        assert_eq!(
            month_id("monthly_processed_data/data-2026-06.parquet").as_deref(),
            Some("2026-06")
        );
        assert_eq!(month_id("data-2024-07.parquet").as_deref(), Some("2024-07"));
        // Upstream also ships raw_data/ and a README; neither is a monthly release.
        assert_eq!(month_id("monthly_processed_data/README.md"), None);
        assert_eq!(month_id("raw_data/year=2026/month=6/part-0.parquet"), None);
        assert_eq!(month_id("monthly_processed_data/data-2026.parquet"), None);
        assert_eq!(month_id("monthly_processed_data/data-2026-6.parquet"), None);
    }

    fn months(ids: &[&str]) -> Vec<Month> {
        ids.iter()
            .map(|id| Month {
                id: (*id).to_string(),
                remote_path: format!("{DIR}/data-{id}.parquet"),
                oid: None,
            })
            .collect()
    }

    #[test]
    fn selection_is_inclusive_on_both_ends() {
        let picked = select(
            months(&["2025-10", "2025-11", "2025-12", "2026-01"]),
            "2025-11",
            Some("2025-12"),
        )
        .unwrap();
        assert_eq!(
            picked.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            ["2025-11", "2025-12"]
        );
    }

    #[test]
    fn an_open_upper_bound_takes_everything_after_from() {
        let picked = select(
            months(&["2025-10", "2025-12", "2026-06"]),
            FIRST_FULL_COVERAGE_MONTH,
            None,
        )
        .unwrap();
        assert_eq!(
            picked.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            ["2025-12", "2026-06"]
        );
    }

    #[test]
    fn an_empty_range_is_an_error_not_a_silent_no_op() {
        // Ingesting nothing and reporting success would look identical to ingesting
        // everything, right up until someone reads an empty table.
        let err = select(months(&["2025-12"]), "2030-01", None).unwrap_err();
        assert!(matches!(err, DatasetError::EmptyRange(..)));
    }

    #[test]
    fn the_download_url_points_at_the_listed_path() {
        let m = &months(&["2026-06"])[0];
        assert_eq!(
            m.url(),
            "https://huggingface.co/datasets/piebro/deutsche-bahn-data/resolve/main/monthly_processed_data/data-2026-06.parquet"
        );
    }
}
