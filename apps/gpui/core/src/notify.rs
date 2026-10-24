//! What the phone puts on its lock screen, and when.
//!
//! The decision is here and needs no device: which entries and tasks earn a reminder, at what
//! minute, and which of the requests already pending must go. The device itself — an
//! `UNUserNotificationCenter` — belongs to the host that owns it (`ios/host/Notifications.swift`),
//! and the drawer's counterpart on the Mac does not exist: this is the phone's, because a phone is
//! the device that is carried when an entry starts.
//!
//! Three rules, each of them a choice rather than a derivation:
//!
//! - A **timed entry** fires its lead time before it starts ([`Lead`]), which is the one number the
//!   person sets.
//! - An **all-day entry** has no start time to be early for, so it is announced at
//!   [`ALL_DAY_HOUR`] on the day it covers. The lead does not apply: subtracting ten minutes from
//!   "all day" means nothing.
//! - A **task** is announced at [`TASK_HOUR`] on its due date, for the same reason: its due date is
//!   a day. A task already done is not announced, and neither is one that is not a task.
//!
//! What is left out, deliberately: an entry whose commitment is `possible` blocks nothing
//! (`crate::calendar::COMMITMENTS`), so a phone that announced it would teach the person to swipe
//! reminders away; and anything whose moment has passed, because a request for the past is one the
//! system either drops or delivers at once.
//!
//! The identifiers are stable across rebuilds — `cal:<entry id>`, `task:<record path>` — so a
//! re-plan replaces a request in place instead of leaving the old one to fire beside it.

use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone};

use crate::shell::{CalendarEntry, GlobalTask};

/// The hour an all-day entry is announced at, in local time.
pub const ALL_DAY_HOUR: u32 = 8;

/// The hour a task is announced at on its due date, in local time.
pub const TASK_HOUR: u32 = 9;

/// How many requests may be pending at once. iOS keeps 64 per app and silently drops the rest, so
/// this stops short of that and says how many it left out ([`Plan::dropped`]) rather than pretending
/// the list was the whole of it.
pub const MAX_PENDING: usize = 60;

/// How long before an entry's start its reminder fires.
///
/// The key is what `notifications.json` stores, so renaming a variant does not lose a person's
/// choice: it falls back to [`Lead::default`] instead of a wrong number of minutes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Lead {
    /// On the minute the entry starts.
    AtStart,
    /// Ten minutes before it, the default: enough to stop what you are doing, not enough to forget.
    #[default]
    Ten,
    Hour,
}

impl Lead {
    /// In the order the settings row offers them, the default first.
    pub const ALL: [Lead; 3] = [Lead::Ten, Lead::AtStart, Lead::Hour];

    pub fn minutes(self) -> i64 {
        match self {
            Self::AtStart => 0,
            Self::Ten => 10,
            Self::Hour => 60,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::AtStart => "at_start",
            Self::Ten => "ten",
            Self::Hour => "hour",
        }
    }

    /// The key a settings file stores, or `None` for one this build does not know.
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|lead| lead.key() == key)
    }

    /// What the chip says: a duration, not a sentence, so three of them fit one row.
    pub fn label(self) -> &'static str {
        match self {
            Self::AtStart => "At start",
            Self::Ten => "10 min",
            Self::Hour => "1 hour",
        }
    }
}

/// One request the host should hold: what to show, when, and where a tap goes.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Reminder {
    /// Stable across rebuilds, so a re-plan replaces rather than duplicates.
    pub id: String,
    pub title: String,
    pub body: String,
    /// Seconds since the Unix epoch, in local time. What the host turns into a trigger.
    pub at: i64,
    /// `calendar:<YYYY-MM-DD>`: the day a tap opens.
    pub link: String,
}

/// A whole rebuild's answer: the requests, and how many the cap left out.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub reminders: Vec<Reminder>,
    pub dropped: usize,
}

impl Plan {
    /// The line under the switch: how many are pending, and the one after that.
    pub fn summary(&self, now: DateTime<Local>) -> String {
        if self.reminders.is_empty() {
            return "Nothing scheduled".to_owned();
        }
        let count = self.reminders.len();
        let noun = if count == 1 { "reminder" } else { "reminders" };
        let first = &self.reminders[0];
        format!(
            "{count} {noun} · next {}",
            when(first.at, &first.title, now)
        )
    }
}

/// `Mon 09:50 · Zahnarzt`: a moment as a person reads it, in the phone's own week.
pub fn when(at: i64, title: &str, now: DateTime<Local>) -> String {
    let Some(moment) = Local.timestamp_opt(at, 0).single() else {
        return title.to_owned();
    };
    let clock = moment.format("%H:%M");
    let day = if moment.date_naive() == now.date_naive() {
        "Today".to_owned()
    } else if moment.date_naive() == now.date_naive() + chrono::Duration::days(1) {
        "Tomorrow".to_owned()
    } else {
        moment.format("%a %-d %b").to_string()
    };
    format!("{day} {clock} · {title}")
}

/// The day a reminder's tap opens, from its [`Reminder::link`].
pub fn link_day(link: &str) -> Option<NaiveDate> {
    let day = link.strip_prefix("calendar:")?;
    NaiveDate::parse_from_str(day.get(..10)?, "%Y-%m-%d").ok()
}

/// The whole rule: what to ask the host for, out of what this phone holds.
///
/// `entries` is the calendar window the phone can answer for and `tasks` the open tasks off the
/// replica; both are the same rows their pages draw, so a reminder cannot disagree with the page
/// beside it. `now` is passed in rather than read, so the rule is testable at any moment.
pub fn plan(
    entries: &[CalendarEntry],
    tasks: &[GlobalTask],
    lead: Lead,
    now: DateTime<Local>,
) -> Plan {
    let mut reminders = Vec::new();
    for entry in entries {
        if let Some(reminder) = entry_reminder(entry, lead, now) {
            reminders.push(reminder);
        }
    }
    for task in tasks {
        if let Some(reminder) = task_reminder(task, now) {
            reminders.push(reminder);
        }
    }
    // Soonest first: the cap then drops the farthest away, which is the end of the list a person
    // has not looked at yet, and `summary` reads the first of these as "next".
    reminders.sort_by(|a, b| a.at.cmp(&b.at).then_with(|| a.id.cmp(&b.id)));
    let dropped = reminders.len().saturating_sub(MAX_PENDING);
    reminders.truncate(MAX_PENDING);
    Plan { reminders, dropped }
}

/// One entry's reminder, or `None` when it does not earn one.
fn entry_reminder(entry: &CalendarEntry, lead: Lead, now: DateTime<Local>) -> Option<Reminder> {
    // `possible` blocks nothing, so it would teach the person to swipe reminders away.
    if entry.commitment == "possible" {
        return None;
    }
    let day = entry_day(entry)?;
    let (at, body) = if entry.all_day {
        let at = moment(day, ALL_DAY_HOUR, 0)?;
        let body = format!("All day · {}", kind(entry));
        (at, body)
    } else {
        let start = entry_start(entry)?;
        // Through the local zone, not `NaiveDateTime::timestamp`, which would read the wall clock
        // as though it were UTC and put every reminder an hour off in a summer-time zone.
        let at = epoch(start)? - lead.minutes() * 60;
        let end = entry_end(entry).unwrap_or(start + chrono::Duration::hours(1));
        let mut body = format!(
            "{}-{} · {}",
            start.format("%H:%M"),
            end.format("%H:%M"),
            kind(entry)
        );
        if let Some(place) = entry.location.as_deref().filter(|l| !l.is_empty()) {
            body.push_str(" · ");
            body.push_str(place);
        }
        (at, body)
    };
    if at <= now.timestamp() || entry.title.trim().is_empty() {
        return None;
    }
    Some(Reminder {
        // The Mac's own id once the sync has named it, so a re-plan after a pull replaces the
        // reminder made before it instead of scheduling a second one.
        id: format!("cal:{}", entry.id),
        title: entry.title.trim().to_owned(),
        body,
        at,
        link: format!("calendar:{day}"),
    })
}

/// One task's reminder, or `None` when it is done, undated, past, or not a task.
fn task_reminder(task: &GlobalTask, now: DateTime<Local>) -> Option<Reminder> {
    if task.done || task.title.trim().is_empty() || (!task.kind.is_empty() && task.kind != "task") {
        return None;
    }
    let due = task.due.as_deref().filter(|due| !due.is_empty())?;
    let day = NaiveDate::parse_from_str(due.get(..10)?, "%Y-%m-%d").ok()?;
    let at = moment(day, TASK_HOUR, 0)?;
    if at <= now.timestamp() {
        return None;
    }
    let body = if task.project.is_empty() {
        "Due today".to_owned()
    } else {
        format!("Due today · {}", task.project)
    };
    Some(Reminder {
        id: format!("task:{}", task.id),
        title: task.title.trim().to_owned(),
        body,
        at,
        link: format!("calendar:{day}"),
    })
}

/// The kind in words, with the fallback the calendar page already uses.
fn kind(entry: &CalendarEntry) -> String {
    let key = if entry.kind.is_empty() {
        "busy"
    } else {
        entry.kind.as_str()
    };
    crate::calendar::kind_label(key).to_owned()
}

/// The day an entry is announced for: an all-day entry's first day, a timed entry's own date.
fn entry_day(entry: &CalendarEntry) -> Option<NaiveDate> {
    crate::calendar::parse_day(&entry.starts_at)
}

/// A timed entry's start, in local time.
fn entry_start(entry: &CalendarEntry) -> Option<NaiveDateTime> {
    parse_local(&entry.starts_at)
}

/// A timed entry's end, in local time.
fn entry_end(entry: &CalendarEntry) -> Option<NaiveDateTime> {
    parse_local(&entry.ends_at)
}

/// `YYYY-MM-DDTHH:MM:SS` as the capability stores it, or the minute-only form some rows carry.
fn parse_local(value: &str) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(value.get(..19)?, "%Y-%m-%dT%H:%M:%S")
        .ok()
        .or_else(|| NaiveDateTime::parse_from_str(value.get(..16)?, "%Y-%m-%dT%H:%M").ok())
}

/// A day and an hour as the epoch seconds the host takes. `None` on a day the clock skips — the
/// hour a daylight-saving change removes — because a request at a moment that never happens is one
/// the system would either drop or deliver at the wrong time.
fn moment(day: NaiveDate, hour: u32, minute: u32) -> Option<i64> {
    let time = NaiveTime::from_hms_opt(hour, minute, 0)?;
    epoch(day.and_time(time))
}

/// A wall-clock moment in this phone's own zone, as the epoch seconds the host takes.
fn epoch(local: NaiveDateTime) -> Option<i64> {
    Some(Local.from_local_datetime(&local).single()?.timestamp())
}

/// What the host should do now: the requests to hold, and the identifiers to take away.
///
/// Adding a request with an identifier the system already holds replaces it, so a rebuild does not
/// need to ask what changed — it hands over the whole list, and cancels only the ids that left
/// (an entry deleted on the phone, a task ticked off, an event moved past).
pub fn reconcile(pending: &[String], next: &[Reminder]) -> (Vec<Reminder>, Vec<String>) {
    let keep: Vec<&str> = next.iter().map(|reminder| reminder.id.as_str()).collect();
    let cancel = pending
        .iter()
        .filter(|id| !keep.contains(&id.as_str()))
        .cloned()
        .collect();
    (next.to_vec(), cancel)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(day: &str, hour: u32, minute: u32) -> DateTime<Local> {
        let date = NaiveDate::parse_from_str(day, "%Y-%m-%d").expect("day");
        Local
            .from_local_datetime(&date.and_hms_opt(hour, minute, 0).expect("time"))
            .single()
            .expect("local")
    }

    fn entry(title: &str, starts_at: &str, ends_at: &str, all_day: bool) -> CalendarEntry {
        CalendarEntry {
            title: title.to_owned(),
            starts_at: starts_at.to_owned(),
            ends_at: ends_at.to_owned(),
            all_day,
            commitment: "planned".to_owned(),
            id: format!("id-{title}"),
            kind: "work_onsite".to_owned(),
            location: None,
            notes: None,
            source: "manual".to_owned(),
        }
    }

    fn task(title: &str, due: &str, done: bool) -> GlobalTask {
        GlobalTask {
            id: format!("Projects/Home/{title}.md"),
            kind: "task".to_owned(),
            title: title.to_owned(),
            done,
            due: Some(due.to_owned()),
            priority: 2,
            summary: None,
            project: "Home".to_owned(),
        }
    }

    /// The three rules, each in the place it decides something: a timed entry takes the lead off its
    /// start, an all-day entry is announced at the all-day hour whatever the lead, and a task at the
    /// task hour on its due date.
    #[test]
    fn a_timed_entry_is_early_by_the_lead_and_an_all_day_one_is_at_eight() {
        let now = at("2026-10-24", 8, 0);
        let timed = entry(
            "Zahnarzt",
            "2026-10-24T10:00:00",
            "2026-10-24T10:45:00",
            false,
        );
        let all_day = entry("Umzug", "2026-10-25", "2026-10-26", true);
        let planned = plan(&[timed, all_day], &[], Lead::Ten, now);

        assert_eq!(planned.reminders.len(), 2);
        assert_eq!(planned.reminders[0].id, "cal:id-Zahnarzt");
        assert_eq!(planned.reminders[0].at, at("2026-10-24", 9, 50).timestamp());
        assert_eq!(planned.reminders[0].link, "calendar:2026-10-24");
        assert_eq!(
            planned.reminders[0].body, "10:00-10:45 · Work (on site)",
            "the body says what the entry is and when it runs"
        );
        assert_eq!(
            planned.reminders[1].at,
            at("2026-10-25", 8, 0).timestamp(),
            "all day is announced at the all-day hour, not ten minutes early"
        );

        // The same entry one hour early, and at its own start: the lead is the only thing that moved.
        let early = entry(
            "Zahnarzt",
            "2026-10-24T10:00:00",
            "2026-10-24T10:45:00",
            false,
        );
        let hour = plan(&[early.clone()], &[], Lead::Hour, now);
        assert_eq!(hour.reminders[0].at, at("2026-10-24", 9, 0).timestamp());
        let start = plan(&[early], &[], Lead::AtStart, now);
        assert_eq!(start.reminders[0].at, at("2026-10-24", 10, 0).timestamp());
    }

    /// The rows that earn nothing: a possible entry, one already past, a done task, an undated one.
    #[test]
    fn what_blocks_nothing_and_what_is_over_is_left_out() {
        let now = at("2026-10-24", 12, 0);
        let mut possible = entry("Kino?", "2026-10-24T20:00:00", "2026-10-24T22:00:00", false);
        possible.commitment = "possible".to_owned();
        let past = entry(
            "Frühstück",
            "2026-10-24T09:00:00",
            "2026-10-24T10:00:00",
            false,
        );
        let plan = plan(
            &[possible, past],
            &[
                task("Steuer", "2026-10-26", false),
                task("Wäsche", "2026-10-26", true),
                GlobalTask {
                    due: None,
                    ..task("Irgendwann", "2026-10-26", false)
                },
            ],
            Lead::Ten,
            now,
        );

        assert_eq!(
            plan.reminders.len(),
            1,
            "the open dated task is the whole of it"
        );
        assert_eq!(plan.reminders[0].id, "task:Projects/Home/Steuer.md");
        assert_eq!(plan.reminders[0].at, at("2026-10-26", 9, 0).timestamp());
        assert_eq!(plan.reminders[0].body, "Due today · Home");
    }

    /// The cap keeps the soonest and admits what it dropped: a silently shortened list reads as the
    /// whole of it.
    #[test]
    fn the_cap_keeps_the_soonest_and_says_how_many_it_left_out() {
        let now = at("2026-10-24", 7, 0);
        let entries: Vec<CalendarEntry> = (0..MAX_PENDING + 5)
            .map(|i| {
                let hour = 8 + i / 60;
                entry(
                    &format!("Eintrag {i}"),
                    &format!("2026-10-24T{hour:02}:00:00"),
                    &format!("2026-10-24T{hour:02}:30:00"),
                    false,
                )
            })
            .collect();
        let plan = plan(&entries, &[], Lead::AtStart, now);
        assert_eq!(plan.reminders.len(), MAX_PENDING);
        assert_eq!(plan.dropped, 5);
        assert_eq!(
            plan.reminders[0].title, "Eintrag 0",
            "the first of the list is the soonest, which is what `next` reads"
        );
    }

    /// A rebuild hands the host the whole list and cancels only what left it; the ids are stable, so
    /// re-planning the same data asks for exactly the same requests.
    #[test]
    fn a_rebuild_cancels_what_left_and_keeps_the_rest() {
        let now = at("2026-10-24", 7, 0);
        let first = plan(
            &[entry(
                "A",
                "2026-10-24T09:00:00",
                "2026-10-24T10:00:00",
                false,
            )],
            &[task("B", "2026-10-24", false)],
            Lead::AtStart,
            now,
        );
        let scheduled: Vec<String> = first.reminders.iter().map(|r| r.id.clone()).collect();
        let (to_schedule, to_cancel) = reconcile(&scheduled, &first.reminders);
        assert_eq!(to_schedule, first.reminders);
        assert!(to_cancel.is_empty(), "the same data asks for nothing new");

        // The task is ticked off at the Mac and the pull lands that: its request goes, the entry's stays.
        let second = plan(
            &[entry(
                "A",
                "2026-10-24T09:00:00",
                "2026-10-24T10:00:00",
                false,
            )],
            &[task("B", "2026-10-24", true)],
            Lead::AtStart,
            now,
        );
        let (_, to_cancel) = reconcile(&scheduled, &second.reminders);
        assert_eq!(to_cancel, vec!["task:Projects/Home/B.md".to_owned()]);
    }

    /// The line under the switch, and the two forms a link can take.
    #[test]
    fn the_summary_and_the_link_read_as_a_person_would() {
        let now = at("2026-10-24", 7, 0);
        let plan = plan(
            &[entry(
                "Zahnarzt",
                "2026-10-24T10:00:00",
                "2026-10-24T10:45:00",
                false,
            )],
            &[],
            Lead::Ten,
            now,
        );
        assert_eq!(
            plan.summary(now),
            "1 reminder · next Today 09:50 · Zahnarzt"
        );
        assert_eq!(
            Plan::default().summary(now),
            "Nothing scheduled",
            "an empty plan is not a failure"
        );
        assert_eq!(
            link_day("calendar:2026-10-24"),
            NaiveDate::from_ymd_opt(2026, 10, 24)
        );
        assert_eq!(
            link_day("settings"),
            None,
            "a link that names no day opens none"
        );
    }
}
