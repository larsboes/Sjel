//! Ely chart views for Projects portfolio health and task due-date pressure.

use std::collections::BTreeMap;

use chrono::{Duration, Local, NaiveDate};
use gpui_kit::{div, prelude::*, px, AnyElement, Context};
use sjel_gpui_core::shell::{GlobalTask, ProjectSummary};

use crate::ely::charts::{Bullet, BulletChart, CalendarHeatmap, Points, ScatterChart, Treemap};
use crate::primitives::chart_card;
use crate::theme::{Density, Palette};

use super::Projects;

const DUE_WINDOW_DAYS: i64 = 56;
const MAX_BULLET_PROJECTS: usize = 6;

/// Portfolio charts for the Projects hub: open-task distribution (`Treemap`) and
/// completion progress (`BulletChart`) across active projects.
pub(super) fn hubs_portfolio_charts(
    projects: &[ProjectSummary],
    density: Density,
    palette: Palette,
    cx: &mut Context<Projects>,
) -> Option<AnyElement> {
    let with_open: Vec<&ProjectSummary> = projects.iter().filter(|p| p.open > 0).collect();
    let with_tasks: Vec<&ProjectSummary> = {
        let mut list: Vec<&ProjectSummary> =
            projects.iter().filter(|p| p.open + p.done > 0).collect();
        list.sort_by(|a, b| b.open.cmp(&a.open).then_with(|| b.done.cmp(&a.done)));
        list
    };
    if with_tasks.is_empty() {
        return None;
    }

    let treemap = (!with_open.is_empty()).then(|| {
        let names: Vec<String> = with_open.iter().map(|p| p.name.clone()).collect();
        let owner = cx.entity();
        let chart = with_open
            .iter()
            .fold(Treemap::new("projects-portfolio-treemap"), |t, project| {
                t.tile(project.name.clone(), project.open as f64)
            })
            .format(|v| format!("{v:.0} open"))
            .on_select(move |ix, _, cx| {
                if let Some(name) = names.get(ix).cloned() {
                    owner.update(cx, |view, cx| {
                        view.open_project(name, cx);
                    });
                }
            })
            .h(px(164.))
            .w_full()
            .into_any_element();
        chart_card(
            "OPEN WORKLOAD",
            Some("Projects sized by open task count · click tile to open".into()),
            chart,
            palette,
        )
    });

    let bullets = {
        let mut chart = BulletChart::new("projects-health-bullets")
            .format(|v| format!("{v:.0} done"));
        for project in with_tasks.iter().take(MAX_BULLET_PROJECTS) {
            let total = (project.open + project.done).max(1) as f64;
            chart = chart.bullet(
                Bullet::new(project.name.clone(), project.done as f64, total).bands([
                    total * 0.5,
                    total * 0.85,
                    total,
                ]),
            );
        }
        chart_card(
            "PROJECT COMPLETION",
            Some("Done tasks against total scope".into()),
            chart.w_full().into_any_element(),
            palette,
        )
    };

    let row = if density == Density::Mobile {
        div().flex().flex_col().gap_3()
    } else {
        div().flex().items_stretch().gap_3()
    };

    Some(
        row.when_some(treemap, |el, card| {
            el.child(div().flex_1().min_w_0().child(card))
        })
        .child(div().flex_1().min_w_0().child(bullets))
        .mb_3()
        .into_any_element(),
    )
}

/// Counts active tasks due on each day in `[start, start + days)`.
pub(super) fn due_counts_window(
    tasks: &[GlobalTask],
    start: NaiveDate,
    days: i64,
) -> Vec<f64> {
    let mut by_date: BTreeMap<NaiveDate, usize> = BTreeMap::new();
    for task in tasks {
        let status = task.status();
        if status == "done" || status == "cancelled" {
            continue;
        }
        let Some(due_raw) = task.due.as_deref() else {
            continue;
        };
        let Ok(due) = NaiveDate::parse_from_str(due_raw.trim(), "%Y-%m-%d") else {
            continue;
        };
        if due >= start && due < start + Duration::days(days) {
            *by_date.entry(due).or_insert(0) += 1;
        }
    }
    (0..days)
        .map(|offset| {
            let day = start + Duration::days(offset);
            by_date.get(&day).copied().unwrap_or(0) as f64
        })
        .collect()
}

/// Task pressure charts for `AllTasks`: an 8-week due-date `CalendarHeatmap` and
/// an urgency vs. priority `ScatterChart`.
pub(super) fn tasks_pressure_charts(
    tasks: &[GlobalTask],
    density: Density,
    palette: Palette,
    cx: &mut Context<Projects>,
) -> Option<AnyElement> {
    let today = Local::now().date_naive();
    let counts = due_counts_window(tasks, today, DUE_WINDOW_DAYS);
    let has_window_due = counts.iter().any(|&c| c > 0.0);

    let heatmap = if has_window_due {
        let owner = cx.entity();
        today
            .to_string()
            .parse()
            .ok()
            .map(|first| {
                let chart = CalendarHeatmap::new("tasks-due-heatmap", first, counts)
                    .format(|c| format!("{c:.0} due"))
                    .on_select(move |offset, _, cx| {
                        let date = (today + Duration::days(offset as i64))
                            .format("%Y-%m-%d")
                            .to_string();
                        owner.update(cx, |view, cx| {
                            view.due_date_filter = if view.due_date_filter.as_deref() == Some(&date)
                            {
                                None
                            } else {
                                Some(date)
                            };
                            cx.notify();
                        });
                    })
                    .w_full()
                    .h(px(96.))
                    .into_any_element();
                chart_card(
                    "DUE-DATE PRESSURE",
                    Some("Active tasks due over the next 8 weeks · click day to filter".into()),
                    chart,
                    palette,
                )
            })
    } else {
        None
    };

    let mut p1_pts = Vec::new();
    let mut p2_pts = Vec::new();
    let mut p3_pts = Vec::new();
    for task in tasks {
        let status = task.status();
        if status == "done" || status == "cancelled" {
            continue;
        }
        let Some(due_raw) = task.due.as_deref() else {
            continue;
        };
        let Ok(due) = NaiveDate::parse_from_str(due_raw.trim(), "%Y-%m-%d") else {
            continue;
        };
        let days_until = (due - today).num_days().clamp(-14, 30) as f64;
        match task.priority {
            1 => p1_pts.push((days_until, 3.0)),
            2 => p2_pts.push((days_until, 2.0)),
            _ => p3_pts.push((days_until, 1.0)),
        }
    }

    let scatter = (!p1_pts.is_empty() || !p2_pts.is_empty() || !p3_pts.is_empty()).then(|| {
        let mut chart = ScatterChart::new("tasks-urgency-priority")
            .axes("Days until due", "Priority (P1 top)")
            .h(px(140.))
            .w_full();
        if !p1_pts.is_empty() {
            chart = chart.points(Points::new("P1", p1_pts));
        }
        if !p2_pts.is_empty() {
            chart = chart.points(Points::new("P2", p2_pts));
        }
        if !p3_pts.is_empty() {
            chart = chart.points(Points::new("P3", p3_pts));
        }
        chart_card(
            "URGENCY × PRIORITY",
            Some("Dated active tasks (negative = overdue)".into()),
            chart.into_any_element(),
            palette,
        )
    });

    if heatmap.is_none() && scatter.is_none() {
        return None;
    }

    let row = if density == Density::Mobile {
        div().flex().flex_col().gap_3()
    } else {
        div().flex().items_stretch().gap_3()
    };

    Some(
        row.when_some(heatmap, |el, card| {
            el.child(div().flex_1().min_w_0().child(card))
        })
        .when_some(scatter, |el, card| {
            el.child(div().flex_1().min_w_0().child(card))
        })
        .mb_3()
        .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_task(due: Option<&str>, done: bool) -> GlobalTask {
        GlobalTask {
            id: "t".into(),
            kind: "task".into(),
            title: "Task".into(),
            done,
            due: due.map(str::to_string),
            priority: 2,
            summary: None,
            project: "P".into(),
            fields: BTreeMap::new(),
        }
    }

    #[test]
    fn due_counts_window_excludes_done_and_out_of_range() {
        let start = NaiveDate::from_ymd_opt(2026, 11, 20).unwrap();
        let tasks = vec![
            sample_task(Some("2026-11-20"), false),
            sample_task(Some("2026-11-20"), true),
            sample_task(Some("2026-11-22"), false),
            sample_task(Some("2026-11-19"), false),
            sample_task(None, false),
        ];
        let counts = due_counts_window(&tasks, start, 5);
        assert_eq!(counts, vec![1.0, 0.0, 1.0, 0.0, 0.0]);
    }
}
