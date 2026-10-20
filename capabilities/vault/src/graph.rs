//! The wikilink graph, and which links resolve to a note that exists.
//!
//! ## Why resolution is a hashmap and not a search
//!
//! Obsidian resolves a bare `[[Name]]` by shortest-unique-path, which sounds
//! like it needs the whole tree. Measured on this vault it mostly does not:
//! 2,757 notes over 2,659 distinct basenames, **89 of those names duplicated**
//! (2026-09-08; it was 14 when this was written, and that growth is the reason
//! the rung below matters more than it used to). A map keyed on basename
//! answers the rest exactly, and the 89 are reported as ambiguous rather than
//! silently resolved to whichever the walk hit first. That is the honest shape —
//! a resolver that picks one and says nothing turns a wrong target into a
//! passing check, which is worse than a broken link because nothing flags it.
//!
//! ## Why frontmatter links are counted, and counted separately
//!
//! 10,995 of this vault's 22,344 wikilinks do not sit in prose at all.
//! They sit in `categories:`, `related:` and `sources:` — which is to say the
//! membership graph every MOC is fed by, and the provenance edges the whole
//! four-bucket model rests on. A link check that reads only the body misses
//! them and reports a vault 60% smaller than it is. This one was written that
//! way first and the acceptance fixture caught it.
//!
//! They stay a separate count because they behave differently: a folder move
//! rewrites a prose link and an editor rewrites a `categories:` entry, and a
//! number that mixes them cannot tell you which kind of repair you owe.
//!
//! ## Why dead links are counted twice
//!
//! A vault carries link-shaped text that was never meant to resolve: block
//! references (`[[#^abc123]]`), embeds of `.base` files, bare numbers. Counting
//! those as rot inflates the number and, worse, makes it unexplainable — the
//! figure moves and nobody can say whether the vault got better. So the report
//! carries both: every unresolved target, and the subset that actually looks
//! like a note. A migration is judged on the second and audited on the first.
//!
//! ## Why a target is also read relative to the note that carries it
//!
//! Obsidian resolves `[[x]]` against three addresses, not one: the path as
//! written from the vault root, the same path read from the linking note's own
//! folder, and finally the name alone. This resolver knew the first and the
//! third, which is why `_attachments/E-Autos-/Image 3.jpg` — written in a note
//! that has an `_attachments/` folder sitting beside it — read as dead. There is
//! no `_attachments/` at the vault root, and 35 files in this vault are called
//! `Image 3.jpg`, so rung one missed and rung three refused to guess. The rung
//! that would have answered was not there.
//!
//! Measured on the operator's vault 2026-09-08: **8,034 dead links before,
//! 5,211 after, 2,823 of them recovered by this rung alone** — 2,763 attachments
//! and 60 notes. No file moved. PRD D4 had attributed 2,739 of those to "image
//! links whose files are not in the vault"; the files are in the vault, one
//! folder below the note that names them. (5,184 once the bracket rule in
//! `targets_in` stopped counting Mermaid nodes and NumPy literals as links.)
//!
//! This is not the same act as guessing at an ambiguous basename, and the
//! difference is why it belongs here while that still does not: a relative path
//! names exactly one file, so the resolver either finds that file or does not.
//! Nothing is picked.

use std::collections::HashMap;

use serde::Serialize;

use crate::note::Note;

#[derive(Debug, Clone, Serialize)]
pub struct DeadLink {
    pub from: String,
    pub target: String,
    /// True when the link spelled a path (`Atlas/Media/X`) rather than a bare
    /// name. Path links are the ones a folder move breaks, and the ones a move
    /// must therefore rewrite.
    pub path_form: bool,
    pub in_frontmatter: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Ambiguity {
    pub basename: String,
    pub candidates: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct LinkReport {
    pub notes: usize,
    pub links_total: usize,
    pub links_in_frontmatter: usize,
    pub links_in_body: usize,
    pub links_resolved: usize,
    /// Of the resolved, how many name a file that is not a note — a PDF, an image, a `.base`.
    /// Reported rather than merged: a reader who sees the resolved count rise wants to know
    /// what started resolving.
    pub links_to_files: usize,
    /// Of the resolved, how many resolved ONLY because the target was read from the linking
    /// note's folder. Separate because it is the size of the instrument's old blind spot, and
    /// a reader watching this number fall is watching the vault adopt root-relative links.
    pub links_relative: usize,
    pub links_dead: usize,
    /// Dead links whose target looks like a note, excluding block refs, `.base`
    /// embeds and bare numbers. The number a migration is judged on.
    pub dead_note_shaped: usize,
    pub path_form_total: usize,
    pub path_form_dead: usize,
    pub distinct_dead_targets: usize,
    pub ambiguous_basenames: Vec<Ambiguity>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub dead: Vec<DeadLink>,
}

use markdown_root::wikilink_targets as targets_in;

/// `targets_in`, for the one other module that needs the same parse.
///
/// `people::linked_basenames` counts journal backlinks and must count exactly what the link
/// checker counts — a second wikilink parser would drift from this one within a week. The parser
/// lives in `markdown_root` since 2026-10-20, so `capabilities/entities` reads the same links.
pub fn targets_for_test(text: &str, body_start: usize) -> Vec<String> {
    targets_in(text, body_start)
        .into_iter()
        .map(|(target, _)| target)
        .collect()
}

/// Does this target look like it names a note at all?
fn note_shaped(target: &str) -> bool {
    if target.starts_with('^') {
        return false; // block reference
    }
    if target.ends_with(".base") || target.ends_with(".canvas") {
        return false;
    }
    if target.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    true
}

fn key(s: &str) -> String {
    s.to_lowercase()
}

/// `target` as an address, read from the folder of the note that carries the link.
///
/// The middle rung of Obsidian's resolution. `..` and `.` are folded here rather than left to
/// the filesystem, because this resolver never touches the filesystem — it compares against an
/// index built once — and because a `..` that would climb above the vault root names nothing
/// inside the vault. That case returns `None` instead of a path an index could accidentally
/// match.
///
/// A note at the vault root gets the same treatment and the answer is simply the target, which
/// the root-relative rung has already tried. Kept uniform rather than special-cased: the one
/// thing it adds is that a root note's `[[Name]]` finds its own sibling before the basename
/// index calls the name ambiguous.
fn relative_to(source_id: &str, target: &str) -> Option<String> {
    let folder = source_id.rsplit_once('/').map_or("", |(dir, _)| dir);
    let mut parts: Vec<&str> = if folder.is_empty() {
        Vec::new()
    } else {
        folder.split('/').collect()
    };
    for segment in target.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other),
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// The note index a link is resolved against. Two maps, because a link may name either
/// identity: the path from the vault root, or the file name alone.
struct NoteIndex {
    by_basename: HashMap<String, Vec<usize>>,
    by_id: HashMap<String, usize>,
}

/// What each of the three rungs answered, kept apart rather than folded to the first hit.
///
/// The caller needs to know WHICH rung answered, not only that one did: `links_relative` is
/// the count of links no rung but the middle one can see.
struct NoteRungs {
    by_path: Option<usize>,
    by_relative: Option<usize>,
    by_name: Option<usize>,
}

impl NoteRungs {
    /// Obsidian's order: the path as written, then the path from the linking note, then the
    /// name.
    fn hit(&self) -> Option<usize> {
        self.by_path.or(self.by_relative).or(self.by_name)
    }
}

impl NoteIndex {
    fn build(notes: &[Note]) -> Self {
        let mut by_basename: HashMap<String, Vec<usize>> = HashMap::new();
        let mut by_id: HashMap<String, usize> = HashMap::new();
        for (i, n) in notes.iter().enumerate() {
            by_basename.entry(key(&n.basename)).or_default().push(i);
            by_id.insert(key(n.id.trim_end_matches(".md")), i);
        }
        Self { by_basename, by_id }
    }

    /// One target, against all three rungs. `report` and `inbound` both go through here: two
    /// resolvers over one vault would drift apart, and the second one was wrong for a year.
    fn rungs(&self, source_id: &str, target: &str) -> NoteRungs {
        let base = target.rsplit('/').next().unwrap_or(target);
        NoteRungs {
            by_path: target
                .contains('/')
                .then(|| {
                    self.by_id
                        .get(&key(target.trim_end_matches(".md")))
                        .copied()
                })
                .flatten(),
            by_relative: relative_to(source_id, target)
                .and_then(|rel| self.by_id.get(&key(rel.trim_end_matches(".md"))).copied()),
            by_name: self
                .by_basename
                .get(&key(base.trim_end_matches(".md")))
                .and_then(|v| (v.len() == 1).then(|| v[0])),
        }
    }
}

/// The link report.
///
/// `attachments` are the vault's non-note files as root-relative ids — PDFs, images, `.base`
/// and `.canvas`. A wikilink may name any of them, and a checker that only knows notes calls
/// every such link dead: measured on the operator's vault on 2026-09-07, that was **685 of
/// 9,362** reported dead links, including every Base a hub note embeds. They resolve here and
/// are counted separately, because "resolved to a note" and "resolved to a file" are different
/// facts and the second one is the one a reader is surprised by.
pub fn report(notes: &[Note], attachments: &[String], include_dead: bool) -> LinkReport {
    let index = NoteIndex::build(notes);
    let by_basename = &index.by_basename;

    let mut ambiguous: Vec<Ambiguity> = by_basename
        .iter()
        .filter(|(_, v)| v.len() > 1)
        .map(|(k, v)| Ambiguity {
            basename: k.clone(),
            candidates: v.iter().map(|&i| notes[i].id.clone()).collect(),
        })
        .collect();
    ambiguous.sort_by(|a, b| a.basename.cmp(&b.basename));

    let mut total = 0usize;
    let mut resolved = 0usize;
    let mut path_form_total = 0usize;
    let mut path_form_dead = 0usize;
    let mut dead_note_shaped = 0usize;
    let mut dead: Vec<DeadLink> = Vec::new();
    let mut distinct_dead: HashMap<String, ()> = HashMap::new();

    // The same two identities, for files that are not notes. An extension is NOT trimmed: a
    // link to `Dokumente.base` names that file and not a note called `Dokumente`.
    let mut attachment_by_path: HashMap<String, ()> = HashMap::new();
    let mut attachment_by_name: HashMap<String, usize> = HashMap::new();
    for id in attachments {
        attachment_by_path.insert(key(id), ());
        let base = id.rsplit('/').next().unwrap_or(id);
        *attachment_by_name.entry(key(base)).or_default() += 1;
    }

    let mut links_to_files = 0usize;
    let mut links_relative = 0usize;
    let mut in_fm = 0usize;
    for n in notes {
        for (target, frontmatter) in targets_in(&n.text, n.body_start) {
            total += 1;
            if frontmatter {
                in_fm += 1;
            }
            let path_form = target.contains('/');
            if path_form {
                path_form_total += 1;
            }

            // Every rung is asked, and none of them short-circuits, because `links_relative`
            // has to say how many links the middle rung is the ONLY one that sees. A chain
            // that stops at the first hit cannot answer that.
            let relative = relative_to(&n.id, &target);
            let base = target.rsplit('/').next().unwrap_or(&target);
            let NoteRungs {
                by_path,
                by_relative,
                by_name,
            } = index.rungs(&n.id, &target);

            // The same three rungs against the files that are not notes.
            let file_by_path = attachment_by_path.contains_key(&key(&target));
            let file_by_relative = relative
                .as_deref()
                .is_some_and(|rel| attachment_by_path.contains_key(&key(rel)));
            // An ambiguous basename is not a resolution, the same rule the note index above
            // applies: two files with one name mean the link names neither.
            let file_by_name = attachment_by_name.get(&key(base)) == Some(&1);

            let hit = by_path.or(by_relative).or(by_name);
            let file_hit = hit.is_none() && (file_by_path || file_by_relative || file_by_name);

            // "Only the relative rung sees this" — which is to say, the set of links that were
            // dead before the rung existed. Measured 2,823 on the operator's vault.
            if (by_relative.is_some() || file_by_relative)
                && by_path.is_none()
                && by_name.is_none()
                && !file_by_path
                && !file_by_name
            {
                links_relative += 1;
            }

            match hit {
                Some(_) => resolved += 1,
                None if file_hit => {
                    resolved += 1;
                    links_to_files += 1;
                }
                None => {
                    if path_form {
                        path_form_dead += 1;
                    }
                    if note_shaped(&target) {
                        dead_note_shaped += 1;
                    }
                    distinct_dead.insert(key(&target), ());
                    if include_dead {
                        dead.push(DeadLink {
                            from: n.id.clone(),
                            target: target.clone(),
                            path_form,
                            in_frontmatter: frontmatter,
                        });
                    }
                }
            }
        }
    }

    dead.sort_by(|a, b| (&a.from, &a.target).cmp(&(&b.from, &b.target)));

    LinkReport {
        notes: notes.len(),
        links_total: total,
        links_in_frontmatter: in_fm,
        links_in_body: total - in_fm,
        links_resolved: resolved,
        links_to_files,
        links_relative,
        links_dead: total - resolved,
        dead_note_shaped,
        path_form_total,
        path_form_dead,
        distinct_dead_targets: distinct_dead.len(),
        ambiguous_basenames: ambiguous,
        dead,
    }
}

/// Which notes inside `folder` are linked from outside it.
///
/// The question a folder move has to answer before it runs: what breaks that is not already
/// broken. Returns distinct note ids, because a move rewrites a target once however many times
/// it is referenced.
///
/// Two things were wrong here until 2026-09-08, both of them the same shape as D4.
///
/// It compared `note.folder`, which is only the FIRST path segment, so `--inbound Atlas/People`
/// found nothing at all and said so as "0 distinct notes" rather than as an error. And it
/// matched targets by basename against the folder's basenames, which counts a link as inbound
/// when some other note of the same name is the one it actually resolves to — 139 against 134
/// on the operator's vault, five links that name a note in `Knowledge/` and open one somewhere
/// else. It now resolves through `NoteIndex::rungs`, the same ladder `report` uses, and asks
/// where the link actually lands.
pub fn inbound(notes: &[Note], folder: &str) -> Vec<String> {
    let index = NoteIndex::build(notes);
    let prefix = format!("{}/", folder.trim_end_matches('/'));

    let mut hits: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for n in notes.iter().filter(|n| !n.id.starts_with(&prefix)) {
        for (target, _) in targets_in(&n.text, n.body_start) {
            if let Some(i) = index.rungs(&n.id, &target).hit() {
                if notes[i].id.starts_with(&prefix) {
                    hits.insert(notes[i].id.clone());
                }
            }
        }
    }
    hits.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(id: &str, body: &str) -> Note {
        Note {
            id: id.to_string(),
            basename: id
                .rsplit('/')
                .next()
                .unwrap_or(id)
                .trim_end_matches(".md")
                .to_string(),
            folder: id.split('/').next().unwrap_or("").to_string(),
            text: body.to_string(),
            body_start: 0,
            fields: Default::default(),
            raw_frontmatter: None,
            path: std::path::PathBuf::from(id),
        }
    }

    /// The 2026-09-07 finding, as a test: a hub note embeds a Base, and the checker called it
    /// dead. 1,330 of 9,362 reported dead links on the operator's vault were this.
    #[test]
    fn a_link_to_a_base_or_a_pdf_resolves_when_the_file_exists() {
        let notes = vec![note(
            "Atlas/Documents/Documents.md",
            "![[Resources/Bases/Dokumente.base]] and [[Abiturzeugnis.pdf]]",
        )];
        let attachments = vec![
            "Resources/Bases/Dokumente.base".to_string(),
            "Atlas/Documents/Bildung/Schule/Abiturzeugnis.pdf".to_string(),
        ];
        let report = report(&notes, &attachments, true);
        assert_eq!((report.links_dead, report.links_to_files), (0, 2));
        assert!(report.dead.is_empty());
    }

    #[test]
    fn a_file_that_is_not_there_is_still_dead() {
        let notes = vec![note("A.md", "[[Missing.pdf]]")];
        let report = report(&notes, &[], true);
        assert_eq!(report.links_dead, 1);
    }

    /// The same rule the note index applies: two files with one name resolve neither, because
    /// a link that could mean either means nothing checkable.
    #[test]
    fn an_ambiguous_attachment_basename_does_not_resolve() {
        let notes = vec![note("A.md", "[[Scan.pdf]]")];
        let attachments = vec!["One/Scan.pdf".to_string(), "Two/Scan.pdf".to_string()];
        assert_eq!(report(&notes, &attachments, true).links_dead, 1);
    }

    /// An extension is part of an attachment's name and is not trimmed. `Dokumente.base` and a
    /// note called `Dokumente` are different targets, and Obsidian treats them that way.
    #[test]
    fn an_attachment_extension_is_part_of_its_name() {
        let notes = vec![note("A.md", "[[Dokumente]]")];
        let attachments = vec!["Resources/Bases/Dokumente.base".to_string()];
        assert_eq!(report(&notes, &attachments, true).links_dead, 1);
    }

    /// D4's largest single cause, as a test. The note sits in `.../E-Autos_/`, its images sit
    /// in `.../E-Autos_/_attachments/E-Autos-/`, and the link names the second from the first.
    /// There is no `_attachments/` at the vault root, and `Image .jpg` is a name dozens of
    /// folders use — so before the relative rung existed this was dead twice over. 2,763 of the
    /// operator's dead links had exactly this shape.
    #[test]
    fn an_attachment_beside_the_note_resolves_and_a_global_twin_does_not_make_it_ambiguous() {
        let notes = vec![note(
            "Atlas/Personal/Archive/Hobbys/E-Autos_/E-Autos-.md",
            "![[_attachments/E-Autos-/Image .jpg]]",
        )];
        let attachments = vec![
            "Atlas/Personal/Archive/Hobbys/E-Autos_/_attachments/E-Autos-/Image .jpg".to_string(),
            // The twin that makes the basename index refuse. It is the reason the middle rung
            // is needed rather than a nicety on top of one that already worked.
            "Atlas/Personal/Archive/Schule/Mathe/Mathe 11/_attachments/Image .jpg".to_string(),
        ];
        let rep = report(&notes, &attachments, true);
        assert_eq!(
            (rep.links_dead, rep.links_to_files, rep.links_relative),
            (0, 1, 1),
            "dead: {:?}",
            rep.dead
        );
    }

    /// The same rung, for notes: `Atlas/Events/X.md` naming `../Reflections/Y`. 60 of the
    /// operator's dead links were notes rather than attachments. The twin in `Knowledge/` is
    /// what makes this a measurement of the relative rung — with one `Signal-Misreading` in the
    /// vault the name alone would have found it, and the assertion would prove nothing.
    #[test]
    fn a_dot_dot_target_climbs_out_of_the_linking_notes_folder() {
        let notes = vec![
            note(
                "Atlas/Events/Berlin.md",
                "[[../Reflections/Signal-Misreading]]",
            ),
            note("Atlas/Reflections/Signal-Misreading.md", ""),
            note("Knowledge/Meta/Signal-Misreading.md", ""),
        ];
        let rep = report(&notes, &[], true);
        assert_eq!(
            (rep.links_dead, rep.links_relative),
            (0, 1),
            "dead: {:?}",
            rep.dead
        );
    }

    /// The rung is a lookup and not a search: a relative address that names nothing is still
    /// dead. Without this the previous two tests would pass against a resolver that simply
    /// stopped reporting anything.
    #[test]
    fn a_relative_target_that_names_nothing_is_still_dead() {
        let notes = vec![note(
            "Atlas/Events/Berlin.md",
            "[[../Reflections/Never Written]] and ![[_attachments/Absent.jpg]]",
        )];
        let attachments = vec!["Elsewhere/_attachments/Different.jpg".to_string()];
        let rep = report(&notes, &attachments, true);
        assert_eq!((rep.links_dead, rep.links_relative), (2, 0));
    }

    /// `..` past the vault root names nothing inside the vault. A resolver that folded the
    /// surplus `..` away would read the tail — `Secret/Elsewhere` — as a root-relative address
    /// and resolve it, which is the wrong file for the right-looking reason. The basename rung
    /// is taken out of the way here by a second `Elsewhere`, so this measures the climb and not
    /// the fallback.
    #[test]
    fn a_target_that_climbs_above_the_root_does_not_resolve() {
        let notes = vec![
            note("Atlas/Escape.md", "[[../../Secret/Elsewhere]]"),
            note("Secret/Elsewhere.md", ""),
            note("Other/Elsewhere.md", ""),
        ];
        let rep = report(&notes, &[], true);
        assert_eq!(
            (rep.links_dead, rep.links_relative),
            (1, 0),
            "dead: {:?}",
            rep.dead
        );
    }

    /// A bracket inside the span means the outer brackets were not a link. Obsidian forbids
    /// `[` and `]` in a note name, so both of these are the surrounding syntax and neither is
    /// a dead link. Both shapes are taken verbatim from the vault.
    #[test]
    fn a_bracket_inside_the_span_means_it_was_never_a_wikilink() {
        let notes = vec![note(
            "Knowledge/NumPy.md",
            "np.array([[1, 2], [3, 4]])\n<% tp.date.now('yyyy-[W]ww', 0) %>\nand [[Real Link]]",
        )];
        let rep = report(&notes, &[], true);
        assert_eq!(
            (rep.links_total, rep.links_dead),
            (1, 1),
            "dead: {:?}",
            rep.dead
        );
        assert_eq!(rep.dead[0].target, "Real Link");
    }

    /// `inbound` over a nested folder. The old comparison was against `note.folder`, the first
    /// path segment only, so this question answered zero — and answered it as a fact.
    #[test]
    fn inbound_answers_for_a_nested_folder() {
        let notes = vec![
            note("Atlas/People/Erika.md", ""),
            note("Journal/2026-01-05.md", "coffee with [[Erika]]"),
        ];
        assert_eq!(
            inbound(&notes, "Atlas/People"),
            vec!["Atlas/People/Erika.md"]
        );
    }

    /// A link is inbound when it OPENS a note in the folder, not when it shares a name with
    /// one. Two notes called `Communication`, and the linking note sits beside the one that is
    /// not in `Knowledge/` — so the link resolves to its sibling and `Knowledge/` gains nothing.
    #[test]
    fn a_link_that_resolves_elsewhere_is_not_inbound() {
        let notes = vec![
            note("Knowledge/Meta/Mind/Communication.md", ""),
            note("Atlas/Reflections/Communication.md", ""),
            note(
                "Atlas/Reflections/Signal-Misreading.md",
                "see [[Communication]]",
            ),
        ];
        assert!(
            inbound(&notes, "Knowledge").is_empty(),
            "a name match was counted as a link into the folder"
        );

        // The control: move the linking note away from its sibling and the same link becomes
        // ambiguous rather than inbound — still not a hit, and for the honest reason.
        let notes = vec![
            note("Knowledge/Meta/Mind/Communication.md", ""),
            note("Atlas/Reflections/Communication.md", ""),
            note("Journal/2026-01-05.md", "see [[Communication]]"),
        ];
        assert!(inbound(&notes, "Knowledge").is_empty());

        // And with the twin gone it resolves, which proves the two cases above are not just
        // `inbound` returning nothing.
        let notes = vec![
            note("Knowledge/Meta/Mind/Communication.md", ""),
            note("Journal/2026-01-05.md", "see [[Communication]]"),
        ];
        assert_eq!(
            inbound(&notes, "Knowledge"),
            vec!["Knowledge/Meta/Mind/Communication.md"]
        );
    }

    /// Precedence, stated as a test because the two rungs disagree here on purpose. A path
    /// written from the vault root means that path, even when a file of the same relative name
    /// sits beside the linking note — that is the order Obsidian resolves in, and the order
    /// that makes `links_relative` mean "only the relative rung can see this".
    #[test]
    fn the_root_relative_address_wins_and_is_not_counted_as_relative() {
        let notes = vec![
            note("Atlas/Events/Berlin.md", "[[Atlas/Events/Notes]]"),
            note("Atlas/Events/Notes.md", ""),
            note("Atlas/Events/Atlas/Events/Notes.md", ""),
        ];
        let rep = report(&notes, &[], true);
        assert_eq!((rep.links_dead, rep.links_relative), (0, 0));
    }
}
