//! Paritaet gegen die aufgezeichneten Verdikte der TypeScript-Vorlage — an der ECHTEN Wohnung.
//!
//! Die Baseline wurde am 2026-08-30 aus der TS-Engine gezogen, bevor sie geloescht wurde. Sie
//! ist kein zweiter Pruefer, sondern ein Protokoll: dieselben zehn Layouts, dieselben Regeln,
//! dieselben Zahlen. Weicht Rust ab, ist entweder die Portierung falsch oder eine Regel hat
//! sich absichtlich geaendert — und dann gehoert die Baseline mit einer Begruendung neu
//! aufgezeichnet, nicht der Test entschaerft.
//!
//! Warum sie im Overlay liegt und nicht hier: sie enthaelt die Korridorbreiten und Moebelmasse
//! EINER Wohnung. Das ist dieselbe Kategorie wie das Raummodell selbst, und dieses Repository
//! ist oeffentlich. Sie liegt deshalb neben der Wohnung, die sie beschreibt, unter
//! `<overlay>/data/interior/flats/<id>/ts-baseline.json`.
//!
//! Ohne `SJEL_PERSONAL_ROOT` meldet dieser Test, warum er nichts getan hat, und kehrt zurueck —
//! der Zustand in CI und auf jedem Rechner, der die Dateien nicht haelt. Die Maschine selbst
//! ist davon unabhaengig geprueft: `tests/engine.rs` laeuft ueberall.
//!
//! Die Routenbreiten haengen an der Rasterweite (geometry::RES = 5 cm). Wer sie aendert,
//! aendert diese Zahlen.
//!
//! Die zehn Layouts stehen seit der Kuratierung vom 2026-08-31 nicht mehr in der Liste, sondern
//! im Archiv daneben. Wie sie trotzdem gefunden werden, steht bei `aufgezeichnetes_layout`.

use interior::clearance::check_layout;
use interior::layout_io;
use interior::model::{default_flat, Layout, Model, ModelError};
use serde_json::Value;

/// Das Modell der aktiven Wohnung und ihre aufgezeichnete Vorlage, oder `None` mit einem Grund.
/// `SJEL_PERSONAL_ROOT` wird gelesen und nie gesetzt: welches Overlay gemeint ist, entscheidet
/// die Umgebung, nicht der Test.
fn live() -> Option<(Model, Value)> {
    sjel_config::env_var_os("SJEL_PERSONAL_ROOT")?;
    let flat = match default_flat() {
        Ok(f) => f,
        Err(e) => {
            eprintln!(
                "SJEL_PERSONAL_ROOT ist gesetzt, aber keine Wohnung waehlbar ({e}); uebersprungen"
            );
            return None;
        }
    };
    let model = match Model::load(&flat) {
        Ok(m) => m,
        Err(e) => {
            eprintln!(
                "SJEL_PERSONAL_ROOT ist gesetzt, aber das Modell laedt nicht ({e}); uebersprungen"
            );
            return None;
        }
    };
    let path = model.flat_dir.join("ts-baseline.json");
    if !path.is_file() {
        eprintln!("keine ts-baseline.json neben dieser Wohnung; uebersprungen");
        return None;
    }
    let text = std::fs::read_to_string(&path).expect("die Baseline muss lesbar sein");
    Some((
        model,
        serde_json::from_str(&text).expect("Baseline ist gueltiges JSON"),
    ))
}

macro_rules! live_or_skip {
    () => {
        match live() {
            Some(v) => v,
            None => {
                eprintln!("setze SJEL_PERSONAL_ROOT, um die Paritaet gegen die echte Wohnung zu pruefen; uebersprungen");
                return;
            }
        }
    };
}

/// Ein aufgezeichnetes Layout laden: erst `layouts/<id>.toml`, dann `layouts/archiv/<id>.toml`.
///
/// **Archivierte Layouts bleiben vergleichbar** — `layout_io::archiviere` (src/layout_io.rs:164-174)
/// loescht nicht, sondern verschiebt nach `layouts/archiv/` und hebt die Datei genau dafuer
/// lesbar auf. Die Kuratierung vom 2026-08-31 hat alle zehn aufgezeichneten Layouts dorthin
/// gelegt; die Vorlage kennt sie unveraendert, also muss dieser Test sie unveraendert finden.
/// Der Vergleich verliert dabei keinen Fall: es bleiben zehn.
///
/// Warum das hier steht und nicht als `Model::load_layout("archiv/…")`: ein Layoutname wird zu
/// einem Dateinamen, und `layout_io::pruefe_id` laesst darin kein Trennzeichen zu — das ist die
/// Grenze, die einen Schreiber aus dem Netz in diesem Verzeichnis haelt. Ein Name mit `/` waere
/// die Ausnahme, die sie aufweicht. Wo eine Datei liegt, ist eine Frage des Ladens, also loest
/// dieser Test den Pfad selbst auf.
fn aufgezeichnetes_layout(model: &Model, id: &str) -> Result<Layout, ModelError> {
    let dir = model.layouts_dir();
    let datei = format!("{id}.toml");
    let pfad = [dir.join(&datei), dir.join(layout_io::ARCHIV).join(&datei)]
        .into_iter()
        .find(|p| p.is_file())
        .ok_or_else(|| {
            ModelError::Missing(format!(
                "weder layouts/{datei} noch layouts/{}/{datei}",
                layout_io::ARCHIV
            ))
        })?;
    let text = std::fs::read_to_string(&pfad).map_err(|source| ModelError::Read {
        path: pfad.clone(),
        source,
    })?;
    let mut layout: Layout =
        toml::from_str(&text).map_err(|source| ModelError::Parse { path: pfad, source })?;
    // Wie in `Model::load_layout`: die Kennung steht nicht in der Datei, sie ist der Dateiname.
    layout.id = id.to_string();
    Ok(layout)
}

#[test]
fn jedes_layout_faellt_gleich_aus_wie_in_der_vorlage() {
    let (model, base) = live_or_skip!();
    let layouts = base["layouts"].as_object().expect("layouts");
    let mut geprueft = 0;
    let mut abweichungen: Vec<String> = Vec::new();

    for (name, want) in layouts {
        let layout = aufgezeichnetes_layout(&model, name).unwrap_or_else(|e| panic!("{name}: {e}"));
        let got = check_layout(&model, &layout).unwrap_or_else(|e| panic!("{name}: {e}"));

        let want_pass = want["pass"].as_bool().unwrap();
        if got.pass != want_pass {
            abweichungen.push(format!("{name}: pass {} statt {}", got.pass, want_pass));
        }
        // Kennungen und nicht nur Anzahlen, seit 2026-08-31.
        //
        // Die Vorlage hat die Kennungen immer aufgezeichnet und dieser Test hat sie immer
        // weggeworfen: er verglich `hard.len()` gegen `hard.len()`. Damit war eine Umbenennung
        // fuer ihn unsichtbar — und genau eine war passiert. Die TypeScript-Fassung meldete
        // die Ausklappzone des Schlafsofas als **R8**, so wie jede rules.toml sie deklariert;
        // die Rust-Portierung nannte sie `couch_ausklappen` und hat die zweite Fassung damit
        // selbst erfunden. Zwei Verstoesse mit vertauschten Namen haetten hier bestanden.
        let ids = |vs: &[interior::clearance::Violation]| -> Vec<String> {
            let mut v: Vec<String> = vs.iter().map(|x| x.rule.clone()).collect();
            v.sort();
            v
        };
        let want_ids = |key: &str| -> Vec<String> {
            let mut v: Vec<String> = want[key]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x["rule"].as_str().unwrap().to_string())
                .collect();
            v.sort();
            v
        };
        // Bewusste Abweichungen stehen in der Vorlage selbst, im Overlay neben der Wohnung —
        // nicht hier. Sie beschreiben eine Wohnung, und dieser Test steht in einem oeffentlichen
        // Repository. Die Vorlage bleibt dabei unveraendert: `abweichungen` ist ein zweiter
        // Schluessel daneben, kein korrigierter Messwert.
        let erlaubt = |key: &str| -> Vec<String> {
            base["abweichungen"][name][format!("{key}_zusaetzlich")]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default()
        };
        for (key, got_v) in [("hard", &got.hard), ("soft", &got.soft)] {
            let (a, mut b) = (ids(got_v), want_ids(key));
            b.extend(erlaubt(key));
            b.sort();
            if a != b {
                abweichungen.push(format!(
                    "{name}: {key} meldet [{}] statt [{}]",
                    a.join(", "),
                    b.join(", ")
                ));
            }
        }
        // Die Korridorbreiten sind die empfindlichste Zahl im ganzen System: sie haengen an
        // Raster, Distanzfeld und Wegsuche zugleich. Stimmen sie, stimmt der Kern.
        let want_c = want["corridors"].as_array().unwrap();
        for (i, wc) in want_c.iter().enumerate() {
            let Some(gc) = got.metrics.corridors.get(i) else {
                abweichungen.push(format!("{name}: Korridor {i} fehlt"));
                continue;
            };
            let w = wc["widthCm"].as_i64().map(|v| v as i32);
            if gc.width_cm != w {
                abweichungen.push(format!(
                    "{name}: Route {} → {} misst {:?} cm statt {:?}",
                    gc.from, gc.to, gc.width_cm, w
                ));
            }
        }
        geprueft += 1;
    }

    assert!(geprueft == 10, "10 Layouts erwartet, {geprueft} geprueft");
    assert!(
        abweichungen.is_empty(),
        "Abweichungen zur Vorlage:\n  {}",
        abweichungen.join("\n  ")
    );
}

#[test]
fn der_katalog_ist_vollstaendig_uebernommen() {
    let (model, base) = live_or_skip!();
    let want = base["catalogue"].as_object().unwrap();
    let mut fehlend: Vec<&String> = want
        .keys()
        .filter(|k| !model.catalogue.contains_key(*k))
        .collect();
    fehlend.sort();
    assert!(fehlend.is_empty(), "im TOML-Inventar fehlen: {fehlend:?}");
    assert_eq!(
        model.catalogue.len(),
        want.len(),
        "Katalog hat {} Eintraege, die Vorlage {}",
        model.catalogue.len(),
        want.len()
    );
}

#[test]
fn die_masse_jedes_moebels_sind_unveraendert() {
    let (model, base) = live_or_skip!();
    let want = base["catalogue"].as_object().unwrap();
    let mut abw = Vec::new();
    for (id, w) in want {
        let Some(item) = model.catalogue.get(id) else {
            continue;
        };
        let wb = w["b"].as_i64().map(|v| v as i32);
        let wt = w["t"].as_i64().map(|v| v as i32);
        if item.b != wb || item.t != wt {
            abw.push(format!(
                "{id}: {:?}×{:?} statt {:?}×{:?}",
                item.b, item.t, wb, wt
            ));
        }
    }
    assert!(abw.is_empty(), "Masse weichen ab:\n  {}", abw.join("\n  "));
}
