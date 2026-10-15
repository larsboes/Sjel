//! `traits` survive a write and a read unchanged; the packing list shows them.

use sjel_inventory::store::{Item, Kind, Store};

#[test]
fn traits_survive_write_and_read() {
    let path = std::env::temp_dir().join(format!("inventory-traits-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let store = Store::open(&path).unwrap();

    let jacket = Item {
        id: "jacket".into(),
        kind: Kind::Piece,
        label: "Rain jacket".into(),
        category: Some("kleidung".into()),
        traits: vec!["rain".into(), "packable".into()],
        ..Item::default()
    };
    store.upsert_item(&jacket).unwrap();
    let (read, _) = store.item("jacket").unwrap().unwrap();
    assert_eq!(
        read.traits,
        vec!["rain".to_string(), "packable".to_string()]
    );

    // Without a value the list stays empty, not null.
    let shirt = Item {
        id: "shirt".into(),
        label: "Shirt".into(),
        ..Item::default()
    };
    store.upsert_item(&shirt).unwrap();
    assert!(store.item("shirt").unwrap().unwrap().0.traits.is_empty());

    let _ = std::fs::remove_file(&path);
}
