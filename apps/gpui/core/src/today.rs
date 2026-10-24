//! The phone's landing page: what the day holds, the tasks that are next, and the projects they
//! belong to.
//!
//! Nothing here knows about GPUI, SQLite or the network: the page hands in what the Mac's copy
//! answered and what the phone's replica holds, and gets back the rows to draw. The reduction —
//! which entry is now, which task is next, which project a task belongs to, how much of a project
//! is done — is then testable without a window and without a database.
//!
//! Two sources meet here and stay distinguishable. The day comes from the Mac's read-only copy
//! ([`crate::mirror`]) because a phone reads its calendar there; the tasks and projects come from
//! the replica ([`crate::data::Board`]) because the phone writes those. A page that mixed them
//! without saying so would be claiming a freshness neither source has.

use chrono::{Datelike, NaiveDate};

use crate::calendar::{self, time_label};
use crate::shell::{CalendarEntry, ProjectRecord};

/// How many tasks the focus list holds before the rest becomes a count. A phone that scrolls a
/// hundred open tasks has answered "what is next" with "everything".
pub const FOCUS_LIMIT: usize = 5;

/// Where an entry sits against the clock. `Next` is the first timed entry still to come, marked
/// whether or not something is running: "now" and "next" answer different questions, and a day with
/// both answers is the one a phone is picked up to read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Spot {
    /// It ended.
    Past,
    /// It is happening.
    Now,
    /// The next timed thing today, whether or not something is running.
    Next,
    /// Later, or an all-day entry, which is never "next" because it is already under way.
    Later,
}

/// One entry of the day, as the page draws it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: String,
    pub title: String,
    /// `09:00–10:30`, `all day`, or `from 22:00` ([`calendar::time_label`]).
    pub when: String,
    /// The location, else the kind's label, so a row never carries an empty second line.
    pub where_: String,
    /// The capability's kind (`event`, `travel`, `focus`, …), for the rail's colour.
    pub kind: String,
    /// An entry that covers the whole day. It is never "next": it is already under way.
    pub all_day: bool,
    pub spot: Spot,
}

/// One task to do, with the project it came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Task {
    /// The record's path, which every task write takes.
    pub path: String,
    pub title: String,
    /// The project's name: the first segment of the path (`<project>/Task-<seconds>.md`).
    pub project: String,
    /// 1 to 3, where 1 is the one the operator put first.
    pub priority: u8,
    pub due: Option<String>,
    /// The due date has passed, so the badge says so whether or not it carries a time.
    pub overdue: bool,
}

/// A project and how much of it is done.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Project {
    /// The path prefix tasks are filed under.
    pub name: String,
    /// What to draw: the project record's title when the operator gave it one, else `name`.
    pub title: String,
    pub open: usize,
    pub done: usize,
}

impl Project {
    /// Done over all, as a fraction the progress bar can draw. A project with no tasks is empty
    /// rather than complete: nothing has been finished, nothing has started.
    pub fn progress(&self) -> f32 {
        let total = self.open + self.done;
        if total == 0 {
            0.0
        } else {
            self.done as f32 / total as f32
        }
    }

    pub fn line(&self) -> String {
        match self.open + self.done {
            0 => "no tasks yet".to_owned(),
            1 => "1 task".to_owned(),
            total => format!("{} of {total} done", self.done),
        }
    }
}

/// Everything the page draws, reduced from the two sources.
#[derive(Clone, Debug, PartialEq)]
pub struct Today {
    pub day: NaiveDate,
    pub entries: Vec<Entry>,
    /// The next few tasks, ordered by the operator's own priority and then by due date.
    pub focus: Vec<Task>,
    /// How many open tasks did not fit in [`FOCUS_LIMIT`].
    pub rest: usize,
    pub projects: Vec<Project>,
    pub open_tasks: usize,
}

impl Today {
    /// The weekday and date, for the page's title line: `Friday, 24 October`.
    pub fn headline(&self) -> String {
        self.day.format("%A, %-d %B").to_string()
    }

    /// One sentence about the day, for the line under the title. The day itself is the headline's
    /// business, so nothing here says "today": a page opened on another day would be lying.
    pub fn summary(&self) -> String {
        let entries = match self.entries.len() {
            0 => "nothing on the calendar".to_owned(),
            1 => "one entry".to_owned(),
            n => format!("{n} entries"),
        };
        let tasks = match self.open_tasks {
            0 => "nothing left to do".to_owned(),
            1 => "1 task open".to_owned(),
            n => format!("{n} tasks open"),
        };
        format!("{entries} · {tasks}")
    }
}

/// Reduces the day's entries and the phone's records into the page.
///
/// `now` is the local `HH:MM`, or `None` for a day that is not today (a screenshot of another day,
/// a test): without a clock no entry is past, now or next, and every timed one is simply later.
pub fn build(
    day: NaiveDate,
    entries: &[CalendarEntry],
    records: &[ProjectRecord],
    now: Option<&str>,
) -> Today {
    let day_of = day.format("%Y-%m-%d").to_string();
    let mut rows: Vec<Entry> = calendar::entries_on(entries, day)
        .into_iter()
        .map(|entry| Entry {
            id: entry.id.clone(),
            title: entry.title.clone(),
            when: time_label(entry, day),
            where_: match entry.location.as_deref() {
                Some(location) if !location.is_empty() => location.to_owned(),
                _ => calendar::kind_label(&entry.kind).to_owned(),
            },
            kind: entry.kind.clone(),
            all_day: entry.all_day,
            spot: spot(entry, day, now),
        })
        .collect();

    // One entry is "next": the first timed entry still to come, whether or not one is running. An
    // all-day entry is skipped because it is already under way, and two "next" rows would be a lie
    // about the day.
    if let Some(next) = rows
        .iter_mut()
        .find(|row| row.spot == Spot::Later && !row.all_day)
    {
        next.spot = Spot::Next;
    }

    let mut tasks: Vec<Task> = records
        .iter()
        .filter(|record| record.kind == "task" && !record.done)
        .map(|record| Task {
            path: record.path.clone(),
            title: record.title.clone(),
            project: project_of(record),
            priority: record.priority.clamp(1, 3),
            due: record.due.clone().filter(|due| !due.is_empty()),
            overdue: record
                .due
                .as_deref()
                .and_then(calendar::parse_day)
                .is_some_and(|due| due < day),
        })
        .collect();
    tasks.sort_by(|a, b| {
        a.priority
            .cmp(&b.priority)
            .then_with(|| match (&a.due, &b.due) {
                (Some(left), Some(right)) => left.cmp(right),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            })
            .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
    });

    let open_tasks = tasks.len();
    let rest = open_tasks.saturating_sub(FOCUS_LIMIT);
    tasks.truncate(FOCUS_LIMIT);

    let mut projects = projects_of(records, &day_of);
    projects.sort_by(|a, b| {
        b.open
            .cmp(&a.open)
            .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
    });

    Today {
        day,
        entries: rows,
        focus: tasks,
        rest,
        projects,
        open_tasks,
    }
}

/// The project a record is filed under.
///
/// The record's own `project` field wins, because a path's first segment is not the project: the
/// vault files `Archive/Soma/Tasks/Reopen.md` under `Soma` (`capabilities/projects`). A record
/// written by this app's `Board::task_path` is `<project>/Task-<seconds>.md`, so for those the
/// first segment is the same answer, and it is the fallback when no project travelled.
pub fn project_of(record: &ProjectRecord) -> String {
    if !record.project.trim().is_empty() {
        return record.project.clone();
    }
    record.path.split('/').next().unwrap_or_default().to_owned()
}

/// Every project the phone knows of, with its task counts.
///
/// A project appears when it has a record of its own or at least one task filed under it: a
/// project record with nothing in it is a project the operator made and has not filled, which is
/// worth one row. `today` is only used to keep the signature honest about the day it describes.
fn projects_of(records: &[ProjectRecord], _today: &str) -> Vec<Project> {
    let mut projects: Vec<Project> = Vec::new();
    for record in records {
        let name = project_of(record);
        if name.is_empty() {
            continue;
        }
        let index = match projects.iter().position(|project| project.name == name) {
            Some(index) => index,
            None => {
                projects.push(Project {
                    name: name.clone(),
                    title: name,
                    open: 0,
                    done: 0,
                });
                projects.len() - 1
            }
        };
        match record.kind.as_str() {
            "project" => {
                if !record.title.is_empty() {
                    projects[index].title = record.title.clone();
                }
            }
            "task" => {
                if record.done {
                    projects[index].done += 1;
                } else {
                    projects[index].open += 1;
                }
            }
            _ => {}
        }
    }
    projects
}

/// Where an entry sits against `now` on `day`.
fn spot(entry: &CalendarEntry, day: NaiveDate, now: Option<&str>) -> Spot {
    // An all-day entry covers the whole day, so it is never "next": it is already happening.
    if entry.all_day {
        return Spot::Later;
    }
    let Some(now) = now.filter(|now| now.len() >= 5) else {
        return Spot::Later;
    };
    let Some(start) = minute(&entry.starts_at) else {
        return Spot::Later;
    };
    // An entry that ends on a later day is running until midnight of this one.
    let end: String = match calendar::parse_day(&entry.ends_at) {
        Some(end) if end > day => "24:00".to_owned(),
        _ => minute(&entry.ends_at).unwrap_or_else(|| start.clone()),
    };
    let now = &now[..5];
    if now < start.as_str() {
        Spot::Later
    } else if now < end.as_str() {
        Spot::Now
    } else {
        Spot::Past
    }
}

/// The `HH:MM` of a local timestamp (`YYYY-MM-DDTHH:MM:SS`).
fn minute(stamp: &str) -> Option<String> {
    let slice = stamp.get(11..16)?;
    slice.get(..2)?.parse::<u8>().ok()?;
    Some(slice.to_owned())
}

/// The day `offset` days from `day`, for a page that steps a day at a time.
pub fn step(day: NaiveDate, offset: i64) -> NaiveDate {
    day + chrono::Duration::days(offset)
}

/// The weekday's short name, for a chip: `Fr`.
pub fn weekday(day: NaiveDate) -> &'static str {
    match day.weekday() {
        chrono::Weekday::Mon => "Mo",
        chrono::Weekday::Tue => "Tu",
        chrono::Weekday::Wed => "We",
        chrono::Weekday::Thu => "Th",
        chrono::Weekday::Fri => "Fr",
        chrono::Weekday::Sat => "Sa",
        chrono::Weekday::Sun => "Su",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, title: &str, starts: &str, ends: &str, all_day: bool) -> CalendarEntry {
        CalendarEntry {
            title: title.to_owned(),
            starts_at: starts.to_owned(),
            ends_at: ends.to_owned(),
            all_day,
            commitment: "planned".to_owned(),
            id: id.to_owned(),
            kind: "event".to_owned(),
            location: None,
            notes: None,
            source: "manual".to_owned(),
        }
    }

    fn record(path: &str, kind: &str, title: &str, done: bool, priority: u8) -> ProjectRecord {
        ProjectRecord {
            project: String::new(),
            path: path.to_owned(),
            kind: kind.to_owned(),
            title: title.to_owned(),
            content: String::new(),
            done,
            due: None,
            priority,
        }
    }

    fn day() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 24).unwrap()
    }

    #[test]
    fn the_clock_marks_one_now_and_one_next() {
        let entries = vec![
            entry(
                "a",
                "Standup",
                "2026-10-24T09:00:00",
                "2026-10-24T09:15:00",
                false,
            ),
            entry(
                "b",
                "Design",
                "2026-10-24T09:30:00",
                "2026-10-24T11:00:00",
                false,
            ),
            entry(
                "c",
                "Lunch",
                "2026-10-24T12:00:00",
                "2026-10-24T13:00:00",
                false,
            ),
            entry(
                "d",
                "Late",
                "2026-10-24T18:00:00",
                "2026-10-24T19:00:00",
                false,
            ),
        ];
        let view = build(day(), &entries, &[], Some("10:15"));
        let spots: Vec<Spot> = view.entries.iter().map(|row| row.spot).collect();
        assert_eq!(
            spots,
            vec![Spot::Past, Spot::Now, Spot::Next, Spot::Later],
            "the running entry is now, the first after it is next, the last stays later"
        );
    }

    #[test]
    fn an_all_day_entry_is_never_next() {
        let entries = vec![
            entry("a", "Away", "2026-10-24", "2026-10-25", true),
            entry(
                "b",
                "Call",
                "2026-10-24T15:00:00",
                "2026-10-24T15:30:00",
                false,
            ),
        ];
        let view = build(day(), &entries, &[], Some("08:00"));
        assert_eq!(view.entries[0].spot, Spot::Later, "all day is under way");
        assert_eq!(
            view.entries[1].spot,
            Spot::Next,
            "the timed entry takes the mark instead"
        );
    }

    #[test]
    fn an_entry_ending_after_midnight_runs_until_the_day_ends() {
        let entries = vec![entry(
            "a",
            "Night train",
            "2026-10-24T22:00:00",
            "2026-10-25T07:00:00",
            false,
        )];
        let view = build(day(), &entries, &[], Some("23:30"));
        assert_eq!(view.entries[0].spot, Spot::Now);
    }

    #[test]
    fn without_a_clock_no_entry_is_past_or_now() {
        let entries = vec![entry(
            "a",
            "Standup",
            "2026-10-24T09:00:00",
            "2026-10-24T09:15:00",
            false,
        )];
        let view = build(day(), &entries, &[], None);
        // The first later entry still takes the mark, so a day read without a clock has a pointer.
        assert_eq!(view.entries[0].spot, Spot::Next);
    }

    #[test]
    fn the_focus_list_is_the_operators_order_and_the_rest_is_a_count() {
        let records: Vec<ProjectRecord> = vec![
            record("Sjel/Task-1.md", "task", "Third", false, 3),
            record("Sjel/Task-2.md", "task", "First", false, 1),
            record("Sjel/Task-3.md", "task", "Second", false, 2),
            record("Sjel/Task-4.md", "task", "Fourth", false, 1),
            record("Sjel/Task-5.md", "task", "Fifth", false, 2),
            record("Sjel/Task-6.md", "task", "Sixth", false, 3),
            record("Sjel/Task-7.md", "task", "Already done", true, 1),
        ];
        let view = build(day(), &[], &records, None);
        assert_eq!(view.open_tasks, 6, "the done task is not open");
        assert_eq!(view.focus.len(), FOCUS_LIMIT);
        assert_eq!(view.rest, 1);
        assert_eq!(view.focus[0].title, "First", "P1 before P2");
        assert_eq!(view.focus[1].title, "Fourth", "P1s by title");
        assert_eq!(view.focus[2].title, "Fifth", "then P2");
    }

    #[test]
    fn a_task_due_before_the_day_is_overdue() {
        let mut record = record("Sjel/Task-1.md", "task", "Late", false, 2);
        record.due = Some("2026-10-21T09:00:00".to_owned());
        let view = build(day(), &[], &[record], None);
        assert!(view.focus[0].overdue);
    }

    #[test]
    fn a_project_takes_the_title_the_operator_gave_it() {
        let records = vec![
            record("Sjel", "project", "Sjel itself", false, 2),
            record("Sjel/Task-1.md", "task", "One", false, 2),
            record("Sjel/Task-2.md", "task", "Two", true, 2),
            record("Nest", "project", "The nest", false, 2),
        ];
        let view = build(day(), &[], &records, None);
        let names: Vec<&str> = view.projects.iter().map(|p| p.title.as_str()).collect();
        assert_eq!(names, vec!["Sjel itself", "The nest"], "open tasks first");
        assert_eq!(view.projects[0].open, 1);
        assert_eq!(view.projects[0].done, 1);
        assert_eq!(view.projects[1].line(), "no tasks yet");
        assert!((view.projects[0].progress() - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn a_project_with_no_tasks_is_empty_rather_than_complete() {
        let records = vec![record("Nest", "project", "The nest", false, 2)];
        let view = build(day(), &[], &records, None);
        assert_eq!(view.projects[0].progress(), 0.0);
    }

    #[test]
    fn the_summary_counts_both_sources() {
        let entries = vec![entry(
            "a",
            "Call",
            "2026-10-24T15:00:00",
            "2026-10-24T15:30:00",
            false,
        )];
        let records = vec![record("Sjel/Task-1.md", "task", "One", false, 2)];
        let view = build(day(), &entries, &records, Some("08:00"));
        assert_eq!(view.summary(), "one entry · 1 task open");
        assert_eq!(view.headline(), "Saturday, 24 October");
    }

    #[test]
    fn nothing_anywhere_is_an_empty_page_not_a_panic() {
        let view = build(day(), &[], &[], Some("08:00"));
        assert!(view.entries.is_empty());
        assert!(view.focus.is_empty());
        assert!(view.projects.is_empty());
        assert_eq!(
            view.summary(),
            "nothing on the calendar · nothing left to do"
        );
    }

    #[test]
    fn the_project_is_the_records_own_and_the_path_is_only_the_fallback() {
        let mut filed = record("Archive/Soma/Tasks/Reopen.md", "task", "Reopen", false, 2);
        filed.project = "Soma".to_owned();
        assert_eq!(project_of(&filed), "Soma", "the vault's own answer wins");
        let unfiled = record("Sjel/Task-1.md", "task", "One", false, 2);
        assert_eq!(project_of(&unfiled), "Sjel", "no project travelled");
        let rootless = record("", "task", "One", false, 2);
        assert_eq!(project_of(&rootless), "");
    }

    #[test]
    fn stepping_moves_one_day_across_a_month_end() {
        let october = NaiveDate::from_ymd_opt(2026, 10, 31).unwrap();
        assert_eq!(
            step(october, 1),
            NaiveDate::from_ymd_opt(2026, 11, 1).unwrap()
        );
        assert_eq!(weekday(october), "Sa");
    }
}
