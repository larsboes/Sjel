//! Data models and pure reductions for CV and career applications (`/cv`).
//!
//! Replaces the dashboard's `/cv` route. Backed by `capabilities/cv/cv` through
//! `GET /api/sjel-status/cv` and `POST /api/sjel-status/cv`.

pub use status_api::{
    count_stale_pdfs, filter_applications, last_status_date, pdf_for_profile, slug_for,
    CvApplication, CvLogEntry, CvPdf, CvView, CvWriteRequest, CvWriteResult,
};

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
