//! A conversation on disk: one [`Message`] per line (JSONL), appended after every turn.
//!
//! Appending, not rewriting, means a crash loses at most the turn in progress. The system
//! prompt is not stored: a resumed session gets the prompt of the run that resumes it, so an
//! `AGENTS.md` edited in between takes effect.
//!
//! A session can hold any file the model read, so a new file is created owner-only (0600).
//! Where the files live is the front end's decision. The CLI puts them in the private overlay.

use std::fs::OpenOptions;
use std::io::{Read, Write};
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

/// Atomically replaces the contents of `path` with `messages` (excluding system prompt).
/// Used after conversation compaction rewrites the session history.
pub fn overwrite(path: &Path, messages: &[Message]) -> std::io::Result<()> {
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
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(&tmp)?
        .write_all(out.as_bytes())?;
    std::fs::rename(&tmp, path)
}

/// Reads a session back. An invalid, unterminated final line is treated as an interrupted write
/// and truncated before returning; a malformed complete line is an error with its line number.
/// Skipping a complete line could send tool results without their calls.
pub fn load(path: &Path) -> Result<Vec<Message>, String> {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|e| format!("{}: {e}", path.display()))?;

    let mut messages = Vec::new();
    let mut offset = 0;
    for (i, line) in bytes.split_inclusive(|byte| *byte == b'\n').enumerate() {
        let terminated = line.ends_with(b"\n");
        let line = line.strip_suffix(b"\n").unwrap_or(line);
        let text = match std::str::from_utf8(line) {
            Ok(text) => text,
            Err(_) if !terminated && offset + line.len() == bytes.len() => {
                file.set_len(offset as u64)
                    .map_err(|e| format!("{}: {e}", path.display()))?;
                return Ok(messages);
            }
            Err(error) => return Err(format!("{}:{}: {error}", path.display(), i + 1)),
        };
        if !text.trim().is_empty() {
            match serde_json::from_str::<Message>(text) {
                Ok(message) if !matches!(message, Message::System { .. }) => {
                    messages.push(message);
                }
                Ok(_) => {}
                Err(_) if !terminated && offset + line.len() == bytes.len() => {
                    // An unterminated final record can be a write interrupted by a crash. Drop
                    // only that tail so the next append starts on a clean line. Complete lines
                    // still fail loudly: skipping one could pair tool results with the wrong call.
                    file.set_len(offset as u64)
                        .map_err(|e| format!("{}: {e}", path.display()))?;
                    return Ok(messages);
                }
                Err(error) => {
                    return Err(format!("{}:{}: {error}", path.display(), i + 1));
                }
            }
        }
        offset += line.len() + usize::from(terminated);
    }
    if !bytes.is_empty() && !bytes.ends_with(b"\n") {
        // A complete record can reach disk before its line terminator. Restore the delimiter so
        // the next append cannot merge its JSON into this record.
        file.write_all(b"\n")
            .map_err(|e| format!("{}: {e}", path.display()))?;
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

    #[test]
    fn load_truncates_an_interrupted_final_record_before_the_next_append() {
        let dir =
            std::env::temp_dir().join(format!("sjel-agent-session-tail-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = new_path(&dir);
        let first = Message::User {
            content: "kept".into(),
        };
        append(&path, std::slice::from_ref(&first)).unwrap();
        let mut interrupted = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        interrupted
            .write_all(b"{\"role\":\"assistant\",\"content\":\"partial")
            .unwrap();
        drop(interrupted);

        let loaded = load(&path).unwrap();
        assert_eq!(loaded.as_slice(), std::slice::from_ref(&first));
        assert!(std::fs::read(&path).unwrap().ends_with(b"\n"));

        let next = Message::User {
            content: "next".into(),
        };
        append(&path, std::slice::from_ref(&next)).unwrap();
        assert_eq!(load(&path).unwrap(), [first, next]);
    }

    #[test]
    fn load_terminates_a_complete_final_record_before_the_next_append() {
        let dir =
            std::env::temp_dir().join(format!("sjel-agent-session-line-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = new_path(&dir);
        let first = Message::User {
            content: "kept".into(),
        };
        let record = serde_json::to_vec(&first).unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &record).unwrap();

        let loaded = load(&path).unwrap();
        assert_eq!(loaded.as_slice(), std::slice::from_ref(&first));
        assert!(std::fs::read(&path).unwrap().ends_with(b"\n"));

        let next = Message::User {
            content: "next".into(),
        };
        append(&path, std::slice::from_ref(&next)).unwrap();
        assert_eq!(load(&path).unwrap(), [first, next]);
    }

    #[test]
    fn load_truncates_an_incomplete_utf8_tail() {
        let dir =
            std::env::temp_dir().join(format!("sjel-agent-session-utf8-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = new_path(&dir);
        let first = Message::User {
            content: "kept".into(),
        };
        append(&path, std::slice::from_ref(&first)).unwrap();
        let mut bytes = std::fs::read(&path).unwrap();
        bytes.extend_from_slice(b"{\"role\":\"user\",\"content\":\"");
        bytes.extend_from_slice(&[0xF0, 0x9F]);
        std::fs::write(&path, bytes).unwrap();

        assert_eq!(load(&path).unwrap(), [first]);
        assert!(std::fs::read(&path).unwrap().ends_with(b"\n"));
    }
}
