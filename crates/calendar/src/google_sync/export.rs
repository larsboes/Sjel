use super::auth::SyncResult;
use super::*;

// ---- export ---------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct ExportOutcome {
    pub entry_id: String,
    pub title: String,
    /// `inserted` on the first push, `patched` afterwards.
    pub operation: &'static str,
    pub google_event_id: Option<String>,
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct ExportReport {
    pub calendar_id: String,
    pub home_timezone: String,
    pub opted_in: usize,
    pub inserted: usize,
    pub patched: usize,
    pub pushed: Vec<ExportOutcome>,
    pub skipped: Vec<SkippedEvent>,
    pub dry_run: bool,
}

/// Pushes the opted-in entries, and only those.
///
/// The queue is the `google_exports` table, which is empty until the operator
/// opts an entry in one at a time — there is no "export everything" path and
/// no default-on flag. An entry that already carries a Google event id is
/// patched rather than inserted, so a second run updates rather than
/// duplicating.
pub fn export(
    store: &dyn ExportStore,
    api: &dyn CalendarApi,
    settings: &Settings,
    dry_run: bool,
) -> SyncResult<ExportReport> {
    let queue = store.queue()?;

    let mut report = ExportReport {
        calendar_id: settings.calendar_id.clone(),
        home_timezone: settings.tz.name().to_string(),
        opted_in: queue.len(),
        dry_run,
        ..Default::default()
    };

    for (optin, entry) in queue {
        let body = match google::export_body(&entry, &settings.tz) {
            Ok(body) => body,
            Err(reason) => {
                report.skipped.push(SkippedEvent {
                    google_event_id: optin.google_event_id.clone().unwrap_or_default(),
                    reason: format!("{}: {reason}", entry.id),
                });
                continue;
            }
        };
        // The ledger's calendar id wins over the current config: an entry
        // opted in against one calendar must not silently move to another.
        let calendar_id = &optin.google_calendar_id;
        let existing_event = optin.google_event_id.as_deref().filter(|id| !id.is_empty());
        let operation = if existing_event.is_some() {
            "patched"
        } else {
            "inserted"
        };

        if dry_run {
            report.pushed.push(ExportOutcome {
                entry_id: entry.id,
                title: entry.title,
                operation,
                google_event_id: existing_event.map(str::to_string),
            });
            continue;
        }

        let pushed = match existing_event {
            Some(event_id) => api.patch_event(calendar_id, event_id, &body),
            None => api.insert_event(calendar_id, &body),
        };
        match pushed {
            Ok(google_event_id) => {
                store.record_push(&entry.id, &google_event_id)?;
                match operation {
                    "patched" => report.patched += 1,
                    _ => report.inserted += 1,
                }
                report.pushed.push(ExportOutcome {
                    entry_id: entry.id,
                    title: entry.title,
                    operation,
                    google_event_id: Some(google_event_id),
                });
            }
            Err(reason) => report.skipped.push(SkippedEvent {
                google_event_id: existing_event.unwrap_or_default().to_string(),
                reason: format!("{}: {reason}", entry.id),
            }),
        }
    }
    Ok(report)
}

/// Entries a UI would offer an export toggle for — everything `export_refusal`
/// does not veto. Exposed so the refusal rule is asked once, here and in the
/// store, rather than reimplemented in a dashboard.
pub fn exportable(entries: &[Entry]) -> Vec<&Entry> {
    entries
        .iter()
        .filter(|entry| google::export_refusal(entry).is_none())
        .collect()
}
