//! Bring the index and the disk back into agreement, in one direction: a recorded location whose
//! path is gone.
//!
//! `index` deliberately keeps a row it did not find on disk — an undeclared disappearance has to
//! stay visible, and silently dropping it would let a wiped volume read as a tidy index. The cost
//! of that default is that nothing can resolve one either. A rename is a path change like any
//! other, so a move made by hand, or by a verb that did not reconcile its own rows, leaves a
//! permanent false absence. Measured 2026-10-01: one in-library `relabel` of 13 files produced 13
//! false absences *and* 13 false duplicate groups from the same stale rows.
//!
//! This verb is the only way to resolve one, and it is deliberately narrow: a gone location is
//! dropped **only** when its digest is also recorded at another path that is present on this
//! volume. If the bytes are nowhere, the row stays and the file is reported as missing. So
//! reconciliation can only ever remove a location whose bytes another indexed location still
//! holds, and it is therefore safe against the failure it most needs to survive — a library root
//! that exists while the volume behind it does not.
//!
//! That guard has one precondition, and the verb enforces it rather than documenting it: the index
//! must be current. A moved file's *new* path is not recorded until `index` runs, so reconciling
//! first would find no witness for it and report a rename as a disappearance — the alarming
//! answer, and the wrong one. An unindexed path therefore refuses the whole run.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::store::{Ledger, Result};

/// A recorded path that is gone, and the paths that still hold its bytes.
#[derive(Debug, Serialize)]
pub struct Relocation {
    pub relpath: String,
    pub digest: String,
    pub size: i64,
    /// Every indexed path on this volume that holds these bytes, sorted.
    pub survives_at: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ReconcileReport {
    pub applied: bool,
    /// False when the run refused: either the index is behind the disk, or it was stopped.
    pub complete: bool,
    pub root: PathBuf,
    pub uuid: String,
    pub disk_files: usize,
    pub indexed_locations: usize,
    /// Recorded paths that are gone and whose bytes survive elsewhere on this volume. Drops these.
    pub relocated: Vec<Relocation>,
    /// Recorded paths that are gone and whose bytes are nowhere in the index. Never dropped.
    pub missing: Vec<String>,
    /// Files on disk with no recorded location, in the other direction of the same disagreement.
    /// `index` records them; they are listed so one run shows both.
    pub not_indexed: Vec<String>,
    pub dropped: usize,
    pub issues: Vec<String>,
    pub limitations: Vec<&'static str>,
}

const LIMITATIONS: [&str; 5] = [
    "A location is dropped only when its digest is recorded at another path present on this volume. Bytes that are nowhere leave the row in place.",
    "The index must be current. An unindexed path refuses the run, because a move whose new path is not yet recorded has no witness and would read as a disappearance.",
    "A rename and the deletion of one of two identical copies are indistinguishable afterwards: both leave one path gone and the same bytes elsewhere. The digest decides; this verb cannot say which happened.",
    "Every digest compared here comes from the index. Run `media audit` to re-read bytes from the disk before treating a survivor as proven.",
    "Nothing on disk is moved, copied, renamed or deleted. This verb changes the ledger only.",
];

pub fn reconcile(ledger: &Ledger, root: &Path, uuid: &str, apply: bool) -> Result<ReconcileReport> {
    let root = root.canonicalize()?;
    let paths = crate::files(&root)?;
    let locations = ledger.locations(uuid)?;

    let on_disk: BTreeSet<&str> = paths.iter().map(|(rel, _)| rel.as_str()).collect();
    let recorded: BTreeSet<&str> = locations.iter().map(|row| row.relpath.as_str()).collect();

    // Only a location that is present can prove its bytes survive. Two absent rows sharing a digest
    // prove nothing, and dropping the first would destroy the last path that names those bytes.
    let mut survivors: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for row in &locations {
        if on_disk.contains(row.relpath.as_str()) {
            survivors
                .entry(row.digest.as_str())
                .or_default()
                .push(row.relpath.as_str());
        }
    }

    let mut report = ReconcileReport {
        applied: apply,
        complete: true,
        root,
        uuid: uuid.to_owned(),
        disk_files: paths.len(),
        indexed_locations: locations.len(),
        relocated: Vec::new(),
        missing: Vec::new(),
        not_indexed: Vec::new(),
        dropped: 0,
        issues: Vec::new(),
        limitations: LIMITATIONS.to_vec(),
    };

    for (rel, _) in &paths {
        if !recorded.contains(rel.as_str()) {
            report.not_indexed.push(rel.clone());
        }
    }
    // Refuse before computing anything that would be misread. A path the index has not seen may be
    // the new home of a gone one, and this verb cannot tell the two apart until `index` has run.
    if !report.not_indexed.is_empty() {
        report.complete = false;
        report.issues.push(format!(
            "{} path(s) are on disk and not in the index; run `media index` first, or a moved file would be reported as missing",
            report.not_indexed.len()
        ));
        return Ok(report);
    }

    for row in &locations {
        if on_disk.contains(row.relpath.as_str()) {
            continue;
        }
        match survivors.get(row.digest.as_str()) {
            Some(alive) => report.relocated.push(Relocation {
                relpath: row.relpath.clone(),
                digest: row.digest.clone(),
                size: row.size,
                survives_at: alive.iter().map(|path| (*path).to_owned()).collect(),
            }),
            None => report.missing.push(row.relpath.clone()),
        }
    }

    if !apply {
        report.issues.push(
            "dry run: nothing was dropped, and the paths that are not indexed are left to `media index`"
                .into(),
        );
        return Ok(report);
    }

    for row in &report.relocated {
        if ledger.forget(uuid, &row.relpath)? {
            report.dropped += 1;
        } else {
            report
                .issues
                .push(format!("{}: already absent from the ledger", row.relpath));
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Location;

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let root = std::env::temp_dir().canonicalize().unwrap().join(format!(
                "media-reconcile-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(root.join("tree")).unwrap();
            Self(root)
        }
        fn tree(&self) -> PathBuf {
            self.0.join("tree")
        }
        fn ledger(&self) -> Ledger {
            Ledger::open(&self.0.join("scratch.db")).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    /// A rename reconciles; a deletion does not. This is the distinction the whole verb turns on.
    #[test]
    fn a_renamed_file_is_dropped_and_a_deleted_one_is_reported() {
        let fixture = Fixture::new();
        let ledger = fixture.ledger();
        std::fs::write(fixture.tree().join("renamed.jpg"), b"rename me").unwrap();
        std::fs::write(fixture.tree().join("deleted.jpg"), b"delete me").unwrap();
        assert_eq!(crate::index(&ledger, &fixture.tree(), "V", "v").unwrap().hashed, 2);

        std::fs::create_dir_all(fixture.tree().join("moved")).unwrap();
        std::fs::rename(
            fixture.tree().join("renamed.jpg"),
            fixture.tree().join("moved/renamed.jpg"),
        )
        .unwrap();
        std::fs::remove_file(fixture.tree().join("deleted.jpg")).unwrap();

        let dry = reconcile(&ledger, &fixture.tree(), "V", false).unwrap();
        assert!(!dry.complete, "the index is behind the disk");
        assert_eq!(dry.not_indexed, vec!["moved/renamed.jpg"]);
        assert!(dry.missing.is_empty(), "a move is not a disappearance");
        assert_eq!(dry.dropped, 0);
        assert_eq!(ledger.counts().unwrap(), (2, 2));

        assert_eq!(crate::index(&ledger, &fixture.tree(), "V", "v").unwrap().hashed, 1);

        let dry = reconcile(&ledger, &fixture.tree(), "V", false).unwrap();
        assert!(dry.complete);
        assert_eq!(dry.dropped, 0, "a dry run drops nothing");
        assert_eq!(dry.relocated.len(), 1);
        assert_eq!(dry.relocated[0].relpath, "renamed.jpg");
        assert_eq!(dry.relocated[0].survives_at, vec!["moved/renamed.jpg"]);
        assert_eq!(dry.missing, vec!["deleted.jpg"]);
        assert!(dry.not_indexed.is_empty());
        assert_eq!(ledger.counts().unwrap(), (2, 3));

        let applied = reconcile(&ledger, &fixture.tree(), "V", true).unwrap();
        assert!(applied.complete);
        assert_eq!(applied.dropped, 1);
        // The stale location goes; the bytes keep a row because `moved/renamed.jpg` holds them.
        assert_eq!(ledger.counts().unwrap(), (2, 2));
        assert!(ledger.location("V", "renamed.jpg").unwrap().is_none());
        // The real disappearance stays visible. That is the whole reason `index` keeps it.
        assert_eq!(
            crate::audit(&ledger, &fixture.tree(), "V", 0)
                .unwrap()
                .disagreements,
            vec!["unindexed absence: deleted.jpg"]
        );
    }

    /// The guard that makes this safe on a volume that is present while its contents are not.
    #[test]
    fn bytes_that_survive_only_at_another_absent_path_are_never_dropped() {
        let fixture = Fixture::new();
        let ledger = fixture.ledger();
        std::fs::write(fixture.tree().join("a.jpg"), b"identical").unwrap();
        std::fs::write(fixture.tree().join("b.jpg"), b"identical").unwrap();
        crate::index(&ledger, &fixture.tree(), "V", "v").unwrap();
        assert_eq!(ledger.counts().unwrap(), (1, 2), "one digest, two locations");

        std::fs::remove_file(fixture.tree().join("a.jpg")).unwrap();
        std::fs::remove_file(fixture.tree().join("b.jpg")).unwrap();

        let report = reconcile(&ledger, &fixture.tree(), "V", true).unwrap();
        assert!(report.complete);
        assert!(report.relocated.is_empty(), "no witness survives");
        assert_eq!(report.missing, vec!["a.jpg", "b.jpg"]);
        assert_eq!(report.dropped, 0);
        assert_eq!(ledger.counts().unwrap(), (1, 2), "the rows stay");
    }

    /// A whole volume whose contents are gone must reconcile to nothing, not to a mass deletion.
    #[test]
    fn an_empty_tree_drops_nothing() {
        let fixture = Fixture::new();
        let ledger = fixture.ledger();
        std::fs::write(fixture.tree().join("only.jpg"), b"bytes").unwrap();
        crate::index(&ledger, &fixture.tree(), "V", "v").unwrap();
        std::fs::remove_file(fixture.tree().join("only.jpg")).unwrap();

        let report = reconcile(&ledger, &fixture.tree(), "V", true).unwrap();
        assert_eq!(report.dropped, 0);
        assert_eq!(ledger.counts().unwrap(), (1, 1));
    }

    /// A location already present is left alone even when a twin of it is gone.
    #[test]
    fn a_present_location_is_never_reported() {
        let fixture = Fixture::new();
        let ledger = fixture.ledger();
        std::fs::write(fixture.tree().join("here.jpg"), b"same").unwrap();
        crate::index(&ledger, &fixture.tree(), "V", "v").unwrap();
        ledger
            .record(&Location {
                uuid: "V".into(),
                relpath: "gone.jpg".into(),
                digest: crate::hash(&fixture.tree().join("here.jpg")).unwrap(),
                size: 4,
                mtime_ns: 0,
            })
            .unwrap();

        let report = reconcile(&ledger, &fixture.tree(), "V", true).unwrap();
        assert_eq!(report.relocated.len(), 1);
        assert_eq!(report.relocated[0].survives_at, vec!["here.jpg"]);
        assert_eq!(report.dropped, 1);
        assert!(ledger.location("V", "here.jpg").unwrap().is_some());
        assert!(crate::audit(&ledger, &fixture.tree(), "V", 0)
            .unwrap()
            .disagreements
            .is_empty());
    }
}
