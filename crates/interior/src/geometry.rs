//! Grid, distance field, path search, all in centimetres.
//!
//! Two primitives do the work: an exact Euclidean distance transform, which gives every free point
//! its distance to the nearest occupied one, and a maximum-bottleneck search, which gives the
//! narrowest point of the widest path. A corridor of width W has clearance W/2 on its centre line,
//! so the doubled bottleneck is the width a person actually gets.

use crate::model::{Pt, Rect};
use std::collections::BinaryHeap;

/// Grid step in cm. The recorded route widths in the overlay's baseline hold for exactly this
/// value; changing it changes the numbers (tests/live_parity.rs).
pub const RES: i32 = 5;

pub fn polygon_area_m2(poly: &[Pt]) -> f64 {
    let mut a = 0i64;
    for i in 0..poly.len() {
        let j = (i + 1) % poly.len();
        a += poly[i][0] as i64 * poly[j][1] as i64 - poly[j][0] as i64 * poly[i][1] as i64;
    }
    (a.abs() as f64 / 2.0) / 10_000.0
}

pub fn point_in_polygon(p: [f64; 2], poly: &[Pt]) -> bool {
    let mut inside = false;
    let mut j = poly.len() - 1;
    for i in 0..poly.len() {
        let (xi, yi) = (poly[i][0] as f64, poly[i][1] as f64);
        let (xj, yj) = (poly[j][0] as f64, poly[j][1] as f64);
        if (yi > p[1]) != (yj > p[1]) && p[0] < (xj - xi) * (p[1] - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

pub struct Grid {
    pub cols: usize,
    pub rows: usize,
    /// 1 where a person can stand: inside the flat's polygon and not occupied.
    pub free: Vec<u8>,
}

impl Grid {
    pub fn new(poly: &[Pt]) -> Grid {
        let max_x = poly.iter().map(|p| p[0]).max().unwrap_or(0);
        let max_y = poly.iter().map(|p| p[1]).max().unwrap_or(0);
        let cols = (max_x as f64 / RES as f64).ceil() as usize;
        let rows = (max_y as f64 / RES as f64).ceil() as usize;
        let mut free = vec![0u8; cols * rows];
        for r in 0..rows {
            for c in 0..cols {
                let centre = [(c as f64 + 0.5) * RES as f64, (r as f64 + 0.5) * RES as f64];
                free[r * cols + c] = point_in_polygon(centre, poly) as u8;
            }
        }
        Grid { cols, rows, free }
    }

    #[inline]
    pub fn idx(&self, c: usize, r: usize) -> usize {
        r * self.cols + c
    }

    pub fn cell_of(&self, p: Pt) -> (usize, usize) {
        let c = (p[0] / RES).clamp(0, self.cols as i32 - 1) as usize;
        let r = (p[1] / RES).clamp(0, self.rows as i32 - 1) as usize;
        (c, r)
    }

    pub fn occupy(&mut self, rect: &Rect) {
        let c0 = (rect.x / RES).max(0) as usize;
        let c1 = (((rect.right() as f64) / RES as f64).ceil() as i32 - 1).min(self.cols as i32 - 1);
        let r0 = (rect.y / RES).max(0) as usize;
        let r1 =
            (((rect.bottom() as f64) / RES as f64).ceil() as i32 - 1).min(self.rows as i32 - 1);
        if c1 < 0 || r1 < 0 {
            return;
        }
        for r in r0..=(r1 as usize) {
            for c in c0..=(c1 as usize) {
                let i = self.idx(c, r);
                self.free[i] = 0;
            }
        }
    }

    pub fn free_cells(&self) -> usize {
        self.free.iter().filter(|v| **v == 1).count()
    }
}

/// Exact 1D distance transform (Felzenszwalb/Huttenlocher), the building block of the 2D one.
fn edt_1d(f: &[f64], out: &mut [f64]) {
    let n = f.len();
    let mut v = vec![0usize; n];
    let mut z = vec![0.0f64; n + 1];
    let mut k = 0usize;
    z[0] = f64::NEG_INFINITY;
    z[1] = f64::INFINITY;
    for q in 1..n {
        loop {
            let p = v[k];
            let s = ((f[q] + (q * q) as f64) - (f[p] + (p * p) as f64))
                / (2.0 * q as f64 - 2.0 * p as f64);
            if s <= z[k] {
                if k == 0 {
                    break;
                }
                k -= 1;
            } else {
                k += 1;
                v[k] = q;
                z[k] = s;
                z[k + 1] = f64::INFINITY;
                break;
            }
        }
        if k == 0 && z[1].is_infinite() && v[0] != q {
            // The backward pass reached the root: q replaces the current parabola.
            v[0] = q;
            z[0] = f64::NEG_INFINITY;
            z[1] = f64::INFINITY;
        }
    }
    let mut k = 0usize;
    for (q, cell) in out.iter_mut().enumerate().take(n) {
        while z[k + 1] < q as f64 {
            k += 1;
        }
        let p = v[k];
        let d = q as f64 - p as f64;
        *cell = d * d + f[p];
    }
}

/// Distance in cm from every free cell to the nearest occupied cell or wall. Exact: a chamfer
/// approximation shifts corridor widths by centimetres, and those centimetres decide pass or fail.
pub fn clearance_field(grid: &Grid) -> Vec<f64> {
    const INF: f64 = 1e12;
    let (cols, rows) = (grid.cols, grid.rows);
    let mut f: Vec<f64> = grid
        .free
        .iter()
        .map(|v| if *v == 1 { INF } else { 0.0 })
        .collect();

    let mut col = vec![0.0f64; rows];
    let mut dst = vec![0.0f64; rows];
    for c in 0..cols {
        for r in 0..rows {
            col[r] = f[r * cols + c];
        }
        edt_1d(&col, &mut dst);
        for r in 0..rows {
            f[r * cols + c] = dst[r];
        }
    }
    let mut row = vec![0.0f64; cols];
    let mut dst = vec![0.0f64; cols];
    for r in 0..rows {
        row.copy_from_slice(&f[r * cols..(r + 1) * cols]);
        edt_1d(&row, &mut dst);
        f[r * cols..(r + 1) * cols].copy_from_slice(&dst);
    }
    for v in f.iter_mut() {
        *v = v.sqrt() * RES as f64;
    }
    f
}

/// The nearest cell a person could actually stand in. Waypoints come from openings and so lie in
/// a wall. A 70 cm radius: smaller, and next to a wide door no open cell is found and the door frame
/// becomes every route's bottleneck.
pub fn nearest_free(grid: &Grid, field: Option<&[f64]>, p: Pt, radius_cm: i32) -> Option<usize> {
    let radius_cells = (radius_cm as f64 / RES as f64).ceil() as i32;
    let (c0, r0) = grid.cell_of(p);
    let mut best: Option<(f64, usize)> = None;
    for dr in -radius_cells..=radius_cells {
        for dc in -radius_cells..=radius_cells {
            let c = c0 as i32 + dc;
            let r = r0 as i32 + dr;
            if c < 0 || r < 0 || c >= grid.cols as i32 || r >= grid.rows as i32 {
                continue;
            }
            let i = grid.idx(c as usize, r as usize);
            if grid.free[i] == 0 {
                continue;
            }
            // The most open cell nearby, not the nearest: one stands in front of a door, not in its frame.
            let score = match field {
                Some(f) => f[i],
                None => -(((dc * dc + dr * dr) as f64).sqrt()),
            };
            if best.is_none_or(|(s, _)| score > s) {
                best = Some((score, i));
            }
        }
    }
    best.map(|(_, i)| i)
}

#[derive(PartialEq)]
struct HeapEntry(f64, usize);
impl Eq for HeapEntry {}
impl Ord for HeapEntry {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0
            .partial_cmp(&other.0)
            .unwrap_or(std::cmp::Ordering::Equal)
    }
}
impl PartialOrd for HeapEntry {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Narrowest point of the widest path in cm; higher is better. A max-heap: a linear scan per pop
/// was O(N²) per route, measured at 67.7 ms per layout check against 2.7 ms with the heap.
pub fn widest_path(grid: &Grid, field: &[f64], from: Pt, to: Pt) -> Option<f64> {
    let start = nearest_free(grid, Some(field), from, 70)?;
    let goal = nearest_free(grid, Some(field), to, 70)?;

    let n = grid.cols * grid.rows;
    let mut best = vec![0.0f64; n];
    let mut seen = vec![0u8; n];
    let mut heap: BinaryHeap<HeapEntry> = BinaryHeap::new();
    best[start] = field[start];
    heap.push(HeapEntry(field[start], start));

    while let Some(HeapEntry(cur_val, cur)) = heap.pop() {
        if seen[cur] == 1 {
            continue; // stale entry; BinaryHeap has no decrease-key
        }
        if cur_val <= 0.0 {
            return None;
        }
        if cur == goal {
            return Some(cur_val);
        }
        seen[cur] = 1;
        let c = (cur % grid.cols) as i32;
        let r = (cur / grid.cols) as i32;
        for (dc, dr) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let (nc, nr) = (c + dc, r + dr);
            if nc < 0 || nr < 0 || nc >= grid.cols as i32 || nr >= grid.rows as i32 {
                continue;
            }
            let ni = grid.idx(nc as usize, nr as usize);
            if seen[ni] == 1 || grid.free[ni] == 0 {
                continue;
            }
            let bottleneck = cur_val.min(field[ni]);
            if bottleneck > best[ni] {
                best[ni] = bottleneck;
                heap.push(HeapEntry(bottleneck, ni));
            }
        }
    }
    None
}

/// A person needs about 60 cm of length to stand in a strip.
pub const STANDING_RUN_CM: i32 = 60;

/// How far past a threshold is still measured, so a reserve has a number. Capping at the threshold
/// would report zero air for both the tightest and the roomiest arrangement. Where the cap hits,
/// the reserve says `capped`.
pub const RESERVE_HORIZON: i32 = 100;

/// Free depth on one side of a rect, capped at `want`. A contiguous run long enough to stand in
/// counts, not every cell: a desk at a bed's head blocks part of the long side without making the
/// bed unreachable.
pub fn free_depth_on_side(grid: &Grid, r: &Rect, side: Side, want: i32) -> i32 {
    let step = RES;
    let mut d = step;
    while d <= want {
        let strip = match side {
            Side::North => Rect {
                x: r.x,
                y: r.y - d,
                w: r.w,
                d: step,
            },
            Side::South => Rect {
                x: r.x,
                y: r.bottom() + d - step,
                w: r.w,
                d: step,
            },
            Side::West => Rect {
                x: r.x - d,
                y: r.y,
                w: step,
                d: r.d,
            },
            Side::East => Rect {
                x: r.right() + d - step,
                y: r.y,
                w: step,
                d: r.d,
            },
        };
        let along_x = matches!(side, Side::North | Side::South);
        if longest_free_run(grid, &strip, along_x) < STANDING_RUN_CM {
            return d - step;
        }
        d += step;
    }
    want
}

/// Longest uninterrupted run of standable cells along a strip, in cm.
pub fn longest_free_run(grid: &Grid, s: &Rect, along_x: bool) -> i32 {
    let floor_div = |v: i32| (v as f64 / RES as f64).floor() as i32;
    let ceil_div = |v: i32| (v as f64 / RES as f64).ceil() as i32;
    let c0 = floor_div(s.x);
    let c1 = ceil_div(s.right()) - 1;
    let r0 = floor_div(s.y);
    let r1 = ceil_div(s.bottom()) - 1;
    let (lo, hi) = if along_x { (c0, c1) } else { (r0, r1) };
    let mut best = 0;
    let mut run = 0;
    for i in lo..=hi {
        let (c, r) = if along_x { (i, r0) } else { (c0, i) };
        let ok = c >= 0
            && r >= 0
            && c < grid.cols as i32
            && r < grid.rows as i32
            && grid.free[grid.idx(c as usize, r as usize)] == 1;
        run = if ok { run + RES } else { 0 };
        if run > best {
            best = run;
        }
    }
    best
}

pub fn overlap_area(a: &Rect, b: &Rect) -> i32 {
    let w = (a.right().min(b.right()) - a.x.max(b.x)).max(0);
    let h = (a.bottom().min(b.bottom()) - a.y.max(b.y)).max(0);
    w * h
}

/// The signed distance of two rects in cm. Positive is the air gap, Euclidean, so measured across
/// corners. Negative is the penetration depth along the cheaper axis out. This is a zone rule's
/// reserve: `overlaps()` only answers yes or no.
pub fn rect_gap(a: &Rect, b: &Rect) -> i32 {
    let ux = (a.right().min(b.right()) - a.x.max(b.x)).max(0);
    let uy = (a.bottom().min(b.bottom()) - a.y.max(b.y)).max(0);
    if ux > 0 && uy > 0 {
        return -ux.min(uy);
    }
    let dx = (b.x - a.right()).max(a.x - b.right()).max(0);
    let dy = (b.y - a.bottom()).max(a.y - b.bottom()).max(0);
    (((dx * dx + dy * dy) as f64).sqrt()).round() as i32
}

#[cfg(test)]
mod gap_tests {
    use super::rect_gap;
    use crate::model::Rect;

    fn r(x: i32, y: i32, w: i32, d: i32) -> Rect {
        Rect { x, y, w, d }
    }

    #[test]
    fn apart_gives_the_air_gap() {
        assert_eq!(rect_gap(&r(0, 0, 100, 100), &r(130, 0, 50, 50)), 30);
    }

    /// Across a corner the gap is the diagonal, not the larger axis.
    #[test]
    fn across_a_corner_the_diagonal_counts() {
        assert_eq!(rect_gap(&r(0, 0, 100, 100), &r(130, 140, 50, 50)), 50);
    }

    /// Touching is zero reserve and no violation: zones are half-open like `overlaps`.
    #[test]
    fn touching_is_zero() {
        assert_eq!(rect_gap(&r(0, 0, 100, 100), &r(100, 0, 50, 50)), 0);
    }

    /// The penetration depth is the shortest way out, not the longest.
    #[test]
    fn overlap_is_negative_and_takes_the_cheaper_axis() {
        assert_eq!(rect_gap(&r(0, 0, 100, 100), &r(90, 50, 100, 100)), -10);
    }
}

/// How deep a rect lies inside the polygon, in cm; negative means it sticks out.
///
/// Measured at the same four corners `check_layout` tests, with the same 1 cm inset, so the reserve
/// never reports what the rule does not. Both share one limit: in a concave polygon all four
/// corners can be inside while an edge cuts the notch; change the rule and this together.
pub fn rect_in_polygon_cm(r: &Rect, poly: &[Pt]) -> i32 {
    let corners = [
        [r.x + 1, r.y + 1],
        [r.right() - 1, r.y + 1],
        [r.x + 1, r.bottom() - 1],
        [r.right() - 1, r.bottom() - 1],
    ];
    let mut worst = f64::MAX;
    for c in corners {
        let p = [c[0] as f64, c[1] as f64];
        let mut nearest = f64::MAX;
        for i in 0..poly.len() {
            let a = poly[i];
            let b = poly[(i + 1) % poly.len()];
            nearest = nearest.min(point_to_segment(p, a, b));
        }
        let inside = point_in_polygon(p, poly);
        let signed = if inside { nearest } else { -nearest };
        worst = worst.min(signed);
    }
    worst.round() as i32
}

/// Distance of a point to a segment.
fn point_to_segment(p: [f64; 2], a: Pt, b: Pt) -> f64 {
    let (ax, ay) = (a[0] as f64, a[1] as f64);
    let (bx, by) = (b[0] as f64, b[1] as f64);
    let (dx, dy) = (bx - ax, by - ay);
    let laenge2 = dx * dx + dy * dy;
    if laenge2 < 1e-9 {
        return ((p[0] - ax).powi(2) + (p[1] - ay).powi(2)).sqrt();
    }
    let t = (((p[0] - ax) * dx + (p[1] - ay) * dy) / laenge2).clamp(0.0, 1.0);
    ((p[0] - (ax + t * dx)).powi(2) + (p[1] - (ay + t * dy)).powi(2)).sqrt()
}

pub use crate::store::Side;

pub const SIDES: [Side; 4] = [Side::North, Side::South, Side::East, Side::West];

#[cfg(test)]
mod polygon_tests {
    use super::rect_in_polygon_cm;
    use crate::model::Rect;

    /// A 400 x 400 square, so every number can be checked in one's head.
    fn room() -> Vec<[i32; 2]> {
        vec![[0, 0], [400, 0], [400, 400], [0, 400]]
    }

    /// Centred: the gap to the nearest wall, minus the 1 cm inset the rule uses too.
    #[test]
    fn centred_is_the_gap_to_the_nearest_wall() {
        let r = Rect {
            x: 100,
            y: 100,
            w: 100,
            d: 100,
        };
        assert_eq!(rect_in_polygon_cm(&r, &room()), 101);
    }

    /// Against the wall the reserve is zero, not a violation.
    #[test]
    fn against_the_wall_the_reserve_is_zero() {
        let r = Rect {
            x: 0,
            y: 0,
            w: 100,
            d: 100,
        };
        assert_eq!(rect_in_polygon_cm(&r, &room()), 1);
    }

    /// Sticking out gives a negative number naming the depth.
    #[test]
    fn outside_is_negative_and_names_the_depth() {
        let r = Rect {
            x: 350,
            y: 100,
            w: 100,
            d: 100,
        };
        assert_eq!(rect_in_polygon_cm(&r, &room()), -49);
    }
}
