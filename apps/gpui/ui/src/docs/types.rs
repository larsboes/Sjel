//! Documentation data sources, anchor indexing, and embedded manual metadata.

use sjel_gpui_core::inline::Inline;

pub(crate) struct DocSource {
    pub(crate) slug: &'static str,
    pub(crate) category: &'static str,
    pub(crate) file: &'static str,
    pub(crate) markdown: &'static str,
}

pub(crate) fn title(doc: &DocSource) -> &str {
    doc.markdown
        .lines()
        .find_map(|line| line.strip_prefix("# "))
        .unwrap_or(doc.slug)
}

pub(crate) fn summary(doc: &DocSource) -> &str {
    doc.markdown
        .lines()
        .find(|line| {
            let line = line.trim();
            !line.is_empty()
                && !line.starts_with('#')
                && !line.starts_with('`')
                && !line.starts_with('|')
                && !line.starts_with('>')
                && !line.starts_with('-')
        })
        .unwrap_or("")
        .trim()
}

pub(crate) fn heading_id(text: &str, seen: &mut std::collections::HashSet<String>) -> String {
    let text = text.trim_end_matches(|ch: char| ch == '#' || ch.is_whitespace());
    let inline = Inline::parse(text);
    let base = inline
        .text
        .to_lowercase()
        .chars()
        .filter_map(|ch| {
            if ch.is_alphanumeric() || matches!(ch, '_' | '-') {
                Some(ch)
            } else if ch.is_whitespace() {
                Some('-')
            } else {
                None
            }
        })
        .collect::<String>();
    let mut id = base.clone();
    let mut suffix = 1;
    while seen.contains(&id) {
        id = format!("{base}-{suffix}");
        suffix += 1;
    }
    seen.insert(id.clone());
    id
}

pub(crate) fn explicit_anchor(text: &str) -> Option<String> {
    let id = text
        .trim()
        .strip_prefix("<a id=\"")?
        .strip_suffix("\"></a>")?;
    (!id.is_empty()
        && id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-')))
    .then(|| id.to_owned())
}

pub(crate) fn anchor_source_range(markdown: &str, target: &str) -> Option<std::ops::Range<usize>> {
    let mut seen = std::collections::HashSet::new();
    let mut offset = 0;
    let mut in_fence: Option<&str> = None;
    for line in markdown.split_inclusive('\n') {
        let line_len = line.len();
        let trimmed = line.trim();
        if let Some(marker) = in_fence {
            if trimmed.starts_with(marker) {
                in_fence = None;
            }
            offset += line_len;
            continue;
        }
        if trimmed.starts_with("```") {
            in_fence = Some("```");
            offset += line_len;
            continue;
        }
        if trimmed.starts_with("~~~") {
            in_fence = Some("~~~");
            offset += line_len;
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix('#') {
            let hashes = 1 + rest.bytes().take_while(|&b| b == b'#').count();
            if hashes <= 6 {
                let heading_text = &trimmed[hashes..].trim_start();
                let id = heading_id(heading_text, &mut seen);
                if id == target {
                    return Some(offset..(offset + line_len));
                }
            }
        } else if let Some(id) = explicit_anchor(trimmed) {
            if id == target {
                return Some(offset..(offset + line_len));
            }
        }
        offset += line_len;
    }
    None
}
