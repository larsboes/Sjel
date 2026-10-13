//! The purchase order of the open needs. The monthly balance it is measured against is
//! inventory's, read from the same store.


pub use sjel_inventory::budget::{monate_bis_bezahlt, monatssaldo, Saldo};


// ---------------------------------------------------------------- die Reihenfolge

/// Wie dringend ein Bedarf ist, aus dem Feld `prioritaet` der Zeile.
///
/// Die Woerter stehen im Inventar seit es das Inventar gibt; gelesen hat sie bis 2026-08-31
/// nur die Anzeige. Eine Rangfolge, die sie ignoriert, sortiert nach Preis — und macht damit
/// aus einer Entscheidung ueber Dringlichkeit eine ueber Guenstigkeit.
///
/// **Ein Wort, zwei Achsen, und beide stehen wirklich in den Daten.** Ein `[[slot]]` sagt, wie
/// dringend das BEDUERFNIS ist (`pflicht`, `empfehlung`, `konzept`); ein `[[produkt]]` sagt,
/// wie weit die ENTSCHEIDUNG ist (`gesetzt`, `kandidat`, `zurueckgestellt`, `verworfen`,
/// `ersetzt`). Sie in eine Ordnung zu bringen ist zulaessig, weil beide dieselbe Frage
/// beantworten — was als naechstes gekauft wird —, und ein gesetztes Produkt steht vorn, weil
/// die Entscheidung daran schon gefallen ist.
///
/// Die Liste ist aus dem Inventar der echten Wohnung gelesen und nicht aus dem PRD abgeschrieben.
/// Der erste Entwurf kannte nur `pflicht`, `empfehlung` und `konzept` — drei der acht Woerter,
/// die dort wirklich vorkommen —, und die uebrigen fielen still auf denselben Rang: ein
/// **gesetztes** Produkt und ein **zurueckgestelltes** standen gleichauf, und was am Ende
/// zwischen ihnen entschied, war der Preis.
fn rang(prioritaet: Option<&str>) -> u8 {
    match prioritaet {
        Some("gesetzt") => 0,
        Some("pflicht") => 1,
        Some("empfehlung") => 2,
        Some("kandidat") => 3,
        Some("konzept") => 4,
        // Ohne Angabe ist die Dringlichkeit unbekannt und nicht niedrig: sie steht hinter allem
        // Eingeordneten und trotzdem VOR dem ausdruecklich Verschobenen. Schweigen ist keine
        // Entscheidung, `zurueckgestellt` ist eine.
        None => 5,
        Some("zurueckgestellt") => 6,
        Some(_) => 7,
    }
}

/// Die Woerter, die diese Rangfolge kennt. Wer hier fehlt, wird gemeldet statt einsortiert.
pub const BEKANNTE_PRIORITAETEN: &[&str] = &[
    "gesetzt",
    "pflicht",
    "empfehlung",
    "kandidat",
    "konzept",
    "zurueckgestellt",
    // Nicht in der Reihenfolge, aber bekannt: gegen beide ist entschieden.
    "verworfen",
    "ersetzt",
];

#[derive(Debug, Clone, serde::Serialize)]
pub struct Kaufposten {
    pub id: String,
    pub label: String,
    pub prioritaet: Option<String>,
    /// Der Preis, mit dem gerechnet wird: `preis_cent`, sonst die untere Kante der Schaetzung.
    pub preis_cent: Option<i64>,
    /// In welchen Layouts dieses Stueck schon eingeplant ist.
    ///
    /// Ein Bedarf, auf den ein Entwurf baut, ist ein anderer als eine Idee: ohne ihn ist das
    /// Layout kein Plan, sondern eine Zeichnung. Deshalb steht er vor gleich dringenden.
    pub in_layouts: Vec<String>,
    /// Die Summe aller Posten bis hierher, diesen eingeschlossen.
    pub kumuliert_cent: i64,
    /// Nach wie vielen Monatssalden dieser Posten erreicht ist.
    ///
    /// `None`, wenn kein Saldo vorliegt oder er nicht positiv ist — dieselbe Haltung wie
    /// `monate_bis_bezahlt`: aus einem Saldo, aus dem nichts uebrig bleibt, spart niemand an.
    pub erreichbar_nach_monaten: Option<f64>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Kaufreihenfolge {
    pub posten: Vec<Kaufposten>,
    /// Bedarfe ohne jeden Preis. Sie stehen NICHT in der Reihenfolge.
    ///
    /// Sie mit null anzusetzen hiesse, sie zuerst zu kaufen, weil sie nichts kosten. Die
    /// Wunschliste zaehlt sie schon als Warnung (PRD B29); hier sind sie das, was einer
    /// Planung im Weg steht, und die Handlung dazu ist ein Preis und kein Kauf.
    pub ohne_preis: Vec<Kaufposten>,
    pub saldo: Option<Saldo>,
    /// Prioritaetswoerter aus den Daten, die diese Rangfolge nicht kennt.
    ///
    /// Sie stehen hinten und werden **gemeldet**, statt still auf einem Rang zu landen. Genau
    /// das ist am 2026-08-31 passiert: drei der acht Woerter der echten Wohnung fehlten in
    /// `rang`, und die Reihenfolge sah trotzdem sortiert aus.
    pub unbekannte_prioritaeten: Vec<String>,
}

/// Was zuerst, was danach — und wann es erreicht ist.
///
/// Kein Budget und keine Empfehlung: eine Reihenfolge aus den Feldern, die schon da sind, und
/// eine Zeitachse aus einer Messung. Was davon gekauft wird, entscheidet niemand hier.
pub fn kaufreihenfolge(
    model: &crate::model::Model,
    saldo: Option<Saldo>,
) -> Result<Kaufreihenfolge, crate::model::ModelError> {
    // Welcher Bedarf in einem Entwurf schon steht. Einmal gelesen, nicht je Posten.
    let mut in_layouts: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    for name in model.layout_names()? {
        let l = model.load_layout(&name)?;
        for it in &l.items {
            in_layouts
                .entry(it.reference.clone())
                .or_default()
                .push(name.clone());
        }
    }

    let mut mit_preis: Vec<Kaufposten> = Vec::new();
    let mut ohne_preis: Vec<Kaufposten> = Vec::new();
    let mut unbekannte: std::collections::BTreeSet<String> = Default::default();
    for i in model.wishlist() {
        // Zwei Woerter bedeuten "dagegen entschieden", und beide gehoeren in keine
        // Kaufreihenfolge: `verworfen` ist abgelehnt, `ersetzt` ist von etwas anderem
        // abgeloest. Das zweite wird beim Import ohnehin zu `gone` — hier steht es, damit die
        // Liste vollstaendig ist und nicht von einer anderen Datei abhaengt.
        if matches!(i.prioritaet.as_deref(), Some("verworfen") | Some("ersetzt")) {
            continue;
        }
        if let Some(p) = i.prioritaet.as_deref() {
            if !BEKANNTE_PRIORITAETEN.contains(&p) {
                unbekannte.insert(p.to_string());
            }
        }
        let preis = i.preis_cent.or(i.kosten_min_cent);
        let posten = Kaufposten {
            id: i.id.clone(),
            label: i.label.clone(),
            prioritaet: i.prioritaet.clone(),
            preis_cent: preis,
            in_layouts: in_layouts.get(&i.id).cloned().unwrap_or_default(),
            kumuliert_cent: 0,
            erreichbar_nach_monaten: None,
        };
        if preis.is_some() {
            mit_preis.push(posten);
        } else {
            ohne_preis.push(posten);
        }
    }

    // Dringlichkeit, dann ob ein Entwurf darauf baut, dann der Preis. Der Preis steht zuletzt
    // und nicht zuerst: er entscheidet zwischen gleich dringenden Posten und nicht darueber,
    // was dringend ist.
    mit_preis.sort_by(|a, b| {
        rang(a.prioritaet.as_deref())
            .cmp(&rang(b.prioritaet.as_deref()))
            .then(b.in_layouts.len().cmp(&a.in_layouts.len()))
            .then(a.preis_cent.cmp(&b.preis_cent))
            .then(a.id.cmp(&b.id))
    });
    ohne_preis.sort_by(|a, b| {
        rang(a.prioritaet.as_deref())
            .cmp(&rang(b.prioritaet.as_deref()))
            .then(a.id.cmp(&b.id))
    });

    let mut summe = 0i64;
    for p in mit_preis.iter_mut() {
        summe += p.preis_cent.unwrap_or(0);
        p.kumuliert_cent = summe;
        p.erreichbar_nach_monaten = saldo.as_ref().and_then(|s| monate_bis_bezahlt(summe, s));
    }

    Ok(Kaufreihenfolge {
        posten: mit_preis,
        ohne_preis,
        saldo,
        unbekannte_prioritaeten: unbekannte.into_iter().collect(),
    })
}
