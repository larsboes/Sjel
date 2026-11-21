//! Export the originals of selected mail for an owner that keeps them, such as `offers import`.
//!
//! Each thread becomes `<out>/<thread-id>/original.eml` (the latest message, RFC 822) and
//! `meta.json` (thread, date, sender, subject). An existing folder is skipped, so a rerun is
//! idempotent. `c3` threads are refused.

use std::path::Path;

use crate::{config::Config, google, store::TriageItem};

#[derive(Debug, Default, serde::Serialize)]
pub struct Report {
    pub considered: usize,
    pub exported: usize,
    pub already_exported: usize,
    pub refused_c3: usize,
    pub missing: usize,
    /// `<thread>: <error>` for each failed fetch or write.
    pub failed: Vec<String>,
}

/// What a run selects. Every condition present must hold.
pub struct Selection<'a> {
    /// Case-insensitive substring of the stored sender.
    pub from: Option<&'a str>,
    pub stream: Option<&'a str>,
    /// Inclusive `YYYY-MM-DD` lower bound on the mail date.
    pub since: Option<&'a str>,
    pub limit: usize,
}

pub fn select<'a>(items: &'a [TriageItem], selection: &Selection<'_>) -> Vec<&'a TriageItem> {
    let from = selection.from.map(str::to_lowercase);
    items
        .iter()
        .filter(|item| {
            from.as_deref().is_none_or(|from| {
                item.from_addr
                    .as_deref()
                    .is_some_and(|addr| addr.to_lowercase().contains(from))
            })
        })
        .filter(|item| selection.stream.is_none_or(|s| item.stream == s))
        .filter(|item| {
            selection.since.is_none_or(|since| {
                item.internal_date_text
                    .as_deref()
                    .and_then(|d| d.get(..10))
                    .is_some_and(|day| day >= since)
            })
        })
        .take(selection.limit)
        .collect()
}

#[derive(serde::Serialize)]
struct Meta<'a> {
    source: String,
    date: Option<&'a str>,
    from: Option<&'a str>,
    subject: Option<&'a str>,
}

/// # Errors
/// The Gmail token cannot be read.
pub fn run(cfg: &Config, items: &[&TriageItem], out: &Path) -> Result<Report, String> {
    let token = google::access_token(&cfg.google_env_path).map_err(|e| e.to_string())?;
    let mut report = Report::default();
    for item in items {
        report.considered += 1;
        if item.data_class == "c3" {
            report.refused_c3 += 1;
            continue;
        }
        let folder = out.join(&item.id);
        if folder.join("meta.json").exists() {
            report.already_exported += 1;
            continue;
        }
        let raw = match google::thread_latest_raw(&token, &item.id) {
            Ok(Some(raw)) => raw,
            Ok(None) => {
                report.missing += 1;
                continue;
            }
            Err(error) => {
                report.failed.push(format!("{}: {error}", item.id));
                continue;
            }
        };
        let meta = Meta {
            source: format!("gmail:{}", item.id),
            date: item.internal_date_text.as_deref().and_then(|d| d.get(..10)),
            from: item.from_addr.as_deref(),
            subject: item.subject.as_deref(),
        };
        // `meta.json` is written last, so a folder without it is an interrupted export.
        let written = std::fs::create_dir_all(&folder)
            .and_then(|()| std::fs::write(folder.join("original.eml"), &raw))
            .and_then(|()| {
                std::fs::write(
                    folder.join("meta.json"),
                    serde_json::to_vec_pretty(&meta).expect("meta serializes"),
                )
            });
        match written {
            Ok(()) => report.exported += 1,
            Err(error) => report.failed.push(format!("{}: {error}", item.id)),
        }
    }
    Ok(report)
}
