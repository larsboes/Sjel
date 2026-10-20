use super::auth::{SyncResult, MAX_REVIEW_DAYS};
use super::*;

// ---- import ---------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct ImportOutcome {
    pub google_event_id: String,
    pub title: String,
    pub action: Action,
    /// The Sjel entry, once there is one.
    pub entry_id: Option<String>,
    /// Set when Sjel's version won: what Google now says, so the divergence is
    /// visible instead of silently dropped.
    pub google_says: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SkippedEvent {
    pub google_event_id: String,
    pub reason: String,
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct ImportReport {
    pub calendar_id: String,
    pub home_timezone: String,
    pub from: String,
    pub to: String,
    pub fetched: usize,
    pub created: usize,
    pub refreshed: usize,
    pub unchanged: usize,
    pub kept_axon_version: usize,
    pub dropped_drafts: usize,
    pub outcomes: Vec<ImportOutcome>,
    pub skipped: Vec<SkippedEvent>,
    pub dry_run: bool,
}

/// The status of one Google event in the review-before-import surface.  This
/// is intentionally more cautious than the unattended import: a person must
/// see why an event is not selectable before Sjel writes even a draft.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReviewStatus {
    Importable,
    LikelyDuplicate,
    AlreadyInAxon,
    Cancelled,
    Invalid,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportCandidate {
    pub google_event_id: String,
    /// Google's revision marker. The selected-import endpoint requires this
    /// exact value again, so an event changed after review cannot slip in.
    pub google_updated: Option<String>,
    pub title: String,
    pub starts_at: Option<String>,
    pub ends_at: Option<String>,
    pub all_day: Option<bool>,
    pub location: Option<String>,
    pub html_link: Option<String>,
    pub recurring_event_id: Option<String>,
    pub status: ReviewStatus,
    pub reason: Option<String>,
    /// An opaque label joining candidates which have identical normalized
    /// title and interval. It is a review hint, never an automatic deletion.
    pub duplicate_group: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportPreview {
    pub calendar_id: String,
    pub home_timezone: String,
    pub from: String,
    pub to: String,
    pub fetched: usize,
    /// The configured cap was reached, so this is not a complete review.
    pub at_event_limit: bool,
    pub candidates: Vec<ImportCandidate>,
}

/// One explicit choice from an `ImportPreview`.
#[derive(Debug, Clone, Deserialize)]
pub struct SelectedGoogleEvent {
    pub google_event_id: String,
    pub google_updated: Option<String>,
}

pub(super) fn review_window(from: &str, to: &str) -> SyncResult<(String, String)> {
    let from_days =
        date::parse_date(from).ok_or_else(|| "from must be a valid YYYY-MM-DD date".to_string())?;
    let to_days =
        date::parse_date(to).ok_or_else(|| "to must be a valid YYYY-MM-DD date".to_string())?;
    let days = to_days - from_days;
    if days <= 0 {
        return Err("to must be after from".into());
    }
    if days > MAX_REVIEW_DAYS {
        return Err(format!(
            "Google import review is limited to {MAX_REVIEW_DAYS} days; narrow the date range before reviewing"
        ));
    }
    Ok((format!("{from}T00:00:00Z"), format!("{to}T00:00:00Z")))
}

pub(super) fn normalized_title(title: &str) -> String {
    let mut normalized = String::with_capacity(title.len());
    for character in title.chars() {
        if character.is_alphanumeric() {
            normalized.extend(character.to_lowercase());
        } else {
            normalized.push(' ');
        }
    }
    normalized.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(super) fn duplicate_key(candidate: &ImportCandidate) -> Option<String> {
    if candidate.status != ReviewStatus::Importable {
        return None;
    }
    Some(format!(
        "{}\u{1f}{}\u{1f}{}\u{1f}{}",
        normalized_title(&candidate.title),
        candidate.starts_at.as_deref().unwrap_or_default(),
        candidate.ends_at.as_deref().unwrap_or_default(),
        candidate.all_day.unwrap_or(false),
    ))
}

pub(super) fn candidate_for(
    event: &GoogleEvent,
    existing: Option<&Entry>,
    tz: &HomeTimezone,
) -> ImportCandidate {
    let mut candidate = ImportCandidate {
        google_event_id: event.id.clone(),
        google_updated: event.updated.clone(),
        title: event
            .summary
            .as_deref()
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .unwrap_or(google::UNTITLED)
            .to_string(),
        starts_at: None,
        ends_at: None,
        all_day: None,
        location: event.location.clone(),
        html_link: event.html_link.clone(),
        recurring_event_id: event.recurring_event_id.clone(),
        status: ReviewStatus::Invalid,
        reason: None,
        duplicate_group: None,
    };

    match google::decide(event, existing) {
        Action::Skip | Action::DropDraft | Action::KeepCancelledAxonVersion => {
            candidate.status = ReviewStatus::Cancelled;
            candidate.reason = Some("Cancelled in Google".into());
        }
        Action::RefreshDraft | Action::KeepAxonVersion => {
            candidate.status = ReviewStatus::AlreadyInAxon;
            candidate.reason = Some(
                match google::decide(event, existing) {
                    Action::RefreshDraft => "Already imported as an Sjel draft",
                    Action::KeepAxonVersion => "Already adopted in Sjel; Sjel keeps its version",
                    _ => unreachable!("handled above"),
                }
                .into(),
            );
        }
        Action::Create => match google::map_event(event, tz) {
            Ok(mapped) => {
                candidate.title = mapped.title;
                candidate.starts_at = Some(mapped.starts_at);
                candidate.ends_at = Some(mapped.ends_at);
                candidate.all_day = Some(mapped.all_day);
                candidate.location = mapped.location;
                candidate.status = ReviewStatus::Importable;
            }
            Err(reason) => candidate.reason = Some(reason),
        },
    }
    candidate
}

pub(super) fn review_events(
    store: &dyn ImportStore,
    events: &[GoogleEvent],
    tz: &HomeTimezone,
) -> SyncResult<Vec<ImportCandidate>> {
    let mut candidates = Vec::with_capacity(events.len());
    for event in events {
        candidates.push(candidate_for(
            event,
            store.existing(event.id.trim())?.as_ref(),
            tz,
        ));
    }

    let mut grouped: HashMap<String, Vec<usize>> = HashMap::new();
    for (index, candidate) in candidates.iter().enumerate() {
        if let Some(key) = duplicate_key(candidate) {
            grouped.entry(key).or_default().push(index);
        }
    }
    let mut group_number = 0usize;
    for indexes in grouped.values().filter(|indexes| indexes.len() > 1) {
        group_number += 1;
        let group = format!("duplicate-{group_number}");
        for index in indexes {
            candidates[*index].status = ReviewStatus::LikelyDuplicate;
            candidates[*index].duplicate_group = Some(group.clone());
            candidates[*index].reason =
                Some("Same normalized title and exact time range as another Google event".into());
        }
    }
    Ok(candidates)
}

/// Fetches a bounded Google slice and classifies it for an operator.  It is
/// read-only: no preview row or imported entry is stored locally.
pub fn preview(
    store: &dyn ImportStore,
    api: &dyn CalendarApi,
    settings: &Settings,
    from: &str,
    to: &str,
) -> SyncResult<ImportPreview> {
    let (time_min, time_max) = review_window(from, to)?;
    let events = api.list_events(
        &settings.calendar_id,
        &time_min,
        &time_max,
        settings.google.max_events,
    )?;
    let candidates = review_events(store, &events, &settings.tz)?;
    Ok(ImportPreview {
        calendar_id: settings.calendar_id.clone(),
        home_timezone: settings.tz.name().to_string(),
        from: time_min,
        to: time_max,
        fetched: events.len(),
        at_event_limit: events.len() == settings.google.max_events,
        candidates,
    })
}

/// Imports only the events explicitly selected from a preview.  The provider
/// is queried again and every `updated` marker must still match, preventing a
/// stale preview from becoming an unintended write.
pub fn import_selected(
    store: &dyn ImportStore,
    api: &dyn CalendarApi,
    settings: &Settings,
    from: &str,
    to: &str,
    selected: &[SelectedGoogleEvent],
) -> SyncResult<ImportReport> {
    if selected.is_empty() {
        return Err("select at least one Google event to import".into());
    }
    if selected.len() > 100 {
        return Err("select at most 100 events per reviewed import".into());
    }
    let (time_min, time_max) = review_window(from, to)?;
    let mut expected: HashMap<String, Option<String>> = HashMap::new();
    for selection in selected {
        let id = selection.google_event_id.trim();
        if id.is_empty() || expected.contains_key(id) {
            return Err("every selected Google event needs one unique id".into());
        }
        expected.insert(id.to_string(), selection.google_updated.clone());
    }

    let events = api.list_events(
        &settings.calendar_id,
        &time_min,
        &time_max,
        settings.google.max_events,
    )?;
    let by_id: HashMap<&str, &GoogleEvent> = events
        .iter()
        .map(|event| (event.id.as_str(), event))
        .collect();
    for (id, updated) in &expected {
        let Some(event) = by_id.get(id.as_str()) else {
            return Err(format!(
                "Google event {id} is no longer in this review window; review again"
            ));
        };
        if event.updated != *updated {
            return Err(format!(
                "Google event {id} changed since the preview; review it again"
            ));
        }
    }

    let reviewed = review_events(store, &events, &settings.tz)?;
    let candidates: HashMap<&str, &ImportCandidate> = reviewed
        .iter()
        .map(|candidate| (candidate.google_event_id.as_str(), candidate))
        .collect();
    let mut duplicate_groups = HashSet::new();
    for id in expected.keys() {
        let candidate = candidates
            .get(id.as_str())
            .ok_or_else(|| format!("Google event {id} could not be reviewed"))?;
        if !matches!(
            candidate.status,
            ReviewStatus::Importable | ReviewStatus::LikelyDuplicate
        ) {
            return Err(format!(
                "Google event {id} is no longer importable; review again"
            ));
        }
        if let Some(group) = &candidate.duplicate_group {
            if !duplicate_groups.insert(group) {
                return Err("choose at most one event from each likely-duplicate group".into());
            }
        }
    }

    let chosen: Vec<GoogleEvent> = selected
        .iter()
        .filter_map(|selection| {
            by_id
                .get(selection.google_event_id.trim())
                .copied()
                .cloned()
        })
        .collect();
    import_events(store, &chosen, settings, &time_min, &time_max, false)
}

/// `timeMin`/`timeMax` for the import, as the UTC-marked RFC 3339 Google
/// wants. Day granularity: the window only bounds the fetch, and the entries
/// inside it are converted individually.
pub fn import_window(today: i64, days_back: i64, days_ahead: i64) -> (String, String) {
    (
        format!("{}T00:00:00Z", date::format_date(today - days_back.max(0))),
        format!("{}T00:00:00Z", date::format_date(today + days_ahead.max(1))),
    )
}

/// Pulls the configured Google calendar into drafts.
///
/// Idempotent by construction: the dedupe key is `(source = "google",
/// external_id = <Google event id>)`, which the store's partial unique index
/// enforces, so a second run over an unchanged calendar writes nothing at all.
/// `google::decide` is what keeps a confirmed entry safe from the second run.
pub fn import(
    store: &dyn ImportStore,
    api: &dyn CalendarApi,
    settings: &Settings,
    dry_run: bool,
) -> SyncResult<ImportReport> {
    let (from, to) = import_window(
        date::today_days(),
        settings.google.import_days_back,
        settings.google.import_days_ahead,
    );
    let events = api.list_events(
        &settings.calendar_id,
        &from,
        &to,
        settings.google.max_events,
    )?;
    import_events(store, &events, settings, &from, &to, dry_run)
}

/// Applies a known provider slice. Kept separate from `import` so a reviewed
/// selection goes through exactly the same conflict policy and upsert path.
pub(super) fn import_events(
    store: &dyn ImportStore,
    events: &[GoogleEvent],
    settings: &Settings,
    from: &str,
    to: &str,
    dry_run: bool,
) -> SyncResult<ImportReport> {
    let mut report = ImportReport {
        calendar_id: settings.calendar_id.clone(),
        home_timezone: settings.tz.name().to_string(),
        from: from.to_string(),
        to: to.to_string(),
        fetched: events.len(),
        dry_run,
        ..Default::default()
    };

    for event in events {
        let existing = store.existing(event.id.trim())?;

        match google::decide(event, existing.as_ref()) {
            Action::Skip => {}
            Action::KeepCancelledAxonVersion => {
                report.kept_axon_version += 1;
                report.outcomes.push(ImportOutcome {
                    google_event_id: event.id.clone(),
                    title: existing
                        .as_ref()
                        .map(|e| e.title.clone())
                        .unwrap_or_default(),
                    action: Action::KeepCancelledAxonVersion,
                    entry_id: existing.as_ref().map(|e| e.id.clone()),
                    google_says: Some(serde_json::json!({ "status": "cancelled" })),
                });
            }
            Action::KeepAxonVersion => {
                let existing = existing.expect("KeepAxonVersion implies an entry");
                report.kept_axon_version += 1;
                report.outcomes.push(ImportOutcome {
                    google_event_id: event.id.clone(),
                    title: existing.title.clone(),
                    action: Action::KeepAxonVersion,
                    entry_id: Some(existing.id),
                    google_says: Some(serde_json::to_value(event).unwrap_or(Value::Null)),
                });
            }
            Action::DropDraft => {
                let draft = existing.expect("DropDraft implies an entry");
                if !dry_run {
                    store.delete(&draft.id)?;
                }
                report.dropped_drafts += 1;
                report.outcomes.push(ImportOutcome {
                    google_event_id: event.id.clone(),
                    title: draft.title.clone(),
                    action: Action::DropDraft,
                    entry_id: Some(draft.id),
                    google_says: Some(serde_json::json!({ "status": "cancelled" })),
                });
            }
            action @ (Action::Create | Action::RefreshDraft) => {
                let candidate = match google::map_event(event, &settings.tz) {
                    Ok(candidate) => candidate,
                    Err(reason) => {
                        report.skipped.push(SkippedEvent {
                            google_event_id: event.id.clone(),
                            reason,
                        });
                        continue;
                    }
                };
                if let Some(existing) = &existing {
                    if !google::differs(&candidate, existing) {
                        report.unchanged += 1;
                        continue;
                    }
                }
                let entry_id = if dry_run {
                    existing.as_ref().map(|e| e.id.clone())
                } else {
                    match store.upsert(&candidate) {
                        Ok(entry) => Some(entry.id),
                        Err(error) => {
                            report.skipped.push(SkippedEvent {
                                google_event_id: event.id.clone(),
                                reason: format!("calendar rejected the entry: {error}"),
                            });
                            continue;
                        }
                    }
                };
                match action {
                    Action::Create => report.created += 1,
                    _ => report.refreshed += 1,
                }
                report.outcomes.push(ImportOutcome {
                    google_event_id: event.id.clone(),
                    title: candidate.title,
                    action,
                    entry_id,
                    google_says: None,
                });
            }
        }
    }
    Ok(report)
}
