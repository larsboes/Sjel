//! The whole read path, against a real directory tree.
//!
//! The unit tests prove each stage on its own: the frontmatter parser on a
//! string, the scorer on constructed profiles, the containment rules on temp
//! dirs. What none of them touches is the seam where a declared source entry
//! becomes an adapter, that adapter's glob becomes files, and those files
//! become scored opportunities. That seam is exactly where path and glob bugs
//! live, because it is the only place a configured string meets a real
//! filesystem (retired-tracker#22).
//!
//! Every fixture here is built by the test and removed on drop. No vault path,
//! no real note and no personal profile enters this file.
//!
//! **Hermetic by construction.** `SJEL_PERSONAL_ROOT` is pointed at an empty
//! temp directory before anything runs, so `embed::embedding_role()` finds no
//! declared role and the scorer falls back to hash embedding. Without that the
//! suite would reach the machine's configured oMLX backend and stop being a
//! test of this crate.

use std::path::{Path, PathBuf};
use std::sync::Once;

use scouting::opportunity::OpportunityType;
use scouting::pipeline;
use scouting::score::load_telos_profiles;
use scouting::source::SearchQuery;
use scouting::sources::{create_adapter, SourceEntry, SourceManifest};

static ISOLATE: Once = Once::new();

/// Cut the process off from the operator's overlay exactly once. Every test in
/// this binary shares one process, and they all want the same value, so a
/// second setter would be writing what is already there.
fn isolate_from_the_operators_overlay() {
    ISOLATE.call_once(|| {
        let empty = std::env::temp_dir().join("axon-scouting-it-no-overlay");
        std::fs::create_dir_all(&empty).expect("empty overlay root");
        std::env::set_var("SJEL_PERSONAL_ROOT", &empty);
    });
}

/// A throwaway directory tree, removed on drop.
struct Tree(PathBuf);

impl Tree {
    fn new(name: &str) -> Self {
        isolate_from_the_operators_overlay();
        let dir = std::env::temp_dir().join(format!("axon-scouting-it-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp tree");
        Tree(dir)
    }

    fn file(&self, relative: &str, body: &str) -> PathBuf {
        let path = self.0.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("parent");
        }
        std::fs::write(&path, body).expect("write");
        path
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Build a manifest the way `Config::load` does — through serde, from the JSON
/// shape an operator actually writes. Constructing `SourceManifest` directly
/// would skip `SourceEntry`'s defaults and tilde expansion, which is half of
/// what this file exists to exercise.
fn manifest(json: serde_json::Value) -> SourceManifest {
    serde_json::from_value::<SourceEntry>(json)
        .expect("source entry deserializes")
        .resolve()
}

fn an_event_note(summary: &str, location: &str, category: &str) -> String {
    format!(
        "---\ntype: event\nsummary: {summary}\nstart: 2026-04-01\nend: 2026-04-01\n\
         location: {location}\ncategory: {category}\nsource_url: https://example.test/e\n---\n\n\
         ## About\n\nA gathering about systems programming and data pipelines.\n"
    )
}

fn a_profile(focus: &str) -> String {
    format!(
        "summary: {focus}\ncurrent_focus: systems programming, data pipelines\n\n\
         > [!quote] Charter\n> Local technical gatherings about systems programming.\n"
    )
}

/// An empty directory to hand `load_telos_profiles` as the legacy profile dir,
/// so a run only ever sees the profiles a source declared.
fn no_legacy_profiles(tree: &Tree) -> String {
    let dir = tree.path().join("no-legacy");
    std::fs::create_dir_all(&dir).expect("legacy dir");
    dir.to_string_lossy().into_owned()
}

// ---------------------------------------------------------------------------

/// The whole chain: a declared entry becomes an adapter, its glob finds the
/// notes, the notes become opportunities, and the profile a *different* root
/// declared is the one that scores them.
#[test]
fn a_declared_source_runs_end_to_end_and_scores_against_its_own_profile_root() {
    let vault = Tree::new("chain-vault");
    vault.file(
        "Applications/Systems Meetup.md",
        &an_event_note("Systems Meetup", "Bonn, DE", "meetup"),
    );
    vault.file(
        "Applications/Data Night.md",
        &an_event_note("Data Night", "Cologne, DE", "meetup"),
    );

    let profiles = Tree::new("chain-profiles");
    profiles.file("Events Profile.md", &a_profile("what is worth going to"));

    let source = manifest(serde_json::json!({
        "id": "chain-probe",
        "adapter": "obsidian-markdown",
        "path": vault.path().to_string_lossy(),
        "opportunities_glob": "Applications/*.md",
        "profile_path": profiles.path().to_string_lossy(),
        "profiles_glob": "Events Profile.md",
    }));

    let telos = load_telos_profiles(&no_legacy_profiles(&vault), std::slice::from_ref(&source));
    assert_eq!(
        telos.len(),
        1,
        "the profile in the separate root is the only one loaded"
    );
    assert_eq!(telos[0].focus_name, "Events Profile");

    let adapter = create_adapter(&source).expect("adapter builds from the manifest");
    let report = pipeline::run(
        adapter.as_ref(),
        &SearchQuery::default(),
        &telos,
        None,
        None, // no store: the chain under test is files to scores, not persistence
        None,
    )
    .expect("pipeline runs");

    assert_eq!(report.scored.len(), 2, "both notes came through the glob");
    assert_eq!(report.store_total, 0, "nothing was persisted");
    for scored in &report.scored {
        assert_eq!(
            scored.matched_focus.as_deref(),
            Some("Events Profile"),
            "every opportunity scored against the profile from the other root"
        );
    }

    // FINDING, pinned rather than fixed here. The adapter answers `name()`
    // with the declared source id -- that was fixed once already, because
    // `store::record_run` keys the cursor row on it and two declared feeds
    // shared one. `Opportunity.source` did not get the same treatment: it is
    // hardcoded to the adapter *type* in sources/obsidian_md.rs. So two
    // obsidian-markdown sources are one string in the stored column and one
    // chip in the dashboard, which is the same class of collision one layer
    // over. Not changed under an issue about test coverage: the column has
    // rows in it already, and rewriting what they mean is its own decision.
    assert_eq!(
        adapter.name(),
        "chain-probe",
        "the cursor is keyed per declared source"
    );
    assert_eq!(
        report.scored[0].opportunity.source, "obsidian_markdown",
        "but the opportunity still carries the adapter type, not the source id"
    );
}

/// The glob is the thing under test. A directory pattern must not reach a
/// sibling directory, which a `read_dir` on the wrong parent silently would.
#[test]
fn a_directory_glob_reaches_its_own_directory_and_no_sibling() {
    let vault = Tree::new("glob-scope");
    vault.file(
        "Applications/Wanted.md",
        &an_event_note("Wanted", "Bonn, DE", "meetup"),
    );
    vault.file(
        "Archive/Unwanted.md",
        &an_event_note("Unwanted", "Bonn, DE", "meetup"),
    );

    let source = manifest(serde_json::json!({
        "id": "scope-probe",
        "adapter": "obsidian-markdown",
        "path": vault.path().to_string_lossy(),
        "opportunities_glob": "Applications/*.md",
    }));

    let adapter = create_adapter(&source).expect("adapter builds");
    let report = pipeline::run(
        adapter.as_ref(),
        &SearchQuery::default(),
        &[],
        None,
        None,
        None,
    )
    .expect("pipeline runs");

    let titles: Vec<&str> = report
        .scored
        .iter()
        .map(|s| s.opportunity.title.as_str())
        .collect();
    assert_eq!(titles, vec!["Wanted"]);
}

/// An exact-file profile glob must load that file alone. The failure mode this
/// pins is a directory-wide sweep picking up a sibling profile, which changes
/// what every opportunity in the run is scored against.
#[test]
fn an_exact_profile_file_does_not_pull_in_its_siblings() {
    let vault = Tree::new("exact-vault");
    vault.file(
        "Applications/Meetup.md",
        &an_event_note("Meetup", "Bonn, DE", "meetup"),
    );

    let profiles = Tree::new("exact-profiles");
    profiles.file("Events Profile.md", &a_profile("events"));
    profiles.file("Scholarship Profile.md", &a_profile("funding"));

    let source = manifest(serde_json::json!({
        "id": "exact-probe",
        "adapter": "obsidian-markdown",
        "path": vault.path().to_string_lossy(),
        "opportunities_glob": "Applications/*.md",
        "profile_path": profiles.path().to_string_lossy(),
        "profiles_glob": "Events Profile.md",
    }));

    let telos = load_telos_profiles(&no_legacy_profiles(&vault), std::slice::from_ref(&source));
    let names: Vec<&str> = telos.iter().map(|p| p.focus_name.as_str()).collect();
    assert_eq!(names, vec!["Events Profile"]);
}

/// A source declaring a type must not surface a note of another type that
/// happens to share the directory. Unit-tested on the parser; proven here
/// through the adapter the config actually builds.
#[test]
fn a_typed_source_ignores_a_note_of_another_type_in_the_same_directory() {
    let vault = Tree::new("typed");
    vault.file(
        "Applications/An Event.md",
        &an_event_note("An Event", "Bonn, DE", "meetup"),
    );
    vault.file(
        "Applications/A Scholarship.md",
        "---\ntype: scholarship\nsummary: A Scholarship\nstatus: radar\n---\n",
    );
    vault.file("Applications/A Hub.md", "---\ntype: moc\n---\n# index\n");

    let source = manifest(serde_json::json!({
        "id": "typed-probe",
        "adapter": "obsidian-markdown",
        "path": vault.path().to_string_lossy(),
        "opportunities_glob": "Applications/*.md",
        "opportunity_type": "event",
    }));

    let adapter = create_adapter(&source).expect("adapter builds");
    assert_eq!(adapter.opportunity_type(), OpportunityType::Event);
    let report = pipeline::run(
        adapter.as_ref(),
        &SearchQuery::default(),
        &[],
        None,
        None,
        None,
    )
    .expect("pipeline runs");

    let titles: Vec<&str> = report
        .scored
        .iter()
        .map(|s| s.opportunity.title.as_str())
        .collect();
    assert_eq!(titles, vec!["An Event"]);
}

/// Vault cross-referencing takes a second real directory, and its only job is
/// to match one path against another. Nothing below the pipeline exercises
/// both directories at once.
#[test]
fn an_existing_note_is_cross_referenced_and_the_note_is_never_written_to() {
    let vault = Tree::new("linker-vault");
    vault.file(
        "Applications/Systems Meetup Bonn.md",
        &an_event_note("Systems Meetup Bonn", "Bonn, DE", "meetup"),
    );

    let notes = Tree::new("linker-notes");
    let existing = notes.file(
        "Systems Meetup Bonn.md",
        "---\ntype: event\n---\nan existing note\n",
    );
    let before = std::fs::read_to_string(&existing).expect("read");

    let source = manifest(serde_json::json!({
        "id": "linker-probe",
        "adapter": "obsidian-markdown",
        "path": vault.path().to_string_lossy(),
        "opportunities_glob": "Applications/*.md",
    }));

    let adapter = create_adapter(&source).expect("adapter builds");
    let report = pipeline::run(
        adapter.as_ref(),
        &SearchQuery::default(),
        &[],
        None,
        None,
        Some(notes.path()),
    )
    .expect("pipeline runs");

    assert_eq!(report.vault_links, 1, "the existing note was found");
    assert!(report.scored[0].rationale.contains("vault link:"));
    assert_eq!(
        std::fs::read_to_string(&existing).expect("read"),
        before,
        "cross-referencing is annotate-only and never touches the note"
    );
}

/// The silent-failure case. A glob that leaves the declared root must stop the
/// run, not return an empty list that reads as "no opportunities today".
#[test]
fn a_glob_that_escapes_the_root_fails_the_run_instead_of_finding_nothing() {
    let vault = Tree::new("escape");
    vault.file(
        "Applications/Inside.md",
        &an_event_note("Inside", "Bonn, DE", "meetup"),
    );

    let source = manifest(serde_json::json!({
        "id": "escape-probe",
        "adapter": "obsidian-markdown",
        "path": vault.path().to_string_lossy(),
        "opportunities_glob": "../../*.md",
    }));

    let adapter = create_adapter(&source).expect("the adapter builds; the glob fails at read time");
    let error = pipeline::run(
        adapter.as_ref(),
        &SearchQuery::default(),
        &[],
        None,
        None,
        None,
    )
    .expect_err("an escaping glob is an error, not an empty sweep");

    let message = error.to_string();
    assert!(message.contains("escape-probe"), "got: {message}");
    assert!(message.contains("escapes"), "got: {message}");
}

/// A root that has moved is the same class: naming it beats reporting nothing.
#[test]
fn a_source_root_that_is_gone_refuses_to_build_an_adapter() {
    isolate_from_the_operators_overlay();
    let missing = std::env::temp_dir().join("axon-scouting-it-gone-7a6b5c");
    let _ = std::fs::remove_dir_all(&missing);

    let source = manifest(serde_json::json!({
        "id": "gone-probe",
        "adapter": "obsidian-markdown",
        "path": missing.to_string_lossy(),
        "opportunities_glob": "*.md",
    }));

    // `Box<dyn SourceAdapter>` is not Debug, so `expect_err` cannot describe
    // the success case; match instead of unwrapping.
    let error = match create_adapter(&source) {
        Err(error) => error,
        Ok(_) => panic!("an adapter must not build on a root that is not there"),
    };
    assert!(error.to_string().contains("does not exist"), "got: {error}");
}
