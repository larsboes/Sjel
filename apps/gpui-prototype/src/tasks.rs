//! A project's tasks as a table: done, title, due, priority, each changed in its cell.
//!
//! Done goes through `POST /projects/api/tasks/done`, which also keeps `status:` and `completed:`
//! in step. Due and priority are frontmatter lines; `set_field` changes only that line, the way
//! `record::set_done` in `capabilities/projects/src/record.rs` does, and the note saves through
//! `POST /projects/api/record` with the content it started from.

use crate::shell::ProjectRecord;

/// `content` with the top-level frontmatter line `key:` set to `value`, added before the closing
/// `---` when missing. `None` when the note has no frontmatter.
pub fn set_field(content: &str, key: &str, value: &str) -> Option<String> {
    let body = content.strip_prefix("---\n")?;
    let close = body
        .match_indices("\n---")
        .map(|(at, _)| at)
        .find(|&at| matches!(body[at + 4..].chars().next(), None | Some('\n')))?;
    let line = |value: &str| {
        if value.is_empty() {
            format!("{key}:")
        } else {
            format!("{key}: {value}")
        }
    };
    let mut found = false;
    let mut lines: Vec<String> = body[..close]
        .split('\n')
        .map(|existing| {
            if existing.split(':').next() == Some(key) {
                found = true;
                line(value)
            } else {
                existing.to_owned()
            }
        })
        .collect();
    if !found {
        lines.push(line(value));
    }
    Some(format!("---\n{}{}", lines.join("\n"), &body[close..]))
}

/// A due date as the table accepts it: empty, or `YYYY-MM-DD`.
pub fn valid_due(due: &str) -> bool {
    due.is_empty() || chrono::NaiveDate::parse_from_str(due, "%Y-%m-%d").is_ok()
}

/// Open tasks first, then by due date with undated last, then by priority.
pub fn ordered(records: &[ProjectRecord]) -> Vec<&ProjectRecord> {
    let mut tasks: Vec<&ProjectRecord> = records.iter().filter(|r| r.kind == "task").collect();
    tasks.sort_by(|a, b| {
        let due = |r: &ProjectRecord| r.due.clone().filter(|d| !d.is_empty());
        (a.done, due(a).is_none(), due(a), a.priority, &a.title).cmp(&(
            b.done,
            due(b).is_none(),
            due(b),
            b.priority,
            &b.title,
        ))
    });
    tasks
}

#[cfg(test)]
mod tests {
    use super::*;

    const TASK: &str = "---\ntype: task\ndue: 2026-10-24\npriority: 1\nprojects:\n  - \"[[P]]\"\n---\n\n## Details\n";

    #[test]
    fn set_field_changes_one_line_and_keeps_the_rest() {
        let moved = set_field(TASK, "due", "2026-11-01").unwrap();
        assert_eq!(moved, TASK.replace("due: 2026-10-24", "due: 2026-11-01"));
        let cleared = set_field(TASK, "due", "").unwrap();
        assert!(cleared.contains("\ndue:\n"));
        // A nested list item named like the key is not the key.
        let added = set_field(TASK, "scheduled", "2026-10-20").unwrap();
        assert!(added.contains("  - \"[[P]]\"\nscheduled: 2026-10-20\n---\n\n## Details\n"));
        assert_eq!(set_field("no frontmatter", "due", "x"), None);
    }

    #[test]
    fn due_dates_are_a_day_or_nothing() {
        assert!(valid_due(""));
        assert!(valid_due("2026-10-24"));
        assert!(!valid_due("24.10.2026"));
        assert!(!valid_due("2026-13-01"));
    }

    #[test]
    fn open_dated_urgent_tasks_come_first() {
        let task = |title: &str, done: bool, due: Option<&str>, priority: u8| ProjectRecord {
            path: format!("P/{title}.md"),
            kind: "task".into(),
            title: title.into(),
            content: String::new(),
            done,
            due: due.map(str::to_owned),
            priority,
        };
        let records = vec![
            task("done", true, Some("2026-01-01"), 1),
            task("undated", false, None, 1),
            task("later", false, Some("2026-12-01"), 1),
            task("soon-low", false, Some("2026-10-20"), 3),
            task("soon-high", false, Some("2026-10-20"), 1),
        ];
        let titles: Vec<&str> = ordered(&records).iter().map(|r| r.title.as_str()).collect();
        assert_eq!(
            titles,
            ["soon-high", "soon-low", "later", "undated", "done"]
        );
    }
}
