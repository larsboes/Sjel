//! Kommt das Stueck ueberhaupt herein und bis an seinen Platz?
//!
//! Jede andere Pruefung dieser Capability fragt, ob ein Moebel an einer Stelle **stehen** darf.
//! Keine fragte, ob es dorthin **gelangt**. Das ist keine akademische Luecke: ein Schrank, der
//! an seinem Platz jede Regel erfuellt und nicht durch die Wohnungstuer passt, ist gekauftes
//! Geld — und die Zahl, die das entscheidet, steht laengst in der Zeile (`b`, `t`) und in
//! `room.toml` (`breite` der Oeffnung). Zusammengerechnet hat sie nie jemand.
//!
//! ## Das Modell, und wo es bewusst zu streng ist
//!
//! Gesucht wird ein Weg im **Konfigurationsraum**: nicht wo das Moebel steht, sondern jede
//! Lage, die es einnehmen kann — Position und Drehung zugleich. Zwei Drehungen genuegen, denn
//! ein Rechteck sieht nach 180 Grad aus wie vorher; die Zustaende sind also
//! `(x, y, quer|laengs)`.
//!
//! Drei Vereinfachungen, alle in dieselbe Richtung, damit ein Ja belastbar ist und ein Nein
//! eine Warnung:
//!
//! 1. **Gedreht wird nur, wo der Umkreis frei ist.** Ein Mensch kippt einen Schrank auf die
//!    Ecke und dreht ihn auf der Stelle; diese Maschine verlangt den ganzen Kreis, den das
//!    Stueck beim Drehen ueberstreicht. Damit ist jedes gefundene Manoever wirklich moeglich,
//!    und ein „geht nicht" heisst „nicht ohne Kippen".
//! 2. **Hindernisse sind nur die festen Einbauten.** Am Einzugstag ist die Wohnung leer, und
//!    in welcher Reihenfolge die uebrigen Stuecke hineingehen, entscheidet nicht diese Datei.
//! 3. **Die Tuerhoehe wird nicht geprueft.** `room.toml` fuehrt Oeffnungen mit `breite` und
//!    ohne Hoehe. Eine Hoehe zu erfinden waere schlimmer als die Luecke zu nennen.
//!
//! Die Tuer selbst liegt in der Wand und damit ausserhalb des begehbaren Polygons. Der Weg
//! beginnt deshalb an der ersten Lage **innerhalb** des Raums vor der Oeffnung, und dass das
//! Stueck durch die Oeffnung passt, ist eine eigene Frage mit einer eigenen Antwort.

use crate::geometry::{Grid, RES};
use crate::model::{footprint, Layout, Model, ModelError, Rect};
use serde::Serialize;
use std::collections::VecDeque;

/// Passt das Stueck durch die Wohnungstuer?
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "art", rename_all = "snake_case")]
pub enum DoorFit {
    /// Die schmale Side passt, mit so vielen cm Luft.
    Fits { air_cm: i32, door_cm: i32 },
    /// Die schmale Side ist breiter als die Oeffnung.
    DoesNotFit { missing_cm: i32, door_cm: i32 },
    /// Passt nicht am Stueck, kommt aber zerlegt herein — die Zeile sagt es (`zerlegbar`).
    ///
    /// Der Unterschied zu `Passt` ist keine Formalie: er sagt, dass hier jemand geschraubt
    /// hat, und dass die Aussage aus einer Deklaration stammt und nicht aus der Geometrie.
    CarriedApart { missing_cm: i32, door_cm: i32 },
    /// Keine Oeffnung dieser Wohnung ist als Eingang deklariert (`eingang = true`).
    NoEntrance,
}

/// Wie das Stueck an seinen Platz kommt — oder warum nicht.
#[derive(Debug, Clone, Serialize)]
pub struct Delivery {
    pub reference: String,
    pub b: i32,
    pub t: i32,
    pub door: DoorFit,
    /// Gibt es einen Weg von der Tuer bis zum Zielplatz.
    pub affordable: bool,
    /// Wie viele Rasterschritte der kuerzeste Weg braucht, Drehungen eingerechnet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub steps: Option<usize>,
    /// Ob der Weg das Stueck mindestens einmal drehen muss.
    pub rotates: bool,
    /// Im Klartext, was fehlt — leer, wenn nichts fehlt.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Die Lagen, die ein Rechteck einnehmen kann: Zelle und Ausrichtung.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Location {
    c: i32,
    r: i32,
    across: bool,
}

/// Das Raster der Wohnung ohne die beweglichen Moebel: Polygon minus feste Einbauten.
fn empty_room(model: &Model) -> Grid {
    let mut g = Grid::new(&model.room.main_room.polygon);
    for f in &model.room.fixed_furniture {
        g.occupy(&f.rect());
    }
    g
}

/// Ist die Grundflaeche an dieser Lage vollstaendig frei?
fn frei(g: &Grid, l: Location, w: i32, d: i32) -> bool {
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

/// Darf an dieser Lage gedreht werden? Der ueberstrichene Kreis muss frei sein.
///
/// Genaehert durch sein umschriebenes Quadrat mit der Diagonalen als Kantenlaenge, um die
/// Mitte des Stuecks. Konservativ und absichtlich: was hier durchgeht, geht wirklich.
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
    frei(g, kreis, diag, diag)
}

/// Dieselbe Lage, um die eigene Mitte gedreht.
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

/// Passt das Stueck durch die Wohnungstuer?
///
/// `zerlegbar` kommt aus der Zeile des Stuecks und nicht aus einer Vermutung ueber Betten.
pub fn through_the_door(model: &Model, b: i32, t: i32, disassembles: bool) -> DoorFit {
    let Some(e) = model
        .room
        .openings
        .iter()
        .find(|o| o.entrance == Some(true))
    else {
        return DoorFit::NoEntrance;
    };
    // Die schmale Side entscheidet: durch eine Tuer geht ein Schrank hochkant.
    let narrow = b.min(t);
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

/// Kommt ein Stueck dieser Groesse von der Tuer bis auf dieses Rechteck?
pub fn path_to_place(
    model: &Model,
    b: i32,
    t: i32,
    target: Rect,
    disassembles: bool,
) -> Result<Delivery, ModelError> {
    let door = through_the_door(model, b, t, disassembles);
    let g = empty_room(model);

    let target_location = Location {
        c: target.x.div_euclid(RES),
        r: target.y.div_euclid(RES),
        across: target.w != b,
    };

    // Startlagen: alles, was unmittelbar vor der Eingangsoeffnung im Raum liegt. Welche Zelle
    // das genau ist, haengt an der Wand, in der die Tuer sitzt — deshalb wird die ganze
    // Spanne der Oeffnung als moeglicher Eintritt genommen und nicht ein gewaehlter Punkt.
    let Some(e) = model
        .room
        .openings
        .iter()
        .find(|o| o.entrance == Some(true))
    else {
        return Ok(Delivery {
            reference: String::new(),
            b,
            t,
            door,
            affordable: false,
            steps: None,
            rotates: false,
            reason: Some(
                "keine Oeffnung ist als Eingang deklariert (`eingang = true` in room.toml)".into(),
            ),
        });
    };
    let Some(seg) = crate::clearance::opening_segment(&model.room, e) else {
        return Err(ModelError::Missing(format!(
            "Oeffnung `{}` liegt in keiner bekannten Wand",
            e.id
        )));
    };

    // Der Eintritt ist der Punkt eine Rasterzelle INNERHALB der Tuermitte. Startlagen sind
    // alle Lagen, deren Grundflaeche diesen Punkt ueberdeckt — also jede Art, wie das Stueck
    // in der Tuer stehen kann. Einen einzelnen Ankerpunkt zu waehlen hiesse, eine Haltung
    // vorzuschreiben, und die falsche Haltung waere ein Nein, das nur vom Raster kommt.
    let ex = ((seg.a[0] + seg.b[0]) as f64 / 2.0 + seg.normal[0] * RES as f64) as i32;
    let ey = ((seg.a[1] + seg.b[1]) as f64 / 2.0 + seg.normal[1] * RES as f64) as i32;
    let (ec, er) = (ex.div_euclid(RES), ey.div_euclid(RES));

    let mut start: Vec<Location> = Vec::new();
    for across in [false, true] {
        let (bw, bd) = if across { (t, b) } else { (b, t) };
        let (cw, cd) = (
            (bw as f64 / RES as f64).ceil() as i32,
            (bd as f64 / RES as f64).ceil() as i32,
        );
        for r in (er - cd + 1)..=er {
            for c in (ec - cw + 1)..=ec {
                let l = Location { c, r, across };
                if frei(&g, l, b, t) {
                    start.push(l);
                }
            }
        }
    }

    if !frei(&g, target_location, b, t) {
        return Ok(Delivery {
            reference: String::new(),
            b,
            t,
            door,
            affordable: false,
            steps: None,
            rotates: false,
            reason: Some(
                "der Zielplatz selbst ist im leeren Raum nicht frei — ein fester Einbau steht dort"
                    .into(),
            ),
        });
    }
    if start.is_empty() {
        return Ok(Delivery {
            reference: String::new(),
            b,
            t,
            door,
            affordable: false,
            steps: None,
            rotates: false,
            reason: Some(
                "direkt hinter der Tuer ist keine Lage frei, in der das Stueck steht".into(),
            ),
        });
    }

    // Breitensuche: vier Nachbarzellen plus die Drehung auf der Stelle. Alle Schritte kosten
    // gleich viel, also findet die erste Ankunft den kuerzesten Weg.
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
                b,
                t,
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
            if frei(&g, n2, b, t) && seen.insert(n2) {
                queue.push_back((n2, n + 1, rotates));
            }
        }
        if may_rotate(&g, l, b, t) {
            let n2 = rotated(l, b, t);
            if frei(&g, n2, b, t) && seen.insert(n2) {
                queue.push_back((n2, n + 1, true));
            }
        }
    }

    Ok(Delivery {
        reference: String::new(),
        b,
        t,
        door,
        affordable: false,
        steps: None,
        rotates: false,
        reason: Some(
            "kein Weg von der Tuer bis zum Platz, ohne das Stueck zu kippen oder einen Einbau zu \
             beruehren"
                .into(),
        ),
    })
}

/// Dieselbe Frage fuer ein Stueck, das in einem Layout schon einen Platz hat.
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
                "`{reference}` steht nicht in \"{}\" — nichts einzubringen",
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
