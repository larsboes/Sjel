//! A note as blocks: the markdown body split into the units the editor shows one textarea for.
//!
//! Re-exported from `sjel_gpui_core::blocks` so core and iOS share the exact same markdown AST.

pub use sjel_gpui_core::blocks::*;

#[cfg(test)]
mod tests {
    use super::*;

    const NOTE: &str = "---\ntype: note\ntitle: \"Hub\"\n---\n# Hub\n\nSome text\nthat wraps.\n\n- one\n  - nested\n- [ ] todo\n- [x] done\n1. first\n2. second\n\n> quoted\n\n```rust\nfn main() {}\n\nlet x = 1;\n```\n| a | b |\n|---|---|\n\n\nEnd";

    #[test]
    fn notes_round_trip_byte_for_byte() {
        for content in [
            NOTE,
            "",
            "\n",
            "plain",
            "plain\n",
            "\n\nlead\n\n\n",
            "---\nunclosed",
            "   \nspaces only\n",
            "```\nnever closed\n",
            "#notaheading\n####### seven\n",
        ] {
            let doc = Doc::parse(content);
            assert!(
                doc.lossless(content),
                "{content:?} became {:?}",
                doc.markdown()
            );
        }
    }

    #[test]
    fn kinds_are_read_from_prefixes() {
        let doc = Doc::parse(NOTE);
        let kinds: Vec<Kind> = doc.blocks.iter().map(|b| b.kind).collect();
        assert_eq!(
            kinds,
            [
                Kind::Heading(1),
                Kind::Paragraph,
                Kind::Bullet,
                Kind::Bullet,
                Kind::Todo(false),
                Kind::Todo(true),
                Kind::Numbered,
                Kind::Numbered,
                Kind::Quote,
                Kind::Code,
                Kind::Paragraph,
                Kind::Paragraph,
            ]
        );
        assert_eq!(doc.blocks[1].text, "Some text\nthat wraps.");
        assert_eq!(doc.blocks[3].prefix, "  - ");
        assert_eq!(doc.blocks[0].text, "Hub");
    }

    #[test]
    fn enter_continues_a_list_and_ends_a_heading() {
        let doc = Doc::parse("- [x] done\n7. seventh\n# Title\n");
        let todo = doc.blocks[0].next("x".into());
        assert_eq!(
            (todo.kind, todo.prefix.as_str(), todo.gap),
            (Kind::Todo(false), "- [ ] ", 0)
        );
        assert_eq!(doc.blocks[1].next(String::new()).prefix, "8. ");
        let after = doc.blocks[2].next(String::new());
        assert_eq!((after.kind, after.gap), (Kind::Paragraph, 1));
    }

    #[test]
    fn shortcuts_turn_a_paragraph_into_a_kind() {
        let mut block = Block::paragraph("[] buy milk", 1);
        assert!(block.apply_shortcut());
        assert_eq!(
            (block.kind, block.prefix.as_str(), block.text.as_str()),
            (Kind::Todo(false), "- [ ] ", "buy milk")
        );
        block.toggle_done();
        assert_eq!(
            (block.kind, block.prefix.as_str()),
            (Kind::Todo(true), "- [x] ")
        );
        let mut heading = Block::paragraph("## Plan", 0);
        assert!(heading.apply_shortcut());
        assert_eq!(heading.kind, Kind::Heading(2));
        assert!(!Block::paragraph("plain #", 0).apply_shortcut());
    }

    #[test]
    fn set_kind_rewrites_the_prefix_and_code_round_trips() {
        let mut block = Block::paragraph("text", 1);
        block.set_kind(Kind::Heading(2));
        assert_eq!(
            (block.prefix.as_str(), block.text.as_str()),
            ("## ", "text")
        );
        block.set_kind(Kind::Code);
        assert_eq!(
            (block.prefix.as_str(), block.text.as_str()),
            ("", "```\ntext\n```")
        );
        block.set_kind(Kind::Todo(false));
        assert_eq!(
            (block.prefix.as_str(), block.text.as_str()),
            ("- [ ] ", "text")
        );
    }

    #[test]
    fn repair_gaps_keeps_a_moved_paragraph_its_own_block() {
        let mut doc = Doc::parse("# H\ntext\n- item\n");
        // A paragraph straight under a heading is fine; under a list item it is not.
        doc.blocks.swap(0, 2);
        doc.repair_gaps();
        let reread = Doc::parse(&doc.markdown());
        assert_eq!(reread.blocks.len(), 3, "{:?}", doc.markdown());
        let mut fine = Doc::parse("# H\ntext\n");
        fine.repair_gaps();
        assert_eq!(fine.markdown(), "# H\ntext\n");
    }

    /// `SJEL_NOTES_DIR=<dir> cargo test real_notes -- --ignored`: every `.md` under the
    /// directory must round-trip. Prints counts only, never note text.
    #[test]
    #[ignore]
    fn real_notes_round_trip() {
        let dir = std::env::var("SJEL_NOTES_DIR").expect("SJEL_NOTES_DIR");
        let mut stack = vec![std::path::PathBuf::from(dir)];
        let (mut checked, mut failed) = (0, Vec::new());
        while let Some(path) = stack.pop() {
            for entry in std::fs::read_dir(&path).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "md") {
                    let content = std::fs::read_to_string(&path).unwrap();
                    checked += 1;
                    if !Doc::parse(&content).lossless(&content) {
                        failed.push(path);
                    }
                }
            }
        }
        println!("{checked} notes checked, {} not lossless", failed.len());
        assert!(failed.is_empty(), "{failed:?}");
    }
}
