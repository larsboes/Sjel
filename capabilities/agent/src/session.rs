//! One conversation: its messages, the event log a stream replays, and the file it is filed under.
//!
//! A session outlives a request. The Mac's finder and a phone both attach to the same run, so the
//! events a run emits are held here rather than pushed into whichever HTTP response happened to be
//! open — that is what makes `GET /stream?from=<seq>` a reconnect instead of a restart.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use sjel_agent::{AgentInput, Message};

use crate::tier::Tier;

/// How many events one session keeps for a reconnect. A stream that asks for an older sequence is
/// told the log was truncated, because a reader silently resumed mid-turn would draw half a tool
/// call as if it were the whole of one.
const EVENT_LIMIT: usize = 4_000;
const TITLE_LIMIT: usize = 80;
/// How much of the first prompt names the session in the list.
const TITLE_BYTES: usize = 200;

/// The events one run emitted, in order, with a sequence number on each.
pub struct EventLog {
    events: Mutex<Vec<Value>>,
    subscribers: Mutex<Vec<tokio::sync::mpsc::Sender<()>>>,
    seq: AtomicU64,
    done: AtomicBool,
    limit: usize,
}

impl EventLog {
    pub fn new() -> Self {
        Self::with_limit(EVENT_LIMIT)
    }

    fn with_limit(limit: usize) -> Self {
        Self {
            events: Mutex::new(Vec::new()),
            subscribers: Mutex::new(Vec::new()),
            seq: AtomicU64::new(0),
            done: AtomicBool::new(false),
            limit,
        }
    }

    #[cfg(test)]
    fn with_limit_for_test(limit: usize) -> Self {
        Self::with_limit(limit)
    }

    /// Appends one event and wakes every open stream. Callable from the blocking thread the agent
    /// loop runs on, which is why the wake is `try_send` on a one-slot channel rather than a
    /// future: the sender never waits for a reader.
    pub fn push(&self, mut value: Value) -> u64 {
        let seq = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        if let Some(object) = value.as_object_mut() {
            object.insert("seq".into(), Value::from(seq));
        }
        {
            let mut events = self.lock_events();
            events.push(value);
            if events.len() > self.limit {
                let excess = events.len() - self.limit;
                events.drain(..excess);
            }
        }
        self.wake();
        seq
    }

    fn wake(&self) {
        let mut subscribers = self
            .subscribers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        subscribers.retain(|sender| sender.try_send(()).is_ok() || !sender.is_closed());
    }

    /// A reader's handle. Subscribe before reading the log, so an event that lands in between is
    /// a wake the reader has not missed.
    pub fn subscribe(&self) -> tokio::sync::mpsc::Receiver<()> {
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        self.subscribers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(tx);
        rx
    }

    /// The next event after `cursor`, one per call.
    ///
    /// One, not all, because a server-sent event carries one `data:` line: batching several JSON
    /// objects into one event would be cut at the first newline by every client that reads the
    /// protocol rather than this implementation.
    pub fn after(&self, cursor: u64) -> Next {
        let events = self.lock_events();
        let first = events
            .first()
            .and_then(|event| event["seq"].as_u64())
            .unwrap_or(cursor + 1);
        // Not `cursor != 0`: a reader that asked for everything and got a log whose oldest event
        // is already newer than that has still missed events, and must be told.
        let truncated = first > cursor + 1;
        let next = events
            .iter()
            .find(|event| event["seq"].as_u64().unwrap_or(0) > cursor)
            .cloned();
        Next {
            event: next,
            truncated,
            first_seq: first,
            done: self.done(),
        }
    }

    pub fn finish(&self) {
        self.done.store(true, Ordering::SeqCst);
        self.wake();
    }

    pub fn done(&self) -> bool {
        self.done.load(Ordering::SeqCst)
    }

    pub fn clear(&self) {
        self.lock_events().clear();
        self.seq.store(0, Ordering::SeqCst);
        self.done.store(false, Ordering::SeqCst);
    }

    fn lock_events(&self) -> std::sync::MutexGuard<'_, Vec<Value>> {
        self.events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl Default for EventLog {
    fn default() -> Self {
        Self::new()
    }
}

/// What one session's owner has already allowed, and the calls waiting for an answer.
///
/// Keyed by tool and arguments, so allowing `cargo test` does not allow `cargo publish`. The
/// pending map exists because an approval is decided by id, and only the gate knows which call
/// that id was for.
#[derive(Default)]
pub struct Grants {
    inner: Mutex<GrantsInner>,
}

#[derive(Default)]
struct GrantsInner {
    allowed: HashSet<String>,
    pending: HashMap<String, String>,
}

impl Grants {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn allows(&self, key: &str) -> bool {
        self.lock().allowed.contains(key)
    }

    pub fn note_pending(&self, approval: &str, key: String) {
        self.lock().pending.insert(approval.to_owned(), key);
    }

    /// The owner allowed this approval for the rest of the session. `false` when the id is not a
    /// call this session raised — an approval from another session is not a grant to this one.
    pub fn allow_for_session(&self, approval: &str) -> bool {
        let mut inner = self.lock();
        match inner.pending.remove(approval) {
            Some(key) => {
                inner.allowed.insert(key);
                true
            }
            None => false,
        }
    }

    pub fn forget(&self, approval: &str) {
        self.lock().pending.remove(approval);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, GrantsInner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// One step of a reader's walk through a log.
pub struct Next {
    /// The next event, when there is one.
    pub event: Option<Value>,
    pub truncated: bool,
    /// Oldest event still retained, so a reader can recover the transcript and resume here.
    pub first_seq: u64,
    pub done: bool,
}

/// What can steer, stop and be waited on while a run is active.
#[derive(Clone)]
pub struct Active {
    pub input: AgentInput,
    pub stop: Arc<AtomicBool>,
}

pub struct Session {
    pub id: String,
    pub root: PathBuf,
    /// What this session may reach. Fixed at creation (A3): a grant in one tier must not be usable
    /// from another, so the only way into a wider tier is a new session.
    pub tier: Tier,
    /// The directory the tier lets this session touch, or `None` when it touches no filesystem.
    pub reach: Option<PathBuf>,
    pub created_at: u64,
    /// The turns so far. Held for the length of a run, so one run per session.
    pub messages: Mutex<Vec<Message>>,
    pub log: Arc<EventLog>,
    pub grants: Arc<Grants>,
    title: Mutex<Option<String>>,
    turns: AtomicUsize,
    active: Mutex<Option<Active>>,
    persisted: Mutex<usize>,
    path: PathBuf,
    meta_path: PathBuf,
    plugins: Vec<String>,
}

impl Session {
    /// Creates the file and its directory, so a session that was announced exists after a crash.
    pub fn create(
        dir: &Path,
        root: &Path,
        tier: Tier,
        reach: Option<PathBuf>,
        plugins: Vec<String>,
    ) -> std::io::Result<Self> {
        let id = new_id();
        std::fs::create_dir_all(dir)?;
        let path = dir.join(format!("{id}.jsonl"));
        let meta_path = dir.join(format!("{id}.meta.json"));
        let session = Self {
            id,
            root: root.to_path_buf(),
            tier,
            reach,
            created_at: now_secs(),
            messages: Mutex::new(Vec::new()),
            log: Arc::new(EventLog::new()),
            grants: Grants::new(),
            title: Mutex::new(None),
            turns: AtomicUsize::new(0),
            active: Mutex::new(None),
            persisted: Mutex::new(0),
            path,
            meta_path,
            plugins,
        };
        session.write_meta()?;
        Ok(session)
    }

    /// A session from an earlier process, read back from its meta and its turns.
    pub fn load(dir: &Path, id: &str) -> Option<Self> {
        if !id.bytes().all(|b| b.is_ascii_hexdigit()) || id.is_empty() || id.len() > 32 {
            return None;
        }
        let meta_path = dir.join(format!("{id}.meta.json"));
        let meta: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&meta_path).ok()?).ok()?;
        let path = dir.join(format!("{id}.jsonl"));
        let messages = sjel_agent::session::load(&path).unwrap_or_default();
        let title = messages.iter().find_map(|message| match message {
            Message::User { content } => Some(title_of(content)),
            _ => None,
        });
        let turns = messages.len();
        Some(Self {
            id: id.to_owned(),
            root: PathBuf::from(meta["root"].as_str().unwrap_or_default()),
            tier: meta["tier"]
                .as_str()
                .and_then(Tier::parse)
                .unwrap_or(crate::tier::DEFAULT),
            reach: meta["reach"].as_str().map(PathBuf::from),
            created_at: meta["created_at"].as_u64().unwrap_or(0),
            messages: Mutex::new(messages),
            log: Arc::new(EventLog::new()),
            grants: Grants::new(),
            title: Mutex::new(title),
            turns: AtomicUsize::new(turns),
            active: Mutex::new(None),
            persisted: Mutex::new(turns),
            path,
            meta_path,
            plugins: meta["plugins"]
                .as_array()
                .map(|list| {
                    list.iter()
                        .filter_map(|v| v.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_else(crate::plugins::configured),
        })
    }

    /// Every session filed under `dir`, newest first, read from disk so a restart keeps the list.
    pub fn list(dir: &Path) -> Vec<serde_json::Value> {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut out: Vec<serde_json::Value> = Vec::new();
        for root_entry in entries.filter_map(Result::ok) {
            let Ok(sessions) = std::fs::read_dir(root_entry.path()) else {
                continue;
            };
            for entry in sessions.filter_map(Result::ok) {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("json") {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(&path) else {
                    continue;
                };
                if let Ok(meta) = serde_json::from_str::<serde_json::Value>(&text) {
                    out.push(meta);
                }
            }
        }
        out.sort_by_key(|meta| std::cmp::Reverse(meta["created_at"].as_u64().unwrap_or(0)));
        out
    }

    pub fn meta(&self) -> Value {
        json!({
            "id": self.id,
            "root": self.root.to_string_lossy(),
            "tier": self.tier.as_str(),
            "reach": self.reach.as_ref().map(|reach| reach.to_string_lossy()),
            "created_at": self.created_at,
            "plugins": self.plugins,
            "turns": self.turns.load(Ordering::Relaxed),
            "active": self.active().is_some(),
            "title": self.title.lock().ok().and_then(|title| title.clone()),
        })
    }

    /// Records a user turn. The title comes from the first one, so a list says what a session is
    /// about without reading its transcript.
    pub fn push_user(&self, content: String) {
        let mut messages = self
            .messages
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        messages.push(Message::User { content });
        self.turns.store(messages.len(), Ordering::Relaxed);
        drop(messages);
        let mut title = self
            .title
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if title.is_none() {
            if let Ok(messages) = self.messages.try_lock() {
                *title = messages.iter().find_map(|message| match message {
                    Message::User { content } => Some(title_of(content)),
                    _ => None,
                });
            }
        }
    }

    /// The plugin names this session was started with.
    pub fn plugins(&self) -> &[String] {
        &self.plugins
    }

    pub fn active(&self) -> Option<Active> {
        self.active
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Claims the session for one run. `Err` when a run is already active.
    pub fn begin(&self, active: Active) -> Result<(), ()> {
        let mut slot = self
            .active
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if slot.is_some() {
            return Err(());
        }
        *slot = Some(active);
        Ok(())
    }

    pub fn end(&self) {
        *self
            .active
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    }

    /// Appends the turns that are not on disk yet. One write per turn, so a reader never sees half.
    pub fn persist(&self) -> std::io::Result<()> {
        let messages = self
            .messages
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut persisted = self
            .persisted
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if *persisted >= messages.len() {
            return Ok(());
        }
        let tail = &messages[*persisted..];
        sjel_agent::session::append(&self.path, tail)?;
        *persisted = messages.len();
        self.turns.store(messages.len(), Ordering::Relaxed);
        drop(persisted);
        drop(messages);
        self.write_meta()
    }

    /// The turns as stored. While a run holds the conversation this reads the file instead of
    /// waiting, because a client polling a transcript must not block the run it is watching.
    pub fn transcript(&self) -> Value {
        let messages = match self.messages.try_lock() {
            Ok(guard) => serde_json::to_value(&*guard).unwrap_or(Value::Null),
            Err(_) => {
                serde_json::to_value(sjel_agent::session::load(&self.path).unwrap_or_default())
                    .unwrap_or(Value::Null)
            }
        };
        json!({
            "id": self.id,
            "root": self.root.to_string_lossy(),
            "messages": messages,
        })
    }

    fn write_meta(&self) -> std::io::Result<()> {
        std::fs::write(&self.meta_path, self.meta().to_string())
    }
}

pub(crate) fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// One line, cut, from the first prompt.
fn title_of(content: &str) -> String {
    let line = content.lines().next().unwrap_or_default().trim();
    line.chars().take(TITLE_LIMIT).collect()
}

pub(crate) fn new_id() -> String {
    let mut raw = [0u8; 8];
    if getrandom::fill(&mut raw).is_err() {
        // A collision here costs one session, not a decision; the clock keeps ids apart when the
        // secure source is unavailable rather than refusing to start at all.
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        raw.copy_from_slice(&nanos.to_le_bytes()[..8]);
    }
    raw.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Every root's directory under the sessions dir, so a session can be found by id alone.
pub fn root_dirs(sessions: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(sessions) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect()
}

/// The directory name one root is filed under: `~/Developer/Sjel` becomes `Developer-Sjel`, so a
/// list of sessions can be read without decoding paths.
pub fn root_slug(root: &Path) -> String {
    let text = root.to_string_lossy();
    let trimmed = text.trim_start_matches('/');
    let replaced: String = trimmed
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let collapsed = replaced.trim_matches('-').to_owned();
    if collapsed.is_empty() {
        "root".to_owned()
    } else {
        collapsed.chars().take(TITLE_BYTES).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sjel-agent-capability-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_stream_reads_from_its_own_cursor() {
        let log = EventLog::new();
        let mut first = log.subscribe();
        log.push(json!({"type": "text", "content": "a"}));
        log.push(json!({"type": "text", "content": "b"}));
        assert!(first.try_recv().is_ok(), "a push wakes an open stream");

        let next = log.after(0);
        assert!(!next.truncated);
        assert!(!next.done);
        assert_eq!(next.event.unwrap()["seq"], 1);

        // A second reader from the middle sees only what it asked for.
        let next = log.after(1);
        assert_eq!(next.event.unwrap()["content"], "b");
        assert!(!next.truncated);
        // And a reader that is caught up is told there is nothing, not that the run ended.
        let next = log.after(2);
        assert!(next.event.is_none());
        assert!(!next.done);
    }

    #[test]
    fn a_cursor_older_than_the_ring_is_told_it_was_truncated() {
        let log = EventLog::with_limit_for_test(2);
        for i in 0..4 {
            log.push(json!({"type": "text", "content": i}));
        }
        let next = log.after(0);
        assert_eq!(next.event.unwrap()["seq"], 3);
        assert!(
            next.truncated,
            "a resumed stream must know it missed events"
        );
        let next = log.after(3);
        assert_eq!(next.event.unwrap()["seq"], 4);
        assert!(!next.truncated);
    }

    #[test]
    fn finishing_ends_every_reader() {
        let log = EventLog::new();
        let mut reader = log.subscribe();
        log.finish();
        assert!(reader.try_recv().is_ok(), "the end is a wake too");
        assert!(log.after(0).done);
    }

    #[test]
    fn a_root_becomes_a_directory_name_that_cannot_escape() {
        assert_eq!(
            root_slug(Path::new("/Users/x/Developer/Sjel")),
            "Users-x-Developer-Sjel"
        );
        assert_eq!(root_slug(Path::new("/")), "root");
        assert_eq!(root_slug(Path::new("")), "root");
        assert!(!root_slug(Path::new("/../..")).contains('/'));
    }

    #[test]
    fn one_run_at_a_time_and_the_turns_reach_the_file() {
        let dir = scratch("session");
        let root = scratch("root");
        let session = Session::create(
            &dir,
            &root,
            crate::tier::Tier::Project,
            Some(root.clone()),
            vec!["fs".into()],
        )
        .unwrap();
        assert!(session.meta()["title"].is_null());
        assert_eq!(session.meta()["tier"], "project");

        session.push_user("fix the inbox test\nand then stop".into());
        assert_eq!(session.meta()["title"], "fix the inbox test");

        let active = Active {
            input: AgentInput::default(),
            stop: Arc::new(AtomicBool::new(false)),
        };
        assert!(session.begin(active.clone()).is_ok());
        assert!(
            session.begin(active).is_err(),
            "a second run must be refused"
        );
        session.end();
        assert!(session.active().is_none());

        session.persist().unwrap();
        let written = std::fs::read_to_string(dir.join(format!("{}.jsonl", session.id))).unwrap();
        assert!(written.contains("fix the inbox test"));
        // Persisting twice does not duplicate the turn.
        session.persist().unwrap();
        let written = std::fs::read_to_string(dir.join(format!("{}.jsonl", session.id))).unwrap();
        assert_eq!(written.matches("fix the inbox test").count(), 1);
    }
}
