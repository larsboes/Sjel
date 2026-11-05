//! A project's tasks as a table: done, title, due, priority, each changed in its cell.
//!
//! Every write goes to the capability, which owns the frontmatter edit and answers with the
//! record's new frontmatter (`POST /api/tasks/done`, `POST /api/record/property`). The table
//! draws that answer, so nothing here rewrites a note.

pub use sjel_gpui_core::shell::{status_closes, TASK_STATUSES};
use sjel_gpui_core::shell::{GlobalTask, ProjectRecord};

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

/// Which statuses the cross-project table shows. `Active` hides closed tasks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusFilter {
    Active,
    All,
    Is(&'static str),
}

impl StatusFilter {
    pub fn matches(self, status: &str) -> bool {
        match self {
            Self::Active => !status_closes(status),
            Self::All => true,
            Self::Is(wanted) => wanted == status,
        }
    }
}

/// The label a status gets on screen. The stored value stays the frontmatter word.
pub fn status_label(status: &str) -> &'static str {
    match status {
        "doing" => "In progress",
        "waiting" => "Waiting",
        "blocked" => "Blocked",
        "done" => "Done",
        "cancelled" => "Cancelled",
        _ => "Open",
    }
}

/// A column the cross-project table sorts by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskSort {
    /// Open first, then by due date with undated last, then by priority: the capability's order.
    Default,
    Title,
    Project,
    Status,
    Due,
    Priority,
}

/// Sorts tasks by `sort`, reversed when `ascending` is false. Ties keep the default order.
pub fn sort_tasks(tasks: &mut [&GlobalTask], sort: TaskSort, ascending: bool) {
    let rank = |t: &GlobalTask| {
        TASK_STATUSES
            .iter()
            .position(|s| *s == t.status())
            .unwrap_or(0)
    };
    let due = |t: &GlobalTask| t.due.clone().filter(|d| !d.is_empty());
    let default = |a: &GlobalTask, b: &GlobalTask| {
        (a.done, due(a).is_none(), due(a), a.priority, &a.title).cmp(&(
            b.done,
            due(b).is_none(),
            due(b),
            b.priority,
            &b.title,
        ))
    };
    tasks.sort_by(|a, b| {
        let primary = match sort {
            TaskSort::Default => std::cmp::Ordering::Equal,
            TaskSort::Title => a.title.to_lowercase().cmp(&b.title.to_lowercase()),
            TaskSort::Project => a.project.cmp(&b.project),
            TaskSort::Status => rank(a).cmp(&rank(b)),
            // Undated tasks stay last in both directions.
            TaskSort::Due => match (due(a), due(b)) {
                (Some(x), Some(y)) => x.cmp(&y),
                (Some(_), None) => return std::cmp::Ordering::Less,
                (None, Some(_)) => return std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            },
            TaskSort::Priority => a.priority.cmp(&b.priority),
        };
        let primary = if ascending {
            primary
        } else {
            primary.reverse()
        };
        primary.then_with(|| default(a, b))
    });
}

/// Consecutive runs of tasks per project, in first-seen order, for the grouped table.
pub fn group_by_project<'a>(tasks: &[&'a GlobalTask]) -> Vec<(String, Vec<&'a GlobalTask>)> {
    let mut groups: Vec<(String, Vec<&'a GlobalTask>)> = Vec::new();
    for task in tasks {
        match groups.iter_mut().find(|(name, _)| *name == task.project) {
            Some((_, list)) => list.push(task),
            None => groups.push((task.project.clone(), vec![task])),
        }
    }
    groups
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

    fn global(
        title: &str,
        project: &str,
        status: &str,
        due: Option<&str>,
        priority: u8,
    ) -> GlobalTask {
        GlobalTask {
            id: format!("{project}/Tasks/{title}.md"),
            kind: "task".into(),
            title: title.into(),
            done: status_closes(status),
            due: due.map(str::to_owned),
            priority,
            summary: None,
            project: project.into(),
            fields: [("status".to_owned(), status.to_owned())].into(),
        }
    }

    #[test]
    fn active_hides_closed_statuses() {
        let statuses: Vec<&str> = TASK_STATUSES
            .iter()
            .copied()
            .filter(|s| StatusFilter::Active.matches(s))
            .collect();
        assert_eq!(statuses, ["open", "doing", "waiting", "blocked"]);
        assert!(StatusFilter::Is("cancelled").matches("cancelled"));
        assert!(!StatusFilter::Is("cancelled").matches("done"));
        assert!(StatusFilter::All.matches("cancelled"));
    }

    #[test]
    fn status_falls_back_to_done_when_frontmatter_disagrees() {
        let mut task = global("x", "P", "waiting", None, 2);
        task.done = true;
        assert_eq!(task.status(), "done");
        task.fields.clear();
        task.done = false;
        assert_eq!(task.status(), "open");
    }

    #[test]
    fn sorts_by_column_and_keeps_undated_last() {
        let a = global("a", "Zeta", "blocked", Some("2026-12-01"), 3);
        let b = global("b", "Alpha", "open", None, 1);
        let c = global("c", "Mid", "doing", Some("2026-11-10"), 2);
        let titles = |sort, asc| {
            let mut list = vec![&a, &b, &c];
            sort_tasks(&mut list, sort, asc);
            list.iter().map(|t| t.title.as_str()).collect::<Vec<_>>()
        };
        assert_eq!(titles(TaskSort::Project, true), ["b", "c", "a"]);
        assert_eq!(titles(TaskSort::Priority, false), ["a", "c", "b"]);
        assert_eq!(titles(TaskSort::Status, true), ["b", "c", "a"]);
        assert_eq!(titles(TaskSort::Due, true), ["c", "a", "b"]);
        assert_eq!(titles(TaskSort::Due, false), ["a", "c", "b"]);
    }

    #[test]
    fn groups_keep_first_seen_project_order() {
        let a = global("a", "Home", "open", None, 1);
        let b = global("b", "Work", "open", None, 1);
        let c = global("c", "Home", "open", None, 1);
        let groups = group_by_project(&[&a, &b, &c]);
        let shape: Vec<(&str, usize)> = groups.iter().map(|(n, l)| (n.as_str(), l.len())).collect();
        assert_eq!(shape, [("Home", 2), ("Work", 1)]);
    }
}
