//! The wishlist's comparison view for one wish group: price against a spec, price over time,
//! and a table of what each candidate offers per euro.
//!
//! The rules live in `sjel_gpui_core::inventory::compare`. This file draws. Each candidate keeps
//! one colour across the scatter, the history and the table, so the table is the legend.

use std::rc::Rc;

use crate::ely::buttons::{Button, ButtonVariant};
use crate::ely::charts::{Points, ScatterChart};
use crate::ely::data_display::Tone;
use crate::ely::tables::{Cell, Column, DataTable, Row, SortKey};
use crate::ely::theme::{ControlSize, Density};
use gpui_kit::{
    canvas, div, point, prelude::*, px, rems, rgb, rgba, AnyElement, Context, PathBuilder, Pixels,
};
use sjel_gpui_core::inventory::compare::{
    day_number, format_per_unit, history_domains, scatter_domains, Fit, GroupComparison,
};
use sjel_gpui_core::inventory::{format_cents, WishGroup};
use sjel_gpui_core::shell::{self, ShellError};

use super::InventoryPage;
use crate::theme::{tint, Palette};

const SCATTER_HEIGHT: f32 = 220.;
const HISTORY_HEIGHT: f32 = 120.;
/// Inside the frame, so a point on the domain edge is not clipped.
const INSET: f32 = 14.;

fn series_colour(palette: &Palette, index: usize) -> u32 {
    let colours = [
        palette.accent,
        palette.green,
        palette.amber,
        palette.red,
        palette.text,
        palette.muted,
    ];
    colours[index % colours.len()]
}

impl InventoryPage {
    /// Flip a group between "every member counts" and "pick one".
    pub(super) fn set_alternatives(&mut self, group: WishGroup, cx: &mut Context<Self>) {
        let shell = match self.repo.shell() {
            Ok(shell) => shell,
            Err(error) => {
                self.wish_error = Some(error.to_string());
                cx.notify();
                return;
            }
        };
        self.group_busy = Some(group.id.clone());
        let work = shell::run(async move {
            shell
                .inventory_set_alternatives(
                    &group.id,
                    &group.title,
                    &group.constraint_text,
                    !group.alternatives,
                )
                .await
        });
        cx.spawn(async move |this, cx| {
            let result = work.await.unwrap_or(Err(ShellError::Unreachable));
            let _ = this.update(cx, |page, cx| {
                page.group_busy = None;
                match result {
                    Ok(()) => page.load(cx),
                    Err(error) => page.wish_error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The comparison for the selected group, or None when the group has nothing to compare.
    pub(super) fn render_comparison(
        &self,
        group_id: &str,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let group = self
            .comparisons
            .value
            .as_ref()?
            .iter()
            .find(|g| g.id == group_id)?
            .clone();
        if !group.is_comparable() {
            return Some(
                div()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child(
                        "Compare: give two candidates a price (`inventory price`) and specs \
                         (`inventory specs`), or let the daily refresh read their geizhals links.",
                    )
                    .into_any_element(),
            );
        }
        let axes = group.axis_keys();
        let axis = self
            .compare_axis
            .clone()
            .filter(|key| axes.contains(key))
            .or_else(|| group.default_axis());

        let mut panel = div()
            .p_4()
            .rounded(px(12.))
            .bg(rgba(tint(palette.raised, 0x22)))
            .border_1()
            .border_color(rgba(tint(palette.border, 0x30)))
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(format!("Compare · {} candidates", group.candidates.len())),
                    )
                    .child(div().flex().gap_1().children(axes.iter().map(|key| {
                        let chosen = axis.as_deref() == Some(key.as_str());
                        let pick = key.clone();
                        let button = Button::new(format!("compare-axis-{key}"), key.clone())
                            .size(ControlSize::Sm)
                            .on_click(cx.listener(move |page, _, _, cx| {
                                page.compare_axis = Some(pick.clone());
                                cx.notify();
                            }));
                        if chosen {
                            button.primary()
                        } else {
                            button.variant(ButtonVariant::Ghost)
                        }
                    }))),
            );
        if let Some(axis) = &axis {
            panel = panel.children(scatter(&group, axis, palette));
        }
        panel = panel
            .children(history(&group, palette))
            .child(table(&group, palette));
        Some(panel.into_any_element())
    }
}

/// Price (up) against one spec (right). A vertical line marks the group's need on that spec,
/// a horizontal one each distinct price limit.
fn scatter(group: &GroupComparison, axis: &str, palette: Palette) -> Option<AnyElement> {
    let (x, y) = scatter_domains(group, axis)?;
    let points: Vec<(f64, f64, u32, bool)> = group
        .candidates
        .iter()
        .enumerate()
        .filter_map(|(i, c)| {
            let latest = c.latest()?;
            Some((
                c.spec(axis)?,
                latest.price_cents as f64,
                series_colour(&palette, i),
                c.within_limit() == Some(true),
            ))
        })
        .collect();
    let need = group.need(axis);
    let mut limits: Vec<f64> = group
        .candidates
        .iter()
        .filter_map(|c| c.cost_max_cents.map(|v| v as f64))
        .collect();
    limits.sort_by(f64::total_cmp);
    limits.dedup();
    let unit = group.unit_of(axis);

    let mut ely_scatter = ScatterChart::new("wishlist-compare-scatter")
        .axes(format!("{axis} ({unit})"), "Price (€)")
        .h(px(SCATTER_HEIGHT))
        .w_full();
    for c in &group.candidates {
        if let (Some(spec_val), Some(latest)) = (c.spec(axis), c.latest()) {
            ely_scatter = ely_scatter.points(Points::new(
                c.label.clone(),
                [(spec_val, latest.price_cents as f64 / 100.0)],
            ));
        }
    }

    let overlay = canvas(
        |bounds, _, _| bounds,
        move |bounds, _, window, _| {
            let w = f32::from(bounds.size.width) - INSET * 2.;
            let h = f32::from(bounds.size.height) - INSET * 2.;
            let at = |fx: f64, fy: f64| {
                bounds.origin + point(px(INSET + fx as f32 * w), px(INSET + (1. - fy as f32) * h))
            };
            let line = |window: &mut gpui_kit::Window,
                        from: gpui_kit::Point<Pixels>,
                        to: gpui_kit::Point<Pixels>,
                        colour: u32| {
                let mut path = PathBuilder::stroke(px(1.5));
                path.move_to(from);
                path.line_to(to);
                if let Ok(path) = path.build() {
                    window.paint_path(path, rgba(colour));
                }
            };
            if let Some(need) = need {
                let fx = x.fraction(need);
                line(window, at(fx, 0.), at(fx, 1.), tint(palette.amber, 0xb0));
            }
            for limit in &limits {
                let fy = y.fraction(*limit);
                line(window, at(0., fy), at(1., fy), tint(palette.red, 0x90));
            }
            for (vx, vy, colour, within) in &points {
                let c = at(x.fraction(*vx), y.fraction(*vy));
                let r = if *within { 7. } else { 5. };
                let mut dot = PathBuilder::fill();
                dot.move_to(c + point(px(-r), px(-r)));
                dot.line_to(c + point(px(r), px(-r)));
                dot.line_to(c + point(px(r), px(r)));
                dot.line_to(c + point(px(-r), px(r)));
                dot.close();
                if let Ok(path) = dot.build() {
                    window.paint_path(path, rgb(*colour));
                }
            }
        },
    )
    .absolute()
    .inset_0()
    .size_full();

    let axis_note = |text: String| div().text_xs().text_color(rgb(palette.muted)).child(text);
    Some(
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(axis_note(format!(
                "Price against {axis}. Amber: need{}. Red: price limit. Hover points for interactive tooltip.",
                need.map(|n| format!(" {n} {unit}")).unwrap_or_default()
            )))
            .child(
                div()
                    .relative()
                    .h(px(SCATTER_HEIGHT))
                    .w_full()
                    .rounded_md()
                    .overflow_hidden()
                    .bg(rgba(tint(palette.surface, 0x60)))
                    .child(ely_scatter)
                    .child(overlay),
            )
            .child(
                div()
                    .flex()
                    .justify_between()
                    .child(axis_note(format!(
                        "{:.0} {unit} · {}",
                        x.min,
                        format_cents(Some(y.min as i64))
                    )))
                    .child(axis_note(format!(
                        "{:.0} {unit} · {}",
                        x.max,
                        format_cents(Some(y.max as i64))
                    ))),
            )
            .into_any_element(),
    )
}

/// Each candidate's prices over time, one line per candidate.
fn history(group: &GroupComparison, palette: Palette) -> Option<AnyElement> {
    let (x, y) = history_domains(group)?;
    let series: Vec<(Vec<(f64, f64)>, u32)> = group
        .candidates
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let points = c
                .observations
                .iter()
                .filter_map(|o| Some((day_number(&o.observed_at)? as f64, o.price_cents as f64)))
                .collect();
            (points, series_colour(&palette, i))
        })
        .collect();
    let draw = canvas(
        |bounds, _, _| bounds,
        move |bounds, _, window, _| {
            let w = f32::from(bounds.size.width) - INSET * 2.;
            let h = f32::from(bounds.size.height) - INSET * 2.;
            let at = |vx: f64, vy: f64| {
                bounds.origin
                    + point(
                        px(INSET + x.fraction(vx) as f32 * w),
                        px(INSET + (1. - y.fraction(vy) as f32) * h),
                    )
            };
            for (points, colour) in &series {
                if let [only] = points.as_slice() {
                    let c = at(only.0, only.1);
                    let mut tick = PathBuilder::stroke(px(2.));
                    tick.move_to(c + point(px(-4.), px(0.)));
                    tick.line_to(c + point(px(4.), px(0.)));
                    if let Ok(path) = tick.build() {
                        window.paint_path(path, rgb(*colour));
                    }
                    continue;
                }
                let mut path = PathBuilder::stroke(px(2.));
                for (i, p) in points.iter().enumerate() {
                    if i == 0 {
                        path.move_to(at(p.0, p.1));
                    } else {
                        path.line_to(at(p.0, p.1));
                    }
                }
                if let Ok(path) = path.build() {
                    window.paint_path(path, rgb(*colour));
                }
            }
        },
    )
    .size_full();
    let first = group
        .candidates
        .iter()
        .flat_map(|c| &c.observations)
        .map(|o| &o.observed_at[..10.min(o.observed_at.len())])
        .min()
        .unwrap_or_default()
        .to_owned();
    let last = group
        .candidates
        .iter()
        .flat_map(|c| &c.observations)
        .map(|o| &o.observed_at[..10.min(o.observed_at.len())])
        .max()
        .unwrap_or_default()
        .to_owned();
    Some(
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(palette.muted))
                    .child(format!("Price history, {first} to {last}")),
            )
            .child(
                div()
                    .h(px(HISTORY_HEIGHT))
                    .w_full()
                    .rounded_md()
                    .overflow_hidden()
                    .bg(rgba(tint(palette.surface, 0x60)))
                    .child(draw),
            )
            .into_any_element(),
    )
}

/// One row per candidate: label, latest price, limit, needs fit, and price per unit.
fn table(group: &GroupComparison, _palette: Palette) -> AnyElement {
    let columns = [
        Column::new("candidate", "CANDIDATE"),
        Column::new("price", "LATEST PRICE").width(rems(8.)).end(),
        Column::new("limit", "LIMIT").width(rems(8.)),
        Column::new("needs", "NEEDS FIT").width(rems(9.)),
        Column::new("value", "PER UNIT"),
    ];
    let rows: Vec<Row> = group
        .candidates
        .iter()
        .map(|c| {
            let (price_cell, source_meta) = match c.latest() {
                Some(o) => {
                    let date = &o.observed_at[..10.min(o.observed_at.len())];
                    let src = short_source(&o.source);
                    let tone = match c.within_limit() {
                        Some(true) => Tone::Success,
                        Some(false) => Tone::Warning,
                        None => Tone::Neutral,
                    };
                    (
                        Cell::Figure(
                            format_cents(Some(o.price_cents)).into(),
                            o.price_cents as f64,
                            tone,
                        ),
                        format!("{date} · {src}"),
                    )
                }
                None => (Cell::Text("no price yet".into()), String::new()),
            };
            let limit_cell = match (c.cost_max_cents, c.within_limit()) {
                (None, _) => Cell::Text("no limit".into()),
                (Some(max), Some(true)) => Cell::Tag(
                    format!("≤ {}", format_cents(Some(max))).into(),
                    Tone::Success,
                ),
                (Some(max), _) => Cell::Tag(
                    format!("limit {}", format_cents(Some(max))).into(),
                    Tone::Neutral,
                ),
            };
            let fits: Vec<String> = c
                .fits(&group.needs)
                .into_iter()
                .map(|fit| match fit {
                    Fit::Meets => "meets".to_owned(),
                    Fit::Short { key, have, need } => format!("{key} {have} < {need}"),
                    Fit::Unknown { key } => format!("{key} unknown"),
                })
                .collect();
            let all_meet = !fits.is_empty() && fits.iter().all(|f| f == "meets");
            let needs_cell = if fits.is_empty() {
                Cell::Empty
            } else {
                Cell::Tag(
                    fits.join(", ").into(),
                    if all_meet {
                        Tone::Success
                    } else {
                        Tone::Warning
                    },
                )
            };
            let scores: Vec<String> = c.scores().iter().map(format_per_unit).collect();
            let alerted = c.observations.iter().any(|o| o.alert);
            let meta = match (source_meta.is_empty(), alerted) {
                (false, true) => format!("{source_meta} · limit reached"),
                (false, false) => source_meta,
                (true, true) => "limit reached".to_owned(),
                (true, false) => String::new(),
            };
            let name_cell = if meta.is_empty() {
                Cell::Text(c.label.clone().into())
            } else {
                Cell::Lines(c.label.clone().into(), meta.into())
            };
            Row::new(
                c.label.clone(),
                [
                    name_cell,
                    price_cell,
                    limit_cell,
                    needs_cell,
                    Cell::Text(scores.join(" · ").into()),
                ],
            )
        })
        .collect();

    DataTable::new("wishlist-comparison-table", columns)
        .label("Candidate comparison")
        .rows(Rc::new(rows))
        .density(Density::Comfortable)
        .sorts([SortKey {
            column: "price".into(),
            rising: true,
        }])
        .into_any_element()
}

/// A URL shows as its host; a manual label shows as itself.
fn short_source(source: &str) -> String {
    source
        .strip_prefix("https://")
        .and_then(|rest| rest.split('/').next())
        .map(|host| host.trim_start_matches("www.").to_owned())
        .unwrap_or_else(|| source.to_owned())
}
