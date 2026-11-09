//! Reviewed, conditional cleanup of exact non-recurring Google event copies.

use std::collections::{BTreeMap, HashSet};
use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::auth::{http_client, SyncResult, CALENDAR_API};
use super::transport::{encode_segment, fail_for};
use super::{access_token, Settings};

const USAGE: &str = "sjel calendar dedupe audit FROM TO PLAN.json\nsjel calendar dedupe apply PLAN.json --apply\nAudit is read-only; dates are YYYY-MM-DD, at most 90 days. Apply only a reviewed private plan.";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Plan {
    version: u8,
    calendar_id: String,
    from: String,
    to: String,
    fetched: usize,
    groups: Vec<Vec<Value>>,
}

fn field<'a>(event: &'a Value, name: &str) -> SyncResult<&'a str> {
    event[name]
        .as_str()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("event needs {name}"))
}

fn content(event: &Value) -> Option<Value> {
    if event["status"] != "confirmed"
        || event["organizer"]["self"] != true
        || ["recurrence", "recurringEventId", "originalStartTime"]
            .iter()
            .any(|name| event.get(name).is_some())
        || event.get("eventType").is_some_and(|kind| kind != "default")
        || ["id", "etag", "created"]
            .iter()
            .any(|name| field(event, name).is_err())
        || !event["start"].is_object()
        || !event["end"].is_object()
    {
        return None;
    }
    let mut object = event.as_object()?.clone();
    for name in [
        "id", "etag", "created", "updated", "iCalUID", "htmlLink", "kind",
    ] {
        object.remove(name);
    }
    Some(Value::Object(object))
}

fn groups(events: Vec<Value>) -> Vec<Vec<Value>> {
    let mut by_content: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for event in events {
        if let Some(key) = content(&event) {
            by_content.entry(key.to_string()).or_default().push(event);
        }
    }
    by_content
        .into_values()
        .filter(|copies| copies.len() > 1)
        .map(|mut copies| {
            copies.sort_by(|left, right| {
                (left["created"].as_str(), left["id"].as_str())
                    .cmp(&(right["created"].as_str(), right["id"].as_str()))
            });
            copies
        })
        .collect()
}

impl Plan {
    fn validate(&self, calendar_id: &str) -> SyncResult<()> {
        if self.version != 1 || self.calendar_id != calendar_id {
            return Err("plan version or configured calendar does not match".into());
        }
        super::import::review_window(&self.from, &self.to)?;
        let mut ids = HashSet::new();
        for group in &self.groups {
            if group.len() < 2 || groups(group.clone()) != vec![group.clone()] {
                return Err("plan contains a non-identical, recurring, or reordered group".into());
            }
            for event in group {
                if !ids.insert(field(event, "id")?) {
                    return Err("plan repeats an event id".into());
                }
            }
        }
        Ok(())
    }
}

trait CleanupApi {
    fn get(&self, id: &str) -> SyncResult<Option<Value>>;
    fn delete(&self, id: &str, etag: &str) -> SyncResult<()>;
}

struct GoogleCleanup<'a> {
    settings: &'a Settings,
}

impl GoogleCleanup<'_> {
    fn url(&self, id: &str) -> String {
        format!(
            "{CALENDAR_API}/{}/events/{}",
            encode_segment(&self.settings.calendar_id),
            encode_segment(id)
        )
    }

    fn list(&self, from: &str, to: &str) -> SyncResult<Vec<Value>> {
        let (from, to) = super::import::review_window(from, to)?;
        let mut events = Vec::new();
        let mut cursor = String::new();
        let mut seen = HashSet::new();
        loop {
            let response = http_client()?
                .get(format!(
                    "{CALENDAR_API}/{}/events",
                    encode_segment(&self.settings.calendar_id)
                ))
                .bearer_auth(access_token(&self.settings.google.env_path())?)
                .query(&[
                    ("timeMin", from.as_str()),
                    ("timeMax", to.as_str()),
                    ("singleEvents", "false"),
                    ("showDeleted", "false"),
                    ("maxResults", "2500"),
                    ("pageToken", cursor.as_str()),
                ])
                .send()
                .map_err(|_| "events.list could not reach Google".to_string())?;
            if !response.status().is_success() {
                return Err(fail_for(response.status(), "events.list"));
            }
            let page: Value = response
                .json()
                .map_err(|_| "events.list returned invalid JSON".to_string())?;
            let items = page["items"]
                .as_array()
                .ok_or("events.list omitted items")?;
            events.extend(items.iter().cloned());
            if events.len() > self.settings.google.max_events {
                return Err(
                    "audit exceeds google.max_events; narrow the window; no plan written".into(),
                );
            }
            match page.get("nextPageToken").and_then(Value::as_str) {
                None => return Ok(events),
                Some(next) if !next.is_empty() && seen.insert(next.to_owned()) => {
                    cursor = next.to_owned()
                }
                _ => {
                    return Err(
                        "events.list repeated or invalid page token; no plan written".into(),
                    );
                }
            }
        }
    }
}

impl CleanupApi for GoogleCleanup<'_> {
    fn get(&self, id: &str) -> SyncResult<Option<Value>> {
        let response = http_client()?
            .get(self.url(id))
            .bearer_auth(access_token(&self.settings.google.env_path())?)
            .send()
            .map_err(|_| "events.get could not reach Google".to_string())?;
        if matches!(response.status().as_u16(), 404 | 410) {
            return Ok(None);
        }
        if !response.status().is_success() {
            return Err(fail_for(response.status(), "events.get"));
        }
        response
            .json()
            .map(Some)
            .map_err(|_| "events.get returned invalid JSON".into())
    }

    fn delete(&self, id: &str, etag: &str) -> SyncResult<()> {
        let response = http_client()?
            .delete(self.url(id))
            .bearer_auth(access_token(&self.settings.google.env_path())?)
            .header(reqwest::header::IF_MATCH, etag)
            .query(&[("sendUpdates", "none")])
            .send()
            .map_err(|_| {
                "events.delete could not reach Google; inspect and rerun the plan".to_string()
            })?;
        if response.status() != reqwest::StatusCode::NO_CONTENT {
            return Err(fail_for(response.status(), "conditional events.delete"));
        }
        Ok(())
    }
}

fn current(api: &dyn CleanupApi, planned: &Value) -> SyncResult<()> {
    let fresh = api
        .get(field(planned, "id")?)?
        .ok_or("planned event is missing; audit again")?;
    if fresh["etag"] != planned["etag"]
        || content(&fresh) != content(planned)
        || content(&fresh).is_none()
    {
        return Err("planned event changed; audit again before deleting".into());
    }
    Ok(())
}

fn apply(
    api: &dyn CleanupApi,
    plan: &Plan,
    mut record: impl FnMut(&str, bool) -> SyncResult<()>,
) -> SyncResult<usize> {
    // Preflight the entire selection before the first irreversible call.
    for group in &plan.groups {
        for event in group {
            current(api, event)?;
        }
    }
    let mut deleted = 0;
    for group in &plan.groups {
        for event in &group[1..] {
            current(api, &group[0])?;
            current(api, event)?;
            let id = field(event, "id")?;
            record(id, false)?;
            api.delete(id, field(event, "etag")?)?;
            record(id, true)?;
            deleted += 1;
        }
    }
    Ok(deleted)
}

fn private_file(path: &Path) -> SyncResult<std::fs::File> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|error| format!("cannot create private output {}: {error}", path.display()))
}

/// Run an explicit audit or apply command. No command defaults to mutation.
///
/// # Errors
/// Invalid arguments, unsafe plans, stale revisions, network or private output failures.
pub fn command(args: &[String]) -> SyncResult<()> {
    if !matches!(args, [verb, _, _, _] if verb == "audit")
        && !matches!(args, [verb, _, flag] if verb == "apply" && flag == "--apply")
    {
        return Err(USAGE.into());
    }
    let settings = Settings::resolve(&crate::config::Config::load())?;
    let api = GoogleCleanup {
        settings: &settings,
    };
    match args {
        [verb, from, to, path] if verb == "audit" => {
            let events = api.list(from, to)?;
            let plan = Plan {
                version: 1,
                calendar_id: settings.calendar_id.clone(),
                from: from.clone(),
                to: to.clone(),
                fetched: events.len(),
                groups: groups(events),
            };
            plan.validate(&settings.calendar_id)?;
            let mut output = private_file(Path::new(path))?;
            serde_json::to_writer_pretty(&mut output, &plan).map_err(|error| error.to_string())?;
            output.sync_all().map_err(|error| error.to_string())?;
            println!(
                "{} events audited, {} exact groups, {} surplus copies; private plan written, nothing deleted",
                plan.fetched,
                plan.groups.len(),
                plan.groups
                    .iter()
                    .map(|group| group.len() - 1)
                    .sum::<usize>()
            );
            Ok(())
        }
        [verb, path, flag] if verb == "apply" && flag == "--apply" => {
            let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
            let plan: Plan = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
            plan.validate(&settings.calendar_id)?;
            let journal_path = format!("{path}.{}.jsonl", super::auth::now_secs());
            let mut journal = private_file(Path::new(&journal_path))?;
            let deleted = apply(&api, &plan, |id, confirmed| {
                writeln!(
                    journal,
                    "{}",
                    serde_json::json!({"calendar_id": plan.calendar_id, "event_id": id, "phase": if confirmed { "deleted" } else { "attempt" }})
                )
                .map_err(|error| error.to_string())?;
                journal.sync_all().map_err(|error| error.to_string())
            })?;
            println!("{deleted} conditional deletions recorded; rerun audit before any retry");
            Ok(())
        }
        _ => Err(USAGE.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::cell::RefCell;

    fn event(id: &str) -> Value {
        json!({"id": id, "etag": format!("\"{id}\""), "created": "2026-01-01T00:00:00Z", "status": "confirmed", "summary": "Event", "start": {"date": "2026-02-01"}, "end": {"date": "2026-02-02"}, "organizer": {"self": true}})
    }

    fn plan() -> Plan {
        Plan {
            version: 1,
            calendar_id: "primary".into(),
            from: "2026-02-01".into(),
            to: "2026-03-01".into(),
            fetched: 2,
            groups: groups(vec![event("a"), event("b")]),
        }
    }

    struct Fake {
        events: RefCell<BTreeMap<String, Value>>,
        deleted: RefCell<Vec<String>>,
    }
    impl Fake {
        fn new() -> Self {
            Self {
                events: RefCell::new([("a".into(), event("a")), ("b".into(), event("b"))].into()),
                deleted: RefCell::new(Vec::new()),
            }
        }
    }
    impl CleanupApi for Fake {
        fn get(&self, id: &str) -> SyncResult<Option<Value>> {
            Ok(self.events.borrow().get(id).cloned())
        }
        fn delete(&self, id: &str, etag: &str) -> SyncResult<()> {
            assert_eq!(self.events.borrow()[id]["etag"], etag);
            self.deleted.borrow_mut().push(id.into());
            self.events.borrow_mut().remove(id);
            Ok(())
        }
    }

    #[test]
    fn different_content_and_recurrences_never_group() {
        for (key, value) in [
            ("description", json!("unique")),
            ("location", json!("elsewhere")),
            ("attendees", json!([{"email": "a@example.invalid"}])),
            ("attachments", json!([{"fileId": "unique"}])),
            ("recurrence", json!(["RRULE:FREQ=DAILY"])),
            ("recurringEventId", json!("series")),
            ("originalStartTime", json!({"date":"2026-02-01"})),
            ("eventType", json!("outOfOffice")),
            ("organizer", json!({"self":false})),
        ] {
            let mut different = event("b");
            different[key] = value;
            assert!(groups(vec![event("a"), different]).is_empty(), "{key}");
        }
    }

    #[test]
    fn keeps_oldest_then_id_and_validates_whole_plan() {
        let mut older = event("z");
        older["created"] = json!("2025-01-01T00:00:00Z");
        assert_eq!(groups(vec![event("a"), older])[0][0]["id"], "z");
        let mut plan = plan();
        assert!(plan.validate("primary").is_ok());
        assert!(plan.validate("other").is_err());
        plan.groups[0].reverse();
        assert!(plan.validate("primary").is_err());
        plan.groups = vec![vec![event("a"), event("a")]];
        assert!(plan.validate("primary").is_err());
    }

    #[test]
    fn stale_revision_prevents_all_deletes() {
        let api = Fake::new();
        api.events.borrow_mut().get_mut("b").unwrap()["etag"] = json!("changed");
        assert!(apply(&api, &plan(), |_, _| Ok(())).is_err());
        assert!(api.deleted.borrow().is_empty());
    }

    #[test]
    fn journal_failure_before_delete_leaves_events_untouched() {
        let api = Fake::new();
        assert!(apply(&api, &plan(), |_, _| Err("disk full".into())).is_err());
        assert!(api.deleted.borrow().is_empty());
    }

    #[test]
    fn missing_survivor_prevents_all_deletes() {
        let api = Fake::new();
        api.events.borrow_mut().remove("a");
        assert!(apply(&api, &plan(), |_, _| Ok(())).is_err());
        assert!(api.deleted.borrow().is_empty());
    }

    #[test]
    fn private_outputs_are_owner_only_and_never_overwritten() {
        use std::os::unix::fs::PermissionsExt;
        let path = std::env::temp_dir().join(format!("calendar-plan-{}.json", std::process::id()));
        let file = private_file(&path).unwrap();
        assert_eq!(file.metadata().unwrap().permissions().mode() & 0o777, 0o600);
        assert!(private_file(&path).is_err());
        std::fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink("/does/not/exist", &path).unwrap();
        assert!(private_file(&path).is_err());
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn apply_requires_explicit_flag_before_loading_configuration() {
        let error = command(&["apply".into(), "plan.json".into()]).unwrap_err();
        assert!(error.starts_with("sjel calendar dedupe audit"));
    }

    #[test]
    fn exact_apply_deletes_only_surplus_and_retry_requires_new_audit() {
        let api = Fake::new();
        let mut log = Vec::new();
        assert_eq!(
            apply(&api, &plan(), |id, confirmed| {
                if confirmed {
                    log.push(id.to_owned());
                }
                Ok(())
            })
            .unwrap(),
            1
        );
        assert_eq!(log, ["b"]);
        assert!(api.events.borrow().contains_key("a"));
        assert!(apply(&api, &plan(), |_, _| Ok(())).is_err());
        assert_eq!(api.deleted.borrow().len(), 1);
    }
}
