//! Positionssuche: haelt ein Layout fest und rastert einzelne Moebel durch, bis eine
//! Kombination ohne harten Verstoss dasteht.
//!
//! Warum das ein Modul ist und kein Skript: die Frage "wohin passt X noch" kam am 2026-08-30
//! viermal, und viermal wurde dafuer ein Wegwerf-Skript geschrieben, ausgefuehrt und geloescht.
//! Jedes hatte leicht andere Annahmen darueber, was eine Sperrflaeche ist, und das Ergebnis des
//! letzten ging beim Sessionende verloren.
//!
//! Zwei Dinge machen die Suche praktikabel:
//!   * Ein billiger Vorfilter vor der vollen Pruefung. Rechteck-Ueberlappung kostet
//!     Nanosekunden, eine Raeumungspruefung 0,18 ms — der Filter entscheidet ueber Sekunden
//!     gegen Minuten.
//!   * Parallelitaet ueber die Kandidaten. Sie sind voneinander unabhaengig; das ist der
//!     Grund, warum hier rayon steht und keine DataFrame-Bibliothek.

use crate::clearance::{check_layout, CheckResult};
use crate::geometry::{point_in_polygon, RES};
use crate::model::{footprint, Layout, Model, ModelError, PlacedItem, Rect, Side};
use rayon::prelude::*;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct Spec {
    /// Ids, die bewegt werden duerfen. Alles andere im Layout bleibt, wo es steht.
    pub move_refs: Vec<String>,
    /// Rasterschritt in cm. Grob suchen, dann verfeinern.
    pub step: i32,
    /// Optional je Id: erlaubtes Band [x0, x1, y0, y1].
    pub bands: BTreeMap<String, [i32; 4]>,
    /// Abbruch nach so vielen Treffern. 0 = alle.
    pub limit: usize,
}

/// Wie viele Ziele eine Aufstellung gleichzeitig verfolgt.
///
/// Vier, und keines von ihnen ist der Sieger: mehr Wand im Ruecken kostet Laufbreite, mehr
/// Reserve kostet Wand. Eine einzelne Zahl aus vieren zu machen hiesse, drei Gewichte zu
/// erfinden, die niemand gemessen hat.
pub const TARGETS: usize = 4;

/// Welche Kandidaten unter KEINER Gewichtung der Ziele die besten sein koennen.
///
/// Ein Punkt ist dominiert, wenn ein anderer in jedem Ziel mindestens so gut und in einem
/// echt besser ist. Solche Punkte koennen nie gewinnen, egal wie man gewichtet — sie hinter
/// die Front zu sortieren ist deshalb keine Meinung ueber Geschmack, sondern Arithmetik.
///
/// Alle Ziele zeigen in dieselbe Richtung: **groesser ist besser**. Wer eine Zahl beitraegt,
/// bei der weniger besser ist, uebergibt sie negiert — `soft` tut das.
///
/// Die Front wird ueber die VERSCHIEDENEN Zielwerten gebildet, nicht ueber die Kandidaten:
/// zehntausend Aufstellungen fallen typisch auf einige hundert verschiedene Vierlinge
/// zusammen, und der quadratische Vergleich laeuft dann auf denen. Ohne das waere die Front
/// bei einer erschoepfenden Suche teurer als die Suche.
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
    /// Engste der geprueften Routen in cm — hoeher ist besser.
    pub bottleneck_cm: i32,
    /// Die knappste harte Messung dieser Aufstellung in cm: um so viel besteht sie.
    ///
    /// Zwei Treffer koennen beide bestehen und dabei 2 cm und 30 cm Luft haben. Bis diese
    /// Zahl hier stand, waren sie in jedem Bericht dasselbe Wort.
    pub tightest_reserve_cm: Option<i32>,
    /// Auf der Pareto-Front: unter keiner Gewichtung der Ziele schlechter als ein anderer.
    pub pareto: bool,
    /// Wie viele Zentimeter der bewegten Moebel an einer Wand anliegen.
    ///
    /// Der Raeumungspruefer kennt nur Abstaende, nicht Sinn: er haelt einen Esstisch mitten
    /// im Raum fuer genauso richtig wie einen an der Wand, solange die Laufwege stimmen. Die
    /// ersten Suchlaeufe am 2026-08-30 lieferten deshalb lauter formal fehlerfreie Layouts,
    /// die niemand so einrichten wuerde. Diese Zahl ist die fehlende Haelfte des Urteils.
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

/// Zentimeter der Rechteckkanten, die unmittelbar an einer Wand liegen.
///
/// Geprueft wird knapp ausserhalb der Kante: liegt der Punkt nicht mehr im Wohnpolygon, ist
/// dort eine Wand. Freistehende Moebel bekommen 0 und sinken damit in der Rangfolge — nicht
/// weil sie regelwidrig waeren, sondern weil sie schlechter sind.
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

/// Sagt dieses Stueck selbst, dass es frei stehen soll?
///
/// Die Wandsumme belohnt eine Wand im Ruecken, und ein Raumtrenner hat per Definition keine.
/// Bis 2026-08-31 sank er deshalb in der Rangfolge — bestraft dafuer, dass er seine Aufgabe
/// erfuellt. Er zaehlt jetzt gar nicht mit, statt mit 0 cm zu zaehlen: ein Regal quer im Raum
/// soll die Bewertung der uebrigen Stuecke weder heben noch senken.
///
/// Freiwillig wie jedes Feld aus PRD Q61. Ein Stueck ohne die Angabe wird weiter an der Wand
/// gemessen, also aendert sich kein bestehender Rang, bis jemand eine Zeile fuellt.
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

/// Sperrflaechen aus dem Modell, nicht aus dem Kopf. Die Wegwerf-Skripte haben genau diese
/// Rechtecke jedes Mal neu abgetippt — hier stammen sie aus room.toml und rules.toml.
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

/// Die vier Ziele eines Treffers, alle so gedreht, dass groesser besser ist.
///
/// Eine fehlende Reserve zaehlt als `i32::MIN`, damit eine Aufstellung, an der nichts gemessen
/// wurde, nicht durch Schweigen die Front gewinnt.
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
                        "`{r}` steht nicht in \"{}\" — nichts zu bewegen",
                        base.name
                    ))
                })
        })
        .collect::<Result<_, _>>()?;

    let block = blockers(model, &fixed)?;

    // Alle geometrisch zulaessigen Kombinationen sammeln, bevor irgendetwas geprueft wird.
    let mut combos: Vec<Vec<(String, [i32; 2], Rect)>> = vec![Vec::new()];
    for m in &movers {
        let (w, d, _) = footprint(m, &model.catalogue)?;
        // Ohne Band die Ausdehnung des Wohnpolygons, NICHT eine gemerkte Zahl. Hier stand
        // [0, 420, 0, 590] — die Masse dieser einen Wohnung, fest im Code einer Bibliothek,
        // die keine Wohnung kennen darf.
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

    // Die volle Pruefung parallel. Jeder Kandidat ist unabhaengig; das Modell wird nur gelesen.
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
                name: format!("{} (Suche)", base.name),
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

    // Die Front zuerst, und das ist keine Geschmacksfrage: ein dominierter Treffer ist unter
    // JEDER Gewichtung der vier Ziele schlechter als ein bestimmter anderer, also kann er die
    // Antwort nicht sein. Innerhalb der Front gilt weiter die Rangfolge von 2026-08-30 — erst
    // keine Warnungen, dann Wand im Ruecken, dann der breiteste Engpass —, weil eine Front
    // ungeordnet ist und eine Liste trotzdem eine Reihenfolge braucht.
    let targets: Vec<[i32; TARGETS]> = hits.iter().map(hit_targets).collect();
    for (h, front) in hits.iter_mut().zip(pareto_front(&targets)) {
        h.pareto = front;
    }
    // Die Reserve steht als LETZTE Stufe und nicht weiter vorn, und das ist der Unterschied
    // zwischen einer neuen Rangfolge und einer widerspruchsfreien. Genau so wird die Kette zu
    // einer lexikographischen Ordnung ueber alle vier Ziele — und damit kann kein Treffer mehr
    // vor einem stehen, der ihn in allem schlaegt. Ein Test haelt das fest; ohne diese Stufe
    // fand er zwei Aufstellungen, die sich nur in einem Zentimeter Reserve unterschieden und
    // in der falschen Reihenfolge standen.
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

/// Die Flaeche, die ein Stueck DAUERHAFT belegt — Grundflaeche plus die Zone, die es selbst
/// verlangt und die nicht wieder freigeraeumt wird.
///
/// Nur Angaben mit einer RICHTUNG: `expands = {dir, to}` und `opens` mit `open_clear`. Die
/// Zahlen stammen aus der Deklaration des Stuecks, hier wird keine Regel nachgebaut — der
/// Unterschied ist, dass `check_layout` prueft, ob die Flaeche frei IST, und dieser Platzierer
/// sie von vornherein mitreserviert.
///
/// `access_sides` bleibt draussen, weil es keine Side nennt: welche Laengsseite eines Bettes
/// begehbar sein soll, entscheidet erst die Aufstellung. Das faengt die Schlusspruefung.
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
        let jetzt = match side {
            Side::North | Side::South => r.d,
            Side::East | Side::West => r.w,
        };
        grow(side, to - jetzt);
    }
    if let (Some(dir), Some(frei)) = (it.opens, it.open_clear) {
        grow(dir.rotated(rot), frei);
    }
    out
}

/// Wo die linke obere Ecke eines Stuecks liegen DARF, als Lauflaengen je Rasterzeile.
///
/// Die Frage, die eine Oberflaeche beim Ziehen stellt: harte Kanten, damit ein Moebel gar nicht
/// erst durch eine Wand geschoben werden kann, statt es hinterher als Verstoss zu melden.
///
/// **Warum das hier steht und nicht im Browser.** Der Hauptraum ist ein Sechseck, kein Rechteck
/// — an einer Ecke liegt das Bad. Auf ein umschliessendes Rechteck zu klemmen wuerde ein Bett in
/// der Kerbe abstellen, die nicht zum Raum gehoert. Richtig klemmen heisst `point_in_polygon`,
/// und eine zweite Fassung davon in JavaScript waere die Doppelung, gegen die diese Capability
/// existiert. Die Oberflaeche bekommt deshalb eine LISTE und rechnet nichts.
///
/// Hart sind Waende und was sich nicht bewegen laesst: feste Einbauten und Tuerschwenkbereiche,
/// dieselben `blockers`, gegen die auch die Suche prueft. **Andere Moebel sind NICHT hart** —
/// beim Umstellen muss man ein Stueck an einem anderen vorbeifuehren koennen, und eine
/// Ueberlappung meldet `kollision` ohnehin sichtbar. Eine Kante, die das Umstellen unmoeglich
/// macht, waere schlimmer als der Verstoss, den sie verhindert.
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
            ModelError::Missing(format!("`{reference}` steht nicht in \"{}\"", base.name))
        })?;
    // Die Grundflaeche fuer die GEFRAGTE Drehung, nicht fuer die aktuelle.
    let rotated = PlacedItem { rot, ..it.clone() };
    let (w, d, _) = footprint(&rotated, &model.catalogue)?;

    // Was sich NICHT bewegen laesst: feste Einbauten und Tuerschwenkbereiche. Beides steht in
    // room.toml, beides wird hier gelesen statt abgetippt.
    let mut sperr: Vec<Rect> = model.room.fixed_furniture.iter().map(|f| f.rect()).collect();
    sperr.extend(
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
            let frei = inside_room(model, &r) && !sperr.iter().any(|b| r.overlaps(b));
            match (frei, run) {
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
    /// Erlaubte x-Bereiche, jeweils [von, bis] einschliesslich.
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

/// Was `compose` stellen soll und wie gruendlich.
#[derive(Debug, Clone)]
pub struct ComposeSpec {
    /// Die Stuecke, in der Reihenfolge, in der sie gestellt werden. Grosses zuerst.
    pub refs: Vec<String>,
    /// Rasterschritt in cm. Grob suchen, dann von Hand verfeinern.
    pub step: i32,
    /// Wie viele Teilaufstellungen je Runde ueberleben.
    pub beam: usize,
    /// Erlaubte Drehungen je Stueck.
    pub rotations: Vec<i32>,
    /// Wie viele fertige Aufstellungen zurueckkommen.
    pub limit: usize,
}

/// Eine fertig gestellte Wohnung und ihr Verdikt.
#[derive(Debug, Clone, Serialize)]
pub struct Composed {
    pub places: Vec<(String, [i32; 2], i32)>,
    pub pass: bool,
    pub hard: Vec<String>,
    pub soft: Vec<String>,
    pub wall_contact_cm: i32,
    pub bottleneck_cm: i32,
    pub free_m2: f64,
    /// Die knappste harte Messung dieser Aufstellung in cm: um so viel besteht sie.
    pub tightest_reserve_cm: Option<i32>,
    /// Auf der Pareto-Front der vier Ziele.
    pub pareto: bool,
}

/// Eine Wohnung von Grund auf stellen, statt eine bestehende Aufstellung zu verschieben.
///
/// `search` bewegt eine Handvoll Stuecke in einem Layout, das schon existiert. Das reicht nicht,
/// um zu fragen *wie sollte diese Wohnung ueberhaupt aussehen* — und die Antwort ist auch nicht
/// dieselbe Funktion mit mehr Stuecken: `search` sammelt das volle kartesische Produkt, bevor es
/// irgendetwas prueft. Sechs Stuecke waeren rund 10^15 Kombinationen. Nicht langsam, unmoeglich.
///
/// Deshalb eine **Strahlsuche**: Stueck fuer Stueck, und nach jeder Runde ueberleben nur die
/// besten `beam` Teilaufstellungen. Das ist der Preis, und er wird hier genannt statt verschwiegen
/// — eine Strahlsuche findet nicht garantiert das Optimum, weil sie eine Teilaufstellung
/// verwerfen kann, die erst durch das letzte Stueck gut geworden waere. Was sie liefert, ist
/// gerechnet und reproduzierbar, nicht geraten.
///
/// **Geprueft wird erst am Ende.** Waehrend des Stellens zaehlt nur Geometrie: im Raum, kein
/// Ueberlapp, moeglichst viel Wand im Ruecken. Die volle Raeumungspruefung auf eine halbe
/// Wohnung anzuwenden waere sinnlos — ohne Moebel ist jeder Laufweg frei, und eine Regel, die
/// auf einer Teilaufstellung besteht, sagt nichts ueber die ganze.
pub fn compose(model: &Model, spec: &ComposeSpec) -> Result<Vec<Composed>, ModelError> {
    // Die vollen harten Zonen, nicht nur die Waende: eine Position in der Anlaufzone der
    // Terrassentuer ist nie gueltig, und ein Platzierer, der sie erst am Ende verwirft, hat den
    // ganzen Strahl mit Kandidaten gefuellt, die nicht bestehen koennen. Genau das ist beim
    // ersten Lauf passiert — alle drei Vorschlaege scheiterten an R1 und R7.
    let sperr = crate::clearance::hard_zones(&model.room, &model.rules)?;

    // Zulaessige Plaetze je (Stueck, Drehung), einmal gegen Waende und feste Einbauten gerechnet.
    // Was gegen die Nachbarn kollidiert, faellt beim Erweitern weg — das haengt an der
    // Teilaufstellung und laesst sich nicht vorher wissen.
    type Place = ([i32; 2], i32, Rect);
    let mut plaetze: BTreeMap<String, Vec<Place>> = BTreeMap::new();
    for r in &spec.refs {
        let mut alle = Vec::new();
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
                    // Der Platz muss fuer die DAUERFLAECHE reichen, nicht nur fuer das Moebel:
                    // die Klappe des Esstischs und die Schranktuer sind dauerhaft freizuhalten,
                    // also darf dort von vornherein nichts hin.
                    let occupied = lasting_area(model, r, *rot, rect);
                    if inside_room(model, &rect) && !sperr.iter().any(|b| occupied.overlaps(b)) {
                        alle.push(([x, y], *rot, occupied));
                    }
                    x += spec.step;
                }
                y += spec.step;
            }
        }
        if alle.is_empty() {
            return Err(ModelError::Missing(format!(
                "fuer `{r}` gibt es bei Schritt {} cm keinen Platz im Raum",
                spec.step
            )));
        }
        plaetze.insert(r.clone(), alle);
    }

    // Runde fuer Runde erweitern, dann auf die besten `beam` kuerzen.
    let mut strahl: Vec<Vec<Place>> = vec![Vec::new()];
    for r in &spec.refs {
        let options = &plaetze[r];
        let mut nearest: Vec<(i32, Vec<Place>)> = strahl
            .par_iter()
            .flat_map(|teil| {
                options
                    .par_iter()
                    .filter(|(_, _, rect)| !teil.iter().any(|(_, _, q)| rect.overlaps(q)))
                    .map(|p| {
                        let mut c = teil.clone();
                        c.push(*p);
                        // Wand im Ruecken, ausser das Stueck sagt, es solle frei stehen.
                        // Wandkontakt am MOEBEL, nicht an seiner reservierten Zone — sonst
                        // zaehlte die Klappe eines Tisches als Rueckenlehne an der Wand.
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
                "nach `{r}` bleibt keine Aufstellung uebrig — Schritt zu grob oder zu viele Stuecke"
            )));
        }
        // Vielfalt vor Menge: je Position des GERADE gestellten Stuecks ueberlebt nur die beste
        // Teilaufstellung. Ohne das fuellt sich der Strahl mit Varianten derselben Idee — der
        // erste Lauf lieferte drei Vorschlaege, die sich in einer einzigen Koordinate
        // unterschieden, und verwarf dafuer jede andere Anordnung.
        nearest.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
        let mut seen: std::collections::HashSet<[i32; 2]> = std::collections::HashSet::new();
        nearest.retain(|(_, c)| {
            c.last()
                .map(|(pos, _, _)| seen.insert(*pos))
                .unwrap_or(false)
        });
        nearest.truncate(spec.beam);
        strahl = nearest.into_iter().map(|(_, c)| c).collect();
    }

    // Erst jetzt die volle Pruefung, auf ganzen Wohnungen.
    let mut done: Vec<Composed> = strahl
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

    // Dieselbe Rangfolge wie `search`: bestehen, wenige Warnungen, Wand im Ruecken, dann der
    // breiteste Engpass. Die dritte Stufe ist nicht schmueckend — mit Engpass davor gewann eine
    // Aufstellung mit 0 cm Wandkontakt, also jedes Stueck frei im Raum. Formal fehlerfrei, und
    // niemand richtet so ein. Zwei Rangfolgen fuer dieselbe Frage waeren ausserdem genau die
    // Doppelung, gegen die diese Capability existiert.
    // Die Front wird nur unter den BESTEHENDEN gebildet. Eine durchgefallene Aufstellung mit
    // viel Wandkontakt waere sonst nicht dominiert und stuende vorn — auf einer Liste, deren
    // erste Frage ist, ob man so einziehen kann.
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

    /// Ein Punkt, der in allem schlechter ist, gehoert nicht auf die Front.
    #[test]
    fn a_point_worse_in_everything_drops_out() {
        let p: Vec<[i32; TARGETS]> = vec![[10, 10, 10, 0], [9, 9, 9, -1]];
        assert_eq!(pareto_front(&p), vec![true, false]);
    }

    /// Ein Tausch ist keine Niederlage: wer ein Ziel aufgibt und ein anderes gewinnt, bleibt.
    #[test]
    fn a_trade_off_keeps_both_on_the_front() {
        let p: Vec<[i32; TARGETS]> = vec![[10, 0, 5, 0], [0, 10, 5, 0]];
        assert_eq!(pareto_front(&p), vec![true, true]);
    }

    /// Gleiche Punkte dominieren einander nicht — sonst faengt die Front an, Duplikate zu
    /// verwerfen, und zwei gleich gute Aufstellungen an verschiedenen Orten waeren eine.
    #[test]
    fn identical_points_both_stay() {
        let p: Vec<[i32; TARGETS]> = vec![[3, 3, 3, 3], [3, 3, 3, 3]];
        assert_eq!(pareto_front(&p), vec![true, true]);
    }

    /// Gleich in drei Zielen, besser im vierten: das ist Dominanz und kein Gleichstand.
    #[test]
    fn one_better_target_is_enough_to_dominate() {
        let p: Vec<[i32; TARGETS]> = vec![[5, 5, 5, 5], [5, 5, 5, 6]];
        assert_eq!(pareto_front(&p), vec![false, true]);
    }

    /// Die Front eines leeren Feldes ist leer und kein Absturz.
    #[test]
    fn empty_stays_empty() {
        assert!(pareto_front(&[]).is_empty());
    }
}
