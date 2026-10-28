//! A project's tasks as a table: done, title, due, priority, each changed in its cell.
//!
//! Every write goes to the capability, which owns the frontmatter edit and answers with the
//! record's new frontmatter (`POST /api/tasks/done`, `POST /api/record/property`). The table
//! draws that answer, so nothing here rewrites a note.

use sjel_gpui_core::shell::ProjectRecord;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskFilter {
    All,
    Open,
    Done,
}

impl TaskFilter {
    pub fn matches(self, done: bool) -> bool {
        match self {
            Self::All => true,
            Self::Open => !done,
            Self::Done => done,
        }
    }
}

#[allow(unused_imports)]
pub use crate::{due_status, DueStatus};

/// A due date as the table accepts it: empty, or `YYYY-MM-DD`.
pub fn valid_due(due: &str) -> bool {
    due.is_empty() || chrono::NaiveDate::parse_from_str(due, "%Y-%m-%d").is_ok()
}

/// Extracts subtask completion from markdown checklist items (`- [ ]` and `- [x]`).
pub fn parse_subtask_progress(content: &str) -> Option<(usize, usize)> {
    let mut done = 0;
    let mut total = 0;
    for line in content.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("- [x]")
            || trimmed.starts_with("- [X]")
            || trimmed.starts_with("* [x]")
            || trimmed.starts_with("* [X]")
        {
            done += 1;
            total += 1;
        } else if trimmed.starts_with("- [ ]") || trimmed.starts_with("* [ ]") {
            total += 1;
        }
    }
    if total > 0 {
        Some((done, total))
    } else {
        None
    }
}

/// Parses inline syntax from a task entry string:
/// `!1`, `!2`, `!3` for priority,
/// `@today`, `@tomorrow`, `@YYYY-MM-DD` for due date.
/// Returns (clean_title, due_date, priority).
pub fn parse_smart_task_input(
    raw: &str,
    today: chrono::NaiveDate,
) -> (String, Option<String>, Option<u8>) {
    let mut due = None;
    let mut priority = None;
    let mut words = Vec::new();

    for word in raw.split_whitespace() {
        if word == "!1" {
            priority = Some(1);
        } else if word == "!2" {
            priority = Some(2);
        } else if word == "!3" {
            priority = Some(3);
        } else if word.eq_ignore_ascii_case("@today") {
            due = Some(today.format("%Y-%m-%d").to_string());
        } else if word.eq_ignore_ascii_case("@tomorrow") {
            due = Some(
                (today + chrono::Duration::days(1))
                    .format("%Y-%m-%d")
                    .to_string(),
            );
        } else if let Some(date_part) = word.strip_prefix('@') {
            if chrono::NaiveDate::parse_from_str(date_part, "%Y-%m-%d").is_ok() {
                due = Some(date_part.to_string());
            } else {
                words.push(word);
            }
        } else {
            words.push(word);
        }
    }

    (words.join(" "), due, priority)
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
            project: String::new(),
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

    #[test]
    fn due_status_computes_relative_labels() {
        assert_eq!(
            due_status(Some("2026-10-19"), false, "2026-10-20"),
            DueStatus::Overdue("2026-10-19".into())
        );
        assert_eq!(
            due_status(Some("2026-10-20"), false, "2026-10-20"),
            DueStatus::Today
        );
        assert_eq!(
            due_status(Some("2026-10-21"), false, "2026-10-20"),
            DueStatus::Tomorrow
        );
        assert_eq!(
            due_status(Some("2026-10-25"), false, "2026-10-20"),
            DueStatus::Upcoming("2026-10-25".into())
        );
        assert_eq!(
            due_status(Some("2026-10-19"), true, "2026-10-20"),
            DueStatus::Upcoming("2026-10-19".into())
        );
        assert_eq!(due_status(None, false, "2026-10-20"), DueStatus::None);
    }

    #[test]
    fn parses_subtask_progress_from_markdown() {
        let content = "# Header\n- [ ] First\n- [x] Second\n* [x] Third\nParagraph\n- [ ] Fourth";
        assert_eq!(parse_subtask_progress(content), Some((2, 4)));
        assert_eq!(parse_subtask_progress("No subtasks here"), None);
    }

    #[test]
    fn parses_smart_task_input_syntax() {
        let today = chrono::NaiveDate::from_ymd_opt(2026, 10, 28).unwrap();
        let (title, due, p) = parse_smart_task_input("Ship GPUI UI !1 @today", today);
        assert_eq!(title, "Ship GPUI UI");
        assert_eq!(due, Some("2026-10-28".into()));
        assert_eq!(p, Some(1));

        let (title2, due2, p2) = parse_smart_task_input("Review @tomorrow !2 draft", today);
        assert_eq!(title2, "Review draft");
        assert_eq!(due2, Some("2026-10-29".into()));
        assert_eq!(p2, Some(2));
    }
}
