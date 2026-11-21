//! Select old mail for the reviewed bulk trash.
//!
//! A cleanup rule only selects. Nothing here calls Gmail: the selected ids go to
//! `POST /triage/bulk` with `action: "trash"`, which waits for the owner's Allow. Receipts,
//! tax mail, keepers and waiting threads are never selected, whatever a rule says.

use std::collections::HashSet;

use serde::Deserialize;

use crate::store::TriageItem;

/// Streams no cleanup rule can select. Receipts and tax mail are evidence.
pub const PROTECTED_STREAMS: [&str; 2] = ["belege", "steuern"];

/// One cleanup rule from the overlay's `cleanup` list. All conditions present must hold.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CleanupRule {
    /// Short name shown in the report.
    pub name: String,
    /// Any of these streams.
    #[serde(default)]
    pub streams: Vec<String>,
    /// Any of these substrings in the stored sender, case-insensitive.
    #[serde(default)]
    pub from_contains: Vec<String>,
    /// Any of these substrings in the stored subject, case-insensitive.
    #[serde(default)]
    pub subject_contains: Vec<String>,
    /// Only mail older than this many days.
    pub older_than_days: u32,
    /// `released`: only threads the owner released (`--released <file>`). Absent: no gate,
    /// for mail with no value.
    #[serde(default)]
    pub gate: Option<String>,
}

impl CleanupRule {
    /// A rule with no stream, sender or subject condition never fires.
    fn selects(&self, item: &TriageItem, cutoff: &str, released: Option<&HashSet<String>>) -> bool {
        match self.gate.as_deref() {
            None => {}
            Some("released") if released.is_some_and(|set| set.contains(&item.id)) => {}
            Some(_) => return false,
        }
        if self.streams.is_empty()
            && self.from_contains.is_empty()
            && self.subject_contains.is_empty()
        {
            return false;
        }
        let any = |needles: &[String], hay: Option<&str>| {
            needles.is_empty() || {
                let hay = hay.unwrap_or_default().to_lowercase();
                needles.iter().any(|n| hay.contains(&n.to_lowercase()))
            }
        };
        let old = item
            .internal_date_text
            .as_deref()
            .and_then(|d| d.get(..10))
            .is_some_and(|day| day < cutoff);
        old && (self.streams.is_empty() || self.streams.contains(&item.stream))
            && any(&self.from_contains, item.from_addr.as_deref())
            && any(&self.subject_contains, item.subject.as_deref())
    }
}

/// Whether any rule may touch this thread at all.
fn eligible(item: &TriageItem) -> bool {
    !PROTECTED_STREAMS.contains(&item.stream.as_str())
        && item.status != "keeper"
        && !item.waiting
        && item.gmail_action.as_deref() != Some("trash")
        && item.gmail_location.as_deref() != Some("trash")
}

#[derive(Debug, serde::Serialize)]
pub struct RuleMatches {
    pub rule: String,
    pub count: usize,
    /// Up to five `date | sender | subject` lines, so the rule can be checked by eye.
    pub examples: Vec<String>,
    pub ids: Vec<String>,
}

/// Each thread counts for the first rule that selects it. `released` holds the thread ids
/// owners released; a `released`-gated rule selects nothing without it.
pub fn select(
    items: &[TriageItem],
    rules: &[CleanupRule],
    today: &str,
    released: Option<&HashSet<String>>,
) -> Vec<RuleMatches> {
    let mut out: Vec<RuleMatches> = rules
        .iter()
        .map(|rule| RuleMatches {
            rule: rule.name.clone(),
            count: 0,
            examples: Vec::new(),
            ids: Vec::new(),
        })
        .collect();
    let cutoffs: Vec<String> = rules
        .iter()
        .map(|rule| days_before(today, rule.older_than_days))
        .collect();
    for item in items.iter().filter(|item| eligible(item)) {
        if let Some(index) = rules
            .iter()
            .zip(&cutoffs)
            .position(|(rule, cutoff)| rule.selects(item, cutoff, released))
        {
            let matches = &mut out[index];
            matches.count += 1;
            if matches.examples.len() < 5 {
                matches.examples.push(format!(
                    "{} | {} | {}",
                    item.internal_date_text
                        .as_deref()
                        .and_then(|d| d.get(..10))
                        .unwrap_or("?"),
                    item.from_addr.as_deref().unwrap_or_default(),
                    item.subject.as_deref().unwrap_or_default()
                ));
            }
            matches.ids.push(item.id.clone());
        }
    }
    out
}

/// One sender's mail among the threads a rule may touch.
#[derive(Debug, serde::Serialize)]
pub struct SenderCount {
    pub sender: String,
    pub count: usize,
    /// The stream most of this sender's mail is in.
    pub stream: String,
    pub oldest: String,
    pub newest: String,
    /// The most recent subjects, newest first, up to the requested number.
    pub examples: Vec<String>,
}

/// Senders by mail count, largest first, over eligible threads only.
/// `items` must be newest first, as `Store::list_triage` returns them.
pub fn senders(items: &[TriageItem], limit: usize, examples: usize) -> Vec<SenderCount> {
    use std::collections::BTreeMap;
    type Tally = (usize, BTreeMap<String, usize>, String, String, Vec<String>);
    let mut by: BTreeMap<String, Tally> = BTreeMap::new();
    for item in items.iter().filter(|item| eligible(item)) {
        let sender = crate::links::address_of(item.from_addr.as_deref().unwrap_or_default());
        let day = item
            .internal_date_text
            .as_deref()
            .and_then(|d| d.get(..10))
            .unwrap_or("?")
            .to_string();
        let entry = by
            .entry(sender)
            .or_insert_with(|| (0, BTreeMap::new(), day.clone(), day.clone(), Vec::new()));
        entry.0 += 1;
        if entry.4.len() < examples {
            entry.4.push(format!(
                "{day} {}",
                item.subject.as_deref().unwrap_or_default()
            ));
        }
        *entry.1.entry(item.stream.clone()).or_default() += 1;
        if day < entry.2 {
            entry.2.clone_from(&day);
        }
        if day > entry.3 {
            entry.3 = day;
        }
    }
    let mut out: Vec<SenderCount> = by
        .into_iter()
        .map(
            |(sender, (count, streams, oldest, newest, examples))| SenderCount {
                sender,
                count,
                stream: streams
                    .into_iter()
                    .max_by_key(|(_, n)| *n)
                    .map(|(s, _)| s)
                    .unwrap_or_default(),
                oldest,
                newest,
                examples,
            },
        )
        .collect();
    out.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.sender.cmp(&b.sender)));
    out.truncate(limit);
    out
}

/// `YYYY-MM-DD` that many days before `today` (`YYYY-MM-DD`). A malformed `today`
/// returns `0000-00-00`, which selects nothing.
pub fn days_before(today: &str, days: u32) -> String {
    let parse =
        |range: std::ops::Range<usize>| today.get(range).and_then(|s| s.parse::<i64>().ok());
    let (Some(y), Some(m), Some(d)) = (parse(0..4), parse(5..7), parse(8..10)) else {
        return "0000-00-00".into();
    };
    let (y, m, d) = civil_from_days(days_from_civil(y, m, d) - i64::from(days));
    format!("{y:04}-{m:02}-{d:02}")
}

/// `YYYY-MM-DD` for days since 1970-01-01.
pub fn date_from_epoch_days(days: i64) -> String {
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}")
}

// Howard Hinnant's civil-date algorithms, proleptic Gregorian.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (
        if m <= 2 {
            yoe + era * 400 + 1
        } else {
            yoe + era * 400
        },
        m,
        d,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_before_crosses_months_years_and_leap_days() {
        assert_eq!(days_before("2026-11-21", 0), "2026-11-21");
        assert_eq!(days_before("2026-11-21", 21), "2026-10-31");
        assert_eq!(days_before("2026-01-01", 1), "2025-12-31");
        assert_eq!(days_before("2024-03-01", 1), "2024-02-29");
        assert_eq!(days_before("2026-11-21", 365), "2025-11-21");
        assert_eq!(days_before("garbage", 3), "0000-00-00");
        assert_eq!(date_from_epoch_days(0), "1970-01-01");
        assert_eq!(date_from_epoch_days(20_413), "2025-11-21");
    }

    #[test]
    fn a_rule_without_conditions_never_fires() {
        let rule = CleanupRule {
            name: "all".into(),
            streams: vec![],
            from_contains: vec![],
            subject_contains: vec![],
            older_than_days: 0,
            gate: None,
        };
        assert!(!rule.selects(&item("werbung", "2020-01-01"), "2030-01-01", None));
    }

    #[test]
    fn protected_keeper_and_waiting_threads_are_never_selected() {
        let rule = CleanupRule {
            name: "old".into(),
            streams: vec!["belege".into(), "werbung".into()],
            from_contains: vec![],
            subject_contains: vec![],
            older_than_days: 30,
            gate: None,
        };
        let mut keeper = item("werbung", "2020-01-01");
        keeper.status = "keeper".into();
        let mut waiting = item("werbung", "2020-01-01");
        waiting.waiting = true;
        let items = [
            item("belege", "2020-01-01"),
            keeper,
            waiting,
            item("werbung", "2026-11-20"),
            item("werbung", "2020-01-01"),
        ];
        let matches = select(&items, std::slice::from_ref(&rule), "2026-11-21", None);
        assert_eq!(matches[0].count, 1);
    }

    #[test]
    fn a_released_gate_selects_only_released_threads() {
        let rule = CleanupRule {
            name: "offers".into(),
            streams: vec!["sonstiges".into()],
            from_contains: vec![],
            subject_contains: vec![],
            older_than_days: 0,
            gate: Some("released".into()),
        };
        let items = [item("sonstiges", "2026-01-01")];
        let rules = std::slice::from_ref(&rule);
        assert_eq!(select(&items, rules, "2026-11-21", None)[0].count, 0);
        let released = HashSet::from(["a1".to_string()]);
        assert_eq!(
            select(&items, rules, "2026-11-21", Some(&released))[0].count,
            1
        );
        let other = HashSet::from(["b2".to_string()]);
        assert_eq!(
            select(&items, rules, "2026-11-21", Some(&other))[0].count,
            0
        );
    }

    fn item(stream: &str, date: &str) -> TriageItem {
        TriageItem {
            internal_date_text: Some(format!("{date} 08:00:00.000+00:00")),
            ..crate::store::db_tests::mk_triage("a1", stream)
        }
    }
}
