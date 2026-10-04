//! A conversation on disk: one [`Message`] per line (JSONL), appended after every turn.
//!
//! Appending, not rewriting, means a crash loses at most the turn in progress. The system
//! prompt is not stored: a resumed session gets the prompt of the run that resumes it, so an
//! `AGENTS.md` edited in between takes effect.
//!
//! A session can hold any file the model read, so a new file is created owner-only (0600).
//! Where the files live is the front end's decision. The CLI puts them in the private overlay.

use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};

use crate::Message;

/// Appends `messages` to `path`, creating it and its directory when missing.
pub fn append(path: &Path, messages: &[Message]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut out = String::new();
    for message in messages
        .iter()
        .filter(|m| !matches!(m, Message::System { .. }))
    {
        out.push_str(&serde_json::to_string(message)?);
        out.push('\n');
    }
    // One write per turn, so a reader never sees half a line from a whole turn.
    OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)?
        .write_all(out.as_bytes())
}

/// Reads a session back. A line that does not parse is an error with its line number, not a
/// skipped line: a conversation with a hole in it sends tool results without their calls.
pub fn load(path: &Path) -> Result<Vec<Message>, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut messages = Vec::new();
    for (i, line) in BufReader::new(file).lines().enumerate() {
        let line = line.map_err(|e| format!("{}: {e}", path.display()))?;
        if line.trim().is_empty() {
            continue;
        }
        let message: Message = serde_json::from_str(&line)
            .map_err(|e| format!("{}:{}: {e}", path.display(), i + 1))?;
        if !matches!(message, Message::System { .. }) {
            messages.push(message);
        }
    }
    Ok(messages)
}

/// The most recent `*.jsonl` in `dir`. Session names start with a millisecond timestamp, so
/// the greatest name is the newest session.
pub fn latest(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
        .max()
}

/// A new session path in `dir`, named for the current time.
pub fn new_path(dir: &Path) -> PathBuf {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    dir.join(format!("{millis:013}.jsonl"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt as _;

    #[test]
    fn append_then_load_round_trips_without_the_system_prompt() {
        let dir = std::env::temp_dir().join(format!("sjel-agent-session-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = new_path(&dir);
        let turn = [
            Message::System {
                content: "s".into(),
            },
            Message::User {
                content: "u".into(),
            },
            Message::Assistant {
                content: Some("a".into()),
                tool_calls: None,
            },
        ];
        append(&path, &turn).unwrap();
        append(
            &path,
            &[Message::User {
                content: "u2".into(),
            }],
        )
        .unwrap();

        let loaded = load(&path).unwrap();
        assert_eq!(loaded.len(), 3);
        assert_eq!(loaded[0], turn[1]);
        assert_eq!(
            loaded[2],
            Message::User {
                content: "u2".into()
            }
        );
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(latest(&dir), Some(path.clone()));

        std::fs::write(&path, "{\"role\":\"user\",\"content\":\"x\"}\nnot json\n").unwrap();
        assert!(load(&path).unwrap_err().contains(":2:"));
    }
}
