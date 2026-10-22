//! Inline markdown for a block that is not being edited: re-exported from `sjel_gpui_core::inline`.

pub use sjel_gpui_core::inline::*;

#[cfg(test)]
mod tests {
    use super::*;

    fn shown(src: &str) -> (String, Vec<(&'static str, String)>) {
        let inline = Inline::parse(src);
        let marks = inline
            .marks
            .iter()
            .map(|(range, mark)| {
                let name = match mark {
                    Mark::Bold => "bold",
                    Mark::Italic => "italic",
                    Mark::Code => "code",
                    Mark::Strike => "strike",
                    Mark::Link => "link",
                    Mark::Highlight => "highlight",
                };
                (name, inline.text[range.clone()].to_owned())
            })
            .collect();
        (inline.text, marks)
    }

    #[test]
    fn markers_hide_and_marks_cover_their_text() {
        assert_eq!(
            shown("a **b** *c* `d*e` ~~f~~ ==g=="),
            (
                "a b c d*e f g".into(),
                vec![
                    ("bold", "b".into()),
                    ("italic", "c".into()),
                    ("code", "d*e".into()),
                    ("strike", "f".into()),
                    ("highlight", "g".into())
                ]
            )
        );
        assert_eq!(
            shown("see [docs](https://x.y) and [[Notes/Plan|the plan]] or [[Inbox]]"),
            (
                "see docs and the plan or Inbox".into(),
                vec![
                    ("link", "docs".into()),
                    ("link", "the plan".into()),
                    ("link", "Inbox".into())
                ]
            )
        );
        assert_eq!(
            shown("**bold [link](u)**").1,
            vec![("link", "link".into()), ("bold", "bold link".into())]
        );
    }

    #[test]
    fn unclosed_and_word_internal_markers_stay_literal() {
        for src in [
            "2 * 3",
            "snake_case_name",
            "**open",
            "`tick",
            "[no link]",
            "a ** b",
        ] {
            assert_eq!(shown(src), (src.to_owned(), vec![]), "{src}");
        }
    }

    #[test]
    fn soft_breaks_flow_and_tables_keep_their_rows() {
        let inline = Inline::parse("one\n  two");
        assert_eq!(inline.text, "one two");
        // `t` sits after the newline and its indent.
        assert_eq!(inline.source_offset(4), 6);
        assert_eq!(Inline::parse("| a |\n| b |").text, "| a |\n| b |");
    }

    #[test]
    fn a_shown_position_maps_back_to_the_source() {
        let inline = Inline::parse("é **bold** end");
        // "é bold end": `b` at shown 3 is source 5, after the opening marker.
        assert_eq!(inline.text, "é bold end");
        assert_eq!(inline.source_offset(3), 5);
        // The end of the text is the end of the source.
        assert_eq!(
            inline.source_offset(inline.text.len()),
            "é **bold** end".len()
        );
        assert_eq!(inline.source_offset(0), 0);
    }
}
