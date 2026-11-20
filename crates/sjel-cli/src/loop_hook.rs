//! `tools/loop-hook` — a Claude Code `PostToolUse` hook that tells the agent when it repeats
//! itself: the same tool with the same input several times, or two calls alternating a-b-a-b.
//!
//! It reports and never blocks. Each loop is reported once per session. State is one small JSON
//! file per session in the temporary directory, so nothing is written into the overlay.

use std::io::Read as _;
use std::path::PathBuf;
use std::process::ExitCode;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

const WINDOW: usize = 12;
const REPEATS: usize = 4;
const MAX_INPUT: u64 = 10 * 1024 * 1024;

#[derive(Default, Serialize, Deserialize)]
struct State {
    window: Vec<Call>,
    reported: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Call {
    sig: String,
    tool: String,
}

pub fn run() -> ExitCode {
    let _ = report();
    ExitCode::SUCCESS
}

fn report() -> Option<()> {
    let mut raw = String::new();
    std::io::stdin()
        .take(MAX_INPUT)
        .read_to_string(&mut raw)
        .ok()?;
    let input: Value = serde_json::from_str(&raw).ok()?;
    let tool = input.get("tool_name")?.as_str()?.to_owned();
    let session = sanitize(
        input
            .get("session_id")
            .and_then(Value::as_str)
            .unwrap_or(""),
    );
    let path = state_dir()?.join(format!("{session}.json"));
    let mut state: State = std::fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    let tool_input = input.get("tool_input").cloned().unwrap_or(Value::Null);
    let message = observe(&mut state, &tool, &tool_input);
    std::fs::write(&path, serde_json::to_vec(&state).ok()?).ok()?;
    let message = message?;
    let out = serde_json::json!({
        "hookSpecificOutput": { "hookEventName": "PostToolUse", "additionalContext": message }
    });
    println!("{out}");
    Some(())
}

fn state_dir() -> Option<PathBuf> {
    let dir = std::env::temp_dir().join("sjel-loop-hook");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

fn sanitize(session: &str) -> String {
    let safe: String = session
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if safe.is_empty() {
        "unknown".into()
    } else {
        safe
    }
}

fn signature(tool: &str, input: &Value) -> String {
    let digest = Sha256::digest(format!("{tool}\n{input}").as_bytes());
    digest.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

/// Record one call and return a message when it completes a loop not yet reported.
fn observe(state: &mut State, tool: &str, input: &Value) -> Option<String> {
    let sig = signature(tool, input);
    state.window.push(Call {
        sig: sig.clone(),
        tool: tool.to_owned(),
    });
    if state.window.len() > WINDOW {
        state.window.remove(0);
    }
    let (key, message) = detect(&state.window, &sig, tool)?;
    if state.reported.contains(&key) {
        return None;
    }
    state.reported.push(key);
    Some(message)
}

fn detect(window: &[Call], sig: &str, tool: &str) -> Option<(String, String)> {
    let repeats = window.iter().filter(|call| call.sig == sig).count();
    if repeats >= REPEATS {
        return Some((
            format!("repeat:{sig}"),
            format!(
                "Loop check: {tool} ran {repeats} times with the same input in the last {} calls. \
                 Stop and name the assumption that may be wrong before you run it again.",
                window.len()
            ),
        ));
    }
    let last: Vec<&Call> = window.iter().rev().take(4).collect();
    if last.len() == 4
        && last[0].sig == last[2].sig
        && last[1].sig == last[3].sig
        && last[0].sig != last[1].sig
    {
        let mut pair = [last[0].sig.as_str(), last[1].sig.as_str()];
        pair.sort_unstable();
        return Some((
            format!("alternate:{}|{}", pair[0], pair[1]),
            format!(
                "Loop check: the last four calls alternate between {} and {} with the same inputs. \
                 Stop and check whether either call changes anything.",
                last[1].tool, last[0].tool
            ),
        ));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn repeated_input_is_reported_once() {
        let mut state = State::default();
        let input = json!({ "command": "cargo test" });
        let messages: Vec<_> = (0..6)
            .filter_map(|_| observe(&mut state, "Bash", &input))
            .collect();
        assert_eq!(messages.len(), 1);
        assert!(messages[0].contains("4 times"));
    }

    #[test]
    fn alternation_is_reported_and_distinct_calls_are_not() {
        let mut state = State::default();
        let (a, b) = (json!({ "file_path": "a" }), json!({ "file_path": "b" }));
        assert!(observe(&mut state, "Read", &a).is_none());
        assert!(observe(&mut state, "Edit", &b).is_none());
        assert!(observe(&mut state, "Read", &a).is_none());
        assert!(observe(&mut state, "Edit", &b)
            .unwrap()
            .contains("alternate"));

        let mut fresh = State::default();
        for n in 0..20 {
            assert!(observe(&mut fresh, "Read", &json!({ "file_path": n })).is_none());
        }
        assert_eq!(fresh.window.len(), WINDOW);
    }

    #[test]
    fn session_ids_cannot_leave_the_state_directory() {
        assert_eq!(sanitize("../../etc"), "______etc");
        assert_eq!(sanitize(""), "unknown");
    }
}
