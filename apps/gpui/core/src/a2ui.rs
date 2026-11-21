//! Rust validator, saved-view persistence, and constrained client state for A2UI-subset view
//! documents.
//!
//! Owns the Sjel A2UI schema, registry read-route admission, cycle and size bounds, data-class
//! tags, inert-text sanitization, and proposal-only action intents so both desktop and phone render
//! validated live and saved views without executing model code.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Supported schema version for saved and live A2UI documents.
pub const A2UI_SCHEMA_VERSION: u32 = 1;

/// Canonical Sjel component catalog identifier.
pub const SJEL_CATALOG_ID: &str = "sjel.gpui.catalog.v1";

/// Maximum serialized byte size of an A2UI document.
pub const MAX_DOCUMENT_BYTES: usize = 65_536;

/// Maximum number of component nodes in one document.
pub const MAX_NODES: usize = 64;

/// Maximum tree depth from root to any leaf node.
pub const MAX_DEPTH: usize = 16;

/// Approved read-only capability routes that an A2UI view may bind to.
pub const APPROVED_READ_ROUTES: &[&str] = &[
    "/api/projects",
    "/api/projects/tasks",
    "/api/projects/notes",
    "/api/calendar/entries",
    "/api/inbox",
    "/api/mail/proposals",
    "/api/people",
    "/api/places",
    "/api/finance/dashboard",
    "/api/inventory/rows",
    "/api/inventory/wishlist",
    "/api/travel/trips",
    "/api/systems/overview",
];

/// Approved proposal intents that an A2UI button or form may emit.
/// Every mutation is a proposal evaluated by the Rust capability owner in owner mode.
pub const APPROVED_PROPOSAL_INTENTS: &[&str] = &[
    "proposal:create_task",
    "proposal:update_task",
    "proposal:append_note_evidence",
    "proposal:triage_inbox",
    "proposal:archive_mail",
    "proposal:review_finance_candidate",
];

/// Data classification boundary attached to a view document and its bound read routes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DataClass {
    #[default]
    C0,
    C1,
    C2,
    C3,
}

impl DataClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::C0 => "c0",
            Self::C1 => "c1",
            Self::C2 => "c2",
            Self::C3 => "c3",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "c0" => Some(Self::C0),
            "c1" => Some(Self::C1),
            "c2" => Some(Self::C2),
            "c3" => Some(Self::C3),
            _ => None,
        }
    }
}

/// Supported component kinds in Sjel's A2UI catalog subset.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ComponentKind {
    Card,
    MetricStrip,
    MetricTile,
    DescriptionList,
    DescriptionItem,
    DataTable,
    Badge,
    Text,
    Form,
    TextField,
    Button,
}

impl ComponentKind {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "Card" => Some(Self::Card),
            "MetricStrip" => Some(Self::MetricStrip),
            "MetricTile" => Some(Self::MetricTile),
            "DescriptionList" => Some(Self::DescriptionList),
            "DescriptionItem" => Some(Self::DescriptionItem),
            "DataTable" => Some(Self::DataTable),
            "Badge" => Some(Self::Badge),
            "Text" => Some(Self::Text),
            "Form" => Some(Self::Form),
            "TextField" => Some(Self::TextField),
            "Button" => Some(Self::Button),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Card => "Card",
            Self::MetricStrip => "MetricStrip",
            Self::MetricTile => "MetricTile",
            Self::DescriptionList => "DescriptionList",
            Self::DescriptionItem => "DescriptionItem",
            Self::DataTable => "DataTable",
            Self::Badge => "Badge",
            Self::Text => "Text",
            Self::Form => "Form",
            Self::TextField => "TextField",
            Self::Button => "Button",
        }
    }
}

/// Column definition for a `DataTable` node.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct A2uiColumn {
    pub key: String,
    pub label: String,
}

/// Raw unvalidated A2UI node inside a view document.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct A2uiNode {
    pub id: String,
    #[serde(rename = "type")]
    pub component_type: String,
    #[serde(default)]
    pub props: BTreeMap<String, Value>,
    #[serde(default)]
    pub children: Vec<String>,
    #[serde(default)]
    pub bind: Option<String>,
}

/// Raw unvalidated A2UI document.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct A2uiDocument {
    #[serde(default = "default_schema_version")]
    pub version: u32,
    #[serde(default = "default_catalog_id")]
    pub catalog_id: String,
    pub surface_id: String,
    pub title: String,
    pub root: String,
    #[serde(default)]
    pub data_class: String,
    #[serde(default)]
    pub read_routes: Vec<String>,
    #[serde(default)]
    pub nodes: Vec<A2uiNode>,
}

fn default_schema_version() -> u32 {
    A2UI_SCHEMA_VERSION
}

fn default_catalog_id() -> String {
    SJEL_CATALOG_ID.to_owned()
}

/// Validation failure category.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum A2uiErrorKind {
    UnsupportedVersion,
    UnknownCatalog,
    DocumentOversized,
    MissingRoot,
    DuplicateNodeId,
    UnknownChildNode,
    UnknownComponent,
    InvalidProperty,
    DisallowedRoute,
    DisallowedWritePath,
    CycleDetected,
    DepthExceeded,
    UnboundNode,
    InvalidDataClass,
}

/// Structured validation error that always names the failing node when applicable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct A2uiValidationError {
    pub kind: A2uiErrorKind,
    pub node_id: Option<String>,
    pub message: String,
}

impl std::fmt::Display for A2uiValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.node_id {
            Some(id) => write!(f, "node '{id}': {}", self.message),
            None => write!(f, "{}", self.message),
        }
    }
}

impl std::error::Error for A2uiValidationError {}

/// Validated node ready for inert GPUI rendering.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidatedNode {
    pub id: String,
    pub kind: ComponentKind,
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub label: Option<String>,
    pub value: Option<String>,
    pub detail: Option<String>,
    pub tone: Option<String>,
    pub field: Option<String>,
    pub proposal_intent: Option<String>,
    pub columns: Vec<A2uiColumn>,
    pub rows: Vec<BTreeMap<String, String>>,
    pub bind_route: Option<String>,
    pub bind_pointer: Option<String>,
    pub children: Vec<String>,
}

/// Validated A2UI document guaranteed to use catalog components, read-only routes, acyclic graph,
/// and inert escaped text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidatedA2uiView {
    pub version: u32,
    pub catalog_id: String,
    pub surface_id: String,
    pub title: String,
    pub root: String,
    pub data_class: DataClass,
    pub read_routes: Vec<String>,
    pub nodes: BTreeMap<String, ValidatedNode>,
    pub order: Vec<String>,
}

/// Saved view envelope that reloads without a model call and re-runs schema, route, and data-class
/// validation on load.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedA2uiView {
    pub saved_id: String,
    pub document: A2uiDocument,
}

impl SavedA2uiView {
    pub fn new(
        saved_id: impl Into<String>,
        document: A2uiDocument,
    ) -> Result<Self, A2uiValidationError> {
        let _ = validate_document(&document)?;
        Ok(Self {
            saved_id: saved_id.into(),
            document,
        })
    }

    pub fn to_json(&self) -> Result<String, String> {
        serde_json::to_string_pretty(self).map_err(|e| e.to_string())
    }

    /// Reloads and re-validates a saved view from JSON without invoking a model, enforcing the
    /// maximum admitted data class for the current surface.
    pub fn reload_from_json(
        raw: &str,
        max_admitted_class: DataClass,
    ) -> Result<(String, ValidatedA2uiView), A2uiValidationError> {
        if raw.len() > MAX_DOCUMENT_BYTES {
            return Err(A2uiValidationError {
                kind: A2uiErrorKind::DocumentOversized,
                node_id: None,
                message: format!(
                    "saved view JSON ({} bytes) exceeds maximum ({MAX_DOCUMENT_BYTES} bytes)",
                    raw.len()
                ),
            });
        }
        let saved: Self = serde_json::from_str(raw).map_err(|err| A2uiValidationError {
            kind: A2uiErrorKind::InvalidProperty,
            node_id: None,
            message: format!("invalid saved view JSON: {err}"),
        })?;
        let view = validate_document(&saved.document)?;
        if view.data_class > max_admitted_class {
            return Err(A2uiValidationError {
                kind: A2uiErrorKind::InvalidDataClass,
                node_id: Some(view.root.clone()),
                message: format!(
                    "view data class '{}' exceeds admitted class '{}'",
                    view.data_class.as_str(),
                    max_admitted_class.as_str()
                ),
            });
        }
        Ok((saved.saved_id, view))
    }
}

/// Proposal intent emitted by a validated view. Always requires owner-mode capability evaluation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct A2uiProposalIntent {
    pub surface_id: String,
    pub node_id: String,
    pub intent: String,
    pub draft_fields: BTreeMap<String, String>,
    pub owner_mode_only: bool,
}

/// Constrained client store that permits writes only to `/draft/*` and `/ui/*` JSON pointers and
/// keeps authorized read snapshots immutable from client actions.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct A2uiClientState {
    snapshots: BTreeMap<String, Value>,
    draft: BTreeMap<String, String>,
    ui: BTreeMap<String, String>,
}

impl A2uiClientState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Host-only replacement of an authorized read-route snapshot. Retains `/draft` and `/ui`.
    pub fn replace_route_snapshot(
        &mut self,
        route: &str,
        payload: Value,
    ) -> Result<(), A2uiValidationError> {
        if !is_approved_read_route(route) {
            return Err(A2uiValidationError {
                kind: A2uiErrorKind::DisallowedRoute,
                node_id: None,
                message: format!("route '{route}' is not an approved registry read route"),
            });
        }
        self.snapshots.insert(route.to_owned(), payload);
        Ok(())
    }

    /// Client write restricted strictly to `/draft/<key>` or `/ui/<key>`.
    pub fn client_write(
        &mut self,
        pointer: &str,
        value: impl Into<String>,
    ) -> Result<(), A2uiValidationError> {
        let key = validate_writable_pointer(pointer, None)?;
        let clean = sanitize_inert_text(&value.into());
        if pointer.starts_with("/draft/") {
            self.draft.insert(key.to_owned(), clean);
        } else {
            self.ui.insert(key.to_owned(), clean);
        }
        Ok(())
    }

    pub fn draft_value(&self, key: &str) -> Option<&str> {
        self.draft.get(key).map(String::as_str)
    }

    pub fn draft_map(&self) -> &BTreeMap<String, String> {
        &self.draft
    }

    pub fn snapshot(&self, route: &str) -> Option<&Value> {
        self.snapshots.get(route)
    }

    /// Builds a typed owner-mode proposal from a validated button or form node.
    pub fn build_proposal(
        &self,
        view: &ValidatedA2uiView,
        node_id: &str,
    ) -> Result<A2uiProposalIntent, A2uiValidationError> {
        let node = view.nodes.get(node_id).ok_or_else(|| A2uiValidationError {
            kind: A2uiErrorKind::UnknownChildNode,
            node_id: Some(node_id.to_owned()),
            message: "node does not exist in validated view".into(),
        })?;
        let intent = node
            .proposal_intent
            .as_ref()
            .ok_or_else(|| A2uiValidationError {
                kind: A2uiErrorKind::InvalidProperty,
                node_id: Some(node_id.to_owned()),
                message: "node has no approved proposal_intent".into(),
            })?;
        if !APPROVED_PROPOSAL_INTENTS.contains(&intent.as_str()) {
            return Err(A2uiValidationError {
                kind: A2uiErrorKind::DisallowedRoute,
                node_id: Some(node_id.to_owned()),
                message: format!("proposal intent '{intent}' is not in the approved catalog"),
            });
        }
        Ok(A2uiProposalIntent {
            surface_id: view.surface_id.clone(),
            node_id: node_id.to_owned(),
            intent: intent.clone(),
            draft_fields: self.draft.clone(),
            owner_mode_only: true,
        })
    }
}

/// Neutralizes control characters and script/HTML tags so source strings render strictly as inert
/// text.
pub fn sanitize_inert_text(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if ch == '\0' || (ch.is_control() && !matches!(ch, '\n' | '\t')) {
            continue;
        }
        match ch {
            '<' => out.push_str("‹"),
            '>' => out.push_str("›"),
            _ => out.push(ch),
        }
    }
    out
}

pub fn is_approved_read_route(route: &str) -> bool {
    let trimmed = route.trim();
    if trimmed.is_empty() {
        return false;
    }
    let upper = trimmed.to_ascii_uppercase();
    if upper.starts_with("POST ")
        || upper.starts_with("PUT ")
        || upper.starts_with("DELETE ")
        || upper.starts_with("PATCH ")
    {
        return false;
    }
    let path = trimmed
        .strip_prefix("GET ")
        .unwrap_or(trimmed)
        .split('?')
        .next()
        .unwrap_or(trimmed);
    APPROVED_READ_ROUTES.contains(&path)
}

fn validate_writable_pointer<'a>(
    pointer: &'a str,
    node_id: Option<&str>,
) -> Result<&'a str, A2uiValidationError> {
    let err = |msg: String| A2uiValidationError {
        kind: A2uiErrorKind::DisallowedWritePath,
        node_id: node_id.map(str::to_owned),
        message: msg,
    };
    let rest = pointer
        .strip_prefix("/draft/")
        .or_else(|| pointer.strip_prefix("/ui/"))
        .ok_or_else(|| {
            err(format!(
                "write path '{pointer}' must stay under /draft/* or /ui/*"
            ))
        })?;
    if rest.is_empty()
        || rest.contains('/')
        || rest.contains("..")
        || matches!(rest, "__proto__" | "prototype" | "constructor")
    {
        return Err(err(format!(
            "write path '{pointer}' contains a forbidden segment"
        )));
    }
    Ok(rest)
}

fn parse_binding(
    bind: &str,
    declared_routes: &[String],
    node_id: &str,
) -> Result<(Option<String>, Option<String>), A2uiValidationError> {
    let trimmed = bind.trim();
    if trimmed.is_empty() {
        return Err(A2uiValidationError {
            kind: A2uiErrorKind::InvalidProperty,
            node_id: Some(node_id.to_owned()),
            message: "bind cannot be empty".into(),
        });
    }
    if trimmed.starts_with("/draft/") || trimmed.starts_with("/ui/") {
        let _ = validate_writable_pointer(trimmed, Some(node_id))?;
        return Ok((None, Some(trimmed.to_owned())));
    }
    let (route_part, pointer_part) = match trimmed.split_once('#') {
        Some((route, ptr)) => (route.trim(), Some(ptr.trim())),
        None => (trimmed, None),
    };
    if !is_approved_read_route(route_part) {
        return Err(A2uiValidationError {
            kind: A2uiErrorKind::DisallowedRoute,
            node_id: Some(node_id.to_owned()),
            message: format!("binding '{trimmed}' does not name an approved registry read route"),
        });
    }
    let canonical_route = route_part
        .strip_prefix("GET ")
        .unwrap_or(route_part)
        .split('?')
        .next()
        .unwrap_or(route_part)
        .to_owned();
    if !declared_routes.is_empty() && !declared_routes.contains(&canonical_route) {
        return Err(A2uiValidationError {
            kind: A2uiErrorKind::DisallowedRoute,
            node_id: Some(node_id.to_owned()),
            message: format!(
                "route '{canonical_route}' is not declared in the document's read_routes"
            ),
        });
    }
    if let Some(ptr) = pointer_part {
        if !ptr.is_empty()
            && (!ptr.starts_with('/')
                || ptr.contains("..")
                || ptr.contains("__proto__")
                || ptr.contains("constructor")
                || ptr.contains("prototype"))
        {
            return Err(A2uiValidationError {
                kind: A2uiErrorKind::InvalidProperty,
                node_id: Some(node_id.to_owned()),
                message: format!("invalid JSON pointer '{ptr}' in binding"),
            });
        }
    }
    Ok((
        Some(canonical_route),
        pointer_part.map(str::to_owned).filter(|s| !s.is_empty()),
    ))
}

/// Validates a raw JSON string into a [`ValidatedA2uiView`].
pub fn validate_json(raw: &str) -> Result<ValidatedA2uiView, A2uiValidationError> {
    if raw.len() > MAX_DOCUMENT_BYTES {
        return Err(A2uiValidationError {
            kind: A2uiErrorKind::DocumentOversized,
            node_id: None,
            message: format!(
                "document size ({} bytes) exceeds limit ({MAX_DOCUMENT_BYTES} bytes)",
                raw.len()
            ),
        });
    }
    let doc: A2uiDocument = serde_json::from_str(raw).map_err(|err| A2uiValidationError {
        kind: A2uiErrorKind::InvalidProperty,
        node_id: None,
        message: format!("invalid A2UI JSON: {err}"),
    })?;
    validate_document(&doc)
}

/// Validates an [`A2uiDocument`] against Sjel's component catalog, read-route registry, cycle and
/// depth limits, and property contracts.
pub fn validate_document(doc: &A2uiDocument) -> Result<ValidatedA2uiView, A2uiValidationError> {
    if doc.version != A2UI_SCHEMA_VERSION {
        return Err(A2uiValidationError {
            kind: A2uiErrorKind::UnsupportedVersion,
            node_id: None,
            message: format!(
                "unsupported schema version {}; expected {A2UI_SCHEMA_VERSION}",
                doc.version
            ),
        });
    }
    if doc.catalog_id != SJEL_CATALOG_ID {
        return Err(A2uiValidationError {
            kind: A2uiErrorKind::UnknownCatalog,
            node_id: None,
            message: format!(
                "unknown catalog_id '{}'; expected '{SJEL_CATALOG_ID}'",
                doc.catalog_id
            ),
        });
    }
    if doc.nodes.len() > MAX_NODES {
        let offending = doc.nodes.get(MAX_NODES).map(|n| n.id.clone());
        return Err(A2uiValidationError {
            kind: A2uiErrorKind::DocumentOversized,
            node_id: offending,
            message: format!(
                "document has {} nodes, exceeding maximum of {MAX_NODES}",
                doc.nodes.len()
            ),
        });
    }
    let data_class = if doc.data_class.trim().is_empty() {
        DataClass::C0
    } else {
        DataClass::parse(&doc.data_class).ok_or_else(|| A2uiValidationError {
            kind: A2uiErrorKind::InvalidDataClass,
            node_id: Some(doc.root.clone()),
            message: format!("unknown data_class '{}'", doc.data_class),
        })?
    };

    let mut canonical_routes = Vec::with_capacity(doc.read_routes.len());
    for route in &doc.read_routes {
        if !is_approved_read_route(route) {
            return Err(A2uiValidationError {
                kind: A2uiErrorKind::DisallowedRoute,
                node_id: Some(doc.root.clone()),
                message: format!("read_route '{route}' is not an approved registry read route"),
            });
        }
        let clean = route
            .trim()
            .strip_prefix("GET ")
            .unwrap_or(route.trim())
            .split('?')
            .next()
            .unwrap_or(route.trim())
            .to_owned();
        if !canonical_routes.contains(&clean) {
            canonical_routes.push(clean);
        }
    }

    let mut raw_by_id: BTreeMap<String, &A2uiNode> = BTreeMap::new();
    for node in &doc.nodes {
        if node.id.trim().is_empty() {
            return Err(A2uiValidationError {
                kind: A2uiErrorKind::InvalidProperty,
                node_id: None,
                message: "node id cannot be empty".into(),
            });
        }
        if raw_by_id.insert(node.id.clone(), node).is_some() {
            return Err(A2uiValidationError {
                kind: A2uiErrorKind::DuplicateNodeId,
                node_id: Some(node.id.clone()),
                message: "duplicate node id".into(),
            });
        }
    }

    if !raw_by_id.contains_key(&doc.root) {
        return Err(A2uiValidationError {
            kind: A2uiErrorKind::MissingRoot,
            node_id: Some(doc.root.clone()),
            message: format!("root node '{}' does not exist", doc.root),
        });
    }

    let mut validated_nodes = BTreeMap::new();
    for node in &doc.nodes {
        let validated = validate_node(node, &raw_by_id, &canonical_routes)?;
        validated_nodes.insert(node.id.clone(), validated);
    }

    // Walk the graph from root to detect cycles, depth violations, and unreachable nodes.
    let mut visiting = BTreeSet::new();
    let mut visited = BTreeSet::new();
    let mut order = Vec::new();
    walk_tree(
        &doc.root,
        0,
        &validated_nodes,
        &mut visiting,
        &mut visited,
        &mut order,
    )?;

    for id in validated_nodes.keys() {
        if !visited.contains(id) {
            return Err(A2uiValidationError {
                kind: A2uiErrorKind::UnboundNode,
                node_id: Some(id.clone()),
                message: "orphan node is not reachable from root".into(),
            });
        }
    }

    Ok(ValidatedA2uiView {
        version: doc.version,
        catalog_id: doc.catalog_id.clone(),
        surface_id: sanitize_inert_text(&doc.surface_id),
        title: sanitize_inert_text(&doc.title),
        root: doc.root.clone(),
        data_class,
        read_routes: canonical_routes,
        nodes: validated_nodes,
        order,
    })
}

fn walk_tree(
    node_id: &str,
    depth: usize,
    nodes: &BTreeMap<String, ValidatedNode>,
    visiting: &mut BTreeSet<String>,
    visited: &mut BTreeSet<String>,
    order: &mut Vec<String>,
) -> Result<(), A2uiValidationError> {
    if depth > MAX_DEPTH {
        return Err(A2uiValidationError {
            kind: A2uiErrorKind::DepthExceeded,
            node_id: Some(node_id.to_owned()),
            message: format!("tree depth exceeds maximum of {MAX_DEPTH}"),
        });
    }
    if !visiting.insert(node_id.to_owned()) {
        return Err(A2uiValidationError {
            kind: A2uiErrorKind::CycleDetected,
            node_id: Some(node_id.to_owned()),
            message: "cycle detected in component graph".into(),
        });
    }
    let node = nodes.get(node_id).ok_or_else(|| A2uiValidationError {
        kind: A2uiErrorKind::UnknownChildNode,
        node_id: Some(node_id.to_owned()),
        message: "referenced child node does not exist".into(),
    })?;
    if visited.insert(node_id.to_owned()) {
        order.push(node_id.to_owned());
    }
    for child in &node.children {
        walk_tree(child, depth + 1, nodes, visiting, visited, order)?;
    }
    visiting.remove(node_id);
    Ok(())
}

fn allowed_props(kind: ComponentKind) -> &'static [&'static str] {
    match kind {
        ComponentKind::Card => &["title", "subtitle"],
        ComponentKind::MetricStrip => &["title"],
        ComponentKind::MetricTile => &["label", "value", "detail"],
        ComponentKind::DescriptionList => &["title"],
        ComponentKind::DescriptionItem => &["label", "value"],
        ComponentKind::DataTable => &["title", "columns", "rows"],
        ComponentKind::Badge => &["label", "tone"],
        ComponentKind::Text => &["text"],
        ComponentKind::Form => &["title", "proposal_intent"],
        ComponentKind::TextField => &["label", "field", "placeholder"],
        ComponentKind::Button => &["label", "proposal_intent"],
    }
}

fn str_prop(
    props: &BTreeMap<String, Value>,
    key: &str,
    node_id: &str,
) -> Result<Option<String>, A2uiValidationError> {
    let Some(val) = props.get(key) else {
        return Ok(None);
    };
    let Some(text) = val.as_str() else {
        return Err(A2uiValidationError {
            kind: A2uiErrorKind::InvalidProperty,
            node_id: Some(node_id.to_owned()),
            message: format!("property '{key}' must be a string"),
        });
    };
    Ok(Some(sanitize_inert_text(text)))
}

fn validate_node(
    node: &A2uiNode,
    all_nodes: &BTreeMap<String, &A2uiNode>,
    declared_routes: &[String],
) -> Result<ValidatedNode, A2uiValidationError> {
    let kind = ComponentKind::parse(&node.component_type).ok_or_else(|| A2uiValidationError {
        kind: A2uiErrorKind::UnknownComponent,
        node_id: Some(node.id.clone()),
        message: format!(
            "component '{}' is not in catalog '{SJEL_CATALOG_ID}'",
            node.component_type
        ),
    })?;

    let permitted = allowed_props(kind);
    for prop_key in node.props.keys() {
        if !permitted.contains(&prop_key.as_str()) {
            return Err(A2uiValidationError {
                kind: A2uiErrorKind::InvalidProperty,
                node_id: Some(node.id.clone()),
                message: format!(
                    "unknown or disallowed property '{prop_key}' on {}",
                    kind.as_str()
                ),
            });
        }
    }

    for child_id in &node.children {
        if !all_nodes.contains_key(child_id) {
            return Err(A2uiValidationError {
                kind: A2uiErrorKind::UnknownChildNode,
                node_id: Some(node.id.clone()),
                message: format!("unknown child node '{child_id}'"),
            });
        }
    }

    let (bind_route, bind_pointer) = match &node.bind {
        Some(bind) => parse_binding(bind, declared_routes, &node.id)?,
        None => (None, None),
    };

    let title = str_prop(&node.props, "title", &node.id)?;
    let subtitle = str_prop(&node.props, "subtitle", &node.id)?;
    let mut label = str_prop(&node.props, "label", &node.id)?;
    let mut value = str_prop(&node.props, "value", &node.id)?;
    let detail = str_prop(&node.props, "detail", &node.id)?;
    let tone = str_prop(&node.props, "tone", &node.id)?;
    let field = str_prop(&node.props, "field", &node.id)?;
    let proposal_intent = str_prop(&node.props, "proposal_intent", &node.id)?;

    if let Some(text) = str_prop(&node.props, "text", &node.id)? {
        value = Some(text);
    }

    let mut columns = Vec::new();
    let mut rows = Vec::new();

    match kind {
        ComponentKind::Card | ComponentKind::MetricStrip | ComponentKind::DescriptionList => {
            if node.children.is_empty() && bind_route.is_none() {
                return Err(A2uiValidationError {
                    kind: A2uiErrorKind::InvalidProperty,
                    node_id: Some(node.id.clone()),
                    message: format!(
                        "{} must have at least one child or bind route",
                        kind.as_str()
                    ),
                });
            }
        }
        ComponentKind::MetricTile | ComponentKind::DescriptionItem => {
            if label.as_deref().unwrap_or("").trim().is_empty() {
                return Err(A2uiValidationError {
                    kind: A2uiErrorKind::InvalidProperty,
                    node_id: Some(node.id.clone()),
                    message: format!("{} requires a non-empty 'label'", kind.as_str()),
                });
            }
            if value.is_none() && bind_route.is_none() && bind_pointer.is_none() {
                return Err(A2uiValidationError {
                    kind: A2uiErrorKind::InvalidProperty,
                    node_id: Some(node.id.clone()),
                    message: format!("{} requires 'value' or 'bind'", kind.as_str()),
                });
            }
        }
        ComponentKind::Badge => {
            if label.as_deref().unwrap_or("").trim().is_empty() && bind_route.is_none() {
                return Err(A2uiValidationError {
                    kind: A2uiErrorKind::InvalidProperty,
                    node_id: Some(node.id.clone()),
                    message: "Badge requires 'label' or 'bind'".into(),
                });
            }
            if let Some(t) = &tone {
                if !matches!(
                    t.as_str(),
                    "neutral" | "accent" | "success" | "warning" | "danger"
                ) {
                    return Err(A2uiValidationError {
                        kind: A2uiErrorKind::InvalidProperty,
                        node_id: Some(node.id.clone()),
                        message: format!("invalid Badge tone '{t}'"),
                    });
                }
            }
        }
        ComponentKind::Text => {
            if value.as_deref().unwrap_or("").trim().is_empty() && bind_route.is_none() {
                return Err(A2uiValidationError {
                    kind: A2uiErrorKind::InvalidProperty,
                    node_id: Some(node.id.clone()),
                    message: "Text requires non-empty 'text' or 'bind'".into(),
                });
            }
        }
        ComponentKind::DataTable => {
            let raw_cols = node
                .props
                .get("columns")
                .and_then(Value::as_array)
                .ok_or_else(|| A2uiValidationError {
                    kind: A2uiErrorKind::InvalidProperty,
                    node_id: Some(node.id.clone()),
                    message: "DataTable requires a 'columns' array".into(),
                })?;
            if raw_cols.is_empty() {
                return Err(A2uiValidationError {
                    kind: A2uiErrorKind::InvalidProperty,
                    node_id: Some(node.id.clone()),
                    message: "DataTable 'columns' cannot be empty".into(),
                });
            }
            for col in raw_cols {
                let key = col
                    .get("key")
                    .and_then(Value::as_str)
                    .map(sanitize_inert_text)
                    .filter(|s| !s.trim().is_empty())
                    .ok_or_else(|| A2uiValidationError {
                        kind: A2uiErrorKind::InvalidProperty,
                        node_id: Some(node.id.clone()),
                        message: "each DataTable column requires a string 'key'".into(),
                    })?;
                let col_label = col
                    .get("label")
                    .and_then(Value::as_str)
                    .map(sanitize_inert_text)
                    .filter(|s| !s.trim().is_empty())
                    .ok_or_else(|| A2uiValidationError {
                        kind: A2uiErrorKind::InvalidProperty,
                        node_id: Some(node.id.clone()),
                        message: "each DataTable column requires a string 'label'".into(),
                    })?;
                columns.push(A2uiColumn {
                    key,
                    label: col_label,
                });
            }
            if let Some(raw_rows) = node.props.get("rows") {
                let arr = raw_rows.as_array().ok_or_else(|| A2uiValidationError {
                    kind: A2uiErrorKind::InvalidProperty,
                    node_id: Some(node.id.clone()),
                    message: "DataTable 'rows' must be an array of objects".into(),
                })?;
                for row_val in arr {
                    let obj = row_val.as_object().ok_or_else(|| A2uiValidationError {
                        kind: A2uiErrorKind::InvalidProperty,
                        node_id: Some(node.id.clone()),
                        message: "each DataTable row must be an object".into(),
                    })?;
                    let mut clean_row = BTreeMap::new();
                    for (k, v) in obj {
                        let cell_text = match v {
                            Value::String(s) => sanitize_inert_text(s),
                            Value::Number(n) => n.to_string(),
                            Value::Bool(b) => b.to_string(),
                            Value::Null => String::new(),
                            _ => sanitize_inert_text(&v.to_string()),
                        };
                        clean_row.insert(sanitize_inert_text(k), cell_text);
                    }
                    rows.push(clean_row);
                }
            } else if bind_route.is_none() {
                return Err(A2uiValidationError {
                    kind: A2uiErrorKind::InvalidProperty,
                    node_id: Some(node.id.clone()),
                    message: "DataTable requires either 'rows' or a read-route 'bind'".into(),
                });
            }
        }
        ComponentKind::Form => {
            if node.children.is_empty() {
                return Err(A2uiValidationError {
                    kind: A2uiErrorKind::InvalidProperty,
                    node_id: Some(node.id.clone()),
                    message: "Form requires child fields or buttons".into(),
                });
            }
            if let Some(intent) = &proposal_intent {
                if !APPROVED_PROPOSAL_INTENTS.contains(&intent.as_str()) {
                    return Err(A2uiValidationError {
                        kind: A2uiErrorKind::DisallowedRoute,
                        node_id: Some(node.id.clone()),
                        message: format!(
                            "Form proposal_intent '{intent}' is not an approved proposal intent"
                        ),
                    });
                }
            }
        }
        ComponentKind::TextField => {
            if label.as_deref().unwrap_or("").trim().is_empty() {
                return Err(A2uiValidationError {
                    kind: A2uiErrorKind::InvalidProperty,
                    node_id: Some(node.id.clone()),
                    message: "TextField requires a 'label'".into(),
                });
            }
            let ptr = field.as_deref().ok_or_else(|| A2uiValidationError {
                kind: A2uiErrorKind::InvalidProperty,
                node_id: Some(node.id.clone()),
                message: "TextField requires a 'field' pointer under /draft/*".into(),
            })?;
            if !ptr.starts_with("/draft/") {
                return Err(A2uiValidationError {
                    kind: A2uiErrorKind::DisallowedWritePath,
                    node_id: Some(node.id.clone()),
                    message: format!("TextField 'field' ('{ptr}') must target /draft/*"),
                });
            }
            let _ = validate_writable_pointer(ptr, Some(&node.id))?;
            if let Some(ph) = str_prop(&node.props, "placeholder", &node.id)? {
                value = Some(ph);
            }
        }
        ComponentKind::Button => {
            if label.as_deref().unwrap_or("").trim().is_empty() {
                return Err(A2uiValidationError {
                    kind: A2uiErrorKind::InvalidProperty,
                    node_id: Some(node.id.clone()),
                    message: "Button requires a 'label'".into(),
                });
            }
            let intent = proposal_intent
                .as_deref()
                .ok_or_else(|| A2uiValidationError {
                    kind: A2uiErrorKind::InvalidProperty,
                    node_id: Some(node.id.clone()),
                    message: "Button requires an approved 'proposal_intent'".into(),
                })?;
            if !APPROVED_PROPOSAL_INTENTS.contains(&intent) {
                return Err(A2uiValidationError {
                    kind: A2uiErrorKind::DisallowedRoute,
                    node_id: Some(node.id.clone()),
                    message: format!(
                        "Button proposal_intent '{intent}' is not an approved proposal intent"
                    ),
                });
            }
            if label.is_none() {
                label = Some(intent.to_owned());
            }
        }
    }

    Ok(ValidatedNode {
        id: node.id.clone(),
        kind,
        title,
        subtitle,
        label,
        value,
        detail,
        tone,
        field,
        proposal_intent,
        columns,
        rows,
        bind_route,
        bind_pointer,
        children: node.children.clone(),
    })
}

/// Builds the canonical synthetic list, detail, and form view document used by the adoption gate
/// and by the agent panel's saved-view shelf.
pub fn synthetic_project_triage_document() -> A2uiDocument {
    use serde_json::json;

    let mut root_props = BTreeMap::new();
    root_props.insert("title".into(), json!("Project Triage Overview"));
    root_props.insert(
        "subtitle".into(),
        json!("Live read snapshot bound to /api/projects/tasks"),
    );

    let mut tile_open_props = BTreeMap::new();
    tile_open_props.insert("label".into(), json!("Open tasks"));
    tile_open_props.insert("value".into(), json!("4"));
    tile_open_props.insert("detail".into(), json!("2 due today"));

    let mut tile_done_props = BTreeMap::new();
    tile_done_props.insert("label".into(), json!("Completed"));
    tile_done_props.insert("value".into(), json!("12"));

    let mut table_props = BTreeMap::new();
    table_props.insert(
        "columns".into(),
        json!([
            { "key": "title", "label": "TASK" },
            { "key": "project", "label": "PROJECT" },
            { "key": "due", "label": "DUE" }
        ]),
    );
    table_props.insert(
        "rows".into(),
        json!([
            { "title": "Validate A2UI view schema", "project": "sjel", "due": "Today" },
            { "title": "Reload saved views offline", "project": "sjel", "due": "Tomorrow" }
        ]),
    );

    let mut item_route_props = BTreeMap::new();
    item_route_props.insert("label".into(), json!("READ ROUTE"));
    item_route_props.insert("value".into(), json!("/api/projects/tasks"));

    let mut item_mode_props = BTreeMap::new();
    item_mode_props.insert("label".into(), json!("MUTATION MODE"));
    item_mode_props.insert("value".into(), json!("Owner-mode proposal only"));

    let mut field_title_props = BTreeMap::new();
    field_title_props.insert("label".into(), json!("Proposed task title"));
    field_title_props.insert("field".into(), json!("/draft/task_title"));
    field_title_props.insert("placeholder".into(), json!("Add follow-up task…"));

    let mut btn_submit_props = BTreeMap::new();
    btn_submit_props.insert("label".into(), json!("Propose task"));
    btn_submit_props.insert("proposal_intent".into(), json!("proposal:create_task"));

    let mut form_props = BTreeMap::new();
    form_props.insert("title".into(), json!("Propose follow-up task"));
    form_props.insert("proposal_intent".into(), json!("proposal:create_task"));

    A2uiDocument {
        version: A2UI_SCHEMA_VERSION,
        catalog_id: SJEL_CATALOG_ID.into(),
        surface_id: "project-triage-live".into(),
        title: "Project Triage".into(),
        root: "root-card".into(),
        data_class: "c1".into(),
        read_routes: vec!["/api/projects/tasks".into(), "/api/projects".into()],
        nodes: vec![
            A2uiNode {
                id: "root-card".into(),
                component_type: "Card".into(),
                props: root_props,
                children: vec![
                    "metrics-strip".into(),
                    "tasks-table".into(),
                    "detail-list".into(),
                    "proposal-form".into(),
                ],
                bind: None,
            },
            A2uiNode {
                id: "metrics-strip".into(),
                component_type: "MetricStrip".into(),
                props: BTreeMap::new(),
                children: vec!["tile-open".into(), "tile-done".into()],
                bind: None,
            },
            A2uiNode {
                id: "tile-open".into(),
                component_type: "MetricTile".into(),
                props: tile_open_props,
                children: Vec::new(),
                bind: Some("/api/projects/tasks#/open_count".into()),
            },
            A2uiNode {
                id: "tile-done".into(),
                component_type: "MetricTile".into(),
                props: tile_done_props,
                children: Vec::new(),
                bind: Some("/api/projects/tasks#/done_count".into()),
            },
            A2uiNode {
                id: "tasks-table".into(),
                component_type: "DataTable".into(),
                props: table_props,
                children: Vec::new(),
                bind: Some("/api/projects/tasks#/rows".into()),
            },
            A2uiNode {
                id: "detail-list".into(),
                component_type: "DescriptionList".into(),
                props: BTreeMap::new(),
                children: vec!["item-route".into(), "item-mode".into()],
                bind: None,
            },
            A2uiNode {
                id: "item-route".into(),
                component_type: "DescriptionItem".into(),
                props: item_route_props,
                children: Vec::new(),
                bind: None,
            },
            A2uiNode {
                id: "item-mode".into(),
                component_type: "DescriptionItem".into(),
                props: item_mode_props,
                children: Vec::new(),
                bind: None,
            },
            A2uiNode {
                id: "proposal-form".into(),
                component_type: "Form".into(),
                props: form_props,
                children: vec!["field-title".into(), "btn-propose".into()],
                bind: None,
            },
            A2uiNode {
                id: "field-title".into(),
                component_type: "TextField".into(),
                props: field_title_props,
                children: Vec::new(),
                bind: Some("/draft/task_title".into()),
            },
            A2uiNode {
                id: "btn-propose".into(),
                component_type: "Button".into(),
                props: btn_submit_props,
                children: Vec::new(),
                bind: None,
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn adoption_gate_synthetic_list_detail_form_saves_and_reloads_without_model() {
        let doc = synthetic_project_triage_document();
        let saved = SavedA2uiView::new("saved-project-triage", doc).expect("valid synthetic view");
        let serialized = saved.to_json().expect("serializes to JSON");

        // Reload without any model call and verify data-class gate.
        let (reloaded_id, view) =
            SavedA2uiView::reload_from_json(&serialized, DataClass::C2).expect("reloads offline");
        assert_eq!(reloaded_id, "saved-project-triage");
        assert_eq!(view.data_class, DataClass::C1);
        assert_eq!(view.read_routes, ["/api/projects/tasks", "/api/projects"]);
        assert_eq!(view.nodes.len(), 11);

        // Reload fails when the surface admits a lower data class than the view requires.
        let denied = SavedA2uiView::reload_from_json(&serialized, DataClass::C0)
            .expect_err("c1 view must be refused on a c0-only surface");
        assert_eq!(denied.kind, A2uiErrorKind::InvalidDataClass);
        assert_eq!(denied.node_id.as_deref(), Some("root-card"));
    }

    #[test]
    fn client_state_retains_local_drafts_across_host_snapshot_refreshes_and_emits_typed_proposal() {
        let doc = synthetic_project_triage_document();
        let view = validate_document(&doc).expect("valid view");
        let mut store = A2uiClientState::new();

        store
            .client_write(
                "/draft/task_title",
                "Ship GPUI live views <script>alert(1)</script>",
            )
            .expect("draft write allowed");
        store
            .replace_route_snapshot("/api/projects/tasks", json!({ "open_count": 7 }))
            .expect("authorized snapshot refresh allowed");

        // Draft survives independent host data refresh and neutralizes angle brackets.
        assert_eq!(
            store.draft_value("task_title"),
            Some("Ship GPUI live views ‹script›alert(1)‹/script›")
        );

        // Client cannot write outside /draft/* or /ui/* or traverse prototype keys.
        let bad_root = store
            .client_write("/snapshots/tasks", "spoof")
            .expect_err("root/snapshot write must fail");
        assert_eq!(bad_root.kind, A2uiErrorKind::DisallowedWritePath);

        let bad_proto = store
            .client_write("/draft/__proto__", "spoof")
            .expect_err("prototype traversal must fail");
        assert_eq!(bad_proto.kind, A2uiErrorKind::DisallowedWritePath);

        // Submitting emits an owner-mode proposal intent, never a direct capability mutation.
        let proposal = store
            .build_proposal(&view, "btn-propose")
            .expect("proposal built");
        assert_eq!(proposal.intent, "proposal:create_task");
        assert!(proposal.owner_mode_only);
        assert_eq!(
            proposal.draft_fields.get("task_title").map(String::as_str),
            Some("Ship GPUI live views ‹script›alert(1)‹/script›")
        );
    }

    #[test]
    fn validator_rejects_unknown_components_invalid_props_write_routes_cycles_and_oversized_docs() {
        // 1. Unknown component names the failing node.
        let mut doc = synthetic_project_triage_document();
        doc.nodes[1].component_type = "ArbitraryIframe".into();
        let err = validate_document(&doc).expect_err("unknown component must fail");
        assert_eq!(err.kind, A2uiErrorKind::UnknownComponent);
        assert_eq!(err.node_id.as_deref(), Some("metrics-strip"));

        // 2. Invalid property names the failing node.
        let mut doc = synthetic_project_triage_document();
        doc.nodes[0]
            .props
            .insert("onClickEval".into(), json!("window.location='http://evil'"));
        let err = validate_document(&doc).expect_err("disallowed property must fail");
        assert_eq!(err.kind, A2uiErrorKind::InvalidProperty);
        assert_eq!(err.node_id.as_deref(), Some("root-card"));

        // 3. Write-route binding names the failing node.
        let mut doc = synthetic_project_triage_document();
        doc.nodes[2].bind = Some("POST /api/people/decide".into());
        let err = validate_document(&doc).expect_err("write route must fail");
        assert_eq!(err.kind, A2uiErrorKind::DisallowedRoute);
        assert_eq!(err.node_id.as_deref(), Some("tile-open"));

        // 4. Cycle in children graph names the failing node.
        let mut doc = synthetic_project_triage_document();
        doc.nodes[1].children.push("root-card".into());
        let err = validate_document(&doc).expect_err("cycle must fail");
        assert_eq!(err.kind, A2uiErrorKind::CycleDetected);
        assert_eq!(err.node_id.as_deref(), Some("root-card"));

        // 5. Oversized document (> MAX_NODES) names the offending node.
        let mut doc = synthetic_project_triage_document();
        for idx in 0..70 {
            let id = format!("extra-node-{idx}");
            let mut props = BTreeMap::new();
            props.insert("text".into(), json!("item"));
            doc.nodes[0].children.push(id.clone());
            doc.nodes.push(A2uiNode {
                id,
                component_type: "Text".into(),
                props,
                children: Vec::new(),
                bind: None,
            });
        }
        let err = validate_document(&doc).expect_err("oversized node count must fail");
        assert_eq!(err.kind, A2uiErrorKind::DocumentOversized);
        assert!(err.node_id.is_some());
    }
}
