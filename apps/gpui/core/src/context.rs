//! Core domain logic for the Today / Context surface.
//!
//! Gathers a day's calendar events, tasks due or touched, and financial transactions
//! into a unified timeline and summary metrics.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::shell::{CalendarEntry, FinanceTransaction, GlobalTask};

/// The kind of timeline item.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TimelineKind {
    Event,
    Task,
    Spend,
}

/// A unified item on the day's timeline.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimelineItem {
    pub id: String,
    pub kind: TimelineKind,
    /// Time string if known (e.g. "09:30"), or None for all-day / anytime.
    pub time: Option<String>,
    pub title: String,
    pub subtitle: String,
    pub badge: Option<String>,
    pub amount_cents: Option<i64>,
    pub currency: Option<String>,
    pub completed: bool,
}

/// How many urgent rows Today draws before the rest becomes a count.
pub const ATTENTION_SHOWN: usize = 6;

/// Open tasks that are P1 or due by `today` (`YYYY-MM-DD`). The TASKS count and its rows both
/// call this, so they cannot disagree.
pub fn needs_attention(tasks: &[GlobalTask], today: &str) -> Vec<GlobalTask> {
    tasks
        .iter()
        .filter(|task| !task.done)
        .filter(|task| {
            task.priority == 1
                || task
                    .due
                    .as_deref()
                    .is_some_and(|due| !due.is_empty() && due <= today)
        })
        .cloned()
        .collect()
}

/// Consolidated state for a day's context.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ContextDay {
    pub day: NaiveDate,
    pub events: Vec<CalendarEntry>,
    pub tasks: Vec<GlobalTask>,
    pub spend: Vec<FinanceTransaction>,
    pub total_spent_cents: i64,
    pub currency: String,
    pub timeline: Vec<TimelineItem>,
}

impl ContextDay {
    /// Builds the consolidated day context from individual capability answers.
    pub fn build(
        day: NaiveDate,
        calendar_entries: &[CalendarEntry],
        all_tasks: &[GlobalTask],
        transactions: &[FinanceTransaction],
    ) -> Self {
        let day_str = day.format("%Y-%m-%d").to_string();

        // 1. Filter events on this day
        let events = crate::calendar::entries_on(calendar_entries, day)
            .into_iter()
            .cloned()
            .collect::<Vec<_>>();

        // 2. Filter tasks relevant for this day (due today, scheduled for today, or open)
        let tasks = all_tasks
            .iter()
            .filter(|t| {
                if let Some(due) = &t.due {
                    if due.starts_with(&day_str) {
                        return true;
                    }
                }
                // Also show open tasks if today is the current day
                false
            })
            .cloned()
            .collect::<Vec<_>>();

        // 3. Filter transactions on this day
        let spend = transactions
            .iter()
            .filter(|tx| tx.date == day_str)
            .cloned()
            .collect::<Vec<_>>();

        // Calculate total spend (expenses only, in cents)
        let mut total_spent_cents = 0i64;
        let mut currency = "EUR".to_string();
        for tx in &spend {
            if tx.kind == "expense" {
                total_spent_cents += tx.amount_cents.abs();
                currency = tx.currency.clone();
            }
        }

        // 4. Build timeline items
        let mut timeline = Vec::new();

        for event in &events {
            let time = if event.all_day {
                None
            } else if event.starts_at.len() >= 16 {
                Some(event.starts_at[11..16].to_string())
            } else {
                None
            };
            timeline.push(TimelineItem {
                id: format!("event:{}", event.id),
                kind: TimelineKind::Event,
                time,
                title: event.title.clone(),
                subtitle: event.location.clone().unwrap_or_else(|| event.kind.clone()),
                badge: Some(event.kind.clone()),
                amount_cents: None,
                currency: None,
                completed: false,
            });
        }

        for task in &tasks {
            let time = task.due.as_ref().and_then(|d| {
                if d.len() >= 16 && d.contains('T') {
                    Some(d[11..16].to_string())
                } else {
                    None
                }
            });
            let badge = if task.priority > 0 {
                Some(format!("P{}", task.priority))
            } else {
                None
            };
            timeline.push(TimelineItem {
                id: format!("task:{}", task.id),
                kind: TimelineKind::Task,
                time,
                title: task.title.clone(),
                subtitle: task.project.clone(),
                badge,
                amount_cents: None,
                currency: None,
                completed: task.done,
            });
        }

        for tx in &spend {
            let badge = if tx.kind == "expense" {
                Some("Spend".to_string())
            } else if tx.kind == "income" {
                Some("Income".to_string())
            } else {
                Some("Transfer".to_string())
            };
            timeline.push(TimelineItem {
                id: format!("spend:{}", tx.id),
                kind: TimelineKind::Spend,
                time: None,
                title: tx.description.clone(),
                subtitle: tx.category.clone(),
                badge,
                amount_cents: Some(tx.amount_cents),
                currency: Some(tx.currency.clone()),
                completed: false,
            });
        }

        // Sort timeline:
        // All-day / untimed first (time == None), then sorted chronologically by time.
        timeline.sort_by(|a, b| match (&a.time, &b.time) {
            (None, None) => a.kind_rank().cmp(&b.kind_rank()),
            (None, Some(_)) => std::cmp::Ordering::Less,
            (Some(_), None) => std::cmp::Ordering::Greater,
            (Some(t1), Some(t2)) => t1.cmp(t2).then_with(|| a.kind_rank().cmp(&b.kind_rank())),
        });

        Self {
            day,
            events,
            tasks,
            spend,
            total_spent_cents,
            currency,
            timeline,
        }
    }
}

impl TimelineItem {
    fn kind_rank(&self) -> u8 {
        match self.kind {
            TimelineKind::Event => 0,
            TimelineKind::Task => 1,
            TimelineKind::Spend => 2,
        }
    }
}

/// Format cents into human-readable currency (e.g. 1450 EUR -> "14,50 €").
pub fn format_money(cents: i64, currency: &str) -> String {
    let sign = if cents < 0 { "-" } else { "" };
    let abs_cents = cents.abs();
    let whole = abs_cents / 100;
    let fraction = abs_cents % 100;

    let symbol = match currency {
        "EUR" => "€",
        "USD" => "$",
        "GBP" => "£",
        "CHF" => "CHF",
        _ => currency,
    };

    if currency == "EUR" {
        format!("{sign}{whole},{fraction:02} {symbol}")
    } else {
        format!("{sign}{symbol}{whole}.{fraction:02}")
    }
}

/// Format a date as a rich header string (e.g. "Thursday, October 22, 2026").
pub fn format_day_header(day: NaiveDate) -> String {
    day.format("%A, %B %-d, %Y").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_money_renders_correctly() {
        assert_eq!(format_money(1450, "EUR"), "14,50 €");
        assert_eq!(format_money(-999, "EUR"), "-9,99 €");
        assert_eq!(format_money(0, "EUR"), "0,00 €");
        assert_eq!(format_money(2500, "USD"), "$25.00");
    }

    #[test]
    fn attention_is_open_and_either_p1_or_due_by_today() {
        let task = |id: &str, due: Option<&str>, priority: u8, done: bool| GlobalTask {
            id: id.to_owned(),
            kind: "task".to_owned(),
            title: id.to_owned(),
            done,
            due: due.map(ToOwned::to_owned),
            priority,
            summary: None,
            project: String::new(),
        };
        let tasks = vec![
            task("p1-no-due", None, 1, false),
            task("p1-done", None, 1, true),
            task("overdue", Some("2026-10-22"), 2, false),
            task("due-today", Some("2026-10-24"), 3, false),
            task("later", Some("2026-10-26"), 2, false),
            task("undated-p2", None, 2, false),
            task("empty-due", Some(""), 2, false),
        ];
        let ids = needs_attention(&tasks, "2026-10-24")
            .into_iter()
            .map(|task| task.id)
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["p1-no-due", "overdue", "due-today"]);
    }

    #[test]
    fn context_day_orders_untimed_first_then_chronological() {
        let day = NaiveDate::from_ymd_opt(2026, 10, 22).unwrap();

        let entries = vec![
            CalendarEntry {
                all_day: false,
                starts_at: "2026-10-22T14:00:00".into(),
                ends_at: "2026-10-22T15:00:00".into(),
                title: "Afternoon Sync".into(),
                commitment: "planned".into(),
                id: "e1".into(),
                kind: "meeting".into(),
                location: None,
                notes: None,
                source: "manual".into(),
            },
            CalendarEntry {
                all_day: true,
                starts_at: "2026-10-22T00:00:00".into(),
                ends_at: "2026-10-23T00:00:00".into(),
                title: "Holiday".into(),
                commitment: "planned".into(),
                id: "e2".into(),
                kind: "holiday".into(),
                location: None,
                notes: None,
                source: "manual".into(),
            },
            CalendarEntry {
                all_day: false,
                starts_at: "2026-10-22T09:00:00".into(),
                ends_at: "2026-10-22T10:00:00".into(),
                title: "Morning Standup".into(),
                commitment: "planned".into(),
                id: "e3".into(),
                kind: "meeting".into(),
                location: None,
                notes: None,
                source: "manual".into(),
            },
        ];

        let txs = vec![FinanceTransaction {
            id: "tx1".into(),
            date: "2026-10-22".into(),
            description: "Coffee".into(),
            kind: "expense".into(),
            account: "Checking".into(),
            category: "food".into(),
            amount_cents: -450,
            currency: "EUR".into(),
            source_id: None,
            trip_id: None,
        }];

        let context = ContextDay::build(day, &entries, &[], &txs);
        assert_eq!(context.total_spent_cents, 450);
        assert_eq!(context.timeline.len(), 4);

        // First item should be untimed (All Day event or spend)
        assert!(context.timeline[0].time.is_none());
        assert_eq!(context.timeline[0].title, "Holiday");

        // Next untimed should be spend
        assert!(context.timeline[1].time.is_none());
        assert_eq!(context.timeline[1].title, "Coffee");

        // Then 09:00, then 14:00
        assert_eq!(context.timeline[2].time.as_deref(), Some("09:00"));
        assert_eq!(context.timeline[2].title, "Morning Standup");

        assert_eq!(context.timeline[3].time.as_deref(), Some("14:00"));
        assert_eq!(context.timeline[3].title, "Afternoon Sync");
    }
}
