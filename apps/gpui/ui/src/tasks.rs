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
}
