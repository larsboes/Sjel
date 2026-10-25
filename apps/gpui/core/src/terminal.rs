//! The terminal capability: pty sessions held on the Mac, their output stream, and keystrokes.
//!
//! A terminal is not a run of the agent loop, so it has its own routes and its own stream. What
//! makes it attachable is the same property sessions have: output is kept on the Mac with a byte
//! offset on every frame, so a second screen asks for what it missed instead of restarting the
//! program. `from` is that offset, and `truncated` is the one answer a reader must act on by
//! clearing its grid and asking again.
//!
//! Nothing here draws. `ui/src/terminal.rs` owns the grid, its colours and the keys.

use base64::Engine as _;
use futures::channel::mpsc::{unbounded, UnboundedReceiver};
use serde::Deserialize;
use serde_json::json;

use crate::shell::{spawn, Shell, ShellResult};

/// The path prefix the shell proxies to the capability.
const BASE: &str = "/agent";
/// Opening a terminal answers once the program is running, or with the reason it is not.
const OPEN_TIMEOUT: u64 = 20;
/// A stream lives until the program ends, which for a shell is however long someone is using it.
const STREAM_TIMEOUT: u64 = 24 * 60 * 60;
/// Opening, typing, resizing and closing are all one small write.
const WRITE_TIMEOUT: u64 = 15;
/// The most input one request may carry, before base64. The capability refuses more.
pub const MAX_INPUT_BYTES: usize = 64 * 1024;

/// One terminal the capability holds, as `GET /api/agent/terminals` lists it.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct Terminal {
    pub id: String,
    /// The preset that started it: `shell`, `claude`, `codex`, `pi` or one the overlay names.
    #[serde(default)]
    pub preset: String,
    /// The program, for a list that has to tell two shells apart.
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub started_at: i64,
    #[serde(default)]
    pub last_output_at: i64,
    /// False once the program has ended, whether by itself or by `DELETE`.
    #[serde(default)]
    pub alive: bool,
    #[serde(default)]
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub exit_signal: Option<String>,
    /// How many streams are attached, this client included.
    #[serde(default)]
    pub clients: usize,
}

impl Terminal {
    /// What a list row says: the program, or the preset when the program gave no name.
    pub fn label(&self) -> String {
        match self.title.trim() {
            "" => self.preset.clone(),
            title => title.to_owned(),
        }
    }

    /// The last path component of the working directory, for a row that has no room for the rest.
    pub fn directory(&self) -> String {
        self.cwd
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .filter(|name| !name.is_empty())
            .unwrap_or(&self.cwd)
            .to_owned()
    }

    /// How it ended, for the line under the grid.
    pub fn ending(&self) -> Option<String> {
        if self.alive {
            return None;
        }
        Some(match (&self.exit_signal, self.exit_code) {
            (Some(signal), _) => format!("terminated by {signal}"),
            (None, Some(code)) => format!("exited with {code}"),
            (None, None) => "ended".to_owned(),
        })
    }
}

/// What one frame of a terminal's stream carried.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Update {
    /// Bytes the terminal printed, at this byte offset in its total output.
    Output { offset: u64, data: Vec<u8> },
    /// The offset this reader asked for is older than the scrollback still held. The client
    /// clears its grid and asks again from `first_offset`; the stream stops after this frame.
    Truncated { first_offset: u64 },
    /// The program ended. The stream closes after this frame.
    Exit { code: i32, signal: Option<String> },
}

/// What `POST /api/agent/terminals` answers.
#[derive(Clone, Debug, Default, Deserialize)]
struct Opened {
    #[serde(default)]
    id: String,
}

impl Shell {
    /// Every terminal held on the Mac, newest first.
    pub async fn terminals(&self) -> ShellResult<Vec<Terminal>> {
        self.json(
            reqwest::Method::GET,
            &format!("{BASE}/api/agent/terminals"),
            None,
            WRITE_TIMEOUT,
        )
        .await
    }

    /// Starts a terminal and answers its id. `cwd` is an absolute path under `$HOME`; absent
    /// means home, which is the capability's default rather than a guess made here.
    pub async fn terminal_open(
        &self,
        preset: &str,
        cwd: Option<&str>,
        cols: u16,
        rows: u16,
    ) -> ShellResult<String> {
        let mut body = json!({ "preset": preset, "cols": cols, "rows": rows });
        if let Some(cwd) = cwd.filter(|cwd| !cwd.trim().is_empty()) {
            body["cwd"] = json!(cwd);
        }
        let opened: Opened = self
            .json(
                reqwest::Method::POST,
                &format!("{BASE}/api/agent/terminals"),
                Some(body),
                OPEN_TIMEOUT,
            )
            .await?;
        Ok(opened.id)
    }

    /// Attaches to a terminal from byte `from`. A fresh attachment asks from the oldest output
    /// the Mac still holds, which is what makes a reopened window show the screen it left.
    pub async fn terminal_stream(
        &self,
        id: &str,
        from: u64,
    ) -> ShellResult<UnboundedReceiver<Update>> {
        let response = self
            .send(
                reqwest::Method::GET,
                &format!("{BASE}/api/agent/terminals/{id}/stream?from={from}"),
                None,
                STREAM_TIMEOUT,
            )
            .await?;
        let (to, channel) = unbounded();
        spawn(async move {
            let mut response = response;
            let mut buffer = Vec::new();
            loop {
                let chunk = match response.chunk().await {
                    Ok(Some(chunk)) => chunk,
                    Ok(None) | Err(_) => break,
                };
                buffer.extend_from_slice(&chunk);
                while let Some(frame) = crate::agent::next_frame(&mut buffer) {
                    let Some(update) = update_from(&frame.event, &frame.data) else {
                        continue;
                    };
                    let ended = !matches!(update, Update::Output { .. });
                    if to.unbounded_send(update).is_err() || ended {
                        return;
                    }
                }
            }
        });
        Ok(channel)
    }

    /// Writes bytes to the pty, as if typed.
    pub async fn terminal_input(&self, id: &str, data: &[u8]) -> ShellResult<()> {
        if data.len() > MAX_INPUT_BYTES {
            return Err(crate::shell::ShellError::Transport(format!(
                "input is at most {MAX_INPUT_BYTES} bytes"
            )));
        }
        let body = json!({ "data": base64::engine::general_purpose::STANDARD.encode(data) });
        let path = format!("{BASE}/api/agent/terminals/{id}/input");
        // The capability answers `204 No Content`; there is no body to read.
        self.send(reqwest::Method::POST, &path, Some(body), WRITE_TIMEOUT)
            .await
            .map(|_| ())
    }

    /// Changes the pty's size. The last resize wins.
    pub async fn terminal_resize(&self, id: &str, cols: u16, rows: u16) -> ShellResult<()> {
        self.send(
            reqwest::Method::POST,
            &format!("{BASE}/api/agent/terminals/{id}/resize"),
            Some(json!({ "cols": cols, "rows": rows })),
            WRITE_TIMEOUT,
        )
        .await
        .map(|_| ())
    }

    /// Ends the program: SIGHUP to its process group, SIGKILL after five seconds.
    pub async fn terminal_close(&self, id: &str) -> ShellResult<()> {
        self.send(
            reqwest::Method::DELETE,
            &format!("{BASE}/api/agent/terminals/{id}"),
            None,
            OPEN_TIMEOUT,
        )
        .await
        .map(|_| ())
    }
}

/// One frame as the client's own vocabulary, or `None` for a name this client does not know.
///
/// An unknown event is skipped rather than fatal: a capability that grows a frame kind must not
/// make an older client drop the connection that carries the ones it does understand.
fn update_from(event: &str, data: &str) -> Option<Update> {
    match event {
        "output" => {
            #[derive(Deserialize)]
            struct Output {
                offset: u64,
                data: String,
            }
            let output: Output = serde_json::from_str(data).ok()?;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(output.data.as_bytes())
                .ok()?;
            Some(Update::Output {
                offset: output.offset,
                data: bytes,
            })
        }
        "truncated" => {
            #[derive(Deserialize)]
            struct Truncated {
                first_offset: u64,
            }
            let truncated: Truncated = serde_json::from_str(data).ok()?;
            Some(Update::Truncated {
                first_offset: truncated.first_offset,
            })
        }
        "exit" => {
            #[derive(Deserialize)]
            struct Exit {
                #[serde(default)]
                code: i32,
                #[serde(default)]
                signal: Option<String>,
            }
            let exit: Exit = serde_json::from_str(data).ok()?;
            Some(Update::Exit {
                code: exit.code,
                signal: exit.signal,
            })
        }
        _ => None,
    }
}

/// Where one attachment is in the output: the byte offset it has drawn up to.
///
/// Separate from the client so a reconnect cannot forget it: reattaching with `offset` is the
/// whole of resume, and a frame that arrives twice — a replay after a reconnect — is not drawn
/// twice.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Position {
    offset: u64,
}

impl Position {
    /// A new attachment from `from`, or from the start of what the Mac still holds.
    pub fn new(from: u64) -> Self {
        Self { offset: from }
    }

    pub fn offset(self) -> u64 {
        self.offset
    }

    /// Take one output frame, and answer whether the grid should draw it.
    ///
    /// A frame that ends at or before the position is a replay and is dropped. A frame that
    /// starts past it leaves a hole the client cannot fill, so it is drawn and the position
    /// jumps: showing the next screen is better than showing a grid that never advances.
    pub fn accept(&mut self, offset: u64, len: usize) -> bool {
        let end = offset + len as u64;
        if end <= self.offset {
            return false;
        }
        self.offset = end;
        true
    }

    /// Move to the oldest offset still held after a `truncated` frame.
    pub fn restart(&mut self, first_offset: u64) {
        self.offset = first_offset;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_output_frame_carries_its_offset_and_its_bytes() {
        let update = update_from("output", r#"{"offset":41,"data":"bHMK"}"#).unwrap();
        assert_eq!(
            update,
            Update::Output {
                offset: 41,
                data: b"ls\n".to_vec()
            }
        );
    }

    #[test]
    fn a_truncated_frame_says_where_the_scrollback_now_starts() {
        assert_eq!(
            update_from("truncated", r#"{"first_offset":900}"#),
            Some(Update::Truncated { first_offset: 900 })
        );
    }

    #[test]
    fn an_exit_frame_keeps_the_signal_as_well_as_the_code() {
        assert_eq!(
            update_from("exit", r#"{"code":1,"signal":"SIGHUP"}"#),
            Some(Update::Exit {
                code: 1,
                signal: Some("SIGHUP".to_owned())
            })
        );
        assert_eq!(
            update_from("exit", r#"{"code":0}"#),
            Some(Update::Exit {
                code: 0,
                signal: None
            })
        );
    }

    #[test]
    fn a_frame_this_client_does_not_know_is_skipped_not_fatal() {
        assert_eq!(update_from("resize", r#"{"cols":100}"#), None);
        // Nor is a frame whose body it cannot read: half a frame must not end the stream.
        assert_eq!(update_from("output", "not json"), None);
        assert_eq!(update_from("output", r#"{"offset":0,"data":"***"}"#), None);
    }

    #[test]
    fn input_is_base64_on_the_wire() {
        // What `echo hi` looks like as the capability's `{data}`.
        assert_eq!(
            base64::engine::general_purpose::STANDARD.encode(b"echo hi\n"),
            "ZWNobyBoaQo="
        );
    }

    #[test]
    fn a_replayed_frame_is_not_drawn_twice() {
        let mut position = Position::new(0);
        assert!(position.accept(0, 5));
        assert_eq!(position.offset(), 5);
        assert!(!position.accept(0, 5), "the same frame arrived twice");
        assert!(position.accept(5, 3));
        assert_eq!(position.offset(), 8);

        // A frame past the position leaves a hole; the grid still moves on.
        assert!(position.accept(20, 2));
        assert_eq!(position.offset(), 22);

        // Truncation moves the position back to what the Mac still holds.
        position.restart(100);
        assert_eq!(position.offset(), 100);
    }

    #[test]
    fn a_row_names_the_program_and_the_directory() {
        let terminal = Terminal {
            title: "zsh".into(),
            preset: "shell".into(),
            cwd: "/Users/lars/Developer/Sjel".into(),
            alive: true,
            ..Terminal::default()
        };
        assert_eq!(terminal.label(), "zsh");
        assert_eq!(terminal.directory(), "Sjel");
        assert_eq!(terminal.ending(), None);

        let unnamed = Terminal {
            title: "  ".into(),
            preset: "claude".into(),
            cwd: "/Users/lars".into(),
            alive: true,
            ..Terminal::default()
        };
        assert_eq!(unnamed.label(), "claude");
        assert_eq!(unnamed.directory(), "lars");
    }

    #[test]
    fn an_ended_terminal_says_how_it_ended() {
        let closed = |code: Option<i32>, signal: Option<&str>| Terminal {
            alive: false,
            exit_code: code,
            exit_signal: signal.map(str::to_owned),
            ..Terminal::default()
        };
        assert_eq!(
            closed(None, Some("SIGHUP")).ending().as_deref(),
            Some("terminated by SIGHUP")
        );
        assert_eq!(
            closed(Some(0), None).ending().as_deref(),
            Some("exited with 0")
        );
        assert_eq!(closed(None, None).ending().as_deref(), Some("ended"));
    }

    #[test]
    fn a_listed_terminal_reads_the_capabilitys_row() {
        let row: Terminal = serde_json::from_str(
            r#"{"id":"cc88","preset":"shell","title":"zsh","cwd":"/Users/lars",
                "started_at":1792952895,"last_output_at":1792952915,"alive":true,
                "exit_code":null,"exit_signal":null,"clients":0}"#,
        )
        .unwrap();
        assert_eq!(row.id, "cc88");
        assert!(row.alive);
        assert_eq!(row.started_at, 1_792_952_895);
        // A capability that grows a field must not make this client unreadable.
        let unknown: Terminal =
            serde_json::from_str(r#"{"id":"x","alive":false,"future_field":true}"#).unwrap();
        assert_eq!(unknown.id, "x");
        assert!(!unknown.alive);
    }
}
