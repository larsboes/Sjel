//! Die Kaufreihenfolge gegen das Musterinventar.

// ---------------------------------------------------------------- die Reihenfolge

/// Fuer die Reihenfolge braucht es das Musterinventar, nicht nur erfundene Buchungen: sie
/// sortiert Zeilen und rechnet nicht mit ihnen.
fn model() -> interior::model::Model {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let fixture =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/overlay");
        let db = std::env::temp_dir().join(format!("interior-budget-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&db);
        std::env::set_var("SJEL_PERSONAL_ROOT", &fixture);
        std::env::set_var("SJEL_DB_PATH", &db);
        std::env::remove_var("SJEL_INTERIOR_FLAT");
        let store = sjel_inventory::store::Store::open(&db).expect("die Testdatenbank oeffnet");
        sjel_inventory::import::inventory(&store, &fixture.join("data/interior/inventory"))
            .expect("das Musterinventar importiert");
    });
    interior::model::Model::load("muster").expect("muster")
}

/// Dringlichkeit schlaegt Preis, und ein Entwurf schlaegt eine Idee.
///
/// Die Fixture ist so gebaut, dass sich die drei Stufen widersprechen: `nachttisch` ist
/// Pflicht und teuer, `wandregal` nur Empfehlung und billig, `bilderleiste` Konzept und am
/// billigsten. Eine Sortierung nach Preis kaeme genau umgekehrt heraus — deshalb ist diese
/// Reihenfolge eine Pruefung und keine Aufzeichnung.
#[test]
fn dringlichkeit_steht_vor_preis() {
    let m = model();
    let r = interior::budget::kaufreihenfolge(&m, None).expect("Reihenfolge");
    let ids: Vec<&str> = r.posten.iter().map(|p| p.id.as_str()).collect();
    let pos = |id: &str| ids.iter().position(|x| *x == id).expect(id);
    assert!(
        pos("nachttisch") < pos("wandregal"),
        "Pflicht vor Empfehlung, auch wenn sie teurer ist: {ids:?}"
    );
    assert!(
        pos("wandregal") < pos("bilderleiste"),
        "Empfehlung vor Konzept: {ids:?}"
    );
}

/// Die ENTSCHEIDUNGSachse zaehlt auch, und sie war bis 2026-08-31 unsichtbar.
///
/// `gesetzt` heisst "entschieden, das wird gekauft" und `zurueckgestellt` heisst "bewusst
/// verschoben". Beide Woerter stehen im Inventar der echten Wohnung, beide fehlten in `rang`,
/// und beide fielen deshalb auf denselben Rang wie alles Unbekannte — was zwischen ihnen
/// entschied, war der Preis. Hier ist der teure Posten der gesetzte und der billige der
/// zurueckgestellte, damit eine Preissortierung genau umgekehrt herauskaeme.
#[test]
fn eine_gefallene_entscheidung_steht_vor_einer_verschobenen() {
    let m = model();
    let r = interior::budget::kaufreihenfolge(&m, None).expect("Reihenfolge");
    let ids: Vec<&str> = r.posten.iter().map(|p| p.id.as_str()).collect();
    let pos = |id: &str| ids.iter().position(|x| *x == id).expect(id);
    assert_eq!(ids[0], "deckenlampe", "das Gesetzte zuerst: {ids:?}");
    assert_eq!(
        *ids.last().unwrap(),
        "garderobenhaken",
        "das Zurueckgestellte zuletzt: {ids:?}"
    );
    assert!(pos("deckenlampe") < pos("nachttisch"), "{ids:?}");
}

/// Ein Wort, das diese Rangfolge nicht kennt, wird gemeldet statt einsortiert.
///
/// Das ist die Lehre aus dem Fehler selbst: die erste Fassung kannte drei der acht Woerter und
/// sah trotzdem sortiert aus. Ein neues Wort in den Daten muss auffallen.
#[test]
fn ein_unbekanntes_prioritaetswort_wird_gemeldet() {
    let mut m = model();
    if let Some(i) = m.catalogue.get_mut("wandregal") {
        i.prioritaet = Some("vielleicht-irgendwann".into());
    }
    let r = interior::budget::kaufreihenfolge(&m, None).expect("Reihenfolge");
    assert_eq!(
        r.unbekannte_prioritaeten,
        vec!["vielleicht-irgendwann".to_string()]
    );
    let ids: Vec<&str> = r.posten.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(
        *ids.last().unwrap(),
        "wandregal",
        "was niemand einordnen kann, steht hinten: {ids:?}"
    );
}

/// Was abgeloest wurde, steht in keiner Kaufreihenfolge.
#[test]
fn ein_ersetzter_bedarf_faellt_heraus() {
    let m = model();
    let r = interior::budget::kaufreihenfolge(&m, None).expect("Reihenfolge");
    assert!(r
        .posten
        .iter()
        .chain(r.ohne_preis.iter())
        .all(|p| p.prioritaet.as_deref() != Some("ersetzt")));
}

/// Die kumulierte Summe waechst monoton und endet auf der Gesamtsumme.
#[test]
fn die_kumulierte_summe_zaehlt_wirklich_zusammen() {
    let m = model();
    let r = interior::budget::kaufreihenfolge(&m, None).expect("Reihenfolge");
    let mut letzte = 0;
    for p in &r.posten {
        assert!(p.kumuliert_cent >= letzte, "die Summe faellt: {:?}", p.id);
        letzte = p.kumuliert_cent;
    }
    let summe: i64 = r.posten.iter().map(|p| p.preis_cent.unwrap_or(0)).sum();
    assert_eq!(letzte, summe);
}

/// Ein Bedarf ohne Preis steht nicht in der Reihenfolge — und schon gar nicht vorn.
///
/// Mit null angesetzt waere er der billigste Posten und stuende zuerst; das waere eine
/// Kaufempfehlung, die aus einer fehlenden Zahl folgt. PRD B29 zaehlt sie in der Wunschliste
/// schon als Warnung, und hier ist die Handlung dazu ein Preis und kein Kauf.
#[test]
fn ein_bedarf_ohne_preis_steht_in_keiner_reihenfolge() {
    let m = model();
    let r = interior::budget::kaufreihenfolge(&m, None).expect("Reihenfolge");
    assert!(
        r.posten.iter().all(|p| p.preis_cent.is_some()),
        "in der Reihenfolge steht nur, was einen Preis hat"
    );
    assert!(
        r.ohne_preis.iter().any(|p| p.id == "teppich"),
        "und der Teppich steht daneben, mit Namen"
    );
}

/// Ohne positiven Saldo gibt es keine Monatszahl — dieselbe Haltung wie `monate_bis_bezahlt`.
#[test]
fn ohne_saldo_gibt_es_keine_zeitachse() {
    let m = model();
    let r = interior::budget::kaufreihenfolge(&m, None).expect("Reihenfolge");
    assert!(
        r.posten.iter().all(|p| p.erreichbar_nach_monaten.is_none()),
        "eine Zeitachse ohne Messung waere eine erfundene Zahl"
    );
}
