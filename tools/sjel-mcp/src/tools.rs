//! The pure half of the MCP server: which tools one capability offers under the owner's mode,
//! the MCP-legal name for one, and the URL one call becomes.
//!
//! Ported from `tools/sjel-mcp.ts` on 2026-10-04 with the server, and deliberately the half with
//! no I/O in it: every case `tools/sjel-mcp.test.ts` held is a unit test here, and the two parts
//! that must be exactly right — a name MCP will accept, and a URL a capability will route — are
//! testable with no capability running.

use std::collections::HashSet;

use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use serde::Deserialize;
use serde_json::{json, Map, Value};

/// The owner's per-capability mode, from `<overlay>/data/agent-modes.json`.
///
/// It decides which tools are *offered*. The gate enforces the same mode on every call, so this
/// list is a convenience an agent reads, not the boundary.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Off,
    ReadOnly,
    Ask,
    Auto,
}

impl Mode {
    /// An unrecognised string behaves as [`Mode::Auto`], which is what the TypeScript did: it
    /// failed every `===` comparison, leaving the writes offered and unmarked.
    pub fn parse(raw: &str) -> Mode {
        match raw {
            "off" => Mode::Off,
            "read-only" => Mode::ReadOnly,
            "ask" => Mode::Ask,
            _ => Mode::Auto,
        }
    }
}

/// One endpoint, as the capability's own `GET /routes` describes it.
#[derive(Clone, Debug, Deserialize)]
pub struct Route {
    pub method: String,
    pub path: String,
    #[serde(default)]
    pub summary: String,
    /// The JSON Schema of the request body, when the route declares one.
    #[serde(default)]
    pub request_schema: Option<Value>,
}

/// A capability's gate as `<overlay>/data/agent-gates/<name>.json` writes it.
///
/// Only the three fields the tool list needs are read; the file holds more (a registration
/// timestamp, at least), and an unknown field is ignored rather than refused.
#[derive(Clone, Debug, Deserialize)]
pub struct Gate {
    pub capability: String,
    #[serde(default)]
    pub get_writes: Vec<String>,
    #[serde(default)]
    pub confirm: Vec<String>,
}

/// One tool, before it is flattened into MCP's `{name, description, inputSchema}`.
#[derive(Clone, Debug)]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    pub method: String,
    pub path: String,
}

/// The methods that read. A `GET` a capability listed in its gate's `get_writes` is a write
/// anyway, and that is checked separately in [`tools_for`].
const READS: [&str; 2] = ["GET", "HEAD"];

/// Paths every capability serves for itself, and never a tool.
const NOT_TOOLS: [&str; 3] = ["/health", "/ready", "/routes"];

/// MCP allows `[a-zA-Z0-9_-]` up to 64 characters.
const MAX_NAME_CHARS: usize = 64;

/// `comms__get_triage_id_status`: the capability, the verb, and the path as one token.
pub fn tool_name(capability: &str, method: &str, path: &str) -> String {
    let slug: String = path
        .replace(['{', '}'], "")
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join("_");
    let slug: String = slug
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let slug = if slug.is_empty() {
        "root".to_string()
    } else {
        slug
    };
    // The hyphen folding runs over the whole name, capability included, because `-` is legal to
    // MCP but this keeps one tool name one token everywhere it is printed.
    let name = format!("{capability}__{}_{}", method.to_lowercase(), slug).replace('-', "_");
    if name.chars().count() <= MAX_NAME_CHARS {
        return name;
    }
    // Long names keep their start and gain a short digest of the whole, so two stay distinct.
    let mut hash: u32 = 0;
    for ch in name.chars() {
        hash = hash.wrapping_mul(31).wrapping_add(ch as u32);
    }
    let digest = base36(hash);
    let head: String = name.chars().take(55).collect();
    format!("{head}_{}", &digest[..digest.len().min(8)])
}

/// `u32` in base 36, the base the TypeScript hashed into (`hash.toString(36)`).
fn base36(mut value: u32) -> String {
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if value == 0 {
        return "0".to_string();
    }
    let mut out = Vec::new();
    while value > 0 {
        out.push(DIGITS[(value % 36) as usize]);
        value /= 36;
    }
    out.reverse();
    String::from_utf8(out).expect("base-36 digits are ascii")
}

/// The `{name}` segments in a path, in the order they appear.
///
/// The scan is the TypeScript's `/\{([^}]+)\}/g`: at least one character that is not `}`, so
/// `{}` is not a placeholder, and an unclosed brace is skipped rather than fatal.
pub fn placeholders(path: &str) -> Vec<String> {
    braces(path).into_iter().map(|(_, _, name)| name).collect()
}

/// Every `{name}` in `path`, as `(start, end_exclusive, name)` byte offsets.
fn braces(path: &str) -> Vec<(usize, usize, String)> {
    let bytes = path.as_bytes();
    let mut out = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'{' {
            let mut end = at + 1;
            while end < bytes.len() && bytes[end] != b'}' {
                end += 1;
            }
            // `j > i + 1` is the regex's `+`: one character minimum between the braces.
            if end < bytes.len() && end > at + 1 {
                out.push((at, end + 1, path[at + 1..end].to_string()));
                at = end + 1;
                continue;
            }
        }
        at += 1;
    }
    out
}

/// The tools one capability offers under `mode`.
pub fn tools_for(capability: &str, mode: Mode, routes: &[Route], gate: &Gate) -> Vec<Tool> {
    if mode == Mode::Off {
        return Vec::new();
    }
    let get_writes: HashSet<&str> = gate.get_writes.iter().map(String::as_str).collect();
    let confirm: HashSet<&str> = gate.confirm.iter().map(String::as_str).collect();
    let mut tools = Vec::new();
    for route in routes {
        let method = route.method.to_uppercase();
        if NOT_TOOLS.contains(&route.path.as_str()) || route.path.starts_with("/__axon/") {
            continue;
        }
        let key = format!("{method} {}", route.path);
        let read = READS.contains(&method.as_str());
        let write = !read || get_writes.contains(key.as_str());
        if write && mode == Mode::ReadOnly {
            continue;
        }
        // A HEAD is the same read a GET is, with no body to give an agent.
        if method == "HEAD" {
            continue;
        }
        let params = placeholders(&route.path);
        let mut properties = Map::new();
        for param in &params {
            properties.insert(
                param.clone(),
                json!({"type": "string", "description": format!("The {{{param}}} segment.")}),
            );
        }
        properties.insert(
            "query".to_string(),
            json!({
                "type": "object",
                "description": "Query parameters, as strings.",
                "additionalProperties": {"type": "string"}
            }),
        );
        if !read {
            properties.insert(
                "body".to_string(),
                route
                    .request_schema
                    .clone()
                    .unwrap_or_else(|| json!({"type": "object", "description": "The JSON body."})),
            );
        }
        let asks = confirm.contains(key.as_str()) || (write && mode == Mode::Ask);
        let mut notes = vec![if write {
            "Changes state."
        } else {
            "Read only."
        }];
        if asks {
            notes.push("Waits for the owner's Allow before it runs.");
        }
        notes.push(
            "Names and addresses come back as tokens like <SENDER_ab12cd>; pass a token back \
             unchanged to act on that value in this same capability.",
        );
        tools.push(Tool {
            name: tool_name(capability, &method, &route.path),
            description: format!(
                "{capability}: {} ({method} {}) {}",
                route.summary,
                route.path,
                notes.join(" ")
            ),
            input_schema: json!({
                "type": "object",
                "properties": properties,
                "required": params
            }),
            method,
            path: route.path.clone(),
        });
    }
    tools
}

/// `encodeURIComponent`'s set: everything but `A-Za-z0-9-_.!~*'()`.
const COMPONENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'!')
    .remove(b'~')
    .remove(b'*')
    .remove(b'\'')
    .remove(b'(')
    .remove(b')');

/// `URLSearchParams`' set (application/x-www-form-urlencoded): `A-Za-z0-9*-._` stay. A space is
/// not in the set — it becomes `+` in a substitution, because that is a rewrite of the output
/// rather than a character the set can name.
const FORM: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'*')
    .remove(b'-')
    .remove(b'.')
    .remove(b'_');

/// The URL for one call, with each `{name}` filled and percent-encoded.
///
/// A missing or empty path parameter is an error, not an empty segment: the capability would
/// answer 404 and the agent would have no idea which argument it forgot.
pub fn call_url(base: &str, path: &str, args: &Map<String, Value>) -> Result<String, String> {
    let filled = fill_path(path, args)?;
    let query = encode_query(args.get("query"));
    Ok(format!(
        "{base}{filled}{}",
        if query.is_empty() {
            String::new()
        } else {
            format!("?{query}")
        }
    ))
}

fn fill_path(path: &str, args: &Map<String, Value>) -> Result<String, String> {
    let mut out = String::new();
    let mut at = 0;
    for (start, end, name) in braces(path) {
        out.push_str(&path[at..start]);
        match args.get(&name) {
            Some(Value::String(value)) if !value.is_empty() => {
                out.push_str(&utf8_percent_encode(value, COMPONENT).to_string());
            }
            _ => return Err(format!("missing path parameter: {name}")),
        }
        at = end;
    }
    out.push_str(&path[at..]);
    Ok(out)
}

fn encode_query(query: Option<&Value>) -> String {
    let Some(Value::Object(map)) = query else {
        return String::new();
    };
    // Sorted by key, deliberately. `serde_json`'s map is an insertion-ordered `IndexMap`
    // whenever any crate in the build enables `preserve_order`, and `libs/extraction` does —
    // through `xberg`. Iterating it directly therefore made the URL depend on the order a
    // caller happened to build its arguments object in, and made this a workspace-only
    // difference: `-p sjel-mcp` was green while `cargo test --workspace --locked` was red
    // (measured 2026-10-04). `tools/claude-code-config/src/check.rs` sorts its keys for the
    // same reason and states the same trap.
    let mut pairs: Vec<(&String, String)> = map
        .iter()
        .filter(|(_, value)| !value.is_null())
        .map(|(key, value)| (key, value_as_text(value)))
        .collect();
    pairs.sort_by(|a, b| a.0.cmp(b.0));
    pairs
        .into_iter()
        .map(|(key, value)| format!("{}={}", form_encode(key), form_encode(&value)))
        .collect::<Vec<_>>()
        .join("&")
}

fn form_encode(raw: &str) -> String {
    utf8_percent_encode(raw, FORM)
        .to_string()
        .replace("%20", "+")
}

/// `String(value)` for the values a query string or a registry row can hold.
pub fn value_as_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Bool(flag) => flag.to_string(),
        Value::Number(number) => number.to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn routes() -> Vec<Route> {
        serde_json::from_value(json!([
            {"method": "GET", "path": "/health", "summary": "Liveness."},
            {"method": "GET", "path": "/routes", "summary": "This manifest."},
            {"method": "GET", "path": "/triage", "summary": "Mail proposals."},
            {"method": "GET", "path": "/discover", "summary": "Crawl and rank."},
            {"method": "POST", "path": "/triage/{id}/status", "summary": "Set a status."},
            {"method": "POST", "path": "/triage/{id}/gmail", "summary": "Gmail action."}
        ]))
        .unwrap()
    }

    fn gate() -> Gate {
        serde_json::from_value(json!({
            "capability": "comms",
            "get_writes": ["GET /discover"],
            "confirm": ["POST /triage/{id}/gmail"]
        }))
        .unwrap()
    }

    fn names(mode: Mode) -> Vec<String> {
        tools_for("comms", mode, &routes(), &gate())
            .into_iter()
            .map(|tool| tool.name)
            .collect()
    }

    #[test]
    fn off_offers_nothing() {
        assert!(names(Mode::Off).is_empty());
    }

    #[test]
    fn read_only_offers_reads_and_not_a_get_that_writes() {
        assert_eq!(names(Mode::ReadOnly), vec!["comms__get_triage"]);
    }

    #[test]
    fn auto_offers_writes_and_a_confirm_route_says_it_waits() {
        let tools = tools_for("comms", Mode::Auto, &routes(), &gate());
        assert_eq!(
            tools.iter().map(|t| t.name.clone()).collect::<Vec<_>>(),
            vec![
                "comms__get_triage",
                "comms__get_discover",
                "comms__post_triage_id_status",
                "comms__post_triage_id_gmail",
            ]
        );
        assert!(tools[3].description.contains("Waits for the owner"));
        assert!(!tools[2].description.contains("Waits for the owner"));
        assert_eq!(tools[2].input_schema["required"], json!(["id"]));
    }

    #[test]
    fn ask_marks_every_write_as_waiting() {
        let tools = tools_for("comms", Mode::Ask, &routes(), &gate());
        let status = tools
            .iter()
            .find(|t| t.name == "comms__post_triage_id_status")
            .unwrap();
        assert!(status.description.contains("Waits for the owner"));
        let read = tools
            .iter()
            .find(|t| t.name == "comms__get_triage")
            .unwrap();
        assert!(!read.description.contains("Waits for the owner"));
    }

    #[test]
    fn a_name_is_mcp_legal_and_at_most_64_characters() {
        let long = tool_name(
            "entities-google-sync",
            "POST",
            "/api/a-very/long/{path}/that/goes/on/and/on/and/on/forever",
        );
        assert!(long.chars().count() <= 64, "{long}");
        assert!(long
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'));
        assert_eq!(
            tool_name("trips", "GET", "/api/plans"),
            "trips__get_api_plans"
        );
    }

    #[test]
    fn two_long_names_that_share_a_start_stay_distinct() {
        let one = tool_name(
            "entities",
            "GET",
            "/api/one/very/long/path/that/keeps/going/on/and/on",
        );
        let two = tool_name(
            "entities",
            "GET",
            "/api/one/very/long/path/that/keeps/going/on/and/off",
        );
        assert_ne!(one, two);
    }

    #[test]
    fn a_placeholder_is_at_least_one_character_between_braces() {
        assert_eq!(placeholders("/triage/{id}/status"), vec!["id"]);
        assert!(placeholders("{}").is_empty());
        assert_eq!(placeholders("/a/{}/b/{id}"), vec!["id"]);
    }

    #[test]
    fn path_parameters_are_filled_and_encoded_and_the_query_is_appended() {
        let args = json!({"id": "a/b", "query": {"limit": 5}});
        let args = args.as_object().unwrap();
        assert_eq!(
            call_url("http://127.0.0.1:8083", "/triage/{id}/status", args).unwrap(),
            "http://127.0.0.1:8083/triage/a%2Fb/status?limit=5"
        );
        let missing = json!({});
        assert_eq!(
            call_url("http://x", "/triage/{id}", missing.as_object().unwrap()),
            Err("missing path parameter: id".to_string())
        );
        let empty = json!({"id": ""});
        assert!(call_url("http://x", "/triage/{id}", empty.as_object().unwrap()).is_err());
    }

    #[test]
    fn a_query_space_is_a_plus_and_a_path_space_is_percent_20() {
        let args = json!({"id": "a b", "query": {"q": "a b", "keep": "*-._"}});
        let args = args.as_object().unwrap();
        assert_eq!(
            call_url("http://x", "/t/{id}", args).unwrap(),
            "http://x/t/a%20b?keep=*-._&q=a+b"
        );
    }

    /// The map is an `IndexMap` whenever any crate in the build enables `preserve_order`
    /// (`libs/extraction` -> `xberg`), so the same arguments built in a different order must
    /// still produce one URL. Without the sort this passes under `-p sjel-mcp` and fails under
    /// `cargo test --workspace --locked`, which is how it was found on 2026-10-04.
    #[test]
    fn the_query_does_not_depend_on_the_order_the_arguments_were_built_in() {
        let one = json!({"id": "x", "query": {"q": "a b", "keep": "*-._"}});
        let other = json!({"id": "x", "query": {"keep": "*-._", "q": "a b"}});
        assert_eq!(
            call_url("http://x", "/t/{id}", one.as_object().unwrap()),
            call_url("http://x", "/t/{id}", other.as_object().unwrap())
        );
    }
}
