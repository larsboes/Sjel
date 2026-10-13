//! The flat declares its rules, and the checker looks them up.
//!
//! Every test here works on a copy of the sample flat in a temp dir, because it has to change the
//! rules to show they take effect; editing a file other test binaries read would race them.

use interior::clearance::{check_layout, RULE_IDS};
use interior::model::Model;
use std::path::{Path, PathBuf};
use std::sync::Once;

const FLAT: &str = "muster";

/// A copy of the fixture with one line replaced. Each variant gets its own dir and flat name, so
/// variants coexist: `SJEL_PERSONAL_ROOT` is process-wide.
fn variant(name: &str, replacements: &[(&str, &str)]) -> Model {
    let wurzel = unique_root();
    let target = wurzel.join("data/interior/flats").join(name);
    if !target.exists() {
        let source = fixture().join("data/interior/flats").join(FLAT);
        copy_layout(&source, &target);
        // Both files, because the rules live in `rules.toml` and what they measure partly in
        // `room.toml` (`eingang = true`). A replacement that matches nowhere is an error.
        for (old, new) in replacements {
            let mut hit = false;
            for file in ["rules.toml", "room.toml"] {
                let path = target.join(file);
                let text = std::fs::read_to_string(&path).expect("file of the copy");
                if text.contains(old) {
                    std::fs::write(&path, text.replace(old, new)).expect("write the copy");
                    hit = true;
                }
            }
            assert!(
                hit,
                "{name}: `{old}` is neither in rules.toml nor in room.toml; the test checks a line that does not exist"
            );
        }
    }
    Model::load(name).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/overlay")
}

/// Sets the environment once per binary and fills the inventory through the same import
/// `interior import` uses.
fn unique_root() -> PathBuf {
    static ONCE: Once = Once::new();
    let wurzel = std::env::temp_dir().join(format!("interior-rules-{}", std::process::id()));
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

fn copy_layout(old: &Path, new: &Path) {
    std::fs::create_dir_all(new).expect("Zielverzeichnis");
    for e in std::fs::read_dir(old).expect("source dir") {
        let e = e.expect("entry");
        let target = new.join(e.file_name());
        if e.file_type().expect("Typ").is_dir() {
            copy_layout(&e.path(), &target);
        } else {
            std::fs::copy(e.path(), &target).expect("copy file");
        }
    }
}

/// Same geometry, same violation, one word different in the file. `b-lichtkorridor` puts a piece
/// too tall into the light corridor: with R3 soft the layout passes with a warning, with R3 hard
/// the same layout fails.
#[test]
fn a_rule_severity_comes_from_the_file() {
    let weich = variant("r3-weich", &[]);
    let hart = variant(
        "r3-hart",
        &[(
            "id = \"R3\"\nschwere = \"weich\"",
            "id = \"R3\"\nschwere = \"hart\"",
        )],
    );

    let l = weich.load_layout("b-lichtkorridor").expect("Layout");
    let w = check_layout(&weich, &l).expect("check soft");
    assert!(w.pass, "R3 soft: the layout passes with a warning");
    assert_eq!(w.soft.iter().filter(|v| v.rule == "R3").count(), 1);
    assert_eq!(w.hard.iter().filter(|v| v.rule == "R3").count(), 0);

    let l = hart.load_layout("b-lichtkorridor").expect("Layout");
    let h = check_layout(&hart, &l).expect("check hard");
    assert!(
        !h.pass,
        "R3 hard: the same layout fails, or the severity still lives in the code"
    );
    assert_eq!(h.hard.iter().filter(|v| v.rule == "R3").count(), 1);
    assert_eq!(h.soft.iter().filter(|v| v.rule == "R3").count(), 0);
}

/// The rule text reaches the report: `message` says what this layout gets wrong, `text` what makes
/// it a rule.
#[test]
fn a_violation_carries_the_flat_rule_text() {
    let m = variant("text", &[]);
    let l = m.load_layout("c-terrassentuer").expect("Layout");
    let r = check_layout(&m, &l).expect("check");

    let v = r
        .hard
        .iter()
        .find(|v| v.rule == "R1")
        .expect("c-terrassentuer breaks R1");
    assert_eq!(
        v.text.as_deref(),
        Some("Anlaufzone vor der gesamten Breite der Terrassentuer freihalten."),
        "the text comes verbatim from rules.toml"
    );
}

/// An invariant carries no rule text: that is the line between the two kinds of ids.
#[test]
fn an_invariant_has_no_rule_text_and_needs_no_declaration() {
    let m = variant("invariante", &[]);
    let l = m.load_layout("d-ausserhalb").expect("Layout");
    let r = check_layout(&m, &l).expect("check");

    let v = r
        .hard
        .iter()
        .find(|v| v.rule == "room_boundary")
        .expect("d-ausserhalb puts a piece through the wall");
    assert!(
        v.text.is_none(),
        "`room_boundary` is geometry, not a house rule; there is no text to look up"
    );
    assert!(
        !RULE_IDS.contains(&"room_boundary"),
        "and so it is not in RULE_IDS either"
    );
}

/// An id the flat does not declare is an error, not a violation without text; the message lists
/// what is declared.
#[test]
fn an_undeclared_id_aborts_the_check() {
    let m = variant(
        "without-r1",
        &[(
            "[[regeln]]\nid = \"R1\"\nschwere = \"hart\"\ntext = \"Anlaufzone vor der gesamten Breite der Terrassentuer freihalten.\"\n",
            "",
        )],
    );
    let l = m.load_layout("c-terrassentuer").expect("Layout");
    let e = check_layout(&m, &l).expect_err("R1 is missing, so there is nothing to look up");
    let text = e.to_string();
    assert!(
        text.contains("R1"),
        "the message names the missing id: {text}"
    );
    assert!(
        text.contains("R2"),
        "and the existing ones, so a typo shows at once: {text}"
    );
}

/// An unknown severity is not silently `soft`.
#[test]
fn an_unknown_severity_is_an_error() {
    let m = variant(
        "schwere-tippfehler",
        &[(
            "id = \"R1\"\nschwere = \"hart\"",
            "id = \"R1\"\nschwere = \"haart\"",
        )],
    );
    let l = m.load_layout("c-terrassentuer").expect("Layout");
    let e = check_layout(&m, &l).expect_err("`haart` is no severity");
    assert!(e.to_string().contains("haart"), "{e}");
}

/// What the flat declares and this checker does not check is in the result: "passes" means passes
/// the rules that were measured.
#[test]
fn declared_rules_without_a_check_are_reported() {
    let m = variant("unchecked", &[]);
    let l = m.load_layout("a-frei").expect("Layout");
    let r = check_layout(&m, &l).expect("check");

    assert!(r.pass, "a-frei besteht");
    let unchecked_r3: Vec<&str> = r.unchecked.iter().map(|u| u.rule.as_str()).collect();
    assert_eq!(
        unchecked_r3,
        vec!["R10"],
        "a passing layout still says what was not measured"
    );
    let u = &r.unchecked[0];
    assert!(
        u.text.contains("Teppich"),
        "with the rule text, so the report says what stayed unchecked: {}",
        u.text
    );
    assert!(
        u.reason.contains("does not check it"),
        "and with the reason: {}",
        u.reason
    );
}

/// Every id this checker can emit is declared by the sample flat too. R2 and R4 fire nothing in
/// the fixture, so only this check would catch a missing declaration before the real flat does.
#[test]
fn the_sample_flat_declares_every_rule_the_checker_knows() {
    let m = variant("complete", &[]);
    for id in RULE_IDS {
        m.rules
            .rule(id)
            .unwrap_or_else(|e| panic!("RULE_IDS names `{id}`, the sample flat does not: {e}"));
    }
}

/// R5 measures the axis light comes along, not the gap between two centres. `a-frei` and
/// `h-blendung` place the same piece in the same room and differ only in `rot`.
#[test]
fn r5_tells_side_light_from_frontal() {
    let m = variant("r5", &[]);

    let frei = m.load_layout("a-frei").expect("Layout");
    let r = check_layout(&m, &frei).expect("check");
    assert!(
        !r.soft.iter().any(|v| v.rule == "R5"),
        "unrotated, the light from the west wall falls from the side"
    );

    let rotated = m.load_layout("h-blendung").expect("Layout");
    let r = check_layout(&m, &rotated).expect("check");
    assert_eq!(
        r.soft.iter().filter(|v| v.rule == "R5").count(),
        1,
        "rotated by 90 degrees, front and back face the glazing"
    );
    assert!(r.hard.is_empty(), "and otherwise this layout breaks nothing");
}

/// R6 looks from the entrance into the room and reports what is seen first.
#[test]
fn r6_reports_the_bed_in_the_entrance_sight_line() {
    let m = variant("r6", &[]);

    let l = m.load_layout("i-blick-aufs-bett").expect("Layout");
    let r = check_layout(&m, &l).expect("check");
    assert_eq!(r.soft.iter().filter(|v| v.rule == "R6").count(), 1);
    assert!(r.hard.is_empty(), "and nothing else");

    let frei = m.load_layout("a-frei").expect("Layout");
    let r = check_layout(&m, &frei).expect("check");
    assert!(
        !r.soft.iter().any(|v| v.rule == "R6"),
        "with the bed out of the line, the rule stays quiet"
    );
}

/// Without a declared entrance R6 reports itself unchecked instead of failing silently.
#[test]
fn without_a_declared_entrance_r6_reports_unchecked() {
    let m = variant("without-entrance", &[("eingang = true\n", "")]);
    let l = m.load_layout("i-blick-aufs-bett").expect("Layout");
    let r = check_layout(&m, &l).expect("check");

    assert!(
        !r.soft.iter().any(|v| v.rule == "R6"),
        "without an entrance there is nothing to measure"
    );
    let unchecked_r3 = r
        .unchecked
        .iter()
        .find(|u| u.rule == "R6")
        .expect("R6 reports itself unchecked");
    assert!(
        unchecked_r3.reason.contains("eingang"),
        "and names what is missing: {}",
        unchecked_r3.reason
    );
}

/// A rule that cannot apply for a missing measurement says so too.
#[test]
fn a_missing_measurement_makes_the_rule_unchecked_not_silent() {
    let m = variant("without-height", &[]);
    // `sideboard` is the sample piece with estimated sizes; here it gets no height at all and stands
    // in the light corridor.
    let mut l = m.load_layout("a-frei").expect("Layout");
    l.items.push(interior::model::PlacedItem {
        reference: "sideboard".into(),
        x: 20,
        y: 100,
        rot: 0,
        size: None,
        kind: None,
    });
    let r = check_layout(&m, &l).expect("check");
    let unchecked_r3 = r.unchecked.iter().find(|u| u.rule == "R3");
    if let Some(u) = unchecked_r3 {
        assert!(
            u.reason.contains("height"),
            "the reason names the missing measurement: {}",
            u.reason
        );
    }
}

/// Rotating turns what a piece demands: `opens` is in the piece's orientation and
/// `Side::rotated` carries it. Same corner, four rotations: south and east open into the room, west
/// and north against the walls at x = 0 and y = 0.
#[test]
fn a_rotation_rotates_the_opening_side() {
    use interior::model::{Layout, PlacedItem};
    let m = variant("drehung", &[]);

    let verdict = |rot: i32| -> (bool, Vec<String>) {
        let l = Layout {
            name: "Drehprobe".into(),
            id: String::new(),
            items: vec![PlacedItem {
                reference: "kleiderschrank".into(),
                x: 0,
                y: 0,
                rot,
                size: None,
                kind: None,
            }],
        };
        let r = check_layout(&m, &l).expect("check");
        (r.pass, r.hard.iter().map(|v| v.rule.clone()).collect())
    };

    assert!(verdict(0).0, "doors to the south: opens into the room");
    assert!(verdict(270).0, "doors to the east: opens into the room");

    let (pass, rule_ids) = verdict(90);
    assert!(!pass, "doors to the west: against the wall at x = 0");
    assert!(rule_ids.contains(&"open".to_string()), "{rule_ids:?}");

    let (pass, rule_ids) = verdict(180);
    assert!(!pass, "doors to the north: against the wall at y = 0");
    assert!(rule_ids.contains(&"open".to_string()), "{rule_ids:?}");
}
