//! A plan is created, and a plan leaves the list, without anything getting lost.
//!
//! Layouts are files, and a file's header carries the reasoning, for a rejected piece the only
//! place it is written. Hence the three cases: a header claiming an origin that does not exist, a
//! name pointing out of the dir, and a clearing-away that takes the reasoning with it. Works on a
//! copy of the sample flat, because it writes.

use interior::layout_io;
use interior::model::{Layout, Model, PlacedItem};
use std::path::{Path, PathBuf};
use std::sync::Once;

const MUSTER: &str = "muster";
/// One of the sample flat's layouts, as a template to copy.
const TEMPLATE: &str = "a-frei";

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/overlay")
}

/// Sets the environment once per binary and fills the inventory through the same import
/// `interior import` uses.
fn unique_root() -> PathBuf {
    static ONCE: Once = Once::new();
    let wurzel = std::env::temp_dir().join(format!("interior-layouts-{}", std::process::id()));
    ONCE.call_once(|| {
        let _ = std::fs::remove_dir_all(&wurzel);
        std::fs::create_dir_all(wurzel.join("data/interior/flats")).expect("Testwurzel");
        let db = wurzel.join("test.db");
        std::env::set_var("SJEL_PERSONAL_ROOT", &wurzel);
        std::env::set_var("SJEL_DB_PATH", &db);
        std::env::remove_var("SJEL_INTERIOR_FLAT");
        let store = sjel_inventory::store::Store::open(&db).expect("the test database opens");
        sjel_inventory::import::inventory(&store, &fixture().join("data/interior/inventory"))
            .expect("the sample inventory imports");
    });
    wurzel
}

/// Its own copy of the sample flat per test: every test here writes files, and
/// `SJEL_PERSONAL_ROOT` is process-wide.
fn flat(name: &str) -> Model {
    let wurzel = unique_root();
    let target = wurzel.join("data/interior/flats").join(name);
    if !target.exists() {
        copy_layout(&fixture().join("data/interior/flats").join(MUSTER), &target);
    }
    Model::load(name).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn copy_layout(von: &Path, nach: &Path) {
    std::fs::create_dir_all(nach).expect("Zielverzeichnis");
    for e in std::fs::read_dir(von).expect("source dir") {
        let e = e.expect("entry");
        let target = nach.join(e.file_name());
        if e.file_type().expect("Typ").is_dir() {
            copy_layout(&e.path(), &target);
        } else {
            std::fs::copy(e.path(), &target).expect("copy file");
        }
    }
}

fn file(m: &Model, id: &str) -> String {
    std::fs::read_to_string(m.layouts_dir().join(format!("{id}.toml")))
        .unwrap_or_else(|e| panic!("{id}.toml: {e}"))
}

fn plan(name: &str, items: Vec<PlacedItem>) -> Layout {
    Layout {
        name: name.to_string(),
        items,
        id: String::new(),
    }
}

/// An empty plan is how hand planning starts, not a special case.
#[test]
fn a_new_plan_may_be_empty() {
    let m = flat("empty");
    layout_io::create(
        &m,
        "nothing-yet",
        &plan("Noch nichts", vec![]),
        "Ein Anfang.",
    )
    .expect("an empty plan is written");

    let l = m.load_layout("nothing-yet").expect("and read back");
    assert!(l.items.is_empty(), "no piece, because none was placed");
    assert_eq!(l.name, "Noch nichts");
    assert!(
        m.layout_names()
            .expect("Liste")
            .contains(&"nothing-yet".to_string()),
        "and it is in the list"
    );
}

/// The header says what the caller said and nothing more: a file claiming a clearance check that
/// never ran is worse than one without a header, because it is believed.
#[test]
fn the_header_claims_no_check_that_did_not_happen() {
    let m = flat("herkunft");
    let note = "Created 2026-08-31 through the API.\n\nSecond paragraph.";
    layout_io::create(&m, "by-hand", &plan("By hand", vec![]), note).expect("written");

    let text = file(&m, "by-hand");
    assert!(
        text.contains("# Created 2026-08-31 through the API."),
        "the note is a comment in the header:\n{text}"
    );
    assert!(
        text.contains("#\n# Second paragraph."),
        "and a paragraph stays a paragraph:\n{text}"
    );
    assert!(
        !text.contains("clearance check"),
        "nobody checked anything here, so the header claims nothing:\n{text}"
    );
}

/// A template is read, not touched.
#[test]
fn a_template_is_copied_and_stays_as_it_was() {
    let m = flat("kopie");
    let before = file(&m, TEMPLATE);
    let template = m.load_layout(TEMPLATE).expect("the template");

    layout_io::create(
        &m,
        "abgeschrieben",
        &plan("Abgeschrieben", template.items.clone()),
        "Kopie.",
    )
    .expect("written");

    let neu = m.load_layout("abgeschrieben").expect("the copy");
    assert_eq!(neu.items.len(), template.items.len());
    for (a, b) in neu.items.iter().zip(&template.items) {
        assert_eq!(
            (&a.reference, a.x, a.y, a.rot),
            (&b.reference, b.x, b.y, b.rot)
        );
    }
    assert_eq!(
        file(&m, TEMPLATE),
        before,
        "the template survived character for character"
    );
}

/// An existing plan is not overwritten, even by a better one: its header is the argument that
/// replaced it.
#[test]
fn an_existing_layout_is_not_overwritten() {
    let m = flat("no-overwrite");
    let before = file(&m, TEMPLATE);
    let e = layout_io::create(&m, TEMPLATE, &plan("Neu", vec![]), "Zweiter Versuch.")
        .expect_err("the name exists already");
    assert!(e.to_string().contains("exists already"), "{e}");
    assert_eq!(file(&m, TEMPLATE), before, "and the file is untouched");
}

/// The name becomes a file name, and over the API it comes from outside.
#[test]
fn a_layout_name_cannot_point_out_of_the_dir() {
    let m = flat("namen");
    let starts = m.layout_names().expect("Liste").len();
    for bad in ["../room", "unter/plan", "plan.toml", ""] {
        assert!(
            layout_io::create(&m, bad, &plan("X", vec![]), "n").is_err(),
            "`{bad}` must not become a file"
        );
    }
    assert_eq!(
        m.layout_names().expect("Liste").len(),
        starts,
        "and none of them wrote a file past the check"
    );
}

/// Archiving means off the list, not off the disk.
#[test]
fn an_archived_layout_leaves_the_list_not_the_disk() {
    let m = flat("archiv");
    layout_io::create(
        &m,
        "verworfen",
        &plan("Verworfen", vec![]),
        "Three paragraphs of reasoning.",
    )
    .expect("written");
    let before = file(&m, "verworfen");

    let whither = layout_io::archive_layout(&m, "verworfen").expect("archiviert");
    assert_eq!(
        whither, "archiv/verworfen.toml",
        "and the answer names no overlay path"
    );

    let names = m.layout_names().expect("Liste");
    assert!(
        !names.contains(&"verworfen".to_string()),
        "off the list: {names:?}"
    );
    assert!(
        !names.contains(&"archiv".to_string()),
        "and the directory itself is no layout: {names:?}"
    );
    let stored = m.layouts_dir().join(&whither);
    assert_eq!(
        std::fs::read_to_string(&stored).expect("the file is in the archive"),
        before,
        "with its reasoning, character for character"
    );
}

/// Two versions under one name would lose a reason.
#[test]
fn archiving_twice_under_one_name_is_refused() {
    let m = flat("archiv-zweimal");
    layout_io::create(&m, "doppelt", &plan("Erst", vec![]), "Erste Fassung.").expect("erste");
    layout_io::archive_layout(&m, "doppelt").expect("erste archiviert");
    layout_io::create(&m, "doppelt", &plan("Dann", vec![]), "Zweite Fassung.").expect("zweite");

    let e = layout_io::archive_layout(&m, "doppelt").expect_err("the archive is taken");
    assert!(e.to_string().contains("archive"), "{e}");
    assert!(
        file(&m, "doppelt").contains("Zweite Fassung"),
        "and the second is still there instead of silently moving over the first"
    );
}

#[test]
fn a_layout_that_does_not_exist_is_not_archived() {
    let m = flat("archive-empty");
    let e =
        layout_io::archive_layout(&m, "gab-es-nie").expect_err("there is nothing to clear away");
    assert!(e.to_string().contains("gab-es-nie"), "{e}");
}
