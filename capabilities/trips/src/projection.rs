//! Every plan as one markdown file, so the rows are not the only copy.
//!
//! PRD Q47 (2026-08-27) measured that 512 of 483,116 rows in the store are
//! irreplaceable, and named this capability's 21 `trips_plan_items` among them: an
//! option set records fares that cannot be queried back later at yesterday's price,
//! so an unrecorded one is gone rather than merely unwritten. The rule it ruled is
//! that a capability holding only-copy rows projects them to files.
//!
//! `capabilities/trips/README.md` had already specified the shape a month earlier
//! and nothing implemented it: one file per plan in a configured vault folder,
//! carrying the trip id, the schema and the revision in frontmatter. Q31 (2026-08-23)
//! then supplied the folder — `Resources/Sjel/` — and Q49 (2026-08-27) supplied the
//! mechanism, which is why the writing happens in `markdown_root::projection` and only
//! the *shape* is here.
//!
//! ## What a projected file is for
//!
//! Reconstruction, not reading. Q46 is explicit that B14's projections are safety
//! copies rather than reading surfaces, so the test every rendering decision here
//! answers is "could the plan be rebuilt from this file alone", not "does it look
//! nice". That is why each item's payload is written out verbatim as JSON: it is the
//! only copy of a fare, a booking reference or a stay's coordinates, and prose about
//! it would not restore it.
//!
//! ## What is deliberately not here
//!
//! No import. This is one-way, like every other machine→vault write in the repo. The
//! two-way slice the README sketches comes after this shape has been used, and it
//! would need a conflict rule that a safety copy does not have.

use markdown_root::{MarkdownRoot, ProjectionOutcome, RegionSpec, RootError};

use crate::store::{PlaceRef, PlanDetails, PlanItem, Retrospective, TripPlan, TripStage};

/// The projection marker owner. Stable forever: changing it makes every file already
/// in the vault look foreign, and `write_projection` then refuses all of them.
pub const OWNER: &str = "trips";

/// Bumped when the rendered shape changes, so a later generator can recognise output
/// it no longer knows how to produce.
///
/// 1 -> 2 on 2026-09-05: the `## Retrospective` section joined the rendered
/// shape.
/// 2 -> 3 on 2026-09-24: a stage's `branch_note` renders under its stage line.
pub const VERSION: u32 = 3;

/// Q31's home for a projection, plus one folder for this capability. Vault-relative
/// and not configurable: a second declaration of where machine output goes is how two
/// hosts end up writing to two different folders and neither notices.
// `Resources/Axon/Trips` until 2026-10-08, when the vault folder took the product name.
pub const DIR: &str = "Resources/Sjel/Trips";

/// One plan's file name and body, ready to write.
pub struct Projection {
    /// Vault-relative, `Resources/Sjel/Trips/<name>.md`.
    pub path: String,
    pub body: String,
}

/// Render every plan, and name the files that should exist afterwards.
///
/// Takes the whole set rather than one plan because the file name is derived from the
/// title, and two plans may share one. A collision is resolved by giving *both* files
/// an id suffix, which is decidable only with the set in hand — and doing it that way
/// keeps the name a function of the data instead of a function of iteration order.
pub fn render_all(plans: &[PlanDetails]) -> Vec<Projection> {
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for details in plans {
        *counts.entry(file_stem(&details.plan)).or_default() += 1;
    }

    plans
        .iter()
        .map(|details| {
            let stem = file_stem(&details.plan);
            let unique = if counts.get(&stem).copied().unwrap_or(0) > 1 {
                format!("{stem} ({})", id_suffix(&details.plan.id))
            } else {
                stem
            };
            Projection {
                path: format!("{DIR}/{unique}.md"),
                body: render(details),
            }
        })
        .collect()
}

/// Write every plan, and delete the projections of plans that no longer exist.
///
/// The sweep is not housekeeping. A plan renamed or deleted leaves a file that still
/// claims to be the safety copy of a live plan, and a stale safety copy is worse than
/// a missing one: it is the copy somebody would restore from. Only files this module
/// wrote are considered, so a human note that landed in the folder is left alone.
pub fn export_all(root: &MarkdownRoot, plans: &[PlanDetails]) -> Result<Report, RootError> {
    let spec = RegionSpec::new(OWNER, VERSION);
    let rendered = render_all(plans);
    let mut report = Report::default();

    for projection in &rendered {
        match root.write_projection(&projection.path, &spec, &projection.body)? {
            ProjectionOutcome::Created => report.created += 1,
            ProjectionOutcome::Updated => report.updated += 1,
            ProjectionOutcome::Unchanged => report.unchanged += 1,
            ProjectionOutcome::NotOurs => report.refused.push(projection.path.clone()),
        }
    }

    // A missing folder is zero files, not an error: the first run has nothing to sweep
    // and `write_projection` above has just created the directory anyway.
    let existing = match root.markdown_files(&format!("{DIR}/*.md")) {
        Ok(files) => files,
        Err(RootError::Unreadable { .. }) => Vec::new(),
        Err(e) => return Err(e),
    };
    let wanted: std::collections::HashSet<&str> =
        rendered.iter().map(|p| p.path.as_str()).collect();
    for file in existing {
        let Some(id) = root.relative_id(&file) else {
            continue;
        };
        if wanted.contains(id.as_str()) {
            continue;
        }
        if root.remove_projection(&id, &spec)? {
            report.removed.push(id);
        }
    }

    Ok(report)
}

/// What one export run did. Counts for the ordinary outcomes, paths for the two a
/// human has to look at.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub created: usize,
    pub updated: usize,
    pub unchanged: usize,
    /// Paths where a file exists that this module did not write. Q31's promotion
    /// signal: a human wrote about the trip, so their note keeps the path.
    pub refused: Vec<String>,
    /// Projections whose plan is gone or renamed.
    pub removed: Vec<String>,
}

/// One plan as a whole document, frontmatter first.
pub fn render(details: &PlanDetails) -> String {
    let plan = &details.plan;
    let mut out = String::new();

    // Frontmatter, as `capabilities/trips/README.md` specified it: the trip id, the
    // schema and the revision. `axon_revision` is the plan's `updated_at`, which is
    // the revision the store already uses as its optimistic-concurrency token
    // (`expected_updated_at`) — a second notion of "which revision is this" would be
    // one nothing else could check against.
    out.push_str("---\n");
    push_field(&mut out, "axon_trip_id", &plan.id);
    push_field(&mut out, "axon_schema", "schemas/trip-plan.schema.json");
    push_field(&mut out, "axon_projection_version", &VERSION.to_string());
    push_field(&mut out, "axon_revision", &plan.updated_at);
    push_field(&mut out, "title", &plan.title);
    push_field(&mut out, "date_start", &plan.date_start);
    push_field(&mut out, "date_end", &plan.date_end);
    push_field(&mut out, "status", &plan.status);
    if let Some(source) = &plan.source {
        push_field(&mut out, "source_kind", &source.kind);
        push_field(&mut out, "source_reference", &source.reference);
    }
    out.push_str("---\n\n");

    out.push_str(&format!("# {}\n\n", plan.title));
    out.push_str(&format!(
        "{} → {}\n\n",
        place(&plan.origin),
        if plan.destinations.is_empty() {
            "(no destination)".to_string()
        } else {
            plan.destinations
                .iter()
                .map(place)
                .collect::<Vec<_>>()
                .join(", ")
        }
    ));
    out.push_str(&format!(
        "- Dates: {} → {}\n",
        plan.date_start, plan.date_end
    ));
    out.push_str(&format!("- Status: {}\n", plan.status));
    if !plan.travelers.is_empty() {
        out.push_str(&format!("- Travelers: {}\n", plan.travelers.join(", ")));
    }
    if !plan.transport_modes.is_empty() {
        out.push_str(&format!("- Modes: {}\n", modes(&plan.transport_modes)));
    }
    if let Some(cents) = plan.budget_cents {
        out.push_str(&format!(
            "- Budget: {} {}\n",
            minor_units(cents),
            plan.currency.as_deref().unwrap_or("EUR")
        ));
    }
    if !plan.interests.trim().is_empty() {
        out.push_str(&format!("- Interests: {}\n", plan.interests.trim()));
    }
    out.push('\n');

    if plan.stages.is_empty() {
        out.push_str("## Stages\n\nNone recorded.\n\n");
    } else {
        out.push_str("## Stages\n\n");
        for stage in &plan.stages {
            out.push_str(&stage_line(stage));
        }
        out.push('\n');
    }

    // The change note is a human sentence that exists nowhere else, so Principle 8
    // ("a database is a rebuildable index, never the only copy") puts it in the
    // safety copy verbatim. This section is that copy and NOT the retrospective's
    // vault presence: the PRD routes that to the Journal, "when it earns one",
    // and this code writes no such note.
    if let Some(retrospective) = &details.retrospective {
        out.push_str(&retrospective_block(retrospective, plan));
    }

    // The reason this file exists. Every item's payload verbatim, because it is the
    // only copy: an unchosen fare cannot be re-queried at yesterday's price, and a
    // booking reference summarised in prose does not restore a booking.
    out.push_str(&format!("## Items ({})\n\n", details.items.len()));
    if details.items.is_empty() {
        out.push_str("None recorded.\n");
    } else {
        for item in &details.items {
            out.push_str(&item_block(item));
        }
    }

    out
}

fn retrospective_block(retrospective: &Retrospective, plan: &TripPlan) -> String {
    let mut out = String::from("## Retrospective\n\n");
    if let Some(cents) = retrospective.cost_cents {
        out.push_str(&format!(
            "- Cost: {} {}\n",
            minor_units(cents),
            // Denominated on the plan, echoed here. The store refuses a cost on
            // a plan with no currency, so this fallback is unreachable in
            // practice and exists so a hand-edited row cannot panic an export.
            retrospective
                .currency
                .as_deref()
                .or(plan.currency.as_deref())
                .unwrap_or("EUR")
        ));
    }
    out.push_str(&format!("- Again: {}\n", retrospective.again));
    if !retrospective.change_note.trim().is_empty() {
        out.push_str(&format!(
            "- Would change: {}\n",
            retrospective.change_note.trim()
        ));
    }
    out.push('\n');
    out
}

fn stage_line(stage: &TripStage) -> String {
    let mut line = format!(
        "{}. {} → {}",
        stage.sequence + 1,
        place(&stage.origin),
        place(&stage.destination)
    );
    if let Some(date) = &stage.date {
        line.push_str(&format!(" · {date}"));
    }
    line.push_str(&format!(" · {}", enum_word(&stage.status)));
    if !stage.transport_modes.is_empty() {
        line.push_str(&format!(" · {}", modes(&stage.transport_modes)));
    }
    if let Some(selected) = &stage.selected_option_id {
        line.push_str(&format!(" · selected `{selected}`"));
    }
    line.push('\n');
    // The branch note is a human sentence that exists nowhere else, so it goes
    // into the safety copy verbatim, indented so it stays inside the list item.
    if let Some(note) = stage.branch_note.as_deref().map(str::trim) {
        if !note.is_empty() {
            line.push_str(&format!(
                "   - Branch note: {}\n",
                note.replace('\n', "\n     ")
            ));
        }
    }
    line
}

fn item_block(item: &PlanItem) -> String {
    let mut out = format!("### {} — {}\n\n", item.item_type, item.title);
    out.push_str(&format!("- id: `{}`\n", item.id));
    out.push_str(&format!("- external_id: `{}`\n", item.external_id));
    out.push_str(&format!(
        "- day: {}\n",
        item.day.as_deref().unwrap_or("unscheduled")
    ));
    out.push_str(&format!("- created_at: {}\n\n", item.created_at));
    // `to_string_pretty` cannot fail for a `Value` that came out of the store, but a
    // fallback that dropped the payload would be the one failure this file exists to
    // prevent — so the compact form is the fallback, never nothing.
    let payload =
        serde_json::to_string_pretty(&item.payload).unwrap_or_else(|_| item.payload.to_string());
    out.push_str("```json\n");
    out.push_str(&payload);
    out.push_str("\n```\n\n");
    out
}

fn place(place: &PlaceRef) -> String {
    match (place.latitude, place.longitude) {
        (Some(lat), Some(lon)) => format!("{} ({lat:.4}, {lon:.4})", place.name),
        _ => place.name.clone(),
    }
}

fn modes(modes: &[crate::store::TransportMode]) -> String {
    modes.iter().map(enum_word).collect::<Vec<_>>().join(", ")
}

/// An enum as the word the API and `schemas/trip-plan.schema.json` already use.
///
/// Through serde, not `{:?}` lowercased. The first version did the latter and wrote
/// `optionselected` for `StageStatus::OptionSelected`, which is a value the contract
/// does not contain — a file meant for reconstruction has to carry the vocabulary the
/// thing being reconstructed accepts.
fn enum_word<T: serde::Serialize>(value: &T) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(word)) => word,
        other => format!("{other:?}"),
    }
}

fn minor_units(cents: i64) -> String {
    format!("{}.{:02}", cents / 100, (cents % 100).abs())
}

/// A frontmatter scalar, always quoted.
///
/// Quoted unconditionally rather than when it looks necessary: live titles already
/// carry `:` and `—` (`St. Gallen — StartHack (Mar 17–21, 2026)`), and a rule that
/// decides per value is a rule that will one day decide wrong on a value nobody
/// anticipated.
fn push_field(out: &mut String, key: &str, value: &str) {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    out.push_str(&format!("{key}: \"{escaped}\"\n"));
}

/// The plan's title as a file name. `markdown_root::projection::file_stem` owns the
/// rules; the fallback for a plan with no usable title is its id tail, which is not
/// pretty and is not ambiguous.
fn file_stem(plan: &TripPlan) -> String {
    markdown_root::projection::file_stem(&plan.title, &id_suffix(&plan.id))
}

/// The tail of a plan id — the part that actually differs. Ids are
/// `trip:plan:18c68af3fd0c77580006`, so the first eight characters of the whole
/// string are the same for every plan ever created.
fn id_suffix(id: &str) -> String {
    let tail = id.rsplit(':').next().unwrap_or(id);
    tail.chars()
        .rev()
        .take(8)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{PlaceKind, PlanSource, StageStatus, TransportMode};
    use serde_json::json;

    fn place_ref(name: &str) -> PlaceRef {
        PlaceRef {
            id: format!("place:{name}"),
            name: name.to_string(),
            kind: PlaceKind::City,
            address: None,
            latitude: None,
            longitude: None,
        }
    }

    fn plan(id: &str, title: &str) -> TripPlan {
        TripPlan {
            id: id.to_string(),
            title: title.to_string(),
            origin: place_ref("Bonn"),
            destinations: vec![place_ref("Berlin")],
            date_start: "2026-10-07".into(),
            date_end: "2026-10-13".into(),
            interests: String::new(),
            status: "saved".into(),
            travelers: vec!["me".into()],
            transport_modes: vec![TransportMode::Train],
            stages: vec![TripStage {
                id: "stage-1".into(),
                sequence: 0,
                origin: place_ref("Bonn"),
                destination: place_ref("Berlin"),
                date: Some("2026-10-07".into()),
                transport_modes: vec![TransportMode::Train],
                travelers: vec![],
                status: StageStatus::Planning,
                selected_option_id: None,
                branch_note: None,
            }],
            cover_image_url: None,
            source: Some(PlanSource {
                kind: "obsidian".into(),
                reference: "Atlas/Events/Berlin.md".into(),
            }),
            created_at: "1786907000".into(),
            updated_at: "1786907994".into(),
            budget_cents: Some(45_000),
            currency: Some("EUR".into()),
        }
    }

    fn details(id: &str, title: &str, items: Vec<PlanItem>) -> PlanDetails {
        PlanDetails {
            plan: plan(id, title),
            items,
            retrospective: None,
        }
    }

    fn item(id: &str, payload: serde_json::Value) -> PlanItem {
        PlanItem {
            id: id.to_string(),
            plan_id: "trip:plan:1".into(),
            item_type: "option_set".into(),
            day: None,
            external_id: "sparpreis:8000044-8011160".into(),
            title: "Sparpreis watch".into(),
            payload,
            created_at: "1786907100".into(),
        }
    }

    #[test]
    fn an_items_payload_survives_verbatim_because_it_is_the_only_copy() {
        let payload = json!({
            "query": {"from": "8000044", "to": "8011160"},
            "options": [{"price_cents": 3999, "train": "ICE 950"}],
        });
        let rendered = render(&details(
            "trip:plan:1",
            "Berlin",
            vec![item("i1", payload.clone())],
        ));
        let fence = rendered
            .split("```json\n")
            .nth(1)
            .and_then(|s| s.split("\n```").next())
            .expect("a json fence");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(fence).unwrap(),
            payload,
            "the fence must parse back to the row's payload, or it is not a safety copy"
        );
    }

    /// Principle 8 and this file's own version contract in one assertion. The
    /// change note is a human sentence that exists nowhere else, so it goes into
    /// the safety copy character for character — and the rendered shape changed,
    /// so VERSION moved with it.
    #[test]
    fn the_projection_carries_the_retrospective_text_verbatim_at_version_3() {
        let note = "Booked the 06:12 rather than the 09:40 — worth the early start.";
        let mut with_retrospective = details("trip:plan:1", "Berlin", vec![]);
        with_retrospective.retrospective = Some(Retrospective {
            plan_id: "trip:plan:1".into(),
            cost_cents: Some(42_150),
            currency: Some("EUR".into()),
            again: "yes".into(),
            change_note: note.into(),
            filled_at: "1786907100".into(),
        });
        let rendered = render(&with_retrospective);
        assert!(rendered.contains("## Retrospective"));
        assert!(
            rendered.contains(note),
            "the change note must survive character for character: {rendered}"
        );
        assert!(rendered.contains("- Cost: 421.50 EUR"));
        assert!(rendered.contains("- Again: yes"));
        let fields = markdown_root::frontmatter(&rendered).unwrap();
        assert_eq!(
            fields.get("axon_projection_version").map(String::as_str),
            Some("3")
        );

        // A plan with no retrospective renders no section at all.
        let bare = render(&details("trip:plan:2", "Berlin", vec![]));
        assert!(!bare.contains("## Retrospective"));
    }

    /// A stage's selected option and its branch note are both only-copy choices,
    /// so both reach the safety copy. The branch note was missing until 2026-09-24.
    #[test]
    fn a_stage_carries_its_selected_option_and_branch_note() {
        let note = "Split here if the night train is sold out.";
        let mut chosen = details("trip:plan:1", "Berlin", vec![]);
        chosen.plan.stages[0].selected_option_id = Some("option:ice-950".into());
        chosen.plan.stages[0].branch_note = Some(note.into());
        let rendered = render(&chosen);
        assert!(rendered.contains("selected `option:ice-950`"), "{rendered}");
        assert!(
            rendered.contains(&format!("   - Branch note: {note}\n")),
            "{rendered}"
        );

        // No note, no line.
        let bare = render(&details("trip:plan:2", "Berlin", vec![]));
        assert!(!bare.contains("Branch note"));
    }

    #[test]
    fn the_frontmatter_carries_id_schema_and_revision() {
        let rendered = render(&details("trip:plan:18c72d1e", "Berlin", vec![]));
        let fields = markdown_root::frontmatter(&rendered).unwrap();
        assert_eq!(fields.get("axon_trip_id").unwrap(), "trip:plan:18c72d1e");
        assert_eq!(fields.get("axon_revision").unwrap(), "1786907994");
        assert_eq!(
            fields.get("axon_schema").unwrap(),
            "schemas/trip-plan.schema.json"
        );
    }

    #[test]
    fn a_title_with_a_colon_stays_readable_in_the_frontmatter_and_the_file_name() {
        let details = details(
            "trip:plan:18c68af3fd0c77580006",
            "St. Gallen: 17–21",
            vec![],
        );
        let rendered = render(&details);
        assert_eq!(
            markdown_root::frontmatter(&rendered)
                .unwrap()
                .get("title")
                .unwrap(),
            "St. Gallen: 17–21"
        );
        assert_eq!(
            render_all(std::slice::from_ref(&details))[0].path,
            "Resources/Sjel/Trips/St. Gallen- 17–21.md"
        );
    }

    #[test]
    fn two_plans_with_one_title_both_get_an_id_suffix() {
        let both = vec![
            details("trip:plan:18c68af3fd0c77580006", "Berlin", vec![]),
            details("trip:plan:18c72d1ebb4aac680000", "Berlin", vec![]),
        ];
        let paths: Vec<String> = render_all(&both).into_iter().map(|p| p.path).collect();
        assert_eq!(
            paths,
            vec![
                "Resources/Sjel/Trips/Berlin (77580006).md",
                "Resources/Sjel/Trips/Berlin (ac680000).md",
            ],
            "both sides of a collision are renamed, so neither name depends on order"
        );
    }

    fn temp_root() -> (std::path::PathBuf, MarkdownRoot) {
        let dir = std::env::temp_dir().join(format!(
            "axon-trips-projection-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let root = MarkdownRoot::declare(&dir).unwrap();
        (dir, root)
    }

    /// The whole contract, against a real directory: a plan becomes a file, a second
    /// run writes nothing, and a plan that goes away takes its file with it.
    #[test]
    fn an_export_creates_settles_and_sweeps() {
        let (dir, root) = temp_root();
        let payload = json!({"options": [{"price_cents": 3999}]});
        let plans = vec![
            details("trip:plan:aaaa1111", "Berlin", vec![item("i1", payload)]),
            details("trip:plan:bbbb2222", "Salzburg", vec![]),
        ];

        let first = export_all(&root, &plans).unwrap();
        assert_eq!((first.created, first.updated, first.unchanged), (2, 0, 0));
        let berlin = dir.join("Resources/Sjel/Trips/Berlin.md");
        assert!(berlin.exists());
        assert!(
            std::fs::read_to_string(&berlin).unwrap().contains("3999"),
            "the payload is why the file exists"
        );

        let second = export_all(&root, &plans).unwrap();
        assert_eq!(
            (second.created, second.updated, second.unchanged),
            (0, 0, 2),
            "an unchanged export must not touch a file, or every run is a vault commit"
        );

        let third = export_all(&root, &plans[..1]).unwrap();
        assert_eq!(
            third.removed,
            vec!["Resources/Sjel/Trips/Salzburg.md"],
            "a stale safety copy is the one somebody would restore from"
        );
        assert!(!dir.join("Resources/Sjel/Trips/Salzburg.md").exists());
        assert!(berlin.exists());

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_sweep_leaves_a_human_note_in_the_folder_alone() {
        let (dir, root) = temp_root();
        export_all(&root, &[details("trip:plan:aaaa1111", "Berlin", vec![])]).unwrap();
        let human = dir.join("Resources/Sjel/Trips/Why I go to Berlin.md");
        std::fs::write(&human, "# Because\n").unwrap();

        let report = export_all(&root, &[details("trip:plan:aaaa1111", "Berlin", vec![])]).unwrap();
        assert!(report.removed.is_empty());
        assert_eq!(std::fs::read_to_string(&human).unwrap(), "# Because\n");

        std::fs::remove_dir_all(dir).unwrap();
    }

    /// `{:?}` lowercased wrote `optionselected`, which is not a value the contract
    /// contains. A file for reconstruction carries the vocabulary that reconstructs.
    #[test]
    fn an_enum_renders_as_the_word_the_schema_uses() {
        let mut plan = details("trip:plan:1", "Berlin", vec![]);
        plan.plan.stages[0].status = StageStatus::OptionSelected;
        let rendered = render(&plan);
        assert!(rendered.contains("option_selected"), "{rendered}");
        assert!(!rendered.contains("optionselected"));
    }

    #[test]
    fn a_titleless_plan_still_gets_a_file() {
        let details = details("trip:plan:18c72d1ebb4aac680000", "  ...  ", vec![]);
        assert_eq!(
            render_all(&[details])[0].path,
            "Resources/Sjel/Trips/ac680000.md"
        );
    }
}
