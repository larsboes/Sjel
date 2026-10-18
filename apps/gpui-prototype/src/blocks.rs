//! A note as blocks: the markdown body split into the units the editor shows one textarea for.
//!
//! Each block keeps its markdown prefix verbatim (`- [ ] `, `  * `, `### `), so `Doc::markdown`
//! writes back exactly the bytes `Doc::parse` read. A note that does not round-trip opens
//! read-only (`Doc::lossless`), so an edit can never rewrite text the editor did not show.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Paragraph,
    Heading(u8),
    Bullet,
    Numbered,
    Todo(bool),
    Quote,
    /// A fenced code block, fences included, edited as raw text.
    Code,
}

impl Kind {
    /// List-like blocks follow each other without a blank line.
    pub fn is_list(self) -> bool {
        matches!(self, Self::Bullet | Self::Numbered | Self::Todo(_))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub kind: Kind,
    /// The markdown before the text, verbatim. Empty for a paragraph and a code block.
    pub prefix: String,
    /// The text the user edits. A paragraph's continuation lines stay in it, joined by `\n`.
    pub text: String,
    /// Blank lines before this block.
    pub gap: usize,
}

impl Block {
    pub fn paragraph(text: impl Into<String>, gap: usize) -> Self {
        Self {
            kind: Kind::Paragraph,
            prefix: String::new(),
            text: text.into(),
            gap,
        }
    }

    /// The block that Enter at the end of this one starts: a list continues, anything else
    /// becomes a paragraph.
    pub fn next(&self, text: String) -> Self {
        let (kind, prefix) = match self.kind {
            Kind::Todo(_) => (
                Kind::Todo(false),
                self.prefix
                    .replacen("[x]", "[ ]", 1)
                    .replacen("[X]", "[ ]", 1),
            ),
            Kind::Bullet => (Kind::Bullet, self.prefix.clone()),
            Kind::Numbered => (Kind::Numbered, next_number(&self.prefix)),
            _ => (Kind::Paragraph, String::new()),
        };
        Self {
            kind,
            prefix,
            text,
            gap: if kind.is_list() { 0 } else { 1 },
        }
    }

    /// `# `, `- `, `[] ` and the like typed at the start of a paragraph turn it into that kind.
    pub fn apply_shortcut(&mut self) -> bool {
        if self.kind != Kind::Paragraph || self.text.contains('\n') {
            return false;
        }
        const SHORTCUTS: &[(&str, &str)] = &[
            ("# ", "# "),
            ("## ", "## "),
            ("### ", "### "),
            ("- ", "- "),
            ("* ", "- "),
            ("[] ", "- [ ] "),
            ("[ ] ", "- [ ] "),
            ("1. ", "1. "),
            ("> ", "> "),
        ];
        let Some((typed, prefix)) = SHORTCUTS
            .iter()
            .find(|(typed, _)| self.text.starts_with(typed))
        else {
            return false;
        };
        self.text.drain(..typed.len());
        self.kind = start(prefix).map_or(Kind::Paragraph, |(kind, _)| kind);
        self.prefix = (*prefix).to_owned();
        true
    }

    /// Turn this block into `kind`, keeping its text. Code wraps the text in a fence.
    pub fn set_kind(&mut self, kind: Kind) {
        if self.kind == Kind::Code {
            // Unwrap the fence lines, keep what was between them.
            let inner: Vec<&str> = self.text.lines().collect();
            let end = inner.len().saturating_sub(1).max(1);
            self.text = inner.get(1..end).map(|l| l.join("\n")).unwrap_or_default();
        }
        self.prefix = match kind {
            Kind::Paragraph | Kind::Code => String::new(),
            Kind::Heading(level) => format!("{} ", "#".repeat(level.into())),
            Kind::Bullet => "- ".into(),
            Kind::Numbered => "1. ".into(),
            Kind::Todo(true) => "- [x] ".into(),
            Kind::Todo(false) => "- [ ] ".into(),
            Kind::Quote => "> ".into(),
        };
        if kind == Kind::Code {
            self.text = format!("```\n{}\n```", self.text);
        }
        self.kind = kind;
    }

    pub fn toggle_done(&mut self) {
        if let Kind::Todo(done) = self.kind {
            self.kind = Kind::Todo(!done);
            self.prefix = if done {
                self.prefix
                    .replacen("[x]", "[ ]", 1)
                    .replacen("[X]", "[ ]", 1)
            } else {
                self.prefix.replacen("[ ]", "[x]", 1)
            };
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Doc {
    /// The frontmatter with its closing `---` line, or empty.
    pub front: String,
    pub blocks: Vec<Block>,
    /// Blank lines after the last block. One is the file's final newline.
    pub trailing: usize,
}

impl Doc {
    pub fn parse(content: &str) -> Self {
        let (front, body) = split_front(content);
        let mut lines = body.split('\n').peekable();
        let mut blocks: Vec<Block> = Vec::new();
        let mut gap = 0;
        while let Some(line) = lines.next() {
            if line.is_empty() {
                gap += 1;
                continue;
            }
            let fence = line.trim_start();
            if fence.starts_with("```") || fence.starts_with("~~~") {
                let marker = &fence[..3];
                let mut text = line.to_owned();
                for next in lines.by_ref() {
                    text.push('\n');
                    text.push_str(next);
                    if next.trim_start().starts_with(marker) {
                        break;
                    }
                }
                blocks.push(Block {
                    kind: Kind::Code,
                    prefix: String::new(),
                    text,
                    gap,
                });
            } else {
                let (kind, prefix_len) = start(line).unwrap_or((Kind::Paragraph, 0));
                let mut text = line[prefix_len..].to_owned();
                // Headings are one line; every other block takes the plain lines under it.
                if !matches!(kind, Kind::Heading(_)) {
                    while let Some(next) = lines.peek() {
                        let opens = next.trim_start();
                        if next.is_empty()
                            || start(next).is_some()
                            || opens.starts_with("```")
                            || opens.starts_with("~~~")
                        {
                            break;
                        }
                        text.push('\n');
                        text.push_str(next);
                        lines.next();
                    }
                }
                blocks.push(Block {
                    kind,
                    prefix: line[..prefix_len].to_owned(),
                    text,
                    gap,
                });
            }
            gap = 0;
        }
        // `split` yields one empty string after a final newline, which counted as a gap.
        Self {
            front: front.to_owned(),
            blocks,
            trailing: gap,
        }
    }

    pub fn markdown(&self) -> String {
        let mut lines: Vec<String> = Vec::new();
        for block in &self.blocks {
            lines.extend(std::iter::repeat_n(String::new(), block.gap));
            lines.push(format!("{}{}", block.prefix, block.text));
        }
        lines.extend(std::iter::repeat_n(String::new(), self.trailing));
        format!("{}{}", self.front, lines.join("\n"))
    }

    /// A paragraph right under a list item, quote or paragraph with no blank line between reads
    /// back as part of that block. After a block moves or changes kind, give it the blank line.
    pub fn repair_gaps(&mut self) {
        for index in 1..self.blocks.len() {
            let above = self.blocks[index - 1].kind;
            let block = &mut self.blocks[index];
            if block.kind == Kind::Paragraph
                && block.gap == 0
                && !matches!(above, Kind::Heading(_) | Kind::Code)
            {
                block.gap = 1;
            }
        }
    }

    /// True when saving this document unchanged writes back exactly `content`.
    pub fn lossless(&self, content: &str) -> bool {
        self.markdown() == content
    }
}

/// The frontmatter, closing `---` line included, and the body after it.
fn split_front(content: &str) -> (&str, &str) {
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

/// The kind a line opens and the byte length of its prefix, or `None` for a plain line.
fn start(line: &str) -> Option<(Kind, usize)> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    let rest = &line[indent..];
    let hashes = rest.bytes().take_while(|&b| b == b'#').count();
    if indent == 0 && (1..=6).contains(&hashes) && rest[hashes..].starts_with(' ') {
        return Some((Kind::Heading(hashes as u8), hashes + 1));
    }
    if indent == 0 && rest.starts_with("> ") {
        return Some((Kind::Quote, 2));
    }
    for marker in ["- ", "* ", "+ "] {
        if let Some(after) = rest.strip_prefix(marker) {
            for (check, done) in [("[ ] ", false), ("[x] ", true), ("[X] ", true)] {
                if after.starts_with(check) {
                    return Some((Kind::Todo(done), indent + 2 + check.len()));
                }
            }
            return Some((Kind::Bullet, indent + 2));
        }
    }
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    if (1..=9).contains(&digits) && rest[digits..].starts_with(". ") {
        return Some((Kind::Numbered, indent + digits + 2));
    }
    None
}

fn next_number(prefix: &str) -> String {
    let indent = prefix.len() - prefix.trim_start().len();
    let number: u32 = prefix.trim().trim_end_matches('.').parse().unwrap_or(0);
    format!("{}{}. ", &prefix[..indent], number + 1)
}

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
