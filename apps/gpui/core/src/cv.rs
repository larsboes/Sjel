//! Data models and pure reductions for CV and career applications (`/cv`).
//!
//! Replaces the dashboard's `/cv` route. Backed by `capabilities/cv/cv` through
//! `GET /api/sjel-status/cv` and `POST /api/sjel-status/cv`.

use serde::{Deserialize, Serialize};

/// One historical event in an application's lifecycle.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct CvLogEntry {
    pub date: String,
    pub status: String,
    #[serde(default)]
    pub note: Option<String>,
}

/// One job application in the ledger.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct CvApplication {
    pub company: String,
    pub role: String,
    #[serde(default)]
    pub url: Option<String>,
    pub profile: String,
    pub lang: String,
    pub status: String,
    #[serde(default)]
    pub log: Vec<CvLogEntry>,
    #[serde(default)]
    pub notes: Option<String>,
    pub slug: String,
}

/// A built CV PDF document and its staleness relative to `master_cv.yaml`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct CvPdf {
    pub name: String,
    pub path: String,
    pub mtime: u64,
    pub stale: bool,
}

/// The full payload emitted by `capabilities/cv/cv status --json`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct CvView {
    pub master: String,
    pub master_mtime: u64,
    pub profiles: Vec<String>,
    pub statuses: Vec<String>,
    pub applications: Vec<CvApplication>,
    pub pdfs: Vec<CvPdf>,
    #[serde(default)]
    pub linkedin_pulled: Option<String>,
}

/// Request sent to `POST /api/sjel-status/cv`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CvWriteRequest {
    pub action: String,
    #[serde(default)]
    pub profile: String,
    #[serde(default)]
    pub lang: String,
    #[serde(default)]
    pub slug: String,
    #[serde(default)]
    pub company: String,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub note: String,
}

/// Response from `POST /api/sjel-status/cv`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct CvWriteResult {
    pub action: String,
    pub detail: String,
    #[serde(default)]
    pub warnings: String,
}

/// Generate a URL/filesystem slug from company and role.
pub fn slug_for(company: &str, role: &str) -> String {
    let raw = format!("{company}-{role}").to_lowercase();
    let mut result = String::new();
    let mut prev_dash = false;

    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() {
            result.push(ch);
            prev_dash = false;
        } else if !prev_dash && !result.is_empty() {
            result.push('-');
            prev_dash = true;
        }
    }

    let trimmed = result.trim_end_matches('-');
    trimmed.chars().take(48).collect()
}

/// Date of the most recent log entry for an application, or empty if none.
pub fn last_status_date(app: &CvApplication) -> &str {
    app.log.last().map(|l| l.date.as_str()).unwrap_or("")
}

/// Find the built PDF matching a specific profile and language (e.g. `CV_default_en.pdf`).
pub fn pdf_for_profile<'a>(view: &'a CvView, profile: &str, lang: &str) -> Option<&'a CvPdf> {
    let target_name = format!("CV_{profile}_{lang}.pdf");
    view.pdfs.iter().find(|pdf| pdf.name == target_name)
}

/// Count how many built PDFs are marked stale.
pub fn count_stale_pdfs(view: &CvView) -> usize {
    view.pdfs.iter().filter(|p| p.stale).count()
}

/// Filter applications by status.
pub fn filter_applications<'a>(view: &'a CvView, status: &str) -> Vec<&'a CvApplication> {
    view.applications
        .iter()
        .filter(|app| app.status == status)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slug_generation() {
        assert_eq!(
            slug_for("Super44 (via Thyme)", "Founding Engineer"),
            "super44-via-thyme-founding-engineer"
        );
        assert_eq!(slug_for("Acme, Inc.", "Staff SRE"), "acme-inc-staff-sre");
        assert_eq!(slug_for("---Spaces---", "---Role---"), "spaces-role");
    }

    #[test]
    fn test_last_status_date() {
        let app = CvApplication {
            company: "Acme".into(),
            role: "Engineer".into(),
            url: None,
            profile: "swe".into(),
            lang: "en".into(),
            status: "interview".into(),
            log: vec![
                CvLogEntry {
                    date: "2026-10-01".into(),
                    status: "applied".into(),
                    note: None,
                },
                CvLogEntry {
                    date: "2026-10-14".into(),
                    status: "interview".into(),
                    note: Some("Initial screening call".into()),
                },
            ],
            notes: None,
            slug: "acme-engineer".into(),
        };
        assert_eq!(last_status_date(&app), "2026-10-14");
    }

    #[test]
    fn test_pdf_lookup_and_stale_count() {
        let view = CvView {
            master: "/path/to/master.yaml".into(),
            master_mtime: 1792014685,
            profiles: vec!["swe".into(), "default".into()],
            statuses: vec!["applied".into()],
            applications: vec![],
            pdfs: vec![
                CvPdf {
                    name: "CV_swe_en.pdf".into(),
                    path: "/path/dist/CV_swe_en.pdf".into(),
                    mtime: 1792014105,
                    stale: false,
                },
                CvPdf {
                    name: "CV_default_de.pdf".into(),
                    path: "/path/dist/CV_default_de.pdf".into(),
                    mtime: 1792014104,
                    stale: true,
                },
            ],
            linkedin_pulled: Some("2026-10-14".into()),
        };

        assert_eq!(count_stale_pdfs(&view), 1);
        let pdf = pdf_for_profile(&view, "swe", "en");
        assert!(pdf.is_some());
        assert_eq!(pdf.unwrap().name, "CV_swe_en.pdf");
        assert!(!pdf.unwrap().stale);

        assert!(pdf_for_profile(&view, "nonexistent", "en").is_none());
    }

    #[test]
    fn test_deserialization_from_live_cv_output() {
        let raw = r#"{
            "master": "/Users/test/data/cv/master_cv.yaml",
            "master_mtime": 1792014685,
            "profiles": ["consulting", "default", "general"],
            "statuses": ["draft", "applied", "interview", "offer", "accepted", "rejected", "withdrawn"],
            "applications": [
                {
                    "company": "Super44 (via Thyme)",
                    "role": "Founding Engineer",
                    "url": "",
                    "profile": "swe",
                    "lang": "en",
                    "status": "draft",
                    "log": [{"date": "2026-10-14", "status": "draft"}],
                    "notes": "Recruiter: Henry",
                    "slug": "super44-founding-engineer"
                }
            ],
            "pdfs": [
                {
                    "name": "CV_consulting_de.pdf",
                    "path": "/path/dist/CV_consulting_de.pdf",
                    "mtime": 1792014104,
                    "stale": true
                }
            ],
            "linkedin_pulled": "2026-10-14"
        }"#;

        let parsed: CvView = serde_json::from_str(raw).expect("Valid JSON");
        assert_eq!(parsed.profiles.len(), 3);
        assert_eq!(parsed.applications.len(), 1);
        assert_eq!(parsed.applications[0].company, "Super44 (via Thyme)");
        assert_eq!(parsed.pdfs.len(), 1);
        assert!(parsed.pdfs[0].stale);
    }
}
