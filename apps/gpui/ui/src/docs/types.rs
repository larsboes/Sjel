//! Documentation data sources, block kinds, anchor indexing, and embedded manual data.

use sjel_gpui_core::blocks::{Block, Kind};
use sjel_gpui_core::inline::Inline;

pub(crate) struct DocSource {
    pub(crate) slug: &'static str,
    pub(crate) category: &'static str,
    pub(crate) file: &'static str,
    pub(crate) markdown: &'static str,
}

pub(crate) enum MarkdownBlock {
    Content(Block),
    Table {
        headers: Vec<String>,
        rows: Vec<Vec<String>>,
    },
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

pub(crate) fn anchor_index(blocks: &[MarkdownBlock], target: &str) -> Option<usize> {
    let mut seen = std::collections::HashSet::new();
    blocks.iter().enumerate().find_map(|(index, block)| {
        let MarkdownBlock::Content(block) = block else {
            return None;
        };
        let id = match block.kind {
            Kind::Heading(_) => Some(heading_id(&block.text, &mut seen)),
            _ => explicit_anchor(&block.text),
        }?;
        (id == target).then_some(index)
    })
}

pub(crate) fn table_cells(line: &str) -> Vec<String> {
    line.trim()
        .trim_start_matches('|')
        .trim_end_matches('|')
        .split('|')
        .map(|cell| cell.trim().to_owned())
        .collect()
}

pub(crate) fn is_table_separator(line: &str) -> bool {
    line.contains('|')
        && line.contains('-')
        && line
            .chars()
            .all(|ch| ch.is_whitespace() || matches!(ch, '|' | ':' | '-'))
}
