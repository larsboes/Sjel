//! Wish comparison against a real SQLite file: specs and needs replace, a price is recorded once
//! per source and day, and the first price at or below the limit is the only alert.

use sjel_inventory::store::compare::Spec;
use sjel_inventory::store::{Item, Kind, Store};
use std::path::PathBuf;

fn temp_file(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "inventory-compare-{name}-{}.db",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    path
}

fn wish(id: &str, limit: Option<i64>) -> Item {
    Item {
        id: id.into(),
        kind: Kind::Gear,
        label: id.into(),
        cost_max_cents: limit,
        ..Item::default()
    }
}

fn spec(key: &str, value: f64) -> Spec {
    Spec {
        key: key.into(),
        value,
        unit: String::new(),
    }
}

#[test]
fn a_group_compares_its_candidates() {
    let path = temp_file("group");
    let store = Store::open(&path).unwrap();
    store.upsert_item(&wish("small", Some(150_000))).unwrap();
    store.upsert_item(&wish("large", None)).unwrap();
    store
        .create_or_update_wish_group("server", "Home server", "")
        .unwrap();
    store
        .set_wish_group_items("server", &["small".into(), "large".into()])
        .unwrap();

    store
        .set_specs(
            "small",
            &[spec("memory_gb", 32.0), spec("bandwidth_gb_s", 170.0)],
        )
        .unwrap();
    store
        .set_specs("large", &[spec("memory_gb", 64.0)])
        .unwrap();
    // A second write replaces the first, it does not add to it.
    store
        .set_specs("large", &[spec("memory_gb", 48.0)])
        .unwrap();
    store
        .set_needs("server", &[spec("memory_gb", 29.0)])
        .unwrap();
    assert!(store
        .set_specs("missing", &[spec("memory_gb", 1.0)])
        .is_err());
    assert!(store
        .set_needs("missing", &[spec("memory_gb", 1.0)])
        .is_err());
    assert!(store.set_specs("small", &[spec("Memory", 1.0)]).is_err());

    let groups = store.comparisons().unwrap();
    assert_eq!(groups.len(), 1);
    let group = &groups[0];
    assert_eq!(group.needs, vec![spec("memory_gb", 29.0)]);
    assert_eq!(group.candidates.len(), 2);
    let large = group.candidates.iter().find(|c| c.id == "large").unwrap();
    assert_eq!(large.specs, vec![spec("memory_gb", 48.0)]);
    let small = group.candidates.iter().find(|c| c.id == "small").unwrap();
    assert_eq!(small.specs.len(), 2);
    assert_eq!(small.cost_max_cents, Some(150_000));

    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_price_is_recorded_once_a_day_and_alerts_once() {
    let path = temp_file("prices");
    let store = Store::open(&path).unwrap();
    store.upsert_item(&wish("small", Some(150_000))).unwrap();
    let src = "https://geizhals.de/x-a1.html";

    let first = store
        .record_price("small", src, 158_900, "32/512", Some("2026-11-03"))
        .unwrap();
    assert!(first.added && !first.alert);
    // The same source on the same day: ignored, even at another price.
    let again = store
        .record_price(
            "small",
            src,
            140_000,
            "32/512",
            Some("2026-11-03T18:00:00Z"),
        )
        .unwrap();
    assert!(!again.added && !again.alert);
    // Another source on the same day counts.
    let manual = store
        .record_price("small", "kleinanzeigen", 160_000, "", Some("2026-11-03"))
        .unwrap();
    assert!(manual.added);

    let at_limit = store
        .record_price("small", src, 150_000, "32/512", Some("2026-11-04"))
        .unwrap();
    assert!(at_limit.added && at_limit.alert, "at the limit is buyable");
    let lower = store
        .record_price("small", src, 139_900, "32/512", Some("2026-11-05"))
        .unwrap();
    assert!(lower.added && !lower.alert, "the limit was reached once");

    assert!(store.record_price("small", src, 0, "", None).is_err());
    assert!(store.record_price("small", " ", 1, "", None).is_err());
    assert!(store.record_price("missing", src, 1, "", None).is_err());
    assert!(store
        .record_price("small", src, 1, "", Some("03.11.2026"))
        .is_err());
    // No date: today, and it is stored.
    assert!(
        store
            .record_price("small", "today", 1, "", None)
            .unwrap()
            .added
    );

    store.create_or_update_wish_group("g", "G", "").unwrap();
    store.set_wish_group_items("g", &["small".into()]).unwrap();
    let history = &store.comparisons().unwrap()[0].candidates[0].observations;
    assert_eq!(history.len(), 5);
    assert_eq!(history.iter().filter(|o| o.alert).count(), 1);
    assert_eq!(history[0].observed_at, "2026-11-03");

    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_group_of_alternatives_costs_its_cheapest_member() {
    let path = temp_file("alternatives");
    let store = Store::open(&path).unwrap();
    for (id, cents) in [("m6", 140_102), ("m5", 247_000)] {
        let mut item = wish(id, None);
        item.price_cents = Some(cents);
        store.upsert_item(&item).unwrap();
    }
    store
        .create_or_update_wish_group("server", "Home server", "")
        .unwrap();
    store
        .set_wish_group_items("server", &["m6".into(), "m5".into()])
        .unwrap();
    let total = |store: &Store| store.wish_groups().unwrap()[0].total_low_cents;
    assert_eq!(total(&store), 387_102, "unmarked, the members add up");

    store.set_alternatives("server", true).unwrap();
    assert!(store.wish_groups().unwrap()[0].alternatives);
    assert_eq!(total(&store), 140_102);
    assert_eq!(
        store.alternative_groups().unwrap(),
        vec![vec!["m5".to_string(), "m6".to_string()]]
    );
    assert!(store.set_alternatives("missing", true).is_err());

    let _ = std::fs::remove_file(&path);
}
