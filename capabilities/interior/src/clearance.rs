//! Beurteilt ein Layout gegen `rules.toml`.
//!
//! Jede Schwelle kommt aus dem Modell. Nichts in dieser Datei entscheidet, wie viel Platz ein
//! Laufweg braucht — sie entscheidet nur, ob das Layout ihn hat.
//!
//! Schwere folgt `rules.toml`: `hart` blockiert, `weich` warnt. Ein Layout besteht nur, wenn
//! keine harte Regel verletzt ist. Es gibt absichtlich keinen Weg, ein Bestehen zu melden,
//! solange ein harter Verstoss offen ist — ein Pruefer, der weich durchwinkt, ist schlechter
//! als gar keiner.
//!
//! Der Satz oben stimmt seit 2026-08-31, und vorher war er eine Behauptung ueber Code, den
//! niemand nachgelesen hatte: die Schwere stand an allen 21 Ausgabestellen als Literal, und
//! `Regel { id, schwere, text }` wurde aus `rules.toml` geparst und von **nichts** gelesen.
//! Eine Wohnung konnte R3 auf `hart` setzen und bekam weiterhin eine Warnung. In genau der
//! Capability, die es gegen zwei Fassungen derselben Zahl gibt.
//!
//! ## Zwei Klassen von Kennungen, und die Grenze dazwischen
//!
//! `REGEL_IDS` sind **Hausregeln**: die Wohnung deklariert sie in `rules.toml` mit Schwere und
//! Text, und dieser Pruefer schlaegt beides ueber `rules.rule(id)` nach. Fehlt die Kennung
//! dort, ist das ein Fehler und kein textloser Verstoss.
//!
//! Die uebrigen Kennungen — `kollision`, `raumgrenze`, `zugang`, `laufweg` und die anderen —
//! sind **keine Hausregeln**, sondern Geometrie und Nutzbarkeit. Zwei Moebel koennen sich
//! nicht ueberlappen, und keine Wohnung kann das erlauben. Sie deklarieren zu lassen hiesse,
//! jede Wohnung eine Invariante wiederholen zu lassen, die sie nicht aendern kann, und die
//! erste vergessene waere eine still abgeschaltete Pruefung.
//!
//! ## Eine Funktion je Regel
//!
//! [`check_layout`] entscheidet nichts. Es baut die `Pruefung` — Raum, Regeln, aufgestellte
//! Stuecke, belegtes Gitter — und ruft jede Regel einmal damit auf; alle schreiben in dieselben
//! `Befunde`. Bis 2026-09-05 stand alles davon in einer Funktion von 1008 Zeilen, in der die
//! Regeln nur durch Bannerkommentare getrennt waren.
//!
//! Zwei Eigenschaften haengen daran und sind jetzt vom Compiler gehalten statt von der
//! Aufmerksamkeit des Lesers: keine Regel kann die Grundlage einer anderen veraendern, weil sie
//! sie nur geliehen bekommt; und keine liest, was eine andere geschrieben hat.
//! Die Reihenfolge im Bericht ist weiterhin die Aufrufreihenfolge in [`check_layout`].

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

/// Die Hausregeln, die diese Maschine prueft.
///
/// Der Abgleich laeuft in beide Richtungen und beide sind noetig. Was hier steht, MUSS die
/// Wohnung deklarieren, sonst bricht die Pruefung ab — eine Kennung ohne Text ist ein Verstoss,
/// den niemand nachschlagen kann. Was die Wohnung deklariert und hier fehlt, meldet
/// `CheckResult::nicht_geprueft` als Luecke, statt es fallen zu lassen.
pub const RULE_IDS: &[&str] = &["R1", "R2", "R3", "R4", "R5", "R6", "R7", "R8"];

/// Hausregeln, die nur laufen, wenn die Wohnung sie **fuehrt**.
///
/// Der Unterschied zu `REGEL_IDS` ist nicht Bequemlichkeit, sondern eine Aussage darueber,
/// wem die Regel gehoert. R1 bis R8 muss jede Wohnung beantworten — sie beschreiben Fragen,
/// die in jeder Wohnung auftreten. R9 verlangt einen Standort, eine Nordrichtung und die
/// Hoehen der Verglasung; eine Wohnung, die davon nichts weiss, soll nicht abbrechen, sondern
/// schweigen.
///
/// Wird sie gefuehrt und fehlen die Angaben, ist das kein Schweigen mehr, sondern eine Luecke:
/// dann steht sie in `nicht_geprueft` mit dem Grund. Genau die Unterscheidung, die B31 fuer
/// R5 und R6 eingefuehrt hat.
pub const OPTIONAL_RULE_IDS: &[&str] = &["R9"];

#[derive(Debug, Clone, Serialize)]
pub struct Violation {
    pub rule: String,
    pub severity: Severity,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item: Option<String>,
    pub message: String,
    /// Der Regeltext aus `rules.toml`, wo es einen gibt.
    ///
    /// Nur Hausregeln haben einen. `message` sagt, was dieses Layout falsch macht; `text` sagt,
    /// welche Regel das ueberhaupt zu einer Regel macht — und der stand bis 2026-08-31 in einer
    /// Datei, die kein Bericht las.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub measured: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required: Option<i32>,
}

/// Ein Verstoss gegen eine deklarierte Hausregel: Schwere und Text kommen aus der Wohnung.
fn house_rule(
    rules: &Rules,
    id: &str,
    item: Option<String>,
    message: String,
    measured: Option<i32>,
    required: Option<i32>,
) -> Result<Violation, ModelError> {
    let regel = rules.rule(id)?;
    Ok(Violation {
        rule: regel.id.clone(),
        severity: regel.level()?,
        item,
        message,
        text: Some(regel.text.clone()),
        measured,
        required,
    })
}

/// Jede Flaeche, die eine HARTE Regel dauerhaft freihaelt.
///
/// Waende sind Geometrie und stehen woanders; das hier sind die Zonen, die aus Regeln folgen:
/// die Anlaufzone vor einer Oeffnung (R1), der Tuerschwenkbereich (R2), der feste Einbau selbst
/// und die Zone, die er sich ausbedingt (R7).
///
/// Steht hier und nicht in `search.rs`, weil es dieselben Rechtecke sind, die `check_layout`
/// prueft. Ein Platzierer, der sie neu ableitet, waere die zweite Fassung, die genau bis zur
/// ersten Regelaenderung mit der ersten uebereinstimmt.
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
            out.push(anlauf_rect(&f.rect(), z.side, rules.threshold(&z.threshold)?));
        }
    }
    Ok(out)
}

/// In welche Richtung eine Schwelle wirkt.
///
/// Ohne diese Unterscheidung haette die Reserve ein Vorzeichen, das von der Regel abhaengt und
/// nirgends steht: R3 begrenzt eine Hoehe nach OBEN, jede Abstandsregel eine Tiefe nach UNTEN,
/// und `measured - required` ist genau bei einer der beiden falsch herum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Limit {
    /// Der gemessene Wert muss mindestens so gross sein wie der geforderte.
    AtLeast,
    /// Der gemessene Wert darf hoechstens so gross sein wie der geforderte.
    AtMost,
}

/// Wie viel Luft eine Pruefung uebrig laesst — **auch dann, wenn sie besteht**.
///
/// Ein Verdikt ist ein Bit, und ein Bit beantwortet die Frage nicht, die beim Moebelkauf
/// wirklich ansteht: besteht dieses Layout mit 2 cm oder mit 40? Beide melden `pass`, und nur
/// eines ueberlebt ein Bandmass, das sich um einen Zentimeter geirrt hat.
///
/// Jede Stelle, die eine Zahl gegen eine Schwelle haelt, legt hier eine Zeile ab, ob sie
/// ausloest oder nicht. Der Verstoss bleibt, was er war; die Reserve ist das, was vorher
/// verworfen wurde, sobald der Vergleich `true` ergab.
#[derive(Debug, Clone, Serialize)]
pub struct Reserve {
    pub rule: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item: Option<String>,
    /// Der Gegenstand des Vergleichs, wo es einen gibt: die Oeffnung, der Einbau, die Route.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bezug: Option<String>,
    pub measured: i32,
    pub required: i32,
    pub limit: Limit,
    /// Was uebrig ist. **Negativ heisst: um so viel verfehlt** — dieselbe Zahl, die der
    /// Verstoss daneben in Worten sagt.
    pub slack: i32,
    /// `cm`, `seiten`, `plaetze`. Nur `cm` geht in `engste_reserve_cm` ein: das Minimum aus
    /// Zentimetern und Sitzplaetzen waere eine Zahl ohne Einheit.
    pub unit: &'static str,
    /// Die Messung hat ihren Horizont erreicht: es ist **mindestens** so viel, nicht genau.
    ///
    /// `free_depth_on_side` hoert bei `RESERVE_HORIZONT` ueber der Schwelle auf zu zaehlen.
    /// Ohne dieses Flag stuende in der Oberflaeche eine gemessene Zahl, wo eine untere Schranke
    /// gemeint ist — und das ist genau die Sorte stiller Praezision, gegen die diese
    /// Capability existiert.
    pub capped: bool,
    /// Waere eine Unterschreitung hart? Folgt bei Hausregeln der Wohnung, nicht dem Code.
    pub hart: bool,
    /// Sagt ein POSITIVER Wert hier etwas darueber, wie viel Luft noch ist?
    ///
    /// Fuer fast jede Regel ja. Fuer die Raumgrenze nein, und das ist keine Feinheit: ein
    /// Schrank an der Wand hat einen Zentimeter Abstand zu ihr, weil er dort **hingehoert**.
    /// Zaehlte das mit, meldete jede vernuenftig eingerichtete Wohnung „1 cm Reserve", und die
    /// Zahl, die sagen soll, wie knapp ein Layout besteht, saehe in jedem Layout gleich aus.
    /// Genau das ist am 2026-08-31 an der echten Wohnung passiert, eine Stunde nachdem die
    /// Messung dazukam.
    ///
    /// Ein NEGATIVER Wert zaehlt immer: herausragen ist ein Verstoss, egal welche Regel.
    pub binding: bool,
}

/// Haelt eine Messung fest, ob sie nun ausloest oder nicht.
#[allow(clippy::too_many_arguments)]
fn remember(
    out: &mut Vec<Reserve>,
    rule: &str,
    item: Option<&str>,
    bezug: Option<&str>,
    measured: i32,
    required: i32,
    limit: Limit,
    unit: &'static str,
    hart: bool,
) {
    remember_up_to(
        out, rule, item, bezug, measured, required, limit, unit, hart, None, true,
    )
}

/// Eine Messung, deren positiver Wert KEINE Reserve ist — siehe `Reserve::bindend`.
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

/// Wie `merken`, aber die Messung kennt einen Horizont und sagt, wenn sie ihn erreicht hat.
#[allow(clippy::too_many_arguments)]
fn remember_up_to(
    out: &mut Vec<Reserve>,
    rule: &str,
    item: Option<&str>,
    bezug: Option<&str>,
    measured: i32,
    required: i32,
    limit: Limit,
    unit: &'static str,
    hart: bool,
    deckel: Option<i32>,
    binding: bool,
) {
    let slack = match limit {
        Limit::AtLeast => measured - required,
        Limit::AtMost => required - measured,
    };
    out.push(Reserve {
        rule: rule.to_string(),
        item: item.map(str::to_string),
        bezug: bezug.map(str::to_string),
        measured,
        required,
        limit,
        slack,
        unit,
        hart,
        capped: deckel == Some(measured),
        binding,
    });
}

/// Bis wohin eine Seitentiefe gemessen wird, wenn ihre Reserve eine Zahl tragen soll.
fn horizon(want: i32) -> i32 {
    want + crate::geometry::RESERVE_HORIZON
}

/// Ob eine Hausregel hart ist — aus der Wohnung, nicht aus dem Code.
fn ist_hart(rules: &Rules, id: &str) -> Result<bool, ModelError> {
    Ok(rules.rule(id)?.level()? == Severity::Hard)
}

/// Legt einen Verstoss in `hart` oder `weich`, je nach seiner eigenen Schwere.
///
/// Die Sortierung folgt jetzt der Datei, also darf sie nicht mehr an der Aufrufstelle stehen:
/// `hard.push(..)` mit einem weichen Verstoss darin waere genau die Divergenz, die diese
/// Aenderung schliesst.
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
    /// Stuecke in diesem Layout, gegen die schon entschieden ist.
    ///
    /// Abgeleitet aus `prioritaet = "verworfen"` im Inventar, nicht am Layout deklariert: die
    /// Entscheidung faellt am Moebel und soll nicht in jeder Datei wiederholt werden, die es
    /// aufstellt. Zwei Layouts stellten am 2026-08-31 einen Tisch auf, der seit dem 30. verworfen
    /// war — eines davon war eines der vier durchgefallenen, und es fiel an genau diesem Tisch.
    /// Ein Verdikt ueber Moebel, die nicht kommen, beantwortet keine Frage, die noch offen ist.
    pub outdated: Vec<String>,
    /// Regeln, die diese Wohnung deklariert und diese Maschine nicht prueft.
    ///
    /// Gehoert in das Ergebnis und nicht in ein Log, weil es das Verdikt qualifiziert:
    /// „bestanden" heisst ab hier „bestanden, gemessen an den Regeln, die gemessen wurden".
    /// Die reale Wohnung fuehrt zwei davon (R5, R6), und vor 2026-08-31 sagte das nichts.
    pub unchecked: Vec<UncheckedRule>,
    /// Jede Messung mit ihrer Schwelle, bestanden oder nicht.
    pub reserves: Vec<Reserve>,
    /// Die knappste harte Messung in cm. **Das ist die Zahl, um die dieses Layout besteht.**
    ///
    /// `None`, wenn keine harte Regel in Zentimetern gemessen hat — nicht `0`, weil das
    /// „knapp bestanden" hiesse, wo „nichts gemessen" gemeint ist. Negativ, wenn ein harter
    /// Verstoss offen ist: dann sagt sie, um wie viel.
    pub tightest_reserve_cm: Option<i32>,
}

/// Eine Regel, die auf dieses Layout nicht angewendet wurde, und warum nicht.
///
/// Zwei Gruende, gleiche Folge. **Nicht implementiert:** die Wohnung fuehrt die Regel, diese
/// Maschine kennt sie nicht. **Nicht anwendbar:** die Maschine kennt sie, aber es fehlt eine
/// Angabe, ohne die sie nichts messen kann.
///
/// Der zweite Fall ist der gefaehrlichere, weil er wie Bestehen aussieht. `kleiderschrank_bestand`
/// stand bis 2026-08-31 ohne gemessene Hoehe in drei Layouts im Lichtkorridor; R3 begrenzt dort
/// auf 140 cm, und `if let Some(h)` uebersprang das Stueck **stillschweigend**. Zwei Layouts
/// bestanden auf einer Regel, die fuer das entscheidende Moebel nie gelaufen war.
#[derive(Debug, Clone, Serialize)]
pub struct UncheckedRule {
    pub rule: String,
    pub text: String,
    /// Im Klartext, was fehlt. Wandert in den Bericht, damit die Luecke benannt ist.
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

/// Zwei Reihenfolgen sind hier wesentlich, beide teuer gelernt: `couch` darf `couchtisch` nicht
/// fangen — sonst gilt ein Couchtisch als Sofa —, und `coffee_table` muss vor `table` stehen,
/// weil ein Couchtisch 40 cm zum Sofa will und nicht 80 cm Stuhlausziehzone.
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

/// Dieselbe Heuristik, nur ueber den Namen — fuer Eintraege, die noch nirgends stehen.
///
/// `deklaration.rs` braucht sie, um einem Inventareintrag zu sagen, wofuer diese Maschine ihn
/// haelt, solange er selbst nichts erklaert. Eine zweite Abschrift derselben Praefixliste waere
/// genau die Doppelung, die `^couch` gegen `couchtisch` einmal gekostet hat.
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
    /// Der Katalogeintrag, falls es einen gibt. Ein Layout darf ein Stueck nennen, das seine
    /// Masse nur ueber `size:` mitbringt und im Inventar fehlt — dann gibt es nichts zu
    /// deklarieren, und der Name entscheidet.
    item: Option<&'a Item>,
    w: i32,
    d: i32,
    h: Option<i32>,
    rect: Rect,
}

/// Eine Oeffnung als Strecke in Raumkoordinaten, mit der Normalen, die nach INNEN zeigt.
///
/// Oeffentlich, weil `einbringung.rs` dieselbe Frage stellt — wo im Raum liegt diese Tuer und
/// wo ist innen — und eine zweite Fassung davon genau die Doppelung waere, gegen die diese
/// Capability existiert.
pub struct Segment {
    pub a: Pt,
    pub b: Pt,
    pub normal: [f64; 2],
}

/// Loest eine Oeffnung in eine Strecke in Raumkoordinaten auf, samt der Normalen, die nach
/// INNEN zeigt. Die Normale wird probiert, nicht angenommen: `west` und `sued_hauptraum`
/// laufen entgegen der Kantenrichtung des Polygons.
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


/// Was ein Stueck an Platz verlangt, wenn es es selbst sagt (PRD Q61 / B26).
///
/// Gibt `false` zurueck, wenn das Stueck gar nichts erklaert — dann entscheidet weiter der
/// Name, und der Aufrufer faellt auf `kind_of` zurueck. Das ist Absicht: 42 Zeilen an einem Tag
/// umzustellen waere ein Stichtag, an dem sich Verdikte aendern, ohne dass jemand die Zahlen
/// dahinter geprueft hat.
///
/// Die drei Pruefungen, und was jede von der Namensfassung uebernimmt:
///
/// * `open_clear` — der Platz vor Tueren und Schubladen. `wall_ok = false` bindet ihn an die
///   Side, die `opens` nennt; sonst reicht die beste, weil ein Stueck gedreht werden kann.
/// * `access_sides` / `access_clear` — wie viele Seiten begehbar sein muessen und wie tief.
/// * `expands` — der zweite Zustand eines Klappmoebels. `to` ist die Gesamttiefe ausgeklappt,
///   also ist der zusaetzlich noetige Platz `to` minus der Tiefe in dieser Richtung.
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
        let deckel = horizon(want);
        let (gemessen, wo) = match (side, binding) {
            (Some(s), true) => (
                free_depth_on_side(grid, &p.rect, s, deckel),
                s.as_str().to_string(),
            ),
            _ => (
                SIDES
                    .iter()
                    .map(|s| free_depth_on_side(grid, &p.rect, *s, deckel))
                    .max()
                    .unwrap_or(0),
                "seiner besten Side".to_string(),
            ),
        };
        remember_up_to(
            reserves,
            "oeffnen",
            Some(name),
            Some(&wo),
            gemessen,
            want,
            Limit::AtLeast,
            "cm",
            true,
            Some(deckel),
            true,
        );
        if gemessen < want {
            hard.push(Violation {
                rule: "oeffnen".into(),
                severity: Severity::Hard,
                item: Some(name.into()),
                message: format!(
                    "\"{name}\" hat {gemessen} cm zum Oeffnen an {wo}, braucht {want}"
                ),
                text: None,
                measured: Some(gemessen),
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
            "zugang",
            Some(name),
            None,
            enough,
            n,
            Limit::AtLeast,
            "seiten",
            true,
        );
        if enough < n {
            hard.push(Violation {
                rule: "zugang".into(),
                severity: Severity::Hard,
                item: Some(name.into()),
                message: format!(
                    "\"{name}\" ist von {enough} Seiten mit {want} cm erreichbar, braucht {n}"
                ),
                text: None,
                measured: Some(enough),
                required: Some(n),
            });
        }
    }

    if let (Some(dir), Some(to)) = (it.expands_dir, it.expands_to) {
        let side = dir.rotated(p.it.rot);
        let tiefe = match side {
            Side::North | Side::South => p.d,
            Side::East | Side::West => p.w,
        };
        let extra = to - tiefe;
        if extra > 0 {
            let deckel = horizon(extra);
            let frei = free_depth_on_side(grid, &p.rect, side, deckel);
            remember_up_to(
                reserves,
                "ausklappen",
                Some(name),
                Some(side.as_str()),
                frei,
                extra,
                Limit::AtLeast,
                "cm",
                true,
                Some(deckel),
                true,
            );
            if frei < extra {
                hard.push(Violation {
                    rule: "ausklappen".into(),
                    severity: Severity::Hard,
                    item: Some(name.into()),
                    message: format!(
                        "\"{name}\" hat {frei} cm nach {} zum Ausklappen, braucht {extra} (ausgeklappt {to} tief)",
                        side.as_str()
                    ),
                    text: None,
                    measured: Some(frei),
                    required: Some(extra),
                });
            }
        }
    }
    true
}

/// Wie weit ein Wegpunkt vor der Sache liegt, zu der er gehoert. Ein Mensch steht VOR einer Tuer
/// und nicht in ihrem Rahmen; wird das kleiner, wird der Rahmen selbst zur Engstelle jeder Route.
/// Eine Zahl, an einer Stelle, fuer Oeffnungen und feste Einbauten gleichermassen.
const WAYPOINT_GAP: i32 = 45;

/// Die Flaeche, die neben einem festen Einbau frei bleiben muss, auf der Side, die er nennt.
fn anlauf_rect(r: &Rect, side: Side, tiefe: i32) -> Rect {
    match side {
        Side::North => Rect {
            x: r.x,
            y: r.y - tiefe,
            w: r.w,
            d: tiefe,
        },
        Side::South => Rect {
            x: r.x,
            y: r.bottom(),
            w: r.w,
            d: tiefe,
        },
        Side::West => Rect {
            x: r.x - tiefe,
            y: r.y,
            w: tiefe,
            d: r.d,
        },
        Side::East => Rect {
            x: r.right(),
            y: r.y,
            w: tiefe,
            d: r.d,
        },
    }
}

/// Wo ein Mensch steht, wenn er diesen Einbau benutzt.
fn approach_point(r: &Rect, side: Side) -> Pt {
    match side {
        Side::North => [r.x + r.w / 2, r.y - WAYPOINT_GAP],
        Side::South => [r.x + r.w / 2, r.bottom() + WAYPOINT_GAP],
        Side::West => [r.x - WAYPOINT_GAP, r.y + r.d / 2],
        Side::East => [r.right() + WAYPOINT_GAP, r.y + r.d / 2],
    }
}

/// Das Rechteck, das ein Mensch vor einer Oeffnung braucht, aus ihrer eigenen Freihaltezone.
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

/// Was jede Regel zu lesen bekommt.
///
/// Der Raum, die Regeln, die gestellten Stuecke mit ihrer fertigen Grundflaeche und das Gitter,
/// gegen das gemessen wird. Einmal gebaut, danach nur noch gelesen: keine Regel darf die
/// Grundlage der naechsten veraendern, und dass sie es nicht kann, sagt hier der Compiler und
/// nicht ein Kommentar.
struct Check<'a> {
    model: &'a Model,
    room: &'a Room,
    rules: &'a Rules,
    placed: Vec<Placed<'a>>,
    /// Der Raum mit allem Belegten darin — Fixmoebel und gestellte Stuecke, aber ohne die
    /// Tuerschwenkflaechen. Warum die fehlen, steht in [`Pruefung::neu`].
    grid: Grid,
}

/// Was jede Regel schreiben darf, und mehr nicht.
///
/// Vier Listen statt vier Rueckgabewerten: eine Regel meldet Verstoesse in zwei Schweren, jede
/// Messung als [`Reserve`] — auch die bestandene — und den Grund, wenn sie gar nicht laufen
/// konnte. Weil alle Regeln in dieselben Listen schreiben, ist die Reihenfolge im Bericht
/// weiterhin die Reihenfolge, in der [`check_layout`] sie aufruft.
#[derive(Default)]
struct Findings {
    hard: Vec<Violation>,
    soft: Vec<Violation>,
    /// Jede Messung, nicht nur die gescheiterten. Siehe [`Reserve`].
    reserves: Vec<Reserve>,
    /// Regeln, die laufen wollten und nicht konnten. Wird im Bericht mit den gar nicht
    /// implementierten zusammengelegt — fuer den Leser ist beides dasselbe: nicht gemessen.
    fell_out: Vec<(String, String)>,
}

impl<'a> Check<'a> {
    /// Stellt das Layout auf und belegt das Gitter damit.
    ///
    /// Die einzige Stelle, die etwas aufbaut. Danach lesen alle Regeln denselben Zustand, und
    /// keine kann ihn der naechsten unter den Fuessen wegziehen.
    fn neu(model: &'a Model, layout: &'a Layout) -> Result<Self, ModelError> {
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
        // Tuerschwenk-Sperrflaechen werden hier absichtlich NICHT blockiert. Sie schraenken ein, wo
        // Moebel stehen duerfen, nicht wo ein Mensch geht — durch den Schwenkbereich der eigenen
        // Wohnungstuer laeuft man taeglich. Blockiert man sie, wird der Schwenkbogen selbst zur
        // Engstelle jeder Route und meldet 50 cm, egal wo die Moebel stehen. `r2_tuerschwenk` prueft
        // sie direkt gegen die Moebelgrundflaechen — so, wie rules.toml sie formuliert.
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

/// Beurteilt ein Layout gegen `rules.toml`.
///
/// Diese Funktion entscheidet nichts. Sie baut die `Pruefung`, laesst jede Regel einmal in
/// dieselben `Befunde` schreiben und uebergibt beides an `bericht`. Die Reihenfolge hier ist
/// die Reihenfolge im Bericht, und sie ist die einzige Verabredung zwischen den Regeln: keine
/// liest, was eine andere geschrieben hat.
pub fn check_layout(
    model: &Model,
    layout: &Layout,
) -> Result<CheckResult, crate::model::ModelError> {
    let check = Check::neu(model, layout)?;
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

    Ok(bericht(&check, layout, out, corridors))
}

/// Raumgrenze und gegenseitige Ueberlappung
///
/// Das knappste Stueck an der Wand, als eine Reserve. Sie kam am 2026-08-31 dazu, und der
/// Anlass war ein Bericht ueber die echte Wohnung: ein Layout meldete DURCHGEFALLEN und
/// gleichzeitig „5 cm Reserve", weil die einzige verletzte Regel als einzige harte Regel
/// gar nichts mass. Eine knappste Zahl, die die verletzte Regel auslaesst, ist schlimmer
/// als keine — sie wird gelesen.
fn room_boundary_and_collision(ctx: &Check, out: &mut Findings) {
    let room = ctx.room;
    let placed = &ctx.placed;

    if let Some(tightest) = placed
        .iter()
        .min_by_key(|p| rect_in_polygon_cm(&p.rect, &room.main_room.polygon))
    {
        remember_only_violation(
            &mut out.reserves,
            "raumgrenze",
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
                rule: "raumgrenze".into(),
                severity: Severity::Hard,
                item: Some(p.it.reference.clone()),
                message: format!(
                    "\"{}\" ragt aus dem Raum bei {},{} ({}×{} cm)",
                    p.it.reference, p.it.x, p.it.y, p.w, p.d
                ),
                text: None,
                measured: None,
                required: None,
            });
        }
    }
    // Der engste Abstand zwischen zwei Stuecken, als EINE Reserve und nicht als n².
    // Ein Paar je Kombination waere bei 13 Stuecken 78 Zeilen, von denen 77 nichts sagen; was
    // interessiert, ist die Stelle, an der es zuerst reisst.
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
                    rule: "kollision".into(),
                    severity: Severity::Hard,
                    item: Some(format!("{} / {}", a.it.reference, b.it.reference)),
                    message: format!(
                        "\"{}\" und \"{}\" ueberlappen sich um {} dm²",
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
            "kollision",
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

/// R1: die Anlaufzone einer Oeffnung bleibt frei.
///
/// R2 und R7 stellen dieselbe Frage an andere Flaechen — den Tuerschwenk und die Anlaufzone
/// eines festen Einbaus — und stehen darum je fuer sich.
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
        // Das naechste Moebel an dieser Zone, ob es sie nun beruehrt oder nicht.
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
                            "\"{}\" ragt in die {} cm Anlaufzone von {}",
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

/// R2: kein Moebel im Schwenkbereich einer Tuer.
///
/// Diese Flaechen sind absichtlich nicht im Gitter belegt — warum, steht in [`Pruefung::neu`].
/// Sie werden hier direkt gegen die Moebelgrundflaechen geprueft, so wie rules.toml sie
/// formuliert.
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
                            "\"{}\" steht im Tuerschwenkbereich von {}",
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

/// R7: die Anlaufzone eines festen Einbaus bleibt frei.
///
/// R7 galt bis 2026-08-30 fuer genau ein Moebel, das der Code beim Namen kannte. Sie gilt
/// jetzt fuer jeden festen Einbau, der eine Anlaufzone deklariert — und die Schwelle schlaegt
/// er ueber ihren Namen in rules.toml nach, statt sie mitzubringen.
fn r7_fixture_approach_zones(ctx: &Check, out: &mut Findings) -> Result<(), ModelError> {
    let room = ctx.room;
    let rules = ctx.rules;
    let placed = &ctx.placed;

    for f in &room.fixed_furniture {
        let Some(zone_decl) = &f.approach_zone else {
            continue;
        };
        let depth = rules.threshold(&zone_decl.threshold)?;
        let zone = anlauf_rect(&f.rect(), zone_decl.side, depth);
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
                            "\"{}\" blockiert die {} cm Anlaufzone von {}",
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

/// R3: der Lichtkorridor bleibt niedrig.
///
/// Anders als in der TypeScript-Vorlage sind diese vier Zahlen KEINE Abschrift aus einem
/// Prosatext mehr: rules.toml fuehrt `[lichtkorridor]` strukturiert, also kann die Regel
/// nicht mehr still von ihrer eigenen Beschreibung abweichen.
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
        // Ohne gemessene Hoehe kann R3 hier nichts sagen — und ein Stueck, das gar nicht im
        // Korridor steht, braucht auch keine: gemeldet wird nur, wo die Regel wirklich gegriffen
        // haette.
        if p.h.is_none() && p.rect.overlaps(&corridor) {
            out.fell_out.push((
                "R3".to_string(),
                format!(
                    "\"{}\" steht im Lichtkorridor und hat keine gemessene Hoehe",
                    p.it.reference
                ),
            ));
        }
        if let (Some(h), true) = (p.h, p.rect.overlaps(&corridor)) {
            // Die einzige Schwelle, die nach oben begrenzt. Ohne `Grenze` traege ihre Reserve
            // das falsche Vorzeichen, und ein 80 cm hohes Lowboard im Korridor sähe knapper
            // aus als ein 200 cm hoher Schrank.
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
                            "\"{}\" ist {} cm hoch im Lichtkorridor (max {}); dieser Raum hat eine Fensterwand",
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

/// R4: Bettkopf weg von der Verglasung
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
                            "Bett beruehrt die Verglasung (Kaelte, Zug, keine Privatsphaere zur Terrasse)".into(),
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

/// R5: Fensterlicht faellt seitlich auf den Schreibtisch
///
/// "Weder frontal zur Verglasung (Blendung) noch mit dem Ruecken dazu (Reflexionen)."
/// Beides sind Aussagen ueber die Achse, auf der der Schreibtisch zur Verglasung steht:
/// sitzt man an einer Tiefenseite, liegt vorne und hinten auf der Tiefenachse. Zeigt die
/// Verglasung dorthin, ist sie im Gesicht oder im Ruecken; liegt sie auf der Breitenachse,
/// faellt das Licht seitlich ein — genau das, was die Regel verlangt.
///
/// Ohne Verglasung im Raum ist nichts zu messen, und das wird gemeldet statt uebersprungen.
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
                "der Raum fuehrt keine Verglasung, gegen die ein Schreibtisch stehen koennte"
                    .to_string(),
            )),
            Some(seg) => {
                // Aus welcher Richtung das Licht kommt, sagt die NORMALE der Verglasung, nicht
                // der Abstand zwischen zwei Mittelpunkten. Der erste Entwurf verglich
                // Mittelpunkte und meldete daraufhin in allen 13 Layouts einen Verstoss — bei
                // einem Schreibtisch, der in allen 13 an derselben Stelle steht und dessen
                // Fenster in der Seitenwand sitzt. Eine Regel, die ueberall feuert, misst
                // nichts.
                //
                // Die Verglasung liegt in einer Wand; ihre Normale zeigt in den Raum. Faellt das
                // Licht entlang der Achse ein, auf der der Schreibtisch vorne und hinten hat,
                // steht er frontal oder mit dem Ruecken dazu. Steht die Normale quer dazu,
                // kommt es seitlich — und genau das verlangt die Regel.
                let light_runs_in_y = seg.normal[1].abs() > seg.normal[0].abs();
                for p in placed.iter().filter(|p| p.kind == Kind::Desk) {
                    // `p.rect` ist bereits gedreht, also entscheidet es selbst, welche Achse
                    // gerade die Tiefe ist: an der Tiefenseite sitzt man.
                    let depth_runs_in_y = p.rect.d <= p.rect.w;
                    if light_runs_in_y == depth_runs_in_y {
                        sort_in(
                            house_rule(
                                rules,
                                "R5",
                                Some(p.it.reference.clone()),
                                format!(
                                    "\"{}\" steht mit Front oder Ruecken zur Verglasung — Blendung bzw. Reflexionen; das Licht soll seitlich einfallen",
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

/// R6: der Blick vom Eingang faellt nicht zuerst aufs Bett
///
/// Ein Strahl von der Eingangsoeffnung nach innen, entlang ihrer Normalen. Was er zuerst
/// trifft, ist das, was man beim Eintreten sieht. Ist das ein Bett, ist die Regel verletzt.
///
/// **Welche Oeffnung der Eingang ist, sagt die Wohnung** (`eingang = true`), nicht der Code.
/// `badtuer` und `eingangstuer` sind beide `typ = "tuer"`, und die nach dem Namen zu
/// unterscheiden waere genau der Fehler, den B26a mit der Kuechen-Anlaufzone geschlossen hat.
/// Sagt die Wohnung nichts, laeuft R6 nicht und sagt das.
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
                "keine Oeffnung ist als Eingang deklariert (`eingang = true` in room.toml)"
                    .to_string(),
            )),
            Some(seg) => {
                let mut x = (seg.a[0] + seg.b[0]) as f64 / 2.0;
                let mut y = (seg.a[1] + seg.b[1]) as f64 / 2.0;
                let mut seen: Option<&Placed> = None;
                // In Schritten von 5 cm, hoechstens die Diagonale eines sehr grossen Raums.
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
                                    "vom Eingang aus faellt der Blick zuerst auf \"{}\"",
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

/// R9: kein Dauersonnenplatz am Schreibtisch (nur wenn die Wohnung sie fuehrt)
///
/// Die erste Regel dieser Maschine, die ein Datum kennt. R5 fragt, auf welcher Achse die
/// Verglasung liegt — eine Aussage ueber die Aufstellung, die ohne Ort und Jahreszeit
/// auskommt und deshalb auch nicht beantworten kann, ob im Maerz um neun die Sonne auf dem
/// Bildschirm steht. R9 rechnet den Sonnenstand und den Lichtfleck auf dem Boden.
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
                "room.toml fuehrt kein [lage] — ohne Breite, Laenge und Nordrichtung gibt es \
                 keinen Sonnenstand"
                    .to_string(),
            )),
            (_, None, _) => out.fell_out.push((
                "R9".to_string(),
                "rules.toml fuehrt R9, aber keinen [sonne]-Block mit der Stundenschwelle"
                    .to_string(),
            )),
            (Some(_), Some(regel), false) => {
                let denominator = crate::sun::checked_hours();
                for p in desks {
                    let n = crate::sun::hours_in_sun(model, &p.rect)?;
                    remember(
                        &mut out.reserves,
                        "R9",
                        Some(&p.it.reference),
                        Some("sonnenstunden"),
                        n as i32,
                        regel.max_hours_at_desk as i32,
                        Limit::AtMost,
                        "stunden",
                        ist_hart(rules, "R9")?,
                    );
                    if n > regel.max_hours_at_desk {
                        sort_in(
                            house_rule(
                                rules,
                                "R9",
                                Some(p.it.reference.clone()),
                                format!(
                                    "\"{}\" liegt in {n} von {denominator} geprueften Stunden in direkter Sonne, erlaubt sind {}",
                                    p.it.reference, regel.max_hours_at_desk
                                ),
                                Some(n as i32),
                                Some(regel.max_hours_at_desk as i32),
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

/// Die Abstaende, die ein Stueck zum Benutztwerden braucht, je nach Art des Stuecks.
///
/// Sagt das Stueck selbst, was es braucht, gilt das und sonst nichts. Beides zu pruefen hiesse,
/// ein Moebel gegen zwei Regelsaetze zu messen und den strengeren gewinnen zu lassen — dann
/// waere die Deklaration keine Ersetzung, sondern eine Verschaerfung.
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

/// Zugang zum Bett: die beste Laengsseite und die zweite.
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
    let deckel = horizon(want.max(second));
    let mut depths: Vec<i32> = long
        .iter()
        .map(|s| free_depth_on_side(grid, &p.rect, *s, deckel))
        .collect();
    depths.sort_unstable_by(|a, b| b.cmp(a));
    remember_up_to(
        &mut out.reserves,
        "bett_zugang",
        Some(&p.it.reference),
        Some("beste laengsseite"),
        depths[0],
        want,
        Limit::AtLeast,
        "cm",
        true,
        Some(deckel),
        true,
    );
    remember_up_to(
        &mut out.reserves,
        "bett_zugang",
        Some(&p.it.reference),
        Some("zweite laengsseite"),
        depths[1],
        second,
        Limit::AtLeast,
        "cm",
        true,
        Some(deckel),
        true,
    );
    if depths[0] < want {
        out.hard.push(Violation {
            rule: "bett_zugang".into(),
            severity: Severity::Hard,
            item: Some(p.it.reference.clone()),
            message: format!(
                "Bett hat {} cm an seiner besten Laengsseite, braucht {}",
                depths[0], want
            ),
            text: None,
            measured: Some(depths[0]),
            required: Some(want),
        });
    }
    if depths[1] < second {
        out.hard.push(Violation {
            rule: "bett_zugang".into(),
            severity: Severity::Hard,
            item: Some(p.it.reference.clone()),
            message: format!(
                "Bett hat {} cm an seiner zweiten Laengsseite, braucht {}",
                depths[1], second
            ),
            text: None,
            measured: Some(depths[1]),
            required: Some(second),
        });
    }
    Ok(())
}

/// Platz fuer den Stuhl am Schreibtisch, an seiner besten Side.
fn chair_zone(ctx: &Check, p: &Placed, out: &mut Findings) -> Result<(), ModelError> {
    let rules = ctx.rules;
    let grid = &ctx.grid;

    let want = rules.threshold("schreibtisch_stuhlzone")?;
    let deckel = horizon(want);
    let best = SIDES
        .iter()
        .map(|s| free_depth_on_side(grid, &p.rect, *s, deckel))
        .max()
        .unwrap_or(0);
    remember_up_to(
        &mut out.reserves,
        "stuhlzone",
        Some(&p.it.reference),
        None,
        best,
        want,
        Limit::AtLeast,
        "cm",
        true,
        Some(deckel),
        true,
    );
    if best < want {
        out.hard.push(Violation {
            rule: "stuhlzone".into(),
            severity: Severity::Hard,
            item: Some(p.it.reference.clone()),
            message: format!(
                "Schreibtisch hat {} cm fuer den Stuhl an seiner besten Side, braucht {}",
                best, want
            ),
            text: None,
            measured: Some(best),
            required: Some(want),
        });
    }
    Ok(())
}

/// Platz, um die Schranktuer zu oeffnen, an der besten Side.
fn wardrobe_door(ctx: &Check, p: &Placed, out: &mut Findings) -> Result<(), ModelError> {
    let rules = ctx.rules;
    let grid = &ctx.grid;

    let want = rules.threshold("schrank_tuer_oeffnen")?;
    let deckel = horizon(want);
    let best = SIDES
        .iter()
        .map(|s| free_depth_on_side(grid, &p.rect, *s, deckel))
        .max()
        .unwrap_or(0);
    remember_up_to(
        &mut out.reserves,
        "schrank_tuer",
        Some(&p.it.reference),
        None,
        best,
        want,
        Limit::AtLeast,
        "cm",
        true,
        Some(deckel),
        true,
    );
    if best < want {
        out.hard.push(Violation {
            rule: "schrank_tuer".into(),
            severity: Severity::Hard,
            item: Some(p.it.reference.clone()),
            message: format!("Schrank hat {} cm zum Tueroeffnen, braucht {}", best, want),
            text: None,
            measured: Some(best),
            required: Some(want),
        });
    }
    Ok(())
}

/// R8: das Schlafsofa hat den Platz, um ausgezogen zu werden.
///
/// Das Schlafsofa wird zum Ausziehen nicht verschoben (Lars, 2026-08-30). Die
/// Ausklapptiefe ist damit dauerhaft belegte Flaeche und nichts, das man sich im
/// Gastfall borgt. Geprueft nur an den Laengsseiten: ausgezogen wird ueber die
/// lange Kante, und in genau eine Richtung, also reicht die bessere Side.
fn r8_couch_unfold(ctx: &Check, p: &Placed, out: &mut Findings) -> Result<(), ModelError> {
    let rules = ctx.rules;
    let grid = &ctx.grid;

    if let Ok(want) = rules.threshold("couch_ausklapptiefe") {
        let long = if p.w >= p.d {
            [Side::North, Side::South]
        } else {
            [Side::West, Side::East]
        };
        let deckel = horizon(want);
        let best = long
            .iter()
            .map(|s| free_depth_on_side(grid, &p.rect, *s, deckel))
            .max()
            .unwrap_or(0);
        remember_up_to(
            &mut out.reserves,
            "R8",
            Some(&p.it.reference),
            Some("beste laengsseite"),
            best,
            want,
            Limit::AtLeast,
            "cm",
            ist_hart(rules, "R8")?,
            Some(deckel),
            true,
        );
        if best < want {
            // Bis 2026-08-31 hiess dieser Verstoss `couch_ausklappen`, waehrend
            // jede rules.toml dieselbe Regel als R8 fuehrte. Zwei Namen fuer eine
            // Regel, und der Bericht nannte den, den die Wohnung nicht kannte.
            sort_in(
                house_rule(
                    rules,
                    "R8",
                    Some(p.it.reference.clone()),
                    format!("Couch hat {} cm zum Ausklappen an ihrer besten Laengsseite, braucht {} — sie soll dafuer nicht verschoben werden", best, want),
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

/// Platz, um am Esstisch einen Stuhl herauszuziehen — und wie viele Seiten ihn haben.
///
/// Ein Esstisch, an dem niemand einen Stuhl herausziehen kann, ist ein Regal. Hart,
/// wenn keine Side den Platz hat; weich, wenn nur eine ihn hat — das ist eine
/// Sitzplatzzahl und kein Verstoss: ein Wandklapptisch hat legitim eine Side.
fn table_chair(ctx: &Check, p: &Placed, out: &mut Findings) -> Result<(), ModelError> {
    let rules = ctx.rules;
    let grid = &ctx.grid;

    let want = rules.threshold("esstisch_stuhl_ausziehen")?;
    let deckel = horizon(want);
    let sides: Vec<i32> = SIDES
        .iter()
        .map(|s| free_depth_on_side(grid, &p.rect, *s, deckel))
        .collect();
    let best = *sides.iter().max().unwrap_or(&0);
    let seats = sides.iter().filter(|d| **d >= want).count();
    remember_up_to(
        &mut out.reserves,
        "stuhl_ausziehen",
        Some(&p.it.reference),
        Some("beste seite"),
        best,
        want,
        Limit::AtLeast,
        "cm",
        true,
        Some(deckel),
        true,
    );
    remember(
        &mut out.reserves,
        "stuhl_ausziehen",
        Some(&p.it.reference),
        Some("sitzplaetze"),
        seats as i32,
        2,
        Limit::AtLeast,
        "plaetze",
        false,
    );
    if seats == 0 {
        out.hard.push(Violation {
            rule: "stuhl_ausziehen".into(),
            severity: Severity::Hard,
            item: Some(p.it.reference.clone()),
            message: format!(
                "Tisch hat {} cm zum Stuhlherausziehen an seiner besten Side, braucht {}",
                best, want
            ),
            text: None,
            measured: Some(best),
            required: Some(want),
        });
    } else if seats < 2 {
        out.soft.push(Violation {
            rule: "stuhl_ausziehen".into(),
            severity: Severity::Soft,
            item: Some(p.it.reference.clone()),
            message: format!(
                "Tisch bietet {} Sitzplatz — nur eine Side hat die {} cm, die ein Stuhl braucht",
                seats, want
            ),
            text: None,
            measured: Some(seats as i32),
            required: Some(2),
        });
    }
    Ok(())
}

/// Abstand zwischen Couchtisch und Sofa, je Sofa im Raum.
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
            "couchtisch_abstand",
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
                rule: "couchtisch_abstand".into(),
                severity: Severity::Soft,
                item: Some(p.it.reference.clone()),
                message: format!("Couchtisch steht {} cm vom Sofa, will {}", gap, want),
                text: None,
                measured: Some(gap),
                required: Some(want),
            });
        }
    }
    Ok(())
}

/// Die Routen, die room.toml fuehrt: wie schmal wird jede an ihrer engsten Stelle.
///
/// Gibt die gemessenen Korridore zurueck, weil der Bericht sie unter `metrics` fuehrt — als
/// einzige Regel, die mehr liefert als Verstoesse und Reserven.
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
        // Ein Wegpunkt, den es nicht gibt, ist ein Fehler und kein uebersprungener Weg. Bis
        // 2026-08-30 hat diese Stelle still weitergemacht: eine vertippte Oeffnung kostete eine
        // Route, und der Bericht sah danach aus wie einer ueber eine Wohnung mit weniger Wegen.
        let (Some(a), Some(b)) = (find(from), find(to)) else {
            let known: Vec<&str> = waypoints.iter().map(|(n, _)| n.as_str()).collect();
            return Err(ModelError::Missing(format!(
                "Route {from} → {to}: kein Wegpunkt dieses Namens. Wegpunkte sind Oeffnungen \
                 ausser Fenstern und feste Moebel mit `anlaufzone`; vorhanden sind: {}",
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
        // Kein Weg ist null Breite und nicht „nicht gemessen": die Route wurde gesucht und es
        // gibt sie nicht, was die knappste denkbare Reserve ist und nicht die fehlende.
        remember(
            &mut out.reserves,
            "laufweg",
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
                rule: "laufweg".into(),
                severity: Severity::Hard,
                item: None,
                message: format!("gar kein begehbarer Weg von {} nach {}", from, to),
                text: None,
                measured: None,
                required: Some(rules.walkways.main_min),
            }),
            Some(w) if w < rules.walkways.main_min => out.hard.push(Violation {
                rule: "laufweg".into(),
                severity: Severity::Hard,
                item: None,
                message: format!(
                    "Route {} → {} schnuert auf {} cm ein, unter dem Minimum von {} cm",
                    from, to, w, rules.walkways.main_min
                ),
                text: None,
                measured: Some(w),
                required: Some(rules.walkways.main_min),
            }),
            Some(w) if w < rules.walkways.main_target => out.soft.push(Violation {
                rule: "laufweg".into(),
                severity: Severity::Soft,
                item: None,
                message: format!(
                    "Route {} → {} schnuert auf {} cm ein, unter dem Ziel von {} cm",
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

/// Der Bericht: was gemessen wurde, was fehlschlug, und was gar nicht geprueft ist.
///
/// Die einzige Stelle, die alle Befunde zusammen sieht. Sie rechnet keine Regel nach — sie
/// zaehlt, sortiert die knappste Reserve heraus und legt die ausgefallenen Regeln mit den nie
/// implementierten zusammen, weil beides fuer den Leser dasselbe ist: nicht gemessen.
fn bericht(ctx: &Check, layout: &Layout, out: Findings, corridors: Vec<Corridor>) -> CheckResult {
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

    // Nur Zentimeter. Das Minimum aus einer Tiefe und einer Sitzplatzzahl waere eine Zahl
    // ohne Einheit, und sie stuende an der Stelle, an der die Oberflaeche „so knapp besteht
    // dieses Layout" schreibt.
    // Was diese Maschine ueberhaupt prueft — die Pflichtregeln plus die optionalen. Ohne die
    // zweite Haelfte meldete eine gefuehrte und tatsaechlich gelaufene R9 sich selbst als
    // ungeprueft.
    let checked: Vec<&str> = RULE_IDS
        .iter()
        .chain(OPTIONAL_RULE_IDS.iter())
        .copied()
        .collect();

    let tightest_reserve_cm = reserves
        .iter()
        .filter(|r| r.hart && r.unit == "cm" && (r.binding || r.slack < 0))
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
                reason: "diese Maschine prueft sie nicht".to_string(),
            })
            .chain(fell_out.into_iter().map(|(id, reason)| UncheckedRule {
                rule: id.clone(),
                text: rules.rule(&id).map(|r| r.text.clone()).unwrap_or_default(),
                reason,
            }))
            .collect(),
    }
}
