//! A project's tasks as a table: done, title, due, priority, each changed in its cell.
//!
//! Every write goes to the capability, which owns the frontmatter edit and answers with the
//! record's new frontmatter (`POST /api/tasks/done`, `POST /api/record/property`). The table
//! draws that answer, so nothing here rewrites a note.

pub mod parser;
pub mod sorting;
pub mod types;

pub use parser::{parse_smart_task_input, parse_subtask_progress, valid_due};
pub use sorting::{group_by_project, ordered, sort_tasks};
pub use types::{status_closes, status_label, StatusFilter, TaskFilter, TaskSort, TASK_STATUSES};

#[allow(unused_imports)]
pub use crate::{due_status, DueStatus};

#[cfg(test)]
mod tests {
    use super::*;
    use sjel_gpui_core::shell::{GlobalTask, ProjectRecord};

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
