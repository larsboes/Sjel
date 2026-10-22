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
