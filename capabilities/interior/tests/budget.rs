//! The purchase order against the sample inventory.

// ---------------------------------------------------------------- the order

/// The order needs the sample inventory, not just invented bookings: it sorts rows.
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
        let store = sjel_inventory::store::Store::open(&db).expect("the test database opens");
        sjel_inventory::import::inventory(&store, &fixture.join("data/interior/inventory"))
            .expect("the sample inventory imports");
    });
    interior::model::Model::load("muster").expect("muster")
}

/// Urgency beats price, and a layout beats an idea. The fixture makes the stages contradict
/// price: `nachttisch` is required and dear, `wandregal` recommended and cheap, `bilderleiste` a
/// concept and cheapest, so a price sort would come out reversed.
#[test]
fn urgency_comes_before_price() {
    let m = model();
    let r = interior::budget::purchase_order(&m, None).expect("Reihenfolge");
    let ids: Vec<&str> = r.lines.iter().map(|p| p.id.as_str()).collect();
    let pos = |id: &str| ids.iter().position(|x| *x == id).expect(id);
    assert!(
        pos("nachttisch") < pos("wandregal"),
        "required before recommended, even when dearer: {ids:?}"
    );
    assert!(
        pos("wandregal") < pos("bilderleiste"),
        "Empfehlung vor Konzept: {ids:?}"
    );
}

/// The decision axis counts too: `decided` means it will be bought, `deferred` means postponed
/// on purpose. The dear line is the decided one and the cheap line the deferred one, so a price
/// sort would come out reversed.
#[test]
fn a_made_decision_comes_before_a_deferred_one() {
    let m = model();
    let r = interior::budget::purchase_order(&m, None).expect("Reihenfolge");
    let ids: Vec<&str> = r.lines.iter().map(|p| p.id.as_str()).collect();
    let pos = |id: &str| ids.iter().position(|x| *x == id).expect(id);
    assert_eq!(ids[0], "deckenlampe", "the decided first: {ids:?}");
    assert_eq!(
        *ids.last().unwrap(),
        "garderobenhaken",
        "the deferred last: {ids:?}"
    );
    assert!(pos("deckenlampe") < pos("nachttisch"), "{ids:?}");
}

/// A word the order does not know is reported rather than sorted in.
#[test]
fn an_unknown_priority_word_is_reported() {
    let mut m = model();
    if let Some(i) = m.catalogue.get_mut("wandregal") {
        i.priority = Some("vielleicht-irgendwann".into());
    }
    let r = interior::budget::purchase_order(&m, None).expect("Reihenfolge");
    assert_eq!(
        r.unknown_priorities,
        vec!["vielleicht-irgendwann".to_string()]
    );
    let ids: Vec<&str> = r.lines.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(
        *ids.last().unwrap(),
        "wandregal",
        "what nobody can rank comes last: {ids:?}"
    );
}

/// What was replaced is in no purchase order.
#[test]
fn a_replaced_need_drops_out() {
    let m = model();
    let r = interior::budget::purchase_order(&m, None).expect("Reihenfolge");
    assert!(r
        .lines
        .iter()
        .chain(r.unpriced.iter())
        .all(|p| p.priority.as_deref() != Some("replaced")));
}

/// The cumulative total grows monotonically and ends at the total.
#[test]
fn the_cumulative_total_really_adds_up() {
    let m = model();
    let r = interior::budget::purchase_order(&m, None).expect("Reihenfolge");
    let mut letzte = 0;
    for p in &r.lines {
        assert!(p.cumulative_cents >= letzte, "the total falls: {:?}", p.id);
        letzte = p.cumulative_cents;
    }
    let total: i64 = r.lines.iter().map(|p| p.price_cents.unwrap_or(0)).sum();
    assert_eq!(letzte, total);
}

/// A need without a price is not in the order, least of all first: at zero it would be the cheapest
/// line.
#[test]
fn a_need_without_a_price_is_in_no_order() {
    let m = model();
    let r = interior::budget::purchase_order(&m, None).expect("Reihenfolge");
    assert!(
        r.lines.iter().all(|p| p.price_cents.is_some()),
        "only priced lines are in the order"
    );
    assert!(
        r.unpriced.iter().any(|p| p.id == "teppich"),
        "and the rug stands beside it, by name"
    );
}

/// Without a positive balance there is no month count, as with `months_to_pay`.
#[test]
fn without_a_balance_there_is_no_timeline() {
    let m = model();
    let r = interior::budget::purchase_order(&m, None).expect("Reihenfolge");
    assert!(
        r.lines.iter().all(|p| p.affordable_after_months.is_none()),
        "a timeline without a measurement would be an invented number"
    );
}
