use super::events::{entry_range, overlaps, resolve, MINUTES_PER_DAY};
use super::*;

/// Return the inclusive/exclusive Calendar query window for a candidate set.
pub fn query_window(candidates: &[Candidate]) -> Result<Option<(String, String)>, String> {
    let mut bounds: Option<(i64, i64)> = None;
    for candidate in candidates {
        let range = resolve(candidate)?;
        let first = range.start.div_euclid(MINUTES_PER_DAY);
        // One day past the exclusive end: an end mid-day still needs that
        // day's entries, and over-fetching a day costs nothing because
        // `verdict_for` filters on the exact range anyway.
        let last = range.end.div_euclid(MINUTES_PER_DAY) + 1;
        bounds = Some(match bounds {
            Some((lo, hi)) => (lo.min(first), hi.max(last)),
            None => (first, last),
        });
    }
    Ok(bounds.map(|(lo, hi)| (date::format_date(lo), date::format_date(hi))))
}

/// Groups `[from_day, to_day)` into the runs of days where travel is possible
/// at all. `min_days` drops runs shorter than that — transit's fare search
/// asks for at least a weekend, and searching a single feasible Tuesday
/// between two `away` blocks is a wasted HAFAS call.
pub fn feasible_windows(
    from_day: i64,
    to_day: i64,
    entries: &[Entry],
    min_days: usize,
) -> Result<Vec<FeasibleWindow>, String> {
    let mut windows = Vec::new();
    let mut run: Vec<(String, Feasibility)> = Vec::new();

    for day in from_day..to_day {
        let range = (day * MINUTES_PER_DAY, (day + 1) * MINUTES_PER_DAY);
        let mut impact = Feasibility::Free;
        for entry in entries {
            if overlaps(range, entry_range(entry)?) {
                impact = impact.max(self::impact(&entry.kind, entry.commitment));
            }
        }
        if impact == Feasibility::Conflicts {
            close_run(&mut windows, &mut run, day, min_days);
        } else {
            run.push((date::format_date(day), impact));
        }
    }
    close_run(&mut windows, &mut run, to_day, min_days);
    Ok(windows)
}

pub(super) fn close_run(
    windows: &mut Vec<FeasibleWindow>,
    run: &mut Vec<(String, Feasibility)>,
    ends_before_day: i64,
    min_days: usize,
) {
    if run.len() >= min_days.max(1) {
        windows.push(FeasibleWindow {
            starts_on: run[0].0.clone(),
            ends_before: date::format_date(ends_before_day),
            days: run.iter().map(|(day, _)| day.clone()).collect(),
            verdict: run
                .iter()
                .map(|(_, impact)| *impact)
                .max()
                .unwrap_or(Feasibility::Free),
            days_needing_travel_day: run
                .iter()
                .filter(|(_, impact)| *impact == Feasibility::NeedsTravelDay)
                .map(|(day, _)| day.clone())
                .collect(),
        });
    }
    run.clear();
}
