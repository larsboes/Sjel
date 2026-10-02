//! Duplicate and look-alike reporting across one volume's indexed locations.
//!
//! Two facts, deliberately kept apart because the 2026-09-29 merge was wrong about both:
//!
//! * A **duplicate group** is one digest with two or more locations. The bytes are equal, so a copy
//!   is redundant — but only if another location in the same group survives.
//! * A **look-alike group** is two or more *different* digests sharing a basename and a byte count.
//!   Those are not duplicates and must never be treated as such: 4 of 17,583 same-name, same-size
//!   pairs on this drive were different files, including truncated DJI MP4s.
//!
//! This verb deletes nothing and has no policy of its own. Which copy is redundant is the
//! operator's declaration, passed in as `--legacy`: a location under that prefix is superseded when
//! an equal-digest location exists outside it. A group whose every copy is legacy yields no
//! removals at all, because removing one would lose the bytes.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::store::{Ledger, Result};

/// How to resolve a group the `--legacy` prefix cannot decide: every copy sits inside one prefix,
/// so which one survives has to come from somewhere other than the path.
pub struct Resolve<'a> {
    /// Only groups whose every location is under this prefix are considered.
    pub inside: &'a str,
    /// Read each copy's embedded capture date and keep the one whose bucket agrees with it.
    pub metadata: bool,
}

/// `YYYY-MM` from a path under a prefix: `by-date/2018-04/x.mov` with prefix `by-date` -> `2018-04`.
fn bucket_month(relpath: &str, prefix: &str) -> Option<String> {
    let rest = relpath.strip_prefix(prefix)?.strip_prefix('/')?;
    let month = rest.get(..7)?;
    crate::preview::metadata_day(&format!("{month}-01"))?;
    Some(month.to_owned())
}

/// The embedded capture month of each file, from one ExifTool pass.
///
/// The field precedence is the preview's, so the two verbs cannot disagree about what a capture
/// date is. A file with no usable date is absent from the map, which the caller treats as unknown
/// rather than inventing a month.
/// The embedded capture month of each file, from the one shared ExifTool reader.
fn capture_months(root: &Path, relpaths: &[String]) -> Result<BTreeMap<String, String>> {
    Ok(crate::preview::capture_days(root, relpaths)?
        .into_iter()
        .map(|(rel, (day, _))| (rel, day[..7.min(day.len())].to_owned()))
        .collect())
}

#[derive(Debug, Serialize)]
pub struct DuplicateReport {
    pub uuid: String,
    pub locations: usize,
    /// Digest groups. `locations` minus this equals how many rows a perfect dedup would remove.
    pub duplicate_groups: usize,
    pub superseded_files: usize,
    pub superseded_bytes: u64,
    /// Groups where every copy is under `--legacy`, so nothing may be removed.
    pub groups_with_no_other_copy: usize,
    /// Groups the `--legacy` rule could not decide and the metadata rule resolved instead.
    pub resolved_by_metadata: usize,
    /// Groups resolved by metadata whose bucket choice was a tie-break rather than a match.
    pub resolved_without_a_metadata_match: usize,
    pub lookalike_groups: usize,
    pub lookalike_locations: usize,
    pub legacy_prefix: Option<String>,
    pub resolve_inside: Option<String>,
    pub list: Option<PathBuf>,
    /// Groups whose metadata could not be read, named rather than folded into a count.
    pub issues: Vec<String>,
    pub groups: Vec<Group>,
    pub lookalikes: Vec<Lookalike>,
    pub limitations: Vec<&'static str>,
}

#[derive(Debug, Serialize)]
pub struct Group {
    pub digest: String,
    pub size: u64,
    /// Every path holding these bytes. Never filtered, so nothing is hidden from review.
    pub locations: Vec<String>,
    /// The copies that survive: equal digest, outside the legacy prefix.
    pub kept: Vec<String>,
    /// The copies under the legacy prefix — redundant, but only offered when `removable`.
    pub superseded: Vec<String>,
    /// True only when a copy outside the legacy prefix exists, so removing the superseded ones
    /// cannot lose the bytes.
    pub removable: bool,
    /// Why this copy survives, when the choice came from metadata rather than the legacy prefix.
    pub resolution: Option<String>,
    /// Set when nothing may be removed, with the reason.
    pub must_keep: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Lookalike {
    pub basename: String,
    pub size: u64,
    pub digests: Vec<String>,
    pub locations: Vec<String>,
}

const LIMITATIONS: [&str; 4] = [
    "Reports the index, not the disk: run `media index` first, and `media audit` to confirm the index still matches.",
    "Digest equality decides duplicates. A look-alike group shares a basename and a size but holds different bytes, so it is never a removal candidate.",
    "`--legacy` is the operator's declaration of which copy is superseded, not a property of the files.",
    "Removing a superseded location is a separate, declared step. This verb opens the store for reading and writes nothing to a volume.",
];

fn is_legacy(relpath: &str, prefix: Option<&str>) -> bool {
    prefix.is_some_and(|p| relpath == p || relpath.starts_with(&format!("{p}/")))
}

fn basename(relpath: &str) -> String {
    relpath.rsplit('/').next().unwrap_or(relpath).to_lowercase()
}

/// (path to keep, paths to supersede, why, whether metadata actually matched)
type Resolution = (String, Vec<String>, String, bool);

/// How many macOS duplicate suffixes a name carries: `X.MP4` is 0, `X 2.MP4` and `X (1).MP4` are 1,
/// `X (1) 2.MP4` is 2. An export can leave one video in a folder four times over, and the library's
/// convention is the plain name — the 2026-09-29 handoff named this as the fix to build rather than
/// patch afterwards.
fn suffix_depth(name: &str) -> usize {
    let mut depth = 0;
    let mut current = name.to_owned();
    while let Some(next) = crate::supersede::strip_one_suffix(&current) {
        depth += 1;
        current = next;
        if depth > 8 {
            break;
        }
    }
    depth
}

/// Choose which copy survives when every copy of a group sits under one prefix.
///
/// Two things decide it, in order. First, when `--metadata` is on, a copy whose `prefix/YYYY-MM`
/// bucket agrees with the file's embedded capture date is preferred over one whose bucket does not.
/// Then, whatever the bucket said, the plainest name wins — a copy carrying ` 2` or ` (1)` is an
/// export artefact and the library keeps the plain name. Ties go to the earliest path.
///
/// A tie-break is reported as a tie-break, never as a match, so the choice is never dressed up as
/// evidence it is not.
fn resolve_group(
    root: &Path,
    locations: &[String],
    rule: &Resolve<'_>,
) -> Result<Option<Resolution>> {
    let months = if rule.metadata {
        capture_months(root, locations)?
    } else {
        BTreeMap::new()
    };
    let matching: Vec<&String> = locations
        .iter()
        .filter(|rel| {
            let Some(bucket) = bucket_month(rel, rule.inside) else {
                return false;
            };
            months.get(*rel).is_some_and(|month| bucket == *month)
        })
        .collect();
    let matched = !matching.is_empty();
    // Prefer a copy whose bucket agreed with its capture date, then the plainest name, then the
    // earliest path. `locations` is already sorted, so the last comparison is stable.
    let candidates: Vec<&String> = if matched {
        matching
    } else {
        locations.iter().collect()
    };
    let keep = candidates
        .into_iter()
        .min_by_key(|rel| {
            let name = rel.rsplit('/').next().unwrap_or(rel.as_str());
            (suffix_depth(name), rel.as_str())
        })
        .ok_or("a duplicate group has no locations")?
        .to_string();
    let name = keep.rsplit('/').next().unwrap_or(&keep);
    let why = match (matched, suffix_depth(name)) {
        (true, 0) => format!("kept {name}: its bucket matches its embedded capture date and its name is plain"),
        (true, n) => format!("kept {name}: its bucket matches its embedded capture date ({n} duplicate suffix(es) remain)"),
        (false, 0) => format!("kept {name}: the plainest name, with no export duplicate suffix"),
        (false, n) => format!("kept {name}: the plainest of the copies, which still carry {n} duplicate suffix(es)"),
    };
    let gone: Vec<String> = locations
        .iter()
        .filter(|rel| **rel != keep)
        .cloned()
        .collect();
    Ok(Some((keep, gone, why, matched)))
}

pub fn duplicates(
    ledger: &Ledger,
    uuid: &str,
    root: &Path,
    legacy: Option<&str>,
    resolve: Option<Resolve<'_>>,
    list: Option<&Path>,
) -> Result<DuplicateReport> {
    let locations = ledger.locations(uuid)?;
    let mut by_digest: BTreeMap<String, Vec<(String, i64)>> = BTreeMap::new();
    let mut by_shape: BTreeMap<(i64, String), Vec<(String, String)>> = BTreeMap::new();
    for row in &locations {
        by_digest
            .entry(row.digest.clone())
            .or_default()
            .push((row.relpath.clone(), row.size));
        by_shape
            .entry((row.size, basename(&row.relpath)))
            .or_default()
            .push((row.digest.clone(), row.relpath.clone()));
    }

    let mut groups = Vec::new();
    let mut superseded_files = 0;
    let mut superseded_bytes = 0;
    let mut groups_with_no_other_copy = 0;
    let mut resolved_by_metadata = 0;
    let mut resolved_without_a_metadata_match = 0;
    let mut resolve_issues: Vec<String> = Vec::new();
    for (digest, rows) in by_digest.iter().filter(|(_, rows)| rows.len() > 1) {
        let mut kept: Vec<String> = Vec::new();
        let mut superseded: Vec<String> = Vec::new();
        for (relpath, _) in rows {
            if is_legacy(relpath, legacy) {
                superseded.push(relpath.clone());
            } else {
                kept.push(relpath.clone());
            }
        }
        kept.sort();
        superseded.sort();
        let size = rows[0].1.max(0) as u64;
        let mut all: Vec<String> = rows.iter().map(|(relpath, _)| relpath.clone()).collect();
        all.sort();
        // A group the legacy prefix cannot decide — because every copy sits on the same side of it —
        // may still be decidable by the resolver. The condition is that every copy is under the
        // declared prefix, never that some other prefix claimed them first.
        let mut resolution = None;
        if let Some(rule) = &resolve {
            if all.iter().all(|rel| is_legacy(rel, Some(rule.inside))) {
                match resolve_group(root, &all, rule) {
                    Ok(Some((keep, gone, why, matched))) => {
                        kept = vec![keep];
                        superseded = gone;
                        resolution = Some(why);
                        if matched {
                            resolved_by_metadata += 1;
                        } else {
                            resolved_without_a_metadata_match += 1;
                        }
                    }
                    Ok(None) => {}
                    Err(error) => resolve_issues.push(format!("group {}: {error}", &digest[..12])),
                }
            }
        }
        let removable = !kept.is_empty();
        let must_keep = if removable {
            superseded_files += superseded.len();
            superseded_bytes += size * superseded.len() as u64;
            None
        } else {
            groups_with_no_other_copy += 1;
            Some(format!(
                "every copy sits under the declared legacy prefix {}; removing one would lose the bytes",
                legacy.unwrap_or("")
            ))
        };
        groups.push(Group {
            digest: digest.clone(),
            size,
            locations: all,
            kept,
            superseded,
            removable,
            resolution,
            must_keep,
        });
    }

    let mut lookalikes = Vec::new();
    let mut lookalike_locations = 0;
    for ((size, name), rows) in &by_shape {
        let digests: BTreeSet<&str> = rows.iter().map(|(d, _)| d.as_str()).collect();
        if digests.len() < 2 {
            continue;
        }
        let mut sorted: Vec<String> = rows.iter().map(|(_, rel)| rel.clone()).collect();
        sorted.sort();
        lookalike_locations += sorted.len();
        lookalikes.push(Lookalike {
            basename: name.clone(),
            size: (*size).max(0) as u64,
            digests: digests.into_iter().map(str::to_owned).collect(),
            locations: sorted,
        });
    }

    // A metadata resolution is an extra limitation; a group whose metadata could not be read is
    // named in `issues` rather than folded into a count.
    let mut limitations: Vec<&'static str> = LIMITATIONS.to_vec();
    if resolve.as_ref().is_some_and(|rule| rule.metadata) {
        limitations.push("A metadata resolution keeps the copy whose bucket agrees with the file's embedded capture date; a group with no agreement is kept by earliest bucket and counted separately.");
    }

    if let Some(path) = list {
        let file = File::create(path)?;
        let mut out = BufWriter::new(file);
        writeln!(out, "digest\tsize\tsuperseded\tkept")?;
        for group in &groups {
            if !group.removable {
                continue;
            }
            if let Some(stays) = group.kept.first() {
                for gone in &group.superseded {
                    writeln!(out, "{}\t{}\t{}\t{}", group.digest, group.size, gone, stays)?;
                }
            }
        }
        out.flush()?;
    }

    Ok(DuplicateReport {
        uuid: uuid.to_owned(),
        locations: locations.len(),
        duplicate_groups: groups.len(),
        superseded_files,
        superseded_bytes,
        groups_with_no_other_copy,
        resolved_by_metadata,
        resolved_without_a_metadata_match,
        lookalike_groups: lookalikes.len(),
        lookalike_locations,
        legacy_prefix: legacy.map(str::to_owned),
        resolve_inside: resolve.as_ref().map(|rule| rule.inside.to_owned()),
        list: list.map(Path::to_path_buf),
        issues: resolve_issues,
        groups,
        lookalikes,
        limitations,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Location;

    fn ledger(dir: &Path) -> Ledger {
        let _ = std::fs::remove_dir_all(dir);
        std::fs::create_dir_all(dir).unwrap();
        let db = Ledger::open(&dir.join("scratch.db")).unwrap();
        // A location references its volume, so the volume has to exist first.
        db.register("V", "v").unwrap();
        db
    }

    fn put(db: &Ledger, relpath: &str, digest: &str, size: i64) {
        db.record(&Location {
            uuid: "V".into(),
            relpath: relpath.into(),
            digest: digest.into(),
            size,
            mtime_ns: 0,
        })
        .unwrap();
    }

    #[test]
    fn a_legacy_copy_is_superseded_only_while_an_equal_digest_copy_survives() {
        let dir = std::env::temp_dir().join(format!("media-dupes-{}", std::process::id()));
        let db = ledger(&dir);
        put(&db, "Trips/2018/2018-03-Iceland/IMG_7811.MOV", "aaa", 100);
        put(&db, "by-date/2018-03/IMG_7811.MOV", "aaa", 100);
        put(&db, "by-date/2016-01/only-copy.JPG", "bbb", 50);
        let report = duplicates(&db, "V", Path::new("/"), Some("by-date"), None, None).unwrap();
        assert_eq!(
            report.duplicate_groups, 1,
            "only digest aaa has two locations"
        );
        // The Iceland pair: one copy superseded, one kept.
        let iceland = report
            .groups
            .iter()
            .find(|g| g.digest == "aaa")
            .expect("iceland group");
        assert_eq!(
            iceland.kept,
            vec!["Trips/2018/2018-03-Iceland/IMG_7811.MOV"]
        );
        assert_eq!(iceland.superseded, vec!["by-date/2018-03/IMG_7811.MOV"]);
        assert_eq!(iceland.must_keep, None);
        assert!(iceland.removable);
        assert_eq!(iceland.locations.len(), 2);
        // "bbb" has one location, so it is no duplicate group at all.
        assert!(report.groups.iter().all(|g| g.digest != "bbb"));
        assert_eq!(report.superseded_files, 1);
        assert_eq!(report.superseded_bytes, 100);
        assert_eq!(report.groups_with_no_other_copy, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_group_that_is_entirely_legacy_yields_no_removals() {
        let dir = std::env::temp_dir().join(format!("media-dupes-all-{}", std::process::id()));
        let db = ledger(&dir);
        put(&db, "by-date/2018-03/one.JPG", "ccc", 10);
        put(&db, "by-date/2018-04/one.JPG", "ccc", 10);
        let report = duplicates(&db, "V", Path::new("/"), Some("by-date"), None, None).unwrap();
        assert_eq!(report.duplicate_groups, 1);
        assert_eq!(report.superseded_files, 0, "nothing may be removed");
        assert_eq!(report.groups_with_no_other_copy, 1);
        assert!(!report.groups[0].removable);
        // Both copies are still named, so the group can be reviewed even though it is not offered.
        assert_eq!(report.groups[0].superseded.len(), 2);
        assert_eq!(report.groups[0].locations.len(), 2);
        assert!(report.groups[0].kept.is_empty());
        assert!(report.groups[0]
            .must_keep
            .as_ref()
            .unwrap()
            .contains("would lose the bytes"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_lookalike_is_reported_and_never_offered_for_removal() {
        let dir = std::env::temp_dir().join(format!("media-look-{}", std::process::id()));
        let db = ledger(&dir);
        put(&db, "Trips/2019/2019-05/DJI_0639.MP4", "ddd", 20_000);
        put(&db, "by-date/2019-05/DJI_0639.MP4", "eee", 20_000);
        let report = duplicates(&db, "V", Path::new("/"), Some("by-date"), None, None).unwrap();
        assert_eq!(
            report.duplicate_groups, 0,
            "different bytes is not a duplicate"
        );
        assert_eq!(report.superseded_files, 0);
        assert_eq!(report.lookalike_groups, 1);
        assert_eq!(report.lookalike_locations, 2);
        let look = &report.lookalikes[0];
        assert_eq!(look.basename, "dji_0639.mp4");
        assert_eq!(look.digests, vec!["ddd", "eee"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_removal_list_names_the_surviving_copy_for_every_superseded_path() {
        let dir = std::env::temp_dir().join(format!("media-list-{}", std::process::id()));
        let db = ledger(&dir);
        put(&db, "Events/2019/x/clip.MOV", "fff", 7);
        put(&db, "by-date/2019-01/clip.MOV", "fff", 7);
        let list = dir.join("removal-list.tsv");
        let report =
            duplicates(&db, "V", Path::new("/"), Some("by-date"), None, Some(&list)).unwrap();
        assert_eq!(report.superseded_files, 1);
        let text = std::fs::read_to_string(&list).unwrap();
        let mut lines = text.lines();
        assert_eq!(lines.next().unwrap(), "digest\tsize\tsuperseded\tkept");
        assert_eq!(
            lines.next().unwrap(),
            "fff\t7\tby-date/2019-01/clip.MOV\tEvents/2019/x/clip.MOV"
        );
        assert_eq!(lines.next(), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
