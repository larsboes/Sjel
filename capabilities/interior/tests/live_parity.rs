//! Parity against the recorded verdicts of the TypeScript predecessor, on the REAL flat.
//!
//! The baseline was taken from the TS engine on 2026-08-30 before it was deleted: the same ten
//! layouts, rules and numbers. If Rust deviates, either the port is wrong or a rule changed on
//! purpose, and then the baseline is re-recorded with a reason; the test is not loosened.
//!
//! It lives in the overlay next to the flat it describes
//! (`<overlay>/data/interior/flats/<id>/ts-baseline.json`), because it holds one flat's corridor
//! widths and sizes and this repository is public. Without `SJEL_PERSONAL_ROOT` the test says why
//! it did nothing and returns; `tests/engine.rs` checks the engine everywhere. Route widths depend
//! on the grid step (`geometry::RES` = 5 cm). The ten layouts now lie in the archive; see
//! `recorded_layout`.

use interior::clearance::check_layout;
use interior::layout_io;
use interior::model::{default_flat, Layout, Model, ModelError};
use serde_json::Value;

/// The active flat's model and its recorded baseline, or `None` with a reason.
/// `SJEL_PERSONAL_ROOT` is read, never set: the environment decides which overlay is meant.
fn live() -> Option<(Model, Value)> {
    sjel_config::env_var_os("SJEL_PERSONAL_ROOT")?;
    let flat = match default_flat() {
        Ok(f) => f,
        Err(e) => {
            eprintln!(
                "SJEL_PERSONAL_ROOT is set, but no flat can be chosen ({e}); skipped"
            );
            return None;
        }
    };
    let model = match Model::load(&flat) {
        Ok(m) => m,
        Err(e) => {
            eprintln!(
                "SJEL_PERSONAL_ROOT is set, but the model does not load ({e}); skipped"
            );
            return None;
        }
    };
    let path = model.flat_dir.join("ts-baseline.json");
    if !path.is_file() {
        eprintln!("no ts-baseline.json beside this flat; skipped");
        return None;
    }
    let text = std::fs::read_to_string(&path).expect("the baseline must be readable");
    Some((
        model,
        serde_json::from_str(&text).expect("the baseline is valid JSON"),
    ))
}

macro_rules! live_or_skip {
    () => {
        match live() {
            Some(v) => v,
            None => {
                eprintln!("set SJEL_PERSONAL_ROOT to check parity against the real flat; skipped");
                return;
            }
        }
    };
}

/// Loads a recorded layout: first `layouts/<id>.toml`, then `layouts/archiv/<id>.toml`.
/// Archiving moves rather than deletes, and all ten recorded layouts were archived on 2026-08-31,
/// so this finds them there. The path is resolved here rather than through
/// `Model::load_layout("archiv/…")`, because layout names may not contain separators.
fn recorded_layout(model: &Model, id: &str) -> Result<Layout, ModelError> {
    let dir = model.layouts_dir();
    let file = format!("{id}.toml");
    let pfad = [dir.join(&file), dir.join(layout_io::ARCHIVE).join(&file)]
        .into_iter()
        .find(|p| p.is_file())
        .ok_or_else(|| {
            ModelError::Missing(format!(
                "neither layouts/{file} nor layouts/{}/{file}",
                layout_io::ARCHIVE
            ))
        })?;
    let text = std::fs::read_to_string(&pfad).map_err(|source| ModelError::Read {
        path: pfad.clone(),
        source,
    })?;
    let mut layout: Layout =
        toml::from_str(&text).map_err(|source| ModelError::Parse { path: pfad, source })?;
    // As in `Model::load_layout`: the id is the file name.
    layout.id = id.to_string();
    Ok(layout)
}

#[test]
fn every_layout_matches_the_template_result() {
    let (model, base) = live_or_skip!();
    let layouts = base["layouts"].as_object().expect("layouts");
    let mut checked = 0;
    let mut deviations: Vec<String> = Vec::new();

    for (name, want) in layouts {
        let layout = recorded_layout(&model, name).unwrap_or_else(|e| panic!("{name}: {e}"));
        let got = check_layout(&model, &layout).unwrap_or_else(|e| panic!("{name}: {e}"));

        let want_pass = want["pass"].as_bool().unwrap();
        if got.pass != want_pass {
            deviations.push(format!("{name}: pass {} statt {}", got.pass, want_pass));
        }
        // Ids, not just counts: comparing `hard.len()` alone would miss a renamed violation.
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
                .map(|x| english_rule_id(x["rule"].as_str().unwrap()))
                .collect();
            v.sort();
            v
        };
        // Deliberate deviations are recorded in the baseline itself, in the overlay next to the flat,
        // under a second key; the recorded values stay unchanged.
        let allowed = |key: &str| -> Vec<String> {
            base["abweichungen"][name][format!("{key}_zusaetzlich")]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(english_rule_id))
                        .collect()
                })
                .unwrap_or_default()
        };
        for (key, got_v) in [("hard", &got.hard), ("soft", &got.soft)] {
            let (a, mut b) = (ids(got_v), want_ids(key));
            b.extend(allowed(key));
            b.sort();
            if a != b {
                deviations.push(format!(
                    "{name}: {key} meldet [{}] statt [{}]",
                    a.join(", "),
                    b.join(", ")
                ));
            }
        }
        // Corridor widths are the most sensitive number: they depend on grid, distance field and path
        // search at once.
        let want_c = want["corridors"].as_array().unwrap();
        for (i, wc) in want_c.iter().enumerate() {
            let Some(gc) = got.metrics.corridors.get(i) else {
                deviations.push(format!("{name}: corridor {i} is missing"));
                continue;
            };
            let w = wc["widthCm"].as_i64().map(|v| v as i32);
            if gc.width_cm != w {
                deviations.push(format!(
                    "{name}: Route {} → {} misst {:?} cm statt {:?}",
                    gc.from, gc.to, gc.width_cm, w
                ));
            }
        }
        checked += 1;
    }

    assert!(checked == 10, "10 layouts expected, {checked} checked");
    assert!(
        deviations.is_empty(),
        "deviations from the baseline:\n  {}",
        deviations.join("\n  ")
    );
}

#[test]
fn the_catalogue_is_taken_over_completely() {
    let (model, base) = live_or_skip!();
    let want = base["catalogue"].as_object().unwrap();
    let mut missing: Vec<&String> = want
        .keys()
        .filter(|k| !model.catalogue.contains_key(*k))
        .collect();
    missing.sort();
    // A superset, not an equal count: the catalogue grows with every wish and garment.
    assert!(missing.is_empty(), "missing from the catalogue: {missing:?}");
}

#[test]
fn every_piece_keeps_its_size() {
    let (model, base) = live_or_skip!();
    let want = base["catalogue"].as_object().unwrap();
    let mut dev = Vec::new();
    for (id, w) in want {
        let Some(item) = model.catalogue.get(id) else {
            continue;
        };
        let wb = w["b"].as_i64().map(|v| v as i32);
        let wt = w["t"].as_i64().map(|v| v as i32);
        if item.width != wb || item.depth != wt {
            dev.push(format!(
                "{id}: {:?}×{:?} instead of {:?}×{:?}",
                item.width, item.depth, wb, wt
            ));
        }
    }
    assert!(dev.is_empty(), "sizes differ:\n  {}", dev.join("\n  "));
}

/// The baseline was recorded with the German rule ids.
fn english_rule_id(id: &str) -> String {
    interior::clearance::LEGACY_RULE_IDS
        .iter()
        .find(|(old, _)| *old == id)
        .map_or(id, |(_, new)| new)
        .to_string()
}
