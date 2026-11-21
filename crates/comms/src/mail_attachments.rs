//! Save the PDF attachments of receipt-mail threads into a folder that `documents inbox` imports.
//!
//! Documents owns originals. This module only stages bytes: each file goes to
//! `<out>/<thread-id>/<filename>`, so the source path Documents records names the Gmail thread.
//! An existing file is never overwritten, which makes a rerun idempotent.

use std::path::{Path, PathBuf};

use crate::{config::Config, google, store::TriageItem};

/// One thread that has no PDF attachment, so its receipt lives only in the mail body.
#[derive(Debug, serde::Serialize)]
pub struct BodyOnly {
    pub thread: String,
    pub date: Option<String>,
    pub subject: Option<String>,
}

#[derive(Debug, Default, serde::Serialize)]
pub struct Report {
    pub considered: usize,
    pub saved: usize,
    pub already_saved: usize,
    pub refused_c3: usize,
    pub missing_thread: usize,
    pub failed: usize,
    /// `<thread>: <error>` for each failed fetch or write.
    pub failed_threads: Vec<String>,
    pub body_only: Vec<BodyOnly>,
}

/// The selection a run reads from the triage store.
pub struct Selection<'a> {
    pub stream: &'a str,
    /// Inclusive `YYYY-MM-DD` lower bound on the mail date.
    pub since: Option<&'a str>,
    pub limit: usize,
}

/// Triage items in the stream, newest first, from `since` on.
pub fn select<'a>(items: &'a [TriageItem], selection: &Selection<'_>) -> Vec<&'a TriageItem> {
    items
        .iter()
        .filter(|item| item.stream == selection.stream)
        .filter(|item| {
            selection.since.is_none_or(|since| {
                item.internal_date_text
                    .as_deref()
                    .is_some_and(|date| date.get(..10).is_some_and(|day| day >= since))
            })
        })
        .take(selection.limit)
        .collect()
}

/// A filename safe to create inside the thread folder. Path separators and control
/// characters become `_`; an empty or dot-only name becomes `attachment.pdf`.
pub fn safe_filename(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or_default();
    let cleaned: String = base
        .chars()
        .map(|c| if c.is_control() || c == ':' { '_' } else { c })
        .collect();
    let cleaned = cleaned.trim().trim_start_matches('.').to_string();
    let cleaned = if cleaned.is_empty() {
        "attachment.pdf".to_string()
    } else {
        cleaned
    };
    if cleaned.to_ascii_lowercase().ends_with(".pdf") {
        cleaned
    } else {
        format!("{cleaned}.pdf")
    }
}

fn target(out: &Path, thread: &str, filename: &str) -> PathBuf {
    out.join(thread).join(safe_filename(filename))
}

/// Fetch and save. With `dry_run`, fetch and count but write nothing.
///
/// # Errors
/// The Gmail token cannot be read.
pub fn run(
    cfg: &Config,
    items: &[&TriageItem],
    out: &Path,
    dry_run: bool,
) -> Result<Report, String> {
    let token = google::access_token(&cfg.google_env_path).map_err(|e| e.to_string())?;
    let mut report = Report::default();
    for item in items {
        report.considered += 1;
        if item.data_class == "c3" {
            report.refused_c3 += 1;
            continue;
        }
        let attachments = match google::thread_pdf_attachments(&token, &item.id) {
            Ok(Some(attachments)) => attachments,
            Ok(None) => {
                report.missing_thread += 1;
                continue;
            }
            Err(error) => {
                report.failed += 1;
                report.failed_threads.push(format!("{}: {error}", item.id));
                continue;
            }
        };
        if attachments.is_empty() {
            report.body_only.push(BodyOnly {
                thread: item.id.clone(),
                date: item
                    .internal_date_text
                    .as_deref()
                    .map(|d| d.chars().take(10).collect()),
                subject: item.subject.clone(),
            });
            continue;
        }
        for attachment in attachments {
            let path = target(out, &item.id, &attachment.filename);
            if path.exists() {
                report.already_saved += 1;
                continue;
            }
            if dry_run {
                report.saved += 1;
                continue;
            }
            let written = path
                .parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|()| {
                    std::fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&path)
                })
                .and_then(|mut file| std::io::Write::write_all(&mut file, &attachment.bytes));
            match written {
                Ok(()) => report.saved += 1,
                Err(error) => {
                    report.failed += 1;
                    report.failed_threads.push(format!("{}: {error}", item.id));
                }
            }
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filenames_stay_inside_the_thread_folder() {
        assert_eq!(safe_filename("../../etc/passwd"), "passwd.pdf");
        assert_eq!(safe_filename("C:\\x\\Rechnung 12.PDF"), "Rechnung 12.PDF");
        assert_eq!(safe_filename(""), "attachment.pdf");
        assert_eq!(safe_filename(".."), "attachment.pdf");
        assert_eq!(safe_filename("a\u{0}b.pdf"), "a_b.pdf");
        assert_eq!(
            target(Path::new("/o"), "abc", "../x.pdf"),
            PathBuf::from("/o/abc/x.pdf")
        );
    }
}
