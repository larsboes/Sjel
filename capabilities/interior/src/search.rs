//! Position search: keep a layout and step single pieces through the grid until a combination
//! without a hard violation stands.
//!
//! A module rather than a script: "where else does X fit" was asked four times on 2026-08-30,
//! answered four times by a throwaway script with slightly different assumptions. Two things make
//! it fast: a cheap rect-overlap prefilter before the full check (nanoseconds against 0.18 ms), and
//! parallelism over the independent candidates.

use crate::clearance::{check_layout, CheckResult};
use crate::geometry::{point_in_polygon, RES};
use crate::model::{footprint, Layout, Model, ModelError, PlacedItem, Rect, Side};
use rayon::prelude::*;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct Spec {
    /// Ids that may move; everything else stays.
    pub move_refs: Vec<String>,
    /// Grid step in cm. Search coarse, then refine.
    pub step: i32,
    /// Optional per id: allowed band [x0, x1, y0, y1].
    pub bands: BTreeMap<String, [i32; 4]>,
    /// Stop after this many hits; 0 means all.
    pub limit: usize,
}

/// The objectives an arrangement pursues at once. Four, and none wins: more wall behind costs
/// walkway width, more reserve costs wall. One number from four would need three invented weights.
pub const TARGETS: usize = 4;

/// Which candidates can be best under some weighting of the objectives.
///
/// A point is dominated when another is at least as good in every objective and better in one;
/// it can never win, so ranking it behind the front is arithmetic, not taste. All objectives point
/// the same way, bigger is better, so `soft` is passed negated.
///
/// The front is built over distinct objective values, not candidates: ten thousand arrangements
/// collapse to a few hundred distinct quadruples, and the quadratic comparison runs on those.
pub fn pareto_front(points: &[[i32; TARGETS]]) -> Vec<bool> {
    let dominates = |a: &[i32; TARGETS], b: &[i32; TARGETS]| {
        a.iter().zip(b.iter()).all(|(x, y)| x >= y) && a.iter().zip(b.iter()).any(|(x, y)| x > y)
    };
    let mut different: Vec<[i32; TARGETS]> = points.to_vec();
    different.sort_unstable_by(|a, b| b.cmp(a));
    different.dedup();

    let mut front: Vec<[i32; TARGETS]> = Vec::new();
    for t in different {
        if front.iter().any(|f| dominates(f, &t)) {
            continue;
        }
        front.retain(|f| !dominates(&t, f));
        front.push(t);
    }
    let front: std::collections::BTreeSet<[i32; TARGETS]> = front.into_iter().collect();
    points.iter().map(|p| front.contains(p)).collect()
}

#[derive(Debug, Clone, Serialize)]
pub struct Hit {
    pub places: BTreeMap<String, [i32; 2]>,
    pub soft: usize,
    /// Narrowest checked route in cm; higher is better.
    pub bottleneck_cm: i32,
    /// The tightest hard measurement of this arrangement in cm: it passes by this much.
    pub tightest_reserve_cm: Option<i32>,
    /// On the Pareto front: under no weighting worse than another.
    pub pareto: bool,
    /// Centimetres of the moved pieces that touch a wall. The checker knows gaps, not sense: a dining
    /// table in the middle of the room passes as well as one at the wall. This is the other half of the
    /// judgement.
    pub wall_contact_cm: i32,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchReport {
    pub base: String,
    pub moved: Vec<String>,
    pub step: i32,
    pub candidates_after_filter: usize,
    pub fully_checked: usize,
    pub hits: Vec<Hit>,
    pub elapsed_ms: u128,
}

/// Centimetres of rect edges lying against a wall, probed just outside the edge: outside the
/// polygon means a wall. Free-standing pieces get 0 and rank lower, as worse, not as wrong.
fn wall_contact_cm(model: &Model, r: &Rect) -> i32 {
    let poly = &model.room.main_room.polygon;
    let outside = |x: f64, y: f64| !point_in_polygon([x, y], poly);
    let probe = 4.0;
    let step = 10;
    let mut n = 0;
    let mut x = r.x + 5;
    while x < r.right() - 5 {
        if outside(x as f64, r.y as f64 - probe) {
            n += step;
        }
        if outside(x as f64, r.bottom() as f64 + probe) {
            n += step;
        }
        x += step;
    }
    let mut y = r.y + 5;
    while y < r.bottom() - 5 {
        if outside(r.x as f64 - probe, y as f64) {
            n += step;
        }
        if outside(r.right() as f64 + probe, y as f64) {
            n += step;
        }
        y += step;
    }
    n
}

/// Does the piece say it should stand free? A room divider has no wall behind by definition, so it
/// is left out of the wall sum instead of counting 0.
fn is_room_divider(model: &Model, reference: &str) -> bool {
    model
        .catalogue
        .get(reference)
        .and_then(|i| i.room_divider)
        .unwrap_or(false)
}

fn inside_room(model: &Model, r: &Rect) -> bool {
    let poly = &model.room.main_room.polygon;
    let xs = [r.x + 1, r.x + r.w / 2, r.right() - 1];
    let ys = [r.y + 1, r.y + r.d / 2, r.bottom() - 1];
    xs.iter().all(|x| {
        ys.iter()
            .all(|y| point_in_polygon([*x as f64, *y as f64], poly))
    })
}

/// Blocked areas from the model (room.toml, rules.toml), not retyped.
fn blockers(model: &Model, fixed: &[&PlacedItem]) -> Result<Vec<Rect>, ModelError> {
    let mut out = Vec::new();
    for it in fixed {
        let (w, d, _) = footprint(it, &model.catalogue)?;
        out.push(Rect {
            x: it.x,
            y: it.y,
            w,
            d,
        });
    }
    for o in &model.room.openings {
        if let Some(sp) = o.blocked_area {
            out.push(sp.rect());
        }
    }
    Ok(out)
}

/// A hit's four objectives, each turned so bigger is better. A missing reserve counts as
/// `i32::MIN`, so an arrangement nothing was measured on cannot win the front by silence.
fn hit_targets(h: &Hit) -> [i32; TARGETS] {
    [
        h.tightest_reserve_cm.unwrap_or(i32::MIN),
        h.wall_contact_cm,
        h.bottleneck_cm,
        -(h.soft as i32),
    ]
}

pub fn search(model: &Model, base: &Layout, spec: &Spec) -> Result<SearchReport, ModelError> {
    let t0 = std::time::Instant::now();

    let fixed: Vec<&PlacedItem> = base
        .items
        .iter()
        .filter(|i| !spec.move_refs.contains(&i.reference))
        .collect();
    let movers: Vec<&PlacedItem> = spec
        .move_refs
        .iter()
        .map(|r| {
            base.items
                .iter()
                .find(|i| &i.reference == r)
                .ok_or_else(|| {
                    ModelError::Missing(format!(
                        "`{r}` is not in \"{}\": nothing to move",
                        base.name
                    ))
                })
        })
        .collect::<Result<_, _>>()?;

    let block = blockers(model, &fixed)?;

    // Collect every geometrically allowed combination before checking anything.
    let mut combos: Vec<Vec<(String, [i32; 2], Rect)>> = vec![Vec::new()];
    for m in &movers {
        let (w, d, _) = footprint(m, &model.catalogue)?;
        // Without a band, the extent of the flat's polygon, not a remembered number.
        let b = spec.bands.get(&m.reference).copied().unwrap_or_else(|| {
            let poly = &model.room.main_room.polygon;
            [
                poly.iter().map(|q| q[0]).min().unwrap_or(0),
                poly.iter().map(|q| q[0]).max().unwrap_or(0),
                poly.iter().map(|q| q[1]).min().unwrap_or(0),
                poly.iter().map(|q| q[1]).max().unwrap_or(0),
            ]
        });
        let mut next = Vec::new();
        for prefix in &combos {
            let mut y = b[2];
            while y <= b[3] - d {
                let mut x = b[0];
                while x <= b[1] - w {
                    let r = Rect { x, y, w, d };
                    if inside_room(model, &r)
                        && !block.iter().any(|q| r.overlaps(q))
                        && !prefix.iter().any(|(_, _, pr)| r.overlaps(pr))
                    {
                        let mut c = prefix.clone();
                        c.push((m.reference.clone(), [x, y], r));
                        next.push(c);
                    }
                    x += spec.step;
                }
                y += spec.step;
            }
        }
        combos = next;
    }
    let candidates = combos.len();

    // The full check in parallel: candidates are independent, the model is only read.
    let checked = std::sync::atomic::AtomicUsize::new(0);
    let mut hits: Vec<Hit> = combos
        .par_iter()
        .filter_map(|combo| {
            let mut items: Vec<PlacedItem> = fixed.iter().map(|i| (*i).clone()).collect();
            for (reference, pos, _) in combo {
                let src = movers.iter().find(|m| &m.reference == reference)?;
                items.push(PlacedItem {
                    x: pos[0],
                    y: pos[1],
                    ..(*src).clone()
                });
            }
            let l = Layout {
                name: format!("{} (search)", base.name),
                items,
                id: String::new(),
            };
            checked.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let r: CheckResult = check_layout(model, &l).ok()?;
            if !r.hard.is_empty() {
                return None;
            }
            let bottleneck = r
                .metrics
                .corridors
                .iter()
                .filter_map(|c| c.width_cm)
                .min()
                .unwrap_or(0);
            Some(Hit {
                places: combo.iter().map(|(k, p, _)| (k.clone(), *p)).collect(),
                soft: r.soft.len(),
                bottleneck_cm: bottleneck,
                tightest_reserve_cm: r.tightest_reserve_cm,
                pareto: false,
                wall_contact_cm: combo
                    .iter()
                    .filter(|(reference, _, _)| !is_room_divider(model, reference))
                    .map(|(_, _, rect)| wall_contact_cm(model, rect))
                    .sum(),
            })
        })
        .collect();

    // The front first: a dominated hit is worse than a particular other under every weighting, so it
    // cannot be the answer. Within the front the older order applies, since a list needs one:
    // no warnings, wall behind, widest bottleneck.
    let targets: Vec<[i32; TARGETS]> = hits.iter().map(hit_targets).collect();
    for (h, front) in hits.iter_mut().zip(pareto_front(&targets)) {
        h.pareto = front;
    }
    // Reserve as the last stage makes the chain a lexicographic order over all four objectives, so no
    // hit can rank before one that beats it everywhere. A test holds this.
    hits.sort_by(|a, b| {
        b.pareto
            .cmp(&a.pareto)
            .then(a.soft.cmp(&b.soft))
            .then(b.wall_contact_cm.cmp(&a.wall_contact_cm))
            .then(b.bottleneck_cm.cmp(&a.bottleneck_cm))
            .then(b.tightest_reserve_cm.cmp(&a.tightest_reserve_cm))
    });
    if spec.limit > 0 {
        hits.truncate(spec.limit);
    }

    Ok(SearchReport {
        base: base.name.clone(),
        moved: spec.move_refs.clone(),
        step: spec.step,
        candidates_after_filter: candidates,
        fully_checked: checked.load(std::sync::atomic::Ordering::Relaxed),
        hits,
        elapsed_ms: t0.elapsed().as_millis(),
    })
}

/// The area a piece occupies for good: footprint plus the zone it demands itself. Only declarations
/// with a direction (`expands`, `opens` with `open_clear`); `access_sides` names no side and is left
/// to the final check.
fn lasting_area(model: &Model, reference: &str, rot: i32, r: Rect) -> Rect {
    let Some(it) = model.catalogue.get(reference) else {
        return r;
    };
    let mut out = r;
    let mut grow = |side: Side, extra: i32| {
        if extra <= 0 {
            return;
        }
        match side {
            Side::North => {
                out.y -= extra;
                out.d += extra;
            }
            Side::South => out.d += extra,
            Side::West => {
                out.x -= extra;
                out.w += extra;
            }
            Side::East => out.w += extra,
        }
    };
    if let (Some(dir), Some(to)) = (it.expands_dir, it.expands_to) {
        let side = dir.rotated(rot);
        let current = match side {
            Side::North | Side::South => r.d,
            Side::East | Side::West => r.w,
        };
        grow(side, to - current);
    }
    if let (Some(dir), Some(clear)) = (it.opens, it.open_clear) {
        grow(dir.rotated(rot), clear);
    }
    out
}

/// Where a piece's top-left corner may lie, as runs per grid row.
///
/// What the dashboard asks while dragging, so a piece cannot be pushed through a wall. Computed here
/// because the room is not a rectangle, and a second `point_in_polygon` in JavaScript would be the
/// duplication this capability exists against.
///
/// Hard are walls and what cannot move: fixtures and door swings, the same `blockers` the search
/// uses. Other pieces are not hard: rearranging means moving one past another, and an overlap is
/// reported anyway.
pub fn allowed_positions(
    model: &Model,
    base: &Layout,
    reference: &str,
    rot: i32,
) -> Result<AllowedPositions, ModelError> {
    let it = base
        .items
        .iter()
        .find(|i| i.reference == reference)
        .ok_or_else(|| {
            ModelError::Missing(format!("`{reference}` is not in \"{}\"", base.name))
        })?;
    // The footprint for the requested rotation, not the current one.
    let rotated = PlacedItem { rot, ..it.clone() };
    let (w, d, _) = footprint(&rotated, &model.catalogue)?;

    // What cannot move: fixtures and door swings, read from room.toml.
    let mut blocked: Vec<Rect> = model.room.fixed_furniture.iter().map(|f| f.rect()).collect();
    blocked.extend(
        model
            .room
            .openings
            .iter()
            .filter_map(|o| o.blocked_area.map(|z| z.rect())),
    );

    let poly = &model.room.main_room.polygon;
    let (mut min_x, mut min_y) = (i32::MAX, i32::MAX);
    let (mut max_x, mut max_y) = (i32::MIN, i32::MIN);
    for p in poly {
        min_x = min_x.min(p[0]);
        min_y = min_y.min(p[1]);
        max_x = max_x.max(p[0]);
        max_y = max_y.max(p[1]);
    }

    let mut rows = Vec::new();
    let mut y = min_y;
    while y + d <= max_y {
        let mut runs: Vec<[i32; 2]> = Vec::new();
        let mut run: Option<i32> = None;
        let mut x = min_x;
        while x + w <= max_x {
            let r = Rect { x, y, w, d };
            let clear = inside_room(model, &r) && !blocked.iter().any(|b| r.overlaps(b));
            match (clear, run) {
                (true, None) => run = Some(x),
                (false, Some(start)) => {
                    runs.push([start, x - RES]);
                    run = None;
                }
                _ => {}
            }
            x += RES;
        }
        if let Some(start) = run {
            runs.push([start, x - RES]);
        }
        if !runs.is_empty() {
            rows.push(AllowedRow { y, x: runs });
        }
        y += RES;
    }

    Ok(AllowedPositions {
        reference: reference.to_string(),
        rot,
        w,
        d,
        step: RES,
        rows,
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct AllowedRow {
    pub y: i32,
    /// Allowed x ranges, each [from, to] inclusive.
    pub x: Vec<[i32; 2]>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AllowedPositions {
    pub reference: String,
    pub rot: i32,
    pub w: i32,
    pub d: i32,
    pub step: i32,
    pub rows: Vec<AllowedRow>,
}

/// What `compose` should place, and how thoroughly.
#[derive(Debug, Clone)]
pub struct ComposeSpec {
    /// The pieces, in placing order; large first.
    pub refs: Vec<String>,
    /// Grid step in cm. Search coarse, refine by hand.
    pub step: i32,
    /// Partial arrangements that survive each round.
    pub beam: usize,
    /// Allowed rotations per piece.
    pub rotations: Vec<i32>,
    /// How many finished arrangements come back.
    pub limit: usize,
}

/// A fully placed flat and its verdict.
#[derive(Debug, Clone, Serialize)]
pub struct Composed {
    pub places: Vec<(String, [i32; 2], i32)>,
    pub pass: bool,
    pub hard: Vec<String>,
    pub soft: Vec<String>,
    pub wall_contact_cm: i32,
    pub bottleneck_cm: i32,
    pub free_m2: f64,
    /// The tightest hard measurement of this arrangement in cm: it passes by this much.
    pub tightest_reserve_cm: Option<i32>,
    /// On the Pareto front of the four objectives.
    pub pareto: bool,
}

/// Furnish a flat from scratch instead of moving pieces in an existing layout.
///
/// `search` collects the full cartesian product before checking; six pieces would be about 10^15
/// combinations. So this is a beam search: piece by piece, keeping the best `beam` partial
/// arrangements per round. The price, stated: it can drop a partial arrangement that the last piece
/// would have made good, so the optimum is not guaranteed. What it returns is computed and
/// reproducible.
///
/// The full check runs at the end: on half a flat every walkway is free, and a rule passing on a
/// partial arrangement says nothing about the whole.
pub fn compose(model: &Model, spec: &ComposeSpec) -> Result<Vec<Composed>, ModelError> {
    // The full hard zones, not only walls: a position in a door's approach zone is never valid, and
    // filtering it late filled the whole beam with candidates that could not pass.
    let blocked = crate::clearance::hard_zones(&model.room, &model.rules)?;

    // Allowed places per (piece, rotation), computed once against walls and fixtures. Collisions with
    // neighbours depend on the partial arrangement and drop out while extending.
    type Place = ([i32; 2], i32, Rect);
    let mut placements_by_ref: BTreeMap<String, Vec<Place>> = BTreeMap::new();
    for r in &spec.refs {
        let mut all = Vec::new();
        for rot in &spec.rotations {
            let probe = PlacedItem {
                reference: r.clone(),
                x: 0,
                y: 0,
                rot: *rot,
                size: None,
                kind: None,
            };
            let (w, d, _) = footprint(&probe, &model.catalogue)?;
            let poly = &model.room.main_room.polygon;
            let (min_x, max_x) = (
                poly.iter().map(|q| q[0]).min().unwrap_or(0),
                poly.iter().map(|q| q[0]).max().unwrap_or(0),
            );
            let (min_y, max_y) = (
                poly.iter().map(|q| q[1]).min().unwrap_or(0),
                poly.iter().map(|q| q[1]).max().unwrap_or(0),
            );
            let mut y = min_y;
            while y + d <= max_y {
                let mut x = min_x;
                while x + w <= max_x {
                    let rect = Rect { x, y, w, d };
                    // The place must hold the lasting area, not just the piece: a table's flap and a wardrobe door
                    // stay clear.
                    let occupied = lasting_area(model, r, *rot, rect);
                    if inside_room(model, &rect) && !blocked.iter().any(|b| occupied.overlaps(b)) {
                        all.push(([x, y], *rot, occupied));
                    }
                    x += spec.step;
                }
                y += spec.step;
            }
        }
        if all.is_empty() {
            return Err(ModelError::Missing(format!(
                "`{r}` has no place in the room at a {} cm step",
                spec.step
            )));
        }
        placements_by_ref.insert(r.clone(), all);
    }

    // Extend round by round, then cut to the best `beam`.
    let mut beam_set: Vec<Vec<Place>> = vec![Vec::new()];
    for r in &spec.refs {
        let options = &placements_by_ref[r];
        let mut nearest: Vec<(i32, Vec<Place>)> = beam_set
            .par_iter()
            .flat_map(|teil| {
                options
                    .par_iter()
                    .filter(|(_, _, rect)| !teil.iter().any(|(_, _, q)| rect.overlaps(q)))
                    .map(|p| {
                        let mut c = teil.clone();
                        c.push(*p);
                        // Wall behind, unless the piece says it stands free. Measured at the piece, not its reserved zone,
                        // or a table's flap would count as its back.
                        let score: i32 = c
                            .iter()
                            .zip(spec.refs.iter())
                            .filter(|(_, name)| !is_room_divider(model, name))
                            .filter_map(|((pos, rot, _), name)| {
                                let probe = PlacedItem {
                                    reference: name.clone(),
                                    x: pos[0],
                                    y: pos[1],
                                    rot: *rot,
                                    size: None,
                                    kind: None,
                                };
                                let (w, d, _) = footprint(&probe, &model.catalogue).ok()?;
                                Some(wall_contact_cm(
                                    model,
                                    &Rect {
                                        x: pos[0],
                                        y: pos[1],
                                        w,
                                        d,
                                    },
                                ))
                            })
                            .sum();
                        (score, c)
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        if nearest.is_empty() {
            return Err(ModelError::Missing(format!(
                "after `{r}` no arrangement is left: step too coarse or too many pieces"
            )));
        }
        // Variety over volume: per position of the piece just placed, only the best partial arrangement
        // survives, or the beam fills with variants of one idea.
        nearest.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
        let mut seen: std::collections::HashSet<[i32; 2]> = std::collections::HashSet::new();
        nearest.retain(|(_, c)| {
            c.last()
                .map(|(pos, _, _)| seen.insert(*pos))
                .unwrap_or(false)
        });
        nearest.truncate(spec.beam);
        beam_set = nearest.into_iter().map(|(_, c)| c).collect();
    }

    // Only now the full check, on whole flats.
    let mut done: Vec<Composed> = beam_set
        .par_iter()
        .filter_map(|c| {
            let items: Vec<PlacedItem> = c
                .iter()
                .zip(spec.refs.iter())
                .map(|((pos, rot, _), name)| PlacedItem {
                    reference: name.clone(),
                    x: pos[0],
                    y: pos[1],
                    rot: *rot,
                    size: None,
                    kind: None,
                })
                .collect();
            let l = Layout {
                name: "compose".into(),
                items,
                id: String::new(),
            };
            let r = crate::clearance::check_layout(model, &l).ok()?;
            Some(Composed {
                places: l
                    .items
                    .iter()
                    .map(|i| (i.reference.clone(), [i.x, i.y], i.rot))
                    .collect(),
                pass: r.pass,
                hard: r.hard.iter().map(|v| v.rule.clone()).collect(),
                soft: r.soft.iter().map(|v| v.rule.clone()).collect(),
                wall_contact_cm: l
                    .items
                    .iter()
                    .filter_map(|i| {
                        let (w, d, _) = footprint(i, &model.catalogue).ok()?;
                        Some(wall_contact_cm(
                            model,
                            &Rect {
                                x: i.x,
                                y: i.y,
                                w,
                                d,
                            },
                        ))
                    })
                    .sum(),
                bottleneck_cm: r
                    .metrics
                    .corridors
                    .iter()
                    .filter_map(|x| x.width_cm)
                    .min()
                    .unwrap_or(0),
                free_m2: r.metrics.free_area_m2,
                tightest_reserve_cm: r.tightest_reserve_cm,
                pareto: false,
            })
        })
        .collect();

    // The same order as `search`: passing, few warnings, wall behind, widest bottleneck; with the
    // bottleneck first an arrangement with every piece free in the room won. The front is built among
    // the passing only, or a failing one with lots of wall would lead a list whose first question is
    // whether one can move in.
    let targets: Vec<[i32; TARGETS]> = done
        .iter()
        .map(|c| {
            [
                c.tightest_reserve_cm.unwrap_or(i32::MIN),
                c.wall_contact_cm,
                c.bottleneck_cm,
                -(c.soft.len() as i32),
            ]
        })
        .collect();
    let front = pareto_front(&targets);
    for (c, f) in done.iter_mut().zip(front) {
        c.pareto = c.pass && f;
    }
    done.sort_by(|a, b| {
        b.pass
            .cmp(&a.pass)
            .then(b.pareto.cmp(&a.pareto))
            .then(a.soft.len().cmp(&b.soft.len()))
            .then(b.wall_contact_cm.cmp(&a.wall_contact_cm))
            .then(b.bottleneck_cm.cmp(&a.bottleneck_cm))
            .then(b.tightest_reserve_cm.cmp(&a.tightest_reserve_cm))
    });
    if spec.limit > 0 {
        done.truncate(spec.limit);
    }
    Ok(done)
}

#[cfg(test)]
mod pareto_tests {
    use super::{pareto_front, TARGETS};

    /// A point worse in everything does not belong on the front.
    #[test]
    fn a_point_worse_in_everything_drops_out() {
        let p: Vec<[i32; TARGETS]> = vec![[10, 10, 10, 0], [9, 9, 9, -1]];
        assert_eq!(pareto_front(&p), vec![true, false]);
    }

    /// A trade-off is no defeat: giving up one objective for another stays on the front.
    #[test]
    fn a_trade_off_keeps_both_on_the_front() {
        let p: Vec<[i32; TARGETS]> = vec![[10, 0, 5, 0], [0, 10, 5, 0]];
        assert_eq!(pareto_front(&p), vec![true, true]);
    }

    /// Equal points do not dominate each other, or two equally good arrangements in different places
    /// would become one.
    #[test]
    fn identical_points_both_stay() {
        let p: Vec<[i32; TARGETS]> = vec![[3, 3, 3, 3], [3, 3, 3, 3]];
        assert_eq!(pareto_front(&p), vec![true, true]);
    }

    /// Equal in three objectives and better in the fourth: dominance, not a tie.
    #[test]
    fn one_better_target_is_enough_to_dominate() {
        let p: Vec<[i32; TARGETS]> = vec![[5, 5, 5, 5], [5, 5, 5, 6]];
        assert_eq!(pareto_front(&p), vec![false, true]);
    }

    /// The front of an empty field is empty, not a crash.
    #[test]
    fn empty_stays_empty() {
        assert!(pareto_front(&[]).is_empty());
    }
}
