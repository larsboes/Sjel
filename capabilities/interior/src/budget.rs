//! The purchase order of the open needs. The monthly balance it is measured against is
//! inventory's, read from the same store.


pub use sjel_inventory::budget::{months_to_pay, monthly_balance, MonthlyBalance};


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
fn rang(priority: Option<&str>) -> u8 {
    match priority {
        Some("decided") => 0,
        Some("required") => 1,
        Some("recommended") => 2,
        Some("candidate") => 3,
        Some("concept") => 4,
        // Ohne Angabe ist die Dringlichkeit unbekannt und nicht niedrig: sie steht hinter allem
        // Eingeordneten und trotzdem VOR dem ausdruecklich Verschobenen. Schweigen ist keine
        // Entscheidung, `zurueckgestellt` ist eine.
        None => 5,
        Some("deferred") => 6,
        Some(_) => 7,
    }
}

/// Die Woerter, die diese Rangfolge kennt. Wer hier fehlt, wird gemeldet statt einsortiert.
pub const KNOWN_PRIORITIES: &[&str] = &[
    "decided",
    "required",
    "recommended",
    "candidate",
    "concept",
    "deferred",
    // Nicht in der Reihenfolge, aber bekannt: gegen beide ist entschieden.
    "rejected",
    "replaced",
];

#[derive(Debug, Clone, serde::Serialize)]
pub struct PurchaseLine {
    pub id: String,
    pub label: String,
    pub priority: Option<String>,
    /// Der Preis, mit dem gerechnet wird: `preis_cent`, sonst die untere Kante der Schaetzung.
    pub price_cents: Option<i64>,
    /// In welchen Layouts dieses Stueck schon eingeplant ist.
    ///
    /// Ein Bedarf, auf den ein Entwurf baut, ist ein anderer als eine Idee: ohne ihn ist das
    /// Layout kein Plan, sondern eine Zeichnung. Deshalb steht er vor gleich dringenden.
    pub in_layouts: Vec<String>,
    /// Die Summe aller Posten bis hierher, diesen eingeschlossen.
    pub cumulative_cents: i64,
    /// Nach wie vielen Monatssalden dieser Posten erreicht ist.
    ///
    /// `None`, wenn kein Saldo vorliegt oder er nicht positiv ist — dieselbe Haltung wie
    /// `monate_bis_bezahlt`: aus einem Saldo, aus dem nichts uebrig bleibt, spart niemand an.
    pub affordable_after_months: Option<f64>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PurchaseOrder {
    pub posten: Vec<PurchaseLine>,
    /// Bedarfe ohne jeden Preis. Sie stehen NICHT in der Reihenfolge.
    ///
    /// Sie mit null anzusetzen hiesse, sie zuerst zu kaufen, weil sie nichts kosten. Die
    /// Wunschliste zaehlt sie schon als Warnung (PRD B29); hier sind sie das, was einer
    /// Planung im Weg steht, und die Handlung dazu ist ein Preis und kein Kauf.
    pub unpriced: Vec<PurchaseLine>,
    pub balance: Option<MonthlyBalance>,
    /// Prioritaetswoerter aus den Daten, die diese Rangfolge nicht kennt.
    ///
    /// Sie stehen hinten und werden **gemeldet**, statt still auf einem Rang zu landen. Genau
    /// das ist am 2026-08-31 passiert: drei der acht Woerter der echten Wohnung fehlten in
    /// `rang`, und die Reihenfolge sah trotzdem sortiert aus.
    pub unknown_priorities: Vec<String>,
}

/// Was zuerst, was danach — und wann es erreicht ist.
///
/// Kein Budget und keine Empfehlung: eine Reihenfolge aus den Feldern, die schon da sind, und
/// eine Zeitachse aus einer Messung. Was davon gekauft wird, entscheidet niemand hier.
pub fn kaufreihenfolge(
    model: &crate::model::Model,
    balance: Option<MonthlyBalance>,
) -> Result<PurchaseOrder, crate::model::ModelError> {
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

    let mut priced: Vec<PurchaseLine> = Vec::new();
    let mut unpriced: Vec<PurchaseLine> = Vec::new();
    let mut unknown: std::collections::BTreeSet<String> = Default::default();
    for i in model.wishlist() {
        // Zwei Woerter bedeuten "dagegen entschieden", und beide gehoeren in keine
        // Kaufreihenfolge: `verworfen` ist abgelehnt, `ersetzt` ist von etwas anderem
        // abgeloest. Das zweite wird beim Import ohnehin zu `gone` — hier steht es, damit die
        // Liste vollstaendig ist und nicht von einer anderen Datei abhaengt.
        if matches!(i.priority.as_deref(), Some("rejected") | Some("replaced")) {
            continue;
        }
        if let Some(p) = i.priority.as_deref() {
            if !KNOWN_PRIORITIES.contains(&p) {
                unknown.insert(p.to_string());
            }
        }
        let price = i.price_cents.or(i.cost_min_cents);
        let posten = PurchaseLine {
            id: i.id.clone(),
            label: i.label.clone(),
            priority: i.priority.clone(),
            price_cents: price,
            in_layouts: in_layouts.get(&i.id).cloned().unwrap_or_default(),
            cumulative_cents: 0,
            affordable_after_months: None,
        };
        if price.is_some() {
            priced.push(posten);
        } else {
            unpriced.push(posten);
        }
    }

    // Dringlichkeit, dann ob ein Entwurf darauf baut, dann der Preis. Der Preis steht zuletzt
    // und nicht zuerst: er entscheidet zwischen gleich dringenden Posten und nicht darueber,
    // was dringend ist.
    priced.sort_by(|a, b| {
        rang(a.priority.as_deref())
            .cmp(&rang(b.priority.as_deref()))
            .then(b.in_layouts.len().cmp(&a.in_layouts.len()))
            .then(a.price_cents.cmp(&b.price_cents))
            .then(a.id.cmp(&b.id))
    });
    unpriced.sort_by(|a, b| {
        rang(a.priority.as_deref())
            .cmp(&rang(b.priority.as_deref()))
            .then(a.id.cmp(&b.id))
    });

    let mut total = 0i64;
    for p in priced.iter_mut() {
        total += p.price_cents.unwrap_or(0);
        p.cumulative_cents = total;
        p.affordable_after_months = balance.as_ref().and_then(|s| months_to_pay(total, s));
    }

    Ok(PurchaseOrder {
        posten: priced,
        unpriced,
        balance,
        unknown_priorities: unknown.into_iter().collect(),
    })
}
