use super::types::{NoteSort, TaskSort};
use sjel_gpui_core::shell::{GlobalNote, GlobalTask, ProjectRecord, TASK_STATUSES};

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

/// Sorts cross-project notes by `sort`, reversed when `ascending` is false.
pub fn sort_notes(notes: &mut [&GlobalNote], sort: NoteSort, ascending: bool) {
    let default = |a: &GlobalNote, b: &GlobalNote| {
        (
            a.kind != "project",
            a.project.to_lowercase(),
            a.title.to_lowercase(),
        )
            .cmp(&(
                b.kind != "project",
                b.project.to_lowercase(),
                b.title.to_lowercase(),
            ))
    };
    notes.sort_by(|a, b| {
        let primary = match sort {
            NoteSort::Default => std::cmp::Ordering::Equal,
            NoteSort::Title => a.title.to_lowercase().cmp(&b.title.to_lowercase()),
            NoteSort::Project => a.project.to_lowercase().cmp(&b.project.to_lowercase()),
            NoteSort::Kind => (a.kind != "project").cmp(&(b.kind != "project")),
        };
        let primary = if ascending {
            primary
        } else {
            primary.reverse()
        };
        primary.then_with(|| default(a, b))
    });
}
