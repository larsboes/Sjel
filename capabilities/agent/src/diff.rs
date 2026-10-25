//! A line diff for the approval card.
//!
//! The card shows what will change, not a summary of it. `edit` already carries both the text it
//! removes and the text it writes, and `write` carries the whole file, so the diff is computed
//! from the bytes on disk and the bytes about to replace them — never from the model's account of
//! what it did.
//!
//! No dependency: an LCS over lines, with the common head and tail trimmed first. Trimming is what
//! keeps this cheap in practice, because an edit of ten lines in a file of two thousand leaves a
//! handful of lines to diff. A pair of inputs too unlike each other for the table to be worth it
//! falls back to "all of the old, then all of the new", which is still the truth.

/// How many lines of table the LCS may allocate. Beyond this the fallback answers instead.
const TABLE_LIMIT: usize = 250_000;

#[derive(Debug, PartialEq, Eq)]
pub enum Line<'a> {
    Same(&'a str),
    Removed(&'a str),
    Added(&'a str),
}

/// The lines that differ, oldest first, with a little unchanged context around them.
pub fn lines<'a>(old: &'a str, new: &'a str) -> Vec<Line<'a>> {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();

    let mut head = 0;
    while head < old_lines.len() && head < new_lines.len() && old_lines[head] == new_lines[head] {
        head += 1;
    }
    let mut tail = 0;
    while tail < old_lines.len() - head
        && tail < new_lines.len() - head
        && old_lines[old_lines.len() - 1 - tail] == new_lines[new_lines.len() - 1 - tail]
    {
        tail += 1;
    }

    let old_mid = &old_lines[head..old_lines.len() - tail];
    let new_mid = &new_lines[head..new_lines.len() - tail];

    let mut out: Vec<Line<'a>> = Vec::new();
    // Two lines of context are enough for a reader to see where they are; anything more is the
    // file, and the file is a click away.
    let context = head.saturating_sub(2);
    out.extend(old_lines[context..head].iter().map(|line| Line::Same(line)));
    out.extend(middle(old_mid, new_mid));
    if tail > 0 {
        out.extend(
            old_lines[old_lines.len() - tail..old_lines.len() - tail + tail.min(2)]
                .iter()
                .map(|line| Line::Same(line)),
        );
    }
    out
}

/// The LCS of two slices, as removals and additions around the lines they keep.
fn middle<'a>(old: &[&'a str], new: &[&'a str]) -> Vec<Line<'a>> {
    if old.is_empty() && new.is_empty() {
        return Vec::new();
    }
    if old.is_empty() {
        return new.iter().map(|line| Line::Added(line)).collect();
    }
    if new.is_empty() {
        return old.iter().map(|line| Line::Removed(line)).collect();
    }
    if old.len().saturating_mul(new.len()) > TABLE_LIMIT {
        let mut out: Vec<Line<'a>> = old.iter().map(|line| Line::Removed(line)).collect();
        out.extend(new.iter().map(|line| Line::Added(line)));
        return out;
    }

    // `table[i][j]` is the LCS length of `old[i..]` and `new[j..]`.
    let mut table = vec![vec![0u32; new.len() + 1]; old.len() + 1];
    for i in (0..old.len()).rev() {
        for j in (0..new.len()).rev() {
            table[i][j] = if old[i] == new[j] {
                table[i + 1][j + 1] + 1
            } else {
                table[i + 1][j].max(table[i][j + 1])
            };
        }
    }

    let mut out = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < old.len() && j < new.len() {
        if old[i] == new[j] {
            out.push(Line::Same(old[i]));
            i += 1;
            j += 1;
        } else if table[i + 1][j] >= table[i][j + 1] {
            out.push(Line::Removed(old[i]));
            i += 1;
        } else {
            out.push(Line::Added(new[j]));
            j += 1;
        }
    }
    out.extend(old[i..].iter().map(|line| Line::Removed(line)));
    out.extend(new[j..].iter().map(|line| Line::Added(line)));
    out
}

/// The diff as a card shows it: prefixed lines, capped, and whether anything was cut.
pub fn render(old: &str, new: &str, max_lines: usize) -> (String, bool) {
    let lines = lines(old, new);
    let mut out = String::new();
    let mut shown = 0;
    for line in &lines {
        if shown == max_lines {
            out.push_str(&format!(
                "… {} more line(s) not shown\n",
                lines.len() - max_lines
            ));
            return (out, true);
        }
        match line {
            Line::Same(text) => out.push_str(&format!("  {text}\n")),
            Line::Removed(text) => out.push_str(&format!("- {text}\n")),
            Line::Added(text) => out.push_str(&format!("+ {text}\n")),
        }
        shown += 1;
    }
    (out, false)
}

/// How many lines were added and removed, for a one-line summary.
pub fn counts(old: &str, new: &str) -> (usize, usize) {
    let mut added = 0;
    let mut removed = 0;
    for line in lines(old, new) {
        match line {
            Line::Added(_) => added += 1,
            Line::Removed(_) => removed += 1,
            Line::Same(_) => {}
        }
    }
    (added, removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_edit_shows_the_line_it_changes_and_nothing_else() {
        let old = "a\nif self.empty {\nb\nc\nd\n";
        let new = "a\nif self.rows.is_empty() {\nb\nc\nd\n";
        let (text, truncated) = render(old, new, 50);
        assert!(!truncated);
        assert!(text.contains("- if self.empty {"), "{text}");
        assert!(text.contains("+ if self.rows.is_empty() {"), "{text}");
        // The one line that changed, with what is next to it and no more: the rest is the file.
        assert_eq!(
            text,
            "  a\n- if self.empty {\n+ if self.rows.is_empty() {\n  b\n  c\n"
        );
        assert_eq!(counts(old, new), (1, 1));
    }

    #[test]
    fn an_append_is_one_added_line() {
        let (text, _) = render("a\nb\n", "a\nb\nc\n", 50);
        assert_eq!(text, "  a\n  b\n+ c\n");
        assert_eq!(counts("a\nb\n", "a\nb\nc\n"), (1, 0));
    }

    #[test]
    fn a_new_file_is_all_additions_and_an_unchanged_file_is_empty() {
        let (text, _) = render("", "one\ntwo\n", 50);
        assert_eq!(text, "+ one\n+ two\n");
        assert_eq!(render("same\n", "same\n", 50).0, "  same\n");
    }

    #[test]
    fn identical_middle_is_kept_once() {
        // Repeated lines are where a naive diff shows a changed line as removed and added.
        let old = "h\nx\ny\nx\nt\n";
        let new = "h\nx\nx\nt\n";
        let (added, removed) = counts(old, new);
        assert_eq!((added, removed), (0, 1), "removing one `y` is one removal");
    }

    #[test]
    fn a_large_rewrite_still_answers_and_says_it_was_cut() {
        let old: String = (0..400).map(|i| format!("old {i}\n")).collect();
        let new: String = (0..400).map(|i| format!("new {i}\n")).collect();
        let (text, truncated) = render(&old, &new, 20);
        assert!(truncated);
        assert!(text.contains("more line(s) not shown"));
        assert_eq!(text.lines().count(), 21);
    }
}
