//! Projects object discovery derived from the classified read contracts.

use crate::reads::*;
use route_manifest::{registry::Registry, response_schema_of, ResponseSchema};
use serde_json::json;

/// Success schemas used by discovery and the HTTP handlers.
pub const READ_RESPONSES: &[ResponseSchema] = &[
    ResponseSchema {
        method: "GET",
        path: crate::ROUTE_PROJECTS,
        schema: response_schema_of::<ProjectListResponse>,
    },
    ResponseSchema {
        method: "GET",
        path: crate::ROUTE_PROJECT,
        schema: response_schema_of::<ProjectReadResponse>,
    },
    ResponseSchema {
        method: "GET",
        path: crate::ROUTE_RECORD,
        schema: response_schema_of::<RecordReadResponse>,
    },
];

/// The first object projections; identity follows stored paths and summary grouping.
pub fn declaration() -> Registry {
    serde_json::from_value(json!({
        "capability": "projects",
        "objects": [
            {
                "id": "projects.project", "label": "Projects",
                "schema": response_schema_of::<ClassifiedProjectSummary>(),
                "identity": ["name", "archived"], "title": "name", "inactive_property": "archived",
                "classification": { "field": "data_class", "inheritance": "all_properties", "derivation": "Maximum class of non-domain records grouped by name and archived; all properties inherit it." },
                "reads": [{"id": "list", "method": "GET", "path": crate::ROUTE_PROJECTS, "member": "projects", "many": true, "parameters": []}],
                "links": [{"label": "Records", "target": "projects.record", "read": "project", "parameters": {"name": "name"}, "equals": {"archived": "archived"}}],
                "actions": []
            },
            {
                "id": "projects.record", "label": "Records",
                "schema": response_schema_of::<RecordRead>(),
                "identity": ["path"], "title": "title", "inactive_property": "archived",
                "classification": {"field": "data_class", "inheritance": "all_properties", "derivation": "Vault path default raised by record frontmatter; all properties inherit it. updated_at is a row timestamp, not property provenance."},
                "reads": [
                    {"id": "project", "method": "GET", "path": crate::ROUTE_PROJECT, "member": "records", "many": true, "parameters": ["name"], "fixed_parameters": {"content": "false"}},
                    {"id": "detail", "method": "GET", "path": crate::ROUTE_RECORD, "member": "record", "many": false, "parameters": ["path"]}
                ],
                "links": [{"label": "Record", "target": "projects.record", "read": "detail", "parameters": {"path": "path"}, "equals": {}}],
                "actions": [
                    {"method": "POST", "path": crate::ROUTE_RECORD_PROPERTY, "governance": "capability-policy"},
                    {"method": "POST", "path": crate::ROUTE_RECORD, "governance": "capability-policy"}
                ]
            }
        ]
    })).expect("static Projects declaration matches registry types")
}
