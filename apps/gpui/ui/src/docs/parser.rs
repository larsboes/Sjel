//! Markdown block splitting, tables, slug resolution, and source URLs for documentation.

use sjel_gpui_core::blocks::Doc;

use super::types::{is_table_separator, table_cells, MarkdownBlock};

pub(crate) fn parse_markdown(content: &str) -> Vec<MarkdownBlock> {
    let lines: Vec<&str> = content.split('\n').collect();
    let mut blocks = Vec::new();
    let mut text = String::new();
    let mut index = 0;
    let mut fence: Option<&str> = None;
    while index < lines.len() {
        let line = lines[index];
        let trimmed = line.trim_start();
        if let Some(marker) = fence {
            text.push_str(line);
            text.push('\n');
            if trimmed.starts_with(marker) {
                fence = None;
            }
            index += 1;
            continue;
        }
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fence = Some(&trimmed[..3]);
            text.push_str(line);
            text.push('\n');
            index += 1;
            continue;
        }
        if index + 1 < lines.len() && line.contains('|') && is_table_separator(lines[index + 1]) {
            if !text.is_empty() {
                blocks.extend(
                    Doc::parse(&text)
                        .blocks
                        .into_iter()
                        .map(MarkdownBlock::Content),
                );
                text.clear();
            }
            let headers = table_cells(line);
            index += 2;
            let mut rows = Vec::new();
            while index < lines.len()
                && lines[index].contains('|')
                && !lines[index].trim().is_empty()
            {
                rows.push(table_cells(lines[index]));
                index += 1;
            }
            blocks.push(MarkdownBlock::Table { headers, rows });
            continue;
        }
        text.push_str(line);
        text.push('\n');
        index += 1;
    }
    if !text.is_empty() {
        blocks.extend(
            Doc::parse(&text)
                .blocks
                .into_iter()
                .map(MarkdownBlock::Content),
        );
    }
    blocks
}

pub(crate) fn doc_slug(href: &str, file: &str) -> Option<String> {
    if href.starts_with('#') || href.starts_with("//") || href.contains("://") {
        return None;
    }
    let path = href.split(['?', '#']).next()?;
    if path.is_empty() {
        return None;
    }
    let mut parts = Vec::new();
    if !path.starts_with('/') {
        if let Some((parent, _)) = file.rsplit_once('/') {
            parts.extend(parent.split('/'));
        }
    }
    for part in path.trim_start_matches('/').split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            value => parts.push(value),
        }
    }
    let joined = parts.join("/");
    let path = joined.strip_prefix("docs/")?;
    let mut slug = path.strip_suffix(".md")?.to_owned();
    if slug == "README" {
        slug = "index".into();
    } else if let Some(parent) = slug.strip_suffix("/index") {
        slug = parent.to_owned();
    }
    Some(slug)
}

pub(crate) fn source_url(href: &str, file: &str) -> Option<String> {
    if href.starts_with("https://") || href.starts_with("http://") || href.starts_with("mailto:") {
        return Some(href.to_owned());
    }
    if href.starts_with('#') || href.starts_with("//") {
        return None;
    }
    let path = href.split(['?', '#']).next()?;
    let mut parts = Vec::new();
    if !path.starts_with('/') {
        if let Some((parent, _)) = file.rsplit_once('/') {
            parts.extend(parent.split('/'));
        }
    }
    for part in path.trim_start_matches('/').split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            value => parts.push(value),
        }
    }
    let suffix = href.find(['?', '#']).map_or("", |at| &href[at..]);
    Some(format!(
        "https://github.com/larsboes/Sjel/blob/main/{}{suffix}",
        parts.join("/")
    ))
}
