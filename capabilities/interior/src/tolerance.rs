//! Was ein Verdikt aushaelt, wenn das Bandmass sich geirrt hat.
//!
//! Das Inventar fuehrt seit jeher `unsicher = ["h"]` — welche Masse geschaetzt sind statt
//! gemessen —, und `check_layout` reicht die Liste unveraendert in jeden Bericht durch. Gelesen
//! hat sie niemand: die Pruefung rechnete mit der geschaetzten Zahl, als waere sie gemessen,
//! und meldete `pass`. Ein Verdikt, das auf einer Schaetzung steht, ist genauso viel wert wie
//! die Schaetzung — und **wie viel das ist, stand nirgends**.
//!
//! Diese Datei beantwortet dieselbe Frage in Zentimetern: *bis zu welchem Messfehler haelt
//! dieses Layout?* Nicht „besteht es", sondern „besteht es noch, wenn der Schrank in
//! Wirklichkeit 3 cm breiter ist".
//!
//! ## Warum eine Bisektion zulaessig ist
//!
//! Gesucht wird der groesste Fehler, bei dem noch alles besteht, und gesucht wird ihn mit einer
//! Halbierung — was nur erlaubt ist, wenn Bestehen monoton faellt. Das tut es, und der Grund
//! ist Geometrie und keine Annahme: die Stoerung vergroessert Masse ausschliesslich. Ein
//! groesseres Rechteck belegt mehr Raster, also kann jede freie Tiefe, jede Korridorbreite und
//! jeder Abstand nur kleiner werden; eine groessere Hoehe kann R3 nur naeher kommen. Es gibt
//! keinen Weg, auf dem ein groesseres Moebel eine Regel erfuellt, die das kleinere verletzt.
//!
//! Gestoert wird **nach oben** und nicht in beide Richtungen. Ein zu klein geschaetztes Mass
//! ist der Fall, der einen Umzug ruiniert; ein zu gross geschaetztes verschenkt Platz und
//! blockiert nichts.
//!
//! ## Was NICHT variiert wird, und warum das im Bericht steht
//!
//! Die Wohnung. `room.toml` traegt ihre Unsicherheit als datierte Korrekturkommentare — *die
//! Terrassentuer wurde als 1,36 m gelesen, bis der bemasste Grundriss 1,96 sagte* — und das ist
//! Prosa fuer einen Menschen und keine Zahl fuer eine Maschine (PRD Q60). Ein `toleranz_cm` in
//! die Geometrie zu erfinden waere eine Zahl, die niemand gemessen hat.
//!
//! Und Stuecke, die ihre Masse im Layout selbst tragen (`size = [b, t]`), denn dort gibt es
//! kein `unsicher`, das sie als Schaetzung ausweisen koennte. Beide Faelle nennt der Bericht,
//! statt sie stillschweigend als sicher zu behandeln.

use crate::clearance::{check_layout, Uncertainty};
use crate::model::{Layout, Model, ModelError};
use serde::Serialize;

/// Wie weit hoch gesucht wird, in cm.
///
/// Wer ein Layout hat, das 50 cm Messfehler ueberlebt, hat keine Messfrage mehr, sondern einen
/// leeren Raum. Der Horizont wird gemeldet und nicht als Ergebnis ausgegeben.
pub const HORIZON_CM: i32 = 50;

/// Wie viel Messfehler ein Verdikt aushaelt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "art", rename_all = "snake_case")]
pub enum Holds {
    /// Faellt schon bei den eingetragenen Massen durch. Ein Messfehler ist hier nicht die Frage.
    Fails,
    /// Kein Mass in diesem Layout ist als geschaetzt gekennzeichnet.
    ///
    /// Kein Freibrief: es heisst, dass das Inventar nichts als unsicher fuehrt, nicht, dass
    /// alles nachgemessen wurde.
    NothingGuessed,
    /// Besteht bis zu diesem Fehler in cm, und bei einem cm mehr nicht mehr.
    Bis { cm: i32 },
    /// Besteht auch bei `HORIZONT_CM` noch.
    BeyondHorizon { horizon_cm: i32 },
}

#[derive(Debug, Clone, Serialize)]
pub struct Robustness {
    pub layout: String,
    pub nominal_pass: bool,
    /// Die knappste harte Messung bei den eingetragenen Massen.
    pub tightest_reserve_cm: Option<i32>,
    pub holds: Holds,
    /// Welche Regeln als erste reissen, einen Zentimeter jenseits von `haelt`.
    ///
    /// Das ist der Teil, der eine Handlung nach sich zieht: nicht „irgendwann wird es eng",
    /// sondern „nachmessen musst du den Schrank, und zwar wegen R7".
    pub tips_at: Vec<String>,
    /// Die Stuecke, deren Masse geschaetzt sind — sie tragen das Risiko.
    pub guessed: Vec<Uncertainty>,
    /// Stuecke mit `size:` im Layout: sie tragen ihr Mass selbst und werden nicht variiert.
    pub not_varied: Vec<String>,
}

/// Dasselbe Modell mit vergroesserten Massen, dort wo sie als geschaetzt gelten.
///
/// Nur `b`, `t` und `h`, und nur, wenn `unsicher` sie nennt. `unsicher = ["h"]` heisst, dass
/// die Hoehe geraten ist, und **nicht**, dass das ganze Stueck unbekannt ist.
fn disturbed(model: &Model, tol: i32) -> Model {
    let mut catalogue = model.catalogue.clone();
    for it in catalogue.values_mut() {
        // Was nicht in `unsicher` steht, bleibt. Ein pauschaler Aufschlag auf alle Masse waere
        // eine andere Frage — er wuerde messen, was passiert, wenn ALLES falsch ist, und nicht,
        // was passiert, wenn das Geratene falsch ist.
        let unsicher = std::mem::take(&mut it.estimated);
        for field in &unsicher {
            let target = match field.as_str() {
                "b" => &mut it.width,
                "t" => &mut it.depth,
                "h" => &mut it.height,
                // `unsicher` darf jeden Feldnamen tragen; variiert wird, was eine Ausdehnung
                // ist. Ein unsicherer Preis aendert keine Raeumung.
                _ => continue,
            };
            if let Some(x) = target.as_mut() {
                *x += tol;
            }
        }
        it.estimated = unsicher;
    }
    Model {
        room: model.room.clone(),
        rules: model.rules.clone(),
        catalogue,
        states: model.states.clone(),
        flat_dir: model.flat_dir.clone(),
    }
}

/// Besteht dieses Layout noch, wenn jedes geschaetzte Mass um `tol` cm daneben liegt?
fn passes_at(model: &Model, layout: &Layout, tol: i32) -> Result<bool, ModelError> {
    Ok(check_layout(&disturbed(model, tol), layout)?.pass)
}

/// Derselbe Aufruf, oeffentlich, damit ein Test die Monotonie an den Zahlen pruefen kann.
///
/// Die Bisektion oben steht und faellt damit, und ein Kommentar, der Monotonie behauptet, ist
/// keine Pruefung. Der Aufruf bleibt ansonsten privat: er ist ein Schritt eines Verfahrens und
/// keine Frage, die jemand stellt.
pub fn passes_at_for_test(model: &Model, layout: &Layout, tol: i32) -> Result<bool, ModelError> {
    passes_at(model, layout, tol)
}

pub fn robustness(model: &Model, layout: &Layout) -> Result<Robustness, ModelError> {
    let nominal = check_layout(model, layout)?;
    let refs: Vec<&str> = layout.items.iter().map(|i| i.reference.as_str()).collect();

    let guessed: Vec<Uncertainty> = nominal.uncertainties.clone();
    let not_varied: Vec<String> = layout
        .items
        .iter()
        .filter(|i| i.size.is_some())
        .map(|i| i.reference.clone())
        .collect();
    // Ein geschaetztes Mass an einem Stueck, das sein Mass im Layout mitbringt, wird von der
    // Stoerung nicht erreicht — `footprint` liest dann `size:` und nicht den Katalog.
    let effective: Vec<&Uncertainty> = guessed
        .iter()
        .filter(|u| !not_varied.contains(&u.reference) && refs.contains(&u.reference.as_str()))
        .collect();

    let holds = if !nominal.pass {
        Holds::Fails
    } else if effective.is_empty() {
        Holds::NothingGuessed
    } else if passes_at(model, layout, HORIZON_CM)? {
        Holds::BeyondHorizon {
            horizon_cm: HORIZON_CM,
        }
    } else {
        // Halbierung ueber [0, HORIZONT]: `lo` besteht immer, `hi` nie. Beide Enden sind oben
        // schon gemessen, also ist die Schleife eine Verfeinerung und keine Suche ins Blaue.
        let (mut lo, mut hi) = (0, HORIZON_CM);
        while hi - lo > 1 {
            let mid = lo + (hi - lo) / 2;
            if passes_at(model, layout, mid)? {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        Holds::Bis { cm: lo }
    };

    let tips_at = match holds {
        Holds::Bis { cm } => {
            let mut ids: Vec<String> = check_layout(&disturbed(model, cm + 1), layout)?
                .hard
                .into_iter()
                .map(|v| v.rule)
                .collect();
            ids.sort();
            ids.dedup();
            ids
        }
        _ => Vec::new(),
    };

    Ok(Robustness {
        layout: layout.name.clone(),
        nominal_pass: nominal.pass,
        tightest_reserve_cm: nominal.tightest_reserve_cm,
        holds,
        tips_at,
        guessed,
        not_varied,
    })
}
