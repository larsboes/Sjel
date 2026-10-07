//! `traits`: was ein Stueck auf einer Reise leistet (2026-10-07). Die Packliste zeigt diese
//! Woerter neben jedem Stueck; sie muessen ein Schreiben und ein Lesen unveraendert ueberstehen.

use sjel_inventory::store::{Item, Kind, Store};

#[test]
fn traits_ueberstehen_schreiben_und_lesen() {
    let pfad = std::env::temp_dir().join(format!("inventory-traits-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&pfad);
    let store = Store::open(&pfad).unwrap();

    let jacke = Item {
        id: "jacke".into(),
        kind: Kind::Piece,
        label: "Regenjacke".into(),
        category: Some("kleidung".into()),
        traits: vec!["rain".into(), "packable".into()],
        ..Item::default()
    };
    store.upsert_item(&jacke).unwrap();
    let (gelesen, _) = store.item("jacke").unwrap().unwrap();
    assert_eq!(
        gelesen.traits,
        vec!["rain".to_string(), "packable".to_string()]
    );

    // Ohne Angabe bleibt die Liste leer, nicht null.
    let hemd = Item {
        id: "hemd".into(),
        label: "Hemd".into(),
        ..Item::default()
    };
    store.upsert_item(&hemd).unwrap();
    assert!(store.item("hemd").unwrap().unwrap().0.traits.is_empty());

    let _ = std::fs::remove_file(&pfad);
}
