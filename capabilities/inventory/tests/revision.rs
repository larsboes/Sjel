//! Die Revision eines Eintrags und das Schreiben gegen sie (PRD §10 A5, Q110).
//!
//! Telefon und Mac bearbeiten dieselben Eintraege. Ohne Revision gewinnt der spaetere
//! Schreiber still; mit ihr wird der zweite abgewiesen und bekommt den Stand, der jetzt gilt.
//! Geprueft gegen eine echte SQLite-Datei, weil die Eigenschaft, um die es geht — Vergleich
//! und Schreiben in einem Statement — in einem Mock nicht vorkommt.

use inventory::store::{Item, Kind, Schreibergebnis, Store};
use std::path::PathBuf;
use std::sync::{Arc, Barrier};

fn tempdatei(name: &str) -> PathBuf {
    let pfad = std::env::temp_dir().join(format!(
        "inventory-revision-{name}-{}.db",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&pfad);
    pfad
}

fn schrank(label: &str) -> Item {
    Item {
        id: "schrank".into(),
        kind: Kind::Piece,
        label: label.into(),
        b: Some(100),
        ..Item::default()
    }
}

#[test]
fn jedes_schreiben_erhoeht_die_revision() {
    let pfad = tempdatei("zaehlt");
    let store = Store::open(&pfad).unwrap();

    assert_eq!(store.upsert_item(&schrank("Schrank")).unwrap(), 1);
    assert_eq!(store.upsert_item(&schrank("Schrank")).unwrap(), 2);
    let (gelesen, _) = store.item("schrank").unwrap().unwrap();
    assert_eq!(gelesen.revision, 2);

    // Eine Revision im Rumpf wird nie geschrieben: sie gehoert dem Server.
    let mut falsch = schrank("Schrank");
    falsch.revision = 99;
    assert_eq!(store.upsert_item(&falsch).unwrap(), 3);

    let _ = std::fs::remove_file(&pfad);
}

#[test]
fn eine_passende_revision_schreibt_und_eine_veraltete_nicht() {
    let pfad = tempdatei("cas");
    let store = Store::open(&pfad).unwrap();
    store.upsert_item(&schrank("alt")).unwrap();

    match store.update_item_if_revision(&schrank("neu"), 1).unwrap() {
        Schreibergebnis::Geschrieben(r) => assert_eq!(r, 2),
        anders => panic!("erwartet geschrieben, bekam {anders:?}"),
    }
    match store
        .update_item_if_revision(&schrank("zu spaet"), 1)
        .unwrap()
    {
        Schreibergebnis::Veraltet(aktuell, _) => {
            assert_eq!(aktuell.revision, 2);
            assert_eq!(
                aktuell.label, "neu",
                "der veraltete Schreiber hat nichts geaendert"
            );
        }
        anders => panic!("erwartet veraltet, bekam {anders:?}"),
    }
    let mut fehlt = schrank("x");
    fehlt.id = "gibt-es-nicht".into();
    assert!(matches!(
        store.update_item_if_revision(&fehlt, 1).unwrap(),
        Schreibergebnis::Fehlt
    ));
    assert!(
        store.item("gibt-es-nicht").unwrap().is_none(),
        "ein bedingtes Schreiben legt nichts an"
    );

    let _ = std::fs::remove_file(&pfad);
}

/// Der eigentliche Fall: zwei Geraete haben Revision 1 gelesen und schreiben gleichzeitig.
///
/// Jeder Schreiber hat seinen eigenen `Store` und damit seinen eigenen Verbindungspool, wie
/// zwei Prozesse. Die Schranke laesst alle zugleich los; mehrere Runden, damit ein Test, der
/// nur zufaellig nacheinander laeuft, nicht als Beweis durchgeht.
#[test]
fn von_gleichzeitigen_schreibern_mit_derselben_revision_gewinnt_genau_einer() {
    const SCHREIBER: usize = 8;
    const RUNDEN: usize = 5;
    let pfad = tempdatei("rennen");
    Store::open(&pfad)
        .unwrap()
        .upsert_item(&schrank("start"))
        .unwrap();

    for runde in 0..RUNDEN {
        let gelesen = Store::open(&pfad)
            .unwrap()
            .item("schrank")
            .unwrap()
            .unwrap()
            .0
            .revision;
        let schranke = Arc::new(Barrier::new(SCHREIBER));
        let faeden: Vec<_> = (0..SCHREIBER)
            .map(|n| {
                let pfad = pfad.clone();
                let schranke = schranke.clone();
                std::thread::spawn(move || {
                    let store = Store::open(&pfad).unwrap();
                    let label = format!("runde {runde} schreiber {n}");
                    schranke.wait();
                    let ergebnis = store
                        .update_item_if_revision(&schrank(&label), gelesen)
                        .unwrap();
                    (label, ergebnis)
                })
            })
            .collect();
        let ergebnisse: Vec<_> = faeden.into_iter().map(|f| f.join().unwrap()).collect();

        let gewinner: Vec<_> = ergebnisse
            .iter()
            .filter_map(|(label, e)| match e {
                Schreibergebnis::Geschrieben(r) => Some((label.clone(), *r)),
                _ => None,
            })
            .collect();
        assert_eq!(gewinner.len(), 1, "Runde {runde}: {ergebnisse:?}");
        let (sieger, revision) = &gewinner[0];
        assert_eq!(*revision, gelesen + 1);
        for (_, e) in &ergebnisse {
            match e {
                Schreibergebnis::Geschrieben(_) => {}
                Schreibergebnis::Veraltet(aktuell, _) => {
                    assert_eq!(aktuell.revision, gelesen + 1);
                    assert_eq!(&aktuell.label, sieger);
                }
                Schreibergebnis::Fehlt => panic!("Runde {runde}: der Eintrag fehlt"),
            }
        }
        let (danach, _) = Store::open(&pfad)
            .unwrap()
            .item("schrank")
            .unwrap()
            .unwrap();
        assert_eq!(&danach.label, sieger);
        assert_eq!(danach.revision, gelesen + 1);
    }

    let _ = std::fs::remove_file(&pfad);
}
