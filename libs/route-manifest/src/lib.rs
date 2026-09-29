//! What a capability answers, as data it serves about itself.
//!
//! Every capability exposes `GET /routes` beside `/health`. `/health` is the
//! one path all of them already share regardless of their other conventions,
//! and it is reachable whether or not the manifest proxies API paths only — so
//! it is the right neighbour for the endpoint that says what else is here.
//!
//! ## Why this exists
//!
//! Axon's HTTP surface grew five conventions across seven capabilities: `/api/…`
//! behind a proxy, bare paths, self-prefixed `/api/<name>/…`, and transit
//! serving both `/health` and `/api/health`. Renaming all of that is churn with
//! a large blast radius; **not being able to find out what exists** was the
//! part that actually cost time. This fixes that half directly, and leaves the
//! naming free to converge later without a flag day.
//!
//! ## Drift is the whole risk
//!
//! A hand-written list of endpoints is wrong the first time someone adds a
//! route and forgets it — and a stale manifest is worse than none, because it
//! is believed. `undeclared_routes` reads the server's own source at compile
//! time via `include_str!` and reports anything the router serves that the
//! manifest does not mention, so the test fails instead of the manifest lying.
//!
//! Consumers link this as an ordinary crate and serve its JSON as the external
//! contract.

use serde::Serialize;
use serde_json::{json, Value};

/// One endpoint, described for whoever is trying to use it rather than for
/// whoever wrote it.
///
/// No `PartialEq`: `request_schema` is a function pointer, and a derived
/// comparison would compare its address — which Rust does not guarantee to be
/// unique per function, so two routes carrying the same schema could compare
/// unequal and two carrying different ones could compare equal. Nothing ever
/// asked whether two `Route`s were equal (`undeclared_routes` compares `path`,
/// a `&'static str`), so the derive bought nothing and rustc's
/// `unpredictable_function_pointer_comparisons` lint is right to refuse it.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Route {
    pub method: &'static str,
    pub path: &'static str,
    /// One line, in the imperative: what calling this gets you. Query
    /// parameters belong here when they are required — a path alone does not
    /// tell a caller that `from` and `to` are mandatory.
    pub summary: &'static str,
    /// The JSON Schema of the body this route accepts, derived from the struct
    /// serde already deserializes.
    ///
    /// A function pointer rather than a value, because a `Route` is built in a
    /// `const` array and a schema is not const-constructible. `manifest()` calls
    /// it once per request.
    ///
    /// Derived, never hand-written, and deliberately not a `$ref` at a file in
    /// `schemas/`: those describe *stored* shapes, and a write body differs from
    /// what comes back (no id, no timestamps, different optionality). A schema
    /// pointing at a document that does not describe the request is worse than
    /// no schema, because it would be believed. Deriving from the struct means
    /// the manifest cannot drift from the code that parses the body.
    #[serde(skip)]
    pub request_schema: Option<fn() -> Value>,
}

/// A route with no body. Most of them.
pub const fn get(method: &'static str, path: &'static str, summary: &'static str) -> Route {
    Route {
        method,
        path,
        summary,
        request_schema: None,
    }
}

/// The body `GET /routes` returns.
pub fn manifest(capability: &str, routes: &[Route]) -> Value {
    let rendered: Vec<Value> = routes
        .iter()
        .map(|route| {
            let mut entry = json!({
                "method": route.method,
                "path": route.path,
                "summary": route.summary,
            });
            if let Some(schema) = route.request_schema {
                entry["request_schema"] = schema();
            }
            entry
        })
        .collect();
    json!({
        "capability": capability,
        "routes": rendered,
    })
}

/// Helper so a consumer writes `schema_of::<CreatePlan>` and nothing else.
pub fn schema_of<T: schemars::JsonSchema>() -> Value {
    serde_json::to_value(schemars::schema_for!(T)).unwrap_or_else(|_| json!({}))
}

/// Routes that take a body but declare no schema for it.
///
/// An agent discovering that `POST /api/plans/{id}/items` exists still cannot
/// send one without knowing the shape, and a one-line English summary cannot
/// carry it. This is the same drift guard as `undeclared_routes`, one level up:
/// the check fails rather than the manifest staying silent.
pub fn bodies_without_schemas(routes: &[Route]) -> Vec<&'static str> {
    routes
        .iter()
        .filter(|route| matches!(route.method, "POST" | "PUT" | "PATCH"))
        .filter(|route| route.request_schema.is_none())
        .map(|route| route.path)
        .collect()
}

/// Paths the router serves that the manifest does not declare.
///
/// `source` is the server's own text, passed in by the caller as
/// `include_str!("server.rs")` — this lib cannot reach the consumer's files, and
/// having the caller name its own source keeps the check honest about what it
/// actually read.
///
/// Deliberately one-directional. A manifest entry with no matching `.route()`
/// is not reported, because a capability may legitimately describe a path its
/// router mounts indirectly; a *served* path that nobody documented is the
/// failure that leaves a caller guessing.
pub fn undeclared_routes(source: &str, routes: &[Route]) -> Vec<String> {
    let mut missing: Vec<String> = Vec::new();
    for served in served_paths(source) {
        let declared = routes.iter().any(|route| route.path == served);
        if !declared && !missing.iter().any(|path| path == &served) {
            missing.push(served);
        }
    }
    missing.sort();
    missing
}

/// Every path literal passed to `.route(` in the given source.
///
/// Text matching rather than parsing, and that is a deliberate trade: it cannot
/// see a route built from a runtime string, and it says so here rather than
/// pretending to be exhaustive. Every router in this repo passes a literal.
///
/// **Whitespace between `.route(` and the literal is skipped, and that is the
/// whole check, not a detail.** The marker used to be the seven bytes
/// `.route("`, which meant a call rustfmt had wrapped —
///
/// ```text
/// .route(
///     "/api/sjel-status/capabilities/{name}/start",
///     post(start_handler),
/// )
/// ```
///
/// — was invisible. Measured across this repo on 2026-08-31: **22 routes in
/// seven capabilities**, of which three were served and undeclared, including
/// `start` and `stop` on `sjel-status`, the two calls that start and stop every
/// capability on the machine. The formatter decides where a line breaks; a
/// detector that reads differently on either side of that decision is a
/// detector that passes for reasons unrelated to the thing it checks.
///
/// It does not strip comments, and that is a decision rather than an oversight:
/// stripping `//` would truncate any line whose string literal contains one — a
/// URL — and the routes it could then miss are exactly what this exists to
/// catch. So the cost lands on prose instead. A comment that spells the mount
/// call out byte for byte reads as a served path; write about it without
/// quoting it, the way `capabilities/sjel-status/src/main.rs` does at the two
/// mounts that occasioned this.
fn served_paths(source: &str) -> Vec<String> {
    const MARKER: &str = ".route(";
    let mut paths = Vec::new();
    let mut rest = source;
    while let Some(start) = rest.find(MARKER) {
        rest = &rest[start + MARKER.len()..];
        let literal = rest.trim_start();
        // Anything that is not a string literal after the paren is a route built
        // some other way. Skipping it is the documented limit above, not a miss.
        let Some(body) = literal.strip_prefix('"') else {
            continue;
        };
        match body.find('"') {
            Some(end) => {
                paths.push(body[..end].to_string());
                rest = &body[end..];
            }
            // An unterminated literal cannot happen in source that compiles;
            // stopping is still better than looping.
            None => break,
        }
    }
    paths
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bug this crate shipped with until 2026-08-31: rustfmt wraps a long
    /// `.route(...)` call, and the detector stopped seeing it. Both shapes are
    /// the same router; a check that disagrees with itself over a line break
    /// reports nothing and passes.
    #[test]
    fn a_route_wrapped_by_the_formatter_is_still_seen() {
        let einzeilig = r#".route("/health", get(h)).route("/api/things", get(t))"#;
        let umbrochen = r#"
            .route(
                "/health",
                get(h),
            )
            .route(
                "/api/things",
                get(t),
            )
        "#;
        assert_eq!(served_paths(einzeilig), served_paths(umbrochen));
        assert_eq!(served_paths(umbrochen), ["/health", "/api/things"]);

        // Und die Wirkung, um die es geht: eine umbrochene, nicht deklarierte
        // Route wird gemeldet statt uebersehen.
        assert_eq!(
            undeclared_routes(umbrochen, &[get("GET", "/health", "Liveness.")]),
            ["/api/things"]
        );
    }

    /// Not every `.route(` is followed by a literal. A router built from a
    /// runtime value is the documented limit of this check — it must be skipped
    /// without swallowing the calls that come after it.
    #[test]
    fn ein_nicht_literaler_pfad_verschluckt_die_folgenden_nicht() {
        let quelle = r#".route(prefix, get(h)).route("/health", get(h))"#;
        assert_eq!(served_paths(quelle), ["/health"]);
    }

    const ROUTES: &[Route] = &[
        get("GET", "/health", "Liveness."),
        get("GET", "/api/things", "Every thing."),
    ];

    #[derive(serde::Deserialize, schemars::JsonSchema)]
    #[allow(dead_code)]
    struct CreateThing {
        name: String,
        count: Option<u32>,
    }

    /// A route that takes a body and declares no schema is reported, so an agent
    /// discovering the path also learns what to send it.
    #[test]
    fn a_body_route_without_a_schema_is_reported() {
        const WITH_BODY: &[Route] = &[
            get("GET", "/api/things", "Every thing."),
            get("POST", "/api/things", "Create a thing."),
        ];
        assert_eq!(bodies_without_schemas(WITH_BODY), vec!["/api/things"]);

        const DECLARED: &[Route] = &[Route {
            method: "POST",
            path: "/api/things",
            summary: "Create a thing.",
            request_schema: Some(schema_of::<CreateThing>),
        }];
        assert!(bodies_without_schemas(DECLARED).is_empty());

        // A GET is not a body route, so it is never asked for one.
        const READS: &[Route] = &[get("DELETE", "/api/things/{id}", "Remove a thing.")];
        assert!(bodies_without_schemas(READS).is_empty());
    }

    /// The schema is derived from the struct serde parses, so it cannot drift
    /// from the code that reads the body.
    #[test]
    fn a_declared_schema_is_derived_from_the_parsing_struct() {
        const DECLARED: &[Route] = &[Route {
            method: "POST",
            path: "/api/things",
            summary: "Create a thing.",
            request_schema: Some(schema_of::<CreateThing>),
        }];
        let body = manifest("things", DECLARED);
        let schema = &body["routes"][0]["request_schema"];
        assert!(
            schema["properties"]["name"].is_object(),
            "the derived schema must carry the struct's own fields, got: {schema}"
        );
        assert_eq!(
            schema["required"],
            serde_json::json!(["name"]),
            "an Option field is not required; a plain one is"
        );
        // A route with no body carries no schema key at all, rather than null.
        let plain = manifest("things", ROUTES);
        assert!(plain["routes"][0].get("request_schema").is_none());
    }

    #[test]
    fn a_served_path_missing_from_the_manifest_is_reported() {
        let source = r#"
            Router::new()
                .route("/health", get(health))
                .route("/api/things", get(list))
                .route("/api/things/{id}", get(one))
        "#;
        assert_eq!(undeclared_routes(source, ROUTES), vec!["/api/things/{id}"]);
    }

    #[test]
    fn a_fully_declared_router_reports_nothing() {
        let source = r#".route("/health", get(h)).route("/api/things", post(c))"#;
        assert!(undeclared_routes(source, ROUTES).is_empty());
    }

    /// The same path mounted twice (GET and POST chained on one `.route`, or
    /// two builders) must not be reported twice.
    #[test]
    fn a_repeated_path_is_reported_once() {
        let source = r#".route("/a", get(x)).route("/a", post(y)).route("/b", get(z))"#;
        let missing = undeclared_routes(source, ROUTES);
        assert_eq!(missing, vec!["/a", "/b"]);
    }

    #[test]
    fn the_manifest_body_names_the_capability_and_its_routes() {
        let body = manifest("calendar", ROUTES);
        assert_eq!(body["capability"], "calendar");
        assert_eq!(body["routes"].as_array().unwrap().len(), 2);
        assert_eq!(body["routes"][0]["method"], "GET");
        assert_eq!(body["routes"][1]["path"], "/api/things");
        assert_eq!(body["routes"][1]["summary"], "Every thing.");
    }

    #[test]
    fn source_with_no_routes_at_all_is_not_an_error() {
        assert!(undeclared_routes("fn main() {}", ROUTES).is_empty());
        assert!(served_paths("").is_empty());
    }
}
