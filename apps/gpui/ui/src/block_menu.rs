//! The kinds a block can turn into, for the `/` menu and the block handle, and the filter that
//! narrows them as the user types after `/`.

use sjel_gpui_core::blocks::Kind;

pub struct Choice {
    pub label: &'static str,
    /// Extra words the filter matches, such as `h1`.
    keywords: &'static str,
    pub kind: Kind,
    pub icon: &'static str,
    pub shortcut: &'static str,
}

const fn c(
    label: &'static str,
    keywords: &'static str,
    kind: Kind,
    icon: &'static str,
    shortcut: &'static str,
) -> Choice {
    Choice {
        label,
        keywords,
        kind,
        icon,
        shortcut,
    }
}

pub const CHOICES: &[Choice] = &[
    c("Text", "paragraph plain", Kind::Paragraph, "¶", "text"),
    c("Heading 1", "h1 title", Kind::Heading(1), "H1", "#"),
    c("Heading 2", "h2", Kind::Heading(2), "H2", "##"),
    c("Heading 3", "h3", Kind::Heading(3), "H3", "###"),
    c("Bulleted list", "ul bullet", Kind::Bullet, "•", "-"),
    c("Numbered list", "ol number", Kind::Numbered, "1.", "1."),
    c("To-do", "todo task check", Kind::Todo(false), "☑", "[]"),
    c("Quote", "blockquote", Kind::Quote, "❝", ">"),
    c("Code", "fence snippet", Kind::Code, "‹/›", "```"),
];

/// The choices whose label or keywords contain `query`, ignoring case and spaces.
pub fn matching(query: &str) -> Vec<&'static Choice> {
    let squash = |s: &str| s.to_lowercase().replace([' ', '-'], "");
    let query = squash(query);
    CHOICES
        .iter()
        .filter(|choice| {
            squash(choice.label).contains(&query)
                || choice.keywords.split(' ').any(|word| word.contains(&query))
        })
        .collect()
}

/// The text after `/` while a paragraph holds only a slash command, or `None`.
pub fn slash_query(kind: Kind, text: &str) -> Option<&str> {
    let query = text.strip_prefix('/')?;
    (kind == Kind::Paragraph && query.len() <= 24 && !query.contains('\n')).then_some(query)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(query: &str) -> Vec<&'static str> {
        matching(query).iter().map(|c| c.label).collect()
    }

    #[test]
    fn the_filter_matches_labels_and_keywords() {
        assert_eq!(labels("").len(), CHOICES.len());
        assert_eq!(labels("h2"), ["Heading 2"]);
        assert_eq!(labels("heading1"), ["Heading 1"]);
        assert_eq!(labels("todo"), ["To-do"]);
        assert_eq!(labels("LIST"), ["Bulleted list", "Numbered list"]);
        assert!(labels("zzz").is_empty());
    }

    #[test]
    fn only_a_paragraph_that_starts_with_a_slash_asks() {
        assert_eq!(slash_query(Kind::Paragraph, "/he"), Some("he"));
        assert_eq!(slash_query(Kind::Paragraph, "a/b"), None);
        assert_eq!(slash_query(Kind::Bullet, "/he"), None);
        assert_eq!(slash_query(Kind::Paragraph, "/a\nb"), None);
    }
}
