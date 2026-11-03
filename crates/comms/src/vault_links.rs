//! Explicit Obsidian link-source scanner.
//!
//! This reads only configured Markdown files. A source may name one heading;
//! when that heading is absent, the scan returns no links. Fenced code,
//! frontmatter and credential-looking URLs are excluded before anything can be
//! handed to the network-facing media importer.

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use crate::config::VaultLinkSourceConfig;
use crate::store::{canonical_url, feed_id};

#[derive(Debug, Clone)]
pub struct VaultLinkCandidate {
    pub id: String,
    pub source_id: String,
    pub source_ref: String,
    pub label: Option<String>,
    pub url: String,
}

pub fn scan(sources: &[VaultLinkSourceConfig]) -> Vec<VaultLinkCandidate> {
    let mut seen = HashSet::new();
    let mut candidates = Vec::new();
    for source in sources.iter().filter(|source| source.enabled) {
        if source.id.trim().is_empty() || source.path.trim().is_empty() {
            continue;
        }
        for candidate in scan_source(source) {
            if seen.insert((source.id.clone(), canonical_url(&candidate.url))) {
                candidates.push(candidate);
            }
        }
    }
    candidates
}

fn scan_source(source: &VaultLinkSourceConfig) -> Vec<VaultLinkCandidate> {
    let path = Path::new(&source.path);
    if path.extension().and_then(|extension| extension.to_str()) != Some("md") {
        return Vec::new();
    }
    let Ok(metadata) = fs::metadata(path) else {
        return Vec::new();
    };
    if !metadata.is_file() || metadata.len() > 2_000_000 {
        return Vec::new();
    }
    let Ok(body) = fs::read_to_string(path) else {
        return Vec::new();
    };

    let mut in_frontmatter = body.starts_with("---\n");
    let mut in_fence = false;
    let mut active_heading_level = if source.heading.is_none() {
        Some(0)
    } else {
        None
    };
    let requested_heading = source.heading.as_deref().map(normalize_heading);
    let mut candidates = Vec::new();

    for (index, raw) in body.lines().enumerate() {
        let line = raw.trim();
        if index == 0 && in_frontmatter {
            continue;
        }
        if in_frontmatter {
            if line == "---" {
                in_frontmatter = false;
            }
            continue;
        }
        if line.starts_with("```") || line.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }

        if let Some((level, title)) = markdown_heading(line) {
            match active_heading_level {
                Some(active) if active > 0 && level <= active => active_heading_level = None,
                _ => {}
            }
            if requested_heading
                .as_deref()
                .is_some_and(|requested| normalize_heading(title) == requested)
            {
                active_heading_level = Some(level);
            }
            continue;
        }
        if active_heading_level.is_none() {
            continue;
        }

        for (label, url) in links_in_line(line) {
            if !safe_http_url(&url) {
                continue;
            }
            candidates.push(VaultLinkCandidate {
                id: feed_id(&url),
                source_id: source.id.clone(),
                source_ref: format!(
                    "{}:{}",
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("vault-note.md"),
                    index + 1
                ),
                label,
                url,
            });
        }
    }
    candidates
}

fn markdown_heading(line: &str) -> Option<(usize, &str)> {
    let level = line
        .chars()
        .take_while(|character| *character == '#')
        .count();
    if level == 0 || level > 6 {
        return None;
    }
    let title = line.get(level..)?.trim();
    (!title.is_empty()).then_some((level, title))
}

fn normalize_heading(value: &str) -> String {
    value.trim().trim_matches('#').trim().to_lowercase()
}

fn links_in_line(line: &str) -> Vec<(Option<String>, String)> {
    let mut links = Vec::new();
    let mut cursor = 0usize;
    while let Some(relative) = line[cursor..].find("](") {
        let boundary = cursor + relative;
        let Some(end_relative) = line[boundary + 2..].find(')') else {
            break;
        };
        let end = boundary + 2 + end_relative;
        let url = line[boundary + 2..end].trim();
        let label_start = line[..boundary].rfind('[');
        let is_image = label_start.is_some_and(|start| {
            start > 0
                && line
                    .as_bytes()
                    .get(start - 1)
                    .is_some_and(|byte| *byte == b'!')
        });
        let label = label_start
            .map(|start| line[start + 1..boundary].trim().to_string())
            .filter(|value| !value.is_empty());
        if !is_image && (url.starts_with("http://") || url.starts_with("https://")) {
            links.push((label, url.to_string()));
        }
        cursor = end + 1;
    }

    let bytes = line.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        let rest = &line[index..];
        let offset = match (rest.find("https://"), rest.find("http://")) {
            (Some(a), Some(b)) => a.min(b),
            (Some(a), None) | (None, Some(a)) => a,
            (None, None) => break,
        };
        let start = index + offset;
        let inside_inline_code = line[..start].matches('`').count() % 2 == 1;
        let inside_markdown_target = line[..start].ends_with("](");
        let end = line[start..]
            .find(|character: char| {
                character.is_whitespace() || matches!(character, ')' | ']' | '>' | '"' | '\'' | '`')
            })
            .map(|relative| start + relative)
            .unwrap_or(line.len());
        let url = line[start..end]
            .trim_end_matches([',', '.', ';', ':'])
            .to_string();
        if !inside_inline_code
            && !inside_markdown_target
            && !links.iter().any(|(_, existing)| existing == &url)
        {
            links.push((None, url));
        }
        index = end.max(start + 1);
    }
    links
}

fn safe_http_url(url: &str) -> bool {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return false;
    }
    let lower = url.to_lowercase();
    let sensitive = [
        "access_token",
        "api_key",
        "apikey",
        "auth=",
        "password=",
        "secret=",
        "session=",
        "/invitation/mailbox/",
    ];
    if sensitive.iter().any(|marker| lower.contains(marker)) {
        return false;
    }
    if lower.contains("://[::1]") {
        return false;
    }
    let host = lower
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or("")
        .split(['/', '?', '#', ':'])
        .next()
        .unwrap_or("");
    let private_host = host == "localhost"
        || host == "0.0.0.0"
        || host == "::1"
        || host.starts_with("127.")
        || host.starts_with("10.")
        || host.starts_with("192.168.")
        || host.ends_with(".local")
        || host
            .strip_prefix("172.")
            .and_then(|rest| rest.split('.').next())
            .and_then(|octet| octet.parse::<u8>().ok())
            .is_some_and(|octet| (16..=31).contains(&octet));
    !private_host
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_markdown_and_bare_links() {
        let links =
            links_in_line("- [Rust](https://blog.rust-lang.org/) and https://example.com/read.");
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].0.as_deref(), Some("Rust"));
        assert_eq!(links[1].1, "https://example.com/read");
    }

    #[test]
    fn credential_urls_are_rejected() {
        assert!(!safe_http_url("https://example.com/?access_token=abc"));
        assert!(safe_http_url("https://example.com/article"));
    }

    #[test]
    fn skips_images_and_inline_code() {
        let links = links_in_line(
            "![cover](https://images.example.org/cover.jpg) `https://admin.example.org/`",
        );
        assert!(links.is_empty());
    }

    #[test]
    fn missing_required_heading_does_not_scan_the_whole_note() {
        let path = std::env::temp_dir().join(format!(
            "axon-vault-links-{}-{}.md",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        fs::write(
            &path,
            "https://outside.example.org/\n## Other\nhttps://also.example.org/",
        )
        .expect("write fixture");
        let source = VaultLinkSourceConfig {
            id: "scratchpad".into(),
            path: path.to_string_lossy().into_owned(),
            heading: Some("Feed Inbox".into()),
            enabled: true,
        };
        assert!(scan_source(&source).is_empty());
        fs::remove_file(path).expect("remove fixture");
    }
}
