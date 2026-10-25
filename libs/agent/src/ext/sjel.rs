//! The `sjel` plugin: read-only answers about Sjel's own data, through the shell proxy.
//!
//! Lifted from `capabilities/assistant/src/main.rs`, where it was written for the assistant's
//! read-only loop. Both front ends pass the same five tools, so the code lives here and each
//! caller supplies the base URL and the credential: a capability holds the deployment token
//! (`sjel_server::InboundAuth`), and this crate must not depend on `sjel-server` to reach it.
//!
//! Nothing here includes `bash`, a file tool or a write. The assistant's rule — that the coding
//! tools never reach it — is enforced by what this module returns, not by a comment.

use std::time::Duration;

use serde_json::{json, Value};

use crate::tools::cap;
use crate::Tool;

/// The name `<overlay>/config/agent.toml` and `--ext` use.
pub const NAME: &str = "sjel";

const PURPOSE: sjel_http::Purpose = sjel_http::Purpose::new("agent-sjel");
const TIMEOUT: Duration = Duration::from_secs(10);
const OUTPUT_LIMIT: usize = 16_000;

/// The five read-only Sjel tools, talking to `base` (the shell proxy) with an optional
/// `Authorization` value.
pub struct SjelTools {
    base: String,
    bearer: Option<String>,
}

impl SjelTools {
    pub fn new(base: impl Into<String>, bearer: Option<String>) -> Self {
        Self {
            base: base.into().trim_end_matches('/').to_owned(),
            bearer,
        }
    }

    fn bridge(&self) -> ShellBridge {
        ShellBridge {
            base: self.base.clone(),
            bearer: self.bearer.clone(),
        }
    }
}

impl crate::Extension for SjelTools {
    fn name(&self) -> &'static str {
        NAME
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        let bridge = self.bridge();
        vec![
            Box::new(SjelHealthTool(bridge.clone())),
            Box::new(CalendarEntriesTool(bridge.clone())),
            Box::new(ProjectsListTool(bridge.clone())),
            Box::new(ProjectRecordsTool(bridge.clone())),
            Box::new(TripsPlansTool(bridge)),
        ]
    }
}

/// Read-only HTTP through the local shell proxy.
#[derive(Clone)]
struct ShellBridge {
    base: String,
    bearer: Option<String>,
}

impl ShellBridge {
    fn get(&self, path: &str) -> Result<String, String> {
        let client = sjel_http::client(PURPOSE, TIMEOUT)
            .map_err(|e| format!("http client: {e}"))?;
        let url = format!("{}{path}", self.base);
        let mut req = client.get(&url);
        if let Some(auth) = &self.bearer {
            req = req.header("authorization", auth);
        }
        let res = req.send().map_err(|e| format!("{path}: {e}"))?;
        let status = res.status();
        let body = res.text().map_err(|e| format!("{path}: {e}"))?;
        if !status.is_success() {
            return Err(format!(
                "{path} returned HTTP {}: {}",
                status.as_u16(),
                cap(&body, 512)
            ));
        }
        Ok(cap(&body, OUTPUT_LIMIT))
    }
}

struct SjelHealthTool(ShellBridge);

impl Tool for SjelHealthTool {
    fn name(&self) -> &'static str {
        "sjel_health"
    }

    fn description(&self) -> &'static str {
        "Read the health, version, uptime, and liveness of all enabled Sjel capabilities on this Mac."
    }

    fn read_only(&self) -> bool {
        true
    }

    fn parameters(&self) -> Value {
        json!({ "type": "object", "properties": {} })
    }

    fn run(&self, _args: &Value) -> Result<String, String> {
        self.0.get("/api/sjel-status/health")
    }
}

struct CalendarEntriesTool(ShellBridge);

impl Tool for CalendarEntriesTool {
    fn name(&self) -> &'static str {
        "calendar_entries"
    }

    fn description(&self) -> &'static str {
        "Read calendar entries between `from` and `to` (inclusive, YYYY-MM-DD)."
    }

    fn read_only(&self) -> bool {
        true
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "required": ["from", "to"],
            "properties": {
                "from": { "type": "string", "description": "Start date in YYYY-MM-DD format." },
                "to": { "type": "string", "description": "End date in YYYY-MM-DD format." }
            }
        })
    }

    fn run(&self, args: &Value) -> Result<String, String> {
        let from = args
            .get("from")
            .and_then(Value::as_str)
            .ok_or_else(|| "missing string argument `from`".to_owned())?;
        let to = args
            .get("to")
            .and_then(Value::as_str)
            .ok_or_else(|| "missing string argument `to`".to_owned())?;
        if !is_iso_date(from) || !is_iso_date(to) {
            return Err("`from` and `to` must be YYYY-MM-DD dates".into());
        }
        self.0
            .get(&format!("/calendar/api/entries?from={from}&to={to}"))
    }
}

struct ProjectsListTool(ShellBridge);

impl Tool for ProjectsListTool {
    fn name(&self) -> &'static str {
        "projects_list"
    }

    fn description(&self) -> &'static str {
        "List all Sjel projects and their note counts."
    }

    fn read_only(&self) -> bool {
        true
    }

    fn parameters(&self) -> Value {
        json!({ "type": "object", "properties": {} })
    }

    fn run(&self, _args: &Value) -> Result<String, String> {
        self.0.get("/projects/api/projects")
    }
}

struct ProjectRecordsTool(ShellBridge);

impl Tool for ProjectRecordsTool {
    fn name(&self) -> &'static str {
        "project_records"
    }

    fn description(&self) -> &'static str {
        "Read the tasks and notes inside one Sjel project by its name."
    }

    fn read_only(&self) -> bool {
        true
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "required": ["name"],
            "properties": {
                "name": { "type": "string", "description": "Project name from projects_list." }
            }
        })
    }

    fn run(&self, args: &Value) -> Result<String, String> {
        let name = args
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "missing string argument `name`".to_owned())?;
        let mut url = reqwest::Url::parse("http://127.0.0.1/projects/api/project")
            .map_err(|e| e.to_string())?;
        url.query_pairs_mut().append_pair("name", name);
        let path = format!("{}?{}", url.path(), url.query().unwrap_or_default());
        self.0.get(&path)
    }
}

struct TripsPlansTool(ShellBridge);

impl Tool for TripsPlansTool {
    fn name(&self) -> &'static str {
        "trips_plans"
    }

    fn description(&self) -> &'static str {
        "Read saved and upcoming trip plans from the trips capability."
    }

    fn read_only(&self) -> bool {
        true
    }

    fn parameters(&self) -> Value {
        json!({ "type": "object", "properties": {} })
    }

    fn run(&self, _args: &Value) -> Result<String, String> {
        self.0.get("/trips/api/plans")
    }
}

fn is_iso_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Extension as _;

    #[test]
    fn the_tool_set_never_carries_a_write_or_a_shell() {
        let tools = SjelTools::new("http://127.0.0.1:8082", None).tools();
        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert_eq!(
            names,
            [
                "sjel_health",
                "calendar_entries",
                "projects_list",
                "project_records",
                "trips_plans"
            ]
        );
        assert!(
            tools.iter().all(|t| t.read_only()),
            "every sjel tool must declare itself read-only, or a turn cannot run them in parallel \
             and the assistant would be offered a write"
        );
    }

    #[test]
    fn a_date_is_only_a_date() {
        assert!(is_iso_date("2026-10-25"));
        for bad in ["2026-1-25", "20261025", "2026-10-25T00:00", ""] {
            assert!(!is_iso_date(bad), "`{bad}` was accepted");
        }
    }
}
