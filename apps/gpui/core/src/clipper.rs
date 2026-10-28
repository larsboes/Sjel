//! The Sjel Clipper as the app sees it: the templates and Interpreter model comms owns, and the
//! clip documents saved through the extension.
//!
//! Everything goes over comms' owner-only clip socket (`libs/clip-socket`), not through
//! sjel-status: clip content is `c2` and stays on this Mac. Templates keep upstream's shape
//! (`capabilities/comms/clipper/src/types/types.ts`); the app edits the fields people change and
//! carries every other field through unchanged.

use serde_json::{json, Value};

/// Upstream's save behaviors, with the label the app shows.
pub const BEHAVIORS: [(&str, &str); 6] = [
    ("create", "New clip"),
    ("overwrite", "Overwrite note"),
    ("append-specific", "Append to note"),
    ("prepend-specific", "Prepend to note"),
    ("append-daily", "Append to daily"),
    ("prepend-daily", "Prepend to daily"),
];

/// The template fields the app edits, as (key, label). Triggers are a list, edited one per line.
pub const FIELDS: [(&str, &str); 4] = [
    ("name", "Name"),
    ("path", "Path"),
    ("noteNameFormat", "Note name"),
    ("noteContentFormat", "Note content"),
];

#[derive(Clone, Debug, PartialEq)]
pub struct Interpreter {
    pub available: bool,
    pub model: Option<String>,
    pub backend: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// 0 until the extension has seeded Sjel with its templates.
    pub revision: i64,
    pub templates: Vec<Value>,
    pub interpreter: Interpreter,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ClipDocument {
    pub key: String,
    pub title: String,
    pub url: String,
    pub versions: u64,
    pub updated_at: String,
    pub markdown: String,
}

/// One line for a refusal code from comms.
pub fn error_line(code: &str) -> String {
    match code {
        "unavailable" => "comms is not answering on this Mac. Start it under Capabilities.".into(),
        "unconfigured" => "No overlay is configured, so the clip socket has no path.".into(),
        "stale" => "The templates changed since they were loaded. Reload, then edit again.".into(),
        "invalid" => "Every template needs a name, a unique id and a save behavior.".into(),
        "too_large" => "The templates exceed comms' 1 MiB limit.".into(),
        "not_found" => "That clip no longer exists.".into(),
        other => format!("comms refused the request ({other})."),
    }
}

fn ask(request: Value) -> Result<Value, String> {
    let Some(socket) = sjel_clip_socket::socket_path() else {
        return Err(error_line("unconfigured"));
    };
    let answer = sjel_clip_socket::relay(&socket, &request);
    if answer["ok"] == true {
        Ok(answer)
    } else {
        Err(error_line(answer["error"].as_str().unwrap_or("unknown")))
    }
}

pub fn parse_settings(answer: &Value) -> Settings {
    let interpreter = &answer["interpreter"];
    Settings {
        revision: answer["revision"].as_i64().unwrap_or(0),
        templates: answer["templates"].as_array().cloned().unwrap_or_default(),
        interpreter: Interpreter {
            available: interpreter["available"] == true,
            model: interpreter["model"].as_str().map(str::to_owned),
            backend: interpreter["backend"].as_str().map(str::to_owned),
        },
    }
}

pub fn parse_document(document: &Value) -> ClipDocument {
    let text = |key: &str| document[key].as_str().unwrap_or_default().to_owned();
    ClipDocument {
        key: text("document_key"),
        title: text("title"),
        url: text("url"),
        versions: document["versions"].as_u64().unwrap_or(0),
        updated_at: text("updated_at"),
        markdown: text("markdown"),
    }
}

/// Blocking: run off the UI thread.
pub fn load_settings() -> Result<Settings, String> {
    ask(json!({ "type": "settings_get" })).map(|answer| parse_settings(&answer))
}

/// Blocking. Returns the new revision, or why comms refused.
pub fn save_templates(templates: &[Value], base_revision: i64) -> Result<i64, String> {
    ask(json!({ "type": "settings_put", "templates": templates, "base_revision": base_revision }))
        .map(|answer| answer["revision"].as_i64().unwrap_or(base_revision))
}

/// Blocking.
pub fn documents(limit: u64) -> Result<Vec<ClipDocument>, String> {
    ask(json!({ "type": "documents", "limit": limit })).map(|answer| {
        answer["documents"]
            .as_array()
            .map(|list| list.iter().map(parse_document).collect())
            .unwrap_or_default()
    })
}

/// Blocking.
pub fn document(key: &str) -> Result<ClipDocument, String> {
    ask(json!({ "type": "document", "key": key })).map(|answer| parse_document(&answer["document"]))
}

pub fn text_field(template: &Value, key: &str) -> String {
    template[key].as_str().unwrap_or_default().to_owned()
}

pub fn triggers(template: &Value) -> String {
    template["triggers"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

/// Apply the edited fields to a template, keeping every field the app does not edit.
pub fn apply_edit(template: &mut Value, fields: &[(&str, String)], behavior: &str, triggers: &str) {
    for (key, value) in fields {
        template[*key] = Value::String(value.clone());
    }
    template["behavior"] = Value::String(behavior.to_owned());
    let list: Vec<Value> = triggers
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| Value::String(line.to_owned()))
        .collect();
    template["triggers"] = Value::Array(list);
}

/// A new template in upstream's default shape, with an id no existing template uses.
pub fn new_template(existing: &[Value], now_millis: u128) -> Value {
    let mut id = format!("sjel-{now_millis:x}");
    while existing.iter().any(|t| t["id"] == id.as_str()) {
        id.push('x');
    }
    json!({
        "id": id,
        "name": "New template",
        "behavior": "create",
        "noteNameFormat": "{{title}}",
        "path": "Clippings",
        "noteContentFormat": "{{content}}",
        "properties": [],
        "triggers": [],
        "context": "",
    })
}

/// Move a template one place; out-of-range moves leave the list as it was.
pub fn move_template(templates: &mut [Value], index: usize, up: bool) -> usize {
    let target = if up {
        index.checked_sub(1)
    } else {
        index.checked_add(1).filter(|t| *t < templates.len())
    };
    match target {
        Some(target) if index < templates.len() => {
            templates.swap(index, target);
            target
        }
        _ => index,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_edit_keeps_the_fields_the_app_does_not_show() {
        let mut template = json!({"id": "a", "name": "Old", "behavior": "create", "properties": [{"name": "tags"}], "context": "{{selection}}"});
        apply_edit(
            &mut template,
            &[
                ("name", "Reddit".into()),
                ("path", "Clippings/Reddit".into()),
            ],
            "append-daily",
            "https://www.reddit.com/\n\n schema:Article \n",
        );
        assert_eq!(template["name"], "Reddit");
        assert_eq!(template["behavior"], "append-daily");
        assert_eq!(
            template["triggers"],
            json!(["https://www.reddit.com/", "schema:Article"])
        );
        assert_eq!(template["properties"], json!([{"name": "tags"}]));
        assert_eq!(template["context"], "{{selection}}");
        assert_eq!(
            triggers(&template),
            "https://www.reddit.com/\nschema:Article"
        );
    }

    #[test]
    fn new_templates_get_an_unused_id_and_moves_stay_in_range() {
        let first = new_template(&[], 255);
        assert_eq!(first["id"], "sjel-ff");
        let second = new_template(std::slice::from_ref(&first), 255);
        assert_eq!(second["id"], "sjel-ffx");
        let mut list = vec![first, second, json!({"id": "c"})];
        assert_eq!(move_template(&mut list, 0, true), 0);
        assert_eq!(move_template(&mut list, 2, false), 2);
        assert_eq!(move_template(&mut list, 2, true), 1);
        assert_eq!(list[1]["id"], "c");
    }

    #[test]
    fn answers_parse_without_panicking_on_missing_fields() {
        let settings = parse_settings(
            &json!({"ok": true, "revision": 3, "templates": [{"id": "a"}], "interpreter": {"available": true, "model": "apple-foundationmodel"}}),
        );
        assert_eq!((settings.revision, settings.templates.len()), (3, 1));
        assert_eq!(
            settings.interpreter.model.as_deref(),
            Some("apple-foundationmodel")
        );
        assert_eq!(parse_settings(&json!({})).revision, 0);
        let doc = parse_document(&json!({"document_key": "clip:x", "versions": 2}));
        assert_eq!(
            (doc.key.as_str(), doc.versions, doc.title.as_str()),
            ("clip:x", 2, "")
        );
        assert!(error_line("stale").contains("Reload"));
    }
}
