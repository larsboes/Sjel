//! Das Repository darf keine Wohnung kennen.
//!
//! Q59 (PRD §8.3) hat diese Capability in das oeffentliche Axon-Repository verschoben. Damit
//! das sicher ist, muss der generische Teil generisch SEIN und nicht nur so heissen. Dieser
//! Test ist die Bedingung dafuer.
//!
//! Er hat beim ersten Lauf am 2026-08-30 drei echte Lecks gefunden — `search.rs` hielt
//! [0, 420, 0, 590] als Standard-Suchband, also die Masse dieser Wohnung, `plan.rs` trug ihren
//! Namen im Seitentitel, und `main.rs` nannte ihn in einer Konstante und einem Doc-Kommentar.
//!
//! Er prueft seit dem Umzug `tests/` MIT, und das war kein Zierrat: die aufgezeichnete
//! Vorlage der TypeScript-Fassung lag als `tests/ts-baseline.json` genau ein Verzeichnis neben
//! dem Code, den dieser Test las, und enthielt Korridorbreiten und Moebelmasse der echten
//! Wohnung. Ein Containment-Test, der nur `src/` liest, haette sie mit veroeffentlicht. Sie
//! liegt jetzt im Overlay neben der Wohnung, die sie beschreibt.

use std::fs;
use std::path::{Path, PathBuf};

/// Masse dieser Wohnung, die in diesem Repository nichts zu suchen haben. Bewusst als Literale
/// gefuehrt: die Liste IST die Behauptung, und sie muss mitwachsen, wenn das Modell Zahlen
/// dazugewinnt.
const FORBIDDEN_SIZES: &[&str] = &["420", "460", "590", "263", "196", "329"];
const FORBIDDEN_NAMES: &[&str] = &["Beuel", "beuel-sued", "Petersbergweg"];

/// Nur Text. Ein Verzeichnis, in dem eine Wohnung stehen koennte, wird durchsucht, nicht
/// ausgenommen — `tests/fixtures/` ist erfundene Geometrie und muss das beweisen koennen.
const ENDINGS: &[&str] = &["rs", "toml", "json", "md"];

/// `unter` ist ein Unterverzeichnis oder "." fuer das Wurzelverzeichnis der Capability.
/// Das Wurzelverzeichnis gehoert dazu: die README nannte bis zum Umzug das Verzeichnis der
/// echten Wohnung beim Namen, und ein Test, der nur Quelltext liest, haette das durchgelassen.
fn files(below: &str) -> Vec<(String, String)> {
    let wurzel = Path::new(env!("CARGO_MANIFEST_DIR")).join(below);
    let mut offen = vec![wurzel.clone()];
    let mut out = Vec::new();
    while let Some(dir) = offen.pop() {
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        for e in rd.filter_map(|e| e.ok()) {
            let p: PathBuf = e.path();
            if p.is_dir() {
                // Bauausgabe und die beiden Unterbaeume, die eigene Laeufe haben.
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if !matches!(name, "target" | "src" | "tests") {
                    offen.push(p);
                }
                continue;
            }
            let fits = p
                .extension()
                .and_then(|x| x.to_str())
                .is_some_and(|x| ENDINGS.contains(&x));
            if !fits {
                continue;
            }
            let Ok(text) = fs::read_to_string(&p) else {
                continue;
            };
            let name = p
                .strip_prefix(Path::new(env!("CARGO_MANIFEST_DIR")))
                .unwrap_or(&p)
                .to_string_lossy()
                .into_owned();
            out.push((name, text));
        }
    }
    out.sort();
    out
}

/// In Rust duerfen Kommentare die Zahlen nennen — dort erklaeren sie, warum etwas so ist, und
/// dieser Test selbst ist das Beispiel. In Daten gibt es diese Trennung nicht: eine TOML- oder
/// JSON-Datei, die eine Wandlaenge nennt, nennt sie, ob mit `#` davor oder ohne.
fn checkable(name: &str, src: &str) -> String {
    if !name.ends_with(".rs") {
        return src.to_string();
    }
    src.lines()
        .map(|l| match l.find("//") {
            Some(i) => &l[..i],
            None => l,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Als eigenstaendige Zahl, nicht als Teilfolge: 460 in 1460 ist keine Wandlaenge.
fn names_number(line: &str, mass: &str) -> bool {
    line.match_indices(mass).any(|(i, _)| {
        let before = i == 0 || !line.as_bytes()[i - 1].is_ascii_digit();
        let j = i + mass.len();
        let after = j >= line.len() || !line.as_bytes()[j].is_ascii_digit();
        before && after
    })
}

#[test]
fn no_flat_data_in_the_repository() {
    let mut found = Vec::new();
    let alle = files("src")
        .into_iter()
        .chain(files("tests"))
        .chain(files("."));
    for (name, src) in alle {
        // Eine Datei, deren Aufgabe das Erkennen von Markern ist, muss sie enthalten. Genau
        // diese Ausnahme, mit genau dieser Begruendung, fuehrt `tools/check-publication-
        // hygiene.sh` fuer sich selbst und seinen Test — hier ist es dieselbe Datei und
        // dieselbe einzige. Jede andere hat keinen Grund, eine dieser Zahlen zu tragen.
        if name.ends_with("containment.rs") {
            continue;
        }
        // Der Name der Wohnung ist auch in einem Kommentar ein Leck, sobald das Repo
        // oeffentlich ist — und dieses ist es.
        for forbidden in FORBIDDEN_NAMES {
            if src.contains(forbidden) {
                found.push(format!("{name}: nennt \"{forbidden}\""));
            }
        }
        let text = checkable(&name, &src);
        for (n, line) in text.lines().enumerate() {
            for mass in FORBIDDEN_SIZES {
                if names_number(line, mass) {
                    found.push(format!(
                        "{name}:{}: Literal {mass} — {}",
                        n + 1,
                        line.trim()
                    ));
                }
            }
        }
    }
    assert!(
        found.is_empty(),
        "Wohnungsdaten im oeffentlichen Repository. Sie gehoeren in das Overlay, unter \
         data/interior/, nicht hierher:\n  {}",
        found.join("\n  ")
    );
}

/// Der Test muss die Dateien auch wirklich sehen. Ein Verzeichniswanderer, der nichts findet,
/// besteht jede Behauptung.
#[test]
fn the_test_reads_all_three_trees() {
    let src = files("src");
    let tests = files("tests");
    let wurzel = files(".");
    assert!(src.len() >= 8, "src/ hat {} Dateien", src.len());
    assert!(
        tests.iter().any(|(n, _)| n.contains("fixtures")),
        "die Musterwohnung wird nicht mitgelesen: {:?}",
        tests.iter().map(|(n, _)| n).collect::<Vec<_>>()
    );
    assert!(
        wurzel.iter().any(|(n, _)| n == "README.md"),
        "die README wird nicht mitgelesen: {:?}",
        wurzel.iter().map(|(n, _)| n).collect::<Vec<_>>()
    );
}

/// Der Medienpfad kommt vom Client, also darf er nicht aus `media/` herausfuehren.
///
/// Geprueft wird die Aufloesung selbst und nicht der HTTP-Weg: `canonicalize` beidseitig, dann
/// `starts_with`. Eine Pruefung auf die Zeichenfolge `..` waere die naheliegende Fassung und die
/// falsche — sie versagt still, sobald ein Symlink aus dem Verzeichnis zeigt, und genau das
/// prueft der letzte Fall hier.
#[test]
fn a_media_path_does_not_lead_out_of_the_dir() {
    let tmp = std::env::temp_dir().join(format!("interior-media-{}", std::process::id()));
    let media = tmp.join("media");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(media.join("produkt")).expect("Testbaum");
    std::fs::write(media.join("produkt/bild.png"), b"x").expect("Bild");
    std::fs::write(tmp.join("geheim.txt"), b"x").expect("Datei daneben");

    // Dieselbe Aufloesung, die `api_media` fuehrt.
    let allowed = |pfad: &str| -> bool {
        let wurzel = match media.canonicalize() {
            Ok(w) => w,
            Err(_) => return false,
        };
        match wurzel.join(pfad).canonicalize() {
            Ok(target) => target.starts_with(&wurzel) && target.is_file(),
            Err(_) => false,
        }
    };

    assert!(
        allowed("produkt/bild.png"),
        "das Bild selbst wird geliefert"
    );
    assert!(!allowed("../geheim.txt"), "eine Ebene hoeher: nein");
    assert!(
        !allowed("produkt/../../geheim.txt"),
        "ueber einen Umweg: nein"
    );
    assert!(!allowed("produkt"), "ein Verzeichnis ist keine Datei");

    // Der Fall, den eine `..`-Pruefung durchliesse: ein Symlink ohne einen einzigen Punkt.
    #[cfg(unix)]
    {
        let link = media.join("raus.txt");
        std::os::unix::fs::symlink(tmp.join("geheim.txt"), &link).expect("Symlink");
        assert!(
            !allowed("raus.txt"),
            "ein Symlink aus dem Verzeichnis heraus wird nicht geliefert"
        );
    }
    let _ = std::fs::remove_dir_all(&tmp);
}
