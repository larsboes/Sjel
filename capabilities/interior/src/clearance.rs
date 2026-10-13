//! Judges a layout against `rules.toml`.
//!
//! Every threshold comes from the model; this file decides only whether a layout has the space,
//! never how much space is needed. Severity follows `rules.toml`: `hard` blocks, `soft` warns, and a
//! layout passes only without an open hard violation.
//!
//! Two kinds of ids. `RULE_IDS` are house rules: the flat declares them in `rules.toml` with
//! severity and text, looked up through `rules.rule(id)`; a missing declaration is an error. The
//! others (`collision`, `room_boundary`, `access`, `walkway`, …) are geometry and usability
//! invariants no flat can relax, so they are not declared.
//!
//! One function per rule. [`check_layout`] builds the `Check` (room, rules, placed pieces, occupied
//! grid) once and calls each rule with it; all write into the same `Findings`. No rule can change
//! another's basis, since it only borrows it, and none reads what another wrote. The report order
//! is the call order.

use crate::geometry::{
    clearance_field, free_depth_on_side, overlap_area, point_in_polygon, rect_gap,
    rect_in_polygon_cm, widest_path, Grid, Side, RES, SIDES,
};
use crate::model::{
    footprint, Catalogue, Layout, Model, ModelError, Opening, PlacedItem, Pt, Rect, Room, Rules,
};
use crate::store::Item;
use serde::Serialize;

pub use crate::model::Severity;

/// The house rules this checker checks. The flat must declare each, or the check stops; rules the
/// flat declares and this list lacks are reported as unchecked.
pub const RULE_IDS: &[&str] = &["R1", "R2", "R3", "R4", "R5", "R6", "R7", "R8"];

/// House rules that run only when the flat has them. R1 to R8 apply to every flat; R9 needs a
/// location, a north direction and glass heights. A flat that has R9 but lacks those data gets it
/// listed as unchecked, with the reason.
pub const OPTIONAL_RULE_IDS: &[&str] = &["R9"];

#[derive(Debug, Clone, Serialize)]
pub struct Violation {
    pub rule: String,
    pub severity: Severity,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item: Option<String>,
    pub message: String,
    /// The rule text from `rules.toml`, where there is one: `message` says what this layout does wrong,
    /// `text` why that is a rule. Only house rules have one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub measured: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required: Option<i32>,
}

/// A violation of a declared house rule; severity and text come from the flat.
fn house_rule(
    rules: &Rules,
    id: &str,
    item: Option<String>,
    message: String,
    measured: Option<i32>,
    required: Option<i32>,
) -> Result<Violation, ModelError> {
    let declared_rule = rules.rule(id)?;
    Ok(Violation {
        rule: declared_rule.id.clone(),
        severity: declared_rule.level()?,
        item,
        message,
        text: Some(declared_rule.text.clone()),
        measured,
        required,
    })
}

/// Every area a hard rule keeps free: an opening's approach zone (R1), a door swing (R2), a fixture
/// and the zone it demands (R7). Here rather than in `search.rs`, because these are the rects
/// `check_layout` checks.
pub fn hard_zones(room: &Room, rules: &Rules) -> Result<Vec<Rect>, ModelError> {
    let mut out: Vec<Rect> = room.fixed_furniture.iter().map(|f| f.rect()).collect();
    for o in &room.openings {
        if let Some(sp) = o.blocked_area {
            out.push(sp.rect());
        }
        if let (Some(depth), Some(seg)) = (o.keep_clear_zone, opening_segment(room, o)) {
            out.push(approach_rect(&seg, depth));
        }
    }
    for f in &room.fixed_furniture {
        if let Some(z) = &f.approach_zone {
            out.push(zone_rect(&f.rect(), z.side, rules.threshold(&z.threshold)?));
        }
    }
    Ok(out)
}

/// Rule ids the checker emits, in their earlier German spelling. Recorded results (the
/// overlay's `ts-baseline.json`) still use the old ones.
pub const LEGACY_RULE_IDS: [(&str, &str); 11] = [
    ("ausklappen", "unfold"),
    ("bett_zugang", "bed_access"),
    ("couchtisch_abstand", "coffee_table_gap"),
    ("kollision", "collision"),
    ("laufweg", "walkway"),
    ("raumgrenze", "room_boundary"),
    ("stuhl_ausziehen", "chair_pull_out"),
    ("stuhlzone", "chair_zone"),
    ("zugang", "access"),
    ("oeffnen", "open"),
    ("schrank_tuer", "wardrobe_door"),
];

/// Which way a threshold works: R3 limits a height upwards, every gap rule a depth downwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Limit {
    /// The measured value must be at least the required one.
    AtLeast,
    /// The measured value may be at most the required one.
    AtMost,
}

/// How much air a check leaves, also when it passes. A verdict is one bit, and 2 cm and 40 cm both
/// pass; only one survives a tape measure off by a centimetre. Every comparison against a threshold
/// records one of these, triggered or not.
#[derive(Debug, Clone, Serialize)]
pub struct Reserve {
    pub rule: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item: Option<String>,
    /// What was compared against, where there is something: the opening, fixture or route.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    pub measured: i32,
    pub required: i32,
    pub limit: Limit,
    /// What is left. Negative means missed by this much, the number the violation states in words.
    pub slack: i32,
    /// `cm`, `sides` or `seats`. Only `cm` enters `tightest_reserve_cm`: a minimum across units means
    /// nothing.
    pub unit: &'static str,
    /// The measurement reached its horizon: at least this much, not exactly. Without the flag the
    /// dashboard would show a lower bound as a measurement.
    pub capped: bool,
    /// Would falling short be hard? For house rules, the flat decides.
    pub hard: bool,
    /// Does a positive value say how much air is left? For nearly every rule, yes. For the room
    /// boundary, no: a wardrobe stands 1 cm from its wall because it belongs there, and counting that
    /// would report "1 cm reserve" for every sensibly furnished flat. A negative value always counts.
    pub binding: bool,
}

/// Records a measurement, triggered or not.
#[allow(clippy::too_many_arguments)]
fn remember(
    out: &mut Vec<Reserve>,
    rule: &str,
    item: Option<&str>,
    subject: Option<&str>,
    measured: i32,
    required: i32,
    limit: Limit,
    unit: &'static str,
    is_hard: bool,
) {
    remember_up_to(
        out, rule, item, subject, measured, required, limit, unit, is_hard, None, true,
    )
}

/// A measurement whose positive value is no reserve; see `Reserve::binding`.
#[allow(clippy::too_many_arguments)]
fn remember_only_violation(
    out: &mut Vec<Reserve>,
    rule: &str,
    item: Option<&str>,
    measured: i32,
    unit: &'static str,
) {
    remember_up_to(
        out,
        rule,
        item,
        None,
        measured,
        0,
        Limit::AtLeast,
        unit,
        true,
        None,
        false,
    )
}

/// Like `remember`, but the measurement has a horizon and says when it reached it.
#[allow(clippy::too_many_arguments)]
fn remember_up_to(
    out: &mut Vec<Reserve>,
    rule: &str,
    item: Option<&str>,
    subject: Option<&str>,
    measured: i32,
    required: i32,
    limit: Limit,
    unit: &'static str,
    is_hard: bool,
    cap: Option<i32>,
    binding: bool,
) {
    let slack = match limit {
        Limit::AtLeast => measured - required,
        Limit::AtMost => required - measured,
    };
    out.push(Reserve {
        rule: rule.to_string(),
        item: item.map(str::to_string),
        subject: subject.map(str::to_string),
        measured,
        required,
        limit,
        slack,
        unit,
        hard: is_hard,
        capped: cap == Some(measured),
        binding,
    });
}

/// How far a side depth is measured when its reserve should carry a number.
fn horizon(want: i32) -> i32 {
    want + crate::geometry::RESERVE_HORIZON
}

/// Whether a house rule is hard, from the flat.
fn ist_hart(rules: &Rules, id: &str) -> Result<bool, ModelError> {
    Ok(rules.rule(id)?.level()? == Severity::Hard)
}

/// Puts a violation into `hard` or `soft` by its own severity, so no call site can sort it.
fn sort_in(v: Violation, hard: &mut Vec<Violation>, soft: &mut Vec<Violation>) {
    match v.severity {
        Severity::Hard => hard.push(v),
        Severity::Soft => soft.push(v),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Corridor {
    pub from: String,
    pub to: String,
    pub width_cm: Option<i32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Uncertainty {
    pub reference: String,
    pub label: String,
    pub fields: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Metrics {
    pub room_area_m2: f64,
    pub occupied_area_m2: f64,
    pub free_area_m2: f64,
    pub corridors: Vec<Corridor>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CheckResult {
    pub layout: String,
    pub pass: bool,
    pub hard: Vec<Violation>,
    pub soft: Vec<Violation>,
    pub uncertainties: Vec<Uncertainty>,
    pub metrics: Metrics,
    /// Pieces in this layout already decided against (`priority = "rejected"` on the item). A verdict
    /// about pieces that will not come answers nothing.
    pub outdated: Vec<String>,
    /// Rules the flat declares and this checker does not check. In the result, not a log, because it
    /// qualifies the verdict: "passes" means passes the rules that were measured.
    pub unchecked: Vec<UncheckedRule>,
    /// Every measurement with its threshold, passed or not.
    pub reserves: Vec<Reserve>,
    /// The tightest hard measurement in cm: the margin this layout passes by. `None` when no hard rule
    /// measured in cm (not 0, which would mean just passing); negative while a hard violation is open.
    pub tightest_reserve_cm: Option<i32>,
}

/// A rule that was not applied to this layout, and why: not implemented, or not applicable because
/// a needed fact is missing. The second looks like passing, which is why it is reported.
#[derive(Debug, Clone, Serialize)]
pub struct UncheckedRule {
    pub rule: String,
    pub text: String,
    /// What is missing, in words, so the gap is named in the report.
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Bed,
    Desk,
    Couch,
    Wardrobe,
    CoffeeTable,
    Table,
    Shelf,
    Other,
}

/// Two orders matter here: `couch` must not catch `couchtisch`, and `coffee_table` comes before
/// `table`, because a coffee table wants 40 cm to the sofa, not 80 cm to pull out chairs.
pub fn kind_of(p: &PlacedItem) -> Kind {
    if let Some(k) = p.kind.as_deref() {
        return match k {
            "bed" => Kind::Bed,
            "desk" => Kind::Desk,
            "couch" => Kind::Couch,
            "wardrobe" => Kind::Wardrobe,
            "coffee_table" => Kind::CoffeeTable,
            "table" => Kind::Table,
            "shelf" => Kind::Shelf,
            _ => Kind::Other,
        };
    }
    kind_of_name(&p.reference)
}

/// The same heuristic by name only, for items not placed anywhere yet; `declarations.rs` uses it.
pub fn kind_of_name(reference: &str) -> Kind {
    let r = reference;
    if r.starts_with("bett") {
        Kind::Bed
    } else if r.starts_with("schreibtisch") || r.starts_with("buerostuhl") {
        Kind::Desk
    } else if r.starts_with("couchtisch") || r.starts_with("beistelltisch") {
        Kind::CoffeeTable
    } else if r.starts_with("couch") || r.starts_with("sofa") {
        Kind::Couch
    } else if r.starts_with("kleiderschrank") || r.starts_with("wardrobe") {
        Kind::Wardrobe
    } else if r.starts_with("esstisch") || r.starts_with("klapptisch") {
        Kind::Table
    } else if r.starts_with("kallax")
        || r.contains("regal")
        || r.starts_with("lowboard")
        || r.starts_with("raumtrenner")
    {
        Kind::Shelf
    } else {
        Kind::Other
    }
}

struct Placed<'a> {
    it: &'a PlacedItem,
    kind: Kind,
    /// The catalogue entry, if any. A layout may name a piece that brings its size via `size:` and is
    /// missing from the inventory; then the name decides.
    item: Option<&'a Item>,
    w: i32,
    d: i32,
    h: Option<i32>,
    rect: Rect,
}

/// An opening as a segment in room coordinates, with the normal pointing inwards. Public because
/// `delivery.rs` asks the same question.
pub struct Segment {
    pub a: Pt,
    pub b: Pt,
    pub normal: [f64; 2],
}

/// Resolves an opening into a segment with its inward normal. The normal is probed, not assumed:
/// some walls run against the polygon's edge direction.
pub fn opening_segment(room: &Room, o: &Opening) -> Option<Segment> {
    let (a, b) = room.opening_span(o)?;
    let len = (((b[0] - a[0]).pow(2) + (b[1] - a[1]).pow(2)) as f64)
        .sqrt()
        .max(1.0);
    let ux = (b[0] - a[0]) as f64 / len;
    let uy = (b[1] - a[1]) as f64 / len;
    let mid = [(a[0] + b[0]) as f64 / 2.0, (a[1] + b[1]) as f64 / 2.0];
    for n in [[-uy, ux], [uy, -ux]] {
        let probe = [mid[0] + n[0] * 20.0, mid[1] + n[1] * 20.0];
        if point_in_polygon(probe, &room.main_room.polygon) {
            return Some(Segment { a, b, normal: n });
        }
    }
    Some(Segment {
        a,
        b,
        normal: [0.0, 0.0],
    })
}


/// What a piece demands, when it says so itself. Returns `false` when it declares nothing, and the
/// caller falls back to `kind_of`.
///
/// * `open_clear`: space in front of doors and drawers. `wall_ok = false` binds it to the side
///   `opens` names; otherwise the best side will do, since a piece can be rotated.
/// * `access_sides` / `access_clear`: how many sides must be walkable, and how deep.
/// * `expands`: the unfolded state; `to` is the total depth, so the extra space is `to` minus the
///   depth in that direction.
fn own_needs(
    p: &Placed,
    grid: &Grid,
    hard: &mut Vec<Violation>,
    reserves: &mut Vec<Reserve>,
) -> bool {
    let Some(it) = p.item else { return false };
    let declared =
        it.open_clear.is_some() || it.access_sides.is_some() || it.expands_to.is_some();
    if !declared {
        return false;
    }
    let name = p.it.reference.as_str();

    if let Some(want) = it.open_clear {
        let side = it.opens.map(|s| s.rotated(p.it.rot));
        let binding = it.wall_ok == Some(false);
        let cap = horizon(want);
        let (got, place) = match (side, binding) {
            (Some(s), true) => (
                free_depth_on_side(grid, &p.rect, s, cap),
                s.as_str().to_string(),
            ),
            _ => (
                SIDES
                    .iter()
                    .map(|s| free_depth_on_side(grid, &p.rect, *s, cap))
                    .max()
                    .unwrap_or(0),
                "its best side".to_string(),
            ),
        };
        remember_up_to(
            reserves,
            "open",
            Some(name),
            Some(&place),
            got,
            want,
            Limit::AtLeast,
            "cm",
            true,
            Some(cap),
            true,
        );
        if got < want {
            hard.push(Violation {
                rule: "open".into(),
                severity: Severity::Hard,
                item: Some(name.into()),
                message: format!(
                    "\"{name}\" has {got} cm to open at {place}, needs {want}"
                ),
                text: None,
                measured: Some(got),
                required: Some(want),
            });
        }
    }

    if let Some(n) = it.access_sides {
        let want = it.access_clear.unwrap_or(0);
        let enough = SIDES
            .iter()
            .filter(|s| free_depth_on_side(grid, &p.rect, **s, want) >= want)
            .count() as i32;
        remember(
            reserves,
            "access",
            Some(name),
            None,
            enough,
            n,
            Limit::AtLeast,
            "sides",
            true,
        );
        if enough < n {
            hard.push(Violation {
                rule: "access".into(),
                severity: Severity::Hard,
                item: Some(name.into()),
                message: format!(
                    "\"{name}\" is reachable from {enough} sides with {want} cm, needs {n}"
                ),
                text: None,
                measured: Some(enough),
                required: Some(n),
            });
        }
    }

    if let (Some(dir), Some(to)) = (it.expands_dir, it.expands_to) {
        let side = dir.rotated(p.it.rot);
        let side_depth = match side {
            Side::North | Side::South => p.d,
            Side::East | Side::West => p.w,
        };
        let extra = to - side_depth;
        if extra > 0 {
            let cap = horizon(extra);
            let free_cm = free_depth_on_side(grid, &p.rect, side, cap);
            remember_up_to(
                reserves,
                "unfold",
                Some(name),
                Some(side.as_str()),
                free_cm,
                extra,
                Limit::AtLeast,
                "cm",
                true,
                Some(cap),
                true,
            );
            if free_cm < extra {
                hard.push(Violation {
                    rule: "unfold".into(),
                    severity: Severity::Hard,
                    item: Some(name.into()),
                    message: format!(
                        "\"{name}\" has {free_cm} cm to the {} to unfold, needs {extra} ({to} deep unfolded)",
                        side.as_str()
                    ),
                    text: None,
                    measured: Some(free_cm),
                    required: Some(extra),
                });
            }
        }
    }
    true
}

/// How far a waypoint lies in front of what it belongs to: one stands before a door, not in its
/// frame. One number for openings and fixtures alike.
const WAYPOINT_GAP: i32 = 45;

/// The area that must stay free beside a fixture, on the side it names.
fn zone_rect(r: &Rect, side: Side, side_depth: i32) -> Rect {
    match side {
        Side::North => Rect {
            x: r.x,
            y: r.y - side_depth,
            w: r.w,
            d: side_depth,
        },
        Side::South => Rect {
            x: r.x,
            y: r.bottom(),
            w: r.w,
            d: side_depth,
        },
        Side::West => Rect {
            x: r.x - side_depth,
            y: r.y,
            w: side_depth,
            d: r.d,
        },
        Side::East => Rect {
            x: r.right(),
            y: r.y,
            w: side_depth,
            d: r.d,
        },
    }
}

/// Where a person stands to use this fixture.
fn approach_point(r: &Rect, side: Side) -> Pt {
    match side {
        Side::North => [r.x + r.w / 2, r.y - WAYPOINT_GAP],
        Side::South => [r.x + r.w / 2, r.bottom() + WAYPOINT_GAP],
        Side::West => [r.x - WAYPOINT_GAP, r.y + r.d / 2],
        Side::East => [r.right() + WAYPOINT_GAP, r.y + r.d / 2],
    }
}

/// The rect a person needs in front of an opening, from its own keep-clear zone.
fn approach_rect(seg: &Segment, depth: i32) -> Rect {
    let d = depth as f64;
    let xs = [
        seg.a[0] as f64,
        seg.b[0] as f64,
        seg.a[0] as f64 + seg.normal[0] * d,
        seg.b[0] as f64 + seg.normal[0] * d,
    ];
    let ys = [
        seg.a[1] as f64,
        seg.b[1] as f64,
        seg.a[1] as f64 + seg.normal[1] * d,
        seg.b[1] as f64 + seg.normal[1] * d,
    ];
    let x = xs.iter().cloned().fold(f64::INFINITY, f64::min);
    let y = ys.iter().cloned().fold(f64::INFINITY, f64::min);
    let x1 = xs.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let y1 = ys.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    Rect {
        x: x as i32,
        y: y as i32,
        w: (x1 - x) as i32,
        d: (y1 - y) as i32,
    }
}

/// What every rule gets to read: room, rules, placed pieces with their footprints, and the grid.
/// Built once, then only read.
struct Check<'a> {
    model: &'a Model,
    room: &'a Room,
    rules: &'a Rules,
    placed: Vec<Placed<'a>>,
    /// The room with everything occupied in it, fixtures and placed pieces, but without door swings;
    /// see [`Check::new`].
    grid: Grid,
}

/// What every rule may write, and nothing else: violations in two severities, every measurement as
/// a [`Reserve`], and the reason when a rule could not run.
#[derive(Default)]
struct Findings {
    hard: Vec<Violation>,
    soft: Vec<Violation>,
    /// Every measurement, not only the failed ones.
    reserves: Vec<Reserve>,
    /// Rules that wanted to run and could not; reported together with unimplemented ones.
    fell_out: Vec<(String, String)>,
}

impl<'a> Check<'a> {
    /// Places the layout and occupies the grid; the only step that builds anything.
    fn new(model: &'a Model, layout: &'a Layout) -> Result<Self, ModelError> {
        let room = &model.room;
        let cat: &Catalogue = &model.catalogue;

        let mut placed: Vec<Placed> = Vec::new();
        for it in &layout.items {
            let (w, d, h) = footprint(it, cat)?;
            placed.push(Placed {
                it,
                kind: kind_of(it),
                item: cat.get(&it.reference),
                w,
                d,
                h,
                rect: Rect {
                    x: it.x,
                    y: it.y,
                    w,
                    d,
                },
            });
        }

        let mut grid = Grid::new(&room.main_room.polygon);
        for f in &room.fixed_furniture {
            grid.occupy(&Rect {
                x: f.x[0],
                y: f.y[0],
                w: f.x[1] - f.x[0],
                d: f.y[1] - f.y[0],
            });
        }
        // Door swings are deliberately not blocked here: they limit where furniture may stand, not where a
        // person walks. Blocked, the swing arc would become every route's bottleneck.
        // `r2_door_swing` checks them against the furniture footprints directly.
        for p in &placed {
            grid.occupy(&p.rect);
        }

        Ok(Self {
            model,
            room,
            rules: &model.rules,
            placed,
            grid,
        })
    }
}

/// Judges a layout against `rules.toml`. Builds the `Check`, lets every rule write into the same
/// `Findings`, and hands both to `finish`. The order here is the order in the report.
pub fn check_layout(
    model: &Model,
    layout: &Layout,
) -> Result<CheckResult, crate::model::ModelError> {
    let check = Check::new(model, layout)?;
    let mut out = Findings::default();

    room_boundary_and_collision(&check, &mut out);
    r1_approach_zones(&check, &mut out)?;
    r2_door_swing(&check, &mut out)?;
    r7_fixture_approach_zones(&check, &mut out)?;
    r3_light_corridor(&check, &mut out)?;
    r4_bed_head(&check, &mut out)?;
    r5_side_light(&check, &mut out)?;
    r6_view_from_entrance(&check, &mut out)?;
    r9_lasting_sun(&check, &mut out)?;
    access_gaps(&check, &mut out)?;
    let corridors = walkways(&check, &mut out)?;

    Ok(finish(&check, layout, out, corridors))
}

/// Room boundary and mutual overlap.
///
/// The tightest piece at a wall becomes one reserve, so a layout that fails here cannot also report
/// a comfortable margin from rules that passed.
fn room_boundary_and_collision(ctx: &Check, out: &mut Findings) {
    let room = ctx.room;
    let placed = &ctx.placed;

    if let Some(tightest) = placed
        .iter()
        .min_by_key(|p| rect_in_polygon_cm(&p.rect, &room.main_room.polygon))
    {
        remember_only_violation(
            &mut out.reserves,
            "room_boundary",
            Some(&tightest.it.reference),
            rect_in_polygon_cm(&tightest.rect, &room.main_room.polygon),
            "cm",
        );
    }
    for p in placed {
        let corners = [
            [p.rect.x + 1, p.rect.y + 1],
            [p.rect.right() - 1, p.rect.y + 1],
            [p.rect.x + 1, p.rect.bottom() - 1],
            [p.rect.right() - 1, p.rect.bottom() - 1],
        ];
        if !corners
            .iter()
            .all(|c| point_in_polygon([c[0] as f64, c[1] as f64], &room.main_room.polygon))
        {
            out.hard.push(Violation {
                rule: "room_boundary".into(),
                severity: Severity::Hard,
                item: Some(p.it.reference.clone()),
                message: format!(
                    "\"{}\" sticks out of the room at {},{} ({}×{} cm)",
                    p.it.reference, p.it.x, p.it.y, p.w, p.d
                ),
                text: None,
                measured: None,
                required: None,
            });
        }
    }
    // The tightest gap between two pieces as one reserve rather than n²: what matters is where it
    // tears first.
    let mut tightest_pair: Option<(i32, String)> = None;
    for i in 0..placed.len() {
        for j in (i + 1)..placed.len() {
            let (a, b) = (&placed[i], &placed[j]);
            let gap = rect_gap(&a.rect, &b.rect);
            if tightest_pair.as_ref().is_none_or(|(g, _)| gap < *g) {
                tightest_pair = Some((gap, format!("{} / {}", a.it.reference, b.it.reference)));
            }
            if a.rect.overlaps(&b.rect) {
                out.hard.push(Violation {
                    rule: "collision".into(),
                    severity: Severity::Hard,
                    item: Some(format!("{} / {}", a.it.reference, b.it.reference)),
                    message: format!(
                        "\"{}\" and \"{}\" overlap by {} dm²",
                        a.it.reference,
                        b.it.reference,
                        (overlap_area(&a.rect, &b.rect) as f64 / 100.0).round() as i32
                    ),
                    text: None,
                    measured: None,
                    required: None,
                });
            }
        }
    }

    if let Some((gap, paar)) = tightest_pair {
        remember(
            &mut out.reserves,
            "collision",
            None,
            Some(&paar),
            gap,
            0,
            Limit::AtLeast,
            "cm",
            true,
        );
    }
}

/// R1: an opening's approach zone stays free. R2 and R7 ask the same of other areas.
fn r1_approach_zones(ctx: &Check, out: &mut Findings) -> Result<(), ModelError> {
    let room = ctx.room;
    let rules = ctx.rules;
    let placed = &ctx.placed;

    for o in &room.openings {
        let Some(depth) = o.keep_clear_zone else {
            continue;
        };
        let Some(seg) = opening_segment(room, o) else {
            continue;
        };
        let zone = approach_rect(&seg, depth);
        // The nearest piece to this zone, touching it or not.
        if let Some(next) = placed.iter().min_by_key(|p| rect_gap(&p.rect, &zone)) {
            remember(
                &mut out.reserves,
                "R1",
                Some(&next.it.reference),
                Some(&o.id),
                rect_gap(&next.rect, &zone),
                0,
                Limit::AtLeast,
                "cm",
                ist_hart(rules, "R1")?,
            );
        }
        for p in placed {
            if p.rect.overlaps(&zone) {
                sort_in(
                    house_rule(
                        rules,
                        "R1",
                        Some(p.it.reference.clone()),
                        format!(
                            "\"{}\" reaches into the {} cm approach zone of {}",
                            p.it.reference, depth, o.id
                        ),
                        None,
                        Some(depth),
                    )?,
                    &mut out.hard,
                    &mut out.soft,
                );
            }
        }
    }

    Ok(())
}

/// R2: no furniture in a door's swing, checked against the footprints directly (see
/// [`Check::new`]).
fn r2_door_swing(ctx: &Check, out: &mut Findings) -> Result<(), ModelError> {
    let room = ctx.room;
    let rules = ctx.rules;
    let placed = &ctx.placed;

    for o in &room.openings {
        let Some(sp) = o.blocked_area else { continue };
        let zone = sp.rect();
        if let Some(next) = placed.iter().min_by_key(|p| rect_gap(&p.rect, &zone)) {
            remember(
                &mut out.reserves,
                "R2",
                Some(&next.it.reference),
                Some(&o.id),
                rect_gap(&next.rect, &zone),
                0,
                Limit::AtLeast,
                "cm",
                ist_hart(rules, "R2")?,
            );
        }
        for p in placed {
            if p.rect.overlaps(&zone) {
                sort_in(
                    house_rule(
                        rules,
                        "R2",
                        Some(p.it.reference.clone()),
                        format!(
                            "\"{}\" stands in the door swing of {}",
                            p.it.reference, o.id
                        ),
                        None,
                        None,
                    )?,
                    &mut out.hard,
                    &mut out.soft,
                );
            }
        }
    }

    Ok(())
}

/// R7: a fixture's approach zone stays free, for every fixture that declares one; the threshold
/// is looked up by name in rules.toml.
fn r7_fixture_approach_zones(ctx: &Check, out: &mut Findings) -> Result<(), ModelError> {
    let room = ctx.room;
    let rules = ctx.rules;
    let placed = &ctx.placed;

    for f in &room.fixed_furniture {
        let Some(zone_decl) = &f.approach_zone else {
            continue;
        };
        let depth = rules.threshold(&zone_decl.threshold)?;
        let zone = zone_rect(&f.rect(), zone_decl.side, depth);
        if let Some(next) = placed.iter().min_by_key(|p| rect_gap(&p.rect, &zone)) {
            remember(
                &mut out.reserves,
                "R7",
                Some(&next.it.reference),
                Some(&f.id),
                rect_gap(&next.rect, &zone),
                0,
                Limit::AtLeast,
                "cm",
                ist_hart(rules, "R7")?,
            );
        }
        for p in placed {
            if p.rect.overlaps(&zone) {
                sort_in(
                    house_rule(
                        rules,
                        "R7",
                        Some(p.it.reference.clone()),
                        format!(
                            "\"{}\" blocks the {} cm approach zone of {}",
                            p.it.reference, depth, f.id
                        ),
                        None,
                        Some(depth),
                    )?,
                    &mut out.hard,
                    &mut out.soft,
                );
            }
        }
    }

    Ok(())
}

/// R3: the light corridor stays low. `rules.toml` holds `[lichtkorridor]` structured, so the rule
/// cannot drift from its description.
fn r3_light_corridor(ctx: &Check, out: &mut Findings) -> Result<(), ModelError> {
    let rules = ctx.rules;
    let placed = &ctx.placed;

    let lc = &rules.light_corridor;
    let corridor = Rect {
        x: lc.x[0],
        y: lc.y[0],
        w: lc.x[1] - lc.x[0],
        d: lc.y[1] - lc.y[0],
    };
    for p in placed {
        // Without a measured height R3 cannot judge, but only pieces actually in the corridor are
        // reported for it.
        if p.h.is_none() && p.rect.overlaps(&corridor) {
            out.fell_out.push((
                "R3".to_string(),
                format!(
                    "\"{}\" stands in the light corridor and has no measured height",
                    p.it.reference
                ),
            ));
        }
        if let (Some(h), true) = (p.h, p.rect.overlaps(&corridor)) {
            // The only threshold limiting upwards; without `Limit` its reserve would carry the wrong sign.
            remember(
                &mut out.reserves,
                "R3",
                Some(&p.it.reference),
                Some("lichtkorridor"),
                h,
                lc.max_height,
                Limit::AtMost,
                "cm",
                ist_hart(rules, "R3")?,
            );
        }
        if let Some(h) = p.h {
            if h > lc.max_height && p.rect.overlaps(&corridor) {
                sort_in(
                    house_rule(
                        rules,
                        "R3",
                        Some(p.it.reference.clone()),
                        format!(
                            "\"{}\" is {} cm high in the light corridor (max {}); this room has a window wall",
                            p.it.reference, h, lc.max_height
                        ),
                        Some(h),
                        Some(lc.max_height),
                    )?,
                    &mut out.hard,
                    &mut out.soft,
                );
            }
        }
    }

    Ok(())
}

/// R4: bed head away from the glazing.
fn r4_bed_head(ctx: &Check, out: &mut Findings) -> Result<(), ModelError> {
    let room = ctx.room;
    let rules = ctx.rules;
    let placed = &ctx.placed;

    if let Some(glaz) = room
        .openings
        .iter()
        .find(|o| o.kind.as_deref().is_some_and(|t| t.starts_with("glass_door")))
    {
        if let Some(seg) = opening_segment(room, glaz) {
            let band = approach_rect(&seg, 20);
            if let Some(next) = placed
                .iter()
                .filter(|p| p.kind == Kind::Bed)
                .min_by_key(|p| rect_gap(&p.rect, &band))
            {
                remember(
                    &mut out.reserves,
                    "R4",
                    Some(&next.it.reference),
                    Some(&glaz.id),
                    rect_gap(&next.rect, &band),
                    0,
                    Limit::AtLeast,
                    "cm",
                    ist_hart(rules, "R4")?,
                );
            }
            for p in placed.iter().filter(|p| p.kind == Kind::Bed) {
                if p.rect.overlaps(&band) {
                    sort_in(
                        house_rule(
                            rules,
                            "R4",
                            Some(p.it.reference.clone()),
                            "bed touches the glazing (cold, draught, no privacy towards the terrace)".into(),
                            None,
                            None,
                        )?,
                        &mut out.hard,
                        &mut out.soft,
                    );
                }
            }
        }
    }

    Ok(())
}

/// R5: window light falls on the desk from the side. Neither facing the glazing (glare) nor with
/// the back to it (reflections): a question of which axis the glazing lies on. Without glazing
/// nothing is measured, and that is reported.
fn r5_side_light(ctx: &Check, out: &mut Findings) -> Result<(), ModelError> {
    let room = ctx.room;
    let rules = ctx.rules;
    let placed = &ctx.placed;

    {
        let glass = room
            .openings
            .iter()
            .find(|o| o.kind.as_deref().is_some_and(|t| t.starts_with("glass_door")));
        let desks = placed.iter().filter(|p| p.kind == Kind::Desk).count();
        match glass.and_then(|g| opening_segment(room, g)) {
            None if desks > 0 => out.fell_out.push((
                "R5".to_string(),
                "the room has no glazing a desk could face"
                    .to_string(),
            )),
            Some(seg) => {
                // The direction light comes from is the glazing's normal, not the line between two centres: the
                // first draft compared centres and flagged all 13 layouts for a desk whose window is in the side
                // wall. If the light runs along the desk's front-back axis, it faces it or has its back to it;
                // across, it comes from the side.
                let light_runs_in_y = seg.normal[1].abs() > seg.normal[0].abs();
                for p in placed.iter().filter(|p| p.kind == Kind::Desk) {
                    // `p.rect` is already rotated, so it decides which axis is the depth: one sits at the depth side.
                    let depth_runs_in_y = p.rect.d <= p.rect.w;
                    if light_runs_in_y == depth_runs_in_y {
                        sort_in(
                            house_rule(
                                rules,
                                "R5",
                                Some(p.it.reference.clone()),
                                format!(
                                    "\"{}\" faces the glazing with its front or back: glare or reflections; light should come from the side",
                                    p.it.reference
                                ),
                                None,
                                None,
                            )?,
                            &mut out.hard,
                            &mut out.soft,
                        );
                    }
                }
            }
            _ => {}
        }
    }

    Ok(())
}

/// R6: the view from the entrance does not fall on the bed first. A ray from the entrance inwards
/// along its normal; what it hits first is what one sees. The flat says which opening is the
/// entrance (`eingang = true`); without it R6 does not run and says so.
fn r6_view_from_entrance(ctx: &Check, out: &mut Findings) -> Result<(), ModelError> {
    let room = ctx.room;
    let rules = ctx.rules;
    let placed = &ctx.placed;

    {
        let beds = placed.iter().filter(|p| p.kind == Kind::Bed).count();
        let entrance = room.openings.iter().find(|o| o.entrance == Some(true));
        match entrance.and_then(|e| opening_segment(room, e)) {
            None if beds > 0 => out.fell_out.push((
                "R6".to_string(),
                "no opening is declared the entrance (`eingang = true` in room.toml)"
                    .to_string(),
            )),
            Some(seg) => {
                let mut x = (seg.a[0] + seg.b[0]) as f64 / 2.0;
                let mut y = (seg.a[1] + seg.b[1]) as f64 / 2.0;
                let mut seen: Option<&Placed> = None;
                // In 5 cm steps, at most the diagonal of a very large room.
                for _ in 0..400 {
                    x += seg.normal[0] * 5.0;
                    y += seg.normal[1] * 5.0;
                    if !point_in_polygon([x, y], &room.main_room.polygon) {
                        break;
                    }
                    if let Some(hit) = placed.iter().find(|p| {
                        x >= p.rect.x as f64
                            && x < p.rect.right() as f64
                            && y >= p.rect.y as f64
                            && y < p.rect.bottom() as f64
                    }) {
                        seen = Some(hit);
                        break;
                    }
                }
                if let Some(p) = seen {
                    if p.kind == Kind::Bed {
                        sort_in(
                            house_rule(
                                rules,
                                "R6",
                                Some(p.it.reference.clone()),
                                format!(
                                    "from the entrance the eye falls first on \"{}\"",
                                    p.it.reference
                                ),
                                None,
                                None,
                            )?,
                            &mut out.hard,
                            &mut out.soft,
                        );
                    }
                }
            }
            _ => {}
        }
    }

    Ok(())
}

/// R9: no lasting sun on the desk (only when the flat has the rule). Computes the sun position and
/// the light patch on the floor, so it answers what R5's axis question cannot.
fn r9_lasting_sun(ctx: &Check, out: &mut Findings) -> Result<(), ModelError> {
    let model = ctx.model;
    let rules = ctx.rules;
    let placed = &ctx.placed;

    if rules.rule("R9").is_ok() {
        let desks: Vec<&Placed> = placed.iter().filter(|p| p.kind == Kind::Desk).collect();
        match (model.room.location, rules.sun, desks.is_empty()) {
            (_, _, true) => {}
            (None, _, _) => out.fell_out.push((
                "R9".to_string(),
                "room.toml has no [lage]; without latitude, longitude and north direction there is \
                 no sun position"
                    .to_string(),
            )),
            (_, None, _) => out.fell_out.push((
                "R9".to_string(),
                "rules.toml has R9 but no [sonne] block with the hour threshold"
                    .to_string(),
            )),
            (Some(_), Some(declared_rule), false) => {
                let denominator = crate::sun::checked_hours();
                for p in desks {
                    let n = crate::sun::hours_in_sun(model, &p.rect)?;
                    remember(
                        &mut out.reserves,
                        "R9",
                        Some(&p.it.reference),
                        Some("sonnenstunden"),
                        n as i32,
                        declared_rule.max_hours_at_desk as i32,
                        Limit::AtMost,
                        "stunden",
                        ist_hart(rules, "R9")?,
                    );
                    if n > declared_rule.max_hours_at_desk {
                        sort_in(
                            house_rule(
                                rules,
                                "R9",
                                Some(p.it.reference.clone()),
                                format!(
                                    "\"{}\" lies in direct sun in {n} of {denominator} checked hours, {} are allowed",
                                    p.it.reference, declared_rule.max_hours_at_desk
                                ),
                                Some(n as i32),
                                Some(declared_rule.max_hours_at_desk as i32),
                            )?,
                            &mut out.hard,
                            &mut out.soft,
                        );
                    }
                }
            }
        }
    }

    Ok(())
}

/// The clearances a piece needs to be used, by kind. A declaration replaces the heuristic rather
/// than adding to it; checking both would make it stricter, not different.
fn access_gaps(ctx: &Check, out: &mut Findings) -> Result<(), ModelError> {
    for p in &ctx.placed {
        if own_needs(p, &ctx.grid, &mut out.hard, &mut out.reserves) {
            continue;
        }
        match p.kind {
            Kind::Bed => bed_access(ctx, p, out)?,
            Kind::Desk => chair_zone(ctx, p, out)?,
            Kind::Wardrobe => wardrobe_door(ctx, p, out)?,
            Kind::Couch => r8_couch_unfold(ctx, p, out)?,
            Kind::Table => table_chair(ctx, p, out)?,
            Kind::CoffeeTable => coffee_table_gap(ctx, p, out)?,
            _ => {}
        }
    }
    Ok(())
}

/// Bed access: the best long side and the second.
fn bed_access(ctx: &Check, p: &Placed, out: &mut Findings) -> Result<(), ModelError> {
    let rules = ctx.rules;
    let grid = &ctx.grid;

    let want = rules.threshold("bett_zugang_laengsseite")?;
    let second = rules.threshold("bett_zugang_zweite_seite")?;
    let long = if p.w >= p.d {
        [Side::North, Side::South]
    } else {
        [Side::West, Side::East]
    };
    let cap = horizon(want.max(second));
    let mut depths: Vec<i32> = long
        .iter()
        .map(|s| free_depth_on_side(grid, &p.rect, *s, cap))
        .collect();
    depths.sort_unstable_by(|a, b| b.cmp(a));
    remember_up_to(
        &mut out.reserves,
        "bed_access",
        Some(&p.it.reference),
        Some("best long side"),
        depths[0],
        want,
        Limit::AtLeast,
        "cm",
        true,
        Some(cap),
        true,
    );
    remember_up_to(
        &mut out.reserves,
        "bed_access",
        Some(&p.it.reference),
        Some("second long side"),
        depths[1],
        second,
        Limit::AtLeast,
        "cm",
        true,
        Some(cap),
        true,
    );
    if depths[0] < want {
        out.hard.push(Violation {
            rule: "bed_access".into(),
            severity: Severity::Hard,
            item: Some(p.it.reference.clone()),
            message: format!(
                "bed has {} cm on its best long side, needs {}",
                depths[0], want
            ),
            text: None,
            measured: Some(depths[0]),
            required: Some(want),
        });
    }
    if depths[1] < second {
        out.hard.push(Violation {
            rule: "bed_access".into(),
            severity: Severity::Hard,
            item: Some(p.it.reference.clone()),
            message: format!(
                "bed has {} cm on its second long side, needs {}",
                depths[1], second
            ),
            text: None,
            measured: Some(depths[1]),
            required: Some(second),
        });
    }
    Ok(())
}

/// Space for the desk chair, on the best side.
fn chair_zone(ctx: &Check, p: &Placed, out: &mut Findings) -> Result<(), ModelError> {
    let rules = ctx.rules;
    let grid = &ctx.grid;

    let want = rules.threshold("schreibtisch_stuhlzone")?;
    let cap = horizon(want);
    let best = SIDES
        .iter()
        .map(|s| free_depth_on_side(grid, &p.rect, *s, cap))
        .max()
        .unwrap_or(0);
    remember_up_to(
        &mut out.reserves,
        "chair_zone",
        Some(&p.it.reference),
        None,
        best,
        want,
        Limit::AtLeast,
        "cm",
        true,
        Some(cap),
        true,
    );
    if best < want {
        out.hard.push(Violation {
            rule: "chair_zone".into(),
            severity: Severity::Hard,
            item: Some(p.it.reference.clone()),
            message: format!(
                "desk has {} cm for the chair on its best side, needs {}",
                best, want
            ),
            text: None,
            measured: Some(best),
            required: Some(want),
        });
    }
    Ok(())
}

/// Space to open the wardrobe doors, on the best side.
fn wardrobe_door(ctx: &Check, p: &Placed, out: &mut Findings) -> Result<(), ModelError> {
    let rules = ctx.rules;
    let grid = &ctx.grid;

    let want = rules.threshold("schrank_tuer_oeffnen")?;
    let cap = horizon(want);
    let best = SIDES
        .iter()
        .map(|s| free_depth_on_side(grid, &p.rect, *s, cap))
        .max()
        .unwrap_or(0);
    remember_up_to(
        &mut out.reserves,
        "wardrobe_door",
        Some(&p.it.reference),
        None,
        best,
        want,
        Limit::AtLeast,
        "cm",
        true,
        Some(cap),
        true,
    );
    if best < want {
        out.hard.push(Violation {
            rule: "wardrobe_door".into(),
            severity: Severity::Hard,
            item: Some(p.it.reference.clone()),
            message: format!("wardrobe has {} cm to open its doors, needs {}", best, want),
            text: None,
            measured: Some(best),
            required: Some(want),
        });
    }
    Ok(())
}

/// R8: the sofa bed has room to unfold. It is not moved for that (Lars, 2026-08-30), so the unfold
/// depth is permanently occupied. Checked on the long sides only, where it unfolds.
fn r8_couch_unfold(ctx: &Check, p: &Placed, out: &mut Findings) -> Result<(), ModelError> {
    let rules = ctx.rules;
    let grid = &ctx.grid;

    if let Ok(want) = rules.threshold("couch_ausklapptiefe") {
        let long = if p.w >= p.d {
            [Side::North, Side::South]
        } else {
            [Side::West, Side::East]
        };
        let cap = horizon(want);
        let best = long
            .iter()
            .map(|s| free_depth_on_side(grid, &p.rect, *s, cap))
            .max()
            .unwrap_or(0);
        remember_up_to(
            &mut out.reserves,
            "R8",
            Some(&p.it.reference),
            Some("best long side"),
            best,
            want,
            Limit::AtLeast,
            "cm",
            ist_hart(rules, "R8")?,
            Some(cap),
            true,
        );
        if best < want {
            // Reported as R8, the id every rules.toml uses.
            sort_in(
                house_rule(
                    rules,
                    "R8",
                    Some(p.it.reference.clone()),
                    format!("couch has {} cm to unfold on its best long side, needs {}; it should not have to be moved for it", best, want),
                    Some(best),
                    Some(want),
                )?,
                &mut out.hard,
                &mut out.soft,
            );
        }
    }
    Ok(())
}

/// Room to pull out a dining chair, and on how many sides. Hard when no side has it; soft when only
/// one does, since that is a seat count: a wall-mounted table legitimately has one side.
fn table_chair(ctx: &Check, p: &Placed, out: &mut Findings) -> Result<(), ModelError> {
    let rules = ctx.rules;
    let grid = &ctx.grid;

    let want = rules.threshold("esstisch_stuhl_ausziehen")?;
    let cap = horizon(want);
    let sides: Vec<i32> = SIDES
        .iter()
        .map(|s| free_depth_on_side(grid, &p.rect, *s, cap))
        .collect();
    let best = *sides.iter().max().unwrap_or(&0);
    let seats = sides.iter().filter(|d| **d >= want).count();
    remember_up_to(
        &mut out.reserves,
        "chair_pull_out",
        Some(&p.it.reference),
        Some("beste seite"),
        best,
        want,
        Limit::AtLeast,
        "cm",
        true,
        Some(cap),
        true,
    );
    remember(
        &mut out.reserves,
        "chair_pull_out",
        Some(&p.it.reference),
        Some("seats"),
        seats as i32,
        2,
        Limit::AtLeast,
        "seats",
        false,
    );
    if seats == 0 {
        out.hard.push(Violation {
            rule: "chair_pull_out".into(),
            severity: Severity::Hard,
            item: Some(p.it.reference.clone()),
            message: format!(
                "table has {} cm to pull out a chair on its best side, needs {}",
                best, want
            ),
            text: None,
            measured: Some(best),
            required: Some(want),
        });
    } else if seats < 2 {
        out.soft.push(Violation {
            rule: "chair_pull_out".into(),
            severity: Severity::Soft,
            item: Some(p.it.reference.clone()),
            message: format!(
                "table offers {} seat: only one side has the {} cm a chair needs",
                seats, want
            ),
            text: None,
            measured: Some(seats as i32),
            required: Some(2),
        });
    }
    Ok(())
}

/// Gap between coffee table and sofa, per sofa in the room.
fn coffee_table_gap(ctx: &Check, p: &Placed, out: &mut Findings) -> Result<(), ModelError> {
    let rules = ctx.rules;
    let placed = &ctx.placed;

    let want = rules.threshold("couchtisch_vor_sofa")?;
    for c in placed.iter().filter(|q| q.kind == Kind::Couch) {
        let dx = (c.rect.x - p.rect.right())
            .max(p.rect.x - c.rect.right())
            .max(0);
        let dy = (c.rect.y - p.rect.bottom())
            .max(p.rect.y - c.rect.bottom())
            .max(0);
        let gap = (((dx * dx + dy * dy) as f64).sqrt()).round() as i32;
        remember(
            &mut out.reserves,
            "coffee_table_gap",
            Some(&p.it.reference),
            Some(&c.it.reference),
            gap,
            want,
            Limit::AtLeast,
            "cm",
            false,
        );
        if gap < want {
            out.soft.push(Violation {
                rule: "coffee_table_gap".into(),
                severity: Severity::Soft,
                item: Some(p.it.reference.clone()),
                message: format!("coffee table stands {} cm from the sofa, wants {}", gap, want),
                text: None,
                measured: Some(gap),
                required: Some(want),
            });
        }
    }
    Ok(())
}

/// The routes room.toml declares: how narrow each gets at its tightest. Returns the measured
/// corridors for `metrics`.
fn walkways(ctx: &Check, out: &mut Findings) -> Result<Vec<Corridor>, ModelError> {
    let room = ctx.room;
    let rules = ctx.rules;
    let grid = &ctx.grid;

    let field = clearance_field(grid);
    let mut waypoints: Vec<(String, Pt)> = Vec::new();
    for o in &room.openings {
        if o.kind.as_deref() == Some("window") {
            continue;
        }
        let Some(seg) = opening_segment(room, o) else {
            continue;
        };
        let mid = [
            (seg.a[0] + seg.b[0]) as f64 / 2.0,
            (seg.a[1] + seg.b[1]) as f64 / 2.0,
        ];
        waypoints.push((
            o.id.clone(),
            [
                (mid[0] + seg.normal[0] * WAYPOINT_GAP as f64) as i32,
                (mid[1] + seg.normal[1] * WAYPOINT_GAP as f64) as i32,
            ],
        ));
    }
    for f in &room.fixed_furniture {
        let Some(zone_decl) = &f.approach_zone else {
            continue;
        };
        waypoints.push((f.id.clone(), approach_point(&f.rect(), zone_decl.side)));
    }
    let find = |id: &str| waypoints.iter().find(|(n, _)| n == id).map(|(_, p)| *p);

    let mut corridors = Vec::new();
    for r in &room.routes {
        let (from, to) = (r.from.as_str(), r.to.as_str());
        // A waypoint that does not exist is an error, not a skipped route.
        let (Some(a), Some(b)) = (find(from), find(to)) else {
            let known: Vec<&str> = waypoints.iter().map(|(n, _)| n.as_str()).collect();
            return Err(ModelError::Missing(format!(
                "route {from} → {to}: no waypoint of that name. Waypoints are openings \
                 other than windows, and fixtures with `anlaufzone`; present are: {}",
                known.join(", ")
            )));
        };
        let bottleneck = widest_path(grid, &field, a, b);
        let width = bottleneck.map(|v| (v * 2.0).round() as i32);
        corridors.push(Corridor {
            from: from.into(),
            to: to.into(),
            width_cm: width,
        });
        // No path is zero width, not "not measured": the route was searched and does not exist.
        remember(
            &mut out.reserves,
            "walkway",
            None,
            Some(&format!("{from} → {to}")),
            width.unwrap_or(0),
            rules.walkways.main_min,
            Limit::AtLeast,
            "cm",
            true,
        );
        match width {
            None => out.hard.push(Violation {
                rule: "walkway".into(),
                severity: Severity::Hard,
                item: None,
                message: format!("no walkable path at all from {} to {}", from, to),
                text: None,
                measured: None,
                required: Some(rules.walkways.main_min),
            }),
            Some(w) if w < rules.walkways.main_min => out.hard.push(Violation {
                rule: "walkway".into(),
                severity: Severity::Hard,
                item: None,
                message: format!(
                    "route {} → {} narrows to {} cm, below the minimum of {} cm",
                    from, to, w, rules.walkways.main_min
                ),
                text: None,
                measured: Some(w),
                required: Some(rules.walkways.main_min),
            }),
            Some(w) if w < rules.walkways.main_target => out.soft.push(Violation {
                rule: "walkway".into(),
                severity: Severity::Soft,
                item: None,
                message: format!(
                    "route {} → {} narrows to {} cm, below the target of {} cm",
                    from, to, w, rules.walkways.main_target
                ),
                text: None,
                measured: Some(w),
                required: Some(rules.walkways.main_target),
            }),
            _ => {}
        }
    }
    Ok(corridors)
}

/// The report: what was measured, what failed, and what was not checked. It recomputes no rule; it
/// counts, picks the tightest reserve, and merges failed-to-run rules with unimplemented ones.
fn finish(ctx: &Check, layout: &Layout, out: Findings, corridors: Vec<Corridor>) -> CheckResult {
    let Findings {
        hard,
        soft,
        reserves,
        fell_out,
    } = out;
    let model = ctx.model;
    let room = ctx.room;
    let rules = ctx.rules;
    let placed = &ctx.placed;
    let grid = &ctx.grid;

    let occupied: i32 = placed.iter().map(|p| p.w * p.d).sum();
    let refs: Vec<&str> = layout.items.iter().map(|i| i.reference.as_str()).collect();
    let uncertainties = model
        .catalogue
        .values()
        .filter(|i: &&Item| i.is_uncertain() && refs.contains(&i.id.as_str()))
        .map(|i| Uncertainty {
            reference: i.id.clone(),
            label: i.label.clone(),
            fields: i.estimated.clone(),
        })
        .collect();

    // Only centimetres: a minimum across a depth and a seat count has no unit.
    // What this checker checks at all: the required rules plus the optional ones, so a declared R9
    // that ran does not report itself as unchecked.
    let checked: Vec<&str> = RULE_IDS
        .iter()
        .chain(OPTIONAL_RULE_IDS.iter())
        .copied()
        .collect();

    let tightest_reserve_cm = reserves
        .iter()
        .filter(|r| r.hard && r.unit == "cm" && (r.binding || r.slack < 0))
        .map(|r| r.slack)
        .min();

    CheckResult {
        layout: layout.name.clone(),
        pass: hard.is_empty(),
        hard,
        soft,
        uncertainties,
        reserves,
        tightest_reserve_cm,
        metrics: Metrics {
            room_area_m2: room.area_m2(),
            occupied_area_m2: occupied as f64 / 10_000.0,
            free_area_m2: (grid.free_cells() as f64 * (RES * RES) as f64) / 10_000.0,
            corridors,
        },
        outdated: placed
            .iter()
            .filter(|p| {
                p.item
                    .and_then(|i| i.priority.as_deref())
                    .is_some_and(|s| s == "rejected")
            })
            .map(|p| p.it.reference.clone())
            .collect(),
        unchecked: rules
            .unchecked(&checked)
            .into_iter()
            .map(|r| UncheckedRule {
                rule: r.id.clone(),
                text: r.text.clone(),
                reason: "this checker does not check it".to_string(),
            })
            .chain(fell_out.into_iter().map(|(id, reason)| UncheckedRule {
                rule: id.clone(),
                text: rules.rule(&id).map(|r| r.text.clone()).unwrap_or_default(),
                reason,
            }))
            .collect(),
    }
}
