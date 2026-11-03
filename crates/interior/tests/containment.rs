//! The repository must not know any flat.
//!
//! This capability lives in a public repository, so its generic part must be generic, and this test
//! is the condition. Its first run found three real leaks: a default search band with this flat's
//! sizes, the flat's name in a page title, and in a constant. It reads `tests/` too: the TypeScript
//! baseline once lay there with real corridor widths.

use std::fs;
use std::path::{Path, PathBuf};

/// Sizes of the real flat that have no place in this repository; the list is the claim.
const FORBIDDEN_SIZES: &[&str] = &["420", "460", "590", "263", "196", "329"];
const FORBIDDEN_NAMES: &[&str] = &["Beuel", "beuel-sued", "Petersbergweg"];

/// Text only. A directory a flat could hide in is searched, not excluded: `tests/fixtures/` is
/// invented geometry and must prove it.
const ENDINGS: &[&str] = &["rs", "toml", "json", "md"];

/// `under` is a subdirectory, or "." for the capability root; the root counts, since the README
/// once named the real flat's directory.
fn files(below: &str) -> Vec<(String, String)> {
    let wurzel = Path::new(env!("CARGO_MANIFEST_DIR")).join(below);
    let mut offen = vec![wurzel.clone()];
    let mut out = Vec::new();
    while let Some(dir) = offen.pop() {
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        for e in rd.filter_map(|e| e.ok()) {
            let p: PathBuf = e.path();
            if p.is_dir() {
                // Build output, and the two subtrees that have their own runs.
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

/// Rust comments may name the numbers, to explain why; data files cannot make that distinction.
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

/// As a standalone number, not a substring: 460 in 1460 is no wall length.
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
        // A file whose job is to recognise the markers must contain them; this is that file, as
        // `tools/check-publication-hygiene.sh` is for itself.
        if name.ends_with("containment.rs") {
            continue;
        }
        // The flat's name is a leak even in a comment, in a public repository.
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
        "flat data in the public repository; it belongs in the overlay under \
         data/interior/, not here:\n  {}",
        found.join("\n  ")
    );
}

/// The test must actually see the files: a walker that finds nothing passes every claim.
#[test]
fn the_test_reads_all_three_trees() {
    let src = files("src");
    let tests = files("tests");
    let wurzel = files(".");
    assert!(src.len() >= 8, "src/ hat {} Dateien", src.len());
    assert!(
        tests.iter().any(|(n, _)| n.contains("fixtures")),
        "the sample flat is not read along: {:?}",
        tests.iter().map(|(n, _)| n).collect::<Vec<_>>()
    );
    assert!(
        wurzel.iter().any(|(n, _)| n == "README.md"),
        "the README is not read along: {:?}",
        wurzel.iter().map(|(n, _)| n).collect::<Vec<_>>()
    );
}

/// The media path comes from the client and must not lead out of `media/`. Checks the resolution
/// itself: `canonicalize` both sides, then `starts_with`. A check for `..` alone fails silently
/// for a symlink, which the last case covers.
#[test]
fn a_media_path_does_not_lead_out_of_the_dir() {
    let tmp = std::env::temp_dir().join(format!("interior-media-{}", std::process::id()));
    let media = tmp.join("media");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(media.join("produkt")).expect("Testbaum");
    std::fs::write(media.join("produkt/bild.png"), b"x").expect("Bild");
    std::fs::write(tmp.join("geheim.txt"), b"x").expect("file beside it");

    // The same resolution `api_media` uses.
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

    assert!(allowed("produkt/bild.png"), "the image itself is served");
    assert!(!allowed("../geheim.txt"), "one level up: no");
    assert!(!allowed("produkt/../../geheim.txt"), "through a detour: no");
    assert!(!allowed("produkt"), "a directory is no file");

    // The case a `..` check would let through: a symlink without a single dot.
    #[cfg(unix)]
    {
        let link = media.join("raus.txt");
        std::os::unix::fs::symlink(tmp.join("geheim.txt"), &link).expect("Symlink");
        assert!(
            !allowed("raus.txt"),
            "a symlink out of the directory is not served"
        );
    }
    let _ = std::fs::remove_dir_all(&tmp);
}
