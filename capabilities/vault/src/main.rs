//! `vault` — read an Obsidian vault as data.
//!
//! Four verbs today, all read-only:
//!
//! ```text
//! vault links [--root PATH] [--json] [--dead] [--inbound FOLDER]
//! vault lint  [--root PATH] [--json] [--carrying KEY]
//! vault class [--root PATH] [--json] [--only c2] [--list]
//! vault people [--root PATH] [--json]
//! vault journal [--root PATH] [--json]
//! ```
//!
//! ## Why this exists as a binary rather than a skill
//!
//! Every planned vault operation — the archive move, the dialect normalisation,
//! the understanding axis, the session deposit — starts by asking the same two
//! questions: what is in here, and what links to what. A skill that describes
//! how to answer them gets a different answer each run. A binary with tests
//! gets the same one, and a migration can be gated on it.
//!
//! ## What checks the counts
//!
//! A second implementation run at the same moment, not a saved number. The
//! table of seven figures measured by `find` and `rg` in August was retired on
//! 2026-09-08: it disagreed with the vault on every line, and one of its rows —
//! 996 notes carrying a `knowledge:` key — described a key the vault no longer
//! has at all. The vault gained 509 notes and dropped a frontmatter convention,
//! and a saved count over a hand-edited vault cannot survive that. Every
//! mismatch then reads as "the vault moved again", which is a test that cannot
//! fail.
//!
//! What holds instead is the property that table actually had: two
//! implementations sharing no code, agreeing on the same vault or explaining why
//! not. See the README for the 2026-09-08 run and the three defects writing the
//! second implementation found.

// The modules live in the library beside this binary, so `vault-server` reads
// notes through the same loader rather than a second copy of it.
use vault::{bases, class, graph, journal, lint, names, note, people};

fn flag(args: &[String], name: &str) -> Option<String> {
    let i = args.iter().position(|a| a == name)?;
    args.get(i + 1).filter(|v| !v.starts_with("--")).cloned()
}

fn has(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}

fn die(msg: impl std::fmt::Display) -> ! {
    eprintln!("error: {msg}");
    std::process::exit(1);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("");
    let json = has(&args, "--json");

    if matches!(cmd, "" | "-h" | "--help" | "help") {
        eprintln!(
            "vault — read an Obsidian vault as data\n\
             \n\
             usage:\n  \
               vault links [--root PATH] [--json] [--dead] [--inbound FOLDER]\n  \
               vault lint  [--root PATH] [--json] [--carrying KEY]\n  \
               vault names [--root PATH] [--json] [--folder Atlas/People]\n  \
               vault class [--root PATH] [--json] [--only c2] [--list]\n  \
               vault people [--root PATH] [--json]\n  \
               vault journal [--root PATH] [--json]\n  \
               vault bases [--root PATH] [--json] [--strict]\n\
             \n\
             The root comes from the overlay's config/knowledge.toml unless --root says otherwise."
        );
        std::process::exit(if cmd.is_empty() { 1 } else { 0 });
    }

    let root = note::resolve_root(flag(&args, "--root")).unwrap_or_else(|e| die(e));
    let (notes, problems) = note::load_all(&root).unwrap_or_else(|e| die(e));

    match cmd {
        "links" => {
            if let Some(folder) = flag(&args, "--inbound") {
                let targets = graph::inbound(&notes, &folder);
                if json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "folder": folder,
                            "inbound_distinct": targets.len(),
                            "targets": targets,
                        }))
                        .unwrap_or_default()
                    );
                } else {
                    println!(
                        "{} distinct notes in {folder}/ are linked from outside it",
                        targets.len()
                    );
                    for t in &targets {
                        println!("  {t}");
                    }
                }
                return;
            }

            // The vault's non-note files, so a link to a PDF, an image or a Base resolves
            // instead of reading as dead. Ids, not paths, because that is what a wikilink in
            // path form carries.
            let attachments: Vec<String> = root
                .files_recursive()
                .map(|files| {
                    files
                        .iter()
                        .filter(|path| path.extension().and_then(|e| e.to_str()) != Some("md"))
                        .filter_map(|path| root.relative_id(path))
                        .collect()
                })
                .unwrap_or_default();
            let rep = graph::report(&notes, &attachments, has(&args, "--dead"));
            if json {
                println!("{}", serde_json::to_string_pretty(&rep).unwrap_or_default());
            } else {
                println!("notes                  {}", rep.notes);
                println!("wikilinks              {}", rep.links_total);
                println!("  in frontmatter       {}", rep.links_in_frontmatter);
                println!("  in body              {}", rep.links_in_body);
                println!("  resolved             {}", rep.links_resolved);
                println!("    to a non-note file {}", rep.links_to_files);
                println!("    relative to source {}", rep.links_relative);
                println!("  dead                 {}", rep.links_dead);
                println!("  dead, note-shaped    {}", rep.dead_note_shaped);
                println!("  distinct dead targets {}", rep.distinct_dead_targets);
                println!("path-form links        {}", rep.path_form_total);
                println!("  of those, dead       {}", rep.path_form_dead);
                println!("ambiguous basenames    {}", rep.ambiguous_basenames.len());
                for a in &rep.ambiguous_basenames {
                    println!("  {} -> {}", a.basename, a.candidates.join(" | "));
                }
                for d in &rep.dead {
                    println!("  DEAD {} -> {}", d.from, d.target);
                }
            }
        }

        "lint" => {
            if let Some(key) = flag(&args, "--carrying") {
                let hits = lint::carrying(&notes, &key);
                if json {
                    let ids: Vec<&str> = hits.iter().map(|n| n.id.as_str()).collect();
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "key": key, "count": ids.len(), "notes": ids,
                        }))
                        .unwrap_or_default()
                    );
                } else {
                    println!("{} notes carry a `{key}:` key", hits.len());
                    for n in hits {
                        println!("  {}", n.id);
                    }
                }
                return;
            }

            let rep = lint::report(&notes, problems);
            if json {
                println!("{}", serde_json::to_string_pretty(&rep).unwrap_or_default());
            } else {
                println!(
                    "notes {} ({} without frontmatter)\n",
                    rep.notes, rep.no_frontmatter
                );
                for f in &rep.folders {
                    println!(
                        "{}  ({} notes, {} bare)",
                        f.folder, f.notes, f.no_frontmatter
                    );
                    for c in &f.fields {
                        if c.present == 0 {
                            continue;
                        }
                        let note = if c.filled == c.present {
                            String::new()
                        } else {
                            format!("  ({} empty)", c.present - c.filled)
                        };
                        println!("    {:<12} {:>5}{}", c.field, c.filled, note);
                    }
                    println!();
                }
                for d in &rep.dialects {
                    if d.forms.len() < 2 {
                        continue;
                    }
                    println!("dialect drift on `{}`:", d.field);
                    for (form, n) in &d.forms {
                        println!("    {:>6}  {}", n, form);
                    }
                    println!();
                }
                if !rep.problems.is_empty() {
                    println!("problems:");
                    for p in &rep.problems {
                        println!("  {p}");
                    }
                }
            }
        }

        // D2: the three People keys that have no producer, computed from the Journal so the
        // claim that they are derivable is measured rather than repeated. Read-only, and the
        // reason is D3 — machine-owned frontmatter has no protection, so a writer could not
        // tell its own value from a human's correction.
        "people" => {
            let rep = people::report(&notes);
            if json {
                println!("{}", serde_json::to_string_pretty(&rep).unwrap_or_default());
            } else {
                println!("people notes           {}", rep.people);
                println!("  named in the Journal {}", rep.with_mentions);
                println!("  carrying a key       {}", rep.carrying_any);
                println!("  stored value differs {}", rep.disagreeing);
                println!();
                println!("mentions  last        first       name");
                for facts in &rep.facts {
                    println!(
                        "{:>8}  {:<10}  {:<10}  {}{}",
                        facts.mention_count,
                        facts.last_contact.as_deref().unwrap_or("-"),
                        facts.met_at.as_deref().unwrap_or("-"),
                        facts.name,
                        if facts.disagrees.is_empty() {
                            String::new()
                        } else {
                            format!("   [stored differs: {}]", facts.disagrees.join(", "))
                        }
                    );
                }
            }
        }

        // Q106: which of the Journal's six health keys can have a producer, and the one that
        // does. The verdict per key is `journal::producer_for`, not this printer, so the CLI,
        // the server and the README cannot end up holding three opinions about it.
        "journal" => {
            let rep = journal::report(&notes);
            if json {
                println!("{}", serde_json::to_string_pretty(&rep).unwrap_or_default());
            } else {
                println!("daily notes            {}", rep.days);
                println!("  not a date           {}", rep.not_a_day);
                println!("people register        {}", rep.people_notes);
                println!();
                println!("key             present  asserted  comment-only  producer");
                for key in &rep.keys {
                    println!(
                        "{:<14}  {:>7}  {:>8}  {:>12}  {}",
                        key.key,
                        key.present,
                        key.asserted,
                        key.comment_only,
                        match key.producer {
                            journal::Producer::JournalLinks => "journal links",
                            journal::Producer::SelfReport => "none: self-report",
                            journal::Producer::HealthStoreUnreachable => "none: no health store",
                            journal::Producer::Unmeasured => "none: unmeasured",
                        }
                    );
                }
                println!();
                println!("social, produced from Journal person links");
                println!(
                    "  days naming a person {}",
                    rep.social.days_with_person_link
                );
                println!("  distinct people      {}", rep.social.people_named);
                println!("  stored social: true  {}", rep.social.stated_true);
                println!("    links agree        {}", rep.social.agrees);
                println!("    links differ       {}", rep.social.disagrees);
                println!(
                    "  template false, links name somebody {}",
                    rep.social.unfilled_with_evidence
                );
                if !rep.unrendered.is_empty() {
                    println!();
                    println!(
                        "{} frontmatter values are still template expressions:",
                        rep.unrendered.len()
                    );
                    for item in &rep.unrendered {
                        println!("  {}  {}: {}", item.id, item.key, item.value);
                    }
                }
            }
        }

        // D5, the half a CLI can answer. Base rendering is not checkable from here and this
        // does not pretend it is; what it checks is everything a Base states about the vault
        // before rendering starts — the folders it queries and the keys it draws as columns.
        "bases" => {
            let files = root.files_recursive().unwrap_or_else(|e| die(e));
            let mut found: Vec<(String, String)> = Vec::new();
            for path in files {
                if path.extension().and_then(|e| e.to_str()) != Some("base") {
                    continue;
                }
                let Some(id) = root.relative_id(&path) else {
                    continue;
                };
                match std::fs::read_to_string(&path) {
                    Ok(text) => found.push((id, text)),
                    // Named rather than skipped: a Base this verb cannot read is the one case
                    // where "0 unresolved" would be a lie.
                    Err(e) => die(format!("{id}: unreadable: {e}")),
                }
            }
            // The disk, not the note index: a folder that is there and holds no note is a
            // different repair from one that is not there at all, and only this tells them
            // apart.
            let dirs = bases::folders_on_disk(root.path()).unwrap_or_else(|e| die(e));
            let rep = bases::report(&notes, &found, &dirs);
            if json {
                println!("{}", serde_json::to_string_pretty(&rep).unwrap_or_default());
            } else {
                println!("bases                  {}", rep.bases);
                println!("folder references      {}", rep.folder_refs);
                println!("  unresolved           {}", rep.unresolved_folders);
                println!("declared columns empty {}", rep.empty_fields);
                println!();
                for base in &rep.items {
                    let scope = match base.scope {
                        Some(n) => format!("{n} notes"),
                        None => "no folder scope".to_string(),
                    };
                    println!("{}  ({}, {} views)", base.id, scope, base.views);
                    for f in &base.folders {
                        let mark = if f.excluded {
                            "excludes"
                        } else if f.resolved() {
                            "ok      "
                        } else {
                            "MISSING "
                        };
                        println!("  {mark} {:<34} {:>5}", f.folder, f.notes);
                        // The verdict first, then the evidence it was read from. A reader who
                        // stops at the first line still knows which of the six repairs this is.
                        if let Some(missing) = &f.missing {
                            println!("           {}", missing.explain());
                        }
                        for c in &f.candidates {
                            println!(
                                "           candidate: {} ({} notes, {}/{} columns)",
                                c.folder, c.notes, c.columns_carried, c.columns_total
                            );
                        }
                    }
                    for field in base.fields.iter().filter(|f| f.carried == 0) {
                        println!("  EMPTY    column `{}` — 0 notes carry it", field.field);
                    }
                    println!();
                }
            }
            // `--strict` is the gate half. Without it this verb answers 0 for a vault where
            // every Base is broken and 0 for one where none is, which is an instrument that
            // cannot be wrong.
            if has(&args, "--strict") && rep.unresolved_folders > 0 {
                die(format!(
                    "{} folder reference(s) name a folder that holds no note",
                    rep.unresolved_folders
                ));
            }
        }

        // Rung 0 of the redaction ladder. See src/names.rs for why the registry,
        // not the matching, is the work.
        "names" => {
            let folder =
                flag(&args, "--folder").unwrap_or_else(|| names::DEFAULT_FOLDER.to_string());
            let reg = names::build(&notes, &folder);
            if json {
                println!("{}", serde_json::to_string_pretty(&reg).unwrap_or_default());
            } else {
                println!("{} people -> {} tokens", reg.people, reg.tokens.len());
                if !reg.withheld.is_empty() {
                    // Withheld, never dropped: a name held back by the stoplist is a
                    // decision the operator should be able to see and overrule.
                    println!("\nwithheld as ambiguous ({}):", reg.withheld.len());
                    for w in &reg.withheld {
                        println!("  {w}");
                    }
                }
                if !reg.refused.is_empty() {
                    println!("\nrefused ({}):", reg.refused.len());
                    for r in &reg.refused {
                        println!("  {} — {}", r.note, r.reason);
                    }
                }
            }
        }

        // Q9a, the reporting half. The rule itself is in
        // content_item::DataClass::classify_vault_note; this prints what it
        // decided so the folder defaults can be checked against a real vault
        // rather than believed.
        "class" => {
            let rep = class::report(&notes);
            let only = flag(&args, "--only");
            if json {
                println!("{}", serde_json::to_string_pretty(&rep).unwrap_or_default());
            } else {
                for c in content_item::DATA_CLASSES {
                    let n = rep.counts.get(c).copied().unwrap_or(0);
                    println!(
                        "{:<4} {:<8} {:>5}",
                        c,
                        content_item::DataClass::new(c, "", "", "").label,
                        n
                    );
                }
                println!();
                println!("frontmatter overrides  {}", rep.overridden.len());
                for o in &rep.overridden {
                    println!("  {} -> {}", o.id, o.class);
                }
                // Refusals first among the things worth acting on: each one is a
                // note whose author believes it carries a class and which no
                // human has actually classified.
                if !rep.refused.is_empty() {
                    println!("\nrefused declarations ({}):", rep.refused.len());
                    for r in &rep.refused {
                        println!("  {} — {}", r.id, r.rationale);
                    }
                }
                if let Some(class) = &only {
                    let hits: Vec<&class::Classified> =
                        rep.notes.iter().filter(|n| &n.class == class).collect();
                    println!("\n{} note(s) at {class}:", hits.len());
                    for h in hits {
                        println!("  {} — {}", h.id, h.rationale);
                    }
                } else if has(&args, "--list") {
                    println!();
                    for n in &rep.notes {
                        println!("{:<4} {}", n.class, n.id);
                    }
                }
            }
        }

        other => die(format!("unknown command `{other}` (try --help)")),
    }
}
