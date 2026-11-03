//! Link newly swept mail to the Projects tasks that wait for it. No model.
//!
//! A task opts in with a `watch` property: comma-separated exact matchers.
//!
//! - `miele.de` (has a dot, no space): the sender's domain or a subdomain of it.
//! - `name@example.com`: that sender address.
//! - a Gmail thread id: that thread.
//! - anything else, at least 4 characters: a case-insensitive substring of the subject, for an
//!   order or item number.
//!
//! Every sweep pass matches the newest stored threads (see [`link_recent`]). On a match comms
//! appends one dated line to the task, with the Gmail link, and moves a
//! `waiting` task to `open`. The link is the idempotency key: a thread already linked in the
//! task is not written again, so a re-run or a manual sweep over the same page is a no-op.
//! A delivery subject ("zugestellt", "delivered") is marked as one. c3 mail never crosses; a c2
//! mail crosses without its subject.

use serde_json::{json, Value};

use crate::{
    config::Config,
    store::{Store, TriageItem},
};

/// Subject or preview terms a carrier or marketplace uses once a parcel is handed over.
/// Lowercase. A shipping notice ("versandt", "unterwegs") is deliberately absent.
const DELIVERED_TERMS: &[&str] = &[
    "zugestellt",
    "abgegeben am",
    "delivered",
    "sendung wurde zugestellt",
    "paket ist da",
];

const MIN_SUBJECT_TERM: usize = 4;

/// How many of the newest stored threads each pass matches. About two weeks of this inbox.
const RECENT_MAIL: usize = 300;

#[derive(Debug, Default, serde::Serialize)]
pub struct LinkReport {
    pub watching: usize,
    pub linked: usize,
    pub reopened: usize,
    pub failed: usize,
}

struct Watch {
    path: String,
    status: String,
    terms: Vec<String>,
}

fn terms(raw: &str) -> Vec<String> {
    raw.trim_matches(|c| c == '[' || c == ']')
        .split(',')
        .map(|t| t.trim().trim_matches('"').to_lowercase())
        .filter(|t| !t.is_empty())
        .collect()
}

fn matches(term: &str, item: &TriageItem) -> bool {
    let from = item.from_addr.as_deref().unwrap_or_default().to_lowercase();
    if term == item.id.to_lowercase() {
        return true;
    }
    if term.contains('@') {
        return from.contains(term);
    }
    if term.contains('.') && !term.contains(' ') {
        return crate::mail_model::sender_domain(&from)
            .is_some_and(|d| d == term || d.ends_with(&format!(".{term}")));
    }
    term.chars().count() >= MIN_SUBJECT_TERM
        && item
            .subject
            .as_deref()
            .unwrap_or_default()
            .to_lowercase()
            .contains(term)
}

fn is_delivery(item: &TriageItem) -> bool {
    let text = format!(
        "{} {}",
        item.subject.as_deref().unwrap_or_default(),
        item.snippet.as_deref().unwrap_or_default()
    )
    .to_lowercase();
    DELIVERED_TERMS.iter().any(|t| text.contains(t))
}

fn gmail_link(id: &str) -> String {
    format!("https://mail.google.com/mail/#all/{id}")
}

/// The line appended to the task. `None` for c3, which never leaves comms.
fn line(item: &TriageItem) -> Option<String> {
    if item.data_class == "c3" {
        return None;
    }
    let day = item
        .internal_date_text
        .as_deref()
        .map(|d| d.chars().take(10).collect::<String>())
        .unwrap_or_default();
    let from = item
        .from_addr
        .as_deref()
        .and_then(crate::mail_model::sender_domain)
        .unwrap_or_else(|| "unknown sender".into());
    let what = if is_delivery(item) {
        "Delivered"
    } else {
        "Mail"
    };
    let subject = match (item.data_class.as_str(), item.subject.as_deref()) {
        ("c2", _) | (_, None) => String::new(),
        (_, Some(s)) => format!(": \"{}\"", s.replace('"', "'")),
    };
    Some(format!(
        "- {day} {what} from {from}{subject} ([mail]({})) — linked by Sjel",
        gmail_link(&item.id)
    ))
}

/// The content with `line` appended under a `## Mail` heading, created when absent.
fn append(content: &str, line: &str) -> String {
    let mut out = content.trim_end().to_string();
    if !out.lines().any(|l| l.trim() == "## Mail") {
        out.push_str("\n\n## Mail\n");
    }
    out.push('\n');
    out.push_str(line);
    out.push('\n');
    out
}

struct Projects<'a> {
    cfg: &'a Config,
    client: reqwest::blocking::Client,
}

impl Projects<'_> {
    fn url(&self, path: &str) -> String {
        format!(
            "{}{path}",
            self.cfg.projects_context.base_url.trim_end_matches('/')
        )
    }

    fn get(&self, path: &str, query: &[(&str, &str)]) -> Result<Value, String> {
        let url = self.url(path);
        let response =
            sjel_server::InboundAuth::with_loopback_auth(self.client.get(&url).query(query), &url)
                .send()
                .map_err(|_| format!("Projects did not answer {path}"))?;
        if !response.status().is_success() {
            return Err(format!(
                "Projects refused {path} (HTTP {})",
                response.status()
            ));
        }
        response
            .json()
            .map_err(|_| format!("Projects answered {path} without JSON"))
    }

    fn post(&self, path: &str, body: &Value) -> Result<(), String> {
        let url = self.url(path);
        let response =
            sjel_server::InboundAuth::with_loopback_auth(self.client.post(&url).json(body), &url)
                .send()
                .map_err(|_| format!("Projects did not answer {path}"))?;
        if !response.status().is_success() {
            return Err(format!(
                "Projects refused {path} (HTTP {})",
                response.status()
            ));
        }
        Ok(())
    }

    fn watches(&self) -> Result<Vec<Watch>, String> {
        let body = self.get("/api/records", &[("kind", "task")])?;
        let rows = body
            .get("records")
            .or_else(|| body.get("tasks"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        Ok(rows
            .iter()
            .filter(|r| r.get("done").and_then(Value::as_bool) != Some(true))
            .filter_map(|r| {
                let fields = r.get("fields")?;
                let raw = fields.get("watch").and_then(Value::as_str)?;
                let terms = terms(raw);
                let path = r.get("id").and_then(Value::as_str)?.to_string();
                (!terms.is_empty()).then(|| Watch {
                    path,
                    status: fields
                        .get("status")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    terms,
                })
            })
            .collect())
    }

    /// Appends `line` unless the task already links this thread. Returns whether it wrote.
    fn link(&self, watch: &Watch, item: &TriageItem, line: &str) -> Result<bool, String> {
        let record = self.get("/api/record", &[("path", &watch.path)])?;
        let content = record
            .pointer("/record/content")
            .and_then(Value::as_str)
            .ok_or("Projects answered a record without content")?;
        if content.contains(&gmail_link(&item.id)) {
            return Ok(false);
        }
        self.post(
            "/api/record",
            &json!({"path": watch.path, "content": append(content, line), "base": content}),
        )?;
        Ok(true)
    }
}

/// Match the newest [`RECENT_MAIL`] stored threads against every open task that carries a
/// `watch`.
///
/// Stored mail, not the page a sweep just fetched. The sweep reads only the newest page, so
/// mail that arrived during an outage, and mail older than a newly added `watch`, are never in
/// it. On 2026-10-23 the eBay delivery for the drain hose task was three days old and outside
/// the 42-thread page. A local read costs no Gmail call; a thread already linked is skipped.
pub fn link_recent(cfg: &Config) -> LinkReport {
    let mut report = LinkReport::default();
    let Ok(client) = sjel_http::client(
        sjel_http::Purpose::new("comms-task-links"),
        std::time::Duration::from_millis(cfg.projects_context.timeout_ms.max(1_000)),
    ) else {
        report.failed = 1;
        return report;
    };
    let projects = Projects { cfg, client };
    let watches = match projects.watches() {
        Ok(w) => w,
        Err(reason) => {
            eprintln!("task links: {reason}");
            report.failed = 1;
            return report;
        }
    };
    report.watching = watches.len();
    if watches.is_empty() {
        return report;
    }
    let Ok(store) = Store::open(&cfg.database_path) else {
        report.failed = 1;
        return report;
    };
    let Ok(items) = store.list_triage(None) else {
        report.failed = 1;
        return report;
    };
    for item in items.into_iter().take(RECENT_MAIL) {
        let Some(line) = line(&item) else { continue };
        for watch in &watches {
            if !watch.terms.iter().any(|t| matches(t, &item)) {
                continue;
            }
            match projects.link(watch, &item, &line) {
                Ok(false) => {}
                Ok(true) => {
                    report.linked += 1;
                    if watch.status == "waiting" {
                        match projects.post(
                            "/api/record/property",
                            &json!({"path": watch.path, "key": "status", "value": "open"}),
                        ) {
                            Ok(()) => report.reopened += 1,
                            Err(reason) => {
                                eprintln!("task links: {reason}");
                                report.failed += 1;
                            }
                        }
                    }
                }
                Err(reason) => {
                    eprintln!("task links: {reason}");
                    report.failed += 1;
                }
            }
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(from: &str, subject: &str, class: &str) -> TriageItem {
        TriageItem {
            id: "1a15954fa689d5cc".into(),
            from_addr: Some(from.into()),
            subject: Some(subject.into()),
            snippet: Some("Abgegeben am Di, 20. Okt".into()),
            internal_date_text: Some("2026-10-20 12:41:43+00".into()),
            internal_date_ms: None,
            stream: "aktiv".into(),
            rationale: String::new(),
            classification_method: "rules".into(),
            classification_version: String::new(),
            data_class: class.into(),
            data_class_rationale: String::new(),
            data_classification_method: "rules".into(),
            data_classification_version: String::new(),
            status: "proposed".into(),
            gmail_action: None,
            gmail_action_at: None,
            purge_after: None,
            gmail_location: None,
            gmail_observed_at: None,
            gmail_sync_status: None,
            gmail_sync_action: None,
            gmail_sync_error: None,
            waiting: false,
            waiting_since: None,
            first_seen: String::new(),
            last_seen: String::new(),
        }
    }

    #[test]
    fn each_matcher_kind_matches_only_its_own_shape() {
        let mail = item(
            "eBay <ebay@ebay.com>",
            "PAKET ZUGESTELLT: Schlauch 1995932",
            "c1",
        );
        for hit in [
            "ebay.com",
            "ebay@ebay.com",
            "1995932",
            "1a15954fa689d5cc",
            "zugestellt",
        ] {
            assert!(terms(hit).iter().all(|t| matches(t, &mail)), "{hit}");
        }
        for miss in ["miele.de", "com", "bay.com", "x@ebay.com", "199", "Miele"] {
            assert!(!terms(miss).iter().any(|t| matches(t, &mail)), "{miss}");
        }
        assert_eq!(terms("[miele.de, \"1995932\"]"), ["miele.de", "1995932"]);
    }

    #[test]
    fn the_line_marks_a_delivery_and_keeps_c2_subjects_and_c3_mail_out() {
        let line = line(&item("eBay <ebay@ebay.com>", "PAKET ZUGESTELLT", "c1")).unwrap();
        assert!(line.starts_with("- 2026-10-20 Delivered from ebay.com: \"PAKET ZUGESTELLT\""));
        assert!(line.contains(&gmail_link("1a15954fa689d5cc")));
        let c2 = super::line(&item("Praxis <t@praxis.de>", "Befund", "c2")).unwrap();
        assert!(!c2.contains("Befund"), "{c2}");
        assert!(super::line(&item("a@b.de", "reset code", "c3")).is_none());
    }

    #[test]
    fn append_adds_one_mail_section() {
        let once = append("---\nstatus: waiting\n---\n\n## Details\n", "- a");
        let twice = append(&once, "- b");
        assert_eq!(twice.matches("## Mail").count(), 1);
        assert!(twice.ends_with("- a\n- b\n"), "{twice}");
    }
}
