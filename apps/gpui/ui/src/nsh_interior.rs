//! Room scan archive, revision review, placements and layout planning on the phone.
use crate::nsh_ui::{action, caption, field, heading, snapshot};
use nsh_core::prelude::*;
use sjel_gpui_core::{
    interior::{
        planning::{add_planning_analysis, planning_candidates, PlanningCandidate, PlanningKind},
        *,
    },
    inventory::{InteriorLayoutSummary, InventoryRow},
    shell,
};
use std::collections::BTreeSet;
use std::sync::{Mutex, OnceLock};
use tokio::sync::watch;

/// Native capture and USDZ previews remain owned by the UIKit host.
pub enum InteriorEvent {
    Capture,
    Sync,
    Import,
    Preview(String),
    CachePreview {
        id: String,
        draft: String,
        bytes: Vec<u8>,
    },
}
type Host = Box<dyn Fn(InteriorEvent) + Send>;
#[derive(Clone, Default)]
struct Snapshot {
    local: Vec<RoomPlanLocalRecord>,
    live: Option<RoomPlanLiveUpdate>,
    revisions: RoomPlanRevisionSet,
    model: Option<RoomPlanModel>,
    spaces: RoomPlanSpaceSet,
    layouts: Vec<InteriorLayoutSummary>,
    inventory: Vec<InventoryRow>,
    placements: Vec<RoomPlanPlacement>,
    selected: Option<String>,
    layout: Option<RoomPlanSavedLayoutPreview>,
    comparison: Option<RoomPlanReconciliationResponse>,
    proposal: Option<RoomPlanPlacementProposalResponse>,
    candidates: Vec<PlanningCandidate>,
    loading: bool,
    busy: bool,
    error: Option<String>,
    status: String,
}
struct State {
    snap: Snapshot,
    source: Option<RoomPlanSource>,
    generation: u64,
    read_generation: u64,
}
struct Feed {
    state: Mutex<State>,
    tx: watch::Sender<Snapshot>,
    host: Mutex<Option<Host>>,
}
fn feed() -> &'static Feed {
    static FEED: OnceLock<Feed> = OnceLock::new();
    FEED.get_or_init(|| {
        let snap = Snapshot::default();
        let (tx, _) = watch::channel(snap.clone());
        Feed {
            state: Mutex::new(State {
                snap,
                source: None,
                generation: 0,
                read_generation: 0,
            }),
            tx,
            host: Mutex::new(None),
        }
    })
}
fn state() -> std::sync::MutexGuard<'static, State> {
    feed().state.lock().unwrap_or_else(|e| e.into_inner())
}
fn publish(s: &State) {
    feed().tx.send_replace(s.snap.clone());
}
fn emit(event: InteriorEvent) {
    if let Some(host) = feed()
        .host
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
    {
        host(event);
    }
}
/// Registers native capture, upload and preview handling.
pub fn set_host(host: impl Fn(InteriorEvent) + Send + 'static) {
    *feed().host.lock().unwrap_or_else(|e| e.into_inner()) = Some(Box::new(host));
}
/// Supplies the paired capability source without moving local scan files.
pub fn connect(source: RoomPlanSource) {
    {
        let mut s = state();
        s.source = Some(source);
        s.generation += 1;
        s.snap.busy = false;
        s.snap.proposal = None;
        s.snap.comparison = None;
    }
    load();
}
/// Replaces the archive summary read by the native host.
pub fn set_local(local: Vec<RoomPlanLocalRecord>) {
    let mut s = state();
    s.snap.local = local;
    publish(&s);
}
/// Displays capture guidance and semantic updates while RoomPlan is running.
pub fn set_live(live: RoomPlanLiveUpdate) {
    let mut s = state();
    s.snap.live = Some(live);
    publish(&s);
}
/// Displays the host's capture or upload outcome.
pub fn set_status(status: String) {
    let mut s = state();
    s.snap.status = status;
    publish(&s);
}
/// Refreshes the paired revision history, room model, saved layouts and inventory.
pub fn load() {
    let (source, generation) = {
        let mut s = state();
        let Some(source) = s.source.clone() else {
            return;
        };
        s.read_generation += 1;
        s.snap.loading = true;
        s.snap.error = None;
        publish(&s);
        (source, s.read_generation)
    };
    shell::spawn(async move {
        let (revisions, model, spaces, layouts, inventory) = futures::join!(
            source.revisions(),
            source.model(),
            source.spaces(),
            source.layouts(),
            source.inventory_rows()
        );
        let placements = if let Ok(model) = &model {
            source.placements(&model.flat).await
        } else {
            Ok(Vec::new())
        };
        let mut s = state();
        if s.read_generation != generation {
            return;
        }
        s.snap.loading = false;
        macro_rules! apply {
            ($result:ident, $field:ident) => {
                match $result {
                    Ok(value) => s.snap.$field = value,
                    Err(error) => s.snap.error = Some(error.to_string()),
                }
            };
        }
        apply!(revisions, revisions);
        apply!(spaces, spaces);
        apply!(layouts, layouts);
        apply!(inventory, inventory);
        apply!(placements, placements);
        match model {
            Ok(model) => s.snap.model = Some(model),
            Err(error) => s.snap.error = Some(error.to_string()),
        }
        publish(&s);
    });
}

enum Action {
    Review(String, RoomPlanDecision),
    Compare(String, String),
    CreateSpace(String),
    AddSpace(String, String),
    Link {
        space: String,
        revision: String,
        observation: String,
        item: String,
    },
    Propose {
        space: String,
        revision: String,
        observation: String,
        item: String,
    },
    Apply {
        space: String,
        revision: String,
        observation: String,
        item: String,
        expected: i64,
        x: i32,
        y: i32,
        rot: i32,
    },
    Preview(Box<RoomPlanDraft>),
    Layout(String),
    Save(usize),
}
enum Outcome {
    Changed,
    Comparison(RoomPlanReconciliationResponse),
    Proposal(RoomPlanPlacementProposalResponse),
    Layout(RoomPlanSavedLayoutPreview),
    Preview(String, String, Vec<u8>),
}
fn perform(action: Action) {
    let (source, generation, candidates) = {
        let mut s = state();
        if s.snap.busy {
            return;
        }
        let Some(source) = s.source.clone() else {
            s.snap.error = Some("Connect to the paired Mac to review this scan.".into());
            publish(&s);
            return;
        };
        s.snap.busy = true;
        s.snap.error = None;
        publish(&s);
        (source, s.generation, s.snap.candidates.clone())
    };
    shell::spawn(async move {
        let result: Result<Outcome, shell::ShellError> = async {
            match action {
                Action::Review(id, decision) => {
                    source.review(&id, decision).await?;
                    Ok(Outcome::Changed)
                }
                Action::Compare(before, after) => source
                    .reconcile(&before, &after)
                    .await
                    .map(Outcome::Comparison),
                Action::CreateSpace(id) => {
                    source.create_space(&id).await?;
                    Ok(Outcome::Changed)
                }
                Action::AddSpace(space, revision) => {
                    source.add_revision_to_space(&space, &revision).await?;
                    Ok(Outcome::Changed)
                }
                Action::Link {
                    space,
                    revision,
                    observation,
                    item,
                } => {
                    source
                        .link_object(&space, &revision, &observation, &item)
                        .await?;
                    Ok(Outcome::Changed)
                }
                Action::Propose {
                    space,
                    revision,
                    observation,
                    item,
                } => source
                    .propose_placement(&space, &revision, &observation, &item)
                    .await
                    .map(Outcome::Proposal),
                Action::Apply {
                    space,
                    revision,
                    observation,
                    item,
                    expected,
                    x,
                    y,
                    rot,
                } => {
                    source
                        .apply_placement(PlacementUpdate {
                            space_id: &space,
                            revision_id: &revision,
                            observation_id: &observation,
                            item_id: &item,
                            expected_revision: expected,
                            x,
                            y,
                            rotation: rot,
                        })
                        .await?;
                    Ok(Outcome::Changed)
                }
                Action::Preview(draft) => {
                    let bytes = source.revision_asset(&draft.draft_id).await?;
                    let json =
                        serde_json::to_string(&draft).map_err(|_| shell::ShellError::Unreadable)?;
                    Ok(Outcome::Preview(draft.draft_id, json, bytes))
                }
                Action::Layout(id) => source.layout(&id).await.map(Outcome::Layout),
                Action::Save(index) => {
                    let candidate = candidates.get(index).ok_or_else(|| {
                        shell::ShellError::Store("This proposal is no longer on screen.".into())
                    })?;
                    let id = format!("roomplan-{}", chrono::Utc::now().timestamp_millis());
                    source
                        .save_proposal(
                            &id,
                            &candidate.title,
                            candidate.items.clone(),
                            &candidate.details,
                        )
                        .await?;
                    Ok(Outcome::Changed)
                }
            }
        }
        .await;
        let mut reload = false;
        let mut preview = None;
        {
            let mut s = state();
            if s.generation != generation {
                return;
            }
            s.snap.busy = false;
            match result {
                Ok(Outcome::Changed) => {
                    s.snap.status = "Saved on the Mac".into();
                    reload = true;
                }
                Ok(Outcome::Comparison(v)) => s.snap.comparison = Some(v),
                Ok(Outcome::Proposal(v)) => s.snap.proposal = Some(v),
                Ok(Outcome::Layout(v)) => s.snap.layout = Some(v),
                Ok(Outcome::Preview(id, draft, bytes)) => {
                    if s.snap.selected.as_ref() == Some(&id) {
                        preview = Some(InteriorEvent::CachePreview { id, draft, bytes });
                    }
                }
                Err(error) => s.snap.error = Some(error.to_string()),
            }
            publish(&s);
        }
        if let Some(preview) = preview {
            emit(preview);
        }
        if reload {
            load();
        }
    });
}
fn plan(kind: PlanningKind, refs: Vec<String>) {
    let (source, generation, layout) = {
        let mut s = state();
        if s.snap.busy {
            return;
        }
        let Some(source) = s.source.clone() else {
            return;
        };
        s.snap.busy = true;
        s.snap.error = None;
        publish(&s);
        (source, s.generation, s.snap.layout.clone())
    };
    shell::spawn(async move {
        let result = async {
            let receipt = match kind {
                PlanningKind::Search => {
                    source
                        .search_layout(
                            &layout
                                .as_ref()
                                .ok_or_else(|| {
                                    shell::ShellError::Store("Select a saved layout first.".into())
                                })?
                                .layout
                                .id,
                            &refs,
                        )
                        .await?
                }
                PlanningKind::Compose => source.compose_layout(&refs).await?,
            };
            for _ in 0..1200 {
                if state().generation != generation {
                    return Err(shell::ShellError::Store("Planning was superseded.".into()));
                }
                let job = source.planning_job(receipt.job).await?;
                match job.state {
                    RoomPlanJobState::Running { .. } => {
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await
                    }
                    RoomPlanJobState::Failed { reason } => {
                        return Err(shell::ShellError::Store(reason))
                    }
                    RoomPlanJobState::Done { outcome } => {
                        let mut candidates = planning_candidates(
                            kind,
                            &outcome,
                            layout
                                .as_ref()
                                .map(|l| l.layout.items.as_slice())
                                .unwrap_or_default(),
                        );
                        for candidate in &mut candidates {
                            match source.preview_placements(&candidate.items).await {
                                Ok(review) => add_planning_analysis(candidate, &review),
                                Err(e) => candidate
                                    .details
                                    .push_str(&format!(" · Detailed review unavailable: {e}")),
                            }
                        }
                        return Ok(candidates);
                    }
                }
            }
            Err(shell::ShellError::Store(
                "Planning did not finish within ten minutes. Refresh before retrying.".into(),
            ))
        }
        .await;
        let mut s = state();
        if s.generation != generation {
            return;
        }
        s.snap.busy = false;
        match result {
            Ok(candidates) => {
                s.snap.candidates = candidates;
                s.snap.status = "Review a proposal before saving it".into();
            }
            Err(e) => s.snap.error = Some(e.to_string()),
        }
        publish(&s);
    });
}

#[component]
fn RevisionDetail() -> impl IntoElement {
    let snap = snapshot(&feed().tx);
    let id = state().snap.selected.clone().unwrap_or_default();
    let space = signal(
        state()
            .snap
            .spaces
            .spaces
            .iter()
            .find(|s| s.space.revision_ids.contains(&id))
            .map(|s| s.space.space_id.clone())
            .unwrap_or_default(),
    );
    let observation = signal(String::new());
    let item = signal(String::new());
    let x = signal("0".to_owned());
    let y = signal("0".to_owned());
    let rotation = signal("0".to_owned());
    let expected = signal(None::<(String, i64)>);
    let draft = snap.with(|s| {
        s.revisions
            .revisions
            .iter()
            .find(|d| d.draft_id == id)
            .cloned()
            .or_else(|| {
                s.local
                    .iter()
                    .find(|r| r.draft.draft_id == id)
                    .map(|r| r.draft.clone())
            })
    });
    let Some(draft) = draft else {
        return view().child(caption("This scan is no longer in the archive."));
    };
    let local = state().snap.local.iter().any(|r| r.draft.draft_id == id);
    let mut out = view()
        .gap_2()
        .child(action("Back to scans", || {
            let mut s = state();
            s.snap.selected = None;
            publish(&s);
        }))
        .child(heading(id.clone()))
        .child(caption(draft.created_at.clone()));
    if local {
        let id = id.clone();
        out = out.child(action("Preview local model", move || {
            emit(InteriorEvent::Preview(id.clone()))
        }));
    }
    if state()
        .snap
        .revisions
        .revisions
        .iter()
        .any(|d| d.draft_id == id)
    {
        let preview = draft.clone();
        let accept = id.clone();
        let reject = id.clone();
        let new_space = id.clone();
        out = out
            .child(action("Preview model from Mac", move || {
                perform(Action::Preview(Box::new(preview.clone())))
            }))
            .child(
                view()
                    .flex_row()
                    .child(action("Accept", move || {
                        perform(Action::Review(accept.clone(), RoomPlanDecision::Accept))
                    }))
                    .child(action("Reject", move || {
                        perform(Action::Review(reject.clone(), RoomPlanDecision::Reject))
                    })),
            )
            .child(action("Start room history", move || {
                perform(Action::CreateSpace(new_space.clone()))
            }));
    }
    let review_id = id.clone();
    out = out.child(dynamic(move || {
        snap.with(|s| {
            s.revisions
                .revisions
                .iter()
                .find(|d| d.draft_id == review_id)
                .and_then(|d| d.review.as_ref())
                .map(|r| caption(format!("Review: {}", r.decision)))
        })
    }));
    for element in draft
        .room
        .surfaces
        .iter()
        .chain(&draft.room.openings)
        .chain(&draft.room.objects)
    {
        let chosen = element.id.clone();
        out = out.child(action(
            format!("{} · {}", element.category, element.id),
            move || observation.set(chosen.clone()),
        ));
        if let Some(dimensions) = element.extra.get("dimensions_m").and_then(|v| v.as_array()) {
            out = out.child(caption(format!(
                "{} m",
                dimensions
                    .iter()
                    .map(|v| v.to_string())
                    .collect::<Vec<_>>()
                    .join(" × ")
            )));
        }
    }
    let revision = id.clone();
    out = out
        .child(dynamic(move || {
            let current = snap.get();
            let mut choices = view().gap_1();
            for entry in current.spaces.spaces {
                let room = entry.space.space_id.clone();
                let chosen = room.clone();
                let revision = revision.clone();
                choices = choices
                    .child(action(
                        format!("Use room · {}", entry.space.label),
                        move || space.set(chosen.clone()),
                    ))
                    .child(action(
                        format!("Add scan to {}", entry.space.label),
                        move || perform(Action::AddSpace(room.clone(), revision.clone())),
                    ));
            }
            for previous in current
                .revisions
                .revisions
                .iter()
                .filter(|d| d.draft_id != revision)
            {
                let before = previous.draft_id.clone();
                let after = revision.clone();
                choices = choices.child(action(
                    format!("Compare with {}", previous.created_at),
                    move || perform(Action::Compare(before.clone(), after.clone())),
                ));
            }
            if let Some(comparison) = current.comparison {
                for pair in comparison.reconciliation.matches {
                    choices = choices.child(caption(format!(
                        "{} → {} · {:.2} m · {}",
                        pair.before_observation_id,
                        pair.after_observation_id,
                        pair.distance_m,
                        pair.suggestion
                    )));
                }
            }
            choices
        }))
        .child(
            text(move || {
                format!(
                    "Room: {} · observation: {} · item: {}",
                    space.get(),
                    observation.get(),
                    item.get()
                )
            })
            .text_sm(),
        );
    out = out.child(dynamic(move || {
        let mut choices = view();
        for row in snap.with(|s| s.inventory.clone()) {
            let id = row.item.id.clone();
            choices = choices.child(action(row.item.label, move || {
                let placement = state()
                    .snap
                    .placements
                    .iter()
                    .find(|p| p.item_id == id)
                    .cloned();
                expected.set(Some((
                    id.clone(),
                    placement.as_ref().map_or(0, |p| p.revision),
                )));
                x.set(placement.as_ref().map_or(0, |p| p.x).to_string());
                y.set(placement.as_ref().map_or(0, |p| p.y).to_string());
                rotation.set(placement.as_ref().map_or(0, |p| p.rot).to_string());
                item.set(id.clone());
            }));
        }
        choices
    }));
    let revision = id.clone();
    let link_revision = id.clone();
    let apply_revision = id.clone();
    out = out
        .child(action("Link observation to inventory item", move || {
            perform(Action::Link {
                space: space.get(),
                revision: link_revision.clone(),
                observation: observation.get(),
                item: item.get(),
            })
        }))
        .child(action("Propose placement", move || {
            perform(Action::Propose {
                space: space.get(),
                revision: revision.clone(),
                observation: observation.get(),
                item: item.get(),
            })
        }))
        .child(dynamic(move || {
            snap.with(|s| s.proposal.clone()).map(|p| {
                caption(format!(
                    "Proposal: {}, {} cm · {}° · alignment error {:.2} m; review before applying",
                    p.proposal.x, p.proposal.y, p.proposal.rot, p.proposal.alignment_error_m
                ))
            })
        }))
        .child(action("Use proposed position", move || {
            if let Some(p) = state().snap.proposal.as_ref().filter(|p| {
                p.space_id == space.get()
                    && p.revision_id == id
                    && p.observation_id == observation.get()
                    && p.item_id == item.get()
            }) {
                x.set(p.proposal.x.to_string());
                y.set(p.proposal.y.to_string());
                rotation.set(p.proposal.rot.to_string());
            }
        }))
        .child(field("X · cm", x))
        .child(field("Y · cm", y))
        .child(field("Rotation · degrees", rotation))
        .child(action("Apply reviewed placement", move || {
            let (Ok(x), Ok(y), Ok(rot)) =
                (x.get().parse(), y.get().parse(), rotation.get().parse())
            else {
                let mut s = state();
                s.snap.error = Some("Use whole numbers for position and rotation.".into());
                publish(&s);
                return;
            };
            let Some((reviewed_item, expected)) = expected.get() else {
                return;
            };
            if reviewed_item != item.get() || space.get().is_empty() || observation.get().is_empty()
            {
                return;
            }
            perform(Action::Apply {
                space: space.get(),
                revision: apply_revision.clone(),
                observation: observation.get(),
                item: item.get(),
                expected,
                x,
                y,
                rot,
            });
        }));
    out
}

#[component]
fn LayoutPlanning() -> impl IntoElement {
    let snap = snapshot(&feed().tx);
    let chosen = signal(BTreeSet::<String>::new());
    let size = signal((320_f32, 260_f32));
    view()
        .gap_2()
        .child(heading("Layouts"))
        .child(dynamic(move || {
            let mut out = view();
            for layout in snap.with(|s| s.layouts.clone()) {
                let id = layout.id;
                out = out.child(action(
                    format!(
                        "{} · {} hard, {} soft",
                        layout.name, layout.hard, layout.soft
                    ),
                    move || perform(Action::Layout(id.clone())),
                ));
            }
            out
        }))
        .child(
            canvas(move || {
                let current = snap.get();
                let (width, height) = size.get();
                let mut p = Painter::new()
                    .stroke_color(color(nsh_base::theme().foreground))
                    .line_width(2.);
                let Some(model) = current.model else {
                    return p;
                };
                let min_x = model.polygon.iter().map(|v| v[0]).min().unwrap_or(0) as f32;
                let min_y = model.polygon.iter().map(|v| v[1]).min().unwrap_or(0) as f32;
                let max_x = model.polygon.iter().map(|v| v[0]).max().unwrap_or(1) as f32;
                let max_y = model.polygon.iter().map(|v| v[1]).max().unwrap_or(1) as f32;
                let scale = ((width - 24.).max(1.) / (max_x - min_x).max(1.))
                    .min((height - 24.).max(1.) / (max_y - min_y).max(1.));
                let at = |x: i32, y: i32| {
                    (
                        12. + (x as f32 - min_x) * scale,
                        12. + (y as f32 - min_y) * scale,
                    )
                };
                for (index, xy) in model.polygon.iter().enumerate() {
                    let (x, y) = at(xy[0], xy[1]);
                    p = if index == 0 {
                        p.begin_path().move_to(x, y)
                    } else {
                        p.line_to(x, y)
                    };
                }
                p = p.close_path().stroke();
                for wall in model.walls.values() {
                    let (x0, y0) = at(wall.from[0], wall.from[1]);
                    let (x1, y1) = at(wall.to[0], wall.to[1]);
                    p = p
                        .line_width(3.)
                        .begin_path()
                        .move_to(x0, y0)
                        .line_to(x1, y1)
                        .stroke();
                }
                for opening in &model.openings {
                    let Some(wall) = model.walls.get(&opening.wall) else {
                        continue;
                    };
                    let horizontal =
                        (wall.to[0] - wall.from[0]).abs() >= (wall.to[1] - wall.from[1]).abs();
                    let ((x0, y0), (x1, y1)) = if horizontal {
                        (at(opening.from, wall.from[1]), at(opening.to, wall.from[1]))
                    } else {
                        (at(wall.from[0], opening.from), at(wall.from[0], opening.to))
                    };
                    p = p
                        .stroke_color(color(nsh_base::theme().primary))
                        .line_width(5.)
                        .begin_path()
                        .move_to(x0, y0)
                        .line_to(x1, y1)
                        .stroke();
                }
                for fixture in &model.fixed_furniture {
                    let (x, y) = at(fixture.x[0], fixture.y[0]);
                    p = p.fill_color(color(nsh_base::theme().muted)).fill_rect(
                        x,
                        y,
                        (fixture.x[1] - fixture.x[0]) as f32 * scale,
                        (fixture.y[1] - fixture.y[0]) as f32 * scale,
                    );
                }
                let pieces: Vec<_> = current.layout.map(|l| l.layout.items).unwrap_or_else(|| {
                    current
                        .placements
                        .into_iter()
                        .map(|p| RoomPlanSavedLayoutItem {
                            reference: p.item_id,
                            x: p.x,
                            y: p.y,
                            rot: p.rot,
                            size: None,
                            kind: None,
                        })
                        .collect()
                });
                for piece in pieces {
                    let dimensions = piece.size.or_else(|| {
                        current
                            .inventory
                            .iter()
                            .find(|r| r.item.id == piece.reference)
                            .and_then(|r| r.item.width.zip(r.item.depth).map(|(w, d)| [w, d]))
                    });
                    let Some([mut w, mut d]) = dimensions else {
                        continue;
                    };
                    if (piece.rot.rem_euclid(180) - 90).abs() < 45 {
                        std::mem::swap(&mut w, &mut d);
                    }
                    let (x, y) = at(piece.x, piece.y);
                    p = p
                        .fill_color(color(nsh_base::theme().primary))
                        .fill_rect(x, y, w as f32 * scale, d as f32 * scale)
                        .fill_color(color(nsh_base::theme().foreground))
                        .font("11px sans-serif")
                        .fill_text(&piece.reference, x, y + 12.);
                }
                p
            })
            .h(px(260.))
            .on_layout(move |b: &nsh_core::Bounds<nsh_core::Points>| {
                size.set((b.size.width.0, b.size.height.0))
            }),
        )
        .child(dynamic(move || {
            let current = snap.get();
            let mut out = view();
            if let Some(model) = &current.model {
                out = out.child(caption(format!(
                    "{} × {} cm · {:.2} m²",
                    model.dimensions.width, model.dimensions.depth, model.area_m2
                )));
                for uncertainty in &model.unmeasured {
                    out = out.child(caption(format!(
                        "Unmeasured: {} · {}",
                        uncertainty.label,
                        uncertainty.fields.join(", ")
                    )));
                }
            }
            if let Some(layout) = current.layout {
                out = out
                    .child(heading(layout.layout.name))
                    .child(caption(format!("Checks: {}", layout.check)));
                for piece in layout.layout.items {
                    let id = piece.reference.clone();
                    out = out.child(action(
                        format!(
                            "{} · {}, {} cm · {}°",
                            piece.reference, piece.x, piece.y, piece.rot
                        ),
                        move || {
                            chosen.update(|set| {
                                if !set.remove(&id) {
                                    set.insert(id.clone());
                                }
                            })
                        },
                    ));
                }
            }
            for row in current.inventory {
                let id = row.item.id;
                out = out.child(action(row.item.label, move || {
                    chosen.update(|set| {
                        if !set.remove(&id) {
                            set.insert(id.clone());
                        }
                    })
                }));
            }
            out
        }))
        .child(
            text(move || {
                format!(
                    "Selected items: {}",
                    chosen.get().into_iter().collect::<Vec<_>>().join(", ")
                )
            })
            .text_sm(),
        )
        .child(action("Search positions in saved layout", move || {
            plan(PlanningKind::Search, chosen.get().into_iter().collect())
        }))
        .child(action("Compose selected inventory items", move || {
            plan(PlanningKind::Compose, chosen.get().into_iter().collect())
        }))
        .child(dynamic(move || {
            let mut out = view().gap_2();
            for (index, candidate) in snap.with(|s| s.candidates.clone()).into_iter().enumerate() {
                out = out
                    .child(heading(candidate.title))
                    .child(text(candidate.details).text_sm())
                    .child(action("Save reviewed proposal", move || {
                        perform(Action::Save(index))
                    }));
            }
            out
        }))
}

/// Native scan summaries, review actions and capability-owned layout planning.
#[component]
pub fn InteriorScreen() -> impl IntoElement {
    let snap = snapshot(&feed().tx);
    let layouts = signal(false);
    let selected = computed(move || snap.with(|s| s.selected.clone()));
    view()
        .flex_1()
        .px_4()
        .gap_2()
        .child(heading("Interior"))
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .child(action("Capture room", || emit(InteriorEvent::Capture)))
                .child(action("Sync scans", || emit(InteriorEvent::Sync)))
                .child(action("Import legacy scans", || {
                    emit(InteriorEvent::Import)
                }))
                .child(action("Refresh", load)),
        )
        .child(
            view()
                .flex_row()
                .child(action("Scans", move || layouts.set(false)))
                .child(action("Layouts", move || layouts.set(true))),
        )
        .child(
            text(move || {
                snap.with(|s| {
                    s.error.clone().unwrap_or_else(|| {
                        if s.busy {
                            "Working…".into()
                        } else if s.loading {
                            "Reading room history…".into()
                        } else {
                            s.status.clone()
                        }
                    })
                })
            })
            .text_sm(),
        )
        .child(scroll().flex_1().child(dynamic(move || {
            if layouts.get() {
                return layout_planning().into_element();
            }
            if selected.get().is_some() {
                return revision_detail().into_element();
            }
            view()
                .child(dynamic(move || {
                    let current = snap.get();
                    let mut out = view().gap_2();
                    if let Some(live) = current.live {
                        out = out
                            .child(heading(live.state))
                            .child(text(live.guidance).text_sm());
                    }
                    for record in current.local {
                        let id = record.draft.draft_id;
                        let chosen = id.clone();
                        out = out
                            .child(action(
                                format!("{} · {}", record.draft.created_at, record.sync_state),
                                move || {
                                    let mut s = state();
                                    s.snap.selected = Some(chosen.clone());
                                    s.snap.proposal = None;
                                    s.snap.comparison = None;
                                    s.snap.comparison = None;
                                    s.snap.proposal = None;
                                    publish(&s);
                                },
                            ))
                            .child(caption(id));
                    }
                    for revision in current.revisions.revisions {
                        let chosen = revision.draft_id;
                        out = out.child(action(
                            format!(
                                "{} · {}",
                                revision.created_at,
                                revision
                                    .review
                                    .map(|r| r.decision)
                                    .unwrap_or_else(|| "Unreviewed".into())
                            ),
                            move || {
                                let mut s = state();
                                s.snap.selected = Some(chosen.clone());
                                s.snap.comparison = None;
                                s.snap.proposal = None;
                                publish(&s);
                            },
                        ));
                    }
                    out
                }))
                .into_element()
        })))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capture_layout_and_revision_review_render_without_a_mac() {
        use nsh_engine::EngineConfig;
        use nsh_testing::Headless;
        let mut app = Headless::start(EngineConfig {
            ui_sets: vec![nsh_base::registry()],
            ..EngineConfig::new(nsh_core::element::Root::of::<InteriorScreen>())
        })
        .unwrap();
        app.wait_for("text:Capture room").unwrap();
        app.press("text:Layouts").unwrap();
        app.wait_for("text:Compose selected inventory items")
            .unwrap();
        app.press("text:Scans").unwrap();
        {
            let mut s = state();
            s.snap.selected = Some("synthetic-scan".into());
            s.snap.revisions.revisions.push(RoomPlanDraft {
                draft_id: "synthetic-scan".into(),
                created_at: "2026-11-09".into(),
                ..Default::default()
            });
            publish(&s);
        }
        app.wait_for("text:Apply reviewed placement").unwrap();
        app.press("text:Apply reviewed placement").unwrap();
        assert!(!state().snap.busy);
        app.press("text:Back to scans").unwrap();
        assert!(state().snap.selected.is_none());
    }
}
