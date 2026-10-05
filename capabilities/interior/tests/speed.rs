//! Wie teuer eine Layoutpruefung ist. Kein Grenzwert, eine Messung — der Wert wandert in den
//! Bericht, damit eine Verlangsamung auffaellt, bevor sie in einer Suche zu Minuten wird.
//!
//! Gemessen wird an der Musterwohnung, nicht an der echten: dieser Test soll ueberall laufen.
//! Die historischen Zahlen unten stammen von einem anderen Grundriss (TypeScript mit linearem
//! Max-Scan 67,7 ms, dieselbe TS-Fassung mit Heap 2,7 ms, beide am 2026-08-30). Sie stehen
//! hier als Groessenordnung und nicht als Vergleichswert — ein anderer Raum ist ein anderes
//! Raster. Die Schranke ist entsprechend grob gesetzt: sie faengt einen Rueckfall auf den
//! linearen Scan, nicht ein paar Prozent.

use interior::clearance::check_layout;
use interior::model::Model;

#[test]
fn eine_layoutpruefung_kostet_wenige_millisekunden() {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/overlay");
    let db = std::env::temp_dir().join(format!("interior-speed-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&db);
    std::env::set_var("SJEL_PERSONAL_ROOT", &fixture);
    std::env::set_var("SJEL_DB_PATH", &db);
    let store = inventory::store::Store::open(&db).unwrap();
    inventory::import::inventory(&store, &fixture.join("data/interior/inventory")).unwrap();
    let model = Model::load("muster").unwrap();
    let layout = model.load_layout("a-frei").unwrap();

    check_layout(&model, &layout).unwrap(); // warmlaufen
    let n = 50;
    let t0 = std::time::Instant::now();
    for _ in 0..n {
        check_layout(&model, &layout).unwrap();
    }
    let per = t0.elapsed().as_secs_f64() * 1000.0 / n as f64;
    println!("check_layout: {per:.2} ms pro Aufruf (Musterwohnung, 600 x 450 cm bei 5 cm Raster)");
    assert!(per < 20.0, "{per:.2} ms — das ist die Groessenordnung des linearen Max-Scans, den der Heap abgeloest hat");
}
