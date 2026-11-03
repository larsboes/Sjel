//! The Sjel Clipper as the app sees it: the templates and Interpreter model comms owns, and the
//! clip documents saved through the extension.
//!
//! Everything goes over comms' owner-only clip socket (`libs/clip-socket`), not through
//! sjel-status: clip content is `c2` and stays on this Mac. Templates keep upstream's shape
//! (`capabilities/comms/clipper/src/types/types.ts`); the app edits the fields people change and
//! carries every other field through unchanged.

use crate::inline::Inline;
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

pub(crate) fn ask(request: Value) -> Result<Value, String> {
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

/// Blocking. Deletes every stored version of one clip document.
pub fn delete_document(key: &str) -> Result<usize, String> {
    ask(json!({ "type": "delete_document", "key": key })).map(|answer| {
        answer["deleted_rows"]
            .as_u64()
            .and_then(|count| usize::try_from(count).ok())
            .unwrap_or_default()
    })
}

/// Blocking. Deletes every stored clip version.
pub fn delete_all_documents() -> Result<usize, String> {
    ask(json!({ "type": "delete_all" })).map(|answer| {
        answer["deleted_rows"]
            .as_u64()
            .and_then(|count| usize::try_from(count).ok())
            .unwrap_or_default()
    })
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

/// Extract an attribute value from an HTML tag string.
pub fn extract_attribute(tag: &str, attr: &str) -> Option<String> {
    let lower = tag.to_lowercase();
    let pattern = format!("{attr}=");
    let at = lower.find(&pattern)?;
    let rest = tag[at + pattern.len()..].trim_start();
    if rest.starts_with('"') {
        let end = rest[1..].find('"')?;
        Some(rest[1..1 + end].to_string())
    } else if rest.starts_with('\'') {
        let end = rest[1..].find('\'')?;
        Some(rest[1..1 + end].to_string())
    } else {
        let end = rest
            .find(|c: char| c.is_whitespace() || c == '>')
            .unwrap_or(rest.len());
        Some(rest[..end].to_string())
    }
}

/// Decode common HTML entities.
pub fn decode_html_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '&' {
            let mut ent = String::new();
            while let Some(&next) = chars.peek() {
                if next == ';' {
                    chars.next();
                    break;
                }
                if next.is_alphanumeric() || next == '#' {
                    ent.push(chars.next().unwrap());
                    if ent.len() > 10 {
                        break;
                    }
                } else {
                    break;
                }
            }
            match ent.as_str() {
                "amp" => out.push('&'),
                "lt" => out.push('<'),
                "gt" => out.push('>'),
                "quot" => out.push('"'),
                "apos" | "#39" => out.push('\''),
                "nbsp" => out.push(' '),
                s if s.starts_with('#') => {
                    let num = if s.starts_with("#x") || s.starts_with("#X") {
                        u32::from_str_radix(&s[2..], 16).ok()
                    } else {
                        s[1..].parse::<u32>().ok()
                    };
                    if let Some(c) = num.and_then(char::from_u32) {
                        out.push(c);
                    } else {
                        out.push('&');
                        out.push_str(&ent);
                        out.push(';');
                    }
                }
                _ => {
                    out.push('&');
                    out.push_str(&ent);
                    out.push(';');
                }
            }
        } else {
            out.push(ch);
        }
    }
    out
}

/// Convert HTML snippet into clean Markdown, preserving links and footnote citations.
pub fn html_to_markdown(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_script_or_style = false;
    let mut in_a: Option<(String, String)> = None;
    let mut in_pre = false;
    let mut in_code = false;
    let mut in_bold = false;
    let mut in_italic = false;
    let mut in_table = false;
    let mut in_table_cell = false;
    let mut table_is_first_row = false;
    let mut current_row_cols = 0;

    let mut i = 0;
    let bytes = html.as_bytes();
    while i < bytes.len() {
        if bytes[i] == b'<' {
            if html[i..].starts_with("<!--") {
                if let Some(end) = html[i..].find("-->") {
                    i += end + 3;
                    continue;
                }
            }
            if let Some(end_offset) = html[i..].find('>') {
                let tag_str = &html[i..i + end_offset + 1];
                let is_closing = tag_str.starts_with("</");
                let tag_name = tag_str
                    .trim_start_matches('<')
                    .trim_start_matches('/')
                    .split(|c: char| c.is_whitespace() || c == '>' || c == '/')
                    .next()
                    .unwrap_or("")
                    .to_lowercase();

                match tag_name.as_str() {
                    "script" | "style" | "head" => {
                        in_script_or_style = !is_closing;
                    }
                    "a" => {
                        if is_closing {
                            if let Some((href, label)) = in_a.take() {
                                let decoded_label = decode_html_entities(label.trim());
                                let clean_label = decoded_label
                                    .trim_start_matches('[')
                                    .trim_end_matches(']')
                                    .trim();
                                if !clean_label.is_empty() {
                                    let link_md = format!("[{clean_label}]({href})");
                                    out.push_str(&link_md);
                                }
                            }
                        } else {
                            let href = extract_attribute(tag_str, "href").unwrap_or_default();
                            let clean_href = unwrap_google_redirect(&href);
                            in_a = Some((clean_href, String::new()));
                        }
                    }
                    "p" | "div" | "section" | "article" => {
                        if is_closing || !out.ends_with("\n\n") {
                            if !out.is_empty() && !out.ends_with('\n') {
                                out.push('\n');
                            }
                            if !out.ends_with("\n\n") {
                                out.push('\n');
                            }
                        }
                    }
                    "br" => {
                        if in_table {
                            out.push_str("<br>");
                        } else {
                            out.push('\n');
                        }
                    }
                    "table" => {
                        if !is_closing {
                            in_table = true;
                            table_is_first_row = true;
                            if !out.is_empty() && !out.ends_with("\n\n") {
                                if !out.ends_with('\n') {
                                    out.push('\n');
                                }
                                out.push('\n');
                            }
                        } else {
                            in_table = false;
                            if !out.ends_with("\n\n") {
                                if !out.ends_with('\n') {
                                    out.push('\n');
                                }
                                out.push('\n');
                            }
                        }
                    }
                    "tr" => {
                        if !is_closing {
                            if !out.ends_with('\n') && !out.is_empty() {
                                out.push('\n');
                            }
                            out.push_str("| ");
                            current_row_cols = 0;
                        } else if table_is_first_row {
                            table_is_first_row = false;
                            let cols = current_row_cols.max(1);
                            out.push('\n');
                            out.push('|');
                            for _ in 0..cols {
                                out.push_str(" --- |");
                            }
                        }
                    }
                    "th" | "td" => {
                        if !is_closing {
                            in_table_cell = true;
                        } else {
                            in_table_cell = false;
                            current_row_cols += 1;
                            while out.ends_with(' ') && !out.ends_with("| ") {
                                out.pop();
                            }
                            out.push_str(" | ");
                        }
                    }
                    "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                        if !is_closing {
                            if !out.is_empty() && !out.ends_with('\n') {
                                out.push('\n');
                            }
                            if !out.ends_with("\n\n") {
                                out.push('\n');
                            }
                            let level = tag_name
                                .chars()
                                .nth(1)
                                .and_then(|c| c.to_digit(10))
                                .unwrap_or(2);
                            for _ in 0..level {
                                out.push('#');
                            }
                            out.push(' ');
                        } else {
                            out.push('\n');
                            out.push('\n');
                        }
                    }
                    "strong" | "b" => {
                        if !is_closing && !in_bold {
                            out.push_str("**");
                            in_bold = true;
                        } else if is_closing && in_bold {
                            out.push_str("**");
                            in_bold = false;
                        }
                    }
                    "em" | "i" => {
                        if !is_closing && !in_italic {
                            out.push('*');
                            in_italic = true;
                        } else if is_closing && in_italic {
                            out.push('*');
                            in_italic = false;
                        }
                    }
                    "pre" => {
                        if !is_closing {
                            if !out.is_empty() && !out.ends_with('\n') {
                                out.push('\n');
                            }
                            out.push_str("```\n");
                            in_pre = true;
                        } else {
                            if !out.ends_with('\n') {
                                out.push('\n');
                            }
                            out.push_str("```\n\n");
                            in_pre = false;
                        }
                    }
                    "code" => {
                        if !in_pre {
                            if !is_closing && !in_code {
                                out.push('`');
                                in_code = true;
                            } else if is_closing && in_code {
                                out.push('`');
                                in_code = false;
                            }
                        }
                    }
                    "li" => {
                        if !is_closing {
                            if !out.ends_with('\n') && !out.is_empty() {
                                out.push('\n');
                            }
                            out.push_str("- ");
                        } else {
                            out.push('\n');
                        }
                    }
                    "ul" | "ol" => {
                        if is_closing && !out.ends_with('\n') {
                            out.push('\n');
                        }
                    }
                    "blockquote" => {
                        if !is_closing {
                            if !out.ends_with('\n') && !out.is_empty() {
                                out.push('\n');
                            }
                            out.push_str("> ");
                        } else {
                            out.push('\n');
                        }
                    }
                    _ => {}
                }

                i += end_offset + 1;
                continue;
            }
        }

        if !in_script_or_style {
            let next_tag = html[i..].find('<').unwrap_or(html[i..].len());
            let text_chunk = &html[i..i + next_tag];
            if in_table && !in_table_cell && text_chunk.trim().is_empty() {
                // Ignore whitespace formatting between table tags
            } else if let Some((_, label_buf)) = in_a.as_mut() {
                label_buf.push_str(text_chunk);
            } else {
                let decoded = decode_html_entities(text_chunk);
                if in_table_cell {
                    let single_line = decoded.replace('\r', "").replace('\n', " ");
                    out.push_str(&single_line);
                } else {
                    out.push_str(&decoded);
                }
            }
            i += next_tag;
        } else {
            i += 1;
        }
    }

    let mut cleaned = String::new();
    let mut consecutive_newlines = 0;
    for ch in out.trim().chars() {
        if ch == '\n' {
            consecutive_newlines += 1;
            if consecutive_newlines <= 2 {
                cleaned.push(ch);
            }
        } else {
            consecutive_newlines = 0;
            cleaned.push(ch);
        }
    }
    cleaned
}

/// One turn of an LLM dialogue (prompt + response + optional thoughts).
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ClipTurn {
    pub index: usize,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thoughts: Option<String>,
    pub response: String,
}

/// Verification status of a link or footnote reference.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum CitationStatus {
    /// Valid URL link (e.g. `[1](https://...)` or `[OpenAI](https://...)`).
    Resolved,
    /// Footnote reference like `[1]` in text with no URL target.
    UnlinkedRef,
    /// Link syntax with an empty or non-web scheme.
    MalformedUrl,
}

/// One audit finding for a link or citation marker.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ClipCitation {
    pub marker: String,
    pub label: String,
    pub url: Option<String>,
    pub status: CitationStatus,
}

/// Unwraps Google interstitial tracking URLs (e.g. `https://www.google.com/url?sa=E&q=...`).
pub fn unwrap_google_redirect(raw_url: &str) -> String {
    let mut current = raw_url.to_string();
    for _ in 0..3 {
        let Ok(parsed) = reqwest::Url::parse(&current) else {
            break;
        };
        let host = parsed.host_str().unwrap_or("");
        if (host.contains("google.") || host == "google") && parsed.path().starts_with("/url") {
            let mut found = None;
            for (k, v) in parsed.query_pairs() {
                if k == "q" || k == "url" {
                    found = Some(v.into_owned());
                    break;
                }
            }
            if let Some(target) = found {
                current = target;
                continue;
            }
        }
        break;
    }
    current
}

/// Resolves a URL by following any HTTP redirects (e.g. from Google Vertex AI Search grounding),
/// returning the final effective destination URL.
pub async fn resolve_redirect_http(raw_url: &str) -> String {
    let clean = unwrap_google_redirect(raw_url);
    if !clean.contains("vertexaisearch.cloud.google.com/grounding-api-redirect/") {
        return clean;
    }
    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/133.0.0.0 Safari/537.36")
        .redirect(reqwest::redirect::Policy::limited(10))
        .timeout(std::time::Duration::from_secs(5))
        .build();
    let Ok(client) = client else {
        return clean;
    };

    if let Ok(resp) = client.head(&clean).send().await {
        let final_url = resp.url().as_str();
        if !final_url.contains("vertexaisearch.cloud.google.com") {
            return final_url.to_string();
        }
    }
    if let Ok(resp) = client.get(&clean).send().await {
        let final_url = resp.url().as_str();
        if !final_url.contains("vertexaisearch.cloud.google.com") {
            return final_url.to_string();
        }
    }
    clean
}

/// Parses all markdown links and bare citations `[N]` in `text`.
pub fn audit_citations(text: &str) -> Vec<ClipCitation> {
    let mut citations = Vec::new();
    let inline = Inline::parse(text);

    // 1. Gather all parsed markdown links
    let mut linked_ranges = Vec::new();
    for (range, raw_url) in &inline.links {
        linked_ranges.push(range.clone());
        let url = unwrap_google_redirect(raw_url);
        let label = if range.end <= inline.text.len() {
            inline.text[range.clone()].trim().to_string()
        } else {
            String::new()
        };
        let is_valid = url.starts_with("http://") || url.starts_with("https://");
        let status = if is_valid {
            CitationStatus::Resolved
        } else {
            CitationStatus::MalformedUrl
        };
        let marker = if label.starts_with('[') && label.ends_with(']') {
            label.clone()
        } else {
            format!("[{label}]")
        };
        citations.push(ClipCitation {
            marker,
            label,
            url: Some(url),
            status,
        });
    }

    // 2. Scan for unlinked numeric citations [N] in inline.text
    let bytes = inline.text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'[' {
            let start = i;
            let mut j = i + 1;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            if j > i + 1 && j < bytes.len() && bytes[j] == b']' {
                let end = j + 1;
                let is_linked = linked_ranges.iter().any(|r| start < r.end && end > r.start);
                if !is_linked {
                    let num_str = &inline.text[i + 1..j];
                    citations.push(ClipCitation {
                        marker: format!("[{num_str}]"),
                        label: num_str.to_string(),
                        url: None,
                        status: CitationStatus::UnlinkedRef,
                    });
                }
                i = end;
                continue;
            }
        }
        i += 1;
    }

    citations
}

/// Formats turns into canonical Markdown.
pub fn turns_to_markdown(turns: &[ClipTurn]) -> String {
    let mut out = String::new();
    for turn in turns {
        if !out.is_empty() {
            out.push_str("\n\n---\n\n");
        }
        out.push_str(&format!("## Prompt {}\n\n", turn.index));
        out.push_str(turn.prompt.trim());
        out.push_str("\n\n");

        if let Some(thoughts) = &turn.thoughts {
            if !thoughts.trim().is_empty() {
                out.push_str("> **Thoughts**:\n");
                for line in thoughts.trim().lines() {
                    out.push_str(&format!("> {line}\n"));
                }
                out.push('\n');
            }
        }

        out.push_str(&format!("## Response {}\n\n", turn.index));
        out.push_str(turn.response.trim());
    }
    out
}

/// Parses canonical or heuristic turn Markdown into a list of `ClipTurn`s.
pub fn markdown_to_turns(markdown: &str) -> Vec<ClipTurn> {
    enum Section {
        None,
        Prompt,
        Thoughts,
        Response,
    }
    let mut turns = Vec::new();
    let mut current_prompt = String::new();
    let mut current_response = String::new();
    let mut current_thoughts = String::new();
    let mut turn_index = 1;
    let mut section = Section::None;

    let flush = |turns: &mut Vec<ClipTurn>,
                 idx: &mut usize,
                 prompt: &mut String,
                 thoughts: &mut String,
                 resp: &mut String| {
        let p = prompt.trim().to_string();
        let r = resp.trim().to_string();
        let t = thoughts.trim();
        let th = if t.is_empty() {
            None
        } else {
            Some(t.to_string())
        };
        if !p.is_empty() || !r.is_empty() || th.is_some() {
            turns.push(ClipTurn {
                index: *idx,
                prompt: p,
                thoughts: th,
                response: r,
            });
            *idx += 1;
        }
        prompt.clear();
        thoughts.clear();
        resp.clear();
    };

    for line in markdown.lines() {
        let trimmed = line.trim();
        if trimmed == "---" {
            flush(
                &mut turns,
                &mut turn_index,
                &mut current_prompt,
                &mut current_thoughts,
                &mut current_response,
            );
            section = Section::None;
            continue;
        }
        if trimmed.starts_with("## Prompt") {
            if matches!(section, Section::Response) {
                flush(
                    &mut turns,
                    &mut turn_index,
                    &mut current_prompt,
                    &mut current_thoughts,
                    &mut current_response,
                );
            }
            section = Section::Prompt;
            continue;
        }
        if trimmed.starts_with("> **Thoughts**:")
            || trimmed.starts_with("> [!NOTE] Thoughts")
            || trimmed.starts_with("> Thoughts:")
        {
            section = Section::Thoughts;
            continue;
        }
        if trimmed.starts_with("## Response") {
            section = Section::Response;
            continue;
        }

        match section {
            Section::None => {
                if !trimmed.is_empty() {
                    section = Section::Prompt;
                    current_prompt.push_str(line);
                    current_prompt.push('\n');
                }
            }
            Section::Prompt => {
                current_prompt.push_str(line);
                current_prompt.push('\n');
            }
            Section::Thoughts => {
                if let Some(stripped) = trimmed.strip_prefix('>') {
                    let s = stripped.trim();
                    if !s.is_empty() && !s.starts_with("[!NOTE]") && !s.starts_with("**Thoughts**")
                    {
                        current_thoughts.push_str(s);
                        current_thoughts.push('\n');
                    }
                } else if trimmed.is_empty() {
                    current_thoughts.push('\n');
                } else {
                    section = Section::Response;
                    current_response.push_str(line);
                    current_response.push('\n');
                }
            }
            Section::Response => {
                current_response.push_str(line);
                current_response.push('\n');
            }
        }
    }

    flush(
        &mut turns,
        &mut turn_index,
        &mut current_prompt,
        &mut current_thoughts,
        &mut current_response,
    );

    if turns.is_empty() {
        turns.push(ClipTurn {
            index: 1,
            prompt: String::new(),
            thoughts: None,
            response: markdown.trim().to_string(),
        });
    }

    turns
}

/// Save a clip document to comms via native socket.
pub fn save_clip(
    url: &str,
    title: &str,
    behavior: &str,
    markdown: &str,
    turns: &[ClipTurn],
    extra_payload: Option<Value>,
) -> Result<String, String> {
    let mut payload = json!({
        "version": 1,
        "behavior": behavior,
        "title": title,
        "markdown": markdown,
        "destination": { "path": "Clippings", "name": title },
        "turns": turns,
        "source": { "client": "sjel-gpui" }
    });
    if let Some(extra) = extra_payload {
        if let Some(map) = extra.as_object() {
            if let Some(payload_map) = payload.as_object_mut() {
                for (k, v) in map {
                    payload_map.insert(k.clone(), v.clone());
                }
            }
        }
    }
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default();
    let request_id = format!("clip-{millis:x}");
    let request = json!({
        "type": "save",
        "clip": {
            "request_id": request_id,
            "url": url,
            "payload": payload,
        }
    });
    let answer = ask(request)?;
    answer["document_key"]
        .as_str()
        .or_else(|| answer["request_id"].as_str())
        .map(str::to_owned)
        .ok_or_else(|| "Saved, but comms returned no document key".to_string())
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

    #[test]
    fn audit_citations_identifies_resolved_malformed_and_unlinked_refs() {
        let text = "Here is [Mistral](https://mistral.ai) and footnote [1](https://example.com/one) plus bare [2] and bad [bad](ftp://foo).";
        let citations = audit_citations(text);
        assert_eq!(citations.len(), 4);

        let mistral = &citations[0];
        assert_eq!(mistral.label, "Mistral");
        assert_eq!(mistral.status, CitationStatus::Resolved);
        assert_eq!(mistral.url.as_deref(), Some("https://mistral.ai"));

        let one = &citations[1];
        assert_eq!(one.label, "1");
        assert_eq!(one.status, CitationStatus::Resolved);
        assert_eq!(one.url.as_deref(), Some("https://example.com/one"));

        let bad = &citations[2];
        assert_eq!(bad.label, "bad");
        assert_eq!(bad.status, CitationStatus::MalformedUrl);

        let bare = &citations[3];
        assert_eq!(bare.marker, "[2]");
        assert_eq!(bare.label, "2");
        assert_eq!(bare.status, CitationStatus::UnlinkedRef);
        assert_eq!(bare.url, None);
    }

    #[test]
    fn turns_round_trip_cleanly_through_markdown() {
        let turns = vec![
            ClipTurn {
                index: 1,
                prompt: "Compare LLMs".into(),
                thoughts: Some("Comparing models...".into()),
                response: "OpenAI vs Anthropic [1](https://anthropic.com)".into(),
            },
            ClipTurn {
                index: 2,
                prompt: "Tell me more".into(),
                thoughts: None,
                response: "Details here".into(),
            },
        ];
        let md = turns_to_markdown(&turns);
        assert!(md.contains("## Prompt 1"));
        assert!(md.contains("## Response 1"));
        assert!(md.contains("> **Thoughts**:\n> Comparing models..."));
        assert!(md.contains("## Prompt 2"));
        assert!(md.contains("## Response 2"));

        let parsed = markdown_to_turns(&md);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].prompt, "Compare LLMs");
        assert_eq!(parsed[0].thoughts.as_deref(), Some("Comparing models..."));
        assert_eq!(
            parsed[0].response,
            "OpenAI vs Anthropic [1](https://anthropic.com)"
        );
        assert_eq!(parsed[1].prompt, "Tell me more");
        assert_eq!(parsed[1].thoughts, None);
        assert_eq!(parsed[1].response, "Details here");
    }

    #[test]
    fn html_to_markdown_converts_rich_formatting_and_citations() {
        let html = r#"
            <p>Building a local-first system like <strong>Sjel</strong> sits right at the fault line.</p>
            <p>The most important takeaway: Paying for a subscription (<a href="https://openai.com">[1]</a><a href="https://anthropic.com">[2]</a>) does not buy API privacy.</p>
            <h3>1. Comparison of Selected Provider Plans</h3>
            <ul>
                <li>Mistral: <em>available</em> via <a href="https://mistral.ai">Mistral La Plateforme</a></li>
            </ul>
            <pre><code>let x = 42;</code></pre>
        "#;
        let md = html_to_markdown(html);
        assert!(md.contains("**Sjel**"));
        assert!(md.contains("[1](https://openai.com)"));
        assert!(md.contains("[2](https://anthropic.com)"));
        assert!(md.contains("### 1. Comparison of Selected Provider Plans"));
        assert!(md.contains("*available*"));
        assert!(md.contains("[Mistral La Plateforme](https://mistral.ai)"));
        assert!(md.contains("```\nlet x = 42;\n```"));

        let citations = audit_citations(&md);
        assert_eq!(citations.len(), 3);
        assert_eq!(citations[0].label, "1");
        assert_eq!(citations[0].marker, "[1]");
        assert_eq!(citations[0].status, CitationStatus::Resolved);
        assert_eq!(citations[0].url.as_deref(), Some("https://openai.com"));
        assert_eq!(citations[1].label, "2");
        assert_eq!(citations[1].marker, "[2]");
        assert_eq!(citations[1].status, CitationStatus::Resolved);
        assert_eq!(citations[2].label, "Mistral La Plateforme");
        assert_eq!(citations[2].status, CitationStatus::Resolved);
    }

    #[test]
    fn html_to_markdown_converts_tables_with_citations() {
        let html = r#"
            <table>
                <thead>
                    <tr>
                        <th>Provider &amp; Variant</th>
                        <th>Price (Monthly)</th>
                        <th>Models</th>
                    </tr>
                </thead>
                <tbody>
                    <tr>
                        <td>OpenAI (Sub)</td>
                        <td>Go: $8/mo<br>Plus: $20/mo</td>
                        <td>GPT-5.6 Terra<a href="https://openai.com/terra">[1]</a></td>
                    </tr>
                    <tr>
                        <td>Anthropic</td>
                        <td>$20/mo</td>
                        <td>Claude 3.7 Sonnet<a href="https://anthropic.com">[2]</a></td>
                    </tr>
                </tbody>
            </table>
        "#;
        let md = html_to_markdown(html);
        assert!(md.contains("| Provider & Variant | Price (Monthly) | Models |"));
        assert!(md.contains("| --- | --- | --- |"));
        assert!(md.contains("| OpenAI (Sub) | Go: $8/mo<br>Plus: $20/mo | GPT-5.6 Terra[1](https://openai.com/terra) |"));
        assert!(md.contains("| Anthropic | $20/mo | Claude 3.7 Sonnet[2](https://anthropic.com) |"));

        let citations = audit_citations(&md);
        assert_eq!(citations.len(), 2);
        assert_eq!(citations[0].label, "1");
        assert_eq!(
            citations[0].url.as_deref(),
            Some("https://openai.com/terra")
        );
        assert_eq!(citations[1].label, "2");
        assert_eq!(citations[1].url.as_deref(), Some("https://anthropic.com"));
    }

    #[test]
    fn unwrap_google_redirect_decodes_target_url() {
        let nested = "https://www.google.com/url?sa=E&q=https%3A%2F%2Fvertexaisearch.cloud.google.com%2Fgrounding-api-redirect%2FAUZIYQHr1rAgghLiyvOlnmUtStinv7TEPq4h4uiU_npZJTTJaXGTfJ1mVqtuKqFeueTPcQ31VW1okiJnEvXNPopRmYCf2-_giaFIqyH0xgI0I-fgVAgh5F7FmJa0gEYSzuTltfs%3D";
        let unwrapped = unwrap_google_redirect(nested);
        assert_eq!(
            unwrapped,
            "https://vertexaisearch.cloud.google.com/grounding-api-redirect/AUZIYQHr1rAgghLiyvOlnmUtStinv7TEPq4h4uiU_npZJTTJaXGTfJ1mVqtuKqFeueTPcQ31VW1okiJnEvXNPopRmYCf2-_giaFIqyH0xgI0I-fgVAgh5F7FmJa0gEYSzuTltfs="
        );

        let direct = "https://github.com/larsboes/Sjel";
        assert_eq!(unwrap_google_redirect(direct), direct);

        let md_with_google = format!("Source [1]({nested})");
        let citations = audit_citations(&md_with_google);
        assert_eq!(citations.len(), 1);
        assert_eq!(
            citations[0].url.as_deref(),
            Some("https://vertexaisearch.cloud.google.com/grounding-api-redirect/AUZIYQHr1rAgghLiyvOlnmUtStinv7TEPq4h4uiU_npZJTTJaXGTfJ1mVqtuKqFeueTPcQ31VW1okiJnEvXNPopRmYCf2-_giaFIqyH0xgI0I-fgVAgh5F7FmJa0gEYSzuTltfs=")
        );
    }
}
