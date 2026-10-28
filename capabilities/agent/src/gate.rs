//! The gate in front of every tool call that writes or runs: it asks the owner.
//!
//! The agent loop's gate is a synchronous hook (`Extension::tool_call`) that runs before the
//! arguments are parsed, which is exactly the shape an approval needs: the same thread that is
//! about to run `bash` is the one that blocks, and the loop already runs that thread blocking.
//!
//! The decision is not held here. It is written to `secrets/agent/approvals/<id>.json` by
//! [`PolicyFiles`], the same record the Systems page lists and every other capability's gate
//! raises. That is why this gate *polls the file* instead of waiting on a channel: an owner who
//! presses Allow on the Systems page and one who presses it in the app both write the same file,
//! and a resumed run reads the file rather than holding state that a restart would lose.
//!
//! A read-only tool never asks. Without that exemption a five-file turn would be five prompts,
//! and the operator would learn to press Allow without reading — which is the failure this whole
//! contract exists to avoid.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sjel_agent::ext::policy::{Class, Decision, Policy};
use sjel_agent::{Extension, ToolCall, Verdict};
use sjel_server::agent_policy::PolicyFiles;

use crate::diff;
use crate::session::{EventLog, Grants};

/// The name a plugin list would use. Not selectable: the gate is always on, because a run without
/// it is a run that edits the checkout because it decided to.
pub const NAME: &str = "approve";

const PREVIEW_LIMIT: usize = 2_000;
const DIFF_LINE_LIMIT: usize = 200;
/// How much of a file is read to build a preview. A preview of a 200 MB file is not a preview.
const PREVIEW_READ_LIMIT: u64 = 512 * 1024;
/// A run waits this long for a decision, then refuses the call. A pending approval that nobody
/// decided must not hold a session open forever.
const WAIT_LIMIT: Duration = Duration::from_secs(15 * 60);
const POLL: Duration = Duration::from_millis(250);

pub struct ApprovalGate {
    policy: Option<PolicyFiles>,
    /// What this session may reach, and so what its commands are classified against (§3).
    commands: Policy,
    log: Arc<EventLog>,
    root: PathBuf,
    read_only: HashSet<String>,
    /// Calls the owner allowed for the rest of this session, keyed by tool and arguments.
    grants: Arc<Grants>,
    stop: Arc<AtomicBool>,
}

impl ApprovalGate {
    pub fn new(
        log: Arc<EventLog>,
        root: PathBuf,
        reach: Option<PathBuf>,
        read_only: HashSet<String>,
        grants: Arc<Grants>,
        stop: Arc<AtomicBool>,
    ) -> Self {
        Self {
            policy: PolicyFiles::from_deployment(),
            commands: Policy::new(reach),
            log,
            root,
            read_only,
            grants,
            stop,
        }
    }

    /// What the card shows: a one-line summary for the approval record, and a diff when the call
    /// changes a file. Computed from the bytes on disk, never from what the model says it did.
    pub fn examine(&self, call: &ToolCall) -> (String, Option<String>) {
        let args: Value =
            serde_json::from_str(call.function.arguments.trim()).unwrap_or(Value::Null);
        let text = |key: &str| args.get(key).and_then(Value::as_str);

        match call.function.name.as_str() {
            "bash" => {
                let command = text("command").unwrap_or_default().trim();
                if command.is_empty() {
                    return (raw(&call.function.arguments), None);
                }
                (cap(command, PREVIEW_LIMIT), None)
            }
            "write" => {
                let Some(path) = text("path") else {
                    return (raw(&call.function.arguments), None);
                };
                let content = text("content").unwrap_or_default();
                let before = self.read(path).unwrap_or_default();
                let (shown, _) = diff::render(&before, content, DIFF_LINE_LIMIT);
                let (added, removed) = diff::counts(&before, content);
                (
                    format!("{path}  +{added} −{removed}"),
                    Some(shown).filter(|shown| !shown.trim().is_empty()),
                )
            }
            "edit" => {
                let Some(path) = text("path") else {
                    return (raw(&call.function.arguments), None);
                };
                let before = self.read(path).unwrap_or_default();
                let old = text("old_text").unwrap_or_default();
                let new = text("new_text").unwrap_or_default();
                // The edit tool refuses a non-unique match, so the preview shows the pair as the
                // model wrote it and the gate does not pretend to know which occurrence it meant.
                let after = match before.matches(old).count() {
                    1 => Some(before.replacen(old, new, 1)),
                    _ => None,
                };
                let (added, removed) = match &after {
                    Some(after) => diff::counts(&before, after),
                    None => (new.lines().count(), old.lines().count()),
                };
                let shown = match &after {
                    Some(after) => diff::render(&before, after, DIFF_LINE_LIMIT).0,
                    None => {
                        let mut text = String::new();
                        for line in old.lines().take(DIFF_LINE_LIMIT) {
                            text.push_str(&format!("- {line}\n"));
                        }
                        for line in new.lines().take(DIFF_LINE_LIMIT) {
                            text.push_str(&format!("+ {line}\n"));
                        }
                        text
                    }
                };
                (
                    format!("{path}  +{added} −{removed}"),
                    Some(shown).filter(|shown| !shown.trim().is_empty()),
                )
            }
            _ => (raw(&call.function.arguments), None),
        }
    }

    /// The class of this call when the policy has one: `bash` and the file writers.
    fn classify(&self, call: &ToolCall) -> Option<Decision> {
        let args: Value =
            serde_json::from_str(call.function.arguments.trim()).unwrap_or(Value::Null);
        match call.function.name.as_str() {
            "bash" => {
                let command = args.get("command").and_then(Value::as_str)?;
                (!command.trim().is_empty()).then(|| self.commands.bash(command))
            }
            "write" | "edit" => {
                let path = args.get("path").and_then(Value::as_str)?;
                Some(self.commands.write(path))
            }
            _ => None,
        }
    }

    fn read(&self, path: &str) -> Option<String> {
        let path = resolve(&self.root, path);
        let meta = std::fs::metadata(&path).ok()?;
        if meta.len() > PREVIEW_READ_LIMIT {
            return None;
        }
        std::fs::read_to_string(path).ok()
    }

    /// The owner already allowed this exact call for the rest of the session.
    fn granted(&self, call: &ToolCall) -> bool {
        self.grants.allows(&key(call))
    }
}

impl Extension for ApprovalGate {
    fn name(&self) -> &'static str {
        NAME
    }

    fn tool_call(&self, call: &ToolCall) -> Verdict {
        if self.read_only.contains(call.function.name.as_str()) {
            return Verdict::Allow;
        }
        if let Some(decision) = self.classify(call) {
            match decision.class {
                Class::Allow => {
                    self.log.push(json!({
                        "type": "allowed",
                        "name": call.function.name,
                        "rule": decision.rule,
                        "reason": decision.reason,
                    }));
                    return Verdict::Allow;
                }
                Class::Block => {
                    self.log.push(json!({
                        "type": "blocked",
                        "name": call.function.name,
                        "rule": decision.rule,
                        "reason": decision.reason,
                    }));
                    return Verdict::Deny(format!(
                        "refused before it ran (rule {}): {}. This one is not a permission the \
                         owner grants for a session; say what you wanted to achieve and ask.",
                        decision.rule, decision.reason
                    ));
                }
                Class::Ask => {}
            }
        }
        if self.granted(call) {
            return Verdict::Allow;
        }
        let Some(policy) = &self.policy else {
            return Verdict::Deny(
                "no overlay is configured, so this run cannot ask for permission. A tool that \
                 writes or runs is refused; the read-only tools still work."
                    .into(),
            );
        };
        let (preview, changes) = self.examine(call);
        let digest = digest(&call.function.arguments);
        let id = match policy.request_approval(
            "agent",
            "TOOL",
            &call.function.name,
            &digest,
            &preview,
        ) {
            Ok(id) => id,
            Err(reason) => {
                return Verdict::Deny(format!(
                    "this call needs the owner's permission and the approval record could not be \
                     written: {reason}"
                ))
            }
        };
        let decision = self.classify(call);
        let rule = decision.as_ref().map(|d| d.rule);
        let class = decision.as_ref().map(|d| match d.class {
            Class::Block => "block",
            Class::Ask => "ask",
            Class::Allow => "allow",
        });
        self.grants.note_pending(&id, key(call));
        self.log.push(json!({
            "type": "needs_approval",
            "approval_id": id,
            "name": call.function.name,
            "arguments": cap(&call.function.arguments, PREVIEW_LIMIT),
            "preview": preview,
            "diff": changes,
            "rule": rule,
            "class": class,
        }));

        let started = Instant::now();
        loop {
            if self.stop.load(Ordering::Relaxed) {
                let _ = policy.decide(&id, false);
                return Verdict::Deny(
                    "the user stopped the turn before this call was decided".into(),
                );
            }
            if started.elapsed() > WAIT_LIMIT {
                let _ = policy.decide(&id, false);
                return Verdict::Deny(format!(
                    "nobody decided on this call within {} minutes, so it was refused",
                    WAIT_LIMIT.as_secs() / 60
                ));
            }
            let state = policy
                .approval(&id)
                .and_then(|record| record["state"].as_str().map(str::to_owned));
            match state.as_deref() {
                Some("allowed") => {
                    self.log.push(json!({
                        "type": "approved",
                        "approval_id": id,
                        "name": call.function.name,
                        "for_session": self.granted(call),
                    }));
                    return Verdict::Allow;
                }
                Some("denied") => {
                    self.grants.forget(&id);
                    self.log.push(
                        json!({"type": "denied", "approval_id": id, "name": call.function.name}),
                    );
                    return Verdict::Deny(format!(
                        "the owner denied `{}`. Do not retry it; say what you would need instead.",
                        call.function.name
                    ));
                }
                _ => std::thread::sleep(POLL),
            }
        }
    }

    /// The rule the run is told once, so a model does not spend a turn discovering that its first
    /// `bash` did not run.
    fn system(&self, prompt: &mut String) {
        prompt.push_str(
            "\n\nEvery tool that writes or runs a command needs the owner's explicit permission, \
             asked one call at a time. A refused call comes back as a result explaining why; do \
             not retry it. Read-only tools never ask.",
        );
    }
}

/// The key a session-wide grant is remembered under: the tool and the exact arguments, so
/// allowing `cargo test` does not allow `cargo publish`.
fn key(call: &ToolCall) -> String {
    format!(
        "{}:{}",
        call.function.name,
        digest(&call.function.arguments)
    )
}

fn digest(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn raw(arguments: &str) -> String {
    cap(arguments.trim(), PREVIEW_LIMIT)
}

fn cap(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_owned();
    }
    let end = text.floor_char_boundary(limit);
    format!("{}…", &text[..end])
}

/// A path relative to the session root, or the path as given when it is absolute.
fn resolve(root: &Path, path: &str) -> PathBuf {
    let given = Path::new(path);
    if given.is_absolute() {
        given.to_path_buf()
    } else {
        root.join(given)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(name: &str, arguments: Value) -> ToolCall {
        ToolCall {
            id: "c1".into(),
            kind: "function".into(),
            function: sjel_agent::FunctionCall {
                name: name.into(),
                arguments: arguments.to_string(),
            },
        }
    }

    fn gate(root: &Path) -> ApprovalGate {
        ApprovalGate {
            policy: None,
            commands: Policy::new(Some(root.to_path_buf())),
            log: Arc::new(EventLog::new()),
            root: root.to_path_buf(),
            read_only: HashSet::from(["read".to_string(), "grep".to_string()]),
            grants: Grants::new(),
            stop: Arc::new(AtomicBool::new(false)),
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("sjel-agent-gate-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_read_only_call_never_asks() {
        let dir = scratch("read-only");
        let gate = gate(&dir);
        assert_eq!(
            gate.tool_call(&call("read", json!({"path": "x"}))),
            Verdict::Allow
        );
        assert_eq!(
            gate.tool_call(&call("grep", json!({"pattern": "x"}))),
            Verdict::Allow
        );
    }

    #[test]
    fn without_an_overlay_a_write_is_refused_rather_than_run_silently() {
        let dir = scratch("no-overlay");
        let gate = gate(&dir);
        // A command the policy asks about, so the refusal comes from the missing overlay rather
        // than from a rule that blocks it outright.
        let Verdict::Deny(reason) =
            gate.tool_call(&call("bash", json!({"command": "git commit -m x"})))
        else {
            panic!("a write with nowhere to record a decision must be refused");
        };
        assert!(reason.contains("cannot ask for permission"), "{reason}");
    }

    #[test]
    fn a_granted_call_is_allowed_without_asking_again() {
        let dir = scratch("granted");
        let gate = gate(&dir);
        let bash = call("bash", json!({"command": "git commit -m x"}));
        gate.grants.note_pending("a1", key(&bash));
        assert!(gate.grants.allow_for_session("a1"));
        assert_eq!(gate.tool_call(&bash), Verdict::Allow);
        // The grant is for those exact arguments: another command asks (and here, has no overlay).
        assert!(matches!(
            gate.tool_call(&call("bash", json!({"command": "git push origin main"}))),
            Verdict::Deny(_)
        ));
    }

    #[test]
    fn a_build_runs_without_a_card_and_the_log_names_its_rule() {
        let dir = scratch("allow");
        let gate = gate(&dir);
        let bash = call("bash", json!({"command": "cargo test -p sjel-comms"}));
        assert_eq!(gate.tool_call(&bash), Verdict::Allow);
        let logged = gate.log.after(0).event.expect("the decision is logged");
        assert_eq!(logged["type"], "allowed");
        assert_eq!(logged["rule"], "R3");
    }

    #[test]
    fn a_blocked_call_never_reaches_the_card_and_names_its_rule() {
        let dir = scratch("block");
        let gate = gate(&dir);
        let mut cursor = 0;
        for (command, rule) in [
            ("rm -rf /", "B1"),
            ("git reset --hard", "B2"),
            ("sudo ls", "B3"),
            ("cargo publish", "B7"),
        ] {
            let Verdict::Deny(reason) = gate.tool_call(&call("bash", json!({"command": command})))
            else {
                panic!("{command} ran");
            };
            assert!(reason.contains(rule), "{command}: {reason}");
            assert!(reason.contains("not a permission"), "{command}: {reason}");
            let logged = gate
                .log
                .after(cursor)
                .event
                .expect("the decision is logged");
            assert_eq!(logged["type"], "blocked");
            assert_eq!(logged["rule"], rule);
            cursor = logged["seq"].as_u64().unwrap();
        }
    }

    #[test]
    fn a_write_outside_the_scope_is_blocked_before_the_card() {
        let dir = scratch("write-block");
        let gate = gate(&dir);
        let Verdict::Deny(reason) = gate.tool_call(&call(
            "write",
            json!({"path": "/etc/hosts", "content": "x"}),
        )) else {
            panic!("a write outside the scope ran");
        };
        assert!(reason.contains("rule B1"), "{reason}");
    }

    #[test]
    fn a_write_previews_the_lines_it_changes() {
        let dir = scratch("preview");
        std::fs::write(dir.join("f.rs"), "if self.empty {\n    todo!()\n}\n").unwrap();
        let gate = gate(&dir);
        let (preview, changes) = gate.examine(&call(
            "edit",
            json!({"path": "f.rs", "old_text": "if self.empty {", "new_text": "if self.rows.is_empty() {"}),
        ));
        assert_eq!(preview, "f.rs  +1 −1");
        let changes = changes.expect("an edit shows a diff");
        assert!(changes.contains("- if self.empty {"), "{changes}");
        assert!(changes.contains("+ if self.rows.is_empty() {"), "{changes}");
    }

    #[test]
    fn a_write_to_a_new_file_is_all_additions_and_an_absolute_path_is_read_as_given() {
        let dir = scratch("new-file");
        let gate = gate(&dir);
        let (preview, changes) = gate.examine(&call(
            "write",
            json!({"path": "new.txt", "content": "one\ntwo\n"}),
        ));
        assert_eq!(preview, "new.txt  +2 −0");
        assert_eq!(changes.unwrap(), "+ one\n+ two\n");

        std::fs::write(dir.join("abs.txt"), "old\n").unwrap();
        let absolute = dir.join("abs.txt");
        let (_, changes) = gate.examine(&call(
            "write",
            json!({"path": absolute.to_string_lossy(), "content": "new\n"}),
        ));
        assert!(changes.unwrap().contains("- old"));
    }

    #[test]
    fn an_unparseable_call_still_shows_what_it_asked_for() {
        let dir = scratch("unparseable");
        let gate = gate(&dir);
        let call = ToolCall {
            id: "c2".into(),
            kind: "function".into(),
            function: sjel_agent::FunctionCall {
                name: "bash".into(),
                arguments: "{\"command\": ".into(),
            },
        };
        let (preview, changes) = gate.examine(&call);
        assert!(preview.contains("command"), "{preview}");
        assert!(changes.is_none());
    }

    #[test]
    fn the_gate_tells_the_model_the_rule_once() {
        let dir = scratch("system");
        let mut prompt = String::from("base");
        gate(&dir).system(&mut prompt);
        assert!(prompt.contains("needs the owner's explicit permission"));
        assert!(prompt.contains("Read-only tools never ask"));
    }
}
