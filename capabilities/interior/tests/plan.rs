//! What is on the sheet must be readable. Checked against the sample flat; the real one lives in
//! the overlay.

use interior::model::Model;
use interior::plan;
use std::sync::Once;

const FLAT: &str = "muster";

/// The same environment `tests/engine.rs` sets, for the same reason. Read only.
fn model() -> Model {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let fixture =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/overlay");
        let db = std::env::temp_dir().join(format!("interior-plan-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&db);
        std::env::set_var("SJEL_PERSONAL_ROOT", &fixture);
        std::env::set_var("SJEL_DB_PATH", &db);
        std::env::remove_var("SJEL_INTERIOR_FLAT");
        let store = sjel_inventory::store::Store::open(&db).expect("the test database opens");
        sjel_inventory::import::inventory(&store, &fixture.join("data/interior/inventory"))
            .expect("the sample inventory imports");
    });
    Model::load(FLAT).unwrap_or_else(|e| panic!("{FLAT}: {e}"))
}

fn draw(layout: &str) -> String {
    let m = model();
    let l = m
        .load_layout(layout)
        .unwrap_or_else(|e| panic!("{layout}: {e}"));
    plan::svg(&m, &l).unwrap_or_else(|e| panic!("{layout}: {e}"))
}

/// The viewBox as four numbers.
fn crop(svg: &str) -> [i32; 4] {
    let von = svg.find("viewBox=\"").expect("the SVG carries a viewBox") + 9;
    let bis = von + svg[von..].find('"').expect("the viewBox ends");
    let mut it = svg[von..bis].split_whitespace().map(|n| {
        n.parse::<i32>()
            .unwrap_or_else(|_| panic!("the viewBox carries numbers, not {n:?}"))
    });
    [
        it.next().expect("min_x"),
        it.next().expect("min_y"),
        it.next().expect("width"),
        it.next().expect("height"),
    ]
}

/// Every text between `>` and `</text>`.
fn texts(svg: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = svg;
    while let Some(i) = rest.find("<text") {
        rest = &rest[i..];
        let Some(onto) = rest.find('>') else { break };
        let Some(zu) = rest.find("</text>") else {
            break;
        };
        out.push(rest[onto + 1..zu].to_string());
        rest = &rest[zu + 7..];
    }
    out
}

/// The catalogue text belongs in the dashboard, not in the rect; checked on the layout whose three
/// pieces carry deliberately long labels.
#[test]
fn the_piece_shows_the_short_name() {
    let svg = draw("a-frei");
    let m = model();
    for ref_ in ["kleiderschrank", "schreibtisch", "bett"] {
        let long = &m.catalogue.get(ref_).expect("im Katalog").label;
        assert!(
            !svg.contains(long.as_str()),
            "the plan carries the catalogue text {long:?} of {ref_}"
        );
    }
    for short in ["Wardrobe", "Desk", "Bed"] {
        assert!(
            texts(&svg).iter().any(|t| t == short),
            "the plan does not carry {short:?}: {:?}",
            texts(&svg)
        );
    }
}

/// One piece, one label: two boxes once carried the same text on top of each other.
#[test]
fn every_piece_carries_exactly_one_text() {
    let svg = draw("a-frei");
    let m = model();
    let l = m.load_layout("a-frei").expect("the sample layout");
    for group in svg.split("<g data-ref=\"").skip(1) {
        let end = group.find("</g>").expect("jede Gruppe schliesst");
        assert_eq!(
            group[..end].matches("<text").count(),
            1,
            "a group does not carry exactly one text: {}",
            &group[..end.min(120)]
        );
    }
    assert_eq!(svg.matches("<g data-ref=\"").count(), l.items.len());
}

/// An estimate does not set the scale of what is measured: the view may show only the margin to
/// the left, not the terrace's estimated depth, which is still drawn.
#[test]
fn the_estimated_terrace_does_not_widen_the_sheet() {
    let m = model();
    let t = m
        .room
        .terrace
        .as_ref()
        .expect("the fixture has a terrace");
    assert!(t.estimated, "this check is about the estimated area");
    let svg = draw("a-frei");
    let [min_x, _, width, _] = crop(&svg);

    let left = m
        .room
        .main_room
        .polygon
        .iter()
        .map(|p| p[0])
        .min()
        .expect("the polygon has corners");
    assert!(
        min_x > t.x[0],
        "the view starts at the estimated terrace depth ({min_x})"
    );
    assert!(
        min_x < left,
        "nothing of the terrace band is visible ({min_x} instead of left of {left})"
    );

    let gemessen = m.room.bathroom.as_ref().map_or(left, |b| b.x[1]).max(
        m.room
            .main_room
            .polygon
            .iter()
            .map(|p| p[0])
            .max()
            .unwrap_or(left),
    );
    assert!(
        (gemessen - left) * 2 > width,
        "the measured part takes less than half the sheet width ({} of {width})",
        gemessen - left
    );
    assert!(
        texts(&svg).iter().any(|t| t.starts_with("TERRACE")),
        "the terrace is no longer labelled"
    );
}
