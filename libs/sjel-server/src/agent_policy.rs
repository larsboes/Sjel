//! What an agent may do on each capability, and the record of what it did (ISA F10).
//!
//! Files under the overlay, shared by every capability's gate and by sjel-status, which owns
//! the Systems page that writes them. Files rather than a table, because this crate is under
//! every capability and must not depend on the store.
//!
//! The policy, the approvals and the log are under `secrets/agent/`, because an agent must
//! not be able to write them: an agent that could edit an approval would allow its own write.
//! The managed Claude Code policy denies `secrets/**` to an agent session
//! (`tools/templates/claude-code/managed-settings.json`).
//!
//! - `secrets/agent/policy.json`: one [`Mode`] per capability. Absent means [`Mode::Auto`], the
//!   principal's default of 2026-10-01. Written only by sjel-status, never by hand.
//! - `secrets/agent/approvals/<id>.json`: one write waiting in ask mode, or decided.
//! - `secrets/agent/calls/<YYYY-MM>.jsonl`: one line per agent call. Never a body or a query.
//!
//! Two files an agent may read, because they grant nothing:
//!
//! - `data/agent-gates/<capability>.json`: which capabilities run a gate, and their routes.
//! - `data/agent-modes.json`: a copy of the modes, written beside every change, so the MCP
//!   server offers only the tools a mode allows. Editing it changes a list, not the gate.

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// What an agent may do on one capability.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mode {
    /// The agent is refused outright.
    Off,
    /// Reads only.
    ReadOnly,
    /// Reads, and writes that wait for the owner's Allow.
    Ask,
    /// Reads and writes. A `confirm` route still asks (Product Rule 5).
    Auto,
}

impl Mode {
    pub const ALL: [Mode; 4] = [Mode::Off, Mode::ReadOnly, Mode::Ask, Mode::Auto];

    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Off => "off",
            Mode::ReadOnly => "read-only",
            Mode::Ask => "ask",
            Mode::Auto => "auto",
        }
    }

    pub fn parse(text: &str) -> Option<Mode> {
        Mode::ALL.into_iter().find(|m| m.as_str() == text)
    }
}

/// The mode when the policy names nothing for a capability.
pub const DEFAULT_MODE: Mode = Mode::Auto;

/// Routes whose agent handling differs from what their method says. Patterns are paths as the
/// capability serves them, with `{name}` matching one segment.
#[derive(Clone, Copy, Debug, Default)]
pub struct AgentRoutes {
    /// A write that leaves Sjel or cannot be undone. It asks in every mode (Product Rule 5).
    pub confirm: &'static [(&'static str, &'static str)],
    /// A `GET` that changes state, so the mode treats it as a write (ISA ISC-38's audit).
    pub get_writes: &'static [(&'static str, &'static str)],
}

impl AgentRoutes {
    pub const NONE: AgentRoutes = AgentRoutes {
        confirm: &[],
        get_writes: &[],
    };

    pub fn confirms(&self, method: &str, path: &str) -> bool {
        any_matches(self.confirm, method, path)
    }

    pub fn get_writes(&self, method: &str, path: &str) -> bool {
        any_matches(self.get_writes, method, path)
    }
}

fn any_matches(list: &[(&str, &str)], method: &str, path: &str) -> bool {
    list.iter()
        .any(|(m, pattern)| m.eq_ignore_ascii_case(method) && path_matches(pattern, path))
}

/// `{name}` matches exactly one non-empty segment; every other segment must be equal.
pub fn path_matches(pattern: &str, path: &str) -> bool {
    let mut p = pattern.trim_end_matches('/').split('/');
    let mut a = path.trim_end_matches('/').split('/');
    loop {
        match (p.next(), a.next()) {
            (None, None) => return true,
            (Some(want), Some(got)) => {
                let wildcard = want.starts_with('{') && want.ends_with('}');
                if (wildcard && got.is_empty()) || (!wildcard && want != got) {
                    return false;
                }
            }
            _ => return false,
        }
    }
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Writes `contents` next to `path` and renames it into place, so a reader never sees half.
fn write_atomic(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
    fs::write(&tmp, contents)?;
    fs::rename(&tmp, path)
}

/// The three files, rooted at one overlay.
#[derive(Debug)]
pub struct PolicyFiles {
    root: PathBuf,
    cache: Mutex<Option<(SystemTime, HashMap<String, Mode>)>>,
}

impl PolicyFiles {
    pub fn new(overlay_root: impl Into<PathBuf>) -> Self {
        Self {
            root: overlay_root.into(),
            cache: Mutex::new(None),
        }
    }

    /// The deployment's files, or `None` when no overlay is configured.
    pub fn from_deployment() -> Option<Self> {
        sjel_config::overlay_root().map(Self::new)
    }

    fn policy_path(&self) -> PathBuf {
        self.root.join("secrets/agent/policy.json")
    }

    fn mirror_path(&self) -> PathBuf {
        self.root.join("data/agent-modes.json")
    }

    fn approvals_dir(&self) -> PathBuf {
        self.root.join("secrets/agent/approvals")
    }

    fn calls_dir(&self) -> PathBuf {
        self.root.join("secrets/agent/calls")
    }

    fn gates_dir(&self) -> PathBuf {
        self.root.join("data/agent-gates")
    }

    /// Records that `capability`'s gate admits the agent, with its special routes, so the
    /// Systems page lists only switches that act on something. Written when the gate starts.
    pub fn register_gate(&self, capability: &str, routes: &AgentRoutes) {
        let pairs = |list: &[(&str, &str)]| -> Vec<String> {
            list.iter().map(|(m, p)| format!("{m} {p}")).collect()
        };
        let record = json!({
            "capability": capability,
            "confirm": pairs(routes.confirm),
            "get_writes": pairs(routes.get_writes),
            "registered_at": now_secs(),
        });
        let safe = !capability.is_empty()
            && capability
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
        if !safe {
            return;
        }
        let path = self.gates_dir().join(format!("{capability}.json"));
        if let Err(error) = write_atomic(&path, record.to_string().as_bytes()) {
            eprintln!("agent gate registration: {error}");
        }
    }

    /// Every registered gate, by name.
    pub fn gates(&self) -> Vec<Value> {
        let Ok(entries) = fs::read_dir(self.gates_dir()) else {
            return Vec::new();
        };
        let mut out: Vec<Value> = entries
            .filter_map(Result::ok)
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .filter_map(|e| fs::read_to_string(e.path()).ok())
            .filter_map(|t| serde_json::from_str::<Value>(&t).ok())
            .collect();
        out.sort_by(|a, b| a["capability"].as_str().cmp(&b["capability"].as_str()));
        out
    }

    /// Every mode the policy names. A name it does not hold is at [`DEFAULT_MODE`].
    ///
    /// Re-read when the file's modification time moves, so a change on the Systems page reaches
    /// every capability's gate on its next request with no restart. An unreadable file refuses
    /// every write: a policy that cannot be read is not a reason to allow.
    pub fn modes(&self) -> Result<HashMap<String, Mode>, String> {
        let path = self.policy_path();
        let modified = match fs::metadata(&path) {
            Ok(meta) => meta.modified().map_err(|e| e.to_string())?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(HashMap::new()),
            Err(e) => return Err(e.to_string()),
        };
        let mut cache = self.cache.lock().unwrap_or_else(|p| p.into_inner());
        if let Some((at, modes)) = cache.as_ref() {
            if *at == modified {
                return Ok(modes.clone());
            }
        }
        let text = fs::read_to_string(&path).map_err(|e| e.to_string())?;
        let value: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        let mut modes = HashMap::new();
        if let Some(map) = value.get("modes").and_then(Value::as_object) {
            for (name, mode) in map {
                let mode = mode
                    .as_str()
                    .and_then(Mode::parse)
                    .ok_or_else(|| format!("agent policy: {name} has no valid mode"))?;
                modes.insert(name.clone(), mode);
            }
        }
        *cache = Some((modified, modes.clone()));
        Ok(modes)
    }

    /// The mode for one capability. A policy that cannot be read answers [`Mode::ReadOnly`].
    pub fn mode_for(&self, capability: &str) -> Mode {
        match self.modes() {
            Ok(modes) => modes.get(capability).copied().unwrap_or(DEFAULT_MODE),
            Err(error) => {
                eprintln!("agent policy unreadable, agents are read-only: {error}");
                Mode::ReadOnly
            }
        }
    }

    pub fn set_mode(&self, capability: &str, mode: Mode) -> Result<(), String> {
        let mut modes = self.modes()?;
        modes.insert(capability.to_string(), mode);
        let mut names: Vec<_> = modes.keys().cloned().collect();
        names.sort();
        let map: serde_json::Map<String, Value> = names
            .into_iter()
            .map(|n| {
                let m = modes[&n];
                (n, Value::from(m.as_str()))
            })
            .collect();
        let text =
            serde_json::to_vec_pretty(&json!({ "modes": map })).map_err(|e| e.to_string())?;
        write_atomic(&self.policy_path(), &text).map_err(|e| e.to_string())?;
        // The readable copy follows the real one. Failing to write it costs a tool list only.
        if let Err(error) = write_atomic(&self.mirror_path(), &text) {
            eprintln!("agent-modes.json: {error}");
        }
        Ok(())
    }

    // --- approvals ----------------------------------------------------------------------

    fn approval_path(&self, id: &str) -> Option<PathBuf> {
        let safe = !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_hexdigit());
        safe.then(|| self.approvals_dir().join(format!("{id}.json")))
    }

    /// Records a write that waits for the owner. Returns its id.
    pub fn request_approval(
        &self,
        capability: &str,
        method: &str,
        path: &str,
        body_digest: &str,
        preview: &str,
    ) -> Result<String, String> {
        let mut raw = [0u8; 16];
        getrandom::fill(&mut raw).map_err(|e| format!("secure random source: {e}"))?;
        let id = hex(&raw);
        let record = json!({
            "id": id,
            "capability": capability,
            "method": method,
            "path": path,
            "body_sha256": body_digest,
            "preview": preview,
            "created_at": now_secs(),
            "state": "pending",
        });
        let path_on_disk = self.approval_path(&id).ok_or("approval id")?;
        write_atomic(&path_on_disk, record.to_string().as_bytes()).map_err(|e| e.to_string())?;
        Ok(id)
    }

    pub fn approval(&self, id: &str) -> Option<Value> {
        let text = fs::read_to_string(self.approval_path(id)?).ok()?;
        serde_json::from_str(&text).ok()
    }

    /// Approvals newest first, optionally only those in `state`.
    pub fn approvals(&self, state: Option<&str>) -> Vec<Value> {
        let Ok(entries) = fs::read_dir(self.approvals_dir()) else {
            return Vec::new();
        };
        let mut out: Vec<Value> = entries
            .filter_map(Result::ok)
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .filter_map(|e| fs::read_to_string(e.path()).ok())
            .filter_map(|t| serde_json::from_str::<Value>(&t).ok())
            .filter(|v| state.is_none_or(|s| v["state"] == s))
            .collect();
        out.sort_by_key(|v| std::cmp::Reverse(v["created_at"].as_i64().unwrap_or(0)));
        out
    }

    /// The owner's Allow or Deny. Only a pending approval can be decided.
    pub fn decide(&self, id: &str, allow: bool) -> Result<Value, String> {
        let mut record = self.approval(id).ok_or("no such approval")?;
        if record["state"] != "pending" {
            return Err(format!("this approval is already {}", record["state"]));
        }
        record["state"] = Value::from(if allow { "allowed" } else { "denied" });
        record["decided_at"] = Value::from(now_secs());
        let path = self.approval_path(id).ok_or("approval id")?;
        write_atomic(&path, record.to_string().as_bytes()).map_err(|e| e.to_string())?;
        Ok(record)
    }

    /// Uses an allowed approval for exactly this write, once.
    pub fn consume(
        &self,
        id: &str,
        capability: &str,
        method: &str,
        path: &str,
        body_digest: &str,
    ) -> Consumed {
        let Some(mut record) = self.approval(id) else {
            return Consumed::Unknown;
        };
        let same = record["capability"] == capability
            && record["method"] == method
            && record["path"] == path
            && record["body_sha256"] == body_digest;
        if !same {
            return Consumed::Mismatch;
        }
        match record["state"].as_str() {
            Some("pending") => return Consumed::Pending,
            Some("denied") => return Consumed::Denied,
            Some("allowed") => {}
            _ => return Consumed::Used,
        }
        let Some(file) = self.approval_path(id) else {
            return Consumed::Unknown;
        };
        // Claimed by a rename before the state is rewritten: of two requests racing for one
        // approval, only one finds the file to rename.
        let claimed = file.with_extension("claimed");
        if fs::rename(&file, &claimed).is_err() {
            return Consumed::Used;
        }
        record["state"] = Value::from("used");
        record["used_at"] = Value::from(now_secs());
        let _ = fs::write(&claimed, record.to_string());
        let _ = fs::rename(&claimed, &file);
        Consumed::Admitted
    }

    // --- the call log --------------------------------------------------------------------

    /// Appends one call. A log that cannot be written is reported and does not block the call.
    pub fn log_call(&self, call: &AgentCall<'_>) {
        let at = now_secs();
        let line = json!({
            "at": at,
            "capability": call.capability,
            "method": call.method,
            "path": call.path,
            "status": call.status,
            "decision": call.decision,
        });
        let file = self.calls_dir().join(format!("{}.jsonl", month_of(at)));
        let result = fs::create_dir_all(self.calls_dir()).and_then(|_| {
            let mut f = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&file)?;
            f.write_all(format!("{line}\n").as_bytes())
        });
        if let Err(error) = result {
            eprintln!("agent call log: {error}");
        }
    }

    /// The latest `limit` calls, newest first.
    pub fn recent_calls(&self, limit: usize) -> Vec<Value> {
        let Ok(entries) = fs::read_dir(self.calls_dir()) else {
            return Vec::new();
        };
        let mut files: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
            .collect();
        files.sort();
        let mut out = Vec::new();
        for file in files.iter().rev() {
            let Ok(text) = fs::read_to_string(file) else {
                continue;
            };
            for line in text.lines().rev() {
                if let Ok(v) = serde_json::from_str::<Value>(line) {
                    out.push(v);
                    if out.len() >= limit {
                        return out;
                    }
                }
            }
        }
        out
    }
}

/// What [`PolicyFiles::consume`] found.
#[derive(Debug, PartialEq, Eq)]
pub enum Consumed {
    Admitted,
    Pending,
    Denied,
    Used,
    Mismatch,
    Unknown,
}

/// One logged agent call. `path` carries no query: a query can hold the values the log must not.
pub struct AgentCall<'a> {
    pub capability: &'a str,
    pub method: &'a str,
    pub path: &'a str,
    pub status: u16,
    pub decision: &'a str,
}

/// `YYYY-MM` of a Unix time, for the log's monthly files.
fn month_of(secs: i64) -> String {
    // Howard Hinnant's civil_from_days.
    let z = secs.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}")
}

/// SHA-256 of a write as the gate admits it: method, path with query, body.
pub fn write_digest(method: &str, path_and_query: &str, body: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(method.as_bytes());
    hasher.update([0]);
    hasher.update(path_and_query.as_bytes());
    hasher.update([0]);
    hasher.update(body);
    hex(&hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(name: &str) -> PolicyFiles {
        let dir =
            std::env::temp_dir().join(format!("sjel-agent-policy-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        PolicyFiles::new(dir)
    }

    #[test]
    fn an_absent_policy_is_auto_and_a_set_mode_is_read_back() {
        let files = files("modes");
        assert_eq!(files.mode_for("comms"), Mode::Auto);
        files.set_mode("comms", Mode::Ask).unwrap();
        files.set_mode("finance", Mode::Off).unwrap();
        assert!(files.policy_path().starts_with(files.root.join("secrets")));
        let mirror = fs::read_to_string(files.mirror_path()).unwrap();
        assert!(mirror.contains("\"finance\": \"off\""), "{mirror}");
        assert_eq!(files.mode_for("comms"), Mode::Ask);
        assert_eq!(files.mode_for("finance"), Mode::Off);
        assert_eq!(files.mode_for("trips"), Mode::Auto);
    }

    #[test]
    fn an_unreadable_policy_is_read_only() {
        let files = files("broken");
        write_atomic(
            &files.policy_path(),
            b"{\"modes\":{\"comms\":\"sometimes\"}}",
        )
        .unwrap();
        assert_eq!(files.mode_for("comms"), Mode::ReadOnly);
    }

    #[test]
    fn a_pattern_matches_one_segment_per_placeholder() {
        assert!(path_matches("/triage/{id}/gmail", "/triage/abc/gmail"));
        assert!(!path_matches("/triage/{id}/gmail", "/triage//gmail"));
        assert!(!path_matches("/triage/{id}/gmail", "/triage/a/b/gmail"));
        assert!(!path_matches("/triage/{id}/gmail", "/triage/abc/gmailx"));
        assert!(path_matches("/discover", "/discover/"));
    }

    #[test]
    fn an_approval_admits_its_own_write_once() {
        let files = files("approve");
        let digest = write_digest("POST", "/triage/1/gmail", b"{\"action\":\"trash\"}");
        let id = files
            .request_approval("comms", "POST", "/triage/1/gmail", &digest, "trash")
            .unwrap();
        let consume = |d: &str| files.consume(&id, "comms", "POST", "/triage/1/gmail", d);
        assert_eq!(consume(&digest), Consumed::Pending);
        files.decide(&id, true).unwrap();
        let other = write_digest("POST", "/triage/1/gmail", b"{\"action\":\"archive\"}");
        assert_eq!(
            consume(&other),
            Consumed::Mismatch,
            "a different body is not this approval"
        );
        assert_eq!(
            files.consume(&id, "trips", "POST", "/triage/1/gmail", &digest),
            Consumed::Mismatch
        );
        assert_eq!(consume(&digest), Consumed::Admitted);
        assert_eq!(consume(&digest), Consumed::Used, "single-use");
        assert!(
            files.decide(&id, true).is_err(),
            "a used approval cannot be decided again"
        );
    }

    #[test]
    fn a_denied_approval_never_admits() {
        let files = files("deny");
        let digest = write_digest("POST", "/x", b"");
        let id = files
            .request_approval("comms", "POST", "/x", &digest, "")
            .unwrap();
        files.decide(&id, false).unwrap();
        assert_eq!(
            files.consume(&id, "comms", "POST", "/x", &digest),
            Consumed::Denied
        );
        assert_eq!(files.approvals(Some("pending")).len(), 0);
        assert_eq!(files.approvals(None).len(), 1);
    }

    #[test]
    fn an_approval_id_cannot_name_a_path() {
        let files = files("ids");
        assert!(files.approval("../../config/agent-policy").is_none());
        assert_eq!(
            files.consume("../x", "comms", "POST", "/x", ""),
            Consumed::Unknown
        );
    }

    #[test]
    fn the_call_log_keeps_no_query_and_reads_newest_first() {
        let files = files("log");
        for (i, path) in ["/feed", "/triage"].iter().enumerate() {
            files.log_call(&AgentCall {
                capability: "comms",
                method: "GET",
                path,
                status: 200 + i as u16,
                decision: "read",
            });
        }
        let calls = files.recent_calls(10);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0]["path"], "/triage");
        assert_eq!(calls[1]["status"], 200);
    }

    #[test]
    fn months_are_civil() {
        assert_eq!(month_of(0), "1970-01");
        assert_eq!(month_of(1_790_000_000), "2026-09");
        assert_eq!(month_of(951_782_400), "2000-02"); // 2000-02-29
    }
}
