//! The `graph` plugin: ask the code graph instead of reading files.
//!
//! `capabilities/knowledge-graph` serves the graph graphify builds, ranked and capped. Three
//! tools read it, and the point of all three is the same: `/api/graph/unit/agent` answers where a
//! unit lives and what it depends on in a few hundred tokens, where `grep` plus a handful of
//! reads spends tens of thousands. The graph is a drill-down that costs one call, not a substitute
//! for reading the file you are about to change — so `read` stays the tool for that.
//!
//! Read-only to the last tool, and it declares it, so a turn that only asks the graph runs its
//! calls in parallel.

use serde_json::{json, Value};

use crate::tools::cap;
use crate::{Extension, Tool};

/// The name `<overlay>/config/agent.toml` and `--ext` use.
pub const NAME: &str = "graph";

const PURPOSE: sjel_http::Purpose = sjel_http::Purpose::new("agent-graph");
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
const OUTPUT_LIMIT: usize = 12_000;
/// knowledge-graph's manifest port. An overlay that moved it sets `SJEL_GRAPH_URL`.
const DEFAULT_BASE: &str = "http://127.0.0.1:4244";

pub struct Graph {
    base: String,
}

impl Graph {
    pub fn new(base: impl Into<String>) -> Self {
        Self {
            base: base.into().trim_end_matches('/').to_owned(),
        }
    }

    /// The base URL from `SJEL_GRAPH_URL`, or the manifest port.
    pub fn from_env() -> Self {
        let base = std::env::var("SJEL_GRAPH_URL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_BASE.to_owned());
        Self::new(base)
    }

    /// `path` is already quoted; `segments` go through [`quote`], because a unit name carries
    /// nothing that needs encoding today and a file path will.
    fn get(&self, path: &str) -> Result<String, String> {
        let client =
            sjel_http::client(PURPOSE, TIMEOUT).map_err(|e| format!("http client: {e}"))?;
        let url = format!("{}{path}", self.base);
        let res = client.get(&url).send().map_err(|e| {
            format!(
                "{path}: {e}\n(the graph is served by capabilities/knowledge-graph; if it is off, \
                 `sjel capability start knowledge-graph`)"
            )
        })?;
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

impl Extension for Graph {
    fn name(&self) -> &'static str {
        NAME
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        let graph = self.base.clone();
        vec![
            Box::new(GraphUnit(Graph::new(&graph))),
            Box::new(GraphSearch(Graph::new(&graph))),
            Box::new(GraphNode(Graph::new(&graph))),
            Box::new(GraphStats(Graph::new(&graph))),
        ]
    }
}

/// Percent-encodes anything outside the unreserved set, so a unit name cannot escape its segment.
fn quote(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    for byte in segment.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

struct GraphUnit(Graph);

impl Tool for GraphUnit {
    fn name(&self) -> &'static str {
        "graph_unit"
    }

    fn description(&self) -> &'static str {
        "One unit's files and the edges among them, ranked by degree and capped. A unit is a \
         capability, a lib or an app (`comms`, `agent`, `gpui`). The answer says `total`, \
         `returned` and `truncated`, so a capped list cannot read as the whole unit."
    }

    fn read_only(&self) -> bool {
        true
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "required": ["name"],
            "properties": {
                "name": { "type": "string", "description": "Unit name, as the self-model spells it." }
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
        self.0.get(&format!("/api/graph/unit/{}", quote(name)))
    }
}

struct GraphSearch(Graph);

impl Tool for GraphSearch {
    fn name(&self) -> &'static str {
        "graph_search"
    }

    fn description(&self) -> &'static str {
        "Nodes whose label, file or type contains a term, with the edges among them. Use it to \
         find where a symbol or file lives without reading anything."
    }

    fn read_only(&self) -> bool {
        true
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "required": ["q"],
            "properties": {
                "q": { "type": "string", "description": "A label, file name, or node type." }
            }
        })
    }

    fn run(&self, args: &Value) -> Result<String, String> {
        let q = args
            .get("q")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "missing string argument `q`".to_owned())?;
        self.0.get(&format!("/api/graph/search?q={}", quote(q)))
    }
}

struct GraphNode(Graph);

impl Tool for GraphNode {
    fn name(&self) -> &'static str {
        "graph_node"
    }

    fn description(&self) -> &'static str {
        "One node and every node it connects to. Use it after `graph_search` to see what a file or \
         symbol touches before reading it."
    }

    fn read_only(&self) -> bool {
        true
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "required": ["id"],
            "properties": {
                "id": { "type": "string", "description": "Node id from graph_search or graph_unit." }
            }
        })
    }

    fn run(&self, args: &Value) -> Result<String, String> {
        let id = args
            .get("id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "missing string argument `id`".to_owned())?;
        self.0.get(&format!("/api/graph/node/{}", quote(id)))
    }
}

struct GraphStats(Graph);

impl Tool for GraphStats {
    fn name(&self) -> &'static str {
        "graph_stats"
    }

    fn description(&self) -> &'static str {
        "Node, edge and community counts, and the commit the graph was built at. Read it before \
         trusting a graph answer: a stale graph is the one way these tools mislead."
    }

    fn read_only(&self) -> bool {
        true
    }

    fn parameters(&self) -> Value {
        json!({ "type": "object", "properties": {} })
    }

    fn run(&self, _args: &Value) -> Result<String, String> {
        self.0.get("/api/graph/stats")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_segment_cannot_escape_its_path() {
        assert_eq!(quote("gpui"), "gpui");
        assert_eq!(quote("libs/agent"), "libs%2Fagent");
        assert_eq!(quote("../secrets"), "..%2Fsecrets");
        assert_eq!(quote("a b"), "a%20b");
    }

    #[test]
    fn every_graph_tool_is_read_only() {
        let tools = Graph::new("http://127.0.0.1:4244").tools();
        assert_eq!(
            tools.iter().map(|t| t.name()).collect::<Vec<_>>(),
            ["graph_unit", "graph_search", "graph_node", "graph_stats"]
        );
        assert!(tools.iter().all(|t| t.read_only()));
    }
}
