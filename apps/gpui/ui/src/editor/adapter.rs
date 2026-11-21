//! Block editor round-trip parity evaluation comparing `sjel_gpui_core::blocks::Doc` against
//! `ely_gpui_component::documents::{BlockData, BlockKind}`.
//!
//! Sjel requires byte-lossless round-tripping (`Doc::lossless`) before allowing edits to any vault
//! note or task record so that unseen syntax, YAML frontmatter, indentation, and blank-line gaps
//! are never silently rewritten.

use crate::ely::documents::{BlockData, BlockKind};
use sjel_gpui_core::blocks::Doc;

/// Result of comparing a markdown document's round-trip through Sjel's [`Doc`] and Ely's
/// [`BlockData`] (with frontmatter split ahead of Ely's parser).
#[derive(Clone, Debug, PartialEq)]
pub struct RoundTripParityReport {
    pub frontmatter: String,
    pub sjel_lossless: bool,
    pub ely_raw_lossless: bool,
    pub ely_with_frontmatter_lossless: bool,
    pub sjel_block_count: usize,
    pub ely_block_count: usize,
    pub ely_blocks: Vec<BlockData>,
    pub ely_markdown_with_frontmatter: String,
}

/// Split leading YAML frontmatter (`---\n...\n---\n`) verbatim before passing the body to Ely's
/// [`BlockData::from_markdown`], which otherwise treats `---` as a thematic break (`Divider`).
pub fn split_frontmatter(content: &str) -> (&str, &str) {
    let Some(rest) = content.strip_prefix("---\n") else {
        return ("", content);
    };
    let mut at = 4;
    for line in rest.split_inclusive('\n') {
        at += line.len();
        if line.trim_end() == "---" {
            return content.split_at(at);
        }
    }
    ("", content)
}

/// Evaluate round-trip parity for `content` across Sjel's [`Doc`] and Ely's [`BlockData`].
pub fn evaluate_round_trip(content: &str) -> RoundTripParityReport {
    let sjel_doc = Doc::parse(content);
    let sjel_lossless = sjel_doc.lossless(content);

    let ely_raw_blocks = BlockData::from_markdown(content);
    let ely_raw_markdown = BlockData::to_markdown(&ely_raw_blocks);
    let ely_raw_lossless = ely_raw_markdown == content;

    let (front, body) = split_frontmatter(content);
    let ely_blocks = BlockData::from_markdown(body);
    let ely_body_markdown = if body.is_empty() {
        String::new()
    } else {
        BlockData::to_markdown(&ely_blocks)
    };
    let ely_markdown_with_frontmatter = format!("{front}{ely_body_markdown}");
    let ely_with_frontmatter_lossless = ely_markdown_with_frontmatter == content;

    RoundTripParityReport {
        frontmatter: front.to_owned(),
        sjel_lossless,
        ely_raw_lossless,
        ely_with_frontmatter_lossless,
        sjel_block_count: sjel_doc.blocks.len(),
        ely_block_count: ely_blocks.len(),
        ely_blocks,
        ely_markdown_with_frontmatter,
    }
}

/// Whether `content` can be edited through Ely's [`BlockData`] without altering a single byte.
pub fn ely_is_byte_lossless(content: &str) -> bool {
    evaluate_round_trip(content).ely_with_frontmatter_lossless
}

/// Map an Ely [`BlockKind`] onto the block kind Enter opens after it, mirroring split behavior.
pub fn next_kind_after_split(kind: &BlockKind) -> BlockKind {
    kind.next()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sjel_gpui_core::blocks::{Block, Kind};

    #[test]
    fn frontmatter_requires_pre_splitting_before_ely_markdown_import() {
        let note = "---\ntype: note\ntitle: \"Hub\"\n---\n# Hub\n\nParagraph text.\n";
        let report = evaluate_round_trip(note);
        assert!(
            report.sjel_lossless,
            "Sjel Doc preserves YAML frontmatter verbatim"
        );
        assert!(
            !report.ely_raw_lossless,
            "Raw Ely BlockData::from_markdown parses --- as Divider and corrupts frontmatter"
        );
        assert!(
            report.ely_with_frontmatter_lossless,
            "Pre-splitting frontmatter makes canonical heading + paragraph lossless in Ely"
        );
        assert_eq!(report.frontmatter, "---\ntype: note\ntitle: \"Hub\"\n---\n");
    }

    #[test]
    fn nested_lists_and_alternate_markers_flatten_in_ely_but_stay_lossless_in_sjel() {
        let nested = "- parent\n  - child\n  * star child\n- [ ] task\n- [x] done\n";
        let report = evaluate_round_trip(nested);
        assert!(
            report.sjel_lossless,
            "Sjel Doc keeps indentation and * markers in Block::prefix"
        );
        assert!(
            !report.ely_with_frontmatter_lossless,
            "Ely flattens nested list items and normalizes * to -"
        );
        assert!(
            !ely_is_byte_lossless(nested),
            "Byte-lossless gate refuses Ely write-back on nested lists"
        );
    }

    #[test]
    fn gfm_tables_normalize_delimiters_in_ely_unless_canonical() {
        let compact_table = "# Table\n\n| Frage | Stand |\n|---|---|\n| Master | Geprüft |\n";
        let compact_report = evaluate_round_trip(compact_table);
        assert!(compact_report.sjel_lossless);
        assert!(
            !compact_report.ely_with_frontmatter_lossless,
            "Ely rewrites |---|---| to | --- | --- |"
        );

        let canonical_table = "# Table\n\n| Frage | Stand |\n| --- | --- |\n| Master | Geprüft |\n";
        let canonical_report = evaluate_round_trip(canonical_table);
        assert!(canonical_report.sjel_lossless);
        assert!(canonical_report.ely_with_frontmatter_lossless);
        assert!(matches!(
            canonical_report.ely_blocks[1].kind,
            BlockKind::Table { columns: 2, .. }
        ));
    }

    #[test]
    fn code_fences_with_arbitrary_languages_and_gap_normalization_differ_between_engines() {
        let rust_code = "```rust\nfn main() {}\n```\n";
        let code_report = evaluate_round_trip(rust_code);
        assert!(code_report.sjel_lossless);
        assert!(
            !code_report.ely_with_frontmatter_lossless,
            "Ely BlockKind::Code drops non-mermaid/math fence language tags on export"
        );

        let mermaid_code = "```mermaid\ngraph LR; A-->B;\n```\n";
        let mermaid_report = evaluate_round_trip(mermaid_code);
        assert!(mermaid_report.sjel_lossless);
        assert!(mermaid_report.ely_with_frontmatter_lossless);
        assert_eq!(mermaid_report.ely_blocks[0].kind, BlockKind::Diagram);

        let toggle_html =
            "<details open>\n<summary>Details</summary>\n\nHidden body\n\n</details>\n";
        let toggle_report = evaluate_round_trip(toggle_html);
        assert!(toggle_report.sjel_lossless);
        assert!(toggle_report.ely_with_frontmatter_lossless);
        assert_eq!(toggle_report.ely_blocks[0].kind, BlockKind::Toggle(true));

        let multi_gap = "First paragraph\n\n\nSecond paragraph\n";
        let gap_report = evaluate_round_trip(multi_gap);
        assert!(
            gap_report.sjel_lossless,
            "Sjel Doc preserves exact blank line count in Block::gap"
        );
        assert!(
            !gap_report.ely_with_frontmatter_lossless,
            "Ely BlockData::to_markdown normalizes inter-block blank lines to a single blank line"
        );
    }

    #[test]
    fn split_merge_and_slash_menu_semantics_align_on_core_prose_kinds() {
        // Enter after list items continues the list; Enter after a heading starts a paragraph.
        assert_eq!(next_kind_after_split(&BlockKind::Bullet), BlockKind::Bullet);
        assert_eq!(
            next_kind_after_split(&BlockKind::Numbered),
            BlockKind::Numbered
        );
        assert_eq!(
            next_kind_after_split(&BlockKind::Todo(true)),
            BlockKind::Todo(false)
        );
        assert_eq!(
            next_kind_after_split(&BlockKind::Heading(2)),
            BlockKind::Paragraph
        );

        let sjel_todo = Block {
            kind: Kind::Todo(true),
            prefix: "- [x] ".into(),
            text: "done".into(),
            gap: 0,
        };
        assert_eq!(sjel_todo.next(String::new()).kind, Kind::Todo(false));

        // Sjel's slash query parser triggers only on a leading '/' in a single-line paragraph.
        assert_eq!(
            crate::block_menu::slash_query(Kind::Paragraph, "/h2"),
            Some("h2")
        );
        assert_eq!(
            crate::block_menu::slash_query(Kind::Heading(1), "/h2"),
            None
        );
        assert!(
            !crate::block_menu::matching("h2").is_empty(),
            "slash query matches Heading 2"
        );
    }
}
