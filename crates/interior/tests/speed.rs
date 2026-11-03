//! What a layout check costs. A measurement, not a limit: the value goes into the output so a
//! slowdown shows before it becomes minutes in a search. Measured on the sample flat so it runs
//! everywhere. The bound is coarse on purpose: it catches a fallback to the linear scan (67.7 ms
//! against 2.7 ms with the heap, measured 2026-08-30 on another plan), not a few percent.

use interior::clearance::check_layout;
use interior::model::Model;

#[test]
fn a_layout_check_costs_a_few_milliseconds() {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/overlay");
    let db = std::env::temp_dir().join(format!("interior-speed-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&db);
    std::env::set_var("SJEL_PERSONAL_ROOT", &fixture);
    std::env::set_var("SJEL_DB_PATH", &db);
    let store = sjel_inventory::store::Store::open(&db).unwrap();
    sjel_inventory::import::inventory(&store, &fixture.join("data/interior/inventory")).unwrap();
    let model = Model::load("muster").unwrap();
    let layout = model.load_layout("a-frei").unwrap();

    check_layout(&model, &layout).unwrap(); // warm up
    let n = 50;
    let t0 = std::time::Instant::now();
    for _ in 0..n {
        check_layout(&model, &layout).unwrap();
    }
    let per = t0.elapsed().as_secs_f64() * 1000.0 / n as f64;
    println!("check_layout: {per:.2} ms per call (sample flat, 600 x 450 cm at a 5 cm grid)");
    assert!(
        per < 20.0,
        "{per:.2} ms: the order of the linear max scan the heap replaced"
    );
}
