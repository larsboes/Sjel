//! Two-way delta sync between a phone's local replica and the Mac it is paired with.
//!
//! The phone writes into [`TursoStore`] and logs every write in `_offline_mutations`. Nothing used
//! to replay that log: a note typed in a tunnel stayed on the phone. [`run`] is the replay, and it
//! runs in both directions over the same signed connection the app already has:
//!
//! 1. **Push.** Each queued write is replayed, oldest first, through the routes the Mac already
//!    has for the web dashboard (`capabilities/projects/README.md`). A write replays in order and
//!    the queue stops at the first refusal, because two edits of one note are a sequence: applying
//!    the older one last would move the note backwards. A record the Mac has never had is created
//!    there with its project, since a record cannot live in a project that does not exist.
//! 2. **Pull.** Every live project is listed without contents, and only the records whose
//!    `updated_at` moved since this replica last agreed with the Mac are read in full. A record
//!    with an unsent local write is skipped: the phone's text is newer than anything the Mac can
//!    answer with. A record the Mac no longer has is deleted here.
//!
//! Two edits of the same note are settled by the queue: the phone's text is replayed over the
//! Mac's version and the report counts it, so nothing typed offline is lost. The Mac's version is
//! what the two sides agree on afterwards, because the pull then reads it back.
//!
//! Nothing here holds a GPUI type or a socket: [`Host`] is one request, `DeviceClient` is the real
//! one, and the tests answer with their own.

use std::collections::HashSet;
use std::future::Future;

use serde_json::{json, Value};

use crate::device::DeviceClient;
use crate::shell::{CalendarEntry, ProjectRecord};
use crate::store::TursoStore;

const PROJECTS: &str = "/projects/api/projects";
const PROJECT: &str = "/projects/api/project";
const RECORD: &str = "/projects/api/record";
const NOTES: &str = "/projects/api/notes";
const TASKS: &str = "/projects/api/tasks";
const TASKS_DONE: &str = "/projects/api/tasks/done";
const RECORD_PROPERTY: &str = "/projects/api/record/property";
const RECORD_REMOVE: &str = "/projects/api/record/remove";
/// The calendar's entries: a create on the collection, a change and a delete by id, and a change
/// feed that answers what moved since a cursor (`capabilities/calendar/src/server.rs`).
const ENTRIES: &str = "/calendar/api/entries";
const ENTRY_CHANGES: &str = "/calendar/api/entries/changes";

/// One HTTP answer from the Mac, undecoded, so a 409's `current` can be read instead of lost.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reply {
    pub status: u16,
    pub body: Vec<u8>,
}

impl Reply {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// The answer's JSON, or `None` when it is not JSON at all.
    pub fn json(&self) -> Option<Value> {
        serde_json::from_slice(&self.body).ok()
    }

    /// The refusal the Mac sent: its `error`, or the status when it sent no message.
    fn refused(&self) -> String {
        self.json()
            .and_then(|body| body["error"].as_str().map(str::to_owned))
            .unwrap_or_else(|| format!("the Mac refused the request (HTTP {})", self.status))
    }

    /// True when the Mac does not have the record, or its project: the write is a create.
    ///
    /// The phrasings are the capability's own, measured against a live shell: `no record "…"` from
    /// a save, `no task "…"` from a done toggle, `no live project named "…"` where the project is
    /// checked first (`capabilities/projects/src/ops.rs`).
    fn missing(&self) -> bool {
        if !matches!(self.status, 400 | 404) {
            return false;
        }
        let error = self.refused();
        ["no record", "no such record", "no task", "no live project"]
            .iter()
            .any(|phrase| error.contains(phrase))
    }
}

/// What a sync needs from the Mac: one signed request. `DeviceClient` is the real one.
///
/// The `Send` bound is spelled out because the future runs on the app's HTTP runtime; an `async
/// fn` in the trait would have implied nothing about it.
pub trait Host {
    fn send(
        &self,
        method: &str,
        path: &str,
        body: Option<&[u8]>,
    ) -> impl Future<Output = Result<Reply, String>> + Send;
}

impl Host for DeviceClient {
    async fn send(&self, method: &str, path: &str, body: Option<&[u8]>) -> Result<Reply, String> {
        let method = match method {
            "GET" => reqwest::Method::GET,
            "POST" => reqwest::Method::POST,
            other => return Err(format!("this app sends no {other} to the Mac")),
        };
        let (status, body) = self.signed(method, path, body.unwrap_or_default()).await?;
        Ok(Reply { status, body })
    }
}

/// A shared host is a host, so a view can hold one [`std::sync::Arc`] and hand it to a sync.
impl<T: Host + Send + Sync> Host for std::sync::Arc<T> {
    async fn send(&self, method: &str, path: &str, body: Option<&[u8]>) -> Result<Reply, String> {
        T::send(self, method, path, body).await
    }
}

/// What one sync did, in the counts the screen shows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// Queued writes replayed on the Mac.
    pub pushed: usize,
    /// Of those, records the Mac had never had.
    pub created: usize,
    /// Queued writes whose note the Mac had changed too. The phone's text was replayed over it.
    pub conflicts: usize,
    /// Records the Mac changed and this device now holds.
    pub pulled: usize,
    /// Records the Mac no longer had, deleted here.
    pub removed: usize,
    /// Writes still queued, each `path (reason)`. Non-empty means the queue stopped there.
    pub kept: Vec<String>,
}

impl Report {
    /// One sentence for the screen: what moved, in the order it moved.
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        if self.pushed > 0 {
            parts.push(format!("{} sent", self.pushed));
        }
        if self.created > 0 {
            parts.push(format!("{} new on the Mac", self.created));
        }
        if self.conflicts > 0 {
            parts.push(format!(
                "{} edited on both sides, this device's text won",
                self.conflicts
            ));
        }
        if self.pulled > 0 {
            parts.push(format!("{} updated from the Mac", self.pulled));
        }
        if self.removed > 0 {
            parts.push(format!("{} removed", self.removed));
        }
        if parts.is_empty() {
            return match self.kept.first() {
                None => "In step with the Mac.".to_string(),
                Some(_) => format!("Kept for later: {}", self.kept.join(", ")),
            };
        }
        let mut text = parts.join(" \u{00b7} ");
        if let Some(first) = self.kept.first() {
            text.push_str(&format!(" \u{00b7} kept for later: {first}"));
        }
        text
    }
}

/// Pushes every queued write, then pulls what the Mac changed.
///
/// # Errors
/// The local store failed, or the Mac could not be reached at all. A refusal from the Mac is not an
/// error: it is reported in [`Report::kept`], and the write stays queued.
pub async fn run<H: Host>(store: &TursoStore, host: &H) -> Result<Report, String> {
    let mut report = Report::default();
    // One push per domain, in a fixed order so a run is reproducible. Ordering only has to hold
    // inside a domain, and a calendar write the Mac refuses must not hold up a note.
    push(store, host, crate::store::DOMAIN_PROJECTS, &mut report).await?;
    push(store, host, crate::store::DOMAIN_CALENDAR, &mut report).await?;
    pull(store, host, &mut report).await?;
    pull_calendar(store, host, &mut report).await?;
    Ok(report)
}

// ---------------------------------------------------------------- push

async fn push<H: Host>(
    store: &TursoStore,
    host: &H,
    domain: &str,
    report: &mut Report,
) -> Result<(), String> {
    // The oldest write each time, read fresh: a create moves its record, and any write queued for
    // that record, to the path the Mac picked (`TursoStore::rekey`), so a list read once at the
    // start would go on naming paths that no longer exist.
    let mut attempted: HashSet<i64> = HashSet::new();
    loop {
        let Some(mutation) = store
            .pending_in(Some(domain))
            .await
            .map_err(|e| e.to_string())?
            .into_iter()
            .next()
        else {
            break;
        };
        if !attempted.insert(mutation.id) {
            report.kept.push(format!(
                "{} (the queue could not be cleared)",
                mutation.path
            ));
            break;
        }
        match replay(store, host, report, &mutation).await {
            Ok(()) => {
                store
                    .clear_mutations(&[mutation.id])
                    .await
                    .map_err(|e| e.to_string())?;
                report.pushed += 1;
            }
            Err(reason) => {
                report.kept.push(format!("{} ({reason})", mutation.path));
                break;
            }
        }
    }
    Ok(())
}

/// Replays one queued write. `Err` means it stays queued, with the reason the Mac gave.
async fn replay<H: Host>(
    store: &TursoStore,
    host: &H,
    report: &mut Report,
    mutation: &crate::store::PendingMutation,
) -> Result<(), String> {
    let payload: Value = serde_json::from_str(&mutation.payload)
        .map_err(|e| format!("this device queued a write it cannot read: {e}"))?;
    match mutation.action.as_str() {
        "save_record" => save(store, host, report, &mutation.path, &payload).await,
        "set_task_done" => done(store, host, report, &mutation.path, &payload).await,
        "set_task_priority" => priority(store, host, report, &mutation.path, &payload).await,
        "set_task_due" => due(host, &mutation.path, &payload).await,
        "delete_record" => remove(store, host, report, &mutation.path).await,
        "calendar_save" => calendar_save(store, host, report, &mutation.path, &payload).await,
        "calendar_delete" => calendar_delete(host, &mutation.path).await,
        other => Err(format!("this app has no way to replay a {other}")),
    }
}

/// Replays a task priority change.
async fn priority<H: Host>(
    _store: &TursoStore,
    host: &H,
    _report: &mut Report,
    path: &str,
    payload: &Value,
) -> Result<(), String> {
    let p = payload["priority"].as_u64().unwrap_or(2);
    let reply = post(
        host,
        RECORD_PROPERTY,
        json!({ "path": path, "key": "priority", "value": p }),
    )
    .await?;
    if !reply.ok() && !reply.missing() {
        return Err(reply.refused());
    }
    Ok(())
}

/// Replays a due date change. An empty date clears it.
async fn due<H: Host>(host: &H, path: &str, payload: &Value) -> Result<(), String> {
    let due = payload["due"].as_str().unwrap_or_default();
    let reply = post(
        host,
        RECORD_PROPERTY,
        json!({ "path": path, "key": "due", "value": due }),
    )
    .await?;
    if !reply.ok() && !reply.missing() {
        return Err(reply.refused());
    }
    Ok(())
}

/// Replays a record deletion on the Mac.
async fn remove<H: Host>(
    store: &TursoStore,
    host: &H,
    report: &mut Report,
    path: &str,
) -> Result<(), String> {
    let reply = post(host, RECORD_REMOVE, json!({ "path": path })).await?;
    if !reply.ok() && !reply.missing() {
        return Err(reply.refused());
    }
    store.remove_local(path).await.map_err(|e| e.to_string())?;
    report.removed += 1;
    Ok(())
}

/// Replays one content write: replace what the Mac has, or create what it does not.
async fn save<H: Host>(
    store: &TursoStore,
    host: &H,
    report: &mut Report,
    path: &str,
    payload: &Value,
) -> Result<(), String> {
    let content = payload["content"].as_str().unwrap_or_default();
    let state = store
        .sync_state(path)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "its record is gone from this device".to_string())?;

    if let Some(base) = state.synced_content.clone() {
        let reply = post(
            host,
            RECORD,
            json!({ "path": path, "content": content, "base": base }),
        )
        .await?;
        if reply.ok() {
            store
                .mark_synced(path, content)
                .await
                .map_err(|e| e.to_string())?;
            return Ok(());
        }
        if reply.status == 409 {
            // Both sides moved. The phone's text is what the person typed offline, so it is
            // replayed over the Mac's version rather than dropped; the pull reads the result back.
            let current = reply
                .json()
                .and_then(|body| body["current"].as_str().map(str::to_owned))
                .unwrap_or_default();
            let retry = post(
                host,
                RECORD,
                json!({ "path": path, "content": content, "base": current }),
            )
            .await?;
            if retry.ok() {
                report.conflicts += 1;
                store
                    .mark_synced(path, content)
                    .await
                    .map_err(|e| e.to_string())?;
                return Ok(());
            }
            return Err(retry.refused());
        }
        if !reply.missing() {
            return Err(reply.refused());
        }
        // The Mac no longer has the record: fall through and create it again.
    }
    let made = create(store, host, report, path, &state, content).await?;
    if state.kind == "task" && state.done {
        tick(host, &made, true).await?;
    }
    Ok(())
}

/// Creates on the Mac a record it has never had, in its project, and takes over the path the Mac
/// picks. Answers with the path the record now has on both sides.
async fn create<H: Host>(
    store: &TursoStore,
    host: &H,
    report: &mut Report,
    path: &str,
    state: &crate::store::SyncState,
    content: &str,
) -> Result<String, String> {
    ensure_project(host, &state.project).await?;
    let made = if state.kind == "project" {
        // A project made on the phone is its hub note here. Creating the project made the Mac's
        // own hub, so the phone takes over that path instead of adding a second note.
        hub_path(host, &state.project).await?
    } else {
        create_record(host, state).await?
    };
    store.rekey(path, &made).await.map_err(|e| e.to_string())?;
    report.created += 1;

    // A record the Mac created carries the frontmatter the vault's template wrote, which is not
    // the file the phone has. Read what the Mac wrote, then write the phone's text over it.
    let (mut record, mut stamp) = read_record_timed(host, &made).await?;
    if !content.trim().is_empty() && content != record.content {
        let reply = post(
            host,
            RECORD,
            json!({ "path": made, "content": content, "base": record.content }),
        )
        .await?;
        if !reply.ok() {
            return Err(reply.refused());
        }
        let (written, moved) = read_record_timed(host, &made).await?;
        record = written;
        stamp = moved;
    }
    store
        .upsert_pulled(&state.project, &record, &stamp)
        .await
        .map_err(|e| e.to_string())?;
    Ok(made)
}

/// The path of a project's hub record on the Mac.
async fn hub_path<H: Host>(host: &H, project: &str) -> Result<String, String> {
    let reply = get(
        host,
        &format!("{PROJECT}?name={}&content=false", quote(project)),
    )
    .await?;
    if !reply.ok() {
        return Err(reply.refused());
    }
    reply
        .json()
        .and_then(|body| {
            body["records"].as_array()?.iter().find_map(|record| {
                (record["kind"].as_str() == Some("project"))
                    .then(|| record["path"].as_str().map(str::to_owned))
                    .flatten()
            })
        })
        .ok_or_else(|| format!("the Mac has no hub note for {project}"))
}

/// Creates a task or a note on the Mac, answering with the path the Mac picked.
async fn create_record<H: Host>(
    host: &H,
    state: &crate::store::SyncState,
) -> Result<String, String> {
    let is_task = state.kind == "task";
    let reply = if is_task {
        post(
            host,
            TASKS,
            json!({
                "project": state.project,
                "title": state.title,
                "summary": "",
                "due": state.due,
                "priority": state.priority,
            }),
        )
        .await?
    } else {
        post(
            host,
            NOTES,
            json!({ "project": state.project, "title": state.title, "summary": "" }),
        )
        .await?
    };
    if !reply.ok() {
        return Err(reply.refused());
    }
    reply
        .json()
        .and_then(|body| {
            body["path"]
                .as_str()
                .or_else(|| body["task"]["id"].as_str())
                .map(str::to_owned)
        })
        .ok_or_else(|| "the Mac did not say which path it created".to_owned())
}

/// Replays one done toggle, creating the task on the Mac first when it has never had it.
async fn done<H: Host>(
    store: &TursoStore,
    host: &H,
    report: &mut Report,
    path: &str,
    payload: &Value,
) -> Result<(), String> {
    let done = payload["done"].as_bool().unwrap_or(false);
    let state = store
        .sync_state(path)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "its record is gone from this device".to_string())?;

    let reply = post(host, TASKS_DONE, json!({ "id": path, "done": done })).await?;
    if reply.ok() {
        return Ok(());
    }
    if !reply.missing() {
        return Err(reply.refused());
    }
    let made = create(store, host, report, path, &state, &state.content).await?;
    tick(host, &made, done).await
}

/// Sets one task's done flag: a property write on the Mac, not a whole-content one.
async fn tick<H: Host>(host: &H, path: &str, done: bool) -> Result<(), String> {
    let reply = post(host, TASKS_DONE, json!({ "id": path, "done": done })).await?;
    if reply.ok() {
        Ok(())
    } else {
        Err(reply.refused())
    }
}

/// Creates the record's project on the Mac when it is not there, so the record has somewhere to
/// live. A project that exists and is archived is left alone: the create is then refused and the
/// write stays queued, which is the honest answer for a record aimed at an archived project.
async fn ensure_project<H: Host>(host: &H, project: &str) -> Result<(), String> {
    let reply = get(host, PROJECTS).await?;
    if !reply.ok() {
        return Err(reply.refused());
    }
    let listed = reply
        .json()
        .and_then(|body| body["projects"].as_array().cloned())
        .unwrap_or_default();
    let live = listed.iter().any(|entry| {
        entry["name"].as_str() == Some(project) && !entry["archived"].as_bool().unwrap_or(false)
    });
    if live {
        return Ok(());
    }
    let reply = post(
        host,
        PROJECTS,
        json!({ "name": project, "summary": "Captured on a phone" }),
    )
    .await?;
    if reply.ok() {
        Ok(())
    } else {
        Err(reply.refused())
    }
}

// ---------------------------------------------------------------- calendar push

/// Replays one entry write: create what the Mac has never had, change what it has.
///
/// An id the phone minted ([`crate::store::LOCAL_ENTRY_ID_PREFIX`]) is a create; the Mac answers
/// with the entry it made, and the local row and its queue take that id (`rekey_calendar`). Any
/// other id is a change, and a Mac that no longer has the entry (404) gets it created again rather
/// than losing the edit.
async fn calendar_save<H: Host>(
    store: &TursoStore,
    host: &H,
    report: &mut Report,
    id: &str,
    payload: &Value,
) -> Result<(), String> {
    if id.starts_with(crate::store::LOCAL_ENTRY_ID_PREFIX) {
        let (entry, stamp) = create_entry(host, payload).await?;
        store
            .rekey_calendar(id, &entry.id)
            .await
            .map_err(|e| e.to_string())?;
        store
            .upsert_pulled_calendar(&entry, &stamp)
            .await
            .map_err(|e| e.to_string())?;
        report.created += 1;
        return Ok(());
    }
    let reply = host
        .send(
            "PATCH",
            &format!("{ENTRIES}/{}", quote(id)),
            Some(payload.to_string().as_bytes()),
        )
        .await?;
    if reply.ok() {
        if let Some((entry, stamp)) = entry_from_reply(&reply, id) {
            store
                .upsert_pulled_calendar(&entry, &stamp)
                .await
                .map_err(|e| e.to_string())?;
        }
        return Ok(());
    }
    if reply.status != 404 {
        return Err(reply.refused());
    }
    // The Mac does not have it any more: create it, and follow the id it picks.
    let (entry, stamp) = create_entry(host, payload).await?;
    store
        .rekey_calendar(id, &entry.id)
        .await
        .map_err(|e| e.to_string())?;
    store
        .upsert_pulled_calendar(&entry, &stamp)
        .await
        .map_err(|e| e.to_string())?;
    report.created += 1;
    Ok(())
}

/// Creates an entry on the Mac and answers the entry it made and its stamp.
async fn create_entry<H: Host>(
    host: &H,
    payload: &Value,
) -> Result<(CalendarEntry, String), String> {
    let reply = host
        .send("POST", ENTRIES, Some(payload.to_string().as_bytes()))
        .await?;
    if !reply.ok() {
        return Err(reply.refused());
    }
    entry_from_reply(&reply, "").ok_or_else(|| "the Mac did not say which entry it made".to_owned())
}

/// Replays one entry deletion. An entry the Mac does not have is already deleted there.
async fn calendar_delete<H: Host>(host: &H, id: &str) -> Result<(), String> {
    let reply = host
        .send("DELETE", &format!("{ENTRIES}/{}", quote(id)), None)
        .await?;
    if reply.ok() || reply.status == 404 {
        return Ok(());
    }
    Err(reply.refused())
}

// ---------------------------------------------------------------- calendar pull

/// Takes what the calendar capability changed since this replica last read it.
///
/// The feed is a cursor, not a scan: `GET /api/entries/changes?since=` answers every entry written
/// or deleted after that stamp, so a pull costs what changed rather than every entry in the
/// database. An id with an unsent local write is skipped, the rule the projects pull keeps: the
/// phone's edit is newer than anything the Mac can answer with. The cursor moves only after every
/// answer has been applied, so a pull that stops halfway is repeated rather than lost.
///
/// No sweep runs afterwards, unlike the projects pull: a feed read from 0 carries every deletion
/// there has ever been, so a local row the Mac no longer has arrives as a tombstone instead of
/// lingering until someone notices.
async fn pull_calendar<H: Host>(
    store: &TursoStore,
    host: &H,
    report: &mut Report,
) -> Result<(), String> {
    let since = store
        .cursor(crate::store::DOMAIN_CALENDAR)
        .await
        .map_err(|e| e.to_string())?
        .unwrap_or_else(|| "0".to_owned());
    let reply = get(host, &format!("{ENTRY_CHANGES}?since={since}")).await?;
    if !reply.ok() {
        return Err(reply.refused());
    }
    let Some(body) = reply.json() else {
        return Err("the Mac's change feed is not readable".to_owned());
    };
    let dirty: HashSet<String> = store
        .dirty_keys(crate::store::DOMAIN_CALENDAR)
        .await
        .map_err(|e| e.to_string())?
        .into_iter()
        .collect();
    for value in body["upserted"].as_array().cloned().unwrap_or_default() {
        let Some((entry, stamp)) = entry_from_json(&value) else {
            continue;
        };
        if dirty.contains(&entry.id) {
            continue;
        }
        let local = store
            .calendar_entry(&entry.id)
            .await
            .map_err(|e| e.to_string())?;
        let news = local.as_ref() != Some(&entry);
        store
            .upsert_pulled_calendar(&entry, &stamp)
            .await
            .map_err(|e| e.to_string())?;
        if news {
            report.pulled += 1;
        }
    }
    for tombstone in body["deleted"].as_array().cloned().unwrap_or_default() {
        let id = tombstone["id"].as_str().unwrap_or_default();
        if id.is_empty() || dirty.contains(id) {
            continue;
        }
        store
            .remove_local_calendar(id)
            .await
            .map_err(|e| e.to_string())?;
        report.removed += 1;
    }
    if let Some(cursor) = body["cursor"].as_str() {
        store
            .set_cursor(crate::store::DOMAIN_CALENDAR, cursor)
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// One entry out of the calendar capability's JSON (`Entry` in
/// `capabilities/calendar/src/model.rs`), with the stamp this replica records it at.
fn entry_from_json(value: &Value) -> Option<(CalendarEntry, String)> {
    let stamp = value["updated_at"].as_str().unwrap_or_default().to_owned();
    let entry = CalendarEntry {
        id: value["id"].as_str()?.to_owned(),
        kind: value["kind"].as_str().unwrap_or("event").to_owned(),
        commitment: value["commitment"]
            .as_str()
            .unwrap_or("possible")
            .to_owned(),
        title: value["title"].as_str().unwrap_or_default().to_owned(),
        starts_at: value["starts_at"].as_str().unwrap_or_default().to_owned(),
        ends_at: value["ends_at"].as_str().unwrap_or_default().to_owned(),
        all_day: value["all_day"].as_bool().unwrap_or(false),
        location: value["location"].as_str().map(str::to_owned),
        notes: value["notes"].as_str().map(str::to_owned),
        source: value["source"].as_str().unwrap_or("manual").to_owned(),
    };
    Some((entry, stamp))
}

/// One entry out of a create or change answer, which is the entry itself.
fn entry_from_reply(reply: &Reply, fallback_id: &str) -> Option<(CalendarEntry, String)> {
    let body = reply.json()?;
    let value = if body["entry"].is_object() {
        body["entry"].clone()
    } else {
        body
    };
    let (mut entry, stamp) = entry_from_json(&value)?;
    if entry.id.is_empty() {
        entry.id = fallback_id.to_owned();
    }
    Some((entry, stamp))
}

// ---------------------------------------------------------------- pull

async fn pull<H: Host>(store: &TursoStore, host: &H, report: &mut Report) -> Result<(), String> {
    let reply = get(host, PROJECTS).await?;
    if !reply.ok() {
        return Err(reply.refused());
    }
    let projects: Vec<String> = reply
        .json()
        .and_then(|body| body["projects"].as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter(|entry| !entry["archived"].as_bool().unwrap_or(false))
        .filter_map(|entry| entry["name"].as_str().map(str::to_owned))
        .collect();

    let dirty: HashSet<String> = store
        .dirty_paths()
        .await
        .map_err(|e| e.to_string())?
        .into_iter()
        .collect();
    let mut seen: HashSet<String> = HashSet::new();
    let mut pulled_projects: Vec<String> = Vec::new();

    for project in projects {
        let uri = format!("{PROJECT}?name={}&content=false", quote(&project));
        let reply = get(host, &uri).await?;
        if reply.status == 404 {
            // The project went between the two reads; the sweep below must not read that as
            // "every record of it was deleted".
            continue;
        }
        if !reply.ok() {
            return Err(reply.refused());
        }
        let listed = reply
            .json()
            .and_then(|body| body["records"].as_array().cloned())
            .unwrap_or_default();
        for entry in listed {
            let path = entry["path"].as_str().unwrap_or_default().to_string();
            if path.is_empty() {
                continue;
            }
            seen.insert(path.clone());
            if dirty.contains(&path) || entry["archived"].as_bool().unwrap_or(false) {
                continue;
            }
            let stamp = entry["updated_at"].as_str().unwrap_or_default().to_string();
            let local = store.sync_state(&path).await.map_err(|e| e.to_string())?;
            let known = local.as_ref().and_then(|state| state.remote_at.clone());
            if known.as_deref() == Some(stamp.as_str()) {
                continue;
            }
            let one = get(host, &format!("{RECORD}?path={}", quote(&path))).await?;
            if !one.ok() {
                // Removed between the list and the read. The sweep below decides whether the
                // local copy goes; here it simply is not in this answer.
                continue;
            }
            let Some(record) = one
                .json()
                .and_then(|body| body.get("record").cloned())
                .map(|value| record_from_json(&path, &value))
            else {
                continue;
            };
            // A stamp the Mac moved because this device just wrote to it is not news. What the
            // Mac says is, and the row keeps the new stamp either way.
            let news = match &local {
                None => true,
                Some(state) => {
                    state.content != record.content
                        || state.kind != record.kind
                        || state.title != record.title
                        || state.done != record.done
                        || state.due != record.due
                        || state.priority != record.priority
                }
            };
            // The record's own project, because a path's first folder is not it: the vault files
            // `Archive/Soma/Tasks/Reopen.md` under `Soma`, and a sweep that wrote `Archive` into
            // the local project column would file the phone's whole history under the wrong name.
            let filed = filed_under(&record);
            store
                .upsert_pulled(&filed, &record, &stamp)
                .await
                .map_err(|e| e.to_string())?;
            if news {
                report.pulled += 1;
            }
        }
        pulled_projects.push(project);
    }

    for path in store.synced_paths().await.map_err(|e| e.to_string())? {
        if seen.contains(&path) || dirty.contains(&path) {
            continue;
        }
        if !pulled_projects
            .iter()
            .any(|done| *done == project_of(&path))
        {
            continue;
        }
        store.remove_local(&path).await.map_err(|e| e.to_string())?;
        report.removed += 1;
    }
    Ok(())
}

// ---------------------------------------------------------------- plumbing

async fn get<H: Host>(host: &H, path: &str) -> Result<Reply, String> {
    host.send("GET", path, None).await
}

async fn post<H: Host>(host: &H, path: &str, body: Value) -> Result<Reply, String> {
    let bytes = serde_json::to_vec(&body).map_err(|e| format!("encode the write: {e}"))?;
    host.send("POST", path, Some(&bytes)).await
}

async fn read_record_timed<H: Host>(
    host: &H,
    path: &str,
) -> Result<(ProjectRecord, String), String> {
    let reply = get(host, &format!("{RECORD}?path={}", quote(path))).await?;
    if !reply.ok() {
        return Err(reply.refused());
    }
    let value = reply
        .json()
        .and_then(|body| body.get("record").cloned())
        .ok_or("the Mac did not answer with the record")?;
    let stamp = value["updated_at"].as_str().unwrap_or_default().to_string();
    if stamp.is_empty() {
        return Err("the Mac did not stamp the record it answered with".into());
    }
    Ok((record_from_json(path, &value), stamp))
}

fn record_from_json(path: &str, value: &Value) -> ProjectRecord {
    let kind = value["kind"].as_str().unwrap_or("note").to_string();
    let project = value["project"].as_str().unwrap_or_default().to_string();
    let title = value["title"].as_str().unwrap_or_default().to_string();
    let priority = value["priority"]
        .as_u64()
        .and_then(|p| u8::try_from(p).ok())
        .unwrap_or(2)
        .clamp(1, 3);
    ProjectRecord {
        project,
        path: path.to_string(),
        kind,
        title,
        content: value["content"].as_str().unwrap_or_default().to_string(),
        done: value["done"].as_bool().unwrap_or(false),
        due: value["due"].as_str().map(str::to_owned),
        priority,
    }
}

/// The project a record belongs to: the answer the Mac filed it under, else the path's first
/// folder, which is what this app writes for a record it made itself (`Board::task_path`).
fn filed_under(record: &ProjectRecord) -> String {
    if !record.project.trim().is_empty() {
        return record.project.clone();
    }
    project_of(&record.path)
}

/// The first folder of a path. Only a fallback: see [`filed_under`].
fn project_of(path: &str) -> String {
    path.split('/').next().unwrap_or_default().to_string()
}

/// Percent-encodes everything but the unreserved characters, so a project called `Security &
/// OSINT Lab` is one query value and not two.
fn quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(char::from(byte));
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{TursoStore, DOMAIN_CALENDAR, LOCAL_ENTRY_ID_PREFIX};
    use std::sync::Mutex;

    /// A pulled record keeps the project the Mac filed it under, and falls back on the path only
    /// when no project travelled: the vault's paths are not `<project>/…`.
    #[test]
    fn a_pulled_record_is_filed_under_the_project_the_mac_named() {
        let mut record = ProjectRecord {
            path: "Archive/Soma/Tasks/Reopen Obsidian.md".to_owned(),
            project: "Soma".to_owned(),
            kind: "task".to_owned(),
            title: "Reopen Obsidian".to_owned(),
            content: String::new(),
            done: false,
            due: None,
            priority: 2,
        };
        assert_eq!(filed_under(&record), "Soma");
        record.project.clear();
        assert_eq!(
            filed_under(&record),
            "Archive",
            "the fallback, and only that"
        );
    }

    /// A Mac that answers from a table. Each answer is used once, by method and exact path, so a
    /// test states the whole conversation and a request nobody expected fails loudly.
    #[derive(Default)]
    struct Fake {
        answers: Mutex<Vec<(String, String, u16, Value)>>,
        seen: Mutex<Vec<(String, String, Value)>>,
    }

    impl Fake {
        fn on(&self, method: &str, path: &str, status: u16, body: Value) {
            self.answers
                .lock()
                .unwrap()
                .push((method.to_string(), path.to_string(), status, body));
        }

        fn sent(&self) -> Vec<(String, String, Value)> {
            self.seen.lock().unwrap().clone()
        }

        fn bodies_to(&self, path: &str) -> Vec<Value> {
            self.sent()
                .into_iter()
                .filter(|(method, sent, _)| method == "POST" && sent == path)
                .map(|(_, _, body)| body)
                .collect()
        }
    }

    impl Host for Fake {
        async fn send(
            &self,
            method: &str,
            path: &str,
            body: Option<&[u8]>,
        ) -> Result<Reply, String> {
            let body = body
                .map(|bytes| serde_json::from_slice::<Value>(bytes).unwrap_or(Value::Null))
                .unwrap_or(Value::Null);
            self.seen
                .lock()
                .unwrap()
                .push((method.to_string(), path.to_string(), body));
            let mut answers = self.answers.lock().unwrap();
            let at = answers
                .iter()
                .position(|(m, p, _, _)| m == method && p == path);
            let Some(at) = at else {
                // `run` pulls the calendar feed after every projects pull, so a test about a note
                // states one conversation and gets an empty feed for the domain it is not about.
                // A test that cares prepares its own answer, which wins above.
                if method == "GET" && path.starts_with(ENTRY_CHANGES) {
                    let cursor = path
                        .split_once("since=")
                        .map(|(_, since)| since)
                        .unwrap_or("0");
                    return Ok(Reply {
                        status: 200,
                        body: serde_json::to_vec(&json!({
                            "upserted": [], "deleted": [], "cursor": cursor,
                        }))
                        .unwrap_or_default(),
                    });
                }
                return Err(format!("no answer prepared for {method} {path}"));
            };
            let (_, _, status, value) = answers.remove(at);
            Ok(Reply {
                status,
                body: serde_json::to_vec(&value).unwrap_or_default(),
            })
        }
    }

    /// The Mac's answer to `GET /projects/api/projects`.
    fn projects(names: &[&str]) -> Value {
        json!({
            "projects": names
                .iter()
                .map(|name| json!({ "name": name, "archived": false, "open": 0, "done": 0, "notes": 0 }))
                .collect::<Vec<_>>()
        })
    }

    fn record_json(path: &str, kind: &str, title: &str, content: &str, stamp: &str) -> Value {
        json!({
            "record": {
                "path": path, "kind": kind, "title": title, "content": content,
                "done": false, "due": Value::Null, "priority": 2, "updated_at": stamp,
            }
        })
    }

    /// A store holding one note that has never been on the Mac.
    async fn store_with_note() -> (TursoStore, String) {
        let store = TursoStore::in_memory().await.expect("store");
        let content = "# Offline\n\nTyped in a tunnel.".to_string();
        store
            .save_record("Inbox", "Inbox/Welcome.md", "note", "Welcome", &content)
            .await
            .expect("save");
        (store, content)
    }

    #[test]
    fn a_note_typed_offline_is_created_on_the_mac_and_keeps_its_text() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        rt.block_on(async {
            let (store, content) = store_with_note().await;
            let fake = Fake::default();
            // The Mac has no Inbox project, so the create makes one first.
            fake.on("GET", PROJECTS, 200, projects(&["Home"]));
            fake.on(
                "POST",
                PROJECTS,
                201,
                json!({ "project": "Inbox/Inbox.md" }),
            );
            fake.on(
                "POST",
                NOTES,
                201,
                json!({ "path": "Inbox/Welcome to Sjel.md" }),
            );
            let made = "Inbox/Welcome to Sjel.md";
            let made_uri = format!("/projects/api/record?path={}", quote(made));
            fake.on(
                "GET",
                &made_uri,
                200,
                record_json(
                    made,
                    "note",
                    "Welcome to Sjel",
                    "---\ntype: note\n---\n\n# Welcome to Sjel\n",
                    "stamp-1",
                ),
            );
            fake.on(
                "POST",
                RECORD,
                200,
                json!({ "path": "Inbox/Welcome to Sjel.md", "kind": "note" }),
            );
            fake.on(
                "GET",
                &made_uri,
                200,
                record_json(made, "note", "Welcome to Sjel", &content, "stamp-2"),
            );
            // The pull that follows finds the record it just pushed, unchanged.
            fake.on("GET", PROJECTS, 200, projects(&["Home", "Inbox"]));
            fake.on(
                "GET",
                "/projects/api/project?name=Home&content=false",
                200,
                json!({ "records": [] }),
            );
            fake.on(
                "GET",
                "/projects/api/project?name=Inbox&content=false",
                200,
                json!({
                    "records": [{
                        "path": "Inbox/Welcome to Sjel.md", "kind": "note",
                        "title": "Welcome to Sjel", "done": false, "due": Value::Null,
                        "priority": 2, "updated_at": "stamp-2",
                    }]
                }),
            );

            let report = run(&store, &fake).await.expect("sync");
            assert_eq!(report.pushed, 1, "{report:?}");
            assert_eq!(report.created, 1, "{report:?}");
            assert_eq!(report.conflicts, 0, "{report:?}");
            assert_eq!(report.pulled, 0, "the record it just wrote is in step");
            assert!(report.kept.is_empty(), "{report:?}");
            assert_eq!(store.pending_mutation_count().await.unwrap(), 0);

            // The record the person typed is on the Mac under the path the Mac chose, with the
            // phone's text rather than the vault's template.
            let writes = fake.bodies_to(RECORD);
            assert_eq!(writes.len(), 1, "{writes:?}");
            assert_eq!(writes[0]["content"], json!(content));
            assert_eq!(writes[0]["path"], json!("Inbox/Welcome to Sjel.md"));

            let moved = store
                .fetch_record("Inbox/Welcome to Sjel.md")
                .await
                .unwrap()
                .expect("the row followed the Mac's path");
            assert_eq!(moved.content, content);
            assert_eq!(moved.title, "Welcome to Sjel");
            assert!(store
                .fetch_record("Inbox/Welcome.md")
                .await
                .unwrap()
                .is_none());
        });
    }

    #[test]
    fn a_note_edited_on_both_sides_replays_the_phones_text() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        rt.block_on(async {
            let store = TursoStore::in_memory().await.expect("store");
            // A record this device already agrees with, then edited offline.
            store
                .upsert_pulled(
                    "Inbox",
                    &ProjectRecord {
                        project: String::new(),
                        path: "Inbox/Plan.md".into(),
                        kind: "note".into(),
                        title: "Plan".into(),
                        content: "old".into(),
                        done: false,
                        due: None,
                        priority: 2,
                    },
                    "stamp-old",
                )
                .await
                .expect("seed");
            store
                .save_record("Inbox", "Inbox/Plan.md", "note", "Plan", "offline edit")
                .await
                .expect("edit");

            let fake = Fake::default();
            fake.on(
                "POST",
                RECORD,
                409,
                json!({
                    "error": "the note changed since it was read; nothing was saved",
                    "code": "stale",
                    "current": "mac edit",
                }),
            );
            fake.on(
                "POST",
                RECORD,
                200,
                json!({ "path": "Inbox/Plan.md", "kind": "note" }),
            );
            fake.on("GET", PROJECTS, 200, projects(&[]));

            let report = run(&store, &fake).await.expect("sync");
            assert_eq!(report.pushed, 1, "{report:?}");
            assert_eq!(report.conflicts, 1, "{report:?}");
            assert_eq!(report.kept.len(), 0, "{report:?}");

            let writes = fake.bodies_to(RECORD);
            assert_eq!(writes.len(), 2, "{writes:?}");
            assert_eq!(writes[0]["base"], json!("old"));
            assert_eq!(
                writes[1]["base"],
                json!("mac edit"),
                "replayed over the Mac"
            );
            assert_eq!(writes[1]["content"], json!("offline edit"));
            assert!(report.summary().contains("this device's text won"));
        });
    }

    #[test]
    fn a_task_typed_and_ticked_offline_is_created_on_the_mac_and_left_ticked() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        rt.block_on(async {
            let store = TursoStore::in_memory().await.expect("store");
            store
                .save_record("Inbox", "Inbox/Tasks/Buy.md", "task", "Buy milk", "")
                .await
                .expect("save");
            store
                .set_task_done("Inbox/Tasks/Buy.md", true)
                .await
                .expect("tick");

            let made = "Inbox/Tasks/Buy milk.md";
            let made_uri = format!("/projects/api/record?path={}", quote(made));
            let fake = Fake::default();
            fake.on("GET", PROJECTS, 200, projects(&[]));
            fake.on(
                "POST",
                PROJECTS,
                201,
                json!({ "project": "Inbox/Inbox.md" }),
            );
            fake.on(
                "POST",
                TASKS,
                201,
                json!({ "task": { "id": made, "kind": "task", "done": false } }),
            );
            fake.on(
                "POST",
                TASKS_DONE,
                200,
                json!({ "task": { "id": made, "done": true } }),
            );
            fake.on(
                "GET",
                &made_uri,
                200,
                record_json(
                    made,
                    "task",
                    "Buy milk",
                    "---\ntype: task\n---\n",
                    "stamp-1",
                ),
            );
            // The queued tick replays too, and the tick moved the Mac's stamp, so the pull reads
            // the record back rather than trusting what it wrote.
            fake.on(
                "POST",
                TASKS_DONE,
                200,
                json!({ "task": { "id": made, "done": true } }),
            );
            fake.on("GET", PROJECTS, 200, projects(&["Inbox"]));
            fake.on(
                "GET",
                "/projects/api/project?name=Inbox&content=false",
                200,
                json!({
                    "records": [{
                        "path": made, "kind": "task", "title": "Buy milk",
                        "done": true, "due": Value::Null, "priority": 2, "updated_at": "stamp-2",
                    }]
                }),
            );
            let mut done_record = record_json(made, "task", "Buy milk", "", "stamp-2");
            done_record["record"]["done"] = json!(true);
            fake.on("GET", &made_uri, 200, done_record);

            let report = run(&store, &fake).await.expect("sync");
            assert_eq!(report.pushed, 2, "{report:?}");
            assert_eq!(report.created, 1, "{report:?}");
            assert_eq!(report.pulled, 1, "the tick's new stamp: {report:?}");
            assert!(report.kept.is_empty(), "{report:?}");

            let ticks = fake.bodies_to(TASKS_DONE);
            assert_eq!(ticks.len(), 2, "{ticks:?}");
            for tick in &ticks {
                assert_eq!(tick["id"], json!(made), "ticked after the Mac named it");
                assert_eq!(tick["done"], json!(true));
            }
            assert_eq!(
                fake.bodies_to(TASKS)[0]["title"],
                json!("Buy milk"),
                "the Mac names the task from the title the phone had"
            );
            assert!(
                store
                    .fetch_record(made)
                    .await
                    .unwrap()
                    .expect("the row followed the Mac")
                    .done
            );
        });
    }

    #[test]
    fn a_task_the_mac_never_had_is_created_when_it_is_ticked() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        rt.block_on(async {
            let store = TursoStore::in_memory().await.expect("store");
            // Blackfold: a record this replica took from the Mac, and the Mac then lost. Its
            // content is empty here so the create has no note body to write back.
            store
                .upsert_pulled(
                    "Inbox",
                    &ProjectRecord {
                        project: String::new(),
                        path: "Inbox/Tasks/Call.md".into(),
                        kind: "task".into(),
                        title: "Call".into(),
                        content: String::new(),
                        done: false,
                        due: None,
                        priority: 2,
                    },
                    "stamp-0",
                )
                .await
                .expect("seed");
            store
                .set_task_done("Inbox/Tasks/Call.md", true)
                .await
                .expect("tick");

            let made = "Inbox/Tasks/Call back.md";
            let made_uri = format!("/projects/api/record?path={}", quote(made));
            let fake = Fake::default();
            // The toggle names a record the Mac does not have, in the capability's own words.
            fake.on(
                "POST",
                TASKS_DONE,
                400,
                json!({ "error": "no task \"Inbox/Tasks/Call.md\"" }),
            );
            fake.on("GET", PROJECTS, 200, projects(&[]));
            fake.on(
                "POST",
                PROJECTS,
                201,
                json!({ "project": "Inbox/Inbox.md" }),
            );
            fake.on(
                "POST",
                TASKS,
                201,
                json!({ "task": { "id": made, "kind": "task", "done": false } }),
            );
            fake.on(
                "POST",
                TASKS_DONE,
                200,
                json!({ "task": { "id": made, "done": true } }),
            );
            fake.on(
                "GET",
                &made_uri,
                200,
                record_json(made, "task", "Call", "", "stamp-1"),
            );
            fake.on("GET", PROJECTS, 200, projects(&[]));

            let report = run(&store, &fake).await.expect("sync");
            assert_eq!(report.pushed, 1, "{report:?}");
            assert_eq!(report.created, 1, "created rather than kept: {report:?}");
            assert!(report.kept.is_empty(), "{report:?}");
            let ticks = fake.bodies_to(TASKS_DONE);
            assert_eq!(ticks.len(), 2, "{ticks:?}");
            assert_eq!(ticks[1]["id"], json!(made));
            assert_eq!(ticks[1]["done"], json!(true));
        });
    }

    #[test]
    fn a_pull_takes_the_macs_changes_and_leaves_an_unsent_edit_alone() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        rt.block_on(async {
            let store = TursoStore::in_memory().await.expect("store");
            let quiet = ProjectRecord {
            project: String::new(),
                path: "Inbox/Quiet.md".into(),
                kind: "note".into(),
                title: "Quiet".into(),
                content: "as agreed".into(),
                done: false,
                due: None,
                priority: 2,
            };
            let dirty = ProjectRecord {
            project: String::new(),
                path: "Inbox/Dirty.md".into(),
                kind: "note".into(),
                title: "Dirty".into(),
                content: "as agreed".into(),
                done: false,
                due: None,
                priority: 2,
            };
            let gone = ProjectRecord {
            project: String::new(),
                path: "Inbox/Gone.md".into(),
                kind: "note".into(),
                title: "Gone".into(),
                content: "deleted on the Mac".into(),
                done: false,
                due: None,
                priority: 2,
            };
            for record in [&quiet, &dirty, &gone] {
                store.upsert_pulled("Inbox", record, "stamp-1").await.expect("seed");
            }
            // A write this device has not managed to send yet.
            store
                .set_task_done("Inbox/Dirty.md", true)
                .await
                .expect("edit");

            let fake = Fake::default();
            // No queued write the Mac accepts: the push stops and keeps it, the pull continues.
            fake.on("POST", TASKS_DONE, 503, json!({ "error": "not now" }));
            fake.on("GET", PROJECTS, 200, projects(&["Inbox"]));
            fake.on(
                "GET",
                "/projects/api/project?name=Inbox&content=false",
                200,
                json!({
                    "records": [
                        { "path": "Inbox/Quiet.md", "kind": "note", "title": "Quiet",
                          "done": false, "due": Value::Null, "priority": 2, "updated_at": "stamp-2" },
                        { "path": "Inbox/Dirty.md", "kind": "note", "title": "Dirty",
                          "done": false, "due": Value::Null, "priority": 2, "updated_at": "stamp-9" },
                    ]
                }),
            );
            fake.on(
                "GET",
                &format!("/projects/api/record?path={}", quote("Inbox/Quiet.md")),
                200,
                record_json("Inbox/Quiet.md", "note", "Quiet", "changed on the Mac", "stamp-2"),
            );

            let report = run(&store, &fake).await.expect("sync");
            assert_eq!(report.pulled, 1, "only the record that moved: {report:?}");
            assert_eq!(report.removed, 1, "the Mac no longer has Gone: {report:?}");
            assert_eq!(report.pushed, 0, "{report:?}");
            assert_eq!(report.kept.len(), 1, "{report:?}");

            let quiet = store.fetch_record("Inbox/Quiet.md").await.unwrap().unwrap();
            assert_eq!(quiet.content, "changed on the Mac");
            let dirty = store.fetch_record("Inbox/Dirty.md").await.unwrap().unwrap();
            assert_eq!(dirty.content, "as agreed", "the unsent edit is not overwritten");
            assert!(store.fetch_record("Inbox/Gone.md").await.unwrap().is_none());
            assert_eq!(store.pending_mutation_count().await.unwrap(), 1);
        });
    }

    #[test]
    fn a_record_that_only_ever_lived_on_the_phone_is_never_swept_away() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        rt.block_on(async {
            let (store, _) = store_with_note().await;
            let fake = Fake::default();
            fake.on(
                "POST",
                NOTES,
                400,
                json!({ "error": "no live project named \"Inbox\"" }),
            );
            fake.on("GET", PROJECTS, 200, projects(&[]));
            fake.on(
                "POST",
                PROJECTS,
                400,
                json!({ "error": "that project could not be created" }),
            );
            fake.on("GET", PROJECTS, 200, projects(&[]));

            let report = run(&store, &fake).await.expect("sync");
            assert_eq!(report.pushed, 0, "{report:?}");
            assert_eq!(report.removed, 0, "{report:?}");
            assert_eq!(report.kept.len(), 1, "{report:?}");
            assert!(
                store
                    .fetch_record("Inbox/Welcome.md")
                    .await
                    .unwrap()
                    .is_some(),
                "the only copy stays"
            );
            assert_eq!(store.pending_mutation_count().await.unwrap(), 1);
        });
    }

    #[test]
    fn a_queued_write_the_queue_cannot_read_is_kept_with_its_reason() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        rt.block_on(async {
            let store = TursoStore::in_memory().await.expect("store");
            store
                .save_record("Inbox", "Inbox/One.md", "note", "One", "text")
                .await
                .expect("save");
            // A row no build of this app writes, oldest so the push meets it first.
            let conn = store.connection().await.expect("conn");
            conn.execute(
                "INSERT INTO _offline_mutations (id, path, action, payload) VALUES (0, ?1, ?2, ?3)",
                libsql::params!["Inbox/One.md", "save_record", "{not json"],
            )
            .await
            .expect("queue");

            let fake = Fake::default();
            fake.on("GET", PROJECTS, 200, projects(&[]));

            let report = run(&store, &fake).await.expect("sync");
            assert_eq!(report.pushed, 0, "{report:?}");
            assert_eq!(report.kept.len(), 1, "{report:?}");
            assert!(
                report.kept[0].contains("cannot read"),
                "the reason travels with it: {report:?}"
            );
            assert!(report.summary().contains("Kept for later"));
            assert_eq!(store.pending_mutation_count().await.unwrap(), 2);
            assert!(
                store.fetch_record("Inbox/One.md").await.unwrap().is_some(),
                "the phone keeps its copy"
            );
        });
    }

    // ------------------------------------------------------------ calendar

    /// One entry as the calendar capability answers it: `Entry` flattened into `EntryListItem`, so
    /// the entry's own fields sit at the top level beside the class.
    fn entry_json(id: &str, title: &str, stamp: &str) -> Value {
        json!({
            "id": id,
            "kind": "event",
            "commitment": "planned",
            "title": title,
            "starts_at": "2026-10-24T09:00:00",
            "ends_at": "2026-10-24T10:00:00",
            "all_day": false,
            "location": Value::Null,
            "notes": Value::Null,
            "attendees": [],
            "source": "manual",
            "external_id": Value::Null,
            "rhythm_id": Value::Null,
            "updated_at": stamp,
            "created_at": stamp,
            "data_class": "personal",
        })
    }

    /// The same entry as this replica holds it.
    fn local_entry(id: &str, title: &str) -> CalendarEntry {
        CalendarEntry {
            id: id.to_string(),
            kind: "event".into(),
            commitment: "planned".into(),
            title: title.into(),
            starts_at: "2026-10-24T09:00:00".into(),
            ends_at: "2026-10-24T10:00:00".into(),
            all_day: false,
            location: None,
            notes: None,
            source: "manual".into(),
        }
    }

    /// `GET /calendar/api/entries/changes` at one cursor. Stamps and tombstones are the integer
    /// milliseconds the feed's cursor counts in.
    fn feed(upserted: Vec<Value>, deleted: &[(&str, &str)], cursor: &str) -> Value {
        json!({
            "upserted": upserted,
            "deleted": deleted
                .iter()
                .map(|(id, at)| json!({ "id": id, "deleted_at": at }))
                .collect::<Vec<_>>(),
            "cursor": cursor,
        })
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
    }

    #[test]
    fn an_entry_made_on_the_phone_is_created_and_takes_the_macs_id() {
        runtime().block_on(async {
            let store = TursoStore::in_memory().await.expect("store");
            let local = format!("{LOCAL_ENTRY_ID_PREFIX}7f3a");
            store
                .save_calendar_entry(&local_entry(&local, "Dentist"))
                .await
                .expect("save");

            let fake = Fake::default();
            fake.on(
                "POST",
                ENTRIES,
                201,
                entry_json("calendar:1f2e", "Dentist", "1761000001000"),
            );
            fake.on("GET", PROJECTS, 200, projects(&[]));

            let report = run(&store, &fake).await.expect("sync");
            assert_eq!(report.created, 1, "{report:?}");
            assert_eq!(report.pushed, 1, "{report:?}");
            assert!(report.kept.is_empty(), "{report:?}");
            assert_eq!(store.pending_mutation_count().await.unwrap(), 0);

            // The row and the write queued for it both moved to the id the Mac assigned.
            let moved = store
                .calendar_entry("calendar:1f2e")
                .await
                .unwrap()
                .expect("the row followed the Mac's id");
            assert_eq!(moved.title, "Dentist");
            assert!(store.calendar_entry(&local).await.unwrap().is_none());

            // The create carried what the person typed, and no id the Mac has not made.
            let writes = fake.bodies_to(ENTRIES);
            assert_eq!(writes.len(), 1, "{writes:?}");
            assert_eq!(writes[0]["title"], json!("Dentist"));
            assert!(writes[0].get("id").is_none(), "{writes:?}");
        });
    }

    #[test]
    fn an_entry_the_mac_already_has_is_changed_rather_than_created() {
        runtime().block_on(async {
            let store = TursoStore::in_memory().await.expect("store");
            let id = "calendar:1f2e";
            store
                .save_calendar_entry(&local_entry(id, "Dentist at nine"))
                .await
                .expect("save");

            let fake = Fake::default();
            let one = format!("{ENTRIES}/calendar%3A1f2e");
            fake.on(
                "PATCH",
                &one,
                200,
                entry_json(id, "Dentist at nine", "1761000002000"),
            );
            fake.on("GET", PROJECTS, 200, projects(&[]));

            let report = run(&store, &fake).await.expect("sync");
            assert_eq!(report.pushed, 1, "{report:?}");
            assert_eq!(report.created, 0, "the Mac already had it");
            assert!(
                fake.bodies_to(ENTRIES).is_empty(),
                "no second entry was made"
            );
            assert_eq!(
                store.calendar_entry(id).await.unwrap().unwrap().title,
                "Dentist at nine"
            );
            assert_eq!(store.pending_mutation_count().await.unwrap(), 0);
        });
    }

    #[test]
    fn an_entry_the_mac_no_longer_has_is_created_again_with_the_phones_text() {
        runtime().block_on(async {
            let store = TursoStore::in_memory().await.expect("store");
            let id = "calendar:1f2e";
            store
                .save_calendar_entry(&local_entry(id, "Dentist at nine"))
                .await
                .expect("save");

            let fake = Fake::default();
            fake.on(
                "PATCH",
                &format!("{ENTRIES}/calendar%3A1f2e"),
                404,
                json!({ "error": "no such entry" }),
            );
            fake.on(
                "POST",
                ENTRIES,
                201,
                entry_json("calendar:9a41", "Dentist at nine", "1761000003000"),
            );
            fake.on("GET", PROJECTS, 200, projects(&[]));

            let report = run(&store, &fake).await.expect("sync");
            assert_eq!(report.created, 1, "{report:?}");
            assert!(report.kept.is_empty(), "{report:?}");

            // The edit survived the Mac losing the entry: it is on the entry the Mac made instead.
            let made = store
                .calendar_entry("calendar:9a41")
                .await
                .unwrap()
                .expect("the entry the Mac made instead");
            assert_eq!(made.title, "Dentist at nine");
            assert!(store.calendar_entry(id).await.unwrap().is_none());
            assert_eq!(store.pending_mutation_count().await.unwrap(), 0);
        });
    }

    #[test]
    fn a_pull_takes_the_macs_entries_and_a_tombstone_after_them() {
        runtime().block_on(async {
            let store = TursoStore::in_memory().await.expect("store");
            store
                .upsert_pulled_calendar(&local_entry("calendar:old", "Gone"), "1761000000000")
                .await
                .expect("seed");

            let fake = Fake::default();
            let first = format!("{ENTRY_CHANGES}?since=0");
            fake.on(
                "GET",
                &first,
                200,
                feed(
                    vec![entry_json("calendar:1f2e", "Dentist", "1761000001000")],
                    &[("calendar:old", "1761000001000")],
                    "1761000001000",
                ),
            );

            let mut report = Report::default();
            pull_calendar(&store, &fake, &mut report)
                .await
                .expect("pull");
            assert_eq!(report.pulled, 1, "{report:?}");
            assert_eq!(report.removed, 1, "{report:?}");
            assert!(store
                .calendar_entry("calendar:1f2e")
                .await
                .unwrap()
                .is_some());
            assert!(
                store
                    .calendar_entry("calendar:old")
                    .await
                    .unwrap()
                    .is_none(),
                "a feed read from 0 carries every deletion there has been"
            );
            assert_eq!(
                store.cursor(DOMAIN_CALENDAR).await.unwrap().as_deref(),
                Some("1761000001000")
            );

            // The next pull asks from where that one ended, so nothing is read twice.
            let second = format!("{ENTRY_CHANGES}?since=1761000001000");
            fake.on("GET", &second, 200, feed(Vec::new(), &[], "1761000001000"));
            let mut report = Report::default();
            pull_calendar(&store, &fake, &mut report)
                .await
                .expect("second pull");
            assert_eq!(report.pulled, 0, "{report:?}");
            assert_eq!(report.removed, 0, "{report:?}");
        });
    }

    #[test]
    fn an_unsent_edit_is_not_written_over_by_the_feed() {
        runtime().block_on(async {
            let store = TursoStore::in_memory().await.expect("store");
            let id = "calendar:1f2e";
            store
                .save_calendar_entry(&local_entry(id, "Mine, typed in a tunnel"))
                .await
                .expect("save");

            let fake = Fake::default();
            fake.on(
                "GET",
                &format!("{ENTRY_CHANGES}?since=0"),
                200,
                feed(
                    vec![entry_json(id, "Theirs", "1761000004000")],
                    &[],
                    "1761000004000",
                ),
            );

            let mut report = Report::default();
            pull_calendar(&store, &fake, &mut report)
                .await
                .expect("pull");
            assert_eq!(report.pulled, 0, "{report:?}");
            assert_eq!(
                store.calendar_entry(id).await.unwrap().unwrap().title,
                "Mine, typed in a tunnel",
                "the phone's unsent edit is newer than anything the Mac can answer with"
            );
            assert_eq!(
                store.cursor(DOMAIN_CALENDAR).await.unwrap().as_deref(),
                Some("1761000004000"),
                "the edit travels, the cursor still moves"
            );
        });
    }

    #[test]
    fn an_entry_this_replica_already_holds_is_not_news() {
        runtime().block_on(async {
            let store = TursoStore::in_memory().await.expect("store");
            let id = "calendar:1f2e";
            store
                .upsert_pulled_calendar(&local_entry(id, "Dentist"), "1761000001000")
                .await
                .expect("seed");

            let fake = Fake::default();
            fake.on(
                "GET",
                &format!("{ENTRY_CHANGES}?since=0"),
                200,
                feed(
                    vec![entry_json(id, "Dentist", "1761000001000")],
                    &[],
                    "1761000001000",
                ),
            );

            let mut report = Report::default();
            pull_calendar(&store, &fake, &mut report)
                .await
                .expect("pull");
            assert_eq!(report.pulled, 0, "the row already said this: {report:?}");
        });
    }
}
