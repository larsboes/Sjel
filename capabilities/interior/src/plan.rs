//! Draws a layout as SVG.
//!
//! Nothing here knows a size: every number on the sheet comes from the model at run time, from the
//! same coordinates the checker judges, so picture and verdict cannot drift apart. Self-contained:
//! no external font, script or image host, so it works from a file and offline.

use crate::clearance::{check_layout, kind_of, Kind};
use crate::model::{footprint, Layout, Model, ModelError, PlacedItem, Rect};

const PAD: i32 = 40;

/// Label font sizes, largest first. The viewBox runs in centimetres, so a font size is a length on
/// the sheet and grows with the plan, not the window.
const GRADE: [i32; 4] = [19, 16, 14, 12];

/// Mean Helvetica glyph width as a share of the font size. Estimated, since the SVG embeds no
/// font, and on the large side: an underestimate runs past its edge.
const GLYPH: f64 = 0.58;

/// Air between text and the piece's edge.
const AIR: i32 = 4;

/// Gap between a piece's edge and a label placed outside it.
const GAP: i32 = 8;

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Head words that identify an id, matched as endings because German compounds end in their
/// head word (`buerostuhl` is a `stuhl`). The longest match wins, so order does not matter. The
/// keys match your item ids and stay German; the labels are English. An id with no match shows
/// its own first word.
const HEADER_WORDS: &[(&str, &str)] = &[
    ("saugroboter", "Vacuum"),
    ("beistelltisch", "Side table"),
    ("couchtisch", "Coffee table"),
    ("schreibtisch", "Desk"),
    ("stuehle", "Chair"),
    ("stuhl", "Chair"),
    ("schrank", "Wardrobe"),
    ("pflanzen", "Plant"),
    ("pflanze", "Plant"),
    ("regale", "Shelf"),
    ("regal", "Shelf"),
    ("platte", "Top"),
    ("tisch", "Table"),
    ("couch", "Couch"),
    ("sofa", "Couch"),
    ("bett", "Bed"),
];

/// The word a declared kind shows. `Other` has none: it means none of the known kinds.
fn kind_word(kind: Kind) -> Option<&'static str> {
    match kind {
        Kind::Bed => Some("Bed"),
        Kind::Desk => Some("Desk"),
        Kind::Couch => Some("Couch"),
        Kind::Wardrobe => Some("Wardrobe"),
        Kind::CoffeeTable => Some("Coffee table"),
        Kind::Table => Some("Table"),
        Kind::Shelf => Some("Shelf"),
        Kind::Other => None,
    }
}

fn capitalised(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// The short name shown inside a piece, from the id rather than the human `label`.
///
/// 1. The id splits at `_`: the first part is the word, a trailing number the count, the rest is
///    dropped (`pflanze_boden_1` -> "Plant 1").
/// 2. The word is shortened to its head word if one matches.
/// 3. Otherwise the kind declared in the layout, else the word itself.
///
/// Name before kind, because the name is more precise: `kind_of_name` files an office chair
/// under Desk so it inherits a workplace's zones, but it should still read "Chair".
pub fn short_label(reference: &str, declared: Option<Kind>) -> String {
    let mut parts: Vec<&str> = reference.split('_').filter(|s| !s.is_empty()).collect();
    let number = match parts.last() {
        Some(l) if parts.len() > 1 && l.chars().all(|c| c.is_ascii_digit()) => parts.pop(),
        _ => None,
    };
    let word = parts
        .first()
        .copied()
        .unwrap_or(reference)
        .to_ascii_lowercase();
    let header = HEADER_WORDS
        .iter()
        .filter(|(ending, _)| word.ends_with(ending))
        .max_by_key(|(ending, _)| ending.len())
        .map(|(_, short)| (*short).to_string())
        .or_else(|| declared.and_then(kind_word).map(str::to_string))
        .unwrap_or_else(|| capitalised(&word));
    match number {
        Some(n) => format!("{header} {n}"),
        None => header,
    }
}

/// How wide a text runs on the sheet; see `GLYPH`.
fn width(n_chars: usize, grad: i32) -> i32 {
    (n_chars as f64 * GLYPH * grad as f64).ceil() as i32
}

/// The area of a centred text, for the room's own labels (bathroom, fixtures, terrace), so piece
/// labels avoid them. `blocking` is the letter-spacing those carry.
fn centred(cx: i32, cy: i32, n_chars: usize, grad: i32, blocking: i32, across: bool) -> Rect {
    let long = width(n_chars, grad) + n_chars as i32 * blocking;
    let (w, d) = if across { (grad, long) } else { (long, grad) };
    Rect {
        x: cx - w / 2,
        y: cy - d / 2,
        w,
        d,
    }
}

/// A placed label: where, how large, which way round, and the area it takes, which the next
/// label checks against.
struct PlacedText {
    x: i32,
    y: i32,
    grad: i32,
    anker: &'static str,
    across: bool,
    leader: Option<[i32; 4]>,
    area: Rect,
}

fn is_free(f: &Rect, obstacles: &[Rect], set: &[Rect], sheet: &Rect) -> bool {
    f.x >= sheet.x
        && f.y >= sheet.y
        && f.right() <= sheet.right()
        && f.bottom() <= sheet.bottom()
        && !obstacles.iter().any(|h| h.overlaps(f))
        && !set.iter().any(|g| g.overlaps(f))
}

/// Where a piece's label goes.
///
/// Inside first, as large as possible, horizontal then upright. If no size fits, outside at the
/// first spot that hits no piece, no label and no sheet edge, held by a leader line. Only when
/// nothing is free does it go small to the east and may collide; a label that fits nowhere says
/// something about the crowding.
///
/// `sheet` is the whole drawing, `core` the measured part: a label above or below its piece is
/// pushed into the core rather than onto the estimated terrace.
fn set_value(
    r: &Rect,
    n_chars: usize,
    obstacles: &[Rect],
    set: &[Rect],
    sheet: &Rect,
    core: &Rect,
) -> PlacedText {
    let (cx, cy) = (r.x + r.w / 2, r.y + r.d / 2);
    for grad in GRADE {
        let tw = width(n_chars, grad);
        if tw + 2 * AIR <= r.w && grad + 2 * AIR <= r.d {
            return PlacedText {
                x: cx,
                y: cy,
                grad,
                anker: "middle",
                across: false,
                leader: None,
                area: Rect {
                    x: cx - tw / 2,
                    y: cy - grad / 2,
                    w: tw,
                    d: grad,
                },
            };
        }
        if tw + 2 * AIR <= r.d && grad + 2 * AIR <= r.w {
            return PlacedText {
                x: cx,
                y: cy,
                grad,
                anker: "middle",
                across: true,
                leader: None,
                area: Rect {
                    x: cx - grad / 2,
                    y: cy - tw / 2,
                    w: grad,
                    d: tw,
                },
            };
        }
    }

    let outside = |grad: i32| -> [PlacedText; 4] {
        let tw = width(n_chars, grad);
        let half = grad / 2;
        // Pushed horizontally into the core while the text fits there at all.
        let lengthwise = if tw <= core.w {
            (cx - tw / 2).clamp(core.x, core.right() - tw)
        } else {
            cx - tw / 2
        };
        [
            PlacedText {
                x: r.right() + GAP,
                y: cy,
                grad,
                anker: "start",
                across: false,
                leader: Some([r.right(), cy, r.right() + GAP - 2, cy]),
                area: Rect {
                    x: r.right() + GAP,
                    y: cy - half,
                    w: tw,
                    d: grad,
                },
            },
            PlacedText {
                x: r.x - GAP,
                y: cy,
                grad,
                anker: "end",
                across: false,
                leader: Some([r.x, cy, r.x - GAP + 2, cy]),
                area: Rect {
                    x: r.x - GAP - tw,
                    y: cy - half,
                    w: tw,
                    d: grad,
                },
            },
            PlacedText {
                x: lengthwise + tw / 2,
                y: r.bottom() + GAP + half,
                grad,
                anker: "middle",
                across: false,
                leader: Some([cx, r.bottom(), cx, r.bottom() + GAP - 2]),
                area: Rect {
                    x: lengthwise,
                    y: r.bottom() + GAP,
                    w: tw,
                    d: grad,
                },
            },
            PlacedText {
                x: lengthwise + tw / 2,
                y: r.y - GAP - half,
                grad,
                anker: "middle",
                across: false,
                leader: Some([cx, r.y, cx, r.y - GAP + 2]),
                area: Rect {
                    x: lengthwise,
                    y: r.y - GAP - grad,
                    w: tw,
                    d: grad,
                },
            },
        ]
    };

    for grad in GRADE {
        for candidate in outside(grad) {
            if is_free(&candidate.area, obstacles, set, sheet) {
                return candidate;
            }
        }
    }
    let [east, ..] = outside(GRADE[GRADE.len() - 1]);
    east
}

pub fn svg(model: &Model, layout: &Layout) -> Result<String, ModelError> {
    let room = &model.room;
    let poly = &room.main_room.polygon;
    // Only what is measured spans the sheet. The terrace is estimated, so it is drawn as a band the
    // sheet edge cuts off: its unknown depth shows in the picture.
    let mut xs: Vec<i32> = poly.iter().map(|p| p[0]).collect();
    let mut ys: Vec<i32> = poly.iter().map(|p| p[1]).collect();
    if let Some(b) = &room.bathroom {
        xs.extend(b.x);
        ys.extend(b.y);
    }
    let (min_x, max_x) = (
        xs.iter().min().unwrap() - PAD,
        xs.iter().max().unwrap() + PAD,
    );
    let (min_y, max_y) = (
        ys.iter().min().unwrap() - PAD,
        ys.iter().max().unwrap() + PAD,
    );

    let mut p = String::new();
    // Set width and height in the viewBox ratio: with only `width`, Quick Look falls back to a square
    // and crops the plan.
    let (vw, vh) = (max_x - min_x, max_y - min_y);
    let out_w = 1100;
    let out_h = (out_w as f64 * vh as f64 / vw as f64).round() as i32;
    p.push_str(&format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="{min_x} {min_y} {vw} {vh}" width="{out_w}" height="{out_h}">"##));
    p.push_str(&format!(
        r##"<rect x="{min_x}" y="{min_y}" width="{}" height="{}" fill="#FBFAF8"/>"##,
        max_x - min_x,
        max_y - min_y
    ));

    // Every text already on the sheet; piece labels avoid them.
    let mut set: Vec<Rect> = Vec::new();

    // Terrace first, so the outer wall lies on top. Hatched, dashed on its long sides, open to the
    // outside and labelled "approx.": here is something nobody measured. Its text blocks like any
    // other; the band itself blocks nothing.
    if let Some(t) = &room.terrace {
        let (x0, x1) = (t.x[0].max(min_x), t.x[1].min(max_x));
        let (y0, y1) = (t.y[0].max(min_y), t.y[1].min(max_y));
        if x1 > x0 && y1 > y0 {
            let (w, h) = (x1 - x0, y1 - y0);
            // Hatching cut per stroke to the band. A `<pattern>` would carry an id that several plans on
            // one page would share.
            let mut s = x0 - h;
            while s < x1 {
                let lo = (x0 - s).max(0);
                let hi = (x1 - s).min(h);
                if hi > lo {
                    p.push_str(&format!(
                        r##"<line x1="{}" y1="{}" x2="{}" y2="{}" stroke="#CFCAC0" stroke-width="2"/>"##,
                        s + lo, y0 + lo, s + hi, y0 + hi));
                }
                s += 30;
            }
            for edge in [y0, y1] {
                p.push_str(&format!(
                    r##"<line x1="{x0}" y1="{edge}" x2="{x1}" y2="{edge}" stroke="#9A968E" stroke-width="3" stroke-dasharray="18 12"/>"##
                ));
            }
            let (cx, cy) = (x0 + w / 2, y0 + h / 2);
            let name = format!("TERRASSE{}", if t.estimated { " (ca.)" } else { "" });
            set.push(centred(cx, cy, name.chars().count(), 20, 1, true));
            p.push_str(&format!(
                r##"<text x="{cx}" y="{cy}" transform="rotate(-90 {cx} {cy})" font-family="Helvetica,Arial" font-size="20" fill="#5B5F63" text-anchor="middle" dominant-baseline="central" letter-spacing="1">{name}</text>"##
            ));
        }
    }

    // Area only; the outline is drawn after the pieces.
    let pts = poly
        .iter()
        .map(|q| format!("{},{}", q[0], q[1]))
        .collect::<Vec<_>>()
        .join(" ");
    p.push_str(&format!(r##"<polygon points="{pts}" fill="#F4F1EB"/>"##));

    if let Some(b) = &room.bathroom {
        let (w, h) = (b.x[1] - b.x[0], b.y[1] - b.y[0]);
        p.push_str(&format!(
            r##"<rect x="{}" y="{}" width="{w}" height="{h}" fill="#E2DDD2" stroke="#9A968E" stroke-width="3"/>"##,
            b.x[0], b.y[0]));
        let (cx, cy) = (b.x[0] + w / 2, b.y[0] + h / 2);
        set.push(centred(cx, cy, 3, 20, 1, false));
        p.push_str(&format!(
            r##"<text x="{cx}" y="{cy}" font-family="Helvetica,Arial" font-size="20" fill="#5B5F63" text-anchor="middle" dominant-baseline="central" letter-spacing="1">BAD</text>"##
        ));
    }

    for f in &room.fixed_furniture {
        let (w, h) = (f.x[1] - f.x[0], f.y[1] - f.y[0]);
        p.push_str(&format!(
            r##"<rect x="{}" y="{}" width="{w}" height="{h}" fill="none" stroke="#6E655A" stroke-width="3" stroke-dasharray="12 8"/>"##,
            f.x[0], f.y[0]));
        // Centred, with the name from the file.
        let name = f.id.to_uppercase();
        let (cx, cy) = (f.x[0] + w / 2, f.y[0] + h / 2);
        set.push(centred(cx, cy, name.chars().count(), 20, 1, false));
        p.push_str(&format!(
            r##"<text x="{cx}" y="{cy}" font-family="Helvetica,Arial" font-size="20" fill="#5B5F63" text-anchor="middle" dominant-baseline="central" letter-spacing="1">{}</text>"##,
            esc(&name)));
    }

    // All footprints first, then the labels: a label that has to go outside needs to know the pieces
    // after it too.
    let mut pieces: Vec<(&PlacedItem, Rect, String)> = Vec::with_capacity(layout.items.len());
    for it in &layout.items {
        let (w, d, _) = footprint(it, &model.catalogue)?;
        let label = short_label(&it.reference, it.kind.as_ref().map(|_| kind_of(it)));
        pieces.push((
            it,
            Rect {
                x: it.x,
                y: it.y,
                w,
                d,
            },
            label,
        ));
    }
    let mut obstacles: Vec<Rect> = pieces.iter().map(|(_, r, _)| *r).collect();
    if let Some(b) = &room.bathroom {
        obstacles.push(Rect {
            x: b.x[0],
            y: b.y[0],
            w: b.x[1] - b.x[0],
            d: b.y[1] - b.y[0],
        });
    }
    for f in &room.fixed_furniture {
        obstacles.push(Rect {
            x: f.x[0],
            y: f.y[0],
            w: f.x[1] - f.x[0],
            d: f.y[1] - f.y[0],
        });
    }
    let sheet = Rect {
        x: min_x,
        y: min_y,
        w: vw,
        d: vh,
    };
    let core = Rect {
        x: min_x + PAD,
        y: min_y + PAD,
        w: vw - 2 * PAD,
        d: vh - 2 * PAD,
    };

    for (it, r, label) in &pieces {
        let (w, d) = (r.w, r.d);
        // Rect and label in one group with the ref as handle, so dragging moves both. `data-x`/`data-y`
        // carry model coordinates in cm, so the dashboard inverts the viewBox matrix instead of keeping a
        // second scale.
        p.push_str(&format!(
            r##"<g data-ref="{}" data-x="{}" data-y="{}" data-w="{w}" data-d="{d}" data-rot="{}">"##,
            esc(&it.reference), r.x, r.y, it.rot));
        p.push_str(&format!(
            r##"<rect x="{}" y="{}" width="{w}" height="{d}" fill="#DAD2C4" stroke="#6E655A" stroke-width="3"/>"##,
            r.x, r.y));
        let s = set_value(
            r,
            label.chars().count(),
            &obstacles,
            &set,
            &sheet,
            &core,
        );
        set.push(s.area);
        if let Some([x1, y1, x2, y2]) = s.leader {
            p.push_str(&format!(
                r##"<line x1="{x1}" y1="{y1}" x2="{x2}" y2="{y2}" stroke="#6E655A" stroke-width="2"/>"##
            ));
        }
        // Only `rotate(-90)`: across means readable bottom to top; the other way is upside down.
        let turn = if s.across {
            format!(r##" transform="rotate(-90 {} {})""##, s.x, s.y)
        } else {
            String::new()
        };
        p.push_str(&format!(
            r##"<text x="{}" y="{}"{turn} font-family="Helvetica,Arial" font-size="{}" fill="#16181A" text-anchor="{}" dominant-baseline="central">{}</text>"##,
            s.x, s.y, s.grad, s.anker, esc(label)));
        p.push_str("</g>");
    }

    // The outline goes on top. A stroke sits centred on its line, so drawn first, every piece at a
    // wall covered half the wall and seemed to stick out of the flat. Openings go above too, because
    // a wardrobe in front of a door is what someone looks at the plan for. Glass door solid, window
    // dashed, door grey: three things, three signatures.
    p.push_str(&format!(
        r##"<polygon points="{pts}" fill="none" stroke="#16181A" stroke-width="5" stroke-linejoin="miter"/>"##
    ));
    for o in &room.openings {
        let Some((a, b)) = room.opening_span(o) else {
            continue;
        };
        let (stroke, dash) = match o.kind.as_deref() {
            Some("door") => ("#5B5F63", r##" stroke-dasharray="16 10""##),
            Some("window") => ("#B0764A", r##" stroke-dasharray="4 6""##),
            _ => ("#B0764A", ""),
        };
        p.push_str(&format!(
            r##"<line x1="{}" y1="{}" x2="{}" y2="{}" stroke="{stroke}" stroke-width="9"{dash}/>"##,
            a[0], a[1], b[0], b[1]
        ));
    }

    p.push_str("</svg>");
    Ok(p)
}

/// Plan and verdict in one file, so a picture never travels without its judgement.
/// Where the positions on a sheet come from: a proposal is computed from the model, the as-is
/// state was placed by a person, and the sheet says which.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Origin {
    /// A proposal: positions in `layouts/*.toml`, every number from the model.
    Proposal,
    /// The as-is state: positions in `interior_placement`, set by a person.
    AsIs,
}

pub fn page(model: &Model, layouts: &[Layout], origin: Origin) -> Result<String, ModelError> {
    // The title comes from the model.
    let mut h = String::from(
        r##"<!doctype html><meta charset="utf-8"><title>Flat — plans</title>
<style>
:root{color-scheme:light dark}
body{margin:0;background:#FBFAF8;color:#16181A;font:16px/1.6 ui-sans-serif,-apple-system,Helvetica,Arial;padding:40px}
@media(prefers-color-scheme:dark){body{background:#131416;color:#E9E7E3}}
h1{font-size:1.8rem;margin:0 0 4px}
h2{font-size:1.15rem;margin:0}
.wrap{max-width:1180px;margin:0 auto}
.card{margin:38px 0;padding:24px;border:1px solid #C9C6C0;border-radius:14px;background:#fff}
@media(prefers-color-scheme:dark){.card{background:#1A1C1F;border-color:#3C4045}}
svg{width:100%;height:auto;display:block;margin:14px 0}
.pass{color:#2E7D32;font-weight:600}.fail{color:#B4402F;font-weight:600}
.warn{color:#9A6A00}
ul{margin:8px 0;padding-left:20px}
.meta{font:13px ui-monospace,Menlo,monospace;color:#5B5F63}
</style><div class="wrap"><h1>{TITLE} — plans</h1>
<p class="meta">Generated from the measured model. Every number comes from <code>room.toml</code>,
<code>rules.toml</code> and the inventory; {ORIGIN}</p>"##)
        .replace("{TITLE}", &esc(&model.room.flat.name))
        .replace(
            "{ORIGIN}",
            match origin {
                Origin::Proposal => "nothing on this page was placed by hand.",
                Origin::AsIs => "a person placed the positions of the as-is state; \
                     the model supplies only the room and the sizes of the pieces.",
            },
        );

    for l in layouts {
        let r = check_layout(model, l)?;
        h.push_str(&format!(
            r##"<div class="card"><h2>{}</h2><p class="{}">{}</p>"##,
            esc(&l.name),
            if r.pass { "pass" } else { "fail" },
            if r.pass {
                "passes: no hard rule broken"
            } else {
                "fails"
            }
        ));
        h.push_str(&svg(model, l)?);
        h.push_str(&format!(
            r##"<p class="meta">belegt {:.2} m² · frei {:.2} m² · {}</p>"##,
            r.metrics.occupied_area_m2,
            r.metrics.free_area_m2,
            r.metrics
                .corridors
                .iter()
                .map(|c| format!(
                    "{} → {}: {}",
                    c.from,
                    c.to,
                    c.width_cm.map_or("no path".into(), |w| format!("{w} cm"))
                ))
                .collect::<Vec<_>>()
                .join(" · ")
        ));
        if !r.hard.is_empty() {
            h.push_str("<ul>");
            for v in &r.hard {
                h.push_str(&format!(
                    r##"<li class="fail">{}: {}</li>"##,
                    v.rule,
                    esc(&v.message)
                ));
            }
            h.push_str("</ul>");
        }
        if !r.soft.is_empty() {
            h.push_str("<ul>");
            for v in &r.soft {
                h.push_str(&format!(
                    r##"<li class="warn">{}: {}</li>"##,
                    v.rule,
                    esc(&v.message)
                ));
            }
            h.push_str("</ul>");
        }
        if !r.uncertainties.is_empty() {
            h.push_str(&format!(
                r##"<p class="meta">measured? no: {}</p>"##,
                r.uncertainties
                    .iter()
                    .map(|u| format!("{} ({})", esc(&u.label), u.fields.join(", ")))
                    .collect::<Vec<_>>()
                    .join(" · ")
            ));
        }
        h.push_str("</div>");
    }
    h.push_str("</div>");
    Ok(h)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each case follows from one of `short_label`'s three rules, not from a table.
    #[test]
    fn a_short_name_comes_from_the_identifier() {
        for (identifier, expected) in [
            // Extras drop, a trailing number stays.
            ("pflanze_boden_1", "Plant 1"),
            ("bett_bestand", "Bed"),
            ("schreibtisch_gestell", "Desk"),
            ("couch_bestand", "Couch"),
            // The head word is at the end.
            ("kleiderschrank_bestand", "Wardrobe"),
            ("esstisch_midyou", "Table"),
            ("esstischstuehle", "Chair"),
            ("schreibtischplatte_bestand", "Top"),
            ("saugroboter", "Vacuum"),
            // The name beats the inherited kind: `kind_of_name` files this piece as Desk.
            ("buerostuhl", "Chair"),
            // No head word and no declared kind: the word itself.
            ("kallax_regale", "Kallax"),
            ("luftreiniger", "Luftreiniger"),
        ] {
            assert_eq!(short_label(identifier, None), expected, "{identifier}");
        }
    }

    #[test]
    fn a_declared_kind_carries_an_unknown_name() {
        assert_eq!(short_label("pinntorp", Some(Kind::Table)), "Table");
        // `other` means none of the known kinds and supplies no word.
        assert_eq!(short_label("pinntorp", Some(Kind::Other)), "Pinntorp");
        // And it does not beat a name that has one.
        assert_eq!(short_label("buerostuhl", Some(Kind::Desk)), "Chair");
    }

    fn sheet() -> Rect {
        Rect {
            x: -100,
            y: -100,
            w: 800,
            d: 800,
        }
    }

    #[test]
    fn a_label_that_fits_stays_inside() {
        let r = Rect {
            x: 0,
            y: 0,
            w: 140,
            d: 80,
        };
        let s = set_value(&r, "Bed".chars().count(), &[r], &[], &sheet(), &sheet());
        assert_eq!(
            s.grad, GRADE[0],
            "a wide rect carries the large size"
        );
        assert!(!s.across);
        assert!(s.leader.is_none(), "inside needs no leader line");
        assert_eq!((s.x, s.y), (70, 40));
    }

    /// A rect deeper than wide carries its text upright, at a smaller size if needed.
    #[test]
    fn a_narrow_deep_rect_labels_across() {
        let r = Rect {
            x: 0,
            y: 0,
            w: 45,
            d: 50,
        };
        let s = set_value(&r, "Chair".chars().count(), &[r], &[], &sheet(), &sheet());
        assert!(s.across);
        assert!(s.grad < GRADE[0]);
        assert!(width(5, s.grad) + 2 * AIR <= r.d);
        assert!(s.leader.is_none());
    }

    /// Too small for any size: the name goes outside on a leader line and covers no piece.
    #[test]
    fn a_too_small_piece_labels_outside_without_hitting_anything() {
        let r = Rect {
            x: 200,
            y: 200,
            w: 35,
            d: 35,
        };
        let neighbour = Rect {
            x: 243,
            y: 180,
            w: 90,
            d: 90,
        };
        let s = set_value(
            &r,
            "Vacuum".chars().count(),
            &[r, neighbour],
            &[],
            &sheet(),
            &sheet(),
        );
        assert!(s.leader.is_some(), "outside needs a leader line");
        assert!(!s.area.overlaps(&r));
        assert!(
            !s.area.overlaps(&neighbour),
            "the spot to the east is taken, so it must not be used"
        );
        assert!(is_free(&s.area, &[r, neighbour], &[], &sheet()));
    }

    /// Two equal pieces side by side, where the old version put the same text in the same place.
    #[test]
    fn two_equal_neighbours_get_two_places() {
        let a = Rect {
            x: 0,
            y: 300,
            w: 40,
            d: 40,
        };
        let b = Rect {
            x: 45,
            y: 300,
            w: 40,
            d: 40,
        };
        let obstacles = [a, b];
        let first_part = set_value(
            &a,
            "Plant 1".chars().count(),
            &obstacles,
            &[],
            &sheet(),
            &sheet(),
        );
        let second = set_value(
            &b,
            "Plant 2".chars().count(),
            &obstacles,
            &[first_part.area],
            &sheet(),
            &sheet(),
        );
        assert!(!first_part.area.overlaps(&second.area));
    }
}
