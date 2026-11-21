//! Room scan archive, revision review, placements and layout planning on the phone.
use sjel_gpui_core::{
    interior::{
        planning::{add_planning_analysis, planning_candidates, PlanningCandidate, PlanningKind},
        *,
    },
    inventory::{InteriorLayoutSummary, InventoryRow},
    shell,
};
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

mod phone;
pub use phone::PhoneInterior;
