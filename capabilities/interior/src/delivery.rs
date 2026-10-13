//! Does the piece get in at all, and as far as its place?
//!
//! Every other check asks whether a piece may stand somewhere; this one asks whether it gets
//! there. A wardrobe that passes every rule at its place and does not fit through the door is money
//! spent.
//!
//! The search runs in configuration space: position and orientation together, `(x, y, across |
//! lengthwise)`, since a rect looks the same after 180°. Three simplifications, all strict, so a yes
//! holds and a no is a warning:
//!
//! 1. Rotation only where the whole swept circle is free; a no means "not without tilting".
//! 2. Only fixtures are obstacles: on moving day the flat is empty.
//! 3. Door height is not checked: `room.toml` has no height, and inventing one is worse than
//!    naming the gap.
//!
//! The door lies in the wall, outside the walkable polygon, so the path starts at the first
//! position inside the room in front of it; fitting through the opening is its own question.

use crate::geometry::{Grid, RES};
use crate::model::{footprint, Layout, Model, ModelError, Rect};
use serde::Serialize;
use std::collections::VecDeque;

/// Does the piece fit through the entrance door?
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "art", rename_all = "snake_case")]
pub enum DoorFit {
    /// The narrow side fits, with this much air in cm.
    Fits { air_cm: i32, door_cm: i32 },
    /// The narrow side is wider than the opening.
    DoesNotFit { missing_cm: i32, door_cm: i32 },
    /// Does not fit assembled, but comes apart (`disassembles` on the row). This comes from a
    /// declaration, not from geometry.
    CarriedApart { missing_cm: i32, door_cm: i32 },
    /// No opening of this flat is declared the entrance (`eingang = true`).
    NoEntrance,
}

/// How the piece reaches its place, or why not.
#[derive(Debug, Clone, Serialize)]
pub struct Delivery {
    pub reference: String,
    pub width: i32,
    pub depth: i32,
    pub door: DoorFit,
    /// Whether a path leads from the door to the target place.
    pub affordable: bool,
    /// Grid steps of the shortest path, rotations included.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub steps: Option<usize>,
    /// Whether the path has to rotate the piece at least once.
    pub rotates: bool,
    /// What is missing, in words; empty when nothing is.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// A position a rect can take: cell and orientation.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Location {
    c: i32,
    r: i32,
    across: bool,
}

/// The flat's grid without movable pieces: polygon minus fixtures.
fn empty_room(model: &Model) -> Grid {
    let mut g = Grid::new(&model.room.main_room.polygon);
    for f in &model.room.fixed_furniture {
        g.occupy(&f.rect());
    }
    g
}

/// Is the footprint at this position entirely free?
fn is_free(g: &Grid, l: Location, w: i32, d: i32) -> bool {
    let (w, d) = if l.across { (d, w) } else { (w, d) };
    let cw = (w as f64 / RES as f64).ceil() as i32;
    let cd = (d as f64 / RES as f64).ceil() as i32;
    if l.c < 0 || l.r < 0 || l.c + cw > g.cols as i32 || l.r + cd > g.rows as i32 {
        return false;
    }
    for r in l.r..(l.r + cd) {
        for c in l.c..(l.c + cw) {
            if g.free[g.idx(c as usize, r as usize)] == 0 {
                return false;
            }
        }
    }
    true
}

/// May the piece rotate here? The swept circle must be free, approximated by the square around
/// it with the diagonal as its side. Conservative on purpose: what passes here really passes.
fn may_rotate(g: &Grid, l: Location, w: i32, d: i32) -> bool {
    let diag = (((w * w + d * d) as f64).sqrt().ceil()) as i32;
    let (bw, bd) = if l.across { (d, w) } else { (w, d) };
    let centre_x = l.c * RES + bw / 2;
    let centre_y = l.r * RES + bd / 2;
    let kreis = Location {
        c: (centre_x - diag / 2).div_euclid(RES),
        r: (centre_y - diag / 2).div_euclid(RES),
        across: false,
    };
    is_free(g, kreis, diag, diag)
}

/// The same position, rotated about its own centre.
fn rotated(l: Location, w: i32, d: i32) -> Location {
    let (bw, bd) = if l.across { (d, w) } else { (w, d) };
    let centre_x = l.c * RES + bw / 2;
    let centre_y = l.r * RES + bd / 2;
    let (nw, nd) = (bd, bw);
    Location {
        c: (centre_x - nw / 2).div_euclid(RES),
        r: (centre_y - nd / 2).div_euclid(RES),
        across: !l.across,
    }
}

/// Does the piece fit through the entrance door? `disassembles` comes from the piece's row.
pub fn through_the_door(model: &Model, width: i32, depth: i32, disassembles: bool) -> DoorFit {
    let Some(e) = model
        .room
        .openings
        .iter()
        .find(|o| o.entrance == Some(true))
    else {
        return DoorFit::NoEntrance;
    };
    // The narrow side decides: a wardrobe goes through a door upright.
    let narrow = width.min(depth);
    if narrow <= e.width {
        DoorFit::Fits {
            air_cm: e.width - narrow,
            door_cm: e.width,
        }
    } else if disassembles {
        DoorFit::CarriedApart {
            missing_cm: narrow - e.width,
            door_cm: e.width,
        }
    } else {
        DoorFit::DoesNotFit {
            missing_cm: narrow - e.width,
            door_cm: e.width,
        }
    }
}

/// Can a piece of this size get from the door onto this rect?
pub fn path_to_place(
    model: &Model,
    width: i32,
    depth: i32,
    target: Rect,
    disassembles: bool,
) -> Result<Delivery, ModelError> {
    let door = through_the_door(model, width, depth, disassembles);
    let g = empty_room(model);

    let target_location = Location {
        c: target.x.div_euclid(RES),
        r: target.y.div_euclid(RES),
        across: target.w != width,
    };

    // Start positions: everything right in front of the entrance opening, over its whole span,
    // because which cell that is depends on the wall the door sits in.
    let Some(e) = model
        .room
        .openings
        .iter()
        .find(|o| o.entrance == Some(true))
    else {
        return Ok(Delivery {
            reference: String::new(),
            width,
            depth,
            door,
            affordable: false,
            steps: None,
            rotates: false,
            reason: Some(
                "no opening is declared the entrance (`eingang = true` in room.toml)".into(),
            ),
        });
    };
    let Some(seg) = crate::clearance::opening_segment(&model.room, e) else {
        return Err(ModelError::Missing(format!(
            "Oeffnung `{}` liegt in keiner bekannten Wand",
            e.id
        )));
    };

    // Entry is one cell inside the door's middle. Every position whose footprint covers it is a
    // start, so no single posture is prescribed and the grid alone cannot say no.
    let ex = ((seg.a[0] + seg.b[0]) as f64 / 2.0 + seg.normal[0] * RES as f64) as i32;
    let ey = ((seg.a[1] + seg.b[1]) as f64 / 2.0 + seg.normal[1] * RES as f64) as i32;
    let (ec, er) = (ex.div_euclid(RES), ey.div_euclid(RES));

    let mut start: Vec<Location> = Vec::new();
    for across in [false, true] {
        let (bw, bd) = if across { (depth, width) } else { (width, depth) };
        let (cw, cd) = (
            (bw as f64 / RES as f64).ceil() as i32,
            (bd as f64 / RES as f64).ceil() as i32,
        );
        for r in (er - cd + 1)..=er {
            for c in (ec - cw + 1)..=ec {
                let l = Location { c, r, across };
                if is_free(&g, l, width, depth) {
                    start.push(l);
                }
            }
        }
    }

    if !is_free(&g, target_location, width, depth) {
        return Ok(Delivery {
            reference: String::new(),
            width,
            depth,
            door,
            affordable: false,
            steps: None,
            rotates: false,
            reason: Some(
                "the target place itself is not free in the empty room: a fixture stands there"
                    .into(),
            ),
        });
    }
    if start.is_empty() {
        return Ok(Delivery {
            reference: String::new(),
            width,
            depth,
            door,
            affordable: false,
            steps: None,
            rotates: false,
            reason: Some(
                "right behind the door there is no free position the piece fits".into(),
            ),
        });
    }

    // Breadth-first: four neighbours plus rotating in place, all at equal cost, so the first arrival
    // is the shortest path.
    let mut seen: std::collections::HashSet<Location> = std::collections::HashSet::new();
    let mut queue: VecDeque<(Location, usize, bool)> = VecDeque::new();
    for s in start {
        if seen.insert(s) {
            queue.push_back((s, 0, false));
        }
    }
    while let Some((l, n, rotates)) = queue.pop_front() {
        if l == target_location {
            return Ok(Delivery {
                reference: String::new(),
                width,
                depth,
                door,
                affordable: true,
                steps: Some(n),
                rotates,
                reason: None,
            });
        }
        for (dc, dr) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let n2 = Location {
                c: l.c + dc,
                r: l.r + dr,
                across: l.across,
            };
            if is_free(&g, n2, width, depth) && seen.insert(n2) {
                queue.push_back((n2, n + 1, rotates));
            }
        }
        if may_rotate(&g, l, width, depth) {
            let n2 = rotated(l, width, depth);
            if is_free(&g, n2, width, depth) && seen.insert(n2) {
                queue.push_back((n2, n + 1, true));
            }
        }
    }

    Ok(Delivery {
        reference: String::new(),
        width,
        depth,
        door,
        affordable: false,
        steps: None,
        rotates: false,
        reason: Some(
            "no path from the door to the place without tilting the piece or touching a fixture"
                .into(),
        ),
    })
}

/// The same question for a piece that already has a place in a layout.
pub fn delivery(
    model: &Model,
    layout: &Layout,
    reference: &str,
) -> Result<Delivery, ModelError> {
    let it = layout
        .items
        .iter()
        .find(|i| i.reference == reference)
        .ok_or_else(|| {
            ModelError::Missing(format!(
                "`{reference}` is not in \"{}\": nothing to deliver",
                layout.name
            ))
        })?;
    let (w, d, _) = footprint(it, &model.catalogue)?;
    let disassembles = model
        .catalogue
        .get(reference)
        .and_then(|i| i.disassembles)
        .unwrap_or(false);
    let mut e = path_to_place(
        model,
        w,
        d,
        Rect {
            x: it.x,
            y: it.y,
            w,
            d,
        },
        disassembles,
    )?;
    e.reference = reference.to_string();
    Ok(e)
}
